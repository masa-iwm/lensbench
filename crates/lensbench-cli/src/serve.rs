//! `lensbench serve`: 系を読み込んだまま待ち受け、外から JSON-RPC で系の変更・解析を繰り返す。
//!
//! 毎回の起動・読み込み・前処理を省いて「解析 → 別アプリで系を更新 → 再解析」のループを速くする。
//! 書式は docs/serve.md。要点:
//!
//! - TCP で 1 行 1 要求の JSON-RPC 2.0（改行で終わる JSON）。既定は 127.0.0.1 だけで待ち受ける
//! - 配列は JSON に入れず、ヘッダの `"bin": [{"name", "dtype", "shape"}, ...]` に並べ、改行の直後に
//!   その順で生のバイト列（リトルエンディアン、C 順）を続ける。要求・応答とも同じ形
//! - 要求を処理する場（系・変更の記録）は lensbench-engine の [`Session`]。系は 1 プロセスに 1 つ。要求は 1 つずつ順に処理する（複数の接続も順番待ち）。別の系を並べて回すときは
//!   プロセスを分ける（`--threads 1` で 1 プロセス 1 スレッドにできる）
//! - 解析（`analyze`）は CLI のサブコマンドと同じ引数・同じ結果。系はファイルを読まず、読み込んだ系を使う

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use web_time::Instant;

use clap::Args;
use lensbench_core::coating::C64;
use lensbench_engine::{Bin, CallError, Data, Files, Session, SysSpec};
use serde_json::{json, Value};

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
    let sess = Arc::new(Session::with_files(files));
    let _ = SHM_OK.set(!a.allow_remote);
    let limit = a.max_request_bytes;
    if let Some(f) = &a.file {
        let spec = SysSpec {
            file: f.clone(),
            glass_dir: a.glass_dir.clone(),
            lens_catalog: a.lens_catalog.clone(),
            cfg: a.cfg,
            state: a.state.clone(),
            path: a.path.clone(),
            ..Default::default()
        };
        sess.open(spec, threads)?;
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
            if let Err(e) = connection(&sess, r, w, &*f, threads, limit) {
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
                let sess = sess.clone();
                std::thread::spawn(move || {
                    if let Err(e) = tcp_connection(&sess, c, threads, limit) {
                        eprintln!("lensbench serve: 接続: {e}");
                    }
                });
            }
            Err(e) => eprintln!("lensbench serve: {e}"),
        }
    }
    Ok(())
}

fn tcp_connection(
    sess: &Session,
    c: TcpStream,
    threads: Option<usize>,
    limit: u64,
) -> std::io::Result<()> {
    c.set_nodelay(true).ok();
    let r = BufReader::with_capacity(1 << 16, c.try_clone()?);
    let w = BufWriter::with_capacity(1 << 16, c.try_clone()?);
    connection(sess, r, w, &c, threads, limit)
}

/// 1 本の接続の要求を順に処理する（TCP・パイプで共通）
fn connection(
    sess: &Session,
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
            Some(e) => Err(CallError {
                code: -32602,
                message: e,
                kind: None,
            }),
            None => sess.handle(&method, &params, m.bins, threads),
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
                let mut er = json!({ "code": e.code, "message": e.message });
                if let Some(k) = e.kind {
                    er["data"] = json!({ "kind": k });
                }
                let h = json!({ "jsonrpc": "2.0", "id": id, "error": er });
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
}
