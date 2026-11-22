//! serve の場（[`Session`]）: 系を読み込んだまま、JSON の要求（method と params）で系の変更・解析を繰り返す。
//!
//! 書式は docs/serve.md。要求を運ぶ道（TCP・パイプ・共有メモリ、1 行 1 要求の JSON-RPC）は CLI の `serve` が持ち、
//! 同じプロセスの GUI・Web 版は [`Session::call`] を直接呼ぶ。要点:
//!
//! - 系は場に 1 つ。要求は場ごとに 1 つずつ順に処理する
//! - 解析（`analyze`）は解析の登録簿の解析で、CLI のサブコマンドと同じ設定・同じ結果。系はファイルを読まず、読み込んだ系を使う

use lensbench_core::{LockExt, RwLockExt};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use web_time::Instant;

use lensbench_core::coating::C64;
use lensbench_core::history::History;
use lensbench_core::system::Launch;
use lensbench_core::System;
use lensbench_export::d2::Plane;
use lensbench_formats::zmx_write;
use serde_json::{json, Map, Value};

use crate::load::{read_file_notes, Faces, ReadFile, SysSpec as LoadSpec};
use lensbench_project::open::Source;
use lensbench_project::retarget::Retarget;
use lensbench_project::{BlockSpans, FaceId};

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
    pub fn check_terms(&self, t: &mut Value) -> lensbench_core::Result<()> {
        Ok(self.check_term_paths(t).map_err(|e| e.msg)?)
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
            "{}: with --allow-remote, requests don't read or write files (limit the place with --root to allow it)",
            p.display()
        ))),
        Files::Root(root) => {
            let q = if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            };
            let bad = || RpcErr::app(format!("{} is outside --root", p.display()));
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
    Edit(Vec<lensbench_project::edit_ops::EditOp>),
}

struct Served {
    sys: Arc<System>,
    spec: LoadSpec,
    /// 読んだときのファイル（別の構成・状態・経路はファイルを読み直さずにここから）
    src: Arc<Source>,
    /// プロジェクトの id のあるブロック・群ごとの系の面の範囲（読んだときのもの。面を足す・消す edit の後は空）
    blocks: Arc<Vec<BlockSpans>>,
    /// プロジェクトの読んだときの系の面ごとの名前（変更の記録 log はこの系からの変更。別の状態・経路へ当て直す）
    faces: Arc<Faces>,
    /// 読んだときの注意（load・reload の返事の notes）
    notes: Vec<String>,
    log: Vec<Change>,
    /// 元に戻す・やり直すための前の系と変更の記録。段の名前は変更の短い記述（GUI の一覧の名前）
    hist: History<Snap, Value>,
    /// 最後に記録した set の名前と時刻（同じ名前の set が続けば 1 段にまとめる。スライダーのドラッグなど）
    last_set: Option<(Vec<String>, Instant)>,
}

/// 読み込んだ系（[`Session::current_full`]）
pub(crate) struct Current {
    /// 読み込んだ後の変更（set・edit）の入った系
    sys: Arc<System>,
    spec: LoadSpec,
    /// 読んだときのファイル
    src: Arc<Source>,
    /// 読んだときの系の面ごとの名前
    faces: Arc<Faces>,
    /// 読み込んだ後の変更
    log: Vec<Change>,
}

/// 読み込んだ .zmx の今の構成（多重構成の表があれば表の今の構成。set_config で替えたものも）
fn active_cfg(cur: &System, spec: &LoadSpec) -> usize {
    cur.configs.as_ref().map_or(spec.cfg, |t| t.active)
}

/// 元に戻す 1 段: 系と読み方、そのときの注意と変更の記録
#[derive(Clone)]
struct Snap {
    sys: Arc<System>,
    spec: LoadSpec,
    src: Arc<Source>,
    blocks: Arc<Vec<BlockSpans>>,
    faces: Arc<Faces>,
    notes: Vec<String>,
    log: Vec<Change>,
}
/// 同じ名前の set をまとめる間隔 [s]
const SET_MERGE_S: f64 = 1.0;

/// serve の 1 つの場: 読み込んだ系・変更の記録・元に戻す段・走っている最適化などの進み具合と、要求で与えたパスの扱い。
/// 系を変える要求は場ごとに 1 つずつ順に、読むだけの要求（[`Access::Read`]）は並行に処理する。
/// 最適化などの長い要求（[`Access::Long`]）は始めたときの系で回し、その間も他の要求を止めない。TCP・パイプの serve は 1 つの場を全部の接続で共有し、GUI は [`global`] の場を、
/// 試験は場を別々に作って使う
pub struct Session {
    pub(crate) files: Files,
    served: Mutex<Option<Served>>,
    /// 系を変える要求は書き、読むだけの要求は読みで取る
    request: std::sync::RwLock<()>,
    /// 長い要求（最適化・公差・差し替え探索・ズーム軌跡）を 1 つずつ
    long: Mutex<()>,
    /// 最適化の apply で当てる変更（長い要求の計算の後、プールの外で当てる。[`dispatch`]）
    pending_apply: Mutex<Option<PendingApply>>,
    /// 処理している要求の中断の印（`cancel` で全部倒す）
    running: Mutex<Vec<lensbench_core::par::CancelToken>>,
    /// 走っている最適化・公差・差し替え探索の進み具合と中断の印（[`optim::Job`]）
    opt_run: optim::JobSlot,
    tol_run: optim::JobSlot,
    swap_run: optim::JobSlot,
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
            request: std::sync::RwLock::new(()),
            long: Mutex::new(()),
            pending_apply: Mutex::new(None),
            running: Mutex::new(Vec::new()),
            opt_run: Default::default(),
            tol_run: Default::default(),
            swap_run: Default::default(),
        }
    }

    /// 系を読んで場に置く（起動の引数で与えた系。要求で与えたパスの扱いは当てない）
    pub fn open(&self, spec: LoadSpec, threads: Option<usize>) -> lensbench_core::Result<()> {
        let r = lensbench_core::with_threads(threads, || read_file_notes(&spec))?;
        store(self, r, spec);
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
        dispatch(self, method, params, bins, threads)
            .and_then(|(a, b)| Ok((a.into_json()?, b)))
            .map_err(CallError::from)
    }

    fn served(&self) -> std::sync::MutexGuard<'_, Option<Served>> {
        self.served.lock_ok()
    }

    /// 読み込んだ系
    pub(crate) fn current_system(&self) -> Result<Arc<System>, RpcErr> {
        Ok(self.current()?.0)
    }

    /// 読み込んだ系・読み方・読んだファイル・変更の記録を 1 度に（鍵を持たない長い要求が、間の set で混ざらないように）
    fn current_full(&self) -> Result<Current, RpcErr> {
        self.served()
            .as_ref()
            .map(|x| Current {
                sys: x.sys.clone(),
                spec: x.spec.clone(),
                src: x.src.clone(),
                faces: x.faces.clone(),
                log: x.log.clone(),
            })
            .ok_or_else(|| RpcErr::app("no system (load first)"))
    }

    fn current(&self) -> Result<(Arc<System>, LoadSpec), RpcErr> {
        self.served()
            .as_ref()
            .map(|x| (x.sys.clone(), x.spec.clone()))
            .ok_or_else(|| RpcErr::app("no system (load first)"))
    }
}

/// 読み込んだ系を置く（変更の記録・元に戻す段は空にする）
fn store(sess: &Session, r: ReadFile, spec: LoadSpec) {
    *sess.served() = Some(Served {
        sys: Arc::new(r.sys),
        spec,
        src: Arc::new(r.src),
        blocks: Arc::new(r.blocks),
        faces: Arc::new(r.faces),
        notes: r.notes,
        log: Vec::new(),
        hist: History::default(),
        last_set: None,
    });
}

/// 読み込んだ系を置く。h が Push なら前の系を段に積み、Keep なら段はそのまま
fn store_loaded(sess: &Session, r: ReadFile, spec: LoadSpec, h: LoadHistory, reload: bool) {
    let mut g = sess.served();
    let Some(mut x) = g.take().filter(|_| h != LoadHistory::Clear) else {
        drop(g);
        store(sess, r, spec);
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
    }
    x.sys = Arc::new(r.sys);
    x.spec = spec;
    x.src = Arc::new(r.src);
    x.blocks = Arc::new(r.blocks);
    x.faces = Arc::new(r.faces);
    x.notes = r.notes;
    x.log = Vec::new();
    x.last_set = None;
    *g = Some(x);
}

mod answer;
use answer::surfaces_view;
pub use answer::Answer;
mod request;
mod run;
pub use request::{AnalyzeReq, GetReq, LoadHistory, LoadReq, Request, ThermalReq};
#[path = "session_hist.rs"]
mod hist;

mod args;
mod catalog;
mod layout;
mod optim;
mod proto;
mod replay;
mod save_project;
mod trace;
pub mod views;
use args::{known_keys, opt_f64, opt_usize, str_list};
pub use catalog::range_system;
pub use catalog::{
    BrowseReq, CadStepReq, CatalogAddReq, CatalogEditReq, CatalogFillReq, SearchReq,
};
use hist::{change_label, config_only, history_json, snap_was_load, summary, CONFIG_KEY};
use layout::{configs, layout, layout_export, layout_with};
pub use layout::{LayoutExportReq, LayoutReq};
use optim::{apply_pending, optimize, swap, tolerance, zoom, PendingApply};
pub use optim::{
    optimize_cancel, optimize_progress, swap_cancel, swap_progress, tolerance_cancel,
    tolerance_progress,
};
pub use optim::{SwapReq, TolReq};
pub use proto::{Access, METHODS, PROTOCOL};
use replay::replay;
use trace::{plugin, trace};
pub use trace::{PluginInput, PluginReq, PluginValue, TraceReq};

/// 今の系の写し
fn snap(x: &Served) -> Snap {
    Snap {
        sys: x.sys.clone(),
        spec: x.spec.clone(),
        src: x.src.clone(),
        blocks: x.blocks.clone(),
        faces: x.faces.clone(),
        notes: x.notes.clone(),
        log: x.log.clone(),
    }
}

/// 今の系を元に戻す段に積む（深さを超えたら古い段を捨てる。やり直す段は捨てる）
fn push_undo(x: &mut Served, what: Value) {
    let s = snap(x);
    x.hist.record(s, what);
}

/// 変更した系を置き、変更を記録する。前の系は元に戻す段に積む（やり直す段は空にする）
fn store_changed(sess: &Session, sys: System, spec: LoadSpec, c: Change) {
    let mut g = sess.served();
    // 読み込む前の set・edit は current で断るので、ここには来ない（一度置いた系は外さない）
    let Some(mut x) = g.take() else {
        return;
    };
    let now = Instant::now();
    // 構成の切り替えだけの edit が続くときは、間があいても 1 段にまとめる（名前は CONFIG_KEY）
    let names = match &c {
        Change::Set(v) => Some(v.iter().map(|r| r.0.clone()).collect::<Vec<_>>()),
        Change::Edit(o) => config_only(o).then(|| vec![CONFIG_KEY.to_string()]),
    };
    let merge = matches!((&names, &x.last_set), (Some(n), Some((m, t))) if n == m
        && (n[..] == [CONFIG_KEY] || now.duration_since(*t).as_secs_f64() < SET_MERGE_S));
    let s = snap(&x);
    x.hist.record_or_merge(s, change_label(Some(&c)), merge);
    // 面を足す・消すと面の番号がずれるので、ブロックの面の範囲は捨てる
    if sys.surfaces.len() != x.sys.surfaces.len() || moves_surfaces(&c) {
        x.blocks = Arc::default();
    }
    x.last_set = names.map(|n| (n, now));
    x.log.push(c);
    x.sys = Arc::new(sys);
    x.spec = spec;
    *g = Some(x);
}

/// 面を足す・消す edit か（面の番号がずれる）
fn moves_surfaces(c: &Change) -> bool {
    use lensbench_project::edit_ops::EditOp as O;
    matches!(c, Change::Edit(ops) if ops.iter().any(|o| matches!(o, O::InsertSurface { .. } | O::DeleteSurface { .. })))
}

/// 元に戻す（back）・やり直す。戻した後の系の面の数と、残りの段の数
fn step_history(sess: &Session, back: bool) -> Result<Value, RpcErr> {
    let mut g = sess.served();
    let x = g
        .as_mut()
        .ok_or_else(|| RpcErr::app("no system (load first)"))?;
    let cur = snap(x);
    let Some(snap) = x.hist.step(cur, back) else {
        return Err(RpcErr::app(if back {
            "no change to undo"
        } else {
            "no change to redo"
        }));
    };
    let file0 = std::mem::replace(&mut x.spec, snap.spec).file;
    x.sys = snap.sys;
    x.src = snap.src;
    x.blocks = snap.blocks;
    x.faces = snap.faces;
    x.notes = snap.notes;
    x.log = snap.log;
    x.last_set = None;
    let hash = lensbench_project::sysfile::system_hash(&x.sys).map_err(RpcErr::app)?;
    let mut v = history_json(x);
    v["surfaces"] = json!(x.sys.surfaces.len());
    v["system_hash"] = json!(hash);
    // 開く・読み直すを戻したら、戻した系の概要（ファイルが変わる）
    if x.spec.file != file0 || snap_was_load(&v, back) {
        v["loaded"] = summary(&x.sys, &x.spec, &x.notes, &x.blocks);
    }
    Ok(v)
}

/// 元に戻せる段・やり直せる段（`history_json`）
fn history(sess: &Session) -> Value {
    sess.served()
        .as_ref()
        .map_or_else(|| json!(views::HistoryView::default()), history_json)
}

/// 結果に段の数と一覧を足す
fn add_history(sess: &Session, v: &mut Value) {
    if let (Some(o), Value::Object(h)) = (v.as_object_mut(), history(sess)) {
        o.extend(h);
    }
}

/// steps 段まとめて戻す・やり直す（途中で尽きたらそこまで）
fn step_history_n(sess: &Session, back: bool, n: usize) -> Result<Value, RpcErr> {
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
        if let Some(x) = sess.served().as_ref() {
            v["loaded"] = summary(&x.sys, &x.spec, &x.notes, &x.blocks);
        }
    }
    Ok(v)
}

/// JSON-RPC のエラー
pub(crate) struct RpcErr {
    code: i64,
    pub(crate) msg: String,
    /// core の誤りの種類（[`lensbench_core::Error::kind`]。返す誤りの `data.kind`）
    kind: Option<&'static str>,
    /// core の誤りの符号（[`lensbench_core::Error::code`]。返す誤りの `data.code`）
    ecode: Option<&'static str>,
}

impl RpcErr {
    fn app(m: impl Into<String>) -> Self {
        RpcErr {
            code: -32000,
            msg: m.into(),
            kind: None,
            ecode: None,
        }
    }
    /// core の誤りの種類と符号を付ける（文を作り直したとき）
    pub(crate) fn with_core(self, e: &lensbench_core::Error) -> Self {
        RpcErr {
            kind: Some(e.kind()),
            ecode: Some(e.code()),
            ..self
        }
    }
    pub(crate) fn params(m: impl Into<String>) -> Self {
        RpcErr {
            code: -32602,
            msg: m.into(),
            kind: None,
            ecode: None,
        }
    }
}

/// 文字列の誤り（project などの `Err(String)`）は英語で書いてあるので、表示の言語に訳す
impl From<String> for RpcErr {
    fn from(m: String) -> Self {
        RpcErr::app(lensbench_core::i18n::tr(&m))
    }
}

impl From<lensbench_core::Error> for RpcErr {
    fn from(e: lensbench_core::Error) -> Self {
        RpcErr::app(e.to_string()).with_core(&e)
    }
}

/// 展開の誤り（ブロックの名前を文の前に付ける）
impl From<lensbench_project::ExpandError> for RpcErr {
    fn from(e: lensbench_project::ExpandError) -> Self {
        lensbench_core::Error::from(e).into()
    }
}

impl From<RpcErr> for CallError {
    fn from(e: RpcErr) -> Self {
        CallError {
            code: e.code,
            message: e.msg,
            kind: e.kind.map(str::to_string),
            data_code: e.ecode.map(str::to_string),
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

/// 要求を 1 つ処理する（鍵は [`Access`]。panic はエラーにする）
fn dispatch(
    sess: &Session,
    method: &str,
    params: &Value,
    bins: Vec<Bin>,
    threads: Option<usize>,
) -> Result<(Answer, Vec<Bin>), RpcErr> {
    // 進み具合・中断・停止は、処理している要求を待たずに答える（別の接続から）
    if let Some(r) = status(sess, method, params) {
        return r.map(|(v, b)| (Answer::Json(v), b));
    }
    // params を読む所の panic も誤りにする
    let req = catch_panic(|| Request::parse(method, params))?;
    dispatch_req(sess, req, bins, threads)
}

/// f の panic を誤り「internal error (panic)」にする（panic の hook が残した文と場所を添える）
pub(crate) fn catch_panic<T>(f: impl FnOnce() -> Result<T, RpcErr>) -> Result<T, RpcErr> {
    LAST_PANIC.with(|l| l.borrow_mut().take());
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|e| {
        // panic の hook（GUI の crash.rs）が残した文と場所。無ければ panic の文だけ
        let t = LAST_PANIC
            .with(|l| l.borrow_mut().take())
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .or_else(|| e.downcast_ref::<String>().cloned());
        Err(RpcErr::app(match t {
            Some(t) => format!("internal error (panic): {t}"),
            None => "internal error (panic)".into(),
        }))
    })
}

/// 型の要求を 1 つ処理する（鍵は [`Access`]。panic はエラーにする）
fn dispatch_req(
    sess: &Session,
    req: Request,
    bins: Vec<Bin>,
    threads: Option<usize>,
) -> Result<(Answer, Vec<Bin>), RpcErr> {
    let (_r, _w, _l);
    let access = Access::of(req.method());
    match access {
        Access::Read => _r = sess.request.read_ok(),
        Access::Write => _w = sess.request.write_ok(),
        // 長い要求は鍵を持たずに回す（系は始めに写しを取る。結果を当てるのは計算の後、プールの外で）
        Access::Long => _l = sess.long.lock_ok(),
    }
    // 中断の印: 呼び出し側（GUI の Worker）が置いた印があればそれを使う
    let tok = lensbench_core::par::current().cancel.unwrap_or_default();
    sess.running.lock_ok().push(tok.clone());
    struct Done<'a>(&'a Session, lensbench_core::par::CancelToken);
    impl Drop for Done<'_> {
        fn drop(&mut self) {
            let mut r = self.0.running.lock_ok();
            if let Some(i) = r.iter().position(|t| t.same(&self.1)) {
                r.swap_remove(i);
            }
        }
    }
    let _d = Done(sess, tok.clone());
    let run = || {
        catch_panic(|| lensbench_core::par::with_cancel(Some(&tok), || run::run(sess, req, bins)))
    };
    if access != Access::Long {
        return lensbench_core::with_threads(threads, run);
    }
    // 長い要求は別のプールで（読む要求のプールの盗みに載せない）。場の鍵はプールのスレッドでは待たない
    let r = lensbench_core::par::with_threads_apart(threads, run);
    let p = sess.pending_apply.lock_ok().take();
    match (r, p) {
        (Ok((mut v, b)), Some(p)) => {
            apply_pending(sess, p, &mut v);
            Ok((v, b))
        }
        (r, _) => r,
    }
}

/// 処理している要求を待たない method: `progress`（最適化・公差・差し替え探索の進み具合と、要求を処理しているか）、
/// `cancel`（処理している要求に途中でやめるよう頼む。解析は「中断した」の誤りで返る）、
/// `stop`（`what` の最適化・公差・差し替え探索を止め、そこまでの結果を返させる）
fn status(sess: &Session, method: &str, p: &Value) -> Option<Result<Reply, RpcErr>> {
    let r = match method {
        "progress" => json!({
            "running": !sess.running.lock_ok().is_empty(),
            "optimize": sess.optimize_progress(),
            "tolerance": sess.tolerance_progress(),
            "swap": sess.swap_progress(),
        }),
        "cancel" => {
            let r = sess.running.lock_ok();
            r.iter().for_each(|t| t.cancel());
            json!(!r.is_empty())
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
                    "stop what must be optimize, tolerance or swap",
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
    /// core の誤りの符号（`lensbench_core::error::code` の値か種類の名前。serve は `data.code` に書く）
    pub data_code: Option<String>,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// 結果の配列（`trace` の x・y・z・opl・err・vig など。名前と f64 の並び、整数も f64 にする）
pub type Arrays = Vec<(String, Vec<f64>)>;

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
        let (a, arrays) = self.call_answer(method, params, threads)?;
        Ok((
            a.into_json()
                .map_err(|e| CallError::from(RpcErr::from(e)))?,
            arrays,
        ))
    }

    /// [`call_arrays`] と同じで、結果の型が決まっている method（面の表・最適化・差し替え探索・公差・ズーム軌跡）は
    /// JSON を通さずに型のまま返す（[`Answer`]。GUI が使う）
    pub fn call_answer(
        &self,
        method: &str,
        params: &Value,
        threads: Option<usize>,
    ) -> Result<(Answer, Arrays), CallError> {
        dispatch(self, method, params, Vec::new(), threads)
            .map(arrays_of)
            .map_err(CallError::from)
    }

    /// 型の要求を同じプロセスの中から渡す（JSON を通さない。GUI が使う）。答えは [`call_answer`] と同じ
    pub fn request(
        &self,
        req: Request,
        threads: Option<usize>,
    ) -> Result<(Answer, Arrays), CallError> {
        let r = match req {
            // 型を付けていない method は JSON の口で（進み具合・中断・停止は待たずに答える）
            Request::Json { method, params } => {
                dispatch(self, &method, &params, Vec::new(), threads)
            }
            req => dispatch_req(self, req, Vec::new(), threads),
        };
        r.map(arrays_of).map_err(CallError::from)
    }

    /// 系を読んで（`load` と同じ params）、そのレイアウト図（`layout` と同じ結果）を返す。読み込んだ系は変えず、
    /// 他の要求の順番も待たない。GUI が、解析を回している間も組合せの図だけ先に描き直すのに使う
    pub fn preview_layout(&self, load: &Value, opts: &Value) -> Result<Value, CallError> {
        let f = || -> Result<(LoadReq, LayoutReq), RpcErr> {
            Ok((LoadReq::parse(load)?, LayoutReq::parse(opts)?))
        };
        let (l, o) = f().map_err(CallError::from)?;
        self.preview(&l, &o)
    }

    /// [`Session::preview_layout`] の型の口
    pub fn preview(&self, load: &LoadReq, opts: &LayoutReq) -> Result<Value, CallError> {
        let f = || preview_of(self, load, opts);
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
            .unwrap_or_else(|_| Err(RpcErr::app("internal error (panic)")))
            .map_err(CallError::from)
    }

    /// カタログの要求（`glasses`・`lenses`・`catalog_search`・`catalog_items`・`catalog_edit`・`catalog_fill`）を、
    /// 読み込んだ系に関係なく、他の要求の順番も待たずに処理する。GUI が、解析を回している間もカタログを読み・
    /// 下書きの worker からユーザーカタログを変えるのに使う（`catalog_add` は読み込んだ系が要るので [`Session::request`] で）
    pub fn catalog(&self, req: &Request) -> Result<Value, CallError> {
        let sess = self;
        let f = || match req {
            Request::Glasses(r) => catalog::glasses(sess, r),
            Request::Lenses(r) => catalog::lenses(sess, r),
            Request::CatalogSearch(r) => catalog::catalog_search(sess, r),
            Request::CatalogItems { catalog } => catalog::catalog_items(sess, catalog),
            Request::CatalogEdit(r) => catalog::catalog_edit(sess, r),
            Request::CatalogFill(r) => catalog::catalog_fill(sess, r),
            r => {
                let m = r.method();
                Err(RpcErr::params(format!(
                    "{m} must be glasses, lenses, catalog_search, catalog_items, catalog_edit or catalog_fill"
                )))
            }
        };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
            .unwrap_or_else(|_| Err(RpcErr::app("internal error (panic)")))
            .map_err(CallError::from)
    }

    /// [`Session::catalog`] の JSON の口（method と serve の params）
    pub fn browse(&self, method: &str, params: &Value) -> Result<Value, CallError> {
        self.catalog(&Request::from_json(method, params)?)
    }
}

/// [`Session::preview`]（読み込んだ系は変えない）
fn preview_of(sess: &Session, load: &LoadReq, opts: &LayoutReq) -> Result<Value, RpcErr> {
    let spec = load.spec(sess)?;
    let r = read_file_notes(&spec)?;
    let mut v = layout_with(&r.sys, &spec, opts, None)?;
    // 要素ごとの面の範囲（GUI の組合せの図・レンズデータの要素の行）
    v["blocks"] = json!(r.blocks);
    Ok(v)
}

/// 答えの bin を名前と f64 の並びに（整数・真偽も f64、複素数は 名前.re・名前.im の 2 つ）
fn arrays_of((v, bins): (Answer, Vec<Bin>)) -> (Answer, Arrays) {
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

/// [`global`] の場の [`Session::request`]
pub fn request(req: Request, threads: Option<usize>) -> Result<(Answer, Arrays), CallError> {
    global().request(req, threads)
}

/// [`global`] の場の [`Session::call_answer`]
pub fn call_answer(
    method: &str,
    params: &Value,
    threads: Option<usize>,
) -> Result<(Answer, Arrays), CallError> {
    global().call_answer(method, params, threads)
}

/// [`global`] の場の [`Session::preview_layout`]
pub fn preview_layout(load: &Value, opts: &Value) -> Result<Value, CallError> {
    global().preview_layout(load, opts)
}

/// [`global`] の場の [`Session::preview`]
pub fn preview(load: &LoadReq, opts: &LayoutReq) -> Result<Value, CallError> {
    global().preview(load, opts)
}

/// [`global`] の場の [`Session::catalog`]
pub fn catalog(req: &Request) -> Result<Value, CallError> {
    global().catalog(req)
}

/// [`global`] の場の [`Session::browse`]
pub fn browse(method: &str, params: &Value) -> Result<Value, CallError> {
    global().browse(method, params)
}

type Reply = (Value, Vec<Bin>);

/// 型を付けていない method（`echo` と、知らない method の誤り）
fn answer(method: &str, p: &Value, bins: Vec<Bin>) -> Result<(Answer, Vec<Bin>), RpcErr> {
    handle(method, p, bins).map(|(v, b)| (Answer::Json(v), b))
}

fn handle(method: &str, p: &Value, bins: Vec<Bin>) -> Result<Reply, RpcErr> {
    match method {
        // 配列をそのまま返す（往復の時間を測る）
        "echo" => Ok((json!({ "params": p }), bins)),
        "" => Err(RpcErr {
            code: -32600,
            msg: "no method".into(),
            kind: None,
            ecode: None,
        }),
        m => Err(RpcErr {
            code: -32601,
            msg: format!("no method {m}"),
            kind: None,
            ecode: None,
        }),
    }
}

/// set の値: {"values": {"3.t": 10.0}} か {"values": [["3.t", 10.0], ...]}
fn parse_changes(
    s: &System,
    values: &[(String, f64)],
) -> Result<Vec<(lensbench_core::param::Param, f64)>, RpcErr> {
    values.iter().map(|(k, v)| Ok((s.param(k)?, *v))).collect()
}

/// 解析の登録簿の解析（と膜の境界）を、読み込んだ系で実行する。最適化・公差・レイアウトの書き出しは専用のメソッド
fn analyze(sess: &Session, r: &AnalyzeReq) -> Result<Answer, RpcErr> {
    sess.current()?;
    let (cmd, m) = (r.cmd.as_str(), &r.args);
    if cmd == "coatings" {
        return crate::analyze::coatings_loaded(sess, m).map(Answer::Json);
    }
    match crate::analyses().get(cmd).cloned() {
        Some(def) => crate::analyze::run(sess, def.as_ref(), m),
        None => Err(RpcErr::params(match cmd {
            "optimize" | "tolerance" | "swap" | "zoom" | "layout" => format!(
                "{cmd} is not an analyze analysis (use method {})",
                if cmd == "layout" {
                    "layout_export"
                } else {
                    cmd
                }
            ),
            _ => format!(
                "{cmd} is not an analyze analysis (list them with analysis_args or plugins)"
            ),
        })),
    }
}

/// 登録簿の解析に渡す系の読み方（load で選んだもの。別に読んだ系にも読み込んだ後の変更を当てる）で f を呼ぶ
pub(crate) fn with_source<R>(
    sess: &Session,
    f: impl FnOnce(&dyn lensbench_analysis::Source) -> R,
) -> Result<R, RpcErr> {
    let Current {
        sys,
        spec,
        src,
        faces,
        log,
    } = sess.current_full()?;
    let r = |s: System, to: &[Option<FaceId>], l: &str, n: &mut Vec<String>| {
        replay(s, &log, (&faces, to), l, n)
    };
    Ok(f(&crate::source::FileSource::loaded(&spec, src, &sys, &r)))
}

fn is_zmx(f: &Path) -> bool {
    f.extension().is_some_and(|x| x.eq_ignore_ascii_case("zmx"))
}

/// .zmx の多重構成を、表（set_config_count・set_config_row・set_config の edit の操作）にする
fn config_diff(spec: &LoadSpec, src: &Source) -> Result<Value, RpcErr> {
    use lensbench_core::param::{Kind, Param};
    if !is_zmx(&spec.file) {
        return Err(RpcErr::params("config_diff is only for .zmx systems"));
    }
    // 読み込んだときのファイルの中身から（読み直さない）
    let n = src.configurations()?;
    if n < 2 {
        return Err(RpcErr::app("this .zmx has no multiple configurations"));
    }
    let mut systems = Vec::with_capacity(n);
    for k in 1..=n {
        let mut c = spec.clone();
        c.cfg = k;
        systems.push(
            crate::load::load_source(src, &c).map_err(|e| e.at(format!("configuration {k}")))?,
        );
    }
    let m = systems[0].surfaces.len();
    if systems.iter().any(|s| s.surfaces.len() != m) {
        return Err(RpcErr::app(
            "the configurations have different numbers of surfaces (no table)",
        ));
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
        type Get = fn(&lensbench_core::surface::Surface) -> String;
        if differs(&|s| s.surfaces[j].mat.name()) {
            let names: Vec<String> = systems.iter().map(|s| s.surfaces[j].mat.name()).collect();
            ops.push(json!({
                "op": "set_config_materials",
                "param": systems[0].param_name(Param::new(j, Kind::Material)),
                "names": names,
            }));
        }
        if differs(&|s| s.surfaces[j].coat.clone()) {
            let names: Vec<String> = systems.iter().map(|s| s.surfaces[j].coat.clone()).collect();
            ops.push(json!({
                "op": "set_config_coatings",
                "param": systems[0].param_name(Param::new(j, Kind::Coating)),
                "names": names,
            }));
        }
        let checks: [(&str, Get); 3] = [
            ("ignore", |d| d.ignore.to_string()),
            ("semi-diameter", |d| format!("{:?}", d.semi)),
            ("stop", |d| d.stop.to_string()),
        ];
        for (what, f) in checks {
            if differs(&|s| f(&s.surfaces[j])) {
                notes.push(format!(
                    "surface {j}: {what} differs by configuration (can't tabulate)"
                ));
            }
        }
    }
    let differs_sys = |f: &dyn Fn(&System) -> String| {
        let a = f(&systems[0]);
        systems.iter().any(|s| f(s) != a)
    };
    if differs_sys(&|s| format!("{:?}", s.wavelengths)) {
        notes.push("wavelengths differ by configuration (can't tabulate)".into());
    }
    if differs_sys(&|s| format!("{:?}", s.fields)) {
        notes.push("fields differ by configuration (can't tabulate)".into());
    }
    ops.push(json!({ "op": "set_config", "k": spec.cfg.clamp(1, n) }));
    Ok(json!({ "ops": ops, "notes": notes, "rows": ops.len() - 2 }))
}

/// プロジェクトファイル（系ファイルは含めない）
fn is_project(f: &Path) -> bool {
    lensbench_project::open::FileKind::of(f) == lensbench_project::open::FileKind::Project
}
