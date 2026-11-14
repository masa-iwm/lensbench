//! serve の場（[`Session`]）: 系を読み込んだまま、JSON の要求（method と params）で系の変更・解析を繰り返す。
//!
//! 書式は docs/serve.md。要求を運ぶ道（TCP・パイプ・共有メモリ、1 行 1 要求の JSON-RPC）は CLI の `serve` が持ち、
//! 同じプロセスの GUI・Web 版は [`Session::call`] を直接呼ぶ。要点:
//!
//! - 系は場に 1 つ。要求は場ごとに 1 つずつ順に処理する
//! - 解析（`analyze`）は解析の登録簿の解析で、CLI のサブコマンドと同じ設定・同じ結果。系はファイルを読まず、読み込んだ系を使う

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use web_time::Instant;

use lensbench_core::coating::C64;
use lensbench_core::system::Launch;
use lensbench_core::{zmx_write, System};
use lensbench_export::d2::Plane;
use serde_json::{json, Map, Value};

use crate::load::{load_file, SysSpec as LoadSpec};

/// 要求で与えたパスの扱い
pub enum Files {
    /// そのまま読み書きする（localhost だけで待ち受け、--root なし）
    Any,
    /// このフォルダ（正規化済み）の下だけ
    Root(PathBuf),
    /// 読み書きしない（--allow-remote で --root なし）
    Deny,
}

/// 評価関数の項（1 つか並び）の zplm のファイル・フォルダを [`Files::check_path`] で確かめる。ファイルを限っているとき
/// （--root・--allow-remote）は、既定の場所（LENSBENCH_MACROS・今のフォルダ）を探さない
impl Files {
    /// 評価関数の項の zplm のファイル・フォルダを確かめる（CLI の optimize・tolerance の項のファイルにも）
    pub fn check_terms(&self, t: &mut Value) -> Result<(), String> {
        self.check_term_paths(t).map_err(|e| e.msg)
    }

    pub(crate) fn check_term_paths(&self, t: &mut Value) -> Result<(), RpcErr> {
        let limited = !matches!(self, Files::Any);
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
                    *f = json!(self.check_path(Path::new(p), false)?.to_string_lossy());
                }
            }
            if let Some(Value::Array(ds)) = o.get_mut("macro_dirs") {
                for d in ds.iter_mut() {
                    if let Some(p) = d.as_str() {
                        *d = json!(self.check_path(Path::new(p), false)?.to_string_lossy());
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
    pub(crate) fn check_path(&self, p: &Path, write: bool) -> Result<PathBuf, RpcErr> {
        match self {
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
    /// 元に戻す・やり直すための前の系と変更の記録（新しいものが後ろ）
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    /// 最後に記録した set の名前と時刻（同じ名前の set が続けば 1 段にまとめる。スライダーのドラッグなど）
    last_set: Option<(Vec<String>, Instant)>,
}

/// 元に戻す 1 段: 系と読み方、そのときの変更の記録、この段から次の段への変更の短い記述（GUI の一覧の名前）
#[derive(Clone)]
struct Snap {
    sys: Arc<System>,
    spec: LoadSpec,
    log: Vec<Change>,
    what: Value,
}

/// 元に戻せる段の数
const UNDO_DEPTH: usize = 100;
/// 同じ名前の set をまとめる間隔 [s]
const SET_MERGE_S: f64 = 1.0;

/// serve の 1 つの場: 読み込んだ系・変更の記録・元に戻す段・走っている最適化などの進み具合と、要求で与えたパスの扱い。
/// 要求は場ごとに 1 つずつ順に処理する。TCP・パイプの serve は 1 つの場を全部の接続で共有し、GUI は [`global`] の場を、
/// 試験は場を別々に作って使う
pub struct Session {
    pub(crate) files: Files,
    served: Mutex<Option<Served>>,
    /// 要求を 1 つずつ処理する
    request: Mutex<()>,
    /// 処理している要求の中断の印（`cancel` で倒す）
    running: Mutex<Option<lensbench_core::par::CancelToken>>,
    opt_run: Mutex<Option<OptRun>>,
    opt_cancel: AtomicBool,
    tol_run: Mutex<Option<(usize, usize)>>,
    tol_cancel: AtomicBool,
    swap_run: Mutex<Option<(usize, usize)>>,
    swap_cancel: AtomicBool,
}

impl Default for Session {
    fn default() -> Self {
        Self::with_files(Files::Any)
    }
}

/// プロセスで共通の場（GUI・[`call`] など場を渡さない関数が使う）
pub fn global() -> &'static Session {
    static G: std::sync::OnceLock<Session> = std::sync::OnceLock::new();
    G.get_or_init(Session::default)
}

impl Session {
    /// 空の場（要求で与えたパスはそのまま読み書きする）
    pub fn new() -> Self {
        Self::default()
    }

    /// 要求で与えたパスの扱いを決めた空の場
    pub fn with_files(files: Files) -> Self {
        Session {
            files,
            served: Mutex::new(None),
            request: Mutex::new(()),
            running: Mutex::new(None),
            opt_run: Mutex::new(None),
            opt_cancel: AtomicBool::new(false),
            tol_run: Mutex::new(None),
            tol_cancel: AtomicBool::new(false),
            swap_run: Mutex::new(None),
            swap_cancel: AtomicBool::new(false),
        }
    }

    /// 系を読んで場に置く（起動の引数で与えた系。要求で与えたパスの扱いは当てない）
    pub fn open(&self, spec: LoadSpec, threads: Option<usize>) -> Result<(), String> {
        let s = lensbench_core::with_threads(threads, || load_file(&spec))
            .map_err(|e| e.to_string())?;
        store(self, s, spec);
        Ok(())
    }

    /// 要求で与えたパスの扱い
    pub fn files(&self) -> &Files {
        &self.files
    }

    /// 要求を 1 つ処理する（要求を運ぶ道から。配列は bin で受け渡す）。panic は誤りにする
    pub fn handle(
        &self,
        method: &str,
        params: &Value,
        bins: Vec<Bin>,
        threads: Option<usize>,
    ) -> Result<(Value, Vec<Bin>), CallError> {
        dispatch(self, method, params, bins, threads).map_err(CallError::from)
    }

    fn served(&self) -> std::sync::MutexGuard<'_, Option<Served>> {
        self.served.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 読み込んだ系
    pub(crate) fn current_system(&self) -> Result<Arc<System>, RpcErr> {
        Ok(self.current()?.0)
    }

    fn current(&self) -> Result<(Arc<System>, LoadSpec), RpcErr> {
        self.served()
            .as_ref()
            .map(|x| (x.sys.clone(), x.spec.clone()))
            .ok_or_else(|| RpcErr::app("系がない（先に load）"))
    }

    /// 読み込んだ後の変更の記録
    fn current_log(&self) -> Vec<Change> {
        self.served()
            .as_ref()
            .map(|x| x.log.clone())
            .unwrap_or_default()
    }
}

/// 読み込んだ系を置く（変更の記録・元に戻す段は空にする）
fn store(sess: &Session, sys: System, spec: LoadSpec) {
    *sess.served() = Some(Served {
        sys: Arc::new(sys),
        spec,
        log: Vec::new(),
        undo: Vec::new(),
        redo: Vec::new(),
        last_set: None,
    });
}

/// load・reload の段の扱い（`history`）
#[derive(Clone, Copy, PartialEq)]
enum LoadHistory {
    /// 段を空にする（既定）
    Clear,
    /// 前の系を 1 段として積む（開く・読み直すを元に戻せる）
    Push,
    /// 段はそのまま（GUI の組合せの作り直しなど、段を別に持つとき）
    Keep,
}

fn load_history(p: &Value) -> Result<LoadHistory, RpcErr> {
    Ok(match p.get("history").and_then(Value::as_str) {
        None | Some("clear") => LoadHistory::Clear,
        Some("push") => LoadHistory::Push,
        Some("keep") => LoadHistory::Keep,
        Some(x) => {
            return Err(RpcErr::params(format!(
                "history は clear・push・keep（{x}）"
            )))
        }
    })
}

/// 読み込んだ系を置く。h が Push なら前の系を段に積み、Keep なら段はそのまま
fn store_loaded(sess: &Session, sys: System, spec: LoadSpec, h: LoadHistory, reload: bool) {
    let mut g = sess.served();
    let Some(mut x) = g.take().filter(|_| h != LoadHistory::Clear) else {
        drop(g);
        store(sess, sys, spec);
        return;
    };
    if h == LoadHistory::Push {
        let name = spec
            .file
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let what = if reload {
            json!({ "reload": name })
        } else {
            json!({ "open": name })
        };
        push_undo(&mut x, what);
        x.redo.clear();
    }
    x.sys = Arc::new(sys);
    x.spec = spec;
    x.log = Vec::new();
    x.last_set = None;
    *g = Some(x);
}

#[path = "session_hist.rs"]
mod hist;

mod catalog;
mod layout;
mod optim;
mod trace;
pub use catalog::range_system;
use catalog::{browse_of, cad_step, catalog_fill, user_catalog};
use hist::{change_label, config_only, history_json, snap_was_load, CONFIG_KEY};
use layout::{configs, layout, layout_export, layout_with};
use optim::{optimize, swap, tolerance, zoom, OptRun};
pub use optim::{
    optimize_cancel, optimize_progress, swap_cancel, swap_progress, tolerance_cancel,
    tolerance_progress,
};
use trace::{plugin, trace};

/// 今の系を元に戻す段に積む（深さを超えたら古い段を捨てる）
fn push_undo(x: &mut Served, what: Value) {
    x.undo.push(Snap {
        sys: x.sys.clone(),
        spec: x.spec.clone(),
        log: x.log.clone(),
        what,
    });
    if x.undo.len() > UNDO_DEPTH {
        x.undo.remove(0);
    }
}

/// 変更した系を置き、変更を記録する。前の系は元に戻す段に積む（やり直す段は空にする）
fn store_changed(sess: &Session, sys: System, spec: LoadSpec, c: Change) {
    let mut g = sess.served();
    let Some(mut x) = g.take() else {
        *g = Some(Served {
            sys: Arc::new(sys),
            spec,
            log: vec![c],
            undo: Vec::new(),
            redo: Vec::new(),
            last_set: None,
        });
        return;
    };
    let now = Instant::now();
    // 構成の切り替えだけの edit が続くときは、間があいても 1 段にまとめる（名前は CONFIG_KEY、GAR-28）
    let names = match &c {
        Change::Set(v) => Some(v.iter().map(|r| r.0.clone()).collect::<Vec<_>>()),
        Change::Edit(o) => config_only(o).then(|| vec![CONFIG_KEY.to_string()]),
    };
    let merge = matches!((&names, &x.last_set), (Some(n), Some((m, t))) if n == m && !x.undo.is_empty()
        && (n[..] == [CONFIG_KEY] || now.duration_since(*t).as_secs_f64() < SET_MERGE_S));
    if !merge {
        let what = change_label(Some(&c));
        push_undo(&mut x, what);
    }
    x.redo.clear();
    x.last_set = names.map(|n| (n, now));
    x.log.push(c);
    x.sys = Arc::new(sys);
    x.spec = spec;
    *g = Some(x);
}

/// 元に戻す（back）・やり直す。戻した後の系の面の数と、残りの段の数
fn step_history(sess: &Session, back: bool) -> Result<Value, RpcErr> {
    let mut g = sess.served();
    let x = g
        .as_mut()
        .ok_or_else(|| RpcErr::app("系がない（先に load）"))?;
    let (from, to) = if back {
        (&mut x.undo, &mut x.redo)
    } else {
        (&mut x.redo, &mut x.undo)
    };
    let Some(snap) = from.pop() else {
        return Err(RpcErr::app(if back {
            "元に戻す変更がない"
        } else {
            "やり直す変更がない"
        }));
    };
    let file0 = x.spec.file.clone();
    to.push(Snap {
        sys: std::mem::replace(&mut x.sys, snap.sys),
        spec: std::mem::replace(&mut x.spec, snap.spec),
        log: std::mem::replace(&mut x.log, snap.log),
        what: snap.what,
    });
    x.last_set = None;
    let hash = lensbench_project::sysfile::system_hash(&x.sys).map_err(RpcErr::app)?;
    let mut v = history_json(x);
    v["surfaces"] = json!(x.sys.surfaces.len());
    v["system_hash"] = json!(hash);
    // 開く・読み直すを戻したら、戻した系の概要（ファイルが変わる）
    if x.spec.file != file0 || snap_was_load(&v, back) {
        v["loaded"] = summary(&x.sys, &x.spec);
    }
    Ok(v)
}

/// 元に戻せる段・やり直せる段（`history_json`）
fn history(sess: &Session) -> Value {
    sess.served().as_ref().map_or_else(
        || json!({ "undo": 0, "redo": 0, "undo_list": [], "redo_list": [] }),
        history_json,
    )
}

/// 結果に段の数と一覧を足す
fn add_history(sess: &Session, v: &mut Value) {
    if let (Some(o), Value::Object(h)) = (v.as_object_mut(), history(sess)) {
        o.extend(h);
    }
}

/// steps 段まとめて戻す・やり直す（途中で尽きたらそこまで）
fn step_history_n(sess: &Session, back: bool, p: &Value) -> Result<Value, RpcErr> {
    let n = opt_usize(p, "steps")?.unwrap_or(1).max(1);
    let mut v = step_history(sess, back)?;
    let mut loaded = v.get("loaded").is_some();
    for _ in 1..n {
        match step_history(sess, back) {
            Ok(w) => {
                loaded |= w.get("loaded").is_some();
                v = w;
            }
            Err(_) => break,
        }
    }
    // 途中で開く・読み直すを越えたら、最後の系の概要
    if loaded && v.get("loaded").is_none() {
        if let Ok((s, spec)) = sess.current() {
            v["loaded"] = summary(&s, &spec);
        }
    }
    Ok(v)
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
pub(crate) struct RpcErr {
    code: i64,
    pub(crate) msg: String,
    /// core の誤りの種類（[`lensbench_core::Error::kind`]。返す誤りの `data.kind`）
    kind: Option<&'static str>,
}

impl RpcErr {
    fn app(m: impl Into<String>) -> Self {
        RpcErr {
            code: -32000,
            msg: m.into(),
            kind: None,
        }
    }
    /// core の誤りの種類を付ける（文を作り直したとき）
    pub(crate) fn with_kind(self, k: &'static str) -> Self {
        RpcErr {
            kind: Some(k),
            ..self
        }
    }
    pub(crate) fn params(m: impl Into<String>) -> Self {
        RpcErr {
            code: -32602,
            msg: m.into(),
            kind: None,
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
        RpcErr {
            kind: Some(e.kind()),
            ..RpcErr::app(e.to_string())
        }
    }
}

impl From<RpcErr> for CallError {
    fn from(e: RpcErr) -> Self {
        CallError {
            code: e.code,
            message: e.msg,
            kind: e.kind.map(str::to_string),
        }
    }
}

/// 配列（バイナリで渡す）
#[derive(Debug, Clone, PartialEq)]
pub enum Data {
    F64(Vec<f64>),
    I32(Vec<i32>),
    U8(Vec<u8>),
    C64(Vec<C64>),
    Bool(Vec<bool>),
}

/// 名前と形のついた配列（要求・応答の bin）
#[derive(Debug, Clone, PartialEq)]
pub struct Bin {
    pub name: String,
    pub shape: Vec<usize>,
    pub data: Data,
}

impl Bin {
    pub fn f64(name: &str, v: Vec<f64>) -> Bin {
        Bin {
            name: name.into(),
            shape: vec![v.len()],
            data: Data::F64(v),
        }
    }
    pub fn i32(name: &str, v: Vec<i32>) -> Bin {
        Bin {
            name: name.into(),
            shape: vec![v.len()],
            data: Data::I32(v),
        }
    }
    /// numpy の dtype の文字
    pub fn dtype(&self) -> &'static str {
        match self.data {
            Data::F64(_) => "<f8",
            Data::I32(_) => "<i4",
            Data::U8(_) => "|u1",
            Data::C64(_) => "<c16",
            Data::Bool(_) => "|b1",
        }
    }
    /// ヘッダの bin の 1 項目
    pub fn header(&self) -> Value {
        json!({ "name": self.name, "dtype": self.dtype(), "shape": self.shape })
    }
    /// 生のバイト列（リトルエンディアン）を足す
    pub fn write_to(&self, out: &mut Vec<u8>) {
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

thread_local! {
    /// このスレッドの最後の panic の文と場所（[`note_panic`]）
    static LAST_PANIC: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// panic の hook から、panic の文と場所を残す（[`Session::handle`] が「内部エラー（panic）: …」に載せる）
pub fn note_panic(text: &str) {
    LAST_PANIC.with(|l| *l.borrow_mut() = Some(text.to_string()));
}

/// 要求を 1 つ処理する（1 つずつ順に。panic はエラーにする）
fn dispatch(
    sess: &Session,
    method: &str,
    params: &Value,
    bins: Vec<Bin>,
    threads: Option<usize>,
) -> Result<Reply, RpcErr> {
    // 進み具合・中断・停止は、処理している要求を待たずに答える（別の接続から）
    if let Some(r) = status(sess, method, params) {
        return r;
    }
    let _g = sess.request.lock().unwrap_or_else(|e| e.into_inner());
    // 中断の印: 呼び出し側（GUI の Worker）が置いた印があればそれを使う
    let tok = lensbench_core::par::current().cancel.unwrap_or_default();
    *sess.running.lock().unwrap_or_else(|e| e.into_inner()) = Some(tok.clone());
    struct Done<'a>(&'a Session);
    impl Drop for Done<'_> {
        fn drop(&mut self) {
            *self.0.running.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
    let _d = Done(sess);
    lensbench_core::with_threads(threads, || {
        LAST_PANIC.with(|l| l.borrow_mut().take());
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            lensbench_core::par::with_cancel(Some(&tok), || handle(sess, method, params, bins))
        }))
        .unwrap_or_else(|e| {
            // panic の hook（GUI の crash.rs）が残した文と場所。無ければ panic の文だけ
            let t = LAST_PANIC
                .with(|l| l.borrow_mut().take())
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .or_else(|| e.downcast_ref::<String>().cloned());
            Err(RpcErr::app(match t {
                Some(t) => format!("内部エラー（panic）: {t}"),
                None => "内部エラー（panic）".into(),
            }))
        })
    })
}

/// 処理している要求を待たない method: `progress`（最適化・公差・差し替え探索の進み具合と、要求を処理しているか）、
/// `cancel`（処理している要求に途中でやめるよう頼む。解析は「中断した」の誤りで返る）、
/// `stop`（`what` の最適化・公差・差し替え探索を止め、そこまでの結果を返させる）
fn status(sess: &Session, method: &str, p: &Value) -> Option<Result<Reply, RpcErr>> {
    let r = match method {
        "progress" => json!({
            "running": sess.running.lock().unwrap_or_else(|e| e.into_inner()).is_some(),
            "optimize": sess.optimize_progress(),
            "tolerance": sess.tolerance_progress(),
            "swap": sess.swap_progress(),
        }),
        "cancel" => {
            let t = sess
                .running
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if let Some(t) = &t {
                t.cancel();
            }
            json!(t.is_some())
        }
        "stop" => match p["what"].as_str() {
            Some("optimize") => {
                sess.optimize_cancel();
                json!(true)
            }
            Some("tolerance") => {
                sess.tolerance_cancel();
                json!(true)
            }
            Some("swap") => {
                sess.swap_cancel();
                json!(true)
            }
            _ => {
                return Some(Err(RpcErr::params(
                    "stop の what は optimize・tolerance・swap のどれか",
                )))
            }
        },
        _ => return None,
    };
    Some(Ok((r, Vec::new())))
}

/// [`call`] のエラー（JSON-RPC の error と同じ code・message）
#[derive(Debug, Clone, PartialEq)]
pub struct CallError {
    pub code: i64,
    pub message: String,
    /// core の誤りの種類（`not_implemented`・`invalid`・`io`・`cancelled`。serve は `data.kind` に書く）
    pub kind: Option<String>,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// 結果の配列（`trace` の x・y・z・opl・err・vig など。名前と f64 の並び、整数も f64 にする）
pub type Arrays = Vec<(String, Vec<f64>)>;

/// 新しい名前か旧名の値（両方は誤り。無ければ null）と、使った鍵
fn renamed_key<'a>(
    p: &'a Value,
    new: &'static str,
    old: &'static str,
) -> Result<(&'a Value, &'static str), RpcErr> {
    match (&p[new], &p[old]) {
        (Value::Null, Value::Null) => Ok((&Value::Null, new)),
        (v, Value::Null) => Ok((v, new)),
        (Value::Null, v) => Ok((v, old)),
        _ => Err(RpcErr::params(format!(
            "{new} と {old}（旧名）の両方は渡せない"
        ))),
    }
}

/// [`renamed_key`] の値だけ
fn renamed<'a>(p: &'a Value, new: &'static str, old: &'static str) -> Result<&'a Value, RpcErr> {
    renamed_key(p, new, old).map(|x| x.0)
}

/// load の params から系の読み方
fn load_spec(sess: &Session, p: &Value) -> Result<LoadSpec, RpcErr> {
    let file = p["file"]
        .as_str()
        .ok_or_else(|| RpcErr::params("load には file"))?;
    let read_all = |v: Vec<PathBuf>| -> Result<Vec<PathBuf>, RpcErr> {
        v.iter().map(|x| sess.files.check_path(x, false)).collect()
    };
    // 名前は Python と同じ（CLI-17）。旧名 glass_dir・lens_catalog・path も受ける
    let glass = renamed(p, "glass_dirs", "glass_dir")?;
    let lens = renamed(p, "lens_catalogs", "lens_catalog")?;
    let (bp, bk) = renamed_key(p, "beam_path", "path")?;
    let path = match bp {
        Value::Null => None,
        v => Some(
            v.as_str()
                .ok_or_else(|| RpcErr::params(format!("{bk} は文字列（BS の経路、例 \"TR\"）")))?
                .to_string(),
        ),
    };
    Ok(LoadSpec {
        file: sess.files.check_path(Path::new(file), false)?,
        glass_dir: read_all(str_list(glass, "glass_dirs")?)?,
        lens_catalog: read_all(str_list(lens, "lens_catalogs")?)?,
        cfg: opt_usize(p, "cfg")?.unwrap_or(1),
        state: p["state"].as_str().map(str::to_string),
        path,
        names: lensbench_project::open::ArgNames::SERVE,
    })
}

impl Session {
    /// serve と同じ method を同じプロセスの中から呼ぶ（GUI が使う）。params・result は TCP の要求と同じ JSON。
    /// 系は場に 1 つで、TCP の要求と同じく 1 つずつ順に処理する。bin（配列）を使う `trace`・`echo` は
    /// JSON の並びで渡す。結果の配列が要るときは [`call_arrays`]。`shutdown` はプロセスを止めない。
    pub fn call(
        &self,
        method: &str,
        params: &Value,
        threads: Option<usize>,
    ) -> Result<Value, CallError> {
        self.call_arrays(method, params, threads).map(|(v, _)| v)
    }

    /// [`call`] と同じで、結果の配列（TCP では bin で返るもの）も受け取る
    pub fn call_arrays(
        &self,
        method: &str,
        params: &Value,
        threads: Option<usize>,
    ) -> Result<(Value, Arrays), CallError> {
        dispatch(self, method, params, Vec::new(), threads)
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
            .map_err(CallError::from)
    }

    /// 系を読んで（`load` と同じ params）、そのレイアウト図（`layout` と同じ結果）を返す。読み込んだ系は変えず、
    /// 他の要求の順番も待たない。GUI が、解析を回している間も組合せの図だけ先に描き直すのに使う
    pub fn preview_layout(&self, load: &Value, opts: &Value) -> Result<Value, CallError> {
        let sess = self;
        let f = || -> Result<Value, RpcErr> {
            let spec = load_spec(sess, load)?;
            let s = load_file(&spec)?;
            layout_with(sess, &s, &spec, opts)
        };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
            .unwrap_or_else(|_| Err(RpcErr::app("内部エラー（panic）")))
            .map_err(CallError::from)
    }

    /// カタログの一覧（`glasses`・`lenses` と同じ params・結果）。読み込んだ系に関係なく、他の要求の順番も待たない。
    /// GUI が、解析を回している間もカタログを読めるように使う
    /// ユーザーカタログの一覧・名前と説明の変更・削除（`catalog_items`・`catalog_edit`）を、読み込んだ系と関係なく
    /// 呼ぶ（GUI の下書きの worker から。`catalog_add` は読み込んだ系が要るので [`call`] で）
    pub fn user_catalog_edit(&self, method: &str, params: &Value) -> Result<Value, CallError> {
        let sess = self;
        let f = || match method {
            "catalog_items" | "catalog_edit" => user_catalog(sess, method, params),
            "catalog_fill" => catalog_fill(sess, params),
            m => Err(RpcErr::params(format!(
                "{m} は catalog_items・catalog_edit・catalog_fill"
            ))),
        };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
            .unwrap_or_else(|_| Err(RpcErr::app("内部エラー（panic）")))
            .map_err(CallError::from)
    }

    pub fn browse(&self, method: &str, params: &Value) -> Result<Value, CallError> {
        let sess = self;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            browse_of(sess, method, params)
        }))
        .unwrap_or_else(|_| Err(RpcErr::app("内部エラー（panic）")))
        .map_err(CallError::from)
    }
}

/// [`global`] の場の [`Session::call`]
pub fn call(method: &str, params: &Value, threads: Option<usize>) -> Result<Value, CallError> {
    global().call(method, params, threads)
}

/// [`global`] の場の [`Session::call_arrays`]
pub fn call_arrays(
    method: &str,
    params: &Value,
    threads: Option<usize>,
) -> Result<(Value, Arrays), CallError> {
    global().call_arrays(method, params, threads)
}

/// [`global`] の場の [`Session::preview_layout`]
pub fn preview_layout(load: &Value, opts: &Value) -> Result<Value, CallError> {
    global().preview_layout(load, opts)
}

/// [`global`] の場の [`Session::user_catalog_edit`]
pub fn user_catalog_edit(method: &str, params: &Value) -> Result<Value, CallError> {
    global().user_catalog_edit(method, params)
}

/// [`global`] の場の [`Session::browse`]
pub fn browse(method: &str, params: &Value) -> Result<Value, CallError> {
    global().browse(method, params)
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

fn handle(sess: &Session, method: &str, p: &Value, bins: Vec<Bin>) -> Result<Reply, RpcErr> {
    match method {
        "ping" => Ok((json!({ "version": lensbench_core::VERSION }), Vec::new())),
        "shutdown" => Ok((json!(true), Vec::new())),
        // 配列をそのまま返す（往復の時間を測る）
        "echo" => Ok((json!({ "params": p }), bins)),
        "load" => {
            let h = load_history(p)?;
            let spec = load_spec(sess, p)?;
            let s = load_file(&spec)?;
            let mut v = summary(&s, &spec);
            store_loaded(sess, s, spec, h, false);
            add_history(sess, &mut v);
            Ok((v, Vec::new()))
        }
        // 同じ読み方でファイルを読み直す（set で変えた値は捨てる）。別アプリが系のファイルを書き換えた後に使う
        "reload" => {
            let h = load_history(p)?;
            let (_, spec) = sess.current()?;
            let s = load_file(&spec)?;
            let mut v = summary(&s, &spec);
            store_loaded(sess, s, spec, h, true);
            add_history(sess, &mut v);
            Ok((v, Vec::new()))
        }
        "system" => {
            let (s, spec) = sess.current()?;
            Ok((summary(&s, &spec), Vec::new()))
        }
        // 読み込んだ後の set・edit（最適化の結果を当てたものも）を 1 段ずつ元に戻す・やり直す（100 段まで。
        // 同じパラメータの set が 1 秒以内に続けば 1 段。load・reload で空になる。history: "push" で読めば読む前の系も 1 段）
        "undo" => Ok((step_history_n(sess, true, p)?, Vec::new())),
        "redo" => Ok((step_history_n(sess, false, p)?, Vec::new())),
        "history" => Ok((history(sess), Vec::new())),
        "get" => {
            let (s, _) = sess.current()?;
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
            let (s, spec) = sess.current()?;
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
            store_changed(sess, n, spec, Change::Set(rec));
            let mut v = history(sess);
            v["changed"] = json!(names);
            Ok((v, Vec::new()))
        }
        // 系を .zmx に書き出す（別アプリに渡す）
        "write" => {
            let (s, _) = sess.current()?;
            let path = p["path"]
                .as_str()
                .ok_or_else(|| RpcErr::params("write には path"))?;
            let notes = zmx_write::save(&sess.files.check_path(Path::new(path), true)?, &s)?;
            Ok((json!({ "written": path, "notes": notes }), Vec::new()))
        }
        // 系の編集: ops の操作の列を順に当てる（どれかが失敗すれば系は変えない）
        "edit" => {
            let (s, spec) = sess.current()?;
            let mut n = (*s).clone();
            let notes =
                lensbench_project::edit_ops::apply(&mut n, &p["ops"]).map_err(RpcErr::app)?;
            let hash = lensbench_project::sysfile::system_hash(&n).map_err(RpcErr::app)?;
            let count = n.surfaces.len();
            store_changed(sess, n, spec, Change::Edit(p["ops"].clone()));
            let mut v = history(sess);
            v["notes"] = json!(notes);
            v["surfaces"] = json!(count);
            v["system_hash"] = json!(hash);
            Ok((v, Vec::new()))
        }
        // 面の値（surface は番号か名前。省略で全部の面と、玉・群の面の範囲 units）
        "surface" => {
            let (s, _) = sess.current()?;
            let one = |j: usize| {
                lensbench_project::edit_ops::surf_fields_json(&s, j).map_err(RpcErr::app)
            };
            match p.get("surface") {
                None | Some(Value::Null) => {
                    let mut all = (0..s.surfaces.len())
                        .map(one)
                        .collect::<Result<Vec<_>, _>>()?;
                    // 面ごとのソルブ・ピックアップ（GUI の面の表の印。形は surface_data と同じ）
                    for (j, f) in all.iter_mut().enumerate() {
                        let sp = lensbench_project::edit_ops::surface_solves(&s, j);
                        if let (Some(o), Value::Object(x)) = (f.as_object_mut(), sp) {
                            o.extend(x);
                        }
                    }
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
                    // 機構部品（系ファイルの mechanics と同じ形、GUI の面の表で編集）
                    let mechs = serde_json::to_value(&s.mechanics).unwrap_or_default();
                    Ok((
                        json!({ "surfaces": all, "units": units, "mechanics": mechs }),
                        Vec::new(),
                    ))
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
            let (s, _) = sess.current()?;
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
            let (s, _) = sess.current()?;
            let v = lensbench_project::edit_ops::system_data(&s).map_err(RpcErr::app)?;
            Ok((json!({ "data": v }), Vec::new()))
        }
        "config_table" => {
            let (s, spec) = sess.current()?;
            let rows: Vec<Value> = s
                .configs()
                .map(|c| {
                    c.rows
                        .iter()
                        .map(|r| {
                            json!({
                                "param": s.param_name(r.param), "values": r.values, "picks": r.picks.iter()
                                    .map(|p| json!({ "k": p.k, "from": p.from, "scale": p.scale, "offset": p.offset }))
                                    .collect::<Vec<_>>(),
                            })
                        })
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
            let (_, spec) = sess.current()?;
            let v = config_diff(&spec)?;
            Ok((v, Vec::new()))
        }
        // 系ファイルに保存する（編集・set の後の系をそのまま）
        "save" => {
            let (s, _) = sess.current()?;
            let path = p["path"]
                .as_str()
                .ok_or_else(|| RpcErr::params("save には path"))?;
            lensbench_project::sysfile::save(
                &sess.files.check_path(Path::new(path), true)?,
                &s,
                &[],
            )
            .map_err(RpcErr::app)?;
            let hash = lensbench_project::sysfile::system_hash(&s).map_err(RpcErr::app)?;
            Ok((json!({ "written": path, "system_hash": hash }), Vec::new()))
        }
        // 今の系の面 from〜to だけを系ファイル（面の並び）に書き出す（GUI のレンズデータの「選んだ範囲を書き出す」）。
        // 物体面・像面は残し、範囲の外の面は消す（外を参照するピックアップ・ソルブは外れる）。絞りが範囲の外なら from を絞りに
        "save_range" => {
            let (s, _) = sess.current()?;
            let path = p["path"]
                .as_str()
                .ok_or_else(|| RpcErr::params("save_range には path"))?;
            let num = |k: &str| {
                p[k].as_u64()
                    .map(|x| x as usize)
                    .ok_or_else(|| RpcErr::params(format!("save_range には {k}（面の番号）")))
            };
            let (from, to) = (num("from")?, num("to")?);
            let s = range_system(&s, from, to).map_err(|e| RpcErr::app(e.to_string()))?;
            lensbench_project::sysfile::save(
                &sess.files.check_path(Path::new(path), true)?,
                &s,
                &[],
            )
            .map_err(RpcErr::app)?;
            Ok((
                json!({ "written": path, "surfaces": s.surfaces.len() }),
                Vec::new(),
            ))
        }
        "analyze" => analyze(sess, p).map(|v| (v, Vec::new())),
        // カタログを探す一覧（GUI のカタログブラウザ）
        "glasses" | "lenses" | "catalog_search" => {
            browse_of(sess, method, p).map(|v| (v, Vec::new()))
        }
        "trace" => trace(sess, p, bins),
        // 解析・検出器のプラグイン
        "plugins" => Ok((crate::load::plugins_json(), Vec::new())),
        // ユーザーカタログ: 読み込んだプロジェクトのブロック・範囲を項目に保存、一覧、名前・説明の変更と削除
        "catalog_add" | "catalog_items" | "catalog_edit" => {
            user_catalog(sess, method, p).map(|v| (v, Vec::new()))
        }
        // カタログの未完の品に厚さ・径を足し、補足のカタログ（ユーザーカタログ）に普通の品として保存
        "catalog_fill" => catalog_fill(sess, p).map(|v| (v, Vec::new())),
        // 品の詳しい形の STEP を 1 件取ってキャッシュに置き、簡易外形と比べる（CLI の cad-step）
        "cad_step" => cad_step(sess, p).map(|v| (v, Vec::new())),
        // analyze の解析の設定の項目（名前・型・既定・説明。GUI の設定欄）
        "analysis_args" => {
            let cmd = p["cmd"]
                .as_str()
                .ok_or_else(|| RpcErr::params("analysis_args には cmd"))?;
            let args = crate::params::analysis_args(cmd).ok_or_else(|| {
                RpcErr::params(format!(
                    "{cmd} は analyze の解析ではない（解析の一覧は plugins）"
                ))
            })?;
            Ok((json!({ "cmd": cmd, "args": args }), Vec::new()))
        }
        "plugin" => plugin(sess, p, bins),
        "layout" => layout(sess, p).map(|v| (v, Vec::new())),
        // レイアウトをファイルに書き出す（GUI の書き出し。形式は拡張子で、CLI の layout -o と同じ）
        "layout_export" => layout_export(sess, p).map(|v| (v, Vec::new())),
        "configs" => configs(sess).map(|v| (v, Vec::new())),
        // 減衰最小二乗の最適化（GUI の最適化パネル）
        "optimize" => optimize(sess, p).map(|v| (v, Vec::new())),
        // カタログレンズの差し替え探索（GUI）
        "swap" => swap(sess, p).map(|v| (v, Vec::new())),
        // モンテカルロ公差
        "tolerance" => tolerance(sess, p).map(|v| (v, Vec::new())),
        // ズーム軌跡（GUI のズームのパネル）
        "zoom" => zoom(sess, p).map(|v| (v, Vec::new())),
        "" => Err(RpcErr {
            code: -32600,
            msg: "method がない".into(),
            kind: None,
        }),
        m => Err(RpcErr {
            code: -32601,
            msg: format!("method {m} はない"),
            kind: None,
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

/// 解析の登録簿の解析（と膜の境界）を、読み込んだ系で実行する。最適化・公差・レイアウトの書き出しは専用のメソッド
fn analyze(sess: &Session, p: &Value) -> Result<Value, RpcErr> {
    sess.current()?;
    let m = p
        .as_object()
        .ok_or_else(|| RpcErr::params("params はオブジェクト"))?;
    let cmd = m
        .get("cmd")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcErr::params("cmd（解析の名前）が要る"))?;
    if cmd == "coatings" {
        return crate::analyze::coatings_loaded(sess, m);
    }
    match crate::analyses().get(cmd).cloned() {
        Some(def) => crate::analyze::run(sess, def.as_ref(), m),
        None => Err(RpcErr::params(match cmd {
            "optimize" | "tolerance" | "swap" | "zoom" | "layout" => format!(
                "{cmd} は analyze の解析ではない（メソッド {} で）",
                if cmd == "layout" {
                    "layout_export"
                } else {
                    cmd
                }
            ),
            _ => format!("{cmd} は analyze の解析ではない（解析の一覧は analysis_args・plugins）"),
        })),
    }
}

/// 登録簿の解析に渡す系の読み方（load で選んだもの。別に読んだ系にも読み込んだ後の変更を当てる）で f を呼ぶ
pub(crate) fn with_source<R>(
    sess: &Session,
    f: impl FnOnce(&dyn lensbench_analysis::Source) -> R,
) -> Result<R, RpcErr> {
    let (_, spec) = sess.current()?;
    let log = sess.current_log();
    let c = spec.clone();
    let r = |s: System, l: &str, n: &mut Vec<String>| replay(s, &log, l, n);
    Ok(f(&crate::source::FileSource {
        c: &c,
        replay: Some(&r),
    }))
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
        let mut c = spec.clone();
        c.cfg = k;
        systems.push(load_file(&c).map_err(|e| e.at(format!("構成 {k}")))?);
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

/// プロジェクトファイル（系ファイルは含めない）
fn is_project(f: &Path) -> bool {
    lensbench_project::open::FileKind::of(f) == lensbench_project::open::FileKind::Project
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
