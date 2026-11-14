//! 計算を GUI の外のスレッドで回す。要求は serve と同じ method・params（[`lensbench_engine::call_arrays`]）。
//!
//! 要求は 1 本のスレッドで順に処理する（serve と同じく系はプロセスに 1 つで、要求は 1 つずつ）。
//! 1 つの要求に複数の呼び出しをまとめられる（スポット図の視野・波長ごとの trace など）。終わると GUI を描き直させる。
//!
//! ブラウザ（wasm）にはスレッドが無いので、Web Worker の中の別の wasm のインスタンスで回す（[`remote`]）。
//! 系（serve の状態）はそちらにあり、ファイルの置き場（[`lensbench_core::vfs`]）は要求・結果と一緒に互いに写す。
//! 中断はできない（系が変われば、古い解析の結果は捨てる）。最適化などの進み具合・停止も Web では効かない。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use web_time::Instant;

use serde_json::Value;

pub use lensbench_engine::Arrays;

/// 要求の目的（結果をどこに入れるか）。u64 はパネルの番号
#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Load,
    Info,
    Analyze(u64),
    /// スポット図の光線（視野・波長ごとの trace）
    Rays(u64),
    Glasses,
    Lenses,
    Get,
    Set,
    Write,
    /// 組合せのレイアウト図
    Layout,
    /// レイアウトをファイルに書き出す（serve の layout_export）
    LayoutExport,
    /// カタログの玉の断面（行の番号）
    LensPreview(usize),
    /// 像面を近軸の焦点へ（軸上の細い光線）
    Focus,
    /// 面の表（全部の面の値と玉・群）
    Surfaces,
    /// 面の表で変えた値（serve の edit）
    Edit,
    /// 系ファイルに保存（serve の save）
    Save,
    /// 選んだ面の範囲を系ファイルに書き出す（serve の save_range）
    SaveRange,
    /// 面の設定（serve の surface_data、面の番号）
    SurfData(usize),
    /// 開いた系の系の設定（serve の system_data）
    SysData,
    /// 多重構成の表（serve の config_table）
    ConfigTable,
    /// .zmx の多重構成を表にする操作（serve の config_diff）
    ConfigDiff,
    /// レイアウト図に重ねられる構成（serve の configs）
    Configs,
    /// ユーザーカタログの保存・一覧・変更（serve の catalog_add・catalog_items・catalog_edit）
    UserCatalog,
    /// 未完のカタログ品を補って保存（serve の catalog_fill）
    CatalogFill,
    /// 最適化（serve の optimize）
    Optimize,
    /// 差し替え探索（serve の swap）
    Swap,
    /// モンテカルロ公差（serve の tolerance）
    Tolerance,
    /// ズーム軌跡（serve の zoom）
    Zoom,
    /// 元に戻す・やり直す（serve の undo・redo）
    History,
    /// 元に戻せる段の数を問い合わせる（serve の history。系を変えないので解析は止めない）
    HistoryDepth,
}

impl Tag {
    /// 解析（系が変われば、待っている分は捨ててよい）
    fn skippable(&self) -> bool {
        matches!(self, Tag::Analyze(_) | Tag::Rays(_))
    }

    /// serve の系を変える要求（出す前に、待っている・回っている解析をやめる）
    fn changes_system(&self) -> bool {
        matches!(self, Tag::Load | Tag::Set | Tag::Edit | Tag::History)
    }
}

struct Job {
    tag: Tag,
    calls: Vec<(String, Value)>,
    /// 出したときの世代（[`Worker::drop_queued`] より前に出した解析は回さない）
    gen: u64,
}

/// 1 つの呼び出しの結果
pub struct Reply {
    pub method: String,
    pub params: Value,
    pub result: Result<(Value, Arrays), String>,
}

pub struct Done {
    pub tag: Tag,
    pub replies: Vec<Reply>,
    pub ms: f64,
    /// 回さずに捨てた、または系が変わる前に出した解析の結果なので捨てた（replies は空）
    pub skipped: bool,
    /// 出したときの世代
    gen: u64,
}

impl Done {
    /// 最初の呼び出しの method・params・結果（1 つだけの要求のとき）
    pub fn first(&self) -> (&str, &Value, Result<&Value, &str>) {
        let r = &self.replies[0];
        (
            &r.method,
            &r.params,
            r.result.as_ref().map(|x| &x.0).map_err(String::as_str),
        )
    }

    /// 最初のエラー
    pub fn error(&self) -> Option<(&str, &str)> {
        self.replies.iter().find_map(|r| {
            r.result
                .as_ref()
                .err()
                .map(|e| (r.method.as_str(), e.as_str()))
        })
    }
}

/// 1 つの呼び出し（method・params・スレッド数）
type CallFn = fn(&str, &Value, Option<usize>) -> Result<(Value, Arrays), String>;

/// 何を受ける Worker か（Web Worker の中でも同じ呼び出しを使う）
#[derive(Clone, Copy)]
pub enum Kind {
    /// serve と同じ要求（系はプロセスに 1 つ）
    Main,
    /// 下書き（[`Worker::preview`]）
    Preview,
    /// 下書きと同じ。Web Worker は要求が来たときに作る（Web 版の別のタブのパネルは計算しない）
    #[cfg(target_arch = "wasm32")]
    Idle,
    /// テスト用の呼び出し
    #[cfg(test)]
    Test(CallFn),
}

impl Kind {
    fn call(self) -> CallFn {
        match self {
            Kind::Main => main_call,
            Kind::Preview => preview_call,
            #[cfg(target_arch = "wasm32")]
            Kind::Idle => preview_call,
            #[cfg(test)]
            Kind::Test(f) => f,
        }
    }

    /// Web Worker へ渡す番号
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn code(self) -> u8 {
        match self {
            Kind::Preview => 1,
            #[cfg(target_arch = "wasm32")]
            Kind::Idle => 1,
            _ => 0,
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn from_code(c: u8) -> Kind {
        if c == 1 {
            Kind::Preview
        } else {
            Kind::Main
        }
    }
}

fn main_call(m: &str, p: &Value, t: Option<usize>) -> Result<(Value, Arrays), String> {
    lensbench_engine::call_arrays(m, p, t).map_err(|e| e.message)
}

fn preview_call(m: &str, p: &Value, _: Option<usize>) -> Result<(Value, Arrays), String> {
    match m {
        "preview" => lensbench_engine::preview_layout(&p["load"], &p["layout"])
            .map(|v| (v, Vec::new()))
            .map_err(|e| e.message),
        "glasses" | "lenses" => lensbench_engine::browse(m, p)
            .map(|v| (v, Vec::new()))
            .map_err(|e| e.message),
        // ユーザーカタログの一覧・変更（読み込んだ系は要らない）
        "catalog_items" | "catalog_edit" | "catalog_fill" => {
            lensbench_engine::user_catalog_edit(m, p)
                .map(|v| (v, Vec::new()))
                .map_err(|e| e.message)
        }
        _ => Err(format!("{m} は下書きでは使えない")),
    }
}

pub struct Worker {
    tx: Sender<Job>,
    rx: Receiver<Done>,
    /// 出して、まだ結果を受け取っていない要求の数
    pending: usize,
    /// そのうち解析でないもの（系の読み込み・レイアウトなど）
    pending_core: usize,
    gen: Arc<AtomicU64>,
    /// 回している要求が解析か（中断を頼んでよいか）。系を変える側と回す側でこの鍵を取って、
    /// 「解析を回している間だけ中断の印を立てる」を確かにする（最適化などの中の解析を止めない）
    running: Arc<Mutex<bool>>,
    /// 中断の印（lensbench_core::par の、プロセスで 1 つ）を使うか。下書きの Worker は使わない
    cancels: bool,
    #[cfg(target_arch = "wasm32")]
    remote: remote::Remote,
}

/// 1 つの要求を回す（計算のスレッドで）
#[cfg(not(target_arch = "wasm32"))]
struct Runner {
    call: CallFn,
    threads: Option<usize>,
    cancels: bool,
    /// 今の世代
    cur: Arc<AtomicU64>,
    run_flag: Arc<Mutex<bool>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Runner {
    fn run(&self, j: Job) -> Done {
        if self.cancels {
            // 前の要求のための中断の印を下ろしてから、世代を確かめる（この間に系が変われば印が残る）
            let mut r = self.run_flag.lock().unwrap_or_else(|e| e.into_inner());
            *r = j.tag.skippable();
            lensbench_core::par::clear_cancel();
        }
        if j.tag.skippable() && j.gen != self.cur.load(Ordering::Relaxed) {
            return Done {
                tag: j.tag,
                replies: Vec::new(),
                ms: 0.0,
                skipped: true,
                gen: j.gen,
            };
        }
        let t0 = Instant::now();
        let mut replies = Vec::with_capacity(j.calls.len());
        for (method, params) in j.calls {
            let result = (self.call)(&method, &params, self.threads);
            let failed = result.is_err();
            replies.push(Reply {
                method,
                params,
                result,
            });
            if failed {
                break;
            }
        }
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        if self.cancels {
            *self.run_flag.lock().unwrap_or_else(|e| e.into_inner()) = false;
        }
        Done {
            tag: j.tag,
            replies,
            ms,
            skipped: false,
            gen: j.gen,
        }
    }
}

impl Worker {
    /// `repaint` は結果が出たときに呼ぶ（GUI を描き直させる）
    pub fn new(threads: Option<usize>, repaint: impl Fn() + Send + 'static) -> Worker {
        Self::with_call(threads, repaint, true, Kind::Main)
    }

    /// レイアウト図の下書きとカタログの一覧用（`preview`・`glasses`・`lenses`・`catalog_items`・`catalog_edit` だけを受ける。serve の系・順番待ちとは
    /// 別に回すので、解析を回している間も図とカタログが出る）
    pub fn preview(repaint: impl Fn() + Send + 'static) -> Worker {
        Self::with_call(None, repaint, false, Kind::Preview)
    }

    /// 計算しない Worker（Web 版の別のタブのパネル。要求を出せば下書きとして回す）
    #[cfg(target_arch = "wasm32")]
    pub fn idle() -> Worker {
        Self::with_call(None, || {}, false, Kind::Idle)
    }

    fn with_call(
        threads: Option<usize>,
        repaint: impl Fn() + Send + 'static,
        cancels: bool,
        kind: Kind,
    ) -> Worker {
        let (tx, jobs) = channel::<Job>();
        let (done, rx) = channel::<Done>();
        let gen = Arc::new(AtomicU64::new(0));
        let running = Arc::new(Mutex::new(false));
        #[cfg(not(target_arch = "wasm32"))]
        let runner = Runner {
            call: kind.call(),
            threads,
            cancels,
            cur: gen.clone(),
            run_flag: running.clone(),
        };
        // ネイティブは別のスレッドで順に回す
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new()
            .name("lensbench-worker".into())
            .spawn(move || {
                for j in jobs {
                    let d = runner.run(j);
                    let skipped = d.skipped;
                    if done.send(d).is_err() {
                        break;
                    }
                    if !skipped {
                        repaint();
                    }
                }
            })
            .expect("計算のスレッドを作れない");
        Worker {
            tx,
            rx,
            pending: 0,
            pending_core: 0,
            gen,
            running,
            cancels,
            // ブラウザ（wasm）: Web Worker の中で回す
            #[cfg(target_arch = "wasm32")]
            remote: remote::Remote::new(kind, threads, jobs, done, Box::new(repaint)),
        }
    }

    pub fn send(&mut self, tag: Tag, method: &str, params: Value) {
        self.send_batch(tag, vec![(method.into(), params)]);
    }

    /// 複数の呼び出しを 1 つの要求として順に（途中でエラーになればそこで止める）
    pub fn send_batch(&mut self, tag: Tag, calls: Vec<(String, Value)>) {
        if calls.is_empty() {
            return;
        }
        let core = !tag.skippable();
        // 系を変えるなら、それより前に出した解析は要らない（待っている分は捨て、回っている分はやめる）
        if tag.changes_system() {
            self.drop_queued();
        }
        let gen = self.gen.load(Ordering::Relaxed);
        if self.tx.send(Job { tag, calls, gen }).is_ok() {
            self.pending += 1;
            if core {
                self.pending_core += 1;
            }
        }
    }

    /// 終わった要求（無ければ None）
    pub fn poll(&mut self) -> Option<Done> {
        // wasm: Web Worker の結果を受け取り、空いていれば次の要求を渡す
        #[cfg(target_arch = "wasm32")]
        self.remote.pump(self.gen.load(Ordering::Relaxed));
        let mut d = self.rx.try_recv().ok()?;
        self.pending -= 1;
        if !d.tag.skippable() {
            self.pending_core -= 1;
        } else if d.gen != self.gen.load(Ordering::Relaxed) {
            // 回している間に系が変わった（古い系の結果、または中断した）
            d.skipped = true;
            d.replies.clear();
        }
        Some(d)
    }

    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    /// 解析でない要求（系の読み込み・レイアウトなど）を待っている
    pub fn busy_core(&self) -> bool {
        self.pending_core > 0
    }

    /// 待っている解析を捨て、回している解析には途中でやめるよう頼む（結果は [`Worker::poll`] で捨てる）。
    /// 系を変える要求（[`Tag::changes_system`]）を出すときは自動で呼ぶ
    pub fn drop_queued(&self) {
        let r = self.running.lock().unwrap_or_else(|e| e.into_inner());
        self.gen.fetch_add(1, Ordering::Relaxed);
        if self.cancels && *r {
            lensbench_core::par::request_cancel();
        }
        // Web 版: 計算の Web Worker の見張りに頼む（スレッドありの版だけ）
        #[cfg(target_arch = "wasm32")]
        if self.cancels {
            self.remote.cancel_running();
        }
    }
}

/// 最適化・公差・差し替え探索の進み具合と停止。ネイティブは同じプロセスの serve
/// （[`lensbench_engine::optimize_progress`] など）、Web 版は計算の Web Worker の見張りのスレッドに訊く
/// （スレッドありの版だけ。1 スレッドの版では進み具合は出ず、止められない）
pub mod progress {
    /// 最適化の反復ごとの評価関数の値（走っていなければ None）
    pub fn optimize() -> Option<Vec<f64>> {
        #[cfg(not(target_arch = "wasm32"))]
        return lensbench_engine::optimize_progress();
        #[cfg(target_arch = "wasm32")]
        super::remote::PROGRESS
            .with(|p| serde_json::from_value(p.borrow()["optimize"].clone()).ok())
    }

    /// 公差の（終えた数、全体の数）
    pub fn tolerance() -> Option<(usize, usize)> {
        #[cfg(not(target_arch = "wasm32"))]
        return lensbench_engine::tolerance_progress();
        #[cfg(target_arch = "wasm32")]
        super::remote::PROGRESS
            .with(|p| serde_json::from_value(p.borrow()["tolerance"].clone()).ok())
    }

    /// 差し替え探索の（終えた数、全体の数）
    pub fn swap() -> Option<(usize, usize)> {
        #[cfg(not(target_arch = "wasm32"))]
        return lensbench_engine::swap_progress();
        #[cfg(target_arch = "wasm32")]
        super::remote::PROGRESS.with(|p| serde_json::from_value(p.borrow()["swap"].clone()).ok())
    }

    /// 最適化（"optimize"）・公差（"tolerance"）・差し替え探索（"swap"）を止める（そこまでの結果を返す）
    pub fn stop(what: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        match what {
            "optimize" => lensbench_engine::optimize_cancel(),
            "tolerance" => lensbench_engine::tolerance_cancel(),
            "swap" => lensbench_engine::swap_cancel(),
            _ => {}
        }
        #[cfg(target_arch = "wasm32")]
        super::remote::stop(what);
    }
}

/// Web Worker の中（計算の側）。lensbench-web の `worker_init`・`worker_call` から呼ぶ
#[cfg(target_arch = "wasm32")]
pub mod entry {
    use super::*;
    use crate::wire::{Request, Response};
    use lensbench_core::vfs::sync;
    use std::cell::Cell;

    thread_local! {
        static KIND: Cell<u8> = const { Cell::new(0) };
    }

    /// 何を受けるか（[`Kind::code`]）
    pub fn init(kind: u8) {
        crate::crash::install();
        KIND.with(|k| k.set(kind));
    }

    /// 回している要求の番号（0 は無し）。計算のスレッドと見張りのスレッド（[`cancel`]・[`stop`]、
    /// スレッドありの版だけ。メモリを共有する）で、この鍵を取って「回している要求だけを止める」を確かにする
    static CUR: std::sync::Mutex<u32> = std::sync::Mutex::new(0);

    fn cur() -> std::sync::MutexGuard<'static, u32> {
        CUR.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 見張りのスレッドから: 要求 id を回していれば、解析に途中でやめるよう頼む
    pub fn cancel(id: u32) {
        let c = cur();
        if *c == id && id != 0 {
            lensbench_core::par::request_cancel();
        }
    }

    /// 見張りのスレッドから: 要求 id を回していれば、最適化（what = "optimize"）・公差（"tolerance"）・
    /// 差し替え探索（"swap"）を止める（そこまでの結果を返す）
    pub fn stop(id: u32, what: &str) {
        let c = cur();
        if *c != id || id == 0 {
            return;
        }
        match what {
            "optimize" => lensbench_engine::optimize_cancel(),
            "tolerance" => lensbench_engine::tolerance_cancel(),
            "swap" => lensbench_engine::swap_cancel(),
            _ => {}
        }
    }

    /// 見張りのスレッドから: 最適化・公差・差し替え探索の進み具合（[`super::progress`] の形の JSON）
    pub fn progress() -> String {
        serde_json::json!({
            "optimize": lensbench_engine::optimize_progress(),
            "tolerance": lensbench_engine::tolerance_progress(),
            "swap": lensbench_engine::swap_progress(),
        })
        .to_string()
    }

    /// 要求（[`Request`] のバイト列）を回して、結果（[`Response`]）を返す
    pub fn call(b: &[u8]) -> Vec<u8> {
        let q = match Request::decode(b) {
            Ok(q) => q,
            Err(e) => {
                return Response {
                    results: vec![Err(e)],
                    ms: 0.0,
                    files: Vec::new(),
                }
                .encode()
            }
        };
        let main = KIND.with(Cell::get) == Kind::Main.code();
        if main {
            // 前の要求のための中断の印を下ろす（見張りは番号が合うときだけ印を立てる）。鍵を持ったまま下ろす
            // （番号を変えてから下ろすまでの間に、この要求への中断が届いて消えないように）
            let mut c = cur();
            *c = q.id;
            lensbench_core::par::clear_cancel();
            drop(c);
        }
        // 画面の側で変わったファイルを当ててから回す（当てた分は返さない）
        for (p, b) in q.files {
            sync::apply(&p, b);
        }
        let seq = sync::seq();
        let call = Kind::from_code(KIND.with(Cell::get)).call();
        let t0 = Instant::now();
        let mut results = Vec::with_capacity(q.calls.len());
        for (m, p) in &q.calls {
            let r = call(m, p, q.threads);
            let failed = r.is_err();
            results.push(r);
            if failed {
                break;
            }
        }
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        if main {
            *cur() = 0;
        }
        let files = sync::changes_since(seq)
            .into_iter()
            .map(|(p, b, _)| (p, b))
            .collect();
        Response { results, ms, files }.encode()
    }
}

/// 画面の側: 置き場のこの版より後の変更を、計算の Web Worker に渡す（それまでは Worker も持っている見本）。
/// 起動の前に 1 回（[`crate::start_web`]）
#[cfg(target_arch = "wasm32")]
pub fn set_sync_base(seq: u64) {
    remote::BASE.with(|b| b.set(Some(seq)));
}

/// 画面の側: Web Worker（`worker.js`、中は同じ wasm）に要求を 1 つずつ渡す
#[cfg(target_arch = "wasm32")]
mod remote {
    use super::*;
    use crate::wire::{Request, Response};
    use lensbench_core::vfs::sync;
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    type Inbox = Rc<RefCell<VecDeque<Result<Vec<u8>, String>>>>;

    thread_local! {
        /// [`super::set_sync_base`]
        pub static BASE: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
        /// 計算の Web Worker（serve と同じ要求を受ける方）の見張りのスレッドへの口（スレッドありの版だけ）
        pub static MONITOR: RefCell<Option<Monitor>> = const { RefCell::new(None) };
        /// 見張りから受け取った進み具合（{"optimize", "tolerance", "swap"}）
        pub static PROGRESS: RefCell<Value> = const { RefCell::new(Value::Null) };
        /// 回している要求の番号（0 は無し）
        static RUNNING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    /// 見張りのスレッド（`monitor.js`）への口
    pub struct Monitor {
        port: web_sys::MessagePort,
        _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
        /// 最後に進み具合を訊いた時刻 [ms]
        asked: f64,
    }

    fn now_ms() -> f64 {
        js_sys::Date::now()
    }

    /// 見張りに渡す（{"cancel": id} など）
    fn tell(v: Value) {
        MONITOR.with(|m| {
            if let Some(m) = &*m.borrow() {
                let o = js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL);
                let _ = m.port.post_message(&o);
            }
        });
    }

    /// [`super::progress::stop`]
    pub fn stop(what: &str) {
        let id = RUNNING.with(|r| r.get());
        if id != 0 {
            tell(serde_json::json!({ "stop": what, "id": id }));
        }
    }

    /// 見張りの口を受け取る（計算の Web Worker が {monitor: MessagePort} を送ってくる）
    fn set_monitor(port: web_sys::MessagePort, repaint: Rc<dyn Fn()>) {
        let on_message = Closure::<dyn FnMut(_)>::new(move |e: web_sys::MessageEvent| {
            if let Some(s) = e.data().as_string() {
                let v: Value = serde_json::from_str(&s).unwrap_or(Value::Null);
                // 要求が終わった後に届いた古い進み具合は捨てる
                if RUNNING.with(|r| r.get()) != 0 {
                    PROGRESS.with(|p| *p.borrow_mut() = v);
                    repaint();
                }
            }
        });
        port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        MONITOR.with(|m| {
            *m.borrow_mut() = Some(Monitor {
                port,
                _on_message: on_message,
                asked: 0.0,
            })
        });
    }

    /// 動いている Web Worker
    struct Live {
        w: web_sys::Worker,
        _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
        _on_error: Closure<dyn FnMut(web_sys::Event)>,
    }

    impl Drop for Live {
        fn drop(&mut self) {
            self.w.terminate();
        }
    }

    pub struct Remote {
        kind: Kind,
        threads: Option<usize>,
        jobs: Receiver<Job>,
        done: Sender<Done>,
        repaint: Rc<dyn Fn()>,
        live: Option<Live>,
        inbox: Inbox,
        /// 渡して、結果を待っている要求とその番号
        inflight: Option<Job>,
        inflight_id: u32,
        next_id: u32,
        /// この版より後の置き場の変更を、次の要求と一緒に渡す
        seq: u64,
        /// Web Worker から受け取って当てたファイル（パス → 当てた版）。送り返さない
        echo: HashMap<String, u64>,
    }

    impl Remote {
        pub fn new(
            kind: Kind,
            threads: Option<usize>,
            jobs: Receiver<Job>,
            done: Sender<Done>,
            repaint: Box<dyn Fn()>,
        ) -> Remote {
            let mut r = Remote {
                kind,
                threads,
                jobs,
                done,
                repaint: Rc::from(repaint),
                live: None,
                inbox: Rc::default(),
                inflight: None,
                inflight_id: 0,
                next_id: 0,
                seq: BASE.with(|b| b.get()).unwrap_or_else(sync::seq),
                echo: HashMap::new(),
            };
            if !matches!(kind, Kind::Idle) {
                r.spawn();
            }
            r
        }

        /// Web Worker を作る（中の wasm は見本のファイルだけを持って始まる）
        fn spawn(&mut self) {
            self.live = None;
            self.inbox.borrow_mut().clear();
            let opts = web_sys::WorkerOptions::new();
            opts.set_type(web_sys::WorkerType::Module);
            // ページ（index.html）が globalThis.lensbenchWorker に URL を置く（スレッドありの版は worker.js?mt）
            let url = js_sys::Reflect::get(&js_sys::global(), &"lensbenchWorker".into())
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| "worker.js".into());
            let w = match web_sys::Worker::new_with_options(&url, &opts) {
                Ok(w) => w,
                Err(e) => {
                    self.inbox
                        .borrow_mut()
                        .push_back(Err(format!("Web Worker を作れない: {e:?}")));
                    return;
                }
            };
            let (inbox, repaint) = (self.inbox.clone(), self.repaint.clone());
            let rp = self.repaint.clone();
            let on_message = Closure::<dyn FnMut(_)>::new(move |e: web_sys::MessageEvent| {
                let d = e.data();
                // 見張りのスレッドの口（スレッドありの版の、serve と同じ要求を受ける Worker だけが送る）
                if let Some(p) = js_sys::Reflect::get(&d, &"monitor".into())
                    .ok()
                    .and_then(|p| p.dyn_into::<web_sys::MessagePort>().ok())
                {
                    set_monitor(p, rp.clone());
                    return;
                }
                let x = match d.dyn_ref::<js_sys::Uint8Array>() {
                    Some(a) => Ok(a.to_vec()),
                    None => Err(js_sys::Reflect::get(&d, &"error".into())
                        .ok()
                        .and_then(|v| v.as_string())
                        .unwrap_or_else(|| format!("{d:?}"))),
                };
                inbox.borrow_mut().push_back(x);
                repaint();
            });
            let (inbox, repaint) = (self.inbox.clone(), self.repaint.clone());
            let on_error = Closure::<dyn FnMut(_)>::new(move |e: web_sys::Event| {
                let m = e
                    .dyn_ref::<web_sys::ErrorEvent>()
                    .map(|e| e.message())
                    .unwrap_or_else(|| "Web Worker のエラー".into());
                inbox.borrow_mut().push_back(Err(m));
                repaint();
            });
            w.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            w.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            let _ = w.post_message(&JsValue::from(self.kind.code()));
            self.live = Some(Live {
                w,
                _on_message: on_message,
                _on_error: on_error,
            });
        }

        /// 結果を受け取り、空いていれば次の要求を渡す。`gen` は今の世代（古い解析は渡さない）
        pub fn pump(&mut self, gen: u64) {
            // 今ある分だけ（作り直しのエラーは次のフレームで）
            let msgs: Vec<_> = self.inbox.borrow_mut().drain(..).collect();
            for m in msgs {
                self.receive(m);
            }
            // 回している間は見張りに進み具合を訊く（0.2 秒ごと）
            if matches!(self.kind, Kind::Main) && self.inflight.is_some() {
                let ask = MONITOR.with(|m| {
                    let mut m = m.borrow_mut();
                    let m = m.as_mut()?;
                    let t = now_ms();
                    (t - m.asked >= 200.0).then(|| m.asked = t)
                });
                if ask.is_some() {
                    tell(serde_json::json!({ "progress": 1 }));
                }
            }
            while self.inflight.is_none() {
                let Ok(j) = self.jobs.try_recv() else { break };
                if j.tag.skippable() && j.gen != gen {
                    let _ = self.done.send(Done {
                        tag: j.tag,
                        replies: Vec::new(),
                        ms: 0.0,
                        skipped: true,
                        gen: j.gen,
                    });
                    continue;
                }
                self.post(j);
            }
        }

        fn post(&mut self, j: Job) {
            // 止まった後は、次の要求のときに作り直す（要求ごとに 1 回だけ。作れ続けなくても回り続けない）
            if self.live.is_none() {
                self.spawn();
            }
            let Some(live) = &self.live else {
                // 作れなかった（エラーは inbox にある）
                self.inflight = Some(j);
                return;
            };
            let now = sync::seq();
            let files = sync::changes_since(self.seq)
                .into_iter()
                .filter(|(p, _, v)| self.echo.get(p) != Some(v))
                .map(|(p, b, _)| (p, b))
                .collect();
            self.seq = now;
            self.echo.clear();
            self.next_id = self.next_id.wrapping_add(1).max(1);
            self.inflight_id = self.next_id;
            if matches!(self.kind, Kind::Main) {
                RUNNING.with(|r| r.set(self.next_id));
            }
            let b = Request {
                id: self.next_id,
                threads: self.threads,
                calls: j.calls.clone(),
                files,
            }
            .encode();
            let a = js_sys::Uint8Array::from(&b[..]);
            if let Err(e) = live
                .w
                .post_message_with_transfer(&a, &js_sys::Array::of1(&a.buffer()))
            {
                self.inbox
                    .borrow_mut()
                    .push_back(Err(format!("Web Worker に渡せない: {e:?}")));
            }
            self.inflight = Some(j);
        }

        /// 回している解析に途中でやめるよう、見張りに頼む（解析でない要求は止めない）
        pub fn cancel_running(&self) {
            if self.inflight.as_ref().is_some_and(|j| j.tag.skippable()) {
                tell(serde_json::json!({ "cancel": self.inflight_id }));
            }
        }

        fn receive(&mut self, msg: Result<Vec<u8>, String>) {
            if matches!(self.kind, Kind::Main) {
                RUNNING.with(|r| r.set(0));
                PROGRESS.with(|p| *p.borrow_mut() = Value::Null);
            }
            let resp = msg.and_then(|b| Response::decode(&b));
            let Some(j) = self.inflight.take() else {
                // 要求を待っていないときのエラー（起動の失敗など）: 止めて、次の要求で作り直す
                if let Err(e) = resp {
                    web_sys::console::error_1(&e.into());
                    self.stop();
                }
                return;
            };
            let (results, ms) = match resp {
                Ok(r) => {
                    for (p, b) in r.files {
                        let v = sync::apply(&p, b);
                        self.echo.insert(p, v);
                    }
                    (r.results, r.ms)
                }
                Err(e) => {
                    // 計算の側が止まった（panic など）。系は失われるので、次の要求で作り直す
                    self.stop();
                    (
                        vec![Err(format!(
                            "計算が止まりました（系を開き直してください）: {e}"
                        ))],
                        0.0,
                    )
                }
            };
            let replies = j
                .calls
                .into_iter()
                .zip(results)
                .map(|((method, params), result)| Reply {
                    method,
                    params,
                    result,
                })
                .collect();
            let _ = self.done.send(Done {
                tag: j.tag,
                replies,
                ms,
                skipped: false,
                gen: j.gen,
            });
        }

        /// Web Worker を止める（次の要求で作り直し、置き場は全部写し直す）
        fn stop(&mut self) {
            if matches!(self.kind, Kind::Main) {
                MONITOR.with(|m| *m.borrow_mut() = None);
                RUNNING.with(|r| r.set(0));
            }
            self.live = None;
            self.seq = 0;
            self.echo.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    static STARTED: AtomicBool = AtomicBool::new(false);

    /// 中断を頼まれるまで回る解析（"spin"）と、すぐ終わる要求
    fn fake(m: &str, _p: &Value, _t: Option<usize>) -> Result<(Value, Arrays), String> {
        if m == "spin" {
            STARTED.store(true, Ordering::SeqCst);
            let t0 = Instant::now();
            while !lensbench_core::par::cancelled() {
                if t0.elapsed() > Duration::from_secs(2) {
                    return Ok((Value::from("timeout"), Vec::new()));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            return Err("cancelled".into());
        }
        Ok((Value::from(m), Vec::new()))
    }

    fn wait_all(w: &mut Worker, n: usize) -> Vec<Done> {
        let t0 = Instant::now();
        let mut out = Vec::new();
        while out.len() < n && t0.elapsed() < Duration::from_secs(10) {
            match w.poll() {
                Some(d) => out.push(d),
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        out
    }

    fn wait_started() {
        let t0 = Instant::now();
        while !STARTED.load(Ordering::SeqCst) && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(1));
        }
        STARTED.store(false, Ordering::SeqCst);
    }

    // 中断の印はプロセスで 1 つなので、1 つのテストの中で順に確かめる
    #[test]
    fn changing_the_system_cancels_and_drops_analyses_but_not_other_jobs() {
        let mut w = Worker::with_call(Some(1), || {}, true, Kind::Test(fake));
        // 回っている解析は中断され、待っていた解析は回らず、どちらも捨てた扱い。系の変更は回る
        w.send(Tag::Analyze(1), "spin", Value::Null);
        w.send(Tag::Analyze(2), "info", Value::Null);
        wait_started();
        w.send(Tag::Edit, "edit", Value::Null);
        w.send(Tag::Analyze(3), "info", Value::Null);
        let d = wait_all(&mut w, 4);
        let got: Vec<(Tag, bool)> = d.iter().map(|d| (d.tag.clone(), d.skipped)).collect();
        assert_eq!(
            got,
            vec![
                (Tag::Analyze(1), true),
                (Tag::Analyze(2), true),
                (Tag::Edit, false),
                (Tag::Analyze(3), false),
            ]
        );
        assert!(!w.busy());
        // 解析でない長い要求（最適化など）は系の変更で止めない（中の解析も止めない）
        w.send(Tag::Optimize, "spin", Value::Null);
        wait_started();
        w.send(Tag::Set, "set", Value::Null);
        let d = wait_all(&mut w, 2);
        assert_eq!(d[0].tag, Tag::Optimize);
        assert!(!d[0].skipped);
        assert_eq!(d[0].first().2, Ok(&Value::from("timeout")));
        assert_eq!(d[1].tag, Tag::Set);
    }
}
