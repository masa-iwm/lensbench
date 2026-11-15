//! 計算を GUI の外のスレッドで回す。要求は serve と同じ method・params（[`lensbench_engine::call_arrays`]）。
//!
//! 要求は 1 本のスレッドで順に処理する。最適化・公差・差し替え探索・ズーム軌跡（[`Tag::long`]）だけは別のスレッドで回し、
//! その間も面の表・レイアウト・解析を止めない（serve は始めたときの系で回す。GAR-20）。
//! 1 つの要求に複数の呼び出しをまとめられる（スポット図の視野・波長ごとの trace など）。終わると GUI を描き直させる。
//!
//! ブラウザ（wasm）にはスレッドが無いので、Web Worker の中の別の wasm のインスタンスで回す（[`remote`]）。
//! 系（serve の状態）はそちらにあり、ファイルの置き場（[`lensbench_core::vfs`]）は要求・結果と一緒に互いに写す。
//! 中断はできない（系が変われば、古い解析の結果は捨てる）。最適化などの進み具合・停止も Web では効かない。

use lensbench_core::LockExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use web_time::Instant;

use lensbench_core::par::CancelToken;
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
    /// 系を変えた後の表の読み直し（面の表・要約・多重構成の表を 1 つの要求で。[`Done::split`] で分ける、GAR-05）
    Refresh,
    /// カタログのフォルダの様子（下書きの folders。AGF の名前・見つからないフォルダ）
    Folders,
    /// 取った品の STEP と簡易外形の照合（下書きの cad_measure、Web 版）
    CadMeasure,
    /// 開くプロジェクトの .json の gui・blocks を見る（下書きの doc_peek）。true は今の組合せを消してよいと聞いた後
    OpenDoc(bool),
}

impl Tag {
    /// 解析（系が変われば、待っている分は捨ててよい）
    fn skippable(&self) -> bool {
        matches!(self, Tag::Analyze(_) | Tag::Rays(_))
    }

    /// 利用者の操作でなく、画面が自分で頼む要求（解析・一覧・表の読み直し）。うまくいっても下の行の文を
    /// 上書きしない（「元に戻した」が続く解析の完了で消えない、GUX-27）
    pub fn quiet(&self) -> bool {
        matches!(
            self,
            Tag::Info
                | Tag::Analyze(_)
                | Tag::Rays(_)
                | Tag::Glasses
                | Tag::Lenses
                | Tag::Get
                | Tag::Layout
                | Tag::LensPreview(_)
                | Tag::Surfaces
                | Tag::SurfData(_)
                | Tag::SysData
                | Tag::ConfigTable
                | Tag::Configs
                | Tag::HistoryDepth
                | Tag::Refresh
                | Tag::Folders
        )
    }

    /// [`Tag::Refresh`] の呼び出しの結果の行き先
    pub fn of_refresh(method: &str) -> Tag {
        match method {
            "surface" => Tag::Surfaces,
            "config_table" => Tag::ConfigTable,
            _ => Tag::Info,
        }
    }

    /// 長い計算（ネイティブは別のスレッドで回し、[`Worker::busy_core`]・[`Worker::busy_short`] に数えない）
    fn long(&self) -> bool {
        matches!(self, Tag::Optimize | Tag::Swap | Tag::Tolerance | Tag::Zoom)
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

    /// 最初の呼び出しの params と結果を取り出す（深い複製をしない。GAR-08）。エラーなら結果は null。
    /// 取り出した後の replies[0] の params・結果の値は null（method と配列は残る）。
    /// 光線（Rays・Focus）は返事ごとの params・配列を見るので取り出さず、null を返す
    pub fn take_first(&mut self) -> (Value, Value) {
        if matches!(self.tag, Tag::Rays(_) | Tag::Focus) {
            return (Value::Null, Value::Null);
        }
        let r = &mut self.replies[0];
        let v = match &mut r.result {
            Ok((v, _)) => v.take(),
            Err(_) => Value::Null,
        };
        (r.params.take(), v)
    }

    /// 返事ごとの結果に分ける（[`Tag::Refresh`]: surface → Surfaces、analyze → Info、config_table → ConfigTable）
    pub fn split(self) -> Vec<Done> {
        let (ms, gen) = (self.ms, self.gen);
        self.replies
            .into_iter()
            .map(|r| Done {
                tag: Tag::of_refresh(&r.method),
                replies: vec![r],
                ms,
                skipped: false,
                gen,
            })
            .collect()
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

    /// Web Worker へ渡す番号（`entry::init`）。言語の位 `LANG_JA` は足さない
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
        // カタログのフォルダの様子（Web 版の画面はメーカーのデータを持たないので、こちらで調べる、GAR-11）
        "folders" => Ok((folders(p), Vec::new())),
        "cad_measure" => crate::cad_fetch::measure_call(p).map(|v| (v, Vec::new())),
        // 開くファイルが組合せを同梱しているか（画面のスレッドで丸ごと読まない、GAR-30）
        "doc_peek" => doc_peek(p).map(|v| (v, Vec::new())),
        _ => Err(format!("{m} is not available for drafts")),
    }
}

/// 下書きの Worker の `doc_peek` {file}: プロジェクトの .json の `gui`（組合せ・パネルの設定）と、ブロックで
/// 書いたか（`blocks`）。組合せを同梱していないブロックのプロジェクトで、組合せで同じ系を書ければその組合せ（`combo`）
fn doc_peek(p: &Value) -> Result<Value, String> {
    let f = p["file"].as_str().ok_or("doc_peek needs file")?;
    let b = lensbench_core::vfs::read(f).map_err(|e| format!("{f}: {e}"))?;
    let v: Value = serde_json::from_slice(&b).map_err(|e| format!("{f}: {e}"))?;
    let mut out = serde_json::json!({
        "gui": v.get("gui").cloned().unwrap_or(Value::Null),
        "blocks": v.get("blocks").is_some(),
    });
    // 組合せを同梱していないブロックのプロジェクトは、組合せで書ければ組合せにする（文書モデル 1 段目、GUX-11）
    if v.get("blocks").is_some() && v.pointer("/gui/combo").is_none() {
        let base = std::path::Path::new(f)
            .parent()
            .unwrap_or(std::path::Path::new(""));
        if let Ok(c) = crate::combo::Combo::from_project(&v, base) {
            out["combo"] = serde_json::to_value(c).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

#[test]
fn doc_peek_reads_gui_and_blocks() {
    let dir = lensbench_core::vfs::temp_dir().join("lensbench-gui-doc-peek");
    lensbench_core::vfs::create_dir_all(&dir).unwrap();
    let f = dir.join("p.json");
    lensbench_core::vfs::write(&f, r#"{"blocks": [], "gui": {"combo": {"name": "x"}}}"#).unwrap();
    let v = doc_peek(&serde_json::json!({ "file": f.display().to_string() })).unwrap();
    assert_eq!(
        v,
        serde_json::json!({ "gui": { "combo": { "name": "x" } }, "blocks": true })
    );
    // 組合せを同梱していないブロックのプロジェクト: 組合せで書ければ combo
    let ex = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/projects");
    let peek = |n: &str| {
        let f = ex.join(n).display().to_string();
        doc_peek(&serde_json::json!({ "file": f })).unwrap()
    };
    assert_eq!(
        peek("P8_catalog_two.json")["combo"]["picks"][0]["part"],
        "PCX-100"
    );
    // 組合せで書けない（入れ子の群）なら combo は無い
    lensbench_core::vfs::write(&f, r#"{"wavelengths": [{"um": 0.55}], "aperture": {"type": "epd", "value": 2},
        "blocks": [{"type": "group", "id": "A", "blocks": [{"type": "group", "id": "B", "blocks": [
            {"type": "lens", "stop": true, "source": {"surfaces": [{"radius": 20, "t": 3, "glass": "N-BK7"}, {"radius": -20}]}}]}]},
            {"type": "air", "t": 9}]}"#)
    .unwrap();
    let v = doc_peek(&serde_json::json!({ "file": f.display().to_string() })).unwrap();
    assert!(v.get("combo").is_none());
    lensbench_core::vfs::write(&f, r#"{"surfaces": []}"#).unwrap();
    let v = doc_peek(&serde_json::json!({ "file": f.display().to_string() })).unwrap();
    assert_eq!(v, serde_json::json!({ "gui": null, "blocks": false }));
    assert!(
        doc_peek(&serde_json::json!({ "file": dir.join("none.json").display().to_string() }))
            .is_err()
    );
    lensbench_core::vfs::remove_file(&f).ok();
}

/// `folders` {glass_dirs, lens_dirs}: {agf: ガラスのフォルダの AGF の名前, missing: 見つからないフォルダ}
fn folders(p: &Value) -> Value {
    let list = |k: &str| -> Vec<String> {
        p[k].as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let (glass, lens) = (list("glass_dirs"), list("lens_dirs"));
    let missing: Vec<&String> = glass
        .iter()
        .chain(&lens)
        .filter(|d| !lensbench_core::vfs::is_dir(d))
        .collect();
    serde_json::json!({ "agf": agf_names(&glass), "missing": missing })
}

/// フォルダの並びにある AGF の名前（拡張子なし、重なりは前のもの、名前の順）
pub(crate) fn agf_names(dirs: &[String]) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for d in dirs {
        let Ok(rd) = lensbench_core::vfs::read_dir(d) else {
            continue;
        };
        for p in rd {
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("agf")) {
                if let Some(n) = p.file_stem().and_then(|x| x.to_str()) {
                    if !v.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                        v.push(n.to_string());
                    }
                }
            }
        }
    }
    v.sort_by_key(|x| x.to_ascii_uppercase());
    v
}

#[test]
fn folders_of_test_glass() {
    let d = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/glass").to_string();
    let v = folders(&serde_json::json!({
        "glass_dirs": [d.clone(), d],
        "lens_dirs": ["/no/such/dir"],
    }));
    let agf: Vec<&str> = v["agf"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(agf.contains(&"SCHOTT"), "{v}");
    let mut w = agf.clone();
    w.dedup();
    assert_eq!(agf, w);
    assert_eq!(v["missing"], serde_json::json!(["/no/such/dir"]));
}

pub struct Worker {
    tx: Sender<Job>,
    /// 長い要求のスレッド（ネイティブの本体の Worker だけ。無ければ tx）
    tx_long: Option<Sender<Job>>,
    rx: Receiver<Done>,
    /// 出して、まだ結果を受け取っていない要求の数
    pending: usize,
    /// そのうち解析でないもの（系の読み込み・レイアウトなど。長い要求は除く）
    pending_core: usize,
    /// そのうち長い要求
    pending_long: usize,
    /// 系の読み込みなどを待っている間に出された長い要求（別のスレッドで回るので、先に出した系の変更が
    /// 終わるまで渡さない。渡すと前の系で計算してしまう）
    held: Vec<Job>,
    gen: Arc<AtomicU64>,
    /// 回している解析の中断の印（[`CancelToken`]。解析でない要求・下書きの Worker では None）。
    /// 印は要求ごとに作るので、最適化などの中の解析や別の Worker の計算は止めない
    running: Arc<Mutex<Option<CancelToken>>>,
    /// Web 版: 計算の Web Worker に中断を頼むか。下書きの Worker は使わない
    #[cfg(target_arch = "wasm32")]
    cancels: bool,
    #[cfg(target_arch = "wasm32")]
    remote: remote::Remote,
}

/// 1 つの要求を回す（計算のスレッドで）
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct Runner {
    call: CallFn,
    threads: Option<usize>,
    cancels: bool,
    /// 今の世代
    cur: Arc<AtomicU64>,
    run_flag: Arc<Mutex<Option<CancelToken>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Runner {
    fn run(&self, j: Job) -> Done {
        if self.cancels {
            // この要求の中断の印を置いてから、世代を確かめる（この後に系が変われば印が立つ）
            *self.run_flag.lock_ok() = j.tag.skippable().then(CancelToken::new);
        }
        let token = self.run_flag.lock_ok().clone();
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
            let result = lensbench_core::par::with_cancel(token.as_ref(), || {
                (self.call)(&method, &params, self.threads)
            });
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
            *self.run_flag.lock_ok() = None;
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
    pub fn new(threads: Option<usize>, repaint: impl Fn() + Send + Sync + 'static) -> Worker {
        Self::with_call(threads, repaint, true, Kind::Main)
    }

    /// レイアウト図の下書きとカタログの一覧用（`preview`・`glasses`・`lenses`・`catalog_items`・`catalog_edit`・`folders`・`cad_measure`・`doc_peek` だけを受ける。serve の系・順番待ちとは
    /// 別に回すので、解析を回している間も図とカタログが出る）
    pub fn preview(repaint: impl Fn() + Send + Sync + 'static) -> Worker {
        Self::with_call(None, repaint, false, Kind::Preview)
    }

    /// 計算しない Worker（Web 版の別のタブのパネル。要求を出せば下書きとして回す）
    #[cfg(target_arch = "wasm32")]
    pub fn idle() -> Worker {
        Self::with_call(None, || {}, false, Kind::Idle)
    }

    fn with_call(
        threads: Option<usize>,
        repaint: impl Fn() + Send + Sync + 'static,
        cancels: bool,
        kind: Kind,
    ) -> Worker {
        let (tx, jobs) = channel::<Job>();
        let (done, rx) = channel::<Done>();
        let gen = Arc::new(AtomicU64::new(0));
        let running = Arc::new(Mutex::new(None));
        #[cfg(not(target_arch = "wasm32"))]
        let runner = Runner {
            call: kind.call(),
            threads,
            cancels,
            cur: gen.clone(),
            run_flag: running.clone(),
        };
        // ネイティブは別のスレッドで順に回す（本体の Worker は長い要求のスレッドも）
        #[cfg(not(target_arch = "wasm32"))]
        let tx_long = {
            let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(repaint);
            let spawn = |name: &str, jobs: Receiver<Job>, runner: Runner| {
                let (done, repaint) = (done.clone(), repaint.clone());
                std::thread::Builder::new()
                    .name(name.into())
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
                    .expect("cannot start the computation thread");
            };
            let long = cancels.then(|| {
                let (tx, jobs) = channel::<Job>();
                // 長い要求は解析の中断の印を使わない（止めるのは stop）
                let r = Runner {
                    cancels: false,
                    ..runner.clone()
                };
                spawn("lensbench-worker-long", jobs, r);
                tx
            });
            spawn("lensbench-worker", jobs, runner);
            long
        };
        #[cfg(target_arch = "wasm32")]
        let tx_long = None;
        Worker {
            tx,
            tx_long,
            rx,
            pending: 0,
            pending_core: 0,
            pending_long: 0,
            held: Vec::new(),
            gen,
            running,
            #[cfg(target_arch = "wasm32")]
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
        let long = tag.long();
        let core = !tag.skippable() && !long;
        // 系を変えるなら、それより前に出した解析は要らない（待っている分は捨て、回っている分はやめる）
        if tag.changes_system() {
            self.drop_queued();
        }
        let gen = self.gen.load(Ordering::Relaxed);
        let job = Job { tag, calls, gen };
        if long && self.tx_long.is_some() && (self.pending_core > 0 || !self.held.is_empty()) {
            self.held.push(job);
            self.pending += 1;
            self.pending_long += 1;
            return;
        }
        let tx = self.tx_long.as_ref().filter(|_| long).unwrap_or(&self.tx);
        if tx.send(job).is_ok() {
            self.pending += 1;
            self.pending_core += core as usize;
            self.pending_long += long as usize;
        }
    }

    /// 待たせた長い要求を、系の変更が終わったら渡す（渡せなかった分は数えない）
    fn release_held(&mut self) {
        if self.pending_core > 0 || self.held.is_empty() {
            return;
        }
        let Some(tx) = self.tx_long.as_ref() else {
            return;
        };
        for job in self.held.drain(..) {
            if tx.send(job).is_err() {
                self.pending -= 1;
                self.pending_long -= 1;
            }
        }
    }

    /// 終わった要求（無ければ None）
    pub fn poll(&mut self) -> Option<Done> {
        self.release_held();
        // wasm: Web Worker の結果を受け取り、空いていれば次の要求を渡す
        #[cfg(target_arch = "wasm32")]
        self.remote.pump(self.gen.load(Ordering::Relaxed));
        let mut d = self.rx.try_recv().ok()?;
        self.pending -= 1;
        if d.tag.long() {
            self.pending_long -= 1;
        } else if !d.tag.skippable() {
            self.pending_core -= 1;
        } else if d.gen != self.gen.load(Ordering::Relaxed) {
            // 回している間に系が変わった（古い系の結果、または中断した）
            d.skipped = true;
            d.replies.clear();
        }
        Some(d)
    }

    /// 何か待っている（長い要求も）
    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    /// 長い要求（最適化など）のほかに待っている
    pub fn busy_short(&self) -> bool {
        self.pending > self.pending_long
    }

    /// 解析でない要求（系の読み込み・レイアウトなど）を待っている
    pub fn busy_core(&self) -> bool {
        self.pending_core > 0
    }

    /// 待っている解析を捨て、回している解析には途中でやめるよう頼む（結果は [`Worker::poll`] で捨てる）。
    /// 系を変える要求（[`Tag::changes_system`]）を出すときは自動で呼ぶ
    pub fn drop_queued(&self) {
        let r = self.running.lock_ok();
        self.gen.fetch_add(1, Ordering::Relaxed);
        if let Some(t) = r.as_ref() {
            t.cancel();
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

/// Worker に渡す番号の上の位: 誤りの文を日本語で（[`lensbench_core::i18n`]）
#[cfg(target_arch = "wasm32")]
const LANG_JA: u8 = 0x80;

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

    /// 何を受けるか（[`Kind::code`]、上の位 [`LANG_JA`] は誤りの文の言語）
    pub fn init(code: u8) {
        crate::crash::install();
        lensbench_core::i18n::set_lang(if code & LANG_JA != 0 {
            lensbench_core::i18n::Lang::Ja
        } else {
            lensbench_core::i18n::Lang::En
        });
        KIND.with(|k| k.set(code & !LANG_JA));
    }

    /// 回している要求の番号（0 は無し）。計算のスレッドと見張りのスレッド（[`cancel`]・[`stop`]、
    /// スレッドありの版だけ。メモリを共有する）で、この鍵を取って「回している要求だけを止める」を確かにする
    /// 番号とその要求の中断の印
    static CUR: std::sync::Mutex<(u32, Option<CancelToken>)> = std::sync::Mutex::new((0, None));

    fn cur() -> std::sync::MutexGuard<'static, (u32, Option<CancelToken>)> {
        CUR.lock_ok()
    }

    /// 見張りのスレッドから: 要求 id を回していれば、解析に途中でやめるよう頼む
    pub fn cancel(id: u32) {
        let c = cur();
        if c.0 == id && id != 0 {
            if let Some(t) = &c.1 {
                t.cancel();
            }
        }
    }

    /// 見張りのスレッドから: 要求 id を回していれば、最適化（what = "optimize"）・公差（"tolerance"）・
    /// 差し替え探索（"swap"）を止める（そこまでの結果を返す）
    pub fn stop(id: u32, what: &str) {
        let c = cur();
        if c.0 != id || id == 0 {
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
                    id: 0,
                    results: vec![Err(e)],
                    ms: 0.0,
                    files: Vec::new(),
                }
                .encode()
            }
        };
        let main = KIND.with(Cell::get) == Kind::Main.code();
        // この要求の中断の印（見張りは番号が合うときだけ印を立てる）
        let token = main.then(CancelToken::new);
        if main {
            *cur() = (q.id, token.clone());
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
            let r = lensbench_core::par::with_cancel(token.as_ref(), || call(m, p, q.threads));
            let failed = r.is_err();
            results.push(r);
            if failed {
                break;
            }
        }
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        if main {
            *cur() = (0, None);
        }
        let files = sync::changes_since(seq)
            .into_iter()
            .map(|(p, b, _)| (p, b))
            .collect();
        Response {
            id: q.id,
            results,
            ms,
            files,
        }
        .encode()
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
                // 元のタブが裏に回っていても、結果を受けて別の窓のパネルを描き直す（#237）
                repaint: Rc::new(move || {
                    repaint();
                    crate::web_bg::kick();
                }),
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
                    self.inbox.borrow_mut().push_back(Err(crate::i18n::trf(
                        "Cannot start the Web Worker: {}",
                        &[&format!("{e:?}")],
                    )));
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
                    .unwrap_or_else(|| crate::i18n::tr("Web Worker error").into());
                inbox.borrow_mut().push_back(Err(m));
                repaint();
            });
            w.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            w.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            // 誤りの文の言語も渡す（Worker は GUI の言語の設定を知らない、XCT-20）
            let lang = if lensbench_core::i18n::is_ja() {
                LANG_JA
            } else {
                0
            };
            let _ = w.post_message(&JsValue::from(self.kind.code() | lang));
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
                self.inbox.borrow_mut().push_back(Err(crate::i18n::trf(
                    "Cannot send to the Web Worker: {}",
                    &[&format!("{e:?}")],
                )));
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
            let resp = msg.and_then(|b| Response::decode(&b));
            // 待っている要求の返事でないもの（エラーで先に片づけた要求の遅れた返事など）は捨てる（GAR-27）
            if let Ok(r) = &resp {
                if r.id != 0 && (self.inflight.is_none() || r.id != self.inflight_id) {
                    return;
                }
            }
            if matches!(self.kind, Kind::Main) {
                RUNNING.with(|r| r.set(0));
                PROGRESS.with(|p| *p.borrow_mut() = Value::Null);
            }
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
                        vec![Err(crate::i18n::trf(
                            "Computation stopped (reopen the system): {}",
                            &[&e],
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
    static LOADED: AtomicBool = AtomicBool::new(false);

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
        // 時間のかかる系の読み込みと、それが終わっていたかを返す要求
        if m == "slowload" {
            std::thread::sleep(Duration::from_millis(100));
            LOADED.store(true, Ordering::SeqCst);
        }
        if m == "loaded" {
            return Ok((Value::from(LOADED.load(Ordering::SeqCst)), Vec::new()));
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

    // 順番待ちと世代は Worker で 1 つなので、1 つのテストの中で順に確かめる
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
        // 解析でない長い要求（最適化など）は系の変更で止めない（中の解析も止めない）。
        // 別のスレッドで回るので、その間の系の変更・解析は先に終わる（GAR-20）
        w.send(Tag::Optimize, "spin", Value::Null);
        wait_started();
        assert!(w.busy() && !w.busy_short() && !w.busy_core());
        w.send(Tag::Set, "set", Value::Null);
        w.send(Tag::Analyze(4), "info", Value::Null);
        let d = wait_all(&mut w, 3);
        let tags: Vec<Tag> = d.iter().map(|d| d.tag.clone()).collect();
        assert_eq!(tags, vec![Tag::Set, Tag::Analyze(4), Tag::Optimize]);
        assert!(!d[2].skipped);
        assert_eq!(d[2].first().2, Ok(&Value::from("timeout")));
        assert!(!w.busy());
        // 系の読み込みの直後に出した長い要求は、読み込みが終わってから回る（前の系で計算しない）
        w.send(Tag::Load, "slowload", Value::Null);
        w.send(Tag::Zoom, "loaded", Value::Null);
        assert!(w.busy_core() && w.busy());
        let d = wait_all(&mut w, 2);
        let tags: Vec<Tag> = d.iter().map(|d| d.tag.clone()).collect();
        assert_eq!(tags, vec![Tag::Load, Tag::Zoom]);
        assert_eq!(d[1].first().2, Ok(&Value::from(true)));
        assert!(!w.busy());
    }
}
