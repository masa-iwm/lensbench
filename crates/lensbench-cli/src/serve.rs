//! `lensbench serve`: 系を読み込んだまま待ち受け、外から JSON-RPC で系の変更・解析を繰り返す。
//!
//! 毎回の起動・読み込み・前処理を省いて「解析 → 別アプリで系を更新 → 再解析」のループを速くする。
//! 書式は docs/serve.md。要点:
//!
//! - TCP で 1 行 1 要求の JSON-RPC 2.0（改行で終わる JSON）。既定は 127.0.0.1 だけで待ち受ける
//! - 配列は JSON に入れず、ヘッダの `"bin": [{"name", "dtype", "shape"}, ...]` に並べ、改行の直後に
//!   その順で生のバイト列（リトルエンディアン、C 順）を続ける。要求・応答とも同じ形
//! - 系は 1 プロセスに 1 つ。要求は 1 つずつ順に処理する（複数の接続も順番待ち）。別の系を並べて回すときは
//!   プロセスを分ける（`--threads 1` で 1 プロセス 1 スレッドにできる）
//! - 解析（`analyze`）は CLI のサブコマンドと同じ引数・同じ結果。系はファイルを読まず、読み込んだ系を使う

use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use web_time::Instant;

use clap::Args;
use lensbench_core::coating::C64;
use lensbench_core::system::Launch;
use lensbench_core::{zmx_write, System};
use lensbench_export::d2::Plane;
use serde_json::{json, Map, Value};

use super::{execute_in, ext, load_file, Cmd, Common};
use crate::fringe_cmd::FringeArgs;

#[derive(Args)]
pub struct ServeArgs {
    /// 最初に読み込む系（.zmx かプロジェクトファイル .json。省くと load で読む）
    file: Option<PathBuf>,
    /// ガラスカタログを探すフォルダ（複数可）
    #[arg(long = "glass-dir")]
    glass_dir: Vec<PathBuf>,
    /// プロジェクトファイルのカタログレンズを探すレンズカタログ（複数可）
    #[arg(long = "lens-catalog")]
    lens_catalog: Vec<PathBuf>,
    /// マルチコンフィグの番号（1 始まり）
    #[arg(long, default_value_t = 1)]
    cfg: usize,
    /// プロジェクトファイルの状態の名前
    #[arg(long)]
    state: Option<String>,
    /// プロジェクトファイルのビームスプリッターの経路（T・R の並び）
    #[arg(long)]
    path: Option<String>,
    /// 待ち受けるアドレス（既定 127.0.0.1。ほかのアドレスは --allow-remote が要る）
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// 待ち受けるポート（0 で空いているポートを選ぶ）。選んだポートは最初の行に出す
    #[arg(long, default_value_t = 7010)]
    port: u16,
    /// 選んだポートの番号を書くファイル（--port 0 と一緒に使う）
    #[arg(long = "port-file")]
    port_file: Option<PathBuf>,
    /// localhost 以外からの接続を受ける（認証はないので信頼できるネットワークだけで）。
    /// このときは要求でファイルを読み書きしない（--root を与えればその下だけ）
    #[arg(long = "allow-remote")]
    allow_remote: bool,
    /// 要求で読み書きするファイルをこのフォルダの下に限る（相対パスはここから）
    #[arg(long)]
    root: Option<PathBuf>,
    /// 1 つの要求の配列の合計の上限 [バイト]（超える要求は断って接続を閉じる）
    #[arg(long = "max-request-bytes", default_value_t = 2 << 30)]
    max_request_bytes: u64,
    /// TCP の代わりにパイプで待ち受ける。Windows は名前付きパイプ（\\.\pipe\NAME、同じ PC だけ）、
    /// ほかは NAME のパスの Unix ドメインソケット
    #[arg(long)]
    pipe: Option<String>,
}

/// 共有メモリを使ってよいか（--allow-remote では使わない）
static SHM_OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// 要求で与えたパスの扱い
enum Files {
    /// そのまま読み書きする（localhost だけで待ち受け、--root なし）
    Any,
    /// このフォルダ（正規化済み）の下だけ
    Root(PathBuf),
    /// 読み書きしない（--allow-remote で --root なし）
    Deny,
}

static FILES: std::sync::OnceLock<Files> = std::sync::OnceLock::new();

/// 評価関数の項（1 つか並び）の zplm のファイル・フォルダを [`check_path`] で確かめる。ファイルを限っているとき
/// （--root・--allow-remote）は、既定の場所（LENSBENCH_MACROS・今のフォルダ）を探さない
fn check_term_paths(t: &mut Value) -> Result<(), RpcErr> {
    let limited = !matches!(FILES.get().unwrap_or(&Files::Any), Files::Any);
    let items: Vec<&mut Value> = match t {
        Value::Array(a) => a.iter_mut().collect(),
        x => vec![x],
    };
    for x in items {
        let Some(o) = x.as_object_mut() else { continue };
        let ty = o.get("type").and_then(Value::as_str).unwrap_or_default();
        if !ty.eq_ignore_ascii_case("zplm") {
            continue;
        }
        if let Some(f) = o.get_mut("file") {
            if let Some(p) = f.as_str() {
                *f = json!(check_path(Path::new(p), false)?.to_string_lossy());
            }
        }
        if let Some(Value::Array(ds)) = o.get_mut("macro_dirs") {
            for d in ds.iter_mut() {
                if let Some(p) = d.as_str() {
                    *d = json!(check_path(Path::new(p), false)?.to_string_lossy());
                }
            }
        }
        if limited {
            o.insert("default_dirs".into(), json!(false));
        }
    }
    Ok(())
}

/// 要求で与えたパスを確かめて、使うパスを返す（write は書き出し）
fn check_path(p: &Path, write: bool) -> Result<PathBuf, RpcErr> {
    match FILES.get().unwrap_or(&Files::Any) {
        Files::Any => Ok(p.to_path_buf()),
        Files::Deny => Err(RpcErr::app(format!(
            "{}: --allow-remote では要求でファイルを読み書きしない（--root で場所を限れば読み書きできる）",
            p.display()
        ))),
        Files::Root(root) => {
            let q = if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            };
            let bad = || RpcErr::app(format!("{} は --root の外", p.display()));
            // 在るものは実体（シンボリックリンクの先）で、書き出す新しいファイルはフォルダの実体で確かめる
            let real = match q.canonicalize() {
                Ok(r) => r,
                Err(_) if write => {
                    let name = q.file_name().filter(|n| *n != "..").ok_or_else(bad)?;
                    let dir = q.parent().ok_or_else(bad)?;
                    let dir = dir.canonicalize().map_err(|e| {
                        RpcErr::app(format!("{}: {e}", dir.display()))
                    })?;
                    dir.join(name)
                }
                Err(e) => return Err(RpcErr::app(format!("{}: {e}", p.display()))),
            };
            if real.starts_with(root) {
                Ok(real)
            } else {
                Err(bad())
            }
        }
    }
}

/// 系の読み方（reload で同じ読み方を繰り返す）
#[derive(Clone)]
struct LoadSpec {
    file: PathBuf,
    glass_dir: Vec<PathBuf>,
    lens_catalog: Vec<PathBuf>,
    cfg: usize,
    state: Option<String>,
    path: Option<String>,
}

impl LoadSpec {
    fn common(&self) -> Common {
        Common {
            file: self.file.clone(),
            glass_dir: self.glass_dir.clone(),
            lens_catalog: self.lens_catalog.clone(),
            cfg: self.cfg,
            state: self.state.clone(),
            path: self.path.clone(),
            json: false,
            save: None,
        }
    }
}

/// 読み込んだ後の変更（干渉の解析で、別の構成・状態の光束にも同じ変更を当てる）
#[derive(Clone)]
enum Change {
    /// set: (名前, 変える前の値, 値)
    Set(Vec<(String, f64, f64)>),
    /// edit の ops
    Edit(Value),
}

struct Served {
    sys: Arc<System>,
    spec: LoadSpec,
    log: Vec<Change>,
}

/// 読み込んだ系（プロセスに 1 つ）
static SERVED: Mutex<Option<Served>> = Mutex::new(None);
/// 要求を 1 つずつ処理する
static REQUEST: Mutex<()> = Mutex::new(());

fn served() -> std::sync::MutexGuard<'static, Option<Served>> {
    SERVED.lock().unwrap_or_else(|e| e.into_inner())
}

/// serve で読み込んだ系（serve でなければ None）。CLI の解析はこれがあればファイルを読まずに使う
pub(crate) fn loaded() -> Option<System> {
    served().as_ref().map(|x| (*x.sys).clone())
}

fn current() -> Result<(Arc<System>, LoadSpec), RpcErr> {
    served()
        .as_ref()
        .map(|x| (x.sys.clone(), x.spec.clone()))
        .ok_or_else(|| RpcErr::app("系がない（先に load）"))
}

/// 読み込んだ後の変更の記録
fn current_log() -> Vec<Change> {
    served().as_ref().map(|x| x.log.clone()).unwrap_or_default()
}

/// 読み込んだ系を置く（変更の記録は空にする）
fn store(sys: System, spec: LoadSpec) {
    *served() = Some(Served {
        sys: Arc::new(sys),
        spec,
        log: Vec::new(),
    });
}

/// 変更した系を置き、変更を記録する
fn store_changed(sys: System, spec: LoadSpec, c: Change) {
    let mut g = served();
    let mut log = g.take().map(|x| x.log).unwrap_or_default();
    log.push(c);
    *g = Some(Served {
        sys: Arc::new(sys),
        spec,
        log,
    });
}

/// 別に読んだ光束の系に、読み込んだ後の変更（set・edit）を順に当てる。set のパラメータの値が、読み込んだ系で
/// 変える前の値と違うもの（構成・状態で値が違うもの）は当てずに notes に書く
fn replay(
    mut s: System,
    log: &[Change],
    beam: &str,
    notes: &mut Vec<String>,
) -> Result<System, String> {
    let e = |e: lensbench_core::Error| format!("{beam}: {e}");
    for c in log {
        match c {
            Change::Set(v) => {
                let mut ch = Vec::with_capacity(v.len());
                for (name, before, value) in v {
                    let q = s.param(name).map_err(e)?;
                    let now = s.get_param(q).map_err(e)?;
                    if now == *before {
                        ch.push((q, *value));
                    } else {
                        let n = format!(
                            "{beam}: {name} は読み込んだ系と値が違う（{now}、読み込んだ系 {before}）ので set を当てない"
                        );
                        if !notes.contains(&n) {
                            notes.push(n);
                        }
                    }
                }
                if !ch.is_empty() {
                    s = s.with_params(&ch).map_err(e)?;
                }
            }
            Change::Edit(ops) => {
                lensbench_project::edit_ops::apply(&mut s, ops)
                    .map_err(|x| format!("{beam}: {x}"))?;
            }
        }
    }
    Ok(s)
}

/// JSON-RPC のエラー
struct RpcErr {
    code: i64,
    msg: String,
}

impl RpcErr {
    fn app(m: impl Into<String>) -> Self {
        RpcErr {
            code: -32000,
            msg: m.into(),
        }
    }
    fn params(m: impl Into<String>) -> Self {
        RpcErr {
            code: -32602,
            msg: m.into(),
        }
    }
}

impl From<String> for RpcErr {
    fn from(m: String) -> Self {
        RpcErr::app(m)
    }
}

impl From<lensbench_core::Error> for RpcErr {
    fn from(e: lensbench_core::Error) -> Self {
        RpcErr::app(e.to_string())
    }
}

/// 配列（バイナリで渡す）
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Data {
    F64(Vec<f64>),
    I32(Vec<i32>),
    U8(Vec<u8>),
    C64(Vec<C64>),
    Bool(Vec<bool>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Bin {
    pub name: String,
    pub shape: Vec<usize>,
    pub data: Data,
}

impl Bin {
    fn f64(name: &str, v: Vec<f64>) -> Bin {
        Bin {
            name: name.into(),
            shape: vec![v.len()],
            data: Data::F64(v),
        }
    }
    fn i32(name: &str, v: Vec<i32>) -> Bin {
        Bin {
            name: name.into(),
            shape: vec![v.len()],
            data: Data::I32(v),
        }
    }
    fn dtype(&self) -> &'static str {
        match self.data {
            Data::F64(_) => "<f8",
            Data::I32(_) => "<i4",
            Data::U8(_) => "|u1",
            Data::C64(_) => "<c16",
            Data::Bool(_) => "|b1",
        }
    }
    fn header(&self) -> Value {
        json!({ "name": self.name, "dtype": self.dtype(), "shape": self.shape })
    }
    fn write_to(&self, out: &mut Vec<u8>) {
        match &self.data {
            Data::F64(v) => {
                out.reserve(v.len() * 8);
                for x in v {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
            Data::I32(v) => {
                out.reserve(v.len() * 4);
                for x in v {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
            Data::U8(v) => out.extend_from_slice(v),
            Data::C64(v) => {
                out.reserve(v.len() * 16);
                for x in v {
                    out.extend_from_slice(&x.re.to_le_bytes());
                    out.extend_from_slice(&x.im.to_le_bytes());
                }
            }
            Data::Bool(v) => out.extend(v.iter().map(|&b| b as u8)),
        }
    }
}

/// ヘッダの "bin" から配列の並びと大きさを読む
/// 配列の名前・dtype・形・バイト数・共有メモリの位置
type Layout = Vec<(String, String, Vec<usize>, usize, Option<u64>)>;

fn bin_layout(h: &Value, limit: u64) -> Result<Layout, String> {
    let Some(list) = h.get("bin") else {
        return Ok(Vec::new());
    };
    let list = list.as_array().ok_or("bin は配列")?;
    let mut total: u64 = 0;
    list.iter()
        .map(|b| {
            let name = b["name"].as_str().ok_or("bin の name がない")?.to_string();
            let dtype = b["dtype"]
                .as_str()
                .ok_or("bin の dtype がない")?
                .to_string();
            let shape: Vec<usize> = match &b["shape"] {
                Value::Array(a) => a
                    .iter()
                    .map(|x| x.as_u64().map(|x| x as usize).ok_or("shape は整数"))
                    .collect::<Result<_, _>>()?,
                Value::Number(n) => vec![n.as_u64().ok_or("shape は整数")? as usize],
                _ => return Err("bin の shape がない".to_string()),
            };
            let item = match dtype.as_str() {
                "<f8" | "f8" | "float64" => 8,
                "<i4" | "i4" | "int32" => 4,
                "|u1" | "u1" | "uint8" | "|b1" | "b1" | "bool" => 1,
                "<c16" | "c16" | "complex128" => 16,
                d => return Err(format!("dtype {d} は使えない（<f8・<i4・|u1・|b1・<c16）")),
            };
            // 積のあふれと、要求の合計の上限を確かめてから確保する
            let too_big =
                || format!("bin が大きすぎる（合計の上限 {limit} バイト、--max-request-bytes）");
            let nb = shape
                .iter()
                .try_fold(item as u64, |a, &d| a.checked_mul(d as u64))
                .ok_or_else(too_big)?;
            total = total.checked_add(nb).ok_or_else(too_big)?;
            if total > limit {
                return Err(too_big());
            }
            let nb = usize::try_from(nb).map_err(|_| too_big())?;
            let off = match b.get("offset") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.as_u64().ok_or("bin の offset は 0 以上の整数")?),
            };
            Ok((name, dtype, shape, nb, off))
        })
        .collect()
}

fn decode(dtype: &str, b: &[u8]) -> Data {
    match dtype {
        "<f8" | "f8" | "float64" => Data::F64(
            b.chunks_exact(8)
                .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
                .collect(),
        ),
        "<i4" | "i4" | "int32" => Data::I32(
            b.chunks_exact(4)
                .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
                .collect(),
        ),
        "<c16" | "c16" | "complex128" => Data::C64(
            b.chunks_exact(16)
                .map(|c| {
                    C64::new(
                        f64::from_le_bytes(c[..8].try_into().unwrap()),
                        f64::from_le_bytes(c[8..].try_into().unwrap()),
                    )
                })
                .collect(),
        ),
        "|b1" | "b1" | "bool" => Data::Bool(b.iter().map(|&x| x != 0).collect()),
        _ => Data::U8(b.to_vec()),
    }
}

/// 1 つのメッセージ（ヘッダの JSON と配列）
pub(crate) struct Msg {
    pub head: Value,
    pub bins: Vec<Bin>,
    /// 共有メモリの配列が読めなかった（ストリームは続けられる）
    pub shm_err: Option<String>,
}

/// ヘッダの共有メモリの名前と、応答を書き始める位置
fn shm_spec(head: &Value) -> Option<(String, u64)> {
    let s = head.get("shm")?;
    let name = s.get("name")?.as_str()?.to_string();
    let out = s.get("out").and_then(Value::as_u64).unwrap_or(0);
    Some((name, out))
}

fn open_shm(head: &Value) -> Result<crate::serve_ipc::Shm, String> {
    if !SHM_OK.get().copied().unwrap_or(false) {
        return Err("共有メモリは使えない（--allow-remote、または serve の外）".into());
    }
    let (name, _) = shm_spec(head).ok_or("bin に offset があるのに shm の name がない")?;
    crate::serve_ipc::Shm::open(&name)
}

/// メッセージを読む（接続が閉じたら None）。JSON が読めないときは Err（配列の長さが分からないので続けられない）
pub(crate) fn read_msg(
    r: &mut impl BufRead,
    limit: u64,
) -> std::io::Result<Option<Result<Msg, String>>> {
    let mut line = Vec::new();
    loop {
        line.clear();
        if r.read_until(b'\n', &mut line)? == 0 {
            return Ok(None);
        }
        if line.iter().any(|c| !c.is_ascii_whitespace()) {
            break;
        }
    }
    let head: Value = match serde_json::from_slice(&line) {
        Ok(v) => v,
        Err(e) => return Ok(Some(Err(format!("JSON が読めない: {e}")))),
    };
    let layout = match bin_layout(&head, limit) {
        Ok(l) => l,
        Err(e) => return Ok(Some(Err(e))),
    };
    let mut bins = Vec::with_capacity(layout.len());
    let mut buf = Vec::new();
    let mut shm = None;
    let mut shm_err = None;
    for (name, dtype, shape, nb, off) in layout {
        buf.resize(nb, 0);
        match off {
            None => r.read_exact(&mut buf)?,
            Some(_) if shm_err.is_some() => continue,
            Some(off) => {
                if shm.is_none() {
                    match open_shm(&head) {
                        Ok(m) => shm = Some(m),
                        Err(e) => {
                            shm_err = Some(e);
                            continue;
                        }
                    }
                }
                if let Err(e) = shm.as_ref().unwrap().read(off, &mut buf) {
                    shm_err = Some(e);
                    continue;
                }
            }
        }
        bins.push(Bin {
            name,
            shape,
            data: decode(&dtype, &buf),
        });
    }
    Ok(Some(Ok(Msg {
        head,
        bins,
        shm_err,
    })))
}

/// 応答の配列を共有メモリに書く（out から 64 バイトそろえで）。収まらなければ書かずに false
fn write_shm(head: &mut Value, bins: &[Bin], shm: &crate::serve_ipc::Shm, out: u64) -> bool {
    let mut off = out;
    let mut list = Vec::with_capacity(bins.len());
    let mut buf = Vec::new();
    let mut place = Vec::with_capacity(bins.len());
    for b in bins {
        off = off.div_ceil(64) * 64;
        let item = match b.data {
            Data::F64(_) => 8,
            Data::I32(_) => 4,
            Data::U8(_) | Data::Bool(_) => 1,
            Data::C64(_) => 16,
        };
        let n: u64 = b.shape.iter().map(|&d| d as u64).product::<u64>() * item;
        place.push(off);
        off += n;
    }
    if off > shm.size() {
        return false;
    }
    for (b, &at) in bins.iter().zip(&place) {
        buf.clear();
        b.write_to(&mut buf);
        if shm.write(at, &buf).is_err() {
            return false;
        }
        let mut h = b.header();
        h["offset"] = json!(at);
        list.push(h);
    }
    head["bin"] = Value::Array(list);
    true
}

/// メッセージを書く（ヘッダに "bin" を足す）
pub(crate) fn write_msg(w: &mut impl Write, mut head: Value, bins: &[Bin]) -> std::io::Result<()> {
    if !bins.is_empty() {
        head["bin"] = Value::Array(bins.iter().map(Bin::header).collect());
    }
    let mut out = serde_json::to_vec(&head).unwrap();
    out.push(b'\n');
    for b in bins {
        b.write_to(&mut out);
    }
    w.write_all(&out)?;
    w.flush()
}

pub fn serve(a: ServeArgs, threads: Option<usize>) -> Result<(), String> {
    let loopback = matches!(a.host.as_str(), "127.0.0.1" | "localhost" | "::1");
    if !loopback && !a.allow_remote {
        return Err(format!(
            "--host {} は localhost ではない（認証がないので、受けるなら --allow-remote）",
            a.host
        ));
    }
    let files = match (&a.root, a.allow_remote) {
        (Some(r), _) => Files::Root(
            r.canonicalize()
                .map_err(|e| format!("--root {}: {e}", r.display()))?,
        ),
        (None, true) => Files::Deny,
        (None, false) => Files::Any,
    };
    let _ = FILES.set(files);
    let _ = SHM_OK.set(!a.allow_remote);
    let limit = a.max_request_bytes;
    if let Some(f) = &a.file {
        let spec = LoadSpec {
            file: f.clone(),
            glass_dir: a.glass_dir.clone(),
            lens_catalog: a.lens_catalog.clone(),
            cfg: a.cfg,
            state: a.state.clone(),
            path: a.path.clone(),
        };
        let s = lensbench_core::with_threads(threads, || load_file(&spec.common()))?;
        store(s, spec);
    }
    if let Some(name) = &a.pipe {
        let path = crate::serve_ipc::pipe_path(name);
        let ready = || {
            println!(
                "{}",
                json!({ "listening": path, "pipe": path, "version": lensbench_core::VERSION })
            );
            std::io::stdout().flush().ok();
            Ok(())
        };
        return crate::serve_ipc::listen_pipe(&path, ready, move |r, w, f| {
            let r = BufReader::with_capacity(1 << 16, r);
            let w = BufWriter::with_capacity(1 << 16, w);
            if let Err(e) = connection(r, w, &*f, threads, limit) {
                eprintln!("lensbench serve: 接続: {e}");
            }
        });
    }
    let l = TcpListener::bind((a.host.as_str(), a.port))
        .map_err(|e| format!("{}:{} で待ち受けられない: {e}", a.host, a.port))?;
    let addr = l.local_addr().map_err(|e| e.to_string())?;
    if let Some(p) = &a.port_file {
        lensbench_core::vfs::write(p, format!("{}\n", addr.port()))
            .map_err(|e| format!("{}: {e}", p.display()))?;
    }
    println!(
        "{}",
        json!({ "listening": addr.to_string(), "port": addr.port(), "version": lensbench_core::VERSION })
    );
    std::io::stdout().flush().ok();
    for c in l.incoming() {
        match c {
            Ok(c) => {
                std::thread::spawn(move || {
                    if let Err(e) = tcp_connection(c, threads, limit) {
                        eprintln!("lensbench serve: 接続: {e}");
                    }
                });
            }
            Err(e) => eprintln!("lensbench serve: {e}"),
        }
    }
    Ok(())
}

fn tcp_connection(c: TcpStream, threads: Option<usize>, limit: u64) -> std::io::Result<()> {
    c.set_nodelay(true).ok();
    let r = BufReader::with_capacity(1 << 16, c.try_clone()?);
    let w = BufWriter::with_capacity(1 << 16, c.try_clone()?);
    connection(r, w, &c, threads, limit)
}

/// 1 本の接続の要求を順に処理する（TCP・パイプで共通）
fn connection(
    mut r: impl BufRead,
    mut w: impl Write,
    flush: &dyn crate::serve_ipc::HardFlush,
    threads: Option<usize>,
    limit: u64,
) -> std::io::Result<()> {
    while let Some(m) = read_msg(&mut r, limit)? {
        let m = match m {
            Ok(m) => m,
            Err(e) => {
                let h = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e } });
                write_msg(&mut w, h, &[])?;
                return Ok(());
            }
        };
        let id = m.head.get("id").cloned().unwrap_or(Value::Null);
        let method = m.head["method"].as_str().unwrap_or("").to_string();
        let params = m.head.get("params").cloned().unwrap_or(json!({}));
        let t0 = Instant::now();
        let res = match m.shm_err {
            Some(e) => Err(RpcErr::params(e)),
            None => dispatch(&method, &params, m.bins, threads),
        };
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        match res {
            Ok((v, bins)) => {
                let mut h = json!({ "jsonrpc": "2.0", "id": id, "result": v, "elapsed_ms": ms });
                // 要求が共有メモリを使っていれば、応答の配列も収まる限りそこに書く
                let shm = match shm_spec(&m.head) {
                    Some((_, out)) if !bins.is_empty() => open_shm(&m.head).ok().map(|s| (s, out)),
                    _ => None,
                };
                if let Some((s, out)) = shm {
                    if write_shm(&mut h, &bins, &s, out) {
                        write_msg(&mut w, h, &[])?;
                        continue;
                    }
                }
                write_msg(&mut w, h, &bins)?;
            }
            Err(e) => {
                let h = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": e.code, "message": e.msg } });
                write_msg(&mut w, h, &[])?;
            }
        }
        if method == "shutdown" {
            w.flush()?;
            flush.hard_flush();
            std::process::exit(0);
        }
    }
    Ok(())
}

/// 要求を 1 つ処理する（1 つずつ順に。panic はエラーにする）
fn dispatch(
    method: &str,
    params: &Value,
    bins: Vec<Bin>,
    threads: Option<usize>,
) -> Result<Reply, RpcErr> {
    let _g = REQUEST.lock().unwrap_or_else(|e| e.into_inner());
    lensbench_core::with_threads(threads, || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handle(method, params, bins)
        }))
        .unwrap_or_else(|_| Err(RpcErr::app("内部エラー（panic）")))
    })
}

/// [`call`] のエラー（JSON-RPC の error と同じ code・message）
#[derive(Debug, Clone, PartialEq)]
pub struct CallError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// serve と同じ method を同じプロセスの中から呼ぶ（GUI が使う）。params・result は TCP の要求と同じ JSON。
/// 系はプロセスに 1 つで、TCP の要求と同じく 1 つずつ順に処理する。bin（配列）を使う `trace`・`echo` は
/// JSON の並びで渡す。結果の配列が要るときは [`call_arrays`]。`shutdown` はプロセスを止めない。
pub fn call(method: &str, params: &Value, threads: Option<usize>) -> Result<Value, CallError> {
    call_arrays(method, params, threads).map(|(v, _)| v)
}

/// 結果の配列（`trace` の x・y・z・opl・err・vig など。名前と f64 の並び、整数も f64 にする）
pub type Arrays = Vec<(String, Vec<f64>)>;

/// load の params から系の読み方
fn load_spec(p: &Value) -> Result<LoadSpec, RpcErr> {
    let file = p["file"]
        .as_str()
        .ok_or_else(|| RpcErr::params("load には file"))?;
    let read_all = |v: Vec<PathBuf>| -> Result<Vec<PathBuf>, RpcErr> {
        v.iter().map(|x| check_path(x, false)).collect()
    };
    Ok(LoadSpec {
        file: check_path(Path::new(file), false)?,
        glass_dir: read_all(str_list(&p["glass_dir"], "glass_dir")?)?,
        lens_catalog: read_all(str_list(&p["lens_catalog"], "lens_catalog")?)?,
        cfg: opt_usize(p, "cfg")?.unwrap_or(1),
        state: p["state"].as_str().map(str::to_string),
        path: p["path"].as_str().map(str::to_string),
    })
}

/// 系を読んで（`load` と同じ params）、そのレイアウト図（`layout` と同じ結果）を返す。読み込んだ系は変えず、
/// 他の要求の順番も待たない。GUI が、解析を回している間も組合せの図だけ先に描き直すのに使う
pub fn preview_layout(load: &Value, opts: &Value) -> Result<Value, CallError> {
    let f = || -> Result<Value, RpcErr> {
        let spec = load_spec(load)?;
        let s = load_file(&spec.common())?;
        layout_with(&s, &spec, opts)
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(RpcErr::app("内部エラー（panic）")))
        .map_err(|e| CallError {
            code: e.code,
            message: e.msg,
        })
}

/// カタログの一覧（`glasses`・`lenses`）
fn browse_of(method: &str, p: &Value) -> Result<Value, RpcErr> {
    let read_all = |k: &str| -> Result<Vec<PathBuf>, RpcErr> {
        str_list(&p[k], k)?
            .iter()
            .map(|x| check_path(x, false))
            .collect()
    };
    let dirs = read_all("glass_dir")?;
    if method == "glasses" {
        return Ok(crate::browse::glasses(&dirs));
    }
    let cat = read_all("lens_catalog")?;
    if method == "catalog_search" {
        let q = catalog_query(p)?;
        return Ok(crate::browse::search(&cat, &dirs, &q)?);
    }
    Ok(crate::browse::lenses(&cat, &dirs)?)
}

/// `catalog_search` の条件: `kinds`（品の種類の並び）・`vendor`・`text`・`specs`（`名前>=値` などの並び）
fn catalog_query(p: &Value) -> Result<lensbench_project::catalog_spec::CatalogQuery, RpcErr> {
    use lensbench_project::catalog_spec::{CatalogQuery, ItemKind, SpecFilter};
    let strs = |k: &str| -> Result<Vec<String>, RpcErr> {
        Ok(str_list(&p[k], k)?
            .into_iter()
            .map(|x| x.to_string_lossy().into_owned())
            .collect())
    };
    let kinds = strs("kinds")?
        .iter()
        .map(|k| {
            ItemKind::parse(k).ok_or_else(|| RpcErr::params(format!("kinds: 知らない種類 {k}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let specs = strs("specs")?
        .iter()
        .map(|s| SpecFilter::parse(s).map_err(RpcErr::params))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CatalogQuery {
        kinds,
        vendor: p["vendor"].as_str().unwrap_or("").to_string(),
        text: p["text"].as_str().unwrap_or("").to_string(),
        specs,
    })
}

/// カタログの一覧（`glasses`・`lenses` と同じ params・結果）。読み込んだ系に関係なく、他の要求の順番も待たない。
/// GUI が、解析を回している間もカタログを読めるように使う
/// ユーザーカタログの一覧・名前と説明の変更・削除（`catalog_items`・`catalog_edit`）を、読み込んだ系と関係なく
/// 呼ぶ（GUI の下書きの worker から。`catalog_add` は読み込んだ系が要るので [`call`] で）
pub fn user_catalog_edit(method: &str, params: &Value) -> Result<Value, CallError> {
    let f = || match method {
        "catalog_items" | "catalog_edit" => user_catalog(method, params),
        m => Err(RpcErr::params(format!(
            "{m} は catalog_items か catalog_edit"
        ))),
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(RpcErr::app("内部エラー（panic）")))
        .map_err(|e| CallError {
            code: e.code,
            message: e.msg,
        })
}

pub fn browse(method: &str, params: &Value) -> Result<Value, CallError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| browse_of(method, params)))
        .unwrap_or_else(|_| Err(RpcErr::app("内部エラー（panic）")))
        .map_err(|e| CallError {
            code: e.code,
            message: e.msg,
        })
}

/// [`call`] と同じで、結果の配列（TCP では bin で返るもの）も受け取る
pub fn call_arrays(
    method: &str,
    params: &Value,
    threads: Option<usize>,
) -> Result<(Value, Arrays), CallError> {
    dispatch(method, params, Vec::new(), threads)
        .map(|(v, bins)| {
            let a = bins
                .into_iter()
                .flat_map(|b| {
                    let d = match b.data {
                        Data::F64(x) => x,
                        Data::I32(x) => x.into_iter().map(f64::from).collect(),
                        Data::U8(x) => x.into_iter().map(f64::from).collect(),
                        Data::Bool(x) => x.into_iter().map(|b| b as u8 as f64).collect(),
                        // 複素数は 名前.re・名前.im の 2 つに分ける
                        Data::C64(x) => {
                            let im = x.iter().map(|z| z.im).collect();
                            return vec![
                                (format!("{}.re", b.name), x.iter().map(|z| z.re).collect()),
                                (format!("{}.im", b.name), im),
                            ];
                        }
                    };
                    vec![(b.name, d)]
                })
                .collect();
            (v, a)
        })
        .map_err(|e| CallError {
            code: e.code,
            message: e.msg,
        })
}

fn str_list(v: &Value, what: &str) -> Result<Vec<PathBuf>, RpcErr> {
    match v {
        Value::Null => Ok(Vec::new()),
        Value::String(s) => Ok(vec![PathBuf::from(s)]),
        Value::Array(a) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(PathBuf::from)
                    .ok_or_else(|| RpcErr::params(format!("{what} は文字列の並び")))
            })
            .collect(),
        _ => Err(RpcErr::params(format!("{what} は文字列の並び"))),
    }
}

fn opt_usize(p: &Value, k: &str) -> Result<Option<usize>, RpcErr> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(|x| Some(x as usize))
            .ok_or_else(|| RpcErr::params(format!("{k} は 0 以上の整数"))),
    }
}

fn opt_f64(p: &Value, k: &str) -> Result<Option<f64>, RpcErr> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .map(Some)
            .ok_or_else(|| RpcErr::params(format!("{k} は数"))),
    }
}

/// 読み込んだ系の概要
fn summary(s: &System, spec: &LoadSpec) -> Value {
    json!({
        "file": spec.file.display().to_string(),
        "name": s.name,
        "cfg": spec.cfg,
        "state": spec.state,
        "surfaces": s.surfaces.len(),
        "image_surface": s.image(),
        "fields": s.fields.iter().map(|f| [f.x, f.y]).collect::<Vec<_>>(),
        "wavelengths_um": s.wavelengths,
        "primary": s.primary + 1,
    })
}

type Reply = (Value, Vec<Bin>);

fn handle(method: &str, p: &Value, bins: Vec<Bin>) -> Result<Reply, RpcErr> {
    match method {
        "ping" => Ok((json!({ "version": lensbench_core::VERSION }), Vec::new())),
        "shutdown" => Ok((json!(true), Vec::new())),
        // 配列をそのまま返す（往復の時間を測る）
        "echo" => Ok((json!({ "params": p }), bins)),
        "load" => {
            let spec = load_spec(p)?;
            let s = load_file(&spec.common())?;
            let v = summary(&s, &spec);
            store(s, spec);
            Ok((v, Vec::new()))
        }
        // 同じ読み方でファイルを読み直す（set で変えた値は捨てる）。別アプリが系のファイルを書き換えた後に使う
        "reload" => {
            let (_, spec) = current()?;
            let s = load_file(&spec.common())?;
            let v = summary(&s, &spec);
            store(s, spec);
            Ok((v, Vec::new()))
        }
        "system" => {
            let (s, spec) = current()?;
            Ok((summary(&s, &spec), Vec::new()))
        }
        "get" => {
            let (s, _) = current()?;
            if let Some(one) = p.get("param").and_then(Value::as_str) {
                let v = s.get_param(s.param(one)?)?;
                return Ok((json!({ "value": v }), Vec::new()));
            }
            let list = p["params"]
                .as_array()
                .ok_or_else(|| RpcErr::params("get には param か params（並び）"))?;
            let mut out = Vec::with_capacity(list.len());
            for x in list {
                let spec = x
                    .as_str()
                    .ok_or_else(|| RpcErr::params("params は文字列の並び"))?;
                out.push(s.get_param(s.param(spec)?)?);
            }
            Ok((json!({ "values": out }), Vec::new()))
        }
        "set" => {
            let (s, spec) = current()?;
            let changes = parse_changes(&s, p)?;
            let mut n = s.with_params(&changes)?;
            // 多重構成の表の行の値なら、表の今の構成の列も
            n.sync_active_config();
            let names: Vec<String> = changes.iter().map(|(q, _)| n.param_name(*q)).collect();
            let rec = changes
                .iter()
                .zip(&names)
                .map(|((q, v), name)| Ok((name.clone(), s.get_param(*q)?, *v)))
                .collect::<Result<Vec<_>, lensbench_core::Error>>()?;
            store_changed(n, spec, Change::Set(rec));
            Ok((json!({ "changed": names }), Vec::new()))
        }
        // 系を .zmx に書き出す（別アプリに渡す）
        "write" => {
            let (s, _) = current()?;
            let path = p["path"]
                .as_str()
                .ok_or_else(|| RpcErr::params("write には path"))?;
            zmx_write::save(&check_path(Path::new(path), true)?, &s)?;
            Ok((json!({ "written": path }), Vec::new()))
        }
        // 系の編集: ops の操作の列を順に当てる（どれかが失敗すれば系は変えない）
        "edit" => {
            let (s, spec) = current()?;
            let mut n = (*s).clone();
            let notes =
                lensbench_project::edit_ops::apply(&mut n, &p["ops"]).map_err(RpcErr::app)?;
            let hash = lensbench_project::sysfile::system_hash(&n).map_err(RpcErr::app)?;
            let count = n.surfaces.len();
            store_changed(n, spec, Change::Edit(p["ops"].clone()));
            Ok((
                json!({ "notes": notes, "surfaces": count, "system_hash": hash }),
                Vec::new(),
            ))
        }
        // 面の値（surface は番号か名前。省略で全部の面と、玉・群の面の範囲 units）
        "surface" => {
            let (s, _) = current()?;
            let one = |j: usize| {
                lensbench_project::edit_ops::surf_fields_json(&s, j).map_err(RpcErr::app)
            };
            match p.get("surface") {
                None | Some(Value::Null) => {
                    let all = (0..s.surfaces.len())
                        .map(one)
                        .collect::<Result<Vec<_>, _>>()?;
                    // 玉・群（ブロック）の面の範囲も（GUI の面の表の階層）
                    // 玉・群の偏心・傾き（motion: dx dy tx ty tz）・移動 shift・媒質の dn・disp（面ごとに違えば null）
                    let units: Vec<Value> = s
                        .units
                        .iter()
                        .enumerate()
                        .map(|(i, u)| {
                            let get =
                                |k| s.get_param(lensbench_core::param::Param::unit(i, k)).ok();
                            json!({
                                "name": u.name, "spans": u.spans, "auto": u.auto,
                                "motion": u.motion, "shift": u.shift,
                                "dn": get(lensbench_core::units::UnitKind::Dn),
                                "disp": get(lensbench_core::units::UnitKind::Disp),
                            })
                        })
                        .collect();
                    Ok((json!({ "surfaces": all, "units": units }), Vec::new()))
                }
                Some(v) => {
                    let j = match v {
                        Value::String(t) => s.find_surface(t)?,
                        _ => s.find_surface(&v.to_string())?,
                    };
                    Ok((json!({ "surface": j, "fields": one(j)? }), Vec::new()))
                }
            }
        }
        // 面のデータを全部の項目で（set_surface_data と同じ形）と、その面のソルブ・ピックアップ（GUI の面の設定）
        "surface_data" => {
            let (s, _) = current()?;
            let j = match p.get("surface") {
                Some(Value::String(t)) => s.find_surface(t)?,
                Some(v) => v
                    .as_u64()
                    .map(|x| x as usize)
                    .ok_or_else(|| RpcErr::params("surface は面の番号か名前"))?,
                None => return Err(RpcErr::params("surface_data には surface")),
            };
            let v = lensbench_project::edit_ops::surface_data(&s, j).map_err(RpcErr::app)?;
            Ok((v, Vec::new()))
        }
        // 多重構成の表: 構成の数 n・今の構成 active・行（パラメータの名前と構成ごとの値）。
        // .zmx なら、ファイルの多重構成の数 zmx_cfgs と読んだ構成 zmx_cfg も
        // 系の設定: 波長・視野・開口・エイミング・環境・GLRS・ROPD・膜ファイルなど（edit の set_system_data と同じ形）
        "system_data" => {
            let (s, _) = current()?;
            let v = lensbench_project::edit_ops::system_data(&s).map_err(RpcErr::app)?;
            Ok((json!({ "data": v }), Vec::new()))
        }
        "config_table" => {
            let (s, spec) = current()?;
            let rows: Vec<Value> = s
                .configs()
                .map(|c| {
                    c.rows
                        .iter()
                        .map(|r| json!({ "param": s.param_name(r.param), "values": r.values }))
                        .collect()
                })
                .unwrap_or_default();
            let (n, active) = s.configs().map_or((1, 1), |c| (c.n, c.active));
            let zmx_cfgs = if is_zmx(&spec.file) {
                let txt = lensbench_core::glass::read_text(&spec.file)
                    .map_err(|x| RpcErr::app(format!("{}: {x}", spec.file.display())))?;
                lensbench_core::zmx::parse_text(&txt)
                    .map_err(|x| RpcErr::app(x.to_string()))?
                    .configurations()
            } else {
                1
            };
            Ok((
                json!({
                    "n": n, "active": active, "rows": rows,
                    "zmx_cfgs": zmx_cfgs, "zmx_cfg": spec.cfg,
                }),
                Vec::new(),
            ))
        }
        // .zmx の多重構成を表にする edit の操作: 各構成を読み、構成で違うパラメータを行にする。
        // 表にできない違い（硝材・無視・面の数・半径以外の値）は notes に
        "config_diff" => {
            let (_, spec) = current()?;
            let v = config_diff(&spec)?;
            Ok((v, Vec::new()))
        }
        // 系ファイルに保存する（編集・set の後の系をそのまま）
        "save" => {
            let (s, _) = current()?;
            let path = p["path"]
                .as_str()
                .ok_or_else(|| RpcErr::params("save には path"))?;
            lensbench_project::sysfile::save(&check_path(Path::new(path), true)?, &s, &[])
                .map_err(RpcErr::app)?;
            let hash = lensbench_project::sysfile::system_hash(&s).map_err(RpcErr::app)?;
            Ok((json!({ "written": path, "system_hash": hash }), Vec::new()))
        }
        "analyze" => analyze(p).map(|v| (v, Vec::new())),
        // カタログを探す一覧（GUI のカタログブラウザ）
        "glasses" | "lenses" | "catalog_search" => browse_of(method, p).map(|v| (v, Vec::new())),
        "trace" => trace(p, bins),
        // 解析・検出器のプラグイン
        "plugins" => Ok((crate::plugins_json(), Vec::new())),
        // ユーザーカタログ: 読み込んだプロジェクトのブロック・範囲を項目に保存、一覧、名前・説明の変更と削除
        "catalog_add" | "catalog_items" | "catalog_edit" => {
            user_catalog(method, p).map(|v| (v, Vec::new()))
        }
        // 解析のサブコマンドの引数の一覧（名前・型・既定・説明。GUI の設定欄）
        "analysis_args" => {
            let cmd = p["cmd"]
                .as_str()
                .ok_or_else(|| RpcErr::params("analysis_args には cmd"))?;
            let args = crate::arg_spec::analysis_args(cmd)
                .ok_or_else(|| RpcErr::params(format!("サブコマンド {cmd} はない")))?;
            Ok((json!({ "cmd": cmd, "args": args }), Vec::new()))
        }
        "plugin" => plugin(p, bins),
        "layout" => layout(p).map(|v| (v, Vec::new())),
        // レイアウトをファイルに書き出す（GUI の書き出し。形式は拡張子で、CLI の layout -o と同じ）
        "layout_export" => layout_export(p).map(|v| (v, Vec::new())),
        "configs" => configs().map(|v| (v, Vec::new())),
        // 減衰最小二乗の最適化（GUI の最適化パネル）
        "optimize" => optimize(p).map(|v| (v, Vec::new())),
        // カタログレンズの差し替え探索（GUI）
        "swap" => swap(p).map(|v| (v, Vec::new())),
        // モンテカルロ公差
        "tolerance" => tolerance(p).map(|v| (v, Vec::new())),
        "" => Err(RpcErr {
            code: -32600,
            msg: "method がない".into(),
        }),
        m => Err(RpcErr {
            code: -32601,
            msg: format!("method {m} はない"),
        }),
    }
}

/// set の値: {"values": {"3.t": 10.0}} か {"values": [["3.t", 10.0], ...]}
fn parse_changes(
    s: &System,
    p: &Value,
) -> Result<Vec<(lensbench_core::param::Param, f64)>, RpcErr> {
    let bad = || RpcErr::params("set には values: {\"3.t\": 10.0} か [[\"3.t\", 10.0]]");
    let pairs: Vec<(&str, f64)> = match p.get("values") {
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, v)| Ok((k.as_str(), v.as_f64().ok_or_else(bad)?)))
            .collect::<Result<_, RpcErr>>()?,
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                let k = x.get(0).and_then(Value::as_str).ok_or_else(bad)?;
                let v = x.get(1).and_then(Value::as_f64).ok_or_else(bad)?;
                Ok((k, v))
            })
            .collect::<Result<_, RpcErr>>()?,
        _ => return Err(bad()),
    };
    pairs
        .into_iter()
        .map(|(k, v)| Ok((s.param(k)?, v)))
        .collect()
}

/// 解析の引数（JSON）を CLI の引数の並びにする。キーの _ は - に、true は旗、false・null は省く、
/// 数の並びはカンマでつなぎ、文字列の並びは旗を繰り返す（layout の out など）
pub(crate) fn analysis_argv(p: &Map<String, Value>, file: &Path) -> Result<Vec<String>, String> {
    let cmd = p
        .get("cmd")
        .and_then(Value::as_str)
        .ok_or("analyze には cmd（spot・wavefront・mtf などのサブコマンド）")?;
    let mut v = vec![
        "lensbench".to_string(),
        cmd.to_string(),
        file.display().to_string(),
    ];
    for (k, x) in p {
        if k == "cmd" {
            continue;
        }
        if matches!(
            k.as_str(),
            "file"
                | "cfg"
                | "state"
                | "path"
                | "glass_dir"
                | "lens_catalog"
                | "glass-dir"
                | "lens-catalog"
        ) {
            return Err(format!("{k} は analyze では変えられない（load で選ぶ）"));
        }
        let flag = format!("--{}", k.replace('_', "-"));
        let scalar = |x: &Value| -> Result<String, String> {
            match x {
                Value::Number(n) => Ok(n.to_string()),
                Value::String(s) => Ok(s.clone()),
                _ => Err(format!("{k} の値 {x}")),
            }
        };
        match x {
            // 設定のオブジェクト（センサの設定など）は JSON の文字列のまま `--<名前>-json` に
            Value::Object(_) => {
                v.push(format!("{flag}-json"));
                v.push(x.to_string());
            }
            Value::Bool(true) => v.push(flag),
            Value::Bool(false) | Value::Null => {}
            Value::Array(a) if a.iter().all(Value::is_number) => {
                let s: Vec<String> = a.iter().map(|n| n.to_string()).collect();
                v.push(flag);
                v.push(s.join(","));
            }
            Value::Array(a) => {
                for e in a {
                    v.push(flag.clone());
                    v.push(scalar(e)?);
                }
            }
            _ => {
                v.push(flag);
                v.push(scalar(x)?);
            }
        }
    }
    Ok(v)
}

/// CLI の解析のサブコマンドを、読み込んだ系で実行する
fn analyze(p: &Value) -> Result<Value, RpcErr> {
    let (_, spec) = current()?;
    let m = p
        .as_object()
        .ok_or_else(|| RpcErr::params("params はオブジェクト"))?;
    let argv = analysis_argv(m, &spec.file).map_err(RpcErr::params)?;
    let cli = crate::parse_cli(&argv).map_err(|e| RpcErr::params(e.render().to_string()))?;
    let mut cmd = cli.cmd;
    if let Cmd::Fringe(a) = &mut cmd {
        return interference_fringe(a, &spec);
    }
    let is_ext = matches!(cmd, Cmd::Ext(_));
    match cmd.common_mut() {
        Some(c) => {
            // 系の読み方は load のもの（解析が系のファイルを読み直す所、info の状態・経路の一覧などでも同じに読む）
            c.glass_dir = spec.glass_dir.clone();
            c.lens_catalog = spec.lens_catalog.clone();
            c.cfg = spec.cfg;
            c.state = spec.state.clone();
            c.path = spec.path.clone();
        }
        // 拡張のサブコマンドは系の読み方を Ctx から自分で読んでもよい
        None if is_ext => {}
        None => return Err(RpcErr::params(format!("{} は系の解析ではない", argv[1]))),
    }
    // 解析が読み書きするファイル（--save・--out など）も要求で与えたパスなので確かめる
    for (q, write) in cmd.paths_mut() {
        *q = check_path(q, write)?;
    }
    let srv = ServeCtx {
        spec: spec.clone(),
        log: current_log(),
    };
    let ctx = ext::Ctx { serve: Some(&srv) };
    let (_, mut out) = execute_in(cmd, &ctx)?;
    if argv[1] == "info" {
        out["state"] = json!(spec.state);
    }
    Ok(out)
}

/// 拡張のサブコマンドに渡す、読み込んだ系（[`ext::Served`]）
struct ServeCtx {
    spec: LoadSpec,
    log: Vec<Change>,
}

impl ext::Served for ServeCtx {
    fn common(&self) -> Common {
        self.spec.common()
    }
    fn check_path(&self, p: &Path, write: bool) -> Result<PathBuf, String> {
        check_path(p, write).map_err(|e| e.msg)
    }
    fn replay(&self, s: System, label: &str, notes: &mut Vec<String>) -> Result<System, String> {
        replay(s, &self.log, label, notes)
    }
}

/// 単色の干渉縞（`fringe`）。光束の構成・状態はファイルから読み、読み込んだ後の変更（set・edit）を当てる
fn interference_fringe(a: &mut FringeArgs, spec: &LoadSpec) -> Result<Value, RpcErr> {
    a.glass_dir = spec.glass_dir.clone();
    a.lens_catalog = spec.lens_catalog.clone();
    a.path = spec.path.clone();
    if let Some(q) = &mut a.out {
        *q = check_path(q, true)?;
    }
    let log = current_log();
    let notes = std::cell::RefCell::new(Vec::new());
    let a: &FringeArgs = a;
    let project = is_project(&spec.file);
    let load = |b: &str| {
        let s = crate::load_beam(a, b)?;
        let beam = if project {
            format!("状態 {b}")
        } else {
            format!("構成 {b}")
        };
        replay(s, &log, &beam, &mut notes.borrow_mut())
    };
    let mut out = crate::fringe_cmd::run(a, &load)?;
    out["notes"] = json!(notes.into_inner());
    Ok(out)
}

fn is_zmx(f: &Path) -> bool {
    f.extension().is_some_and(|x| x.eq_ignore_ascii_case("zmx"))
}

/// .zmx の多重構成を、表（set_config_count・set_config_row・set_config の edit の操作）にする
fn config_diff(spec: &LoadSpec) -> Result<Value, RpcErr> {
    use lensbench_core::param::{Kind, Param};
    if !is_zmx(&spec.file) {
        return Err(RpcErr::params("config_diff は .zmx の系だけ"));
    }
    let txt = lensbench_core::glass::read_text(&spec.file)
        .map_err(|x| RpcErr::app(format!("{}: {x}", spec.file.display())))?;
    let n = lensbench_core::zmx::parse_text(&txt)
        .map_err(|x| RpcErr::app(x.to_string()))?
        .configurations();
    if n < 2 {
        return Err(RpcErr::app("この .zmx は多重構成でない"));
    }
    let mut systems = Vec::with_capacity(n);
    for k in 1..=n {
        let mut c = spec.common();
        c.cfg = k;
        systems.push(load_file(&c).map_err(|e| RpcErr::app(format!("構成 {k}: {e}")))?);
    }
    let m = systems[0].surfaces.len();
    if systems.iter().any(|s| s.surfaces.len() != m) {
        return Err(RpcErr::app("構成で面の数が違う（表にできない）"));
    }
    let mut kinds = vec![Kind::Curv, Kind::Conic, Kind::Thick];
    kinds.extend((0..8).map(Kind::Asph));
    kinds.extend([
        Kind::Dx,
        Kind::Dy,
        Kind::Tx,
        Kind::Ty,
        Kind::Tz,
        Kind::Dn,
        Kind::Disp,
    ]);
    let mut ops = vec![json!({ "op": "set_config_count", "n": n })];
    let mut notes = Vec::new();
    for j in 0..m {
        for &kind in &kinds {
            let p = Param::new(j, kind);
            let vals: Option<Vec<f64>> = systems.iter().map(|s| s.get_param(p).ok()).collect();
            let Some(vals) = vals else { continue };
            if vals.iter().any(|v| v.to_bits() != vals[0].to_bits()) {
                ops.push(json!({
                    "op": "set_config_row", "param": systems[0].param_name(p), "values": vals,
                }));
            }
        }
        let differs = |f: &dyn Fn(&System) -> String| {
            let a = f(&systems[0]);
            systems.iter().any(|s| f(s) != a)
        };
        if differs(&|s| s.surfaces[j].mat.clone()) {
            notes.push(format!("面 {j}: 硝材が構成で違う（表にできない）"));
        }
        if differs(&|s| s.surfaces[j].ignore.to_string()) {
            notes.push(format!("面 {j}: 無視が構成で違う（表にできない）"));
        }
        if differs(&|s| format!("{:?}", s.surfaces[j].semi)) {
            notes.push(format!("面 {j}: 半径が構成で違う（表にできない）"));
        }
        if differs(&|s| s.surfaces[j].stop.to_string()) {
            notes.push(format!("面 {j}: 絞りが構成で違う（表にできない）"));
        }
    }
    let differs_sys = |f: &dyn Fn(&System) -> String| {
        let a = f(&systems[0]);
        systems.iter().any(|s| f(s) != a)
    };
    if differs_sys(&|s| format!("{:?}", s.wavelengths)) {
        notes.push("波長が構成で違う（表にできない）".into());
    }
    if differs_sys(&|s| format!("{:?}", s.fields)) {
        notes.push("視野が構成で違う（表にできない）".into());
    }
    ops.push(json!({ "op": "set_config", "k": spec.cfg.clamp(1, n) }));
    Ok(json!({ "ops": ops, "notes": notes, "rows": ops.len() - 2 }))
}

/// ユーザーカタログの項目の一覧（名前・説明・ブロックの数）
fn catalog_listing(path: &str, cat: &lensbench_project::UserCatalog) -> Value {
    let items: Vec<Value> = cat
        .items
        .iter()
        .map(|(k, it)| {
            json!({ "name": k, "description": it.description, "glass_catalogs": it.glass_catalogs,
                    "blocks": it.blocks.len() })
        })
        .collect();
    json!({ "catalog": path, "vendor": cat.vendor, "items": items })
}

/// ユーザーカタログの操作（GUI の保存）
/// - `catalog_add` {catalog, name, blocks: [id…]（1 つか範囲の最初と最後）, description?, replace?, vendor?}:
///   読み込んだプロジェクト（状態を当てて）のブロックを項目にする。面の表の変更は焼き込む。ファイルが無ければ作る
/// - `catalog_items` {catalog}: 項目の一覧
/// - `catalog_edit` {catalog, name, rename?, description?, delete?, vendor?}
fn user_catalog(method: &str, p: &Value) -> Result<Value, RpcErr> {
    use lensbench_project::UserCatalog;
    let path = p["catalog"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            RpcErr::params(format!(
                "{method} には catalog（ユーザーカタログのファイル）"
            ))
        })?;
    let file = check_path(Path::new(path), method != "catalog_items")?;
    let name = || {
        p["name"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RpcErr::params(format!("{method} には name（項目の名前）")))
    };
    let set_vendor = |cat: &mut UserCatalog| {
        if let Some(v) = p["vendor"].as_str() {
            cat.vendor = v.trim().to_string();
        }
    };
    match method {
        "catalog_items" => {
            let cat = UserCatalog::load(&file)?;
            Ok(catalog_listing(path, &cat))
        }
        "catalog_add" => {
            let name = name()?;
            let ids: Vec<String> = match (&p["blocks"], &p["block"]) {
                (Value::Array(a), _) => a
                    .iter()
                    .map(|x| x.as_str().map(str::to_string))
                    .collect::<Option<_>>()
                    .ok_or_else(|| RpcErr::params("blocks は id の並び"))?,
                (_, Value::String(b)) => vec![b.clone()],
                _ => {
                    return Err(RpcErr::params(
                        "catalog_add には blocks（ブロックの id の並び）",
                    ))
                }
            };
            let (_, spec) = current()?;
            if !is_project(&spec.file) || lensbench_project::sysfile::is_system_file(&spec.file) {
                return Err(RpcErr::app(
                    "catalog_add はプロジェクトファイル（組合せ）を読み込んだときだけ",
                ));
            }
            let mut cat = UserCatalog::load_or_new(&file)?;
            if cat.items.contains_key(name) && p["replace"].as_bool() != Some(true) {
                return Err(RpcErr::app(format!(
                    "項目 {name} はもうある（置き換えは replace: true）"
                )));
            }
            let txt = lensbench_core::vfs::read_to_string(&spec.file)
                .map_err(|x| RpcErr::app(format!("{}: {x}", spec.file.display())))?;
            let mut pr = lensbench_project::Project::from_json(&txt)?;
            if let Some(st) = &spec.state {
                pr = pr.with_state(st)?;
            }
            let lc = lensbench_core::catalog::LensCatalog::load_registered(&spec.lens_catalog)?;
            let look = |v: &str, q: &str| lc.zmx(v, q);
            let env = lensbench_project::Env {
                base: spec
                    .file
                    .parent()
                    .map(|x| x.to_path_buf())
                    .unwrap_or_default(),
                glass_dirs: spec.glass_dir.clone(),
                catalog: Some(&look),
            };
            let desc = p["description"].as_str().unwrap_or_default();
            let item = lensbench_project::blocks_item(&pr, &env, &ids, desc)?;
            cat.items.insert(name.to_string(), item);
            set_vendor(&mut cat);
            cat.save(&file)?;
            let mut out = catalog_listing(path, &cat);
            out["added"] = json!(name);
            Ok(out)
        }
        _ => {
            let name = name()?;
            let mut cat = UserCatalog::load(&file)?;
            if !cat.items.contains_key(name) {
                return Err(RpcErr::app(format!("項目 {name} はない")));
            }
            if p["delete"].as_bool() == Some(true) {
                cat.items.remove(name);
            } else {
                if let Some(d) = p["description"].as_str() {
                    cat.items.get_mut(name).unwrap().description = d.to_string();
                }
                if let Some(to) = p["rename"].as_str().map(str::trim) {
                    if to.is_empty() {
                        return Err(RpcErr::params("rename の名前が空"));
                    }
                    if to != name {
                        if cat.items.contains_key(to) {
                            return Err(RpcErr::app(format!("項目 {to} はもうある")));
                        }
                        let it = cat.items.remove(name).unwrap();
                        cat.items.insert(to.to_string(), it);
                    }
                }
            }
            set_vendor(&mut cat);
            cat.save(&file)?;
            Ok(catalog_listing(path, &cat))
        }
    }
}

fn is_project(f: &Path) -> bool {
    f.extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
}

/// 配列を JSON か bin から読む
fn floats(p: &Value, bins: &mut HashMap<String, Bin>, k: &str) -> Result<Option<Vec<f64>>, RpcErr> {
    if let Some(b) = bins.remove(k) {
        return match b.data {
            Data::F64(v) => Ok(Some(v)),
            _ => Err(RpcErr::params(format!("{k} は <f8"))),
        };
    }
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                x.as_f64()
                    .ok_or_else(|| RpcErr::params(format!("{k} は数の並び")))
            })
            .collect::<Result<_, _>>()
            .map(Some),
        _ => Err(RpcErr::params(format!("{k} は数の並び"))),
    }
}

/// 番号の並び（1 始まり）を 0 始まりに。省略は None
fn opt_index_list(p: &Value, k: &str, n: usize) -> Result<Option<Vec<usize>>, RpcErr> {
    let v = match p.get(k) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(a)) => a.clone(),
        Some(x) => vec![x.clone()],
    };
    v.iter()
        .map(|x| match x.as_u64() {
            Some(i) if i >= 1 && i as usize <= n => Ok(i as usize - 1),
            _ => Err(RpcErr::params(format!("{k} は 1〜{n} の番号"))),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// 断面図（YZ）の図形と、玉・群・面の位置（GUI のレイアウト図）。座標は [z, y] mm（大域座標）
///
/// `cfgs`・`states`・`paths`（並びか "all"）を与えると、その構成（.zmx の多重構成・プロジェクトの状態・BS の経路）を
/// ファイルから読み直して重ねる。構成 k は `offset` [dx, dy, dz] の k 倍ずらす。`set` で変えた値は入らない。
/// `surfaces`・`units` は読み込んだ系のもの
fn layout(p: &Value) -> Result<Value, RpcErr> {
    let (s, spec) = current()?;
    layout_with(&s, &spec, p)
}

fn layout_with(s: &System, spec: &LoadSpec, p: &Value) -> Result<Value, RpcErr> {
    let sc = layout_scene(s, spec, p)?;
    view_json(s, &sc, p)
}

/// layout_export: {"file": 書き出すファイル, layout と同じ引数}。2D（svg・png・dxf）は plane の面
/// （"3d" なら yz）、3D（gltf・glb・obj・stl・step）は 3D の場面（unfold は無視）
fn layout_export(p: &Value) -> Result<Value, RpcErr> {
    let file = p["file"]
        .as_str()
        .ok_or_else(|| RpcErr::params("layout_export には file"))?;
    let path = check_path(Path::new(file), true)?;
    let f = lensbench_export::LayoutFormat::from_path(&path).ok_or_else(|| {
        RpcErr::params(format!(
            "{file}: 拡張子は svg・png・dxf・gltf・glb・obj・stl・step・stp のどれか"
        ))
    })?;
    let mut q = p.clone();
    if let Some(m) = q.as_object_mut() {
        if f.is_2d() {
            if is_3d(p) {
                m.insert("plane".into(), json!("yz"));
            }
        } else {
            m.insert("plane".into(), json!("3d"));
            m.remove("unfold");
        }
    }
    let (s, spec) = current()?;
    let sc = layout_scene(&s, &spec, &q)?;
    let o2 = lensbench_export::d2::Options2D {
        plane: if f.is_2d() { plane_of(&q)? } else { Plane::YZ },
        ..Default::default()
    };
    lensbench_export::write_layout(&path, &sc, &o2, &Default::default()).map_err(RpcErr::app)?;
    Ok(json!({
        "written": path.display().to_string(),
        "format": format!("{f:?}").to_lowercase(),
        "solids": sc.solids.len(),
        "mechanics": sc.mechs.len(),
        "rays": sc.rays.len(),
    }))
}

/// layout・layout_export の場面（構成を選べば重ねる）
fn layout_scene(
    s: &System,
    spec: &LoadSpec,
    p: &Value,
) -> Result<lensbench_export::scene::Scene, RpcErr> {
    let sel = config_sel(p)?;
    if !sel.any() {
        return Ok(lensbench_export::scene::build(s, &scene_opts(s, p)?)?);
    }
    let systems = crate::overlay_systems(&spec.common(), &sel)?;
    let offset = match p.get("offset") {
        None | Some(Value::Null) => [0.0; 3],
        Some(v) => {
            let a: Vec<f64> = v
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default();
            if a.len() != 3 {
                return Err(RpcErr::params("offset は [dx, dy, dz]"));
            }
            [a[0], a[1], a[2]]
        }
    };
    use lensbench_export::scene;
    let mut opt = scene_opts(s, p)?;
    // 描き始める面をそろえる（物体面から描けない構成があれば全部面 1 から。CLI の layout と同じ）
    if opt.first.is_none() && systems.iter().any(|x| scene::auto_first(&x.1) != 0) {
        opt.first = Some(1);
    }
    let mut parts = Vec::with_capacity(systems.len());
    for (name, sy) in &systems {
        let sc = scene::build(sy, &opt).map_err(|x| RpcErr::app(format!("{name}: {x}")))?;
        parts.push((name.clone(), sc));
    }
    Ok(scene::overlay(parts, offset)?)
}

/// layout の構成の選び方（文字列か並び。"all" は全部）
fn config_sel(p: &Value) -> Result<crate::ConfigSel, RpcErr> {
    let one = |k: &str| -> Result<Option<String>, RpcErr> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(x)) => Ok(Some(x.clone())),
            Some(Value::Number(x)) => Ok(Some(x.to_string())),
            Some(Value::Array(a)) if !a.is_empty() => a
                .iter()
                .map(|x| match x {
                    Value::String(t) => Ok(t.clone()),
                    Value::Number(n) => Ok(n.to_string()),
                    _ => Err(RpcErr::params(format!("{k} は文字列か番号の並び"))),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|v| Some(v.join(","))),
            _ => Err(RpcErr::params(format!(
                "{k} は文字列か番号の並び（\"all\" で全部）"
            ))),
        }
    };
    Ok(crate::ConfigSel {
        cfgs: one("cfgs")?,
        states: one("states")?,
        paths: one("paths")?,
    })
}

/// 重ねて描ける構成: .zmx は構成の数、プロジェクトは状態の名前と BS の経路（今の状態で）
fn configs() -> Result<Value, RpcErr> {
    let (_, spec) = current()?;
    let is_json = spec
        .file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"));
    if lensbench_project::sysfile::is_system_file(&spec.file) || !is_json {
        let n = if is_json {
            1
        } else {
            let txt = lensbench_core::glass::read_text(&spec.file)
                .map_err(|x| RpcErr::app(format!("{}: {x}", spec.file.display())))?;
            lensbench_core::zmx::parse_text(&txt)
                .map_err(|x| RpcErr::app(x.to_string()))?
                .configurations()
        };
        return Ok(json!({ "cfgs": n, "cfg": spec.cfg, "states": [], "paths": [] }));
    }
    let txt = lensbench_core::vfs::read_to_string(&spec.file)
        .map_err(|x| RpcErr::app(format!("{}: {x}", spec.file.display())))?;
    let p = lensbench_project::Project::from_json(&txt)?;
    let ps = match &spec.state {
        Some(n) => p.with_state(n)?,
        None => p.clone(),
    };
    let cat = lensbench_core::catalog::LensCatalog::load(
        &spec.lens_catalog,
        &[&lensbench_core::catalog::ZmxFileSource],
    )?;
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let env = lensbench_project::Env {
        base: spec
            .file
            .parent()
            .map(|x| x.to_path_buf())
            .unwrap_or_default(),
        glass_dirs: spec.glass_dir.clone(),
        catalog: Some(&look),
    };
    let paths = lensbench_project::paths(&ps, &env)?;
    Ok(json!({
        "cfgs": 1,
        "states": p.state_names(),
        "state": spec.state,
        "paths": paths,
        "path": spec.path,
    }))
}

fn scene_opts(s: &System, p: &Value) -> Result<lensbench_export::scene::Options, RpcErr> {
    use lensbench_export::scene;
    // 光線の扇の既定は見る面の向き（xz 面なら x の扇）
    // 3D は光束の形が分かる円周（CLI の 3D の書き出しと同じ）
    let d3 = is_3d(p);
    let fan: scene::Fan = match p.get("fan").and_then(Value::as_str) {
        Some(f) => f.parse().map_err(RpcErr::params)?,
        None if d3 => scene::Fan::Ring,
        None if plane_of(p)? == Plane::XZ => scene::Fan::X,
        None => scene::Fan::Y,
    };
    let unfold = match p.get("unfold") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(RpcErr::params("unfold は true か false")),
    };
    if unfold && d3 {
        return Err(RpcErr::params("unfold は 2D（plane が yz・xz）だけ"));
    }
    let n0 = if fan == scene::Fan::Ring { 12 } else { 7 };
    Ok(scene::Options {
        fields: opt_index_list(p, "field", s.fields.len())?,
        waves: opt_index_list(p, "wave", s.wavelengths.len())?,
        fan,
        rays: opt_usize(p, "rays")?.unwrap_or(n0).clamp(1, 200),
        first: opt_usize(p, "first")?,
        last: opt_usize(p, "last")?,
        unfold,
    })
}

/// 見る面に合わせて、場面を断面図（yz・xz）か 3D の網（"3d"）の JSON に
fn view_json(s: &System, sc: &lensbench_export::scene::Scene, p: &Value) -> Result<Value, RpcErr> {
    if is_3d(p) {
        return Ok(scene3d_json(s, sc));
    }
    let mut v = scene_json(s, sc, plane_of(p)?);
    if p["unfold"] == true {
        // 面の頂点も展開した位置に（寸法線・群の位置）
        let maps = lensbench_export::scene::unfold_maps(s);
        let fr = s.frames();
        let xz = v["plane"] == "xz";
        if let Some(a) = v["surfaces"].as_array_mut() {
            for (j, x) in a.iter_mut().enumerate() {
                if let (Some(f), Some(m)) = (fr.surf[j], maps.get(j)) {
                    let q = m.0.point(f.o);
                    x["vertex"] = json!(if xz { [q[2], q[0]] } else { [q[2], q[1]] });
                }
            }
        }
        v["unfold"] = json!(true);
    }
    Ok(v)
}

/// layout の 3D 表示か（plane が "3d"）
fn is_3d(p: &Value) -> bool {
    p["plane"]
        .as_str()
        .is_some_and(|x| x.eq_ignore_ascii_case("3d"))
}

/// layout の見る面（"yz"（既定）か "xz"。"3d" は [`is_3d`]）
fn plane_of(p: &Value) -> Result<Plane, RpcErr> {
    match p.get("plane") {
        None | Some(Value::Null) => Ok(Plane::YZ),
        Some(Value::String(x)) => x.parse().map_err(RpcErr::params),
        Some(_) => Err(RpcErr::params("plane は \"yz\"・\"xz\"・\"3d\"")),
    }
}

/// 場面を GUI の 3D 表示の JSON に: 立体・面・光線の網（座標は大域 [x, y, z] mm を平らに並べたもの）。
/// 立体と面は `surfaces`（面の番号）を持ち、組合せの要素の選択に使う
fn scene3d_json(s: &System, sc: &lensbench_export::scene::Scene) -> Value {
    use lensbench_export::mesh::{self, MeshKind};
    use lensbench_export::scene;
    let o = mesh::Options3D {
        segments: 32,
        rings: 6,
        ..Default::default()
    };
    // 立体と面だけ網に（光線は折れ線で別に）
    let only = scene::Scene {
        name: sc.name.clone(),
        solids: sc.solids.clone(),
        plates: sc.plates.clone(),
        mechs: sc.mechs.clone(),
        rays: Vec::new(),
        fields: sc.fields,
        waves: sc.waves.clone(),
        configs: sc.configs.clone(),
    };
    let ms = mesh::meshes(&only, &o, None);
    let ns = sc.solids.len();
    let np = sc.plates.len();
    // 小数 4 桁（0.1 µm）に丸めて JSON を小さく
    let r4 = |v: f64| (v * 1e4).round() / 1e4;
    let meshes: Vec<Value> = ms
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let (surfs, kind) = if i < ns {
                let k = if m.kind == MeshKind::Mirror {
                    "mirror"
                } else {
                    "glass"
                };
                ([sc.solids[i].front.surf, sc.solids[i].back.surf], k)
            } else if i >= ns + np {
                // 機構部品: 面の番号なし
                ([0, 0], "mech")
            } else {
                let pl = &sc.plates[i - ns];
                let k = match pl.kind {
                    scene::PlateKind::Image => "image",
                    scene::PlateKind::Object => "object",
                    scene::PlateKind::Paraxial => "paraxial",
                };
                ([pl.face.surf, pl.face.surf], k)
            };
            json!({
                "name": m.name,
                "kind": kind,
                "surfaces": surfs,
                "pos": m.pos.iter().flat_map(|p| p.map(r4)).collect::<Vec<_>>(),
                "tris": m.tris.iter().flatten().collect::<Vec<_>>(),
            })
        })
        .collect();
    let rays: Vec<Value> = sc
        .rays
        .iter()
        .filter(|r| r.points.len() >= 2)
        .map(|r| {
            json!({
                "field": r.field + 1, "wave": r.wave + 1, "cfg": r.cfg, "px": r.px, "py": r.py,
                "pts": r.points.iter().map(|q| q.map(r4)).collect::<Vec<_>>(),
            })
        })
        .collect();
    let fr = s.frames();
    let surfaces: Vec<Value> = (0..s.surfaces.len())
        .map(|j| json!({ "vertex": fr.surf[j].map(|f| f.o), "stop": j == s.stop() }))
        .collect();
    let units: Vec<Value> = s
        .units
        .iter()
        .map(|u| json!({ "name": u.name, "spans": u.spans }))
        .collect();
    json!({
        "meshes": meshes, "rays": rays, "surfaces": surfaces, "units": units, "configs": sc.configs,
        "plane": "3d",
    })
}

/// 場面を GUI の断面図の JSON に（座標は [z, y]、xz 面なら [z, x]）
fn scene_json(s: &System, sc: &lensbench_export::scene::Scene, plane: Plane) -> Value {
    use lensbench_export::scene;
    let n = 48;
    let xz = plane == Plane::XZ;
    let pr = |q: [f64; 3]| if xz { [q[2], q[0]] } else { [q[2], q[1]] };
    let solid_pts = |sd: &scene::Solid| {
        if xz {
            scene::solid_section_xz(sd, n)
        } else {
            scene::solid_section(sd, n)
        }
    };
    let face_pts = |f: &scene::Face| {
        if xz {
            scene::face_section_xz(f, n)
        } else {
            scene::face_section(f, n)
        }
    };
    let solids: Vec<Value> = sc
        .solids
        .iter()
        .map(|sd| {
            json!({
                "name": sd.name,
                "kind": match sd.kind { scene::SolidKind::Glass => "glass", scene::SolidKind::Mirror => "mirror" },
                "material": sd.material,
                "surfaces": [sd.front.surf, sd.back.surf],
                "pts": solid_pts(sd).into_iter().map(pr).collect::<Vec<_>>(),
            })
        })
        .collect();
    let plates: Vec<Value> = sc
        .plates
        .iter()
        .map(|pl| {
            json!({
                "name": pl.name,
                "kind": match pl.kind {
                    scene::PlateKind::Image => "image",
                    scene::PlateKind::Object => "object",
                    scene::PlateKind::Paraxial => "paraxial",
                },
                "surface": pl.face.surf,
                "pts": face_pts(&pl.face).into_iter().map(pr).collect::<Vec<_>>(),
            })
        })
        .collect();
    let rays: Vec<Value> = sc
        .rays
        .iter()
        .filter(|r| r.points.len() >= 2)
        .map(|r| {
            json!({
                "field": r.field + 1, "wave": r.wave + 1, "cfg": r.cfg, "px": r.px, "py": r.py, "vig": r.vig, "err": r.err,
                "pts": r.points.iter().map(|&q| pr(q)).collect::<Vec<_>>(),
            })
        })
        .collect();
    // 機構部品: 輪の壁の断面の四角
    let mechs: Vec<Value> = sc
        .mechs
        .iter()
        .map(|m| {
            json!({
                "name": m.name,
                "quads": m.section(xz).into_iter().map(|q| q.into_iter().map(pr).collect::<Vec<_>>()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let fr = s.frames();
    // 面の頂点（無視する面・無限遠の物体面は null）と半径
    let surfaces: Vec<Value> = (0..s.surfaces.len())
        .map(|j| {
            let d = &s.surfaces[j];
            json!({
                "vertex": fr.surf[j].map(|f| pr(f.o)),
                "stop": j == s.stop(),
                "mirror": d.mirror,
                "semi": d.semi,
                "t": d.t,
            })
        })
        .collect();
    let units: Vec<Value> = s
        .units
        .iter()
        .map(|u| json!({ "name": u.name, "spans": u.spans, "motion": u.motion, "shift": u.shift, "auto": u.auto }))
        .collect();
    json!({
        "solids": solids, "plates": plates, "mechs": mechs, "rays": rays, "surfaces": surfaces, "units": units,
        "configs": sc.configs, "plane": if xz { "xz" } else { "yz" },
    })
}

/// JSON の数の並び（入れ子で多次元）を形と平らな並びに
fn nested(
    v: &Value,
    shape: &mut Vec<usize>,
    out: &mut Vec<f64>,
    depth: usize,
) -> Result<(), String> {
    match v {
        Value::Number(n) => {
            if depth != shape.len() {
                return Err("並びの深さがそろわない".into());
            }
            out.push(n.as_f64().ok_or("数")?);
            Ok(())
        }
        Value::Array(a) => {
            if depth == shape.len() {
                if !out.is_empty() {
                    return Err("並びの深さがそろわない".into());
                }
                shape.push(a.len());
            } else if shape[depth] != a.len() {
                return Err("並びの長さがそろわない".into());
            }
            for x in a {
                nested(x, shape, out, depth + 1)?;
            }
            Ok(())
        }
        _ => Err(format!("数の並びでない（{v}）")),
    }
}

/// 解析・検出器のプラグインを 1 回呼ぶ。params: name・config（数・文字列・数の並び）・inputs（JSON の
/// 数の並び、入れ子で多次元。bin でもよい）。系の要るプラグインには読み込んだ系（set・edit の後）と膜を渡す。
/// 結果は bin（<f8・<c16・|b1）、数 1 つの出力は values にも入れる
fn plugin(p: &Value, bins: Vec<Bin>) -> Result<Reply, RpcErr> {
    use lensbench_core::plugin::{self as pl, Array};
    let name = p["name"]
        .as_str()
        .ok_or_else(|| RpcErr::params("plugin には name"))?;
    let mut config = pl::Config::new();
    if let Some(m) = p.get("config").filter(|v| !v.is_null()) {
        let m = m
            .as_object()
            .ok_or_else(|| RpcErr::params("config はオブジェクト"))?;
        for (k, v) in m {
            let x = match v {
                Value::Number(n) => pl::Value::Num(n.as_f64().unwrap_or(f64::NAN)),
                Value::String(t) => pl::Value::Str(t.clone()),
                Value::Bool(b) => pl::Value::Num(*b as u8 as f64),
                Value::Array(a) => pl::Value::Nums(
                    a.iter()
                        .map(|x| x.as_f64())
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| RpcErr::params(format!("config の {k} は数の並び")))?,
                ),
                _ => return Err(RpcErr::params(format!("config の {k} の値 {v}"))),
            };
            config.insert(k.clone(), x);
        }
    }
    let mut inputs = pl::Arrays::new();
    if let Some(m) = p.get("inputs").filter(|v| !v.is_null()) {
        let m = m
            .as_object()
            .ok_or_else(|| RpcErr::params("inputs はオブジェクト"))?;
        for (k, v) in m {
            let (mut shape, mut out) = (Vec::new(), Vec::new());
            nested(v, &mut shape, &mut out, 0)
                .map_err(|e| RpcErr::params(format!("inputs の {k}: {e}")))?;
            inputs.insert(k.clone(), Array::f64(shape, out)?);
        }
    }
    for b in bins {
        let data = match b.data {
            Data::F64(v) => pl::Data::F64(v),
            Data::I32(v) => pl::Data::F64(v.into_iter().map(f64::from).collect()),
            Data::U8(v) => pl::Data::F64(v.into_iter().map(f64::from).collect()),
            Data::C64(v) => pl::Data::C64(v),
            Data::Bool(v) => pl::Data::Bool(v),
        };
        inputs.insert(b.name, Array::new(b.shape, data)?);
    }
    let reg = crate::builtin_plugins();
    let plug = reg.get(name)?;
    let inst = plug.prepare(&config)?;
    let sys = served().as_ref().map(|x| x.sys.clone());
    let coat = match &sys {
        Some(s) if plug.spec().needs_system => {
            Some(lensbench_core::coating::Coatings::for_system(s)?)
        }
        _ => None,
    };
    let ctx = pl::Context {
        system: sys.as_ref(),
        coatings: coat.as_ref(),
    };
    let out = inst.run(&ctx, &inputs)?;
    let mut values = Map::new();
    let mut list = Vec::with_capacity(out.len());
    for (k, a) in out {
        if let pl::Data::F64(v) = &a.data {
            if v.len() == 1 && a.shape.iter().all(|&d| d == 1) {
                values.insert(k.clone(), json!(v[0]));
            }
        }
        let data = match a.data {
            pl::Data::F64(v) => Data::F64(v),
            pl::Data::C64(v) => Data::C64(v),
            pl::Data::Bool(v) => Data::Bool(v),
        };
        list.push(Bin {
            name: k,
            shape: a.shape,
            data,
        });
    }
    Ok((json!({ "name": name, "values": values }), list))
}

/// 正規化瞳座標 (px, py) の光線を追う。結果の配列は最後の面の局所座標（global なら大域）
fn trace(p: &Value, bins: Vec<Bin>) -> Result<Reply, RpcErr> {
    let (s, _) = current()?;
    let mut bins: HashMap<String, Bin> = bins.into_iter().map(|b| (b.name.clone(), b)).collect();
    let px = floats(p, &mut bins, "px")?.ok_or_else(|| RpcErr::params("trace には px"))?;
    let py = floats(p, &mut bins, "py")?.ok_or_else(|| RpcErr::params("trace には py"))?;
    if px.len() != py.len() {
        return Err(RpcErr::params(format!(
            "px と py の長さが違う（{} と {}）",
            px.len(),
            py.len()
        )));
    }
    let w = match opt_usize(p, "wave")? {
        None => s.primary,
        Some(i) if i >= 1 && i <= s.wavelengths.len() => i - 1,
        Some(i) => {
            return Err(RpcErr::params(format!(
                "波長 {i} はありません（1〜{}）",
                s.wavelengths.len()
            )))
        }
    };
    let (hx, hy) = match opt_usize(p, "field")? {
        Some(i) if i >= 1 && i <= s.fields.len() => s.hxhy(i - 1),
        Some(i) => {
            return Err(RpcErr::params(format!(
                "視野 {i} はありません（1〜{}）",
                s.fields.len()
            )))
        }
        None => (
            opt_f64(p, "hx")?.unwrap_or(0.0),
            opt_f64(p, "hy")?.unwrap_or(0.0),
        ),
    };
    let last = opt_usize(p, "last")?;
    if let Some(l) = last {
        if l == 0 || l > s.image() {
            return Err(RpcErr::params(format!("last {l} は 1〜{}", s.image())));
        }
    }
    let global = p.get("global").and_then(Value::as_bool).unwrap_or(false);
    let launch = Launch::Pupil {
        spec: s.object_spec(hx, hy)?,
        signed: false,
        px: &px,
        py: &py,
        vig: Some(s.vig_factors(hx, hy)?),
        hxy: Some((hx, hy)),
    };
    let t = s.trace_launch(&launch, w, last, false);
    let n = t.rays.len();
    let mut c: [Vec<f64>; 7] = std::array::from_fn(|_| Vec::with_capacity(n));
    let mut err = Vec::with_capacity(n);
    let mut vig = Vec::with_capacity(n);
    let mut ok = 0;
    for r in &t.rays {
        let (a, b) = if global { (r.p, r.k) } else { (r.xyz, r.lmn) };
        for i in 0..3 {
            c[i].push(a[i]);
            c[3 + i].push(b[i]);
        }
        c[6].push(r.opl);
        err.push(r.err as i32);
        vig.push(r.vig as i32);
        if r.err == 0 && r.vig == 0 {
            ok += 1;
        }
    }
    let [x, y, z, l, m, nn, opl] = c;
    let out = vec![
        Bin::f64("x", x),
        Bin::f64("y", y),
        Bin::f64("z", z),
        Bin::f64("l", l),
        Bin::f64("m", m),
        Bin::f64("n", nn),
        Bin::f64("opl", opl),
        Bin::i32("err", err),
        Bin::i32("vig", vig),
    ];
    Ok((
        json!({
            "rays": n, "ok": ok, "hx": hx, "hy": hy, "wave": w + 1, "last": t.last,
            "frame": if global { "global" } else { "local" },
        }),
        out,
    ))
}

// ------------------------------------------------------------------------------------------ 最適化

/// 最適化の進み具合（同じプロセスの GUI が読む、[`optimize_progress`]）
struct OptRun {
    /// 反復ごとの評価関数の値（初期値を含む）
    history: Vec<f64>,
}

static OPT_RUN: Mutex<Option<OptRun>> = Mutex::new(None);
static OPT_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 走っている最適化の、反復ごとの評価関数の値（走っていなければ None）。同じプロセスから呼ぶ（GUI の進み具合）
pub fn optimize_progress() -> Option<Vec<f64>> {
    OPT_RUN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|r| r.history.clone())
}

/// 走っている最適化を止める（次の反復の終わりで止まり、そこまでの結果を返す）。同じプロセスから呼ぶ
pub fn optimize_cancel() {
    OPT_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// 構成（.zmx の構成番号か、プロジェクトの状態の名前）の系。読み込んだ系と同じ構成なら読み込んだ系（set・edit の後）、
/// ほかはファイルから読み、読み込んだ後の変更を当てる
fn config_system(
    b: &Value,
    cur: &Arc<System>,
    spec: &LoadSpec,
    notes: &mut Vec<String>,
) -> Result<System, RpcErr> {
    let mut sp = spec.clone();
    if is_project(&spec.file) {
        let name = b
            .as_str()
            .ok_or_else(|| RpcErr::params("configs はプロジェクトの状態の名前の並び"))?;
        if spec.state.as_deref() == Some(name) {
            return Ok((**cur).clone());
        }
        sp.state = Some(name.to_string());
    } else {
        let c = b
            .as_u64()
            .or_else(|| b.as_str().and_then(|x| x.parse().ok()))
            .ok_or_else(|| RpcErr::params("configs は .zmx の構成番号（1 始まり）の並び"))?
            as usize;
        if c == spec.cfg {
            return Ok((**cur).clone());
        }
        sp.cfg = c;
    }
    let s = load_file(&sp.common())?;
    let beam = if is_project(&spec.file) {
        format!("状態 {b}")
    } else {
        format!("構成 {b}")
    };
    Ok(replay(s, &current_log(), &beam, notes)?)
}

/// 最適化。params:
/// - vars: 変数の並び。各要素は "7.t" か {"name": "7.t", "lo": 下限, "hi": 上限, "configs": [構成の番号（1 始まり）]}
/// - merit: 既定の評価関数 "spot"・"wavefront"・null（なし）。rings（3）・reference（"centroid"）
/// - terms: 足す項（`lensbench_project::merit` の JSON）
/// - configs: 構成（.zmx の構成番号か状態の名前）の並び。省けば読み込んだ系だけ。項・変数の構成の番号はこの並びの順
/// - iters（50）・jac（"auto"・"fd"）
/// - apply: true なら最適化した値を読み込んだ系に set する（読み込んだ系の構成に効く変数だけ）
///
/// 同じプロセスからは [`optimize_progress`] で進み具合を読み、[`optimize_cancel`] で止められる
fn optimize(p: &Value) -> Result<Value, RpcErr> {
    use lensbench_core::optim;
    let (cur, spec) = current()?;
    let mut notes = Vec::new();
    let (systems, cur_idx) = match p.get("configs").and_then(Value::as_array) {
        Some(bs) if !bs.is_empty() => {
            let mut v = Vec::new();
            let mut cur_idx = None;
            for b in bs {
                let same = if is_project(&spec.file) {
                    spec.state.as_deref() == b.as_str()
                } else {
                    b.as_u64()
                        .or_else(|| b.as_str().and_then(|x| x.parse().ok()))
                        == Some(spec.cfg as u64)
                };
                if same && cur_idx.is_none() {
                    cur_idx = Some(v.len());
                }
                v.push(config_system(b, &cur, &spec, &mut notes)?);
            }
            (v, cur_idx)
        }
        _ => (vec![(*cur).clone()], Some(0)),
    };
    let mut prob = optim::Problem::new(systems);
    let vars = p["vars"]
        .as_array()
        .ok_or_else(|| RpcErr::params("optimize には vars（変数の並び）"))?;
    for v in vars {
        let (name, lo, hi, cfgs) = match v {
            Value::String(n) => (n.as_str(), None, None, None),
            Value::Object(_) => (
                v["name"]
                    .as_str()
                    .ok_or_else(|| RpcErr::params("vars の要素に name"))?,
                opt_f64(v, "lo")?,
                opt_f64(v, "hi")?,
                match v.get("configs") {
                    None | Some(Value::Null) => None,
                    Some(c) => Some(
                        c.as_array()
                            .ok_or_else(|| RpcErr::params("vars の configs は番号の並び"))?
                            .iter()
                            .map(|x| match x.as_u64() {
                                Some(k) if k >= 1 && (k as usize) <= prob.configs.len() => {
                                    Ok(k as usize - 1)
                                }
                                _ => Err(RpcErr::params(format!(
                                    "vars の configs {x}（1〜{}）",
                                    prob.configs.len()
                                ))),
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                },
            ),
            _ => return Err(RpcErr::params("vars の要素は \"7.t\" か {\"name\": ...}")),
        };
        let first = cfgs.as_ref().and_then(|c| c.first().copied()).unwrap_or(0);
        let param = prob.configs[first].param(name)?;
        let mut var = optim::Var::new(param)
            .bounds(lo.unwrap_or(f64::NEG_INFINITY), hi.unwrap_or(f64::INFINITY));
        if let Some(c) = cfgs {
            var = var.in_configs(c);
        }
        prob.vars.push(var);
    }
    let reference = match p["reference"].as_str().unwrap_or("centroid") {
        "centroid" => lensbench_core::sweep::RmsRef::Centroid,
        "chief" => lensbench_core::sweep::RmsRef::Chief,
        r => {
            return Err(RpcErr::params(format!(
                "reference は centroid か chief（{r}）"
            )))
        }
    };
    let rings = opt_usize(p, "rings")?.unwrap_or(3);
    match p.get("merit").map(|m| m.as_str()) {
        None | Some(Some("spot")) => prob.add_default(optim::DefaultMerit::Spot, rings, reference),
        Some(Some("wavefront")) => {
            prob.add_default(optim::DefaultMerit::Wavefront, rings, reference)
        }
        Some(None) | Some(Some("none")) => {}
        Some(Some(m)) => {
            return Err(RpcErr::params(format!(
                "merit は spot・wavefront・none（{m}）"
            )))
        }
    }
    let n_default = prob.terms.len();
    if let Some(t) = p.get("terms").filter(|t| !t.is_null()) {
        let mut t = t.clone();
        check_term_paths(&mut t)?;
        let ts =
            lensbench_project::merit::terms_from_json(&t, &prob.configs).map_err(RpcErr::params)?;
        prob.terms.extend(ts);
    }
    prob.jac = match p["jac"].as_str().unwrap_or("auto") {
        "auto" => optim::JacMode::Auto,
        "fd" => optim::JacMode::FiniteDiff,
        j => return Err(RpcErr::params(format!("jac は auto か fd（{j}）"))),
    };
    prob.check()?;
    let o = optim::DlsOpts {
        max_iter: opt_usize(p, "iters")?.unwrap_or(50),
        ..Default::default()
    };
    let m0 = prob.merit(&prob.x0()?)?;
    OPT_CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
    *OPT_RUN.lock().unwrap_or_else(|e| e.into_inner()) = Some(OptRun { history: vec![m0] });
    let r = optim::dls_with(&prob, &o, |pr| {
        if let Some(run) = OPT_RUN.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            run.history.push(pr.merit);
        }
        !OPT_CANCEL.load(std::sync::atomic::Ordering::SeqCst)
    });
    *OPT_RUN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let r = r?;
    let names: Vec<String> = prob
        .vars
        .iter()
        .map(|v| prob.configs[0].param_name(v.param))
        .collect();
    let tv0 = prob.term_values(&r.x0)?;
    let tv = prob.term_values(&r.x)?;
    let stop = match r.stop {
        optim::Stop::Converged => "converged",
        optim::Stop::SmallStep => "small_step",
        optim::Stop::NoProgress => "no_progress",
        optim::Stop::MaxIter => "max_iter",
        optim::Stop::Cancelled => "cancelled",
        optim::Stop::NoVars => "no_vars",
    };
    // 読み込んだ系に当てられる変数（読み込んだ系の構成に効くもの）
    let applicable: Vec<bool> = prob
        .vars
        .iter()
        .map(|v| cur_idx.is_some_and(|c| v.configs.as_ref().map_or(true, |cs| cs.contains(&c))))
        .collect();
    let mut applied = false;
    if p["apply"].as_bool() == Some(true) {
        let ch: Vec<(lensbench_core::param::Param, f64)> = prob
            .vars
            .iter()
            .zip(&r.x)
            .zip(&applicable)
            .filter(|(_, &a)| a)
            .map(|((v, &x), _)| (v.param, x))
            .collect();
        if !ch.is_empty() {
            let n = cur.with_params(&ch)?;
            let rec = ch
                .iter()
                .map(|(q, v)| Ok((n.param_name(*q), cur.get_param(*q)?, *v)))
                .collect::<Result<Vec<_>, lensbench_core::Error>>()?;
            store_changed(n, spec, Change::Set(rec));
            applied = true;
        }
    }
    let terms: Vec<Value> = (0..prob.terms.len())
        .map(|i| json!({ "default": i < n_default, "value0": tv0[i], "value": tv[i] }))
        .collect();
    Ok(json!({
        "names": names,
        "x0": r.x0,
        "x": r.x,
        "applicable": applicable,
        "applied": applied,
        "merit0": r.merit0,
        "merit": r.merit,
        "history": r.history,
        "iterations": r.iterations,
        "evals": r.evals,
        "stop": stop,
        "terms": terms,
        "notes": notes,
    }))
}

// ------------------------------------------------------------------------------------ 公差

/// 公差のモンテカルロの進み具合（同じプロセスの GUI が読む、[`tolerance_progress`]）
static TOL_RUN: Mutex<Option<(usize, usize)>> = Mutex::new(None);
static TOL_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 走っている公差のモンテカルロの (終えた評価, 評価の数)（感度の 2 × 公差の数 + 試行の数。走っていなければ None）
pub fn tolerance_progress() -> Option<(usize, usize)> {
    *TOL_RUN.lock().unwrap_or_else(|e| e.into_inner())
}

/// 走っている公差のモンテカルロを止める（走っている試行は終わらせ、そこまでの結果を返す）。同じプロセスから呼ぶ
pub fn tolerance_cancel() {
    TOL_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// モンテカルロ公差。params は `lensbench tolerance --spec` の JSON（`lensbench_project::tolerance`）に加えて
/// configs（構成の番号か状態の名前の並び。optimize と同じ。省けば読み込んだ系だけ）。読み込んだ系（set・edit の後）を使う。
/// 同じプロセスからは [`tolerance_progress`] で進み具合を読み、[`tolerance_cancel`] で止められる
fn tolerance(p: &Value) -> Result<Value, RpcErr> {
    let (cur, spec) = current()?;
    let mut notes = Vec::new();
    let systems = match p.get("configs").and_then(Value::as_array) {
        Some(bs) if !bs.is_empty() => bs
            .iter()
            .map(|b| config_system(b, &cur, &spec, &mut notes))
            .collect::<Result<Vec<_>, _>>()?,
        _ => vec![(*cur).clone()],
    };
    let vendors = if is_project(&spec.file) {
        lensbench_core::vfs::read_to_string(&spec.file)
            .ok()
            .and_then(|t| lensbench_project::Project::from_json(&t).ok())
            .map(|p| lensbench_project::tolerance::catalog_vendors(&p))
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let mut v = p.clone();
    if let Some(o) = v.as_object_mut() {
        o.remove("configs");
        if let Some(t) = o.get_mut("terms") {
            check_term_paths(t)?;
        }
    }
    let ms = lensbench_project::tolerance::spec_from_json(&v, systems, &vendors)
        .map_err(RpcErr::params)?;
    let oo = lensbench_project::tolerance::out_opts(&v).map_err(RpcErr::params)?;
    TOL_CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
    *TOL_RUN.lock().unwrap_or_else(|e| e.into_inner()) = Some((0, 0));
    let run = || {
        lensbench_core::tolerance::run_with(&ms, |k, n| {
            let mut g = TOL_RUN.lock().unwrap_or_else(|e| e.into_inner());
            // 並列の試行は終わる順が前後するので、大きい方を残す
            let k = g.map_or(k, |(d, _)| d.max(k));
            *g = Some((k, n));
            !TOL_CANCEL.load(std::sync::atomic::Ordering::SeqCst)
        })
    };
    let r = match oo.threads {
        Some(t) => lensbench_core::with_threads(Some(t), run),
        None => run(),
    };
    *TOL_RUN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let r = r?;
    let mut out = lensbench_project::tolerance::result_json(&ms, &r, &oo);
    out["notes"] = json!(notes);
    Ok(out)
}

// ------------------------------------------------------------------------------------ 差し替え探索

/// 差し替え探索の進み具合（同じプロセスの GUI が読む、[`swap_progress`]）
static SWAP_RUN: Mutex<Option<(usize, usize)>> = Mutex::new(None);
static SWAP_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 走っている差し替え探索の (終えた組, 組の数)（走っていなければ None）。同じプロセスから呼ぶ
pub fn swap_progress() -> Option<(usize, usize)> {
    *SWAP_RUN.lock().unwrap_or_else(|e| e.into_inner())
}

/// 走っている差し替え探索を止める（走っている組は終わらせ、そこまでの結果を返す）。同じプロセスから呼ぶ
pub fn swap_cancel() {
    SWAP_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// カタログレンズの差し替え探索（GUI）。params は `lensbench swap --spec` の JSON（slots・vars・merit・
/// reference・rings・iters・efl・state・max_count）に加えて:
/// - project: プロジェクトの JSON（省けば読み込んだファイル。読み込んだ後の変更は使わない）
/// - top: 返す組の数（50。0 なら全部）
/// - threads: スレッド数
/// - count_only: true なら回さず、組の数と所ごとの候補だけ返す
///
/// 読み込んだ系のガラス・レンズカタログのフォルダを使う。同じプロセスからは [`swap_progress`] で進み具合を読み、
/// [`swap_cancel`] で止められる
fn swap(p: &Value) -> Result<Value, RpcErr> {
    use lensbench_project::swap::{SwapSearch, SwapSpec};
    let (_, ls) = current()?;
    let proj = match p.get("project").filter(|v| !v.is_null()) {
        // 中の zmx・カタログ・膜のパスを check_path で確かめられないので、要求のファイルを制限するときは断る
        Some(_) if !matches!(FILES.get().unwrap_or(&Files::Any), Files::Any) => {
            return Err(RpcErr::app(
                "swap の project は --allow-remote・--root では使えない（中のパスを確かめられない。読み込んだファイルで回す）",
            ))
        }
        Some(v) => serde_json::from_value::<lensbench_project::Project>(v.clone())
            .map_err(|e| RpcErr::params(format!("project: {e}")))?,
        None => {
            let txt = lensbench_core::vfs::read_to_string(&ls.file)
                .map_err(|e| RpcErr::app(format!("{}: {e}", ls.file.display())))?;
            lensbench_project::Project::from_json(&txt)
                .map_err(|e| RpcErr::app(format!("読み込んだ系がプロジェクトでない（{e}）")))?
        }
    };
    let mut sp = p.clone();
    if let Some(o) = sp.as_object_mut() {
        for k in ["project", "top", "threads", "count_only"] {
            o.remove(k);
        }
        if !o.contains_key("state") && p.get("project").is_none() {
            if let Some(s) = &ls.state {
                o.insert("state".into(), json!(s));
            }
        }
    }
    let spec: SwapSpec = serde_json::from_value(sp)
        .map_err(|e| RpcErr::params(format!("差し替え探索の設定: {e}")))?;
    let cat = lensbench_core::catalog::LensCatalog::load_registered(&ls.lens_catalog)?;
    let env = lensbench_project::Env {
        base: ls
            .file
            .parent()
            .map(|x| x.to_path_buf())
            .unwrap_or_default(),
        glass_dirs: ls.glass_dir.clone(),
        catalog: None,
    };
    let s = SwapSearch::new(&proj, &env, &cat, spec)?;
    let n = s.count();
    let slots: Vec<Value> = s
        .picks
        .iter()
        .map(|v| {
            json!(v
                .iter()
                .map(|k| json!({
                    "vendor": k.lens.vendor, "part": k.lens.part, "reversed": k.reversed,
                    "efl": k.lens.efl, "diameter": k.lens.diameter,
                }))
                .collect::<Vec<_>>())
        })
        .collect();
    if p["count_only"].as_bool() == Some(true) {
        return Ok(json!({ "count": n, "slots": slots }));
    }
    let rows: Mutex<Vec<Value>> = Mutex::new(Vec::new());
    SWAP_CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
    *SWAP_RUN.lock().unwrap_or_else(|e| e.into_inner()) = Some((0, n));
    let show = |pr: lensbench_core::runs::Progress| {
        *SWAP_RUN.lock().unwrap_or_else(|e| e.into_inner()) = Some((pr.done, pr.total));
    };
    let o = lensbench_core::runs::RunOpts {
        threads: opt_usize(p, "threads")?,
        progress: Some(&show),
        cancel: Some(&SWAP_CANCEL),
        header: "{}".into(),
        ..Default::default()
    };
    let t0 = web_time::Instant::now();
    let r = lensbench_core::runs::run(n, 0, &o, |i, _| {
        let v = s.eval(i);
        rows.lock().unwrap_or_else(|e| e.into_inner()).push(v);
        Ok("{}".into())
    });
    *SWAP_RUN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let sum = r?;
    let mut rows = rows.into_inner().unwrap_or_else(|e| e.into_inner());
    lensbench_project::swap::sort_results(&mut rows);
    let done = rows.len();
    let failed = rows.iter().filter(|x| x.get("error").is_some()).count();
    let top = opt_usize(p, "top")?.unwrap_or(50);
    if top > 0 {
        rows.truncate(top);
    }
    Ok(json!({
        "count": n,
        "done": done,
        "failed": failed,
        "cancelled": sum.cancelled,
        "seconds": t0.elapsed().as_secs_f64(),
        "slots": slots,
        "results": rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msg_roundtrip() {
        let bins = vec![
            Bin::f64("a", vec![1.0, -2.5, f64::NAN]),
            Bin::i32("b", vec![3, -4]),
        ];
        let mut buf = Vec::new();
        write_msg(&mut buf, json!({ "id": 1, "method": "echo" }), &bins).unwrap();
        // 空行は飛ばす
        let mut all = b"\n".to_vec();
        all.extend(&buf);
        all.extend(&buf);
        let mut r = std::io::Cursor::new(all);
        for _ in 0..2 {
            let m = read_msg(&mut r, 1 << 20).unwrap().unwrap().unwrap();
            assert_eq!(m.head["method"], "echo");
            assert_eq!(m.bins.len(), 2);
            assert_eq!(m.bins[1], bins[1]);
            match &m.bins[0].data {
                Data::F64(v) => {
                    assert_eq!(v[..2], [1.0, -2.5]);
                    assert!(v[2].is_nan());
                }
                _ => panic!(),
            }
        }
        assert!(read_msg(&mut r, 1 << 20).unwrap().is_none());
    }

    #[test]
    fn bin_size_is_limited() {
        let h = |shape: Value| json!({ "bin": [{ "name": "a", "dtype": "<f8", "shape": shape }] });
        assert_eq!(bin_layout(&h(json!([10, 3])), 240).unwrap()[0].3, 240);
        assert!(bin_layout(&h(json!([10, 3])), 239).is_err());
        // 積があふれる形
        assert!(bin_layout(&h(json!([1u64 << 40, 1u64 << 40])), u64::MAX).is_err());
        // 合計で上限を超える
        let two = json!({ "bin": [
            { "name": "a", "dtype": "|u1", "shape": [100] },
            { "name": "b", "dtype": "|u1", "shape": [100] }
        ] });
        assert!(bin_layout(&two, 150).is_err());
        assert!(bin_layout(&two, 200).is_ok());
        // 読むときは確保の前に断る
        let mut r = std::io::Cursor::new(
            serde_json::to_vec(&h(json!([1u64 << 40])))
                .unwrap()
                .into_iter()
                .chain(*b"\n")
                .collect::<Vec<u8>>(),
        );
        assert!(read_msg(&mut r, 1 << 20).unwrap().unwrap().is_err());
    }

    #[test]
    fn argv_from_json() {
        let p = json!({ "cmd": "mtf", "field": 2, "freq": [10, 20.5], "json": true, "x": false,
                        "out": ["a.svg", "b.step"], "max_freq": 50 });
        let v = analysis_argv(p.as_object().unwrap(), Path::new("s.zmx")).unwrap();
        assert_eq!(
            v,
            [
                "lensbench",
                "mtf",
                "s.zmx",
                "--field",
                "2",
                "--freq",
                "10,20.5",
                "--json",
                "--out",
                "a.svg",
                "--out",
                "b.step",
                "--max-freq",
                "50"
            ]
        );
        for bad in [
            json!({ "cmd": "spot", "cfg": 2 }),
            // path（経路）も load で選ぶ。黙って load の値で上書きされないよう誤りにする
            json!({ "cmd": "info", "path": "A" }),
            json!({ "cmd": "spot", "lens-catalog": "x" }),
        ] {
            let e = analysis_argv(bad.as_object().unwrap(), Path::new("s.zmx")).unwrap_err();
            assert!(e.contains("load で選ぶ"), "{e}");
        }
    }

    /// 設定のオブジェクト（GUI のセンサの欄など）は --<名前>-json の JSON 文字列になる（受けるのは拡張の
    /// サブコマンド。tests/ext.rs）
    #[test]
    fn object_arg_becomes_json_flag() {
        let p = json!({ "cmd": "camera", "sensor": { "mla": { "pitch": [0.2, 0.2] } } });
        let v = analysis_argv(p.as_object().unwrap(), Path::new("s.zmx")).unwrap();
        assert_eq!(v[3..], ["--sensor-json", r#"{"mla":{"pitch":[0.2,0.2]}}"#]);
    }
}
