//! 窓の骨格: 上の帯（ファイル・探す・計算）、パネル（ドッキング・タブ・フロート・別の OS の窓）、
//! 下の状態の行（焦点距離・F 値など）。
//!
//! 計算はすべて serve と同じ method（load・analyze・get・set・write・trace・glasses・lenses）を [`Worker`] に頼む。
//! 系のブロック編集とレイアウト図は、系の編集・保存形式の上にパネルとして足す。

mod ask;
mod doc_file;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use doc_file::convert_old;
mod doc_loc;
mod folders;
mod help;
mod history;
mod ld_combo;
use history::{ComboHist, DocHist};
mod menus;
mod open_doc;
mod panels;
mod result;
mod status;
mod sys_copy;
mod tick;
use doc_loc::DocLoc;
use panels::Panels;
use result::AnaRes;
use sys_copy::SysCopy;
mod take;
mod to_combo;
mod tour_steps;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use egui::{Color32, Key, Modifiers, RichText, Stroke, Ui};
use egui_dock::{DockArea, NodeIndex, SurfaceIndex, TabViewer};
use lensbench_engine::session::views;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[path = "app_combo_ops.rs"]
mod combo_ops;

use crate::argform;
use crate::catalog::{self, Browser, Combo};
use crate::catsave::CatSave;
use crate::combo;
use crate::config_panel;
use crate::figures;
use crate::i18n::{self, n_, tr, trf, Lang};
use crate::json_view::{self, fmt_num};
use crate::layout::{self, AnaCfg, Kind, Layout, OsWin, Pane, Preset, View};
use crate::lens_data;
use crate::missing_glass::{missing_glass_name, model_glass_zmx};
use crate::open_sysset;
use crate::palette::{Item, Palette};
use crate::plots::{self, SpotRays, SysInfo};
use crate::split;
use crate::theme::{self, Look, Mode};
use crate::tour::{Args, Step, Tour};
use crate::worker::{Done, Tag, Worker};

/// 覚えておく設定（次に開いたときも使う）
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub glass_dirs: Vec<String>,
    pub lens_dirs: Vec<String>,
    pub recent: Vec<String>,
    pub combo: Combo,
    pub look: Look,
    pub lang: Lang,
    /// 今の並び
    pub layout: Option<Layout>,
    /// 名前を付けて保存した並び
    pub layouts: Vec<(String, Layout)>,
    /// 系のパネルで見るパラメータ（3.t、G1.shift など）
    pub params: Vec<String>,
    /// 組合せを変えたらすぐ系を作り直す
    pub auto_build: bool,
    /// レイアウト図の見る面
    pub view_plane: ViewPlane,
    /// レンズデータの表をブロックごとに折りたたむ表示（false はブロックに入って中を見る表示）
    pub ld_tree: bool,
    /// レンズデータの表の下に詳細設定（開口・傾き偏心・形のデータ・ブロックの設定）を出す
    pub ld_detail: bool,
    /// レンズデータの表を面単位で選ぶ（ブロックを自動で選ばない）
    pub ld_by_surface: bool,
    /// レイアウト図・書き出しの光線と書き出しの設定
    pub rays: crate::layout_rays::RaySet,
    /// カタログレンズの一覧で隠す列（[`catalog::LensCol::key`]）
    pub lens_hide: Vec<String>,
    /// アプリ版で OS のタイトルバーを使う（false は上の帯をタイトルバーにする、#138）
    pub os_titlebar: bool,
    /// 品の STEP の共有の置き場（アプリ版。空なら環境変数 LENSBENCH_CAD_STORE、なければ配布元だけ）
    pub cad_store: crate::cad_store::CadStore,
    /// 最適化・公差・差し替え・ズームのパネルの設定（serve と同じ形。次に開いたときも残す、GPN-14）
    pub panels: Value,
    /// 開いた系（組合せでない系）のファイルごとのロックしたブロック（開き直しても残す、GPN-14）
    pub locked: std::collections::BTreeMap<String, BTreeSet<String>>,
}

impl Settings {
    /// 設定の中のパス（硝材・レンズのフォルダ、最近のファイル、組合せの .zmx）をそれぞれ f で変える
    pub fn map_paths(&mut self, f: impl Fn(&str) -> String) {
        for p in self
            .glass_dirs
            .iter_mut()
            .chain(self.lens_dirs.iter_mut())
            .chain(self.recent.iter_mut())
        {
            *p = f(p);
        }
        self.combo.map_paths(&f);
    }

    fn paths(&self) -> Settings {
        Settings {
            glass_dirs: self.glass_dirs.clone(),
            lens_dirs: self.lens_dirs.clone(),
            recent: self.recent.clone(),
            combo: self.combo.clone(),
            ..Default::default()
        }
    }

    fn set_paths(&mut self, k: Settings) {
        self.glass_dirs = k.glass_dirs;
        self.lens_dirs = k.lens_dirs;
        self.recent = k.recent;
        self.combo = k.combo;
    }

    /// 読んだ設定の相対パスを、設定のフォルダから戻す（ポータブル・--settings のとき）
    fn resolve_paths(&mut self) {
        if let Some(b) = crate::portable::base() {
            self.map_paths(|p| crate::portable::from_stored(p, b));
        }
    }
}

/// レイアウト図の見る面（serve の layout の plane）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewPlane {
    #[default]
    Yz,
    Xz,
    /// 光路に沿って展開した YZ 面の図
    Unfold,
    /// 3D 表示
    #[serde(rename = "3d")]
    D3,
}

impl ViewPlane {
    pub const ALL: [ViewPlane; 4] = [
        ViewPlane::Yz,
        ViewPlane::Xz,
        ViewPlane::Unfold,
        ViewPlane::D3,
    ];

    /// serve の layout の plane
    pub fn key(self) -> &'static str {
        match self {
            ViewPlane::Yz | ViewPlane::Unfold => "yz",
            ViewPlane::Xz => "xz",
            ViewPlane::D3 => "3d",
        }
    }
    /// 図の縦の軸の名前
    pub fn vert(self) -> &'static str {
        match self {
            ViewPlane::Xz => "x",
            _ => "y",
        }
    }
    /// 見る面の選択の表示
    pub fn label(self) -> &'static str {
        match self {
            ViewPlane::Yz => "YZ",
            ViewPlane::Xz => "XZ",
            ViewPlane::Unfold => tr("Unfolded"),
            ViewPlane::D3 => "3D",
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            glass_dirs: Vec::new(),
            lens_dirs: Vec::new(),
            recent: Vec::new(),
            combo: Combo::default(),
            look: Look::default(),
            lang: Lang::Auto,
            layout: None,
            layouts: Vec::new(),
            params: Vec::new(),
            auto_build: true,
            view_plane: ViewPlane::Yz,
            // 既定は折りたたみ表示で全部開く（曲率半径・硝材がすぐ見える、GUX-05）
            ld_tree: true,
            ld_detail: false,
            ld_by_surface: false,
            rays: Default::default(),
            lens_hide: Vec::new(),
            os_titlebar: false,
            cad_store: Default::default(),
            panels: Value::Null,
            locked: Default::default(),
        }
    }
}

#[derive(Default)]
struct Status {
    text: String,
    error: bool,
}

impl Core {
    /// 下の行に知らせを出す（利用者の操作の結果、GAR-16）
    fn info(&mut self, text: impl Into<String>) {
        self.status = Status {
            text: text.into(),
            error: false,
        };
    }

    /// 下の行に誤りを出す（GAR-16）
    fn fail(&mut self, text: impl Into<String>) {
        self.status = Status {
            text: lensbench_core::i18n::tr(&text.into()), // 計算側の英語の文を訳す（XCT-20）
            error: true,
        };
    }
}

/// 結果の JSON の表示で見せる長さ（字数。これより長い分は切る）
const JSON_VIEW_MAX: usize = 200_000;

/// serve の要求の名前を画面の言葉に（状態の行、GUX-06）
fn method_label(m: &str) -> String {
    let ja = match m {
        "load" => "Load",
        "reload" => "Reload",
        "edit" => "Edit",
        "set" => "Value change",
        "analyze" => "Analysis",
        "save" => "Save",
        "write" => "Export",
        "layout" | "layout_export" => "Layout",
        "optimize" => "Optimize",
        "tolerance" => "Tolerance",
        "swap" => "Swap",
        "zoom" => "Zoom",
        "surface" => "Lens Data",
        "history" => "Revert",
        _ => return m.to_string(),
    };
    tr(ja).to_string()
}

/// serve の誤りの文から、GUI の利用者に関係の無い内部の言葉を外す（「操作 1: 」、serve の操作名の案内）
fn plain_error(e: &str) -> String {
    let mut t = e.to_string();
    // 編集の操作の番号（1 つだけ送ったときは要らない）
    for p in ["操作 1: ", "operation 1: "] {
        if let Some(rest) = t.strip_prefix(p) {
            t = rest.to_string();
        }
    }
    t = t.replace("不正: ", "").replace("invalid: ", "");
    let add = tr("(add glass catalogs in File > Catalog folders…)");
    t.replace("（カタログを追加: add_glasses・add_glass_catalogs）", add)
        .replace(
            " (add catalogs: add_glasses, add_glass_catalogs)",
            &format!(" {add}"),
        )
}

/// カタログに無いガラスで開けなかった系（GUX-01）
#[derive(Default, Clone)]
struct MissingGlass {
    /// 開こうとしたファイル
    file: PathBuf,
    /// 見つからなかったガラスの名前
    names: Vec<String>,
    /// モデルガラスで代用した写し（読み直しで次の無いガラスも代用する）
    model: Option<PathBuf>,
    /// 代用を続けた回数（止まらないときの上限）
    tries: usize,
}

/// パレット・メニューから実行する操作
#[derive(Clone)]
enum Cmd {
    New,
    /// 元に戻す（true）・やり直す、段の数
    Undo(bool, usize),
    Open,
    Recent(String),
    Reload,
    RunAll,
    WriteZmx,
    /// 保存（true は名前を付けて保存）
    Save(bool),
    Add(Kind),
    Focus(u64),
    /// その種類のパネルを表に出す（無ければ足す）
    FocusKind(Kind),
    ToWindow(u64),
    Preset(Preset),
    Layout(usize),
    SaveLayout,
    Theme(Mode),
    Accent(usize),
    Lang(Lang),
    Folders,
    Help,
    /// 最適化を今の設定で回す・公差を回す・像面を近軸の焦点へ（パレットの実行系、GUX-16）
    RunOpt,
    RunTol,
    FocusImage,
}

/// パネルの外の状態（パネルの中から触る）
struct Core {
    /// パネルの状態（GAR-01）
    p: Panels,
    s: Settings,
    /// ファイルメニューの「見本」（Web 版の同梱の系）
    samples: Vec<String>,
    /// カタログのフォルダの様子（AGF の名前・見つからないフォルダ）
    folders: folders::Folders,
    worker: Worker,
    /// 組合せのレイアウト図だけを先に作る（解析の順番を待たない）
    preview: Worker,
    /// 系の読み込みを待っている間に組合せが変わった（読み込みが空いたら最新を読む）
    load_stale: bool,
    /// カタログの玉の断面を頼んだ行
    preview_req: Option<usize>,
    status: Status,
    /// 下の行の誤りを出した要求とその文（同じ要求がうまくいったら消す、GUX-27）
    status_err: Option<(Tag, String)>,
    /// 文書の所在（開いた系のファイル・保存先・最後に作った組合せ、GAR-01）
    loc: DocLoc,
    _work: doc_loc::WorkDir,
    /// serve の系の写し（概要・面の表・元に戻す段など、GAR-01）
    sys: SysCopy,
    results: HashMap<u64, AnaRes>,
    new_param: String,
    /// まだ送っていない値の変更（計算中に動かした分はまとめて送る）
    pending_set: std::collections::BTreeMap<String, f64>,
    /// 開いている解析をすべて計算し直す（true は live の解析だけ）
    rerun: Option<bool>,
    browser: Browser,
    /// 組合せ（GUI が持つ文書）と、カタログから足す位置（GPN-28）
    doc: crate::combo_doc::ComboDoc,
    folders_open: bool,
    help_open: bool,
    save_layout_open: bool,
    /// 答えを待っている「はい・いいえ」の確かめ（[`Core::ask`]、GAR-24）
    ask: Option<ask::Ask>,
    /// Web 版: 今描いている別の窓のパネル（確かめをその窓に出す）
    drawing_in: Option<u64>,
    layout_name: String,
    /// 組合せのレイアウト図（serve の layout）
    layout_data: Option<combo::LayoutData>,
    /// レイアウト図に重ねる構成
    overlay: Overlay,
    /// レイアウトの光線・書き出しの設定の欄を次の描画で開く（通し確認）
    rays_open: bool,
    /// 前の画面で図で選んでいた要素（図で選び直したかを見る）
    combo_sel_prev: Option<usize>,
    /// 開いた系を組合せ（系の取り込み 1 つ）にする（系の設定を待っている）。true は今の組合せの要素を後ろに残す（#135）
    to_combo: Option<bool>,
    /// 開いた系を組合せにするとき差し込む要素と、差し込む間隔（その前の面の番号。None は系の後ろ）
    open_insert: Option<(Option<usize>, combo::Pick)>,
    /// 前の画面の組合せの要素の数（空の組合せに足したかを見る。usize::MAX はまだ見ていない）
    combo_len_prev: usize,
    font: Option<String>,
    next_id: u64,
    /// パネルの中から頼まれた操作
    cmds: Vec<Cmd>,
    /// 最後に組合せを変えて系を作り直した時刻（動かしている間は解析を待たせる）
    combo_edit: Option<web_time::Instant>,
    /// 組合せの元に戻す・やり直す
    combo_hist: ComboHist,
    /// 文書を替えた段（新規・開く）の元に戻す・やり直す
    docs: DocHist,
    /// 玉・群・機構部品の欄を開く（通し確認）
    um_open: bool,
    /// レンズデータの硝材の候補（ガラスカタログを読んだときに作る）
    ld_glasses: Vec<lens_data::GlassChoice>,
    /// 閉じる前に「保存しますか」を聞いている
    close_ask: bool,
    /// 前の画面の組合せの要素・状態・面の上書きの数
    combo_census: Option<[usize; 3]>,
    /// 図を画像にする（パネル、写すか、解析の名前）。画面の写しが届いたら切り出す
    shot: Option<(u64, bool, String)>,
    /// 解析パネルの図の矩形
    fig_rects: HashMap<u64, egui::Rect>,
    /// 最後に値の変更を送った時刻（解析を中断して送る間隔）
    last_set: Option<web_time::Instant>,
    /// 状態の行の履歴（文、誤りか）と、その窓を開いているか
    status_log: Vec<(String, bool)>,
    status_log_open: bool,
    /// 開けなかった系の、カタログに無いガラス（案内の窓、GUX-01）
    missing_glass: Option<MissingGlass>,
    /// 未保存の確認の後に閉じる（もう聞かない）
    close_ok: bool,
    /// 保存が終わったら閉じる
    close_pending: bool,
    /// 未完のカタログ品を、行を選んだら裏で補う
    auto_fill: crate::fill::AutoFill,
    swap_ids: Vec<String>,
    /// 開いたプロジェクトの玉・間隔（ファイルと、その中の候補）
    swap_file: Option<(PathBuf, crate::swap_panel::Cands)>,
    /// 結果・系の概要を受け取った回数（Web 版の別のタブに送り直す目安）
    rev: u64,
    /// [`Core::param_sys`] の写し（rev が変わったときだけ作り直す、GAR-09）
    param_sys_cache: std::cell::RefCell<Option<(u64, std::rc::Rc<crate::param_pick::Sys>)>>,
    /// 系を変える要求（読込・値の変更・編集・元に戻す）の返事を受け取った回数（最適化の結果を当ててよいか）
    sys_gen: u64,
}

pub struct App {
    c: Core,
    dock: egui_dock::DockState<Pane>,
    windows: Vec<OsWin>,
    palette: Palette,
    /// 別の窓を開いたときの大きさ・位置（毎フレーム同じ値を渡す。今の値を渡し直すと、DPI の違うモニターへ
    /// 移したときに大きさの変換が往復して窓が暴れる）
    win_init: HashMap<u64, ([f32; 2], Option<[f32; 2]>)>,
    /// 引きずっているタブと、最後のポインターの位置（本体の窓の座標）
    dragging_tab: Option<(u64, egui::Pos2)>,
    /// 画面の通し確認（--tour）
    tour: Option<Tour>,
    /// 窓に送った OS のタイトルバーの有無（設定と違えば送り直す）
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    decorations: Option<bool>,
    /// Web 版: 別の窓に出したパネル（元のタブ）
    #[cfg(target_arch = "wasm32")]
    tabs: Option<crate::web_tabs::Tabs>,
    /// Web 版: このタブが別の窓に出した解析以外のパネルなら、元のタブで描いた画面を受けて描く口（#137）
    #[cfg(target_arch = "wasm32")]
    viewer: Option<crate::web_tabs::Viewer>,
    /// Web 版（元のタブ）: 別の窓に出した解析以外のパネルを描く画面（パネル id ごと）
    #[cfg(target_arch = "wasm32")]
    hosts: HashMap<u64, crate::web_tabs::Host>,
}

/// 見本の名前（例題のプロジェクトはファイル名、tests/refs の系はフォルダの名前）
fn sample_name(f: &str) -> String {
    let p = Path::new(f);
    let stem = p
        .file_stem()
        .map(|x| x.to_string_lossy().into_owned())
        .unwrap_or_default();
    match p.parent().and_then(|d| d.file_name()) {
        Some(d) if stem == "system" => d.to_string_lossy().into_owned(),
        _ => stem,
    }
}

/// 窓のアイコン（assets/icon.png、元は icon.svg: 両凸レンズで平行光が 1 点に集まる）
pub fn icon() -> std::sync::Arc<egui::IconData> {
    // 別の窓は毎フレーム作るので、PNG の復号は 1 回だけ
    static ICON: std::sync::OnceLock<std::sync::Arc<egui::IconData>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        // ブラウザ（wasm）ではタブのアイコンは index.html が決める
        #[cfg(target_arch = "wasm32")]
        let d = egui::IconData::default();
        #[cfg(not(target_arch = "wasm32"))]
        let d = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
            .unwrap_or_default();
        std::sync::Arc::new(d)
    })
    .clone()
}

/// 上の帯のロゴ（アイコンと同じ図柄を線で描く）
fn logo(ui: &mut Ui, size: f32, accent: Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let p = ui.painter();
    // アイコンの 256 の座標を size に
    let at = |x: f32, y: f32| r.min + egui::vec2(x, y) * (size / 256.0);
    let w = (size / 256.0 * 14.0).max(1.4);
    let ray = Stroke::new(w, Color32::from_rgb(0xFF, 0xB5, 0x47));
    p.line_segment(
        [at(20.0, 128.0), at(226.0, 128.0)],
        Stroke::new(w, Color32::from_rgb(0xFF, 0xE0, 0x8A)),
    );
    for y in [70.0, 186.0] {
        p.line(vec![at(20.0, y), at(104.0, y), at(226.0, 128.0)], ray);
    }
    // レンズ: 2 本の円弧の近似
    let lens: Vec<egui::Pos2> = (0..=24)
        .map(|i| {
            let t = i as f32 / 24.0;
            let y = 36.0 + 184.0 * t;
            let b = 1.0 - (2.0 * t - 1.0).powi(2);
            at(104.0 + 40.0 * b, y)
        })
        .chain((0..=24).map(|i| {
            let t = 1.0 - i as f32 / 24.0;
            let y = 36.0 + 184.0 * t;
            let b = 1.0 - (2.0 * t - 1.0).powi(2);
            at(104.0 - 40.0 * b, y)
        }))
        .collect();
    p.add(egui::Shape::convex_polygon(
        lens,
        accent.gamma_multiply(0.85),
        Stroke::new(w * 0.5, accent),
    ));
    p.circle_filled(at(226.0, 128.0), size / 256.0 * 20.0, Color32::WHITE);
}

fn dirs(v: &[String]) -> Value {
    json!(v)
}

/// serve に渡すガラスのフォルダ（アプリ版は同梱のカタログを後ろに足す、GUX-01）
fn glass_dirs(v: &[String]) -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    return crate::builtin_glass::dirs(v);
    #[cfg(target_arch = "wasm32")]
    dirs(v)
}

impl Core {
    /// 最適化・公差・差し替えのパネルの設定（変数・項・公差の表。serve の optimize・tolerance・swap と同じ形）。
    /// 空のパネルは入れない（GPN-14）
    fn panels_value(&self) -> Value {
        let mut v = serde_json::Map::new();
        if self.p.opt.vars.iter().any(|x| !x.name.trim().is_empty()) || !self.p.opt.terms.is_empty()
        {
            if let Ok(p) = self.p.opt.params() {
                v.insert("optimize".into(), p);
            }
        }
        if let Ok(p) = self.p.tol.spec() {
            if p != crate::tol_panel::TolPanel::default()
                .spec()
                .unwrap_or(Value::Null)
            {
                v.insert("tolerance".into(), p);
            }
        }
        if let Ok(p) = self.p.swap.spec() {
            if p != crate::swap_panel::SwapPanel::default()
                .spec()
                .unwrap_or(Value::Null)
            {
                v.insert("swap".into(), p);
            }
        }
        if let Some(p) = self.p.zoom.saved() {
            v.insert("zoom".into(), p);
        }
        if v.is_empty() {
            Value::Null
        } else {
            Value::Object(v)
        }
    }

    /// [`Core::panels_value`] をパネルに戻す（読めない項目は飛ばす）
    fn set_panels(&mut self, v: &Value) {
        if let Some(p) = v.get("optimize") {
            let _ = self.p.opt.set_params(p);
        }
        if let Some(p) = v.get("tolerance") {
            let _ = self.p.tol.set_spec(p);
        }
        if let Some(p) = v.get("swap") {
            let _ = self.p.swap.set_spec(p);
        }
        if let Some(p) = v.get("zoom") {
            self.p.zoom.restore(p);
        }
    }

    /// 保存していない変更がある（組合せは最後に保存・開いたものと違う、開いた系は面の表・値を変えた）
    fn dirty(&self) -> bool {
        let combo =
            self.is_combo() || (self.loc.file.is_none() && !self.doc.combo.picks.is_empty());
        if combo {
            match &self.loc.saved_combo {
                Some(c) => c != &self.doc.combo,
                None => !self.doc.combo.picks.is_empty(),
            }
        } else {
            self.loc.ld_dirty
        }
    }

    /// 閉じると失う変更がある。一度もファイルに保存・ファイルから開いていない組合せは設定に残り、次に起動したときに
    /// 戻るので失わない（毎回の閉じる確認を出さない）
    fn close_loses(&self) -> bool {
        let combo =
            self.is_combo() || (self.loc.file.is_none() && !self.doc.combo.picks.is_empty());
        if combo && self.loc.saved_combo.is_none() {
            return false;
        }
        self.dirty()
    }

    /// 最近のファイルの先頭に足す。Web 版の保存先（`/downloads/`、ダウンロードにして置き場から消える）は
    /// 開き直せないので足さない（GAR-12）
    fn push_recent(&mut self, file: &str) {
        if file.starts_with("/downloads/") {
            return;
        }
        self.s.recent.retain(|x| x != file);
        self.s.recent.insert(0, file.to_string());
        self.s.recent.truncate(10);
    }

    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// 読み直す。開いた系は前の系を段に積む（読み直すを元に戻せる）。組合せの系は組合せから作り直す
    fn reload(&mut self) {
        let Some(f) = self.loc.file.clone() else {
            return;
        };
        if self.is_combo() {
            self.open(&f);
            return;
        }
        self.worker
            .send(Tag::Load, "reload", json!({ "history": "push" }));
        self.info(trf("Reading {}…", &[&f.display()]));
    }

    /// 系のファイルを選んで開く。start のフレームで選び始める（Web 版は選び終わった後のフレームで開くので、
    /// 毎フレーム start = false で呼ぶ）
    fn pick_and_open_when(&mut self, start: bool) {
        if let Some(p) = crate::files::open("open-system", start, || {
            crate::files::FileDialog::new()
                .add_filter(tr("Systems (.zmx, project .json)"), &["zmx", "json"])
        }) {
            self.open_doc(&p);
        }
    }

    fn pick_and_open(&mut self) {
        self.pick_and_open_when(true);
    }

    fn refresh_catalog(&mut self) {
        self.folders.stale();
        // 系の解析の順番を待たない（下書きの worker で読む）
        self.preview.send(
            Tag::Glasses,
            "glasses",
            json!({ "glass_dirs": glass_dirs(&self.s.glass_dirs) }),
        );
        self.preview.send(
            Tag::Lenses,
            "lenses",
            json!({
                "lens_catalogs": dirs(&self.s.lens_dirs),
                "glass_dirs": glass_dirs(&self.s.glass_dirs),
            }),
        );
        self.browser.loaded = true;
        self.browser.glasses_pending = true;
        self.browser.lenses_pending = true;
    }

    /// serve の layout の params（重ねる構成・見る面と光線の設定）
    fn layout_params(&self) -> Value {
        let mut v = self.overlay.params_in(self.s.view_plane);
        self.s
            .rays
            .apply(&mut v, self.n_fields(), self.waves_um().len(), false);
        v
    }

    fn n_fields(&self) -> usize {
        self.sys
            .summary
            .as_ref()
            .and_then(|s| s["fields"].as_array().map(Vec::len))
            .unwrap_or(0)
    }

    /// パラメータの名前の欄の補完・階層選択の候補（面・玉・群・波長の数）
    /// 変数名の補完の候補を作る系の面・玉・群（6 つのパネルが毎フレーム呼ぶので rev ごとに 1 回だけ作る、GAR-09）
    fn param_sys(&self) -> std::rc::Rc<crate::param_pick::Sys> {
        if let Some((r, s)) = &*self.param_sys_cache.borrow() {
            if *r == self.rev {
                return s.clone();
            }
        }
        let s = std::rc::Rc::new(self.build_param_sys());
        *self.param_sys_cache.borrow_mut() = Some((self.rev, s.clone()));
        s
    }

    fn build_param_sys(&self) -> crate::param_pick::Sys {
        crate::param_pick::Sys {
            surfs: self
                .sys
                .ld_rows
                .iter()
                .map(|r| {
                    (
                        r.typ.clone(),
                        r.comment.clone(),
                        r.material.eq_ignore_ascii_case("MIRROR"),
                    )
                })
                .collect(),
            units: self.sys.ld_units.iter().map(|u| u.name.clone()).collect(),
            waves: self.waves_um().len(),
        }
    }

    fn waves_um(&self) -> Vec<f64> {
        // 配列を複製せずに読む（GAR-09）
        self.sys
            .summary
            .as_ref()
            .and_then(|s| s["wavelengths_um"].as_array())
            .map(|a| a.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default()
    }

    fn primary(&self) -> usize {
        self.sys
            .summary
            .as_ref()
            .and_then(|s| s["primary"].as_u64())
            .unwrap_or(1) as usize
    }

    /// 主波長の回折の遮断周波数 [cycles/mm]（作動 F 値から）
    fn cutoff(&self) -> Option<f64> {
        let f = self.info_num("wfno")?;
        let um = *self.waves_um().get(self.primary().saturating_sub(1))?;
        (f > 0.0 && um > 0.0).then(|| 1.0 / (um * 1e-3 * f))
    }

    /// 解析の params（cmd・視野・波長と、追加の引数の JSON。図に要る引数も足す）
    fn analysis_params(cfg: &AnaCfg, cutoff: Option<f64>) -> Result<Value, String> {
        let mut m = Map::new();
        m.insert("cmd".into(), json!(cfg.cmd));
        if layout::takes_sel(&cfg.cmd) {
            if cfg.field > 0 {
                m.insert("field".into(), json!(cfg.field));
            }
            if cfg.wave > 0 {
                m.insert("wave".into(), json!(cfg.wave));
            }
        }
        // 引数の欄（視野・波長は上の選択）と、拡張の解析の設定のオブジェクトの欄
        let specs = argform::analysis_specs(&cfg.cmd);
        let mut args = cfg.args.clone();
        if layout::takes_sel(&cfg.cmd) {
            args.remove("field");
            args.remove("wave");
        }
        // 周波数を書いたら上限・分割数は使わない（CLI では一緒に使えない）
        if args.contains_key("freqs") {
            args.remove("max_freq");
            args.remove("freq_steps");
        }
        m.extend(argform::to_map(&specs, &args)?);
        if let Some(o) = crate::ext::get(&cfg.cmd).and_then(|a| a.object) {
            if !cfg.sensor.is_empty() {
                let specs = argform::object_specs(&o);
                let sm = argform::to_map(&specs, &cfg.sensor)?;
                if !sm.is_empty() {
                    m.remove(o.arg);
                    m.insert(o.arg.into(), argform::nest(&sm));
                }
            }
        }
        let t = cfg.extra.trim();
        if !t.is_empty() {
            if !t.starts_with('{') {
                return Err(tr(
                    "Extra arguments must be a JSON object (e.g. {\"freqs\": [10, 20]})",
                )
                .into());
            }
            let v: Value =
                serde_json::from_str(t).map_err(|e| trf("Extra arguments: {}", &[&e]))?;
            if let Value::Object(o) = v {
                m.extend(o);
            }
        }
        figures::figure_params(&cfg.cmd, &mut m, cutoff, cfg.view == View::Figure);
        Ok(Value::Object(m))
    }

    /// 解析パネルを計算する（スポットの図なら光線も追跡する）
    fn run(&mut self, id: u64, cfg: &AnaCfg) {
        if self.sys.summary.is_none() {
            return;
        }
        match Self::analysis_params(cfg, self.cutoff()) {
            Ok(p) => {
                self.worker.send(Tag::Analyze(id), "analyze", p);
                self.ana_sent(id);
                self.info(trf("Computing {}…", &[&layout::analysis_label(&cfg.cmd)]));
            }
            Err(e) => {
                self.results.entry(id).or_default().error = Some(e.clone());
                self.fail(e);
                return;
            }
        }
        if cfg.cmd == "spot" && cfg.view == View::Figure {
            let (px, py) = plots::pupil_grid(8);
            let fields: Vec<usize> = if cfg.field > 0 {
                vec![cfg.field]
            } else {
                (1..=self.n_fields()).collect()
            };
            let nw = self.waves_um().len();
            let waves: Vec<usize> = if cfg.wave > 0 {
                vec![cfg.wave]
            } else {
                (1..=nw).collect()
            };
            let mut calls = Vec::new();
            for &f in &fields {
                for &w in &waves {
                    calls.push((
                        "trace".to_string(),
                        json!({ "px": px, "py": py, "field": f, "wave": w }),
                    ));
                }
            }
            self.worker.send_batch(Tag::Rays(id), calls);
        }
    }

    fn read_params(&mut self) {
        for p in self.s.params.clone() {
            self.worker.send(Tag::Get, "get", json!({ "param": p }));
        }
    }

    /// 組合せのプロジェクトを一時フォルダに書いて開く
    fn build_combo(&mut self, project: Value) {
        let dir = doc_loc::work_dir();
        let name: String = self
            .doc
            .combo
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let path = dir.join(format!(
            "{}.json",
            if name.is_empty() {
                "combo".into()
            } else {
                name
            }
        ));
        // 失敗しても同じ組合せを毎フレーム書き直さない
        self.loc.built_combo = Some(self.doc.combo.clone());
        // 別の名前に書いてから置き換える（系の読み込みが書きかけのファイルを読まないように）
        let tmp = path.with_extension("json.tmp");
        let res = lensbench_core::vfs::create_dir_all(&dir)
            .and_then(|_| {
                lensbench_core::vfs::write(
                    &tmp,
                    serde_json::to_string_pretty(&project).unwrap_or_default(),
                )
            })
            .and_then(|_| {
                // Windows で読んでいる最中なら置き換えられないので、そのときは上書き
                lensbench_core::vfs::rename(&tmp, &path).or_else(|_| {
                    lensbench_core::vfs::read(&tmp)
                        .and_then(|b| lensbench_core::vfs::write(&path, b))
                })
            });
        match res {
            Ok(()) => {
                self.loc.built = Some(project);
                self.loc.combo_path = Some(path.clone());
                // 図は下書きで先に。系の読み込みは、前の読み込みが終わってから最新のファイルを読む
                self.preview.send(
                    Tag::Layout,
                    "preview",
                    json!({ "load": self.load_params(&path), "layout": self.layout_params() }),
                );
                if self.worker.busy_core() {
                    self.load_stale = true;
                } else {
                    self.worker.drop_queued();
                    self.open(&path);
                }
            }
            Err(e) => self.fail(format!("{}: {e}", path.display())),
        }
    }

    /// 保存（Ctrl+S）・名前を付けて保存（Ctrl+Shift+S）。組合せの系は組合せを同梱したプロジェクトの .json に、
    /// 開いた系は系ファイル（.json）に書く。書き先は開いた・前に保存した .json（.zmx から開いた系・Web 版・
    /// as_new は書き先を選ぶ）（#129）
    fn save(&mut self, as_new: bool) {
        let combo =
            self.is_combo() || (self.loc.file.is_none() && !self.doc.combo.picks.is_empty());
        if !combo && self.loc.file.is_none() {
            return;
        }
        let keep = if as_new || cfg!(target_arch = "wasm32") {
            None
        } else {
            self.loc.doc_path.clone()
        };
        let path = match keep {
            Some(p) => p,
            None => {
                let name = (self.loc.doc_path.as_ref())
                    .or(self.loc.shown(combo))
                    .and_then(|f| f.file_stem())
                    .map(|x| x.to_string_lossy().into_owned())
                    .filter(|x| !x.is_empty())
                    .or_else(|| {
                        let n = self.doc.combo.name.trim();
                        (combo && !n.is_empty()).then(|| n.to_string())
                    })
                    .unwrap_or_else(|| "project".into());
                let Some(p) = crate::files::FileDialog::new()
                    .add_filter(tr("Project"), &["json"])
                    .set_file_name(format!("{name}.json"))
                    .save_file()
                else {
                    return;
                };
                p
            }
        };
        if combo {
            let mut v = doc_file::project_file(&self.doc.combo);
            let panels = self.panels_value();
            if let Some(o) = panels.as_object() {
                for (k, x) in o {
                    v["gui"][k] = x.clone();
                }
            }
            let r = lensbench_core::vfs::write(
                &path,
                serde_json::to_string_pretty(&v).unwrap_or_default(),
            );
            self.status = match r {
                Ok(()) => {
                    self.loc.doc_path = Some(path.clone());
                    self.loc.saved_combo = Some(self.doc.combo.clone());
                    self.push_recent(&path.display().to_string());
                    Status {
                        text: trf("Saved to {}", &[&path.display()]),
                        error: false,
                    }
                }
                Err(e) => Status {
                    text: format!("{}: {e}", path.display()),
                    error: true,
                },
            };
        } else {
            self.loc.doc_path = Some(path.clone());
            self.loc.doc_from = None;
            self.worker.send(
                Tag::Save,
                "save",
                json!({ "path": path.display().to_string() }),
            );
        }
    }

    /// レイアウトを書き出す（形式は拡張子）。今の見る面・重ねた構成で。2D は断面、3D は立体
    fn export_layout(&mut self) {
        let base = self
            .loc
            .file
            .as_ref()
            .and_then(|f| f.file_stem())
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_else(|| "layout".into());
        let ext = if self.s.view_plane == ViewPlane::D3 {
            "step"
        } else {
            "svg"
        };
        if let Some(p) = crate::files::FileDialog::new()
            .add_filter(tr("2D drawing"), &["svg", "png", "dxf"])
            .add_filter(
                tr("3D solids"),
                &["step", "stp", "glb", "gltf", "obj", "stl"],
            )
            .set_file_name(format!("{base}.{ext}"))
            .save_file()
        {
            let mut params = self.overlay.params_in(self.s.view_plane);
            self.s
                .rays
                .apply(&mut params, self.n_fields(), self.waves_um().len(), true);
            params["file"] = json!(p.display().to_string());
            self.worker.send(Tag::LayoutExport, "layout_export", params);
        }
    }

    fn write_zmx(&mut self) {
        if let Some(p) = crate::files::FileDialog::new()
            .add_filter("Zemax", &["zmx"])
            .save_file()
        {
            self.worker.send(
                Tag::Write,
                "write",
                json!({ "path": p.display().to_string() }),
            );
        }
    }

    /// 値の変更を送る（計算中なら終わってからまとめて）
    /// 値の変更を送る。解析でない要求（読み込み・前の値の変更）を待っている間だけ待つ。解析が回っている間は
    /// 送れば中断する（Set は系を変える）が、動かし続けると解析が 1 つも出ないので 250 ms に 1 回まで（GAR-06）
    fn flush_set(&mut self) {
        if self.pending_set.is_empty() || self.worker.busy_core() {
            return;
        }
        if self.worker.busy_short()
            && self
                .last_set
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(250))
        {
            return;
        }
        let values = std::mem::take(&mut self.pending_set);
        self.worker
            .send(Tag::Set, "set", json!({ "values": values }));
        self.last_set = Some(web_time::Instant::now());
    }

    fn info_num(&self, k: &str) -> Option<f64> {
        self.sys.info.as_ref().and_then(|i| i[k].as_f64())
    }

    // ---- パネル ----

    fn pane_ui(&mut self, ui: &mut Ui, pane: &mut Pane) {
        let id = pane.id;
        match &mut pane.kind {
            Kind::System => self.system_ui(ui),
            Kind::Catalog => self.catalog_ui(ui),
            Kind::Layout => self.layout_ui(ui),
            Kind::LensData => self.lens_data_ui(ui),
            Kind::MultiConfig => self.config_ui(ui),
            Kind::Optimize => self.optimize_ui(ui),
            Kind::Swap => self.swap_ui(ui),
            Kind::Tolerance => self.tolerance_ui(ui),
            Kind::Zoom => self.zoom_ui(ui),
            Kind::SysSettings => self.sysset_ui(ui),
            Kind::Analysis(cfg) => self.analysis_ui(ui, id, cfg),
        }
    }

    /// 開いている系は組合せから作ったものか
    /// レイアウト図を描き直す（重ねる構成を変えたとき）。組合せの系は下書きで
    fn request_layout(&mut self) {
        match (&self.loc.combo_path, self.is_combo()) {
            (Some(path), true) => {
                let load = self.load_params(path);
                self.preview.send(
                    Tag::Layout,
                    "preview",
                    json!({ "load": load, "layout": self.layout_params() }),
                );
            }
            _ => self
                .worker
                .send(Tag::Layout, "layout", self.layout_params()),
        }
    }

    /// 系を読んだ（開いた・読み直した・それを戻した）後: ファイル名・概要を置き、面の表・解析を読み直す
    fn on_loaded(&mut self, v: Value) {
        let new_file = v["file"].as_str().map(PathBuf::from);
        if new_file != self.loc.file {
            // 別の系: 前との差は出さない
            for r in self.results.values_mut() {
                r.value = None;
                r.prev = None;
            }
        }
        // 読み直した系には面の表の変更は無い。別の系なら階層・選択・ロックも戻す（組合せの作り直しは同じ系）
        self.loc.ld_dirty = false;
        if new_file != self.loc.file {
            self.p.ld.reset();
            let key = new_file.as_ref().map(|f| f.display().to_string());
            self.p.ld.locked = key
                .and_then(|k| self.s.locked.get(&k).cloned())
                .unwrap_or_default();
        }
        self.loc.file = new_file;
        self.sys.summary = Some(v);
        self.sys.info = None;
        self.refresh();
        self.read_params();
        // 組合せの図は下書きで作る（ここで作ると、動かしている途中の古い図に戻ることがある）
        if !self.is_combo() {
            self.worker
                .send(Tag::Layout, "layout", self.layout_params());
        }
        self.worker.send(Tag::Configs, "configs", json!({}));
        self.rerun = Some(false);
    }

    /// 面の表・要約・多重構成の表を 1 つの要求で読み直す（途中の状態を見せない、GAR-05）
    fn refresh(&mut self) {
        self.worker.send_batch(Tag::Refresh, take::refresh_calls());
    }

    /// レンズデータで面 j の行を選ぶ: 最上位のブロック（玉・群）の中なら、そのブロックの行（今のセルはブロックの最後の面）
    fn select_ld_surface(&mut self, j: usize) {
        self.p.ld.scope.clear();
        let top = self
            .sys
            .ld_blocks
            .iter()
            .find(|b| b.parent.is_none() && b.surfs.contains(&j));
        match top {
            Some(b) => {
                self.p.ld.sel = Some(b.name.clone());
                // ブロックの最後の面（たたんだ行の鍵。続けて足すとブロックの後に入る）
                let last = b.surfs.last().copied().unwrap_or(j);
                self.p.ld.cur = Some((last, lens_data::Col::R));
            }
            None => {
                self.p.ld.sel = None;
                self.p.ld.cur = Some((j, lens_data::Col::R));
            }
        }
    }

    fn is_combo(&self) -> bool {
        self.loc.file.is_some() && self.loc.file == self.loc.combo_path
    }

    /// 最適化
    fn optimize_ui(&mut self, ui: &mut Ui) {
        use crate::optim_panel::{self as op, Action};
        if self.sys.summary.is_none() {
            ui.add_space(8.0);
            ui.label(tr(
                "Open a system or build one from a combination to optimize.",
            ));
            return;
        }
        let is_combo = self.is_combo();
        let cands = op::Cands {
            units: self.sys.ld_units.iter().map(|u| u.name.clone()).collect(),
            combo: is_combo,
            sys: self.param_sys(),
            rows: &self.sys.ld_rows,
        };
        let progress = if self.p.opt.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
            crate::worker::progress::optimize()
        } else {
            None
        };
        let acts = op::ui(ui, &mut self.p.opt, &cands, progress.as_deref());
        let mut load = false;
        for a in acts {
            match a {
                Action::Run(p) => {
                    self.p.opt.running = true;
                    self.p.opt.result = None;
                    self.p.opt.gen = self.sys_gen;
                    self.worker.send(Tag::Optimize, "optimize", p);
                }
                Action::Stop => crate::worker::progress::stop("optimize"),
                Action::Apply => {
                    let Some(r) = self.p.opt.result.clone() else {
                        continue;
                    };
                    // 最適化の間・後に系が変わっていれば、古い系の値なので当てない（GAR-20）
                    if self.p.opt.gen != self.sys_gen || self.worker.busy_core() {
                        self.p.opt.error = Some(
                            tr("The system changed after the optimization started, so the result was not applied (optimize again)")
                                .to_string(),
                        );
                        continue;
                    }
                    if self.apply_values(&op::result_values(&r, false)) {
                        self.p.opt.undo = Some(op::result_values(&r, true));
                    }
                }
                Action::Undo => {
                    if let Some(u) = self.p.opt.undo.take() {
                        self.apply_values(&u);
                    }
                }
                Action::Save(v) => {
                    if let Some(path) = crate::files::FileDialog::new()
                        .add_filter(tr("Optimization settings"), &["json"])
                        .set_file_name("optimize.json")
                        .save_file()
                    {
                        let txt = serde_json::to_string_pretty(&v).unwrap_or_default();
                        self.status = match lensbench_core::vfs::write(&path, txt + "\n") {
                            Ok(()) => Status {
                                text: trf("Saved to {}", &[&path.display()]),
                                error: false,
                            },
                            Err(e) => Status {
                                text: format!("{}: {e}", path.display()),
                                error: true,
                            },
                        };
                    }
                }
                Action::Load => load = true,
            }
        }
        // 設定の読み込み（Web 版は選び終わった後のフレームで返るので毎フレーム見る）
        if let Some(path) = crate::files::open("opt-load", load, || {
            crate::files::FileDialog::new().add_filter(tr("Optimization settings"), &["json"])
        }) {
            let r = lensbench_core::vfs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                .and_then(|v| self.p.opt.set_params(&v));
            if let Err(e) = r {
                self.fail(format!("{}: {e}", path.display()));
            }
        }
    }

    /// モンテカルロ公差
    /// ズーム軌跡を解ける状態（.zmx の構成か、プロジェクトの状態）
    fn zoom_cands(&self) -> Vec<crate::zoom_panel::Cand> {
        self.overlay
            .items
            .iter()
            .filter(|x| x.0 == "cfgs" || x.0 == "states")
            .map(|x| (x.0 == "cfgs", x.1.clone()))
            .collect()
    }

    fn zoom_ui(&mut self, ui: &mut Ui) {
        if self.sys.summary.is_none() {
            ui.add_space(8.0);
            ui.label(tr("Open a system to solve a zoom track."));
            return;
        }
        let cands = self.zoom_cands();
        let sys = self.param_sys();
        if let Some(p) = crate::zoom_panel::ui(ui, &mut self.p.zoom, &cands, &sys) {
            self.p.zoom.running = true;
            self.p.zoom.result = None;
            self.worker.send(Tag::Zoom, "zoom", p);
        }
    }

    /// 文字列をファイルに保存する（保存先はダイアログで選ぶ）
    fn save_text(&mut self, filter: &str, ext: &str, name: &str, txt: String) {
        if let Some(path) = crate::files::FileDialog::new()
            .add_filter(filter, &[ext])
            .set_file_name(name)
            .save_file()
        {
            self.status = match lensbench_core::vfs::write(&path, txt) {
                Ok(()) => Status {
                    text: trf("Saved to {}", &[&path.display()]),
                    error: false,
                },
                Err(e) => Status {
                    text: format!("{}: {e}", path.display()),
                    error: true,
                },
            };
        }
    }

    fn tolerance_ui(&mut self, ui: &mut Ui) {
        use crate::tol_panel::{self as tp, Action};
        if self.sys.summary.is_none() {
            ui.add_space(8.0);
            ui.label(tr(
                "Open a system or build one from a combination to run tolerancing.",
            ));
            return;
        }
        let cands = crate::optim_panel::Cands {
            units: self.sys.ld_units.iter().map(|u| u.name.clone()).collect(),
            // 補償子は系の中で動かすだけ（組合せに当てない）ので、面の値の種類を絞らない
            combo: false,
            sys: self.param_sys(),
            rows: &self.sys.ld_rows,
        };
        let progress = if self.p.tol.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
            Some(crate::worker::progress::tolerance().unwrap_or((0, 0)))
        } else {
            None
        };
        let acts = tp::ui(ui, &mut self.p.tol, &cands, progress);
        let mut load = false;
        for a in acts {
            match a {
                Action::Run(p) => {
                    self.p.tol.running = true;
                    self.p.tol.result = None;
                    self.worker.send(Tag::Tolerance, "tolerance", p);
                }
                Action::Stop => crate::worker::progress::stop("tolerance"),
                Action::Save(v) => {
                    let txt = serde_json::to_string_pretty(&v).unwrap_or_default() + "\n";
                    self.save_text(tr("Tolerance settings"), "json", "tolerance.json", txt);
                }
                Action::SaveResult(v) => {
                    let txt = serde_json::to_string_pretty(&v).unwrap_or_default() + "\n";
                    self.save_text(
                        tr("Tolerance results"),
                        "json",
                        "tolerance_result.json",
                        txt,
                    );
                }
                Action::SaveCsv(txt) => {
                    self.save_text("CSV", "csv", "tolerance_compensators.csv", txt);
                }
                Action::Load => load = true,
            }
        }
        // 設定の読み込み（Web 版は選び終わった後のフレームで返るので毎フレーム見る）
        if let Some(path) = crate::files::open("tol-load", load, || {
            crate::files::FileDialog::new().add_filter(tr("Tolerance settings"), &["json"])
        }) {
            let r = lensbench_core::vfs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                .and_then(|v| self.p.tol.set_spec(&v));
            if let Err(e) = r {
                self.fail(format!("{}: {e}", path.display()));
            }
        }
    }

    /// 差し替え探索
    fn swap_ui(&mut self, ui: &mut Ui) {
        use crate::swap_panel::{self as sp, Action};
        if self.sys.summary.is_none() {
            ui.add_space(8.0);
            ui.label(tr(
                "Build a system from a combination, or open a project with lens blocks, to run a swap search.",
            ));
            return;
        }
        let is_combo = self.is_combo();
        let (project, mut cands) = if is_combo {
            let (p, c) = sp::combo_project(&self.doc.combo);
            (Some(p), c)
        } else {
            let f = self.loc.file.clone().unwrap_or_default();
            if self.swap_file.as_ref().map(|x| &x.0) != Some(&f) {
                let c = lensbench_core::vfs::read_to_string(&f)
                    .ok()
                    .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                    .map(|v| sp::project_cands(&v))
                    .unwrap_or_default();
                self.swap_file = Some((f, c));
            }
            (
                None,
                std::mem::take(&mut self.swap_file.as_mut().unwrap().1),
            )
        };
        // 同時に評価できる状態（組合せは組合せの状態、開いたプロジェクトは読み込んだ系の状態）
        cands.states = if is_combo {
            self.doc.combo.states.clone()
        } else {
            self.overlay
                .items
                .iter()
                .filter(|x| x.0 == "states")
                .map(|x| x.1.clone())
                .collect()
        };
        let progress = if self.p.swap.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
            crate::worker::progress::swap()
        } else {
            None
        };
        cands.sys = self.param_sys();
        let acts = sp::ui(ui, &mut self.p.swap, &cands, progress);
        if !is_combo {
            if let Some(x) = self.swap_file.as_mut() {
                x.1 = cands;
            }
        }
        let mut load = false;
        for a in acts {
            let count = matches!(a, Action::Count(_));
            match a {
                Action::Run(_) | Action::Count(_) if self.p.swap.running => {}
                Action::Run(mut v) | Action::Count(mut v) => {
                    if count {
                        v["count_only"] = json!(true);
                    }
                    if let Some(p) = &project {
                        v["project"] = p.clone();
                    }
                    v["top"] = json!(self.p.swap.top);
                    self.swap_ids = self.doc.combo.ids();
                    self.p.swap.running = true;
                    self.p.swap.result = None;
                    self.worker.send(Tag::Swap, "swap", v);
                }
                Action::Stop => crate::worker::progress::stop("swap"),
                Action::Insert(i) => {
                    let res = self.p.swap.result.as_ref().and_then(|r| r.results.as_ref());
                    let Some(row) = res.and_then(|r| r.get(i)).cloned() else {
                        continue;
                    };
                    let before = self.doc.combo.clone();
                    match sp::insert_row(&mut self.doc.combo, &self.swap_ids, &row) {
                        Ok(()) => {
                            if self.p.swap.undo.is_none() {
                                self.p.swap.undo = Some(before);
                            }
                            self.p.swap.inserted = Some(i);
                            if !self.s.auto_build {
                                self.build_combo(self.doc.combo.project());
                            }
                        }
                        Err(e) => self.fail(e),
                    }
                }
                Action::Undo => {
                    if let Some(c) = self.p.swap.undo.take() {
                        self.doc.combo = c;
                        self.p.swap.inserted = None;
                        if !self.s.auto_build {
                            self.build_combo(self.doc.combo.project());
                        }
                    }
                }
                Action::Save(v) => {
                    if let Some(path) = crate::files::FileDialog::new()
                        .add_filter(tr("Swap search settings"), &["json"])
                        .set_file_name("swap.json")
                        .save_file()
                    {
                        let txt = serde_json::to_string_pretty(&v).unwrap_or_default();
                        self.status = match lensbench_core::vfs::write(&path, txt + "\n") {
                            Ok(()) => Status {
                                text: trf("Saved to {}", &[&path.display()]),
                                error: false,
                            },
                            Err(e) => Status {
                                text: format!("{}: {e}", path.display()),
                                error: true,
                            },
                        };
                    }
                }
                Action::Load => load = true,
            }
        }
        // 設定の読み込み（Web 版は選び終わった後のフレームで返るので毎フレーム見る）
        if let Some(path) = crate::files::open("swap-load", load, || {
            crate::files::FileDialog::new().add_filter(tr("Swap search settings"), &["json"])
        }) {
            let r = lensbench_core::vfs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                .and_then(|v| self.p.swap.set_spec(&v));
            if let Err(e) = r {
                self.fail(format!("{}: {e}", path.display()));
            }
        }
    }

    /// 変数の値を開いている系に当てる（開いた系は serve の set、組合せの系は組合せの要素に当てて作り直す）。
    /// 当てられない値があれば何もせず false
    fn apply_values(&mut self, vals: &[(String, f64)]) -> bool {
        if vals.is_empty() {
            return false;
        }
        if !self.is_combo() {
            let m: Map<String, Value> = vals.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
            self.worker.send(Tag::Set, "set", json!({ "values": m }));
            return true;
        }
        let mut edits = Vec::new();
        for (n, x) in vals {
            match crate::optim_panel::combo_edit(n, *x) {
                Ok(e) => edits.push(e),
                Err(n) => {
                    self.fail(trf(
                        "Variable {} cannot be applied to the combination (only surface thickness, radius and conic)",
                        &[&n],
                    ));
                    return false;
                }
            }
        }
        let ids = self.doc.combo.span_ids();
        let lead = self.doc.combo.lead_stop();
        let spans = lens_data::elem_spans(&ids, &self.sys.ld_units, lead, &self.sys.ld_rows);
        for (j, key, value) in edits {
            if !self.combo_surface_edit(&spans, j, key, value) {
                return false;
            }
        }
        if !self.s.auto_build {
            self.build_combo(self.doc.combo.project());
        }
        true
    }

    /// 組合せの系の設定
    fn sysset_ui(&mut self, ui: &mut Ui) {
        self.ask_folders();
        if self.loc.file.is_some() && !self.is_combo() {
            // 開いた系の設定。組合せの設定はその下にたたむ
            if self.p.oss.want() {
                self.worker.send(Tag::SysData, "system_data", json!({}));
            }
            let ns = self.sys.ld_rows.len();
            egui::ScrollArea::vertical()
                .id_salt("sysset_scroll")
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(tr(
                            "Settings of the open system. Changes go into the system; keep them with \"Save system file…\".",
                        ))
                        .small()
                        .weak(),
                    );
                    if let Some(op) = open_sysset::ui(ui, &mut self.p.oss, ns) {
                        self.worker.send(Tag::Edit, "edit", json!({ "ops": [op] }));
                    }
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new(tr("Combination settings (used when a combination is built)"))
                        .id_salt("combo_sysset")
                        .default_open(false)
                        .show(ui, |ui| {
                            crate::sysset::ui(ui, &mut self.doc.combo, &self.folders.agf)
                        });
                });
            return;
        }
        let cats = &self.folders.agf;
        egui::ScrollArea::vertical()
            .id_salt("sysset_scroll")
            .show(ui, |ui| crate::sysset::ui(ui, &mut self.doc.combo, cats));
    }

    /// 組合せのレイアウト図と要素の操作
    fn layout_ui(&mut self, ui: &mut Ui) {
        let is_combo = self.is_combo();
        // 開いた系の図の下の欄（図と分けたとき）
        let mut below: Option<Ui> = None;
        let accent = theme::accent(&self.s.look);
        if self.loc.file.is_some() {
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label(tr("View plane"));
                for p in ViewPlane::ALL {
                    let r = ui.selectable_value(&mut self.s.view_plane, p, p.label());
                    let r = match p {
                        ViewPlane::Unfold => r.on_hover_text(tr(
                            "Sections after mirror/BS folds laid out along the original axis (moves along the axis only)",
                        )),
                        ViewPlane::D3 => r.on_hover_text(tr(
                            "Drag to rotate, Shift- or right-drag to pan, wheel to zoom, double-click to reset",
                        )),
                        _ => r,
                    };
                    changed |= r.changed();
                }
                ui.separator();
                if ui
                    .button(tr("Export…"))
                    .on_hover_text(tr(
                        "Export with the current view and overlaid configurations. The format follows the extension: svg, png, dxf for 2D; step, glb, gltf, obj, stl for 3D",
                    ))
                    .clicked()
                {
                    self.export_layout();
                }
            });
            changed |= self.overlay.ui(ui, self.s.view_plane.vert());
            let (nf, nw) = (self.n_fields(), self.waves_um().len());
            let head = if self.s.rays.is_default() {
                tr("Rays and export settings").to_string()
            } else {
                format!("{} ●", tr("Rays and export settings"))
            };
            egui::CollapsingHeader::new(head)
                .id_salt("layout_rays")
                .open(std::mem::take(&mut self.rays_open).then_some(true))
                .show(ui, |ui| {
                    changed |= self.s.rays.ui(ui, nf, nw);
                });
            if changed {
                self.request_layout();
            }
        }
        if self.loc.file.is_some() && !is_combo {
            ui.horizontal_wrapped(|ui| {
                ui.weak(tr(
                    "The open system was not built from the combination (the layout is view-only).",
                ));
                if ui
                    .button(tr("Make this system a combination"))
                    .on_hover_text(tr(
                        "Make the opened system one element of a combination (a system import) and move its settings (aperture, fields, wavelengths, …). Catalog lenses etc. can then be added before or after it",
                    ))
                    .clicked()
                {
                    if self.doc.combo.picks.is_empty() {
                        self.to_combo = Some(false);
                    } else {
                        self.ask(
                            tr("Make a combination"),
                            tr("Replace the current combination with one that imports the opened system?"),
                            |c, yes| {
                                if yes {
                                    c.to_combo = Some(false);
                                }
                            },
                        );
                    }
                }
                if !self.doc.combo.picks.is_empty()
                    && ui.button(tr("▶ Build the combination")).clicked()
                {
                    self.build_combo(self.doc.combo.project());
                }
            });
            // 開いている系のレイアウト図（組合せと同じ描き方）
            // 組合せの要素のパレット・図はその下に
            // 図と下の組合せの欄に残りを分ける（パネルが低くても下が押し出されないように）
            if let Some(d) = &self.layout_data {
                let (mut top, b) = split::two(ui, "open_layout", (0.55, 0.55), 80.0);
                let h = top.available_height();
                let hl = self.p.ld.cur.map(|x| x.0);
                let nav = Some(&mut self.p.open_nav);
                let clicked = combo::view_only(&mut top, d, h, accent, nav, hl);
                match clicked {
                    // 図でクリックした玉の行をレンズデータで選ぶ
                    Some(combo::OpenAct::Clicked([a, _])) => self.select_ld_surface(a),
                    // カタログの行を落とした間隔に差し込む（開いた系を組合せにして）
                    Some(combo::OpenAct::Drop { after, pick }) => {
                        let n = self.doc.combo.picks.len();
                        self.insert_into_open(Some(after), *pick, n, |_, _| {});
                    }
                    None => {}
                }
                below = Some(b);
            }
        }
        let ui = match below.as_mut() {
            Some(b) => b,
            None => ui,
        };
        let data = if is_combo {
            self.layout_data.as_ref()
        } else {
            None
        };
        let paths: Vec<String> = self
            .overlay
            .items
            .iter()
            .filter(|x| x.0 == "paths")
            .map(|x| x.1.clone())
            .collect();
        // 面の表で選んだ面を図で目立たせる（#125）
        self.p.combo_view.hl_surf = if is_combo {
            self.p.ld.cur.map(|x| x.0)
        } else {
            None
        };
        let act = combo::ui(
            ui,
            &mut self.doc.combo,
            data,
            &mut self.p.combo_view,
            &mut self.s.auto_build,
            self.worker.busy_short(),
            accent,
            &paths,
        );
        match act {
            Some(combo::Act::Catalog) => self.cmds.push(Cmd::FocusKind(Kind::Catalog)),
            Some(combo::Act::Build) => self.build_combo(self.doc.combo.project()),
            Some(combo::Act::Focus) => self.request_focus(),
            Some(combo::Act::SaveToCatalog(k)) => {
                self.p.cat_save.open_for(k, &self.doc.combo);
                let c = self.p.cat_save.catalog.trim().to_string();
                if !c.is_empty() && self.p.cat_save.listed != c {
                    self.preview
                        .send(Tag::UserCatalog, "catalog_items", json!({ "catalog": c }));
                }
            }
            Some(combo::Act::Open(k)) => {
                if let Some(id) = self.doc.combo.ids().get(k) {
                    // 面の表がまだ前の系なら、木に無い名前は入れない（最上位で選ぶ）
                    self.p.ld.scope.clear();
                    self.p.ld.enter(&self.sys.ld_blocks, id);
                    if self.p.ld.scope.is_empty() {
                        self.p.ld.sel = Some(id.clone());
                    }
                }
                self.cmds.push(Cmd::FocusKind(Kind::LensData));
            }
            None => {}
        }
    }

    /// 面の表: 開いた系は serve の edit で変え、組合せの系は組合せ（要素の後の空気・面の上書き）を変えて作り直す
    fn lens_data_ui(&mut self, ui: &mut Ui) {
        use lens_data::{Act, Col};
        if self.sys.summary.is_none() || self.sys.ld_rows.is_empty() {
            ui.add_space(8.0);
            ui.label(tr(
                "Open a system or build one from the combination to see its surfaces here.",
            ));
            return;
        }
        let is_combo = self.is_combo();
        let ids = if is_combo {
            self.doc.combo.ids()
        } else {
            Vec::new()
        };
        // 組合せ: 図で選んだ要素を表でも選ぶ（最上位のとき）
        if is_combo && self.p.ld.scope.is_empty() {
            if let Some(k) = self.p.combo_view.sel {
                self.p.ld.sel = ids.get(k).cloned();
            }
        }
        // 今の経路に沿った要素（分岐の無い組合せは並びそのもの）
        let picks = &self.doc.combo.chain_picks();
        let locked: BTreeSet<String> = if is_combo {
            ids.iter()
                .zip(picks)
                .filter(|(_, p)| p.locked)
                .map(|(i, _)| i.clone())
                .collect()
        } else {
            self.p.ld.locked.clone()
        };
        let lead = self.doc.combo.lead_stop();
        let spans = if is_combo {
            let live = self.doc.combo.span_ids();
            lens_data::elem_spans(&live, &self.sys.ld_units, lead, &self.sys.ld_rows)
        } else {
            Vec::new()
        };
        // 図で別の要素を選んだら、その要素の外の今の面は外す（図の目立たせと表を合わせる）
        if is_combo && self.p.combo_view.sel != self.combo_sel_prev {
            if let (Some(k), Some((j, _))) = (self.p.combo_view.sel, self.p.ld.cur) {
                let inside = spans
                    .get(k)
                    .copied()
                    .flatten()
                    .is_some_and(|s| s[0] <= j && j <= s[1]);
                if !inside {
                    self.p.ld.cur = None;
                }
            }
        }
        self.combo_sel_prev = self.p.combo_view.sel;
        let rows = &self.sys.ld_rows;
        let read_only = |j: usize, col: Col| -> Option<String> {
            ld_combo::read_only(is_combo, rows, &spans, picks, lead, j, col)
        };
        let overridden =
            |j: usize, col: Col| is_combo && ld_combo::overridden(rows, &spans, picks, j, col);
        let cfg_rows: Vec<String> = self
            .p
            .mc
            .table
            .as_ref()
            .map(|t| t.rows.iter().map(|r| r.0.clone()).collect())
            .unwrap_or_default();
        let ctx = lens_data::Ctx {
            config_n: self.p.mc.table.as_ref().map_or(1, |t| t.n),
            config_rows: &cfg_rows,
            glasses: &self.ld_glasses,
            glasses_loaded: self.browser.loaded && !self.browser.glasses_pending,
            rows,
            blocks: &self.sys.ld_blocks,
            locked: &locked,
            read_only: &read_only,
            overridden: &overridden,
            combo: is_combo,
            unit_mech: (!is_combo).then_some(self.um_open),
            note: is_combo.then(|| tr(
                "Combination system: changes go into the combination elements (the trailing air is the element spacing). Catalog lenses and imported elements keep them as surface overrides, marked in the cell (right-click to remove). Set decenter, tilt and BS in the element settings.",
            )),
        };
        let (acts, mut bottom) = lens_data::ui(ui, &mut self.p.ld, &ctx);
        for a in acts {
            match a {
                Act::Select(name) => {
                    if is_combo && self.p.ld.scope.is_empty() {
                        if let Some(k) = name.and_then(|n| ids.iter().position(|i| *i == n)) {
                            self.p.combo_view.select(k);
                        }
                    }
                }
                Act::Lock(name, on) => {
                    if is_combo {
                        match ids.iter().position(|i| *i == name) {
                            Some(k) => {
                                if let Some(p) = self.doc.combo.el_mut(k) {
                                    p.locked = on;
                                }
                            }
                            None => self.fail(tr("In a combination, lock whole elements")),
                        }
                    } else {
                        if on {
                            self.p.ld.locked.insert(name);
                        } else {
                            self.p.ld.locked.remove(&name);
                        }
                        // ファイルごとに設定へ残す（空になったら消す、GPN-14）
                        if let Some(f) = self.loc.file.as_ref().map(|f| f.display().to_string()) {
                            if self.p.ld.locked.is_empty() {
                                self.s.locked.remove(&f);
                            } else {
                                self.s.locked.insert(f, self.p.ld.locked.clone());
                            }
                        }
                    }
                }
                Act::ExportRange([a, b]) => {
                    let stem = self
                        .loc
                        .file
                        .as_ref()
                        .and_then(|f| f.file_stem())
                        .map(|x| x.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "system".into());
                    if let Some(p) = crate::files::FileDialog::new()
                        .add_filter(tr("System file"), &["json"])
                        .set_file_name(format!("{stem}_{a}-{b}.json"))
                        .save_file()
                    {
                        self.worker.send(
                            Tag::SaveRange,
                            "save_range",
                            json!({ "path": p.display().to_string(), "from": a, "to": b }),
                        );
                    }
                }
                Act::UnitMech => {
                    self.um_open = !self.um_open;
                    // 範囲の欄が空なら、選んでいる面から
                    if self.um_open && self.p.um.new_spans.trim().is_empty() {
                        if let Some((j, _)) = self.p.ld.cur {
                            self.p.um.new_spans = j.to_string();
                        }
                    }
                }
                Act::Refused(why) => self.fail(why),
                Act::Edit { j, col, value } => {
                    let mut fields = Map::new();
                    fields.insert(col.field().into(), value);
                    self.surface_fields(is_combo, &spans, j, fields, false);
                }
                Act::Fields { j, fields } => {
                    self.surface_fields(is_combo, &spans, j, fields, false)
                }
                Act::Data { j, data } => self.surface_fields(is_combo, &spans, j, data, true),
                Act::Ops(ops) => {
                    if is_combo {
                        match self.combo_ops(&spans, &ids, &ops) {
                            Ok(()) => {
                                if !self.s.auto_build {
                                    self.build_combo(self.doc.combo.project());
                                }
                            }
                            Err(e) => self.fail(e),
                        }
                    } else {
                        self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
                    }
                }
                Act::DropOverride { j, col } => {
                    if let Some((k, i)) = self.combo_face(&spans, j) {
                        let names = ld_combo::override_fields(col);
                        if self.doc.combo.el(k).is_some_and(|p| p.locked) {
                            self.fail(tr("A locked element cannot be changed"));
                            continue;
                        }
                        if let Some(p) = self.doc.combo.el_mut(k) {
                            p.edits
                                .retain(|e| !(e.surface == i && names.contains(&e.field.as_str())));
                        }
                        if !self.s.auto_build {
                            self.build_combo(self.doc.combo.project());
                        }
                    }
                }
                Act::DropPick { after, pick } => {
                    if is_combo {
                        // 組合せの系: 面 after を持つ要素の後（要素の外の面なら、その面より前の要素の後）
                        let at = spans
                            .iter()
                            .rposition(|s| s.is_some_and(|s| s[0] <= after))
                            .map_or(0, |k| k + 1);
                        let k = combo::insert_pick(&mut self.doc.combo, at, *pick);
                        self.p.combo_view.select(k);
                    } else if self.loc.file.is_some() {
                        if open_gap_ok(&self.sys.ld_rows, after) {
                            let n = self.doc.combo.picks.len();
                            self.insert_into_open(Some(after), *pick, n, |_, _| {});
                        } else {
                            self.fail(trf(
                                "Cannot insert after surface {}: not an air gap",
                                &[&after],
                            ));
                        }
                    }
                }
                Act::NeedGlasses => {
                    if !self.browser.glasses_pending {
                        self.refresh_catalog();
                    }
                }
                Act::OptVar(name) => {
                    if !self.p.opt.vars.iter().any(|v| v.name.trim() == name) {
                        // 空の行があればそこに入れる
                        match self
                            .p
                            .opt
                            .vars
                            .iter_mut()
                            .find(|v| v.name.trim().is_empty())
                        {
                            Some(v) => v.name = name.clone(),
                            None => self.p.opt.vars.push(crate::optim_panel::VarRow {
                                name: name.clone(),
                                ..Default::default()
                            }),
                        }
                    }
                    self.cmds.push(Cmd::FocusKind(Kind::Optimize));
                    self.info(trf("Added {} to the optimization variables", &[&name]));
                }
                Act::NeedData(j) => {
                    self.worker
                        .send(Tag::SurfData(j), "surface_data", json!({ "surface": j }));
                }
            }
        }
        // ブロックの設定（組合せは要素の設定、開いた系は玉・群の偏心・傾き・移動・dn・disp）と、
        // 開いた系の玉・群を足す・機構部品の欄（同じスクロールの中）
        let view = self.p.ld.block_view.clone();
        if view.is_some() {
            egui::ScrollArea::vertical()
                .id_salt("block_edit")
                .auto_shrink([false, false])
                .show(&mut bottom, |ui| {
                    if let Some(name) = &view {
                        if is_combo {
                            match ids.iter().position(|i| i == name) {
                                Some(k) => combo::block_settings(ui, &mut self.doc.combo, k),
                                None => {
                                    ui.weak(trf(
                                        "{} is not a combination element (select the element for its settings)",
                                        &[name],
                                    ));
                                }
                            }
                        } else {
                            self.unit_settings(ui, name);
                        }
                    }
                });
        }
        // 開いた系の玉・群を足す・機構部品は別の小窓（面の表の下では、面の設定の欄に押し出されて見えないことがあった、#136）
        if !is_combo && self.um_open {
            let mut open = true;
            let n = self.sys.ld_rows.len();
            let mut ops = Vec::new();
            egui::Window::new(tr("Lenses, groups and mechanics"))
                .id(egui::Id::new("unit_mech_win"))
                .open(&mut open)
                .default_width(380.0)
                .vscroll(true)
                .show(ui.ctx(), |ui| {
                    ops = crate::unit_mech::ui(ui, &mut self.p.um, n);
                });
            if !ops.is_empty() {
                self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
            }
            self.um_open = open;
        }
    }

    /// 多重構成の次（back なら前）の構成を系に入れる（Ctrl+A・Ctrl+Shift+A、最後の次は最初）
    fn cycle_config(&mut self, back: bool) {
        let Some(t) = self.p.mc.table.as_ref().filter(|t| t.n > 1) else {
            return;
        };
        let k = if back {
            if t.active <= 1 {
                t.n
            } else {
                t.active - 1
            }
        } else if t.active >= t.n {
            1
        } else {
            t.active + 1
        };
        self.info(trf("Configuration {} / {}", &[&k, &t.n]));
        self.worker.send(
            Tag::Edit,
            "edit",
            json!({ "ops": [{ "op": "set_config", "k": k }] }),
        );
    }

    /// 多重構成の表
    fn config_ui(&mut self, ui: &mut Ui) {
        let combo = self.is_combo();
        let cands = self.param_sys();
        for a in config_panel::ui(ui, &mut self.p.mc, combo, &cands) {
            match a {
                config_panel::Act::Ops(ops) => {
                    self.p.mc.notes.clear();
                    self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
                }
                config_panel::Act::Import => {
                    self.worker.send(Tag::ConfigDiff, "config_diff", json!({}));
                }
                config_panel::Act::Refused(why) => self.fail(why),
            }
        }
    }

    /// 開いた系の玉・群の設定（serve の set の `名前.dx` など）
    fn unit_settings(&mut self, ui: &mut Ui, name: &str) {
        // 名前・範囲を変える・外す（serve の edit）
        if let Some(u) = self.sys.ld_units.iter().find(|u| u.name == name) {
            let moved = u.motion.iter().any(|&x| x != 0.0) || u.shift != 0.0;
            let spans = u.spans.clone();
            let ops = crate::unit_mech::unit_ui(ui, &mut self.p.um, name, &spans, moved);
            for op in &ops {
                match op["op"].as_str() {
                    Some("remove_unit") => self.p.ld.block_view = None,
                    _ => {
                        if let Some(n) = op["name"].as_str() {
                            self.p.ld.block_view = Some(n.to_string());
                            self.p.ld.sel = Some(n.to_string());
                        }
                    }
                }
            }
            if !ops.is_empty() {
                self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
            }
        }
        let Some(u) = self.sys.ld_units.iter_mut().find(|u| u.name == name) else {
            return;
        };
        let weak = theme::weak_text(ui.visuals());
        let locked = self
            .sys
            .ld_blocks
            .iter()
            .position(|b| b.name == name)
            .is_some_and(|i| lens_data::block_locked(&self.sys.ld_blocks, &self.p.ld.locked, i));
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(name).strong());
            let spans = u
                .spans
                .iter()
                .map(|s| format!("{}–{}", s[0], s[1]))
                .collect::<Vec<_>>()
                .join(", ");
            ui.label(RichText::new(trf("Surface {}", &[&spans])).color(weak));
            if locked {
                ui.label(RichText::new(trf("{} is locked", &[&name])).color(weak));
            }
        });
        let mut set = Vec::new();
        // ロックは中身（dn・disp）だけを止める。偏心・傾き・移動は変えられる
        {
            ui.horizontal_wrapped(|ui| {
                let labels = [
                    (tr("Decenter x"), " mm"),
                    (tr("Decenter y"), " mm"),
                    (tr("Tilt x"), "°"),
                    (tr("Tilt y"), "°"),
                    (tr("Tilt z"), "°"),
                ];
                for (i, (k, (label, unit))) in ["dx", "dy", "tx", "ty", "tz"]
                    .iter()
                    .zip(labels)
                    .enumerate()
                {
                    ui.label(RichText::new(label).color(weak));
                    let r = ui.add(
                        egui::DragValue::new(&mut u.motion[i])
                            .speed(0.01)
                            .max_decimals(6)
                            .suffix(unit),
                    );
                    if r.changed() {
                        set.push((*k, u.motion[i]));
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("Shift")).color(weak))
                    .on_hover_text(tr(
                        "Group shift (positive along the light). Added to the gap before and taken from the gap after",
                    ));
                if ui
                    .add(
                        egui::DragValue::new(&mut u.shift)
                            .speed(0.01)
                            .max_decimals(6)
                            .suffix(" mm"),
                    )
                    .changed()
                {
                    set.push(("shift", u.shift));
                }
                for (k, v) in [("dn", &mut u.dn), ("disp", &mut u.disp)] {
                    ui.label(RichText::new(k).color(weak));
                    match v {
                        Some(x) => {
                            if ui
                                .add_enabled(
                                    !locked,
                                    egui::DragValue::new(x).speed(1e-5).max_decimals(8),
                                )
                                .changed()
                            {
                                set.push((k, *x));
                            }
                        }
                        None => {
                            ui.weak("—").on_hover_text(tr(
                                "No medium, or the value differs per surface (change it in the surface settings)",
                            ));
                        }
                    }
                }
            });
        }
        for (k, v) in set {
            self.pending_set.insert(format!("{name}.{k}"), v);
        }
    }

    /// 組合せの系の面 j の値を、組合せの要素に当てて作り直す
    /// 面 j の値を変える。開いた系は serve の edit（data は set_surface_data、ほかは set_surface）、組合せの系は
    /// 組合せの要素に当てて作り直す
    fn surface_fields(
        &mut self,
        is_combo: bool,
        spans: &[Option<[usize; 2]>],
        j: usize,
        fields: Map<String, Value>,
        data: bool,
    ) {
        if !is_combo {
            let op = if data {
                json!({ "op": "set_surface_data", "surface": j, "data": fields })
            } else {
                json!({ "op": "set_surface", "surface": j, "fields": fields })
            };
            self.worker.send(Tag::Edit, "edit", json!({ "ops": [op] }));
            return;
        }
        for (key, value) in fields {
            if !self.combo_surface_edit(spans, j, &key, value) {
                return;
            }
        }
        if !self.s.auto_build {
            self.build_combo(self.doc.combo.project());
        }
    }

    /// 組合せの系の面 j（系の面の番号）の要素と、要素の面の番号（座標ブレークを除く）
    fn combo_face(&self, spans: &[Option<[usize; 2]>], j: usize) -> Option<(usize, usize)> {
        ld_combo::face_of(&self.sys.ld_rows, spans, j)
    }

    /// 組合せの系の面 j の値 key を、組合せの要素に当てる（要素の最後の面の厚さは後の空気、ほかは要素の中身か面の上書き）
    fn combo_surface_edit(
        &mut self,
        spans: &[Option<[usize; 2]>],
        j: usize,
        key: &str,
        value: Value,
    ) -> bool {
        let rows = &self.sys.ld_rows;
        let k = spans
            .iter()
            .position(|s| s.is_some_and(|s| s[0] <= j && j <= s[1]));
        let x = value.as_f64();
        match (k, key, x) {
            // 前の絞りの厚さ = 絞りから最初の要素まで
            (None, "t", Some(x)) if x >= 0.0 && rows[j].stop => self.doc.combo.stop_gap = x,
            (Some(k), "t", Some(x)) if spans[k].is_some_and(|s| s[1] == j) => {
                // 要素の最後の面の厚さ = 後の空気（鏡の後は符号が逆）
                let air = if rows[j].t < 0.0 { -x } else { x };
                if air < 0.0 {
                    self.fail(tr("The spacing must be 0 or more"));
                    return false;
                }
                match self.doc.combo.gap_after(k) {
                    Some(v) => *v = air,
                    None => {
                        self.fail(tr("This value cannot be changed here"));
                        return false;
                    }
                }
            }
            (Some(k), _, _) => {
                let a = spans[k].map(|s| s[0]).unwrap_or(j);
                let rel = (a..j).filter(|&i| !rows[i].is_cb()).count();
                if let Some(p) = self.doc.combo.el_mut(k) {
                    p.write_face(rel, key, value);
                }
            }
            _ => {
                self.fail(tr("This value cannot be changed here"));
                return false;
            }
        }
        true
    }

    /// 組合せが変わっていて系の読み込みが空いていれば、系を作り直す。待っている解析は捨てる
    /// （作り直した後にまとめて回す）
    fn auto_build(&mut self) {
        // 組合せでない系を開いたまま空の組合せに要素を足したら、開いた系を組合せの先頭に取り込む（足した玉だけの系に
        // 置き換わらないように、#135）
        let n = self.doc.combo.picks.len();
        if self.loc.file.is_some()
            && !self.is_combo()
            && self.to_combo.is_none()
            && self.combo_len_prev == 0
            && n > 0
            && !self
                .loc
                .built_combo
                .as_ref()
                .is_some_and(|b| !b.picks.is_empty())
        {
            self.to_combo = Some(true);
        }
        self.combo_len_prev = n;
        if let Some(keep) = self.to_combo {
            self.open_to_combo(keep);
            return;
        }
        if self.load_stale && !self.worker.busy_core() {
            self.load_stale = false;
            if let Some(p) = self.loc.combo_path.clone() {
                self.worker.drop_queued();
                self.open(&p);
            }
        }
        if !self.s.auto_build
            || self.preview.busy()
            || self.doc.combo.picks.is_empty()
            || self.loc.built_combo.as_ref() == Some(&self.doc.combo)
            || !self.doc.combo.settings_problems().is_empty()
        {
            return;
        }
        self.combo_edit = Some(web_time::Instant::now());
        self.build_combo(self.doc.combo.project());
    }

    /// 像面の前の面まで、軸上の細い光線を追う
    fn request_focus(&mut self) {
        let Some(img) = self.layout_data.as_ref().and_then(|d| d.image_surface()) else {
            return;
        };
        if img < 2 {
            return;
        }
        self.worker.send(
            Tag::Focus,
            "trace",
            json!({ "px": [0.0], "py": [1e-3], "hx": 0.0, "hy": 0.0, "last": img - 1 }),
        );
    }

    /// 細い光線が軸と交わる所までを、最後の空気にする
    fn focus_from(&mut self, d: &Done) {
        let Some(Ok((_, a))) = d.replies.first().map(|r| r.result.as_ref()) else {
            return;
        };
        let get = |k: &str| {
            a.iter()
                .find(|x| x.0 == k)
                .and_then(|x| x.1.first().copied())
        };
        let (Some(y), Some(z), Some(m), Some(n)) = (get("y"), get("z"), get("m"), get("n")) else {
            return;
        };
        // 止まった・けられた光線の値は使わない
        if get("err").unwrap_or(1.0) != 0.0 || get("vig").unwrap_or(1.0) != 0.0 {
            self.fail(tr(
                "The on-axis ray does not reach the surface before the image",
            ));
            return;
        }
        match combo::paraxial_focus(y, z, m, n) {
            Some(t) => {
                if let Some(v) = self.doc.combo.image_gap() {
                    *v = t;
                }
                self.info(trf(
                    "Image at paraxial focus (last air {} mm)",
                    &[&fmt_num((t * 1e4).round() / 1e4)],
                ));
                if !self.s.auto_build {
                    self.build_combo(self.doc.combo.project());
                }
            }
            None => self.fail(tr("The on-axis ray does not cross the axis (no focus)")),
        }
    }

    fn catalog_ui(&mut self, ui: &mut Ui) {
        if !self.browser.loaded {
            self.refresh_catalog();
        }
        // ダブルクリックで足す玉は、組合せで選んでいる要素の直後へ
        self.doc.insert_after = self.p.combo_view.sel.filter(|&k| k < self.doc.combo.len());
        self.browser.lens_hide.clone_from(&self.s.lens_hide);
        let n_old = self.doc.combo.picks.len();
        let act = self.browser.ui(
            ui,
            &mut self.doc,
            self.s.glass_dirs.len(),
            self.s.lens_dirs.len(),
        );
        self.s.lens_hide.clone_from(&self.browser.lens_hide);
        if let Some(k) = self.doc.added.take() {
            if self.loc.file.is_some() && !self.is_combo() {
                // 開いた系へ足すときは、組合せの末尾に足したときの像面の合わせ直し（GUX-10）は要らない。立てたままだと
                // 組合せにした後に開いた系の像面までの距離を書き換える
                self.doc.refocus = false;
                // 組合せでない系を開いているときは開いた系に足す: レンズデータで選んでいる面の後（空気の間隔なら）、
                // 無ければ系の後ろ。以前は前の組合せが残っていると、足した玉は前の組合せに入り、開いた系が前の組合せの
                // 系に置き換わっていた
                if let Some(p) = self.doc.combo.el(k).cloned() {
                    let after = self
                        .p
                        .ld
                        .cur
                        .map(|(j, _)| j)
                        .filter(|&j| open_gap_ok(&self.sys.ld_rows, j));
                    self.insert_into_open(after, p, n_old, move |c, yes| {
                        if yes {
                            c.doc.combo.remove_chain(k);
                        } else {
                            // 置き換えない: 前の組合せに足したまま、開いた系は作り直さない
                            c.loc.built_combo = Some(c.doc.combo.clone());
                            c.p.combo_view.select(k);
                            c.info(tr(
                                "Added to the combination (the open system is unchanged; build it with \"▶ Build a system from this combination\")",
                            ));
                        }
                    });
                }
            } else {
                self.p.combo_view.select(k);
            }
        }
        self.lens_preview();
        match act {
            Some(catalog::Action::Refresh) => self.refresh_catalog(),
            Some(catalog::Action::Settings) => self.folders_open = true,
            Some(catalog::Action::SysSettings) => self.cmds.push(Cmd::FocusKind(Kind::SysSettings)),
            Some(catalog::Action::New) => self.cmds.push(Cmd::New),
            Some(catalog::Action::Build(p)) => self.build_combo(p),
            None => {}
        }
    }

    /// カタログで選んだ玉の断面を下書きで作る（選び直したら作り直す）
    fn lens_preview(&mut self) {
        let b = &self.browser;
        let Some(i) = b.lens_sel.filter(|&i| i < b.lenses.len()) else {
            return;
        };
        if b.preview.as_ref().is_some_and(|p| p.0 == i) || self.preview_req == Some(i) {
            return;
        }
        if self.preview.busy() {
            return;
        }
        let r = &b.lenses[i];
        if !r.placeable() {
            self.browser.preview = Some((i, combo::LayoutData::default()));
            return;
        }
        let project = catalog::preview_project(r);
        let path = doc_loc::work_dir().join("catalog-preview.json");
        let ok = lensbench_core::vfs::create_dir_all(path.parent().unwrap_or(&path))
            .and_then(|_| lensbench_core::vfs::write(&path, project.to_string()));
        if ok.is_err() {
            return;
        }
        self.preview_req = Some(i);
        self.preview.send(
            Tag::LensPreview(i),
            "preview",
            json!({ "load": self.load_params(&path), "layout": { "rays": 5 } }),
        );
    }

    /// 系の要約（大きな数）とパラメータ
    fn system_ui(&mut self, ui: &mut Ui) {
        let weak = theme::weak_text(ui.visuals());
        let Some(sum) = self.sys.summary.clone() else {
            ui.add_space(8.0);
            ui.label(tr(
                "Open a system file (.zmx or project .json), or drop it onto the window.",
            ));
            ui.label(tr("To combine catalog lenses, go to the Catalog panel."));
            if ui.button(tr("Open… (Ctrl+O)")).clicked() {
                self.pick_and_open();
            }
            if !self.s.recent.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(tr("Recent files")).small().color(weak));
                for f in self.s.recent.clone().iter().take(6) {
                    let name = Path::new(f)
                        .file_name()
                        .map(|x| x.to_string_lossy().into_owned())
                        .unwrap_or_else(|| f.clone());
                    if ui.link(name).on_hover_text(f).clicked() {
                        self.open(Path::new(f));
                    }
                }
            }
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            let name = sum["name"].as_str().unwrap_or_default();
            if !name.is_empty() {
                ui.label(RichText::new(name).strong());
            }
            ui.add_space(4.0);
            let card = |ui: &mut Ui, label: &str, v: Option<f64>, unit: &str, digits: f64| {
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(label).small().color(weak));
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(
                                    v.map(|x| fmt_num((x * digits).round() / digits))
                                        .unwrap_or("—".into()),
                                )
                                .size(19.0)
                                .monospace()
                                .strong(),
                            );
                            if !unit.is_empty() {
                                ui.label(RichText::new(unit).small().color(weak));
                            }
                        });
                    });
            };
            for row in [
                [
                    (tr("Focal length"), self.info_num("effl"), "mm", 1e3),
                    (tr("Working F/#"), self.info_num("wfno"), "", 1e3),
                ],
                [
                    (tr("Image space NA"), self.info_num("image_na"), "", 1e4),
                    (tr("Total track"), self.info_num("total_track"), "mm", 1e3),
                ],
            ] {
                ui.columns(2, |cols| {
                    for (ui, (l, v, u, d)) in cols.iter_mut().zip(row) {
                        card(ui, l, v, u, d);
                    }
                });
            }
            ui.add_space(4.0);
            egui::Grid::new("summary")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    // 値は折り返す（視野が多くてもパネルの右で切れない、GUX-14）
                    let mut row = |k: &str, v: RichText| {
                        ui.label(RichText::new(k).color(weak));
                        ui.add(egui::Label::new(v).wrap());
                        ui.end_row();
                    };
                    let n = |x: Option<f64>| {
                        x.map(|x| fmt_num((x * 1e4).round() / 1e4))
                            .unwrap_or("—".into())
                    };
                    row(
                        tr("Entrance pupil diameter [mm]"),
                        RichText::new(n(self.info_num("epd"))),
                    );
                    row(
                        tr("Paraxial image height [mm]"),
                        RichText::new(n(self.info_num("paraxial_image_height"))),
                    );
                    row(
                        tr("Number of surfaces"),
                        RichText::new(format!("{}", sum["surfaces"])),
                    );
                    let fields: Vec<String> = sum["fields"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .map(|f| {
                                    format!(
                                        "({}, {})",
                                        fmt_num(f[0].as_f64().unwrap_or(0.0)),
                                        fmt_num(f[1].as_f64().unwrap_or(0.0))
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    row(tr("Fields"), RichText::new(fields.join(" ")));
                });
            // 波長（色付き）
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("Wavelengths [µm]")).color(weak));
                let p = self.primary();
                for (i, w) in self.waves_um().iter().enumerate() {
                    let t = RichText::new(fmt_num(*w)).color(theme::wave_color(*w));
                    ui.label(if i + 1 == p {
                        t.strong().underline()
                    } else {
                        t
                    });
                }
            });
            ui.separator();
            ui.label(RichText::new(tr("Parameters")).small().strong().color(weak));
            ui.label(
                RichText::new(tr(
                    "Changing a value recomputes the open analyses right away (↑↓ keys also work)",
                ))
                .small()
                .color(weak),
            );
            let mut remove = None;
            for (i, p) in self.s.params.clone().iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(p).monospace());
                    match self.sys.param_vals.get(p).copied() {
                        Some(mut x) => {
                            let step = (x.abs() * 1e-3).max(1e-3);
                            let r =
                                ui.add(egui::DragValue::new(&mut x).speed(step).max_decimals(8));
                            if r.changed() {
                                self.sys.param_vals.insert(p.clone(), x);
                                self.pending_set.insert(p.clone(), x);
                            }
                        }
                        None => {
                            ui.weak("—");
                        }
                    }
                    if ui.small_button("✖").on_hover_text(tr("Remove")).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                let p = self.s.params.remove(i);
                self.sys.param_vals.remove(&p);
            }
            ui.horizontal(|ui| {
                let sys = self.param_sys();
                let (_, enter) = crate::param_pick::edit_pick(
                    ui,
                    &mut self.new_param,
                    120.0,
                    "3.t · STOP.r · G1.shift",
                    &sys,
                    crate::param_pick::Set::Any,
                );
                if (ui.button(tr("+ Add parameter")).clicked() || enter)
                    && !self.new_param.trim().is_empty()
                {
                    let p = self.new_param.trim().to_string();
                    self.worker.send(Tag::Get, "get", json!({ "param": p }));
                    self.new_param.clear();
                }
            });
            ui.label(
                RichText::new(tr(
                    "Names are the same as in Python and serve (3.t, STOP.r, G1.shift, …)",
                ))
                .small()
                .color(weak),
            );
            if ui
                .button(tr("Reload (discard changed values; Ctrl+Z to undo)"))
                .clicked()
            {
                self.reload();
            }
        });
    }

    /// 解析の引数の欄に並べる項目（視野・波長を上で選ぶ解析はそれを除く。図に要る引数は図のときは GUI が足すので除く）
    fn shown_specs(&self, cfg: &AnaCfg) -> Vec<argform::ArgSpec> {
        let object = crate::ext::get(&cfg.cmd).and_then(|a| a.object);
        let sel = layout::takes_sel(&cfg.cmd);
        let fig = cfg.view == View::Figure;
        argform::analysis_specs(&cfg.cmd)
            .into_iter()
            // 結果の書き出し（save）は「JSON をコピー」で足りるので出さない
            // 設定のオブジェクトの引数（とそのファイル）は下の欄（JSON を読む）で
            .filter(|s| s.name != "save" && !object.is_some_and(|o| o.covers(&s.name)))
            .filter(|s| !(sel && (s.name == "field" || s.name == "wave")))
            .filter(|s| {
                !(fig
                    && matches!(
                        (cfg.cmd.as_str(), s.name.as_str()),
                        ("wavefront", "map") | ("interference" | "pop", "grid")
                    ))
            })
            .map(|mut s| {
                // GUI が既定で足す周波数は欄の薄い字に
                if matches!(cfg.cmd.as_str(), "mtf-field" | "mtf-focus") && s.name == "freqs" {
                    s.default = json!(figures::default_freqs(self.cutoff()));
                    s.required = false;
                }
                s
            })
            .collect()
    }

    /// 上の行の「⚙ 設定」（開閉と変えた数）と「すべて既定に」。計算し直すなら true
    fn args_head(&self, ui: &mut Ui, cfg: &mut AnaCfg) -> bool {
        let shown = self.shown_specs(cfg);
        let set = shown
            .iter()
            .filter(|s| cfg.args.contains_key(&s.name))
            .count()
            + cfg.sensor.len()
            + usize::from(!cfg.extra.trim().is_empty());
        // 変えた数はボタンの中の小さい印にして幅を変えない（押そうとした欄が動かない、GUX-23）
        let head = if set > 0 {
            format!("{} ●{set}", tr("⚙ Settings"))
        } else {
            tr("⚙ Settings").to_string()
        };
        let mut rerun = false;
        let font = egui::TextStyle::Button.resolve(ui.style());
        let w = ui.fonts(|f| {
            f.layout_no_wrap(format!("{} ●99", tr("⚙ Settings")), font, Color32::WHITE)
                .size()
                .x
        }) + 2.0 * ui.spacing().button_padding.x;
        let tip = if set > 0 {
            trf("Open / close the analysis settings ({} changed)", &[&set])
        } else {
            tr("Show or hide the analysis settings").to_string()
        };
        if ui
            .add_sized(
                [w, ui.spacing().interact_size.y],
                egui::SelectableLabel::new(cfg.args_open, head),
            )
            .on_hover_text(tip)
            .clicked()
        {
            cfg.args_open = !cfg.args_open;
        }
        if ui
            .add_enabled(set > 0, egui::Button::new("↺").small())
            .on_hover_text(tr("Reset all"))
            .clicked()
        {
            cfg.args.clear();
            cfg.sensor.clear();
            cfg.extra.clear();
            rerun = true;
        }
        rerun
    }

    /// 解析の設定の欄、拡張の解析なら設定のオブジェクトの欄、詳細（追加の引数の JSON）。計算し直すなら true
    fn args_body(&mut self, ui: &mut Ui, id: u64, cfg: &mut AnaCfg) -> bool {
        let mut rerun = false;
        let specs = self.shown_specs(cfg);
        let shown: Vec<&argform::ArgSpec> = specs.iter().collect();
        let object = crate::ext::get(&cfg.cmd).and_then(|a| a.object);
        let weak = theme::weak_text(ui.visuals());
        if shown.is_empty() && object.is_none() {
            ui.weak(tr("This analysis has no settings"));
        } else if argform::form(ui, ("a", id), &shown, &mut cfg.args) {
            rerun = true;
        }
        if let Some(o) = object {
            if self.object_ui(ui, id, cfg, &o) {
                rerun = true;
            }
        }
        let extra_set = !cfg.extra.trim().is_empty();
        egui::CollapsingHeader::new(RichText::new(tr("Advanced (extra arguments as JSON)")).small().color(weak))
            .id_salt(("args-extra", id))
            .default_open(extra_set)
            .show(ui, |ui| {
                let r = ui
                    .add(
                        egui::TextEdit::singleline(&mut cfg.extra)
                            .hint_text("{\"freqs\": [10, 20]}")
                            .desired_width(f32::INFINITY),
                    )
                    .on_hover_text(tr(
                        "The CLI arguments as JSON (_ in keys becomes -; see analyze in docs/serve.md). Overrides the fields",
                    ));
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    rerun = true;
                }
            });
        rerun
    }

    /// 設定のオブジェクトの欄（拡張の解析の [`crate::ext::ObjectArg`]: センサの設定など）。JSON に保存・読込できる
    fn object_ui(
        &mut self,
        ui: &mut Ui,
        id: u64,
        cfg: &mut AnaCfg,
        o: &crate::ext::ObjectArg,
    ) -> bool {
        let specs = argform::object_specs(o);
        let mut changed = false;
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(tr(o.title));
            let start = ui
                .small_button(tr("Load JSON…"))
                .on_hover_text(tr("JSON with the same items as the fields"))
                .clicked();
            // パネルの番号で区別する（ui.id() は兄弟の Ui で同じになる）
            let key = format!("object-{}-{id}", o.arg);
            {
                if let Some(p) = crate::files::open(&key, start, || {
                    crate::files::FileDialog::new().add_filter("JSON", &["json"])
                }) {
                    match lensbench_core::vfs::read_to_string(&p)
                        .map_err(|e| e.to_string())
                        .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                    {
                        Ok(v) => {
                            let f = argform::flatten(&v);
                            let unknown: Vec<&str> = f
                                .keys()
                                .filter(|k| !specs.iter().any(|s| &s.name == *k))
                                .map(String::as_str)
                                .collect();
                            if unknown.is_empty() {
                                cfg.sensor = f;
                                changed = true;
                            } else {
                                self.fail(trf(
                                    "{}: unknown items {}",
                                    &[&p.display(), &unknown.join(", ")],
                                ));
                            }
                        }
                        Err(e) => self.fail(format!("{}: {e}", p.display())),
                    }
                }
            }
            if ui.small_button(tr("Save JSON…")).clicked() {
                match argform::to_map(&specs, &cfg.sensor) {
                    Ok(m) => {
                        if let Some(p) = crate::files::FileDialog::new()
                            .add_filter("JSON", &["json"])
                            .set_file_name(o.file_name)
                            .save_file()
                        {
                            let t = serde_json::to_string_pretty(&argform::nest(&m))
                                .unwrap_or_default();
                            if let Err(e) = lensbench_core::vfs::write(&p, t + "\n") {
                                self.fail(format!("{}: {e}", p.display()));
                            }
                        }
                    }
                    Err(e) => self.fail(e),
                }
            }
        });
        ui.label(
            RichText::new(tr(o.note))
                .small()
                .color(theme::weak_text(ui.visuals())),
        );
        let mut done = vec![false; specs.len()];
        let mut groups: Vec<(String, &str, Vec<usize>)> = Vec::new();
        for (prefix, title) in o.groups {
            let idx: Vec<usize> = (0..specs.len())
                .filter(|&k| !done[k] && specs[k].name.starts_with(prefix))
                .collect();
            for &k in &idx {
                done[k] = true;
            }
            groups.push((prefix.to_string(), tr(title), idx));
        }
        let rest: Vec<usize> = (0..specs.len()).filter(|&k| !done[k]).collect();
        groups.push((String::new(), tr("Other"), rest));
        for (gi, (prefix, title, idx)) in groups.iter().enumerate() {
            if idx.is_empty() {
                continue;
            }
            let group: Vec<&argform::ArgSpec> = idx.iter().map(|&k| &specs[k]).collect();
            let n = group
                .iter()
                .filter(|s| cfg.sensor.contains_key(&s.name))
                .count();
            let title = if n > 0 {
                format!("{title} ({n})")
            } else {
                title.to_string()
            };
            egui::CollapsingHeader::new(title)
                .id_salt(("object", id, prefix.as_str()))
                .default_open(gi == 0)
                .show(ui, |ui| {
                    if argform::form(ui, ("s", id, prefix.as_str()), &group, &mut cfg.sensor) {
                        changed = true;
                    }
                });
        }
        changed
    }

    /// 解析の種類の選択: 群に分けた一覧と、打って絞り込む欄（Enter で最初の候補）
    fn analysis_picker(ui: &mut Ui, id: u64, cfg: &mut AnaCfg) {
        let fkey = egui::Id::new(("ana-filter", id));
        let r = egui::ComboBox::from_id_salt(("cmd", id))
            .selected_text(layout::analysis_label(&cfg.cmd))
            .height(480.0)
            .show_ui(ui, |ui| {
                let mut q: String = ui.data(|d| d.get_temp(fkey)).unwrap_or_default();
                let te = ui.add(
                    egui::TextEdit::singleline(&mut q)
                        .hint_text(tr("Filter…"))
                        .desired_width(200.0),
                );
                // 開いたときに絞り込みの欄へ
                if !ui.data(|d| d.get_temp::<bool>(fkey.with("f")).unwrap_or(false)) {
                    te.request_focus();
                    ui.data_mut(|d| d.insert_temp(fkey.with("f"), true));
                }
                let ql = q.to_lowercase();
                let hit = |c: &str, l: &'static str| {
                    ql.is_empty()
                        || c.to_lowercase().contains(&ql)
                        || tr(l).to_lowercase().contains(&ql)
                        || layout::analysis_help(c)
                            .is_some_and(|h| tr(h).to_lowercase().contains(&ql))
                };
                let mut list: Vec<(&str, &str)> = layout::analyses()
                    .into_iter()
                    .filter(|(c, l, _)| hit(c, l))
                    .map(|(c, l, _)| (c, l))
                    .collect();
                list.sort_by_key(|(c, _)| {
                    let g = layout::analysis_group(c);
                    layout::ANALYSIS_GROUPS.iter().position(|x| *x == g)
                });
                if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    if let Some((c, _)) = list.first() {
                        cfg.cmd = c.to_string();
                        ui.memory_mut(|m| m.close_popup());
                    }
                }
                ui.data_mut(|d| d.insert_temp(fkey, q.clone()));
                let weak = theme::weak_text(ui.visuals());
                let mut last = "";
                for (c, l) in &list {
                    let g = layout::analysis_group(c);
                    if g != last {
                        ui.label(RichText::new(tr(g)).small().color(weak));
                        last = g;
                    }
                    let r = ui.selectable_value(&mut cfg.cmd, c.to_string(), tr(l));
                    if let Some(h) = layout::analysis_help(c) {
                        r.on_hover_text(tr(h));
                    }
                }
                if list.is_empty() {
                    ui.weak(tr("No matching analysis"));
                }
            });
        let r = {
            // 閉じたら絞り込みを忘れる
            if r.inner.is_none() {
                ui.data_mut(|d| {
                    d.remove::<String>(fkey);
                    d.remove::<bool>(fkey.with("f"));
                });
            }
            r.response
        };
        if let Some(h) = layout::analysis_help(&cfg.cmd) {
            r.on_hover_text(tr(h));
        }
    }

    fn analysis_ui(&mut self, ui: &mut Ui, id: u64, cfg: &mut AnaCfg) {
        if self.sys.summary.is_none() {
            ui.weak(tr("Open a system to analyze it."));
            return;
        }
        let nf = self.n_fields();
        let waves = self.waves_um();
        let fields: Vec<String> = self
            .sys
            .summary
            .as_ref()
            .and_then(|s| s["fields"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|f| {
                format!(
                    "({}, {})",
                    fmt_num(f[0].as_f64().unwrap_or(0.0)),
                    fmt_num(f[1].as_f64().unwrap_or(0.0))
                )
            })
            .collect();
        let before = cfg.clone();
        let mut rerun = false;
        ui.horizontal_wrapped(|ui| {
            Self::analysis_picker(ui, id, cfg);
            ui.add_enabled_ui(layout::takes_sel(&cfg.cmd), |ui| {
                let fname = |i: usize| match fields.get(i - 1) {
                    Some(f) => trf("Field {} {}", &[&i, f]),
                    None => trf("Field {}", &[&i]),
                };
                egui::ComboBox::from_id_salt(("field", id))
                    .selected_text(if cfg.field == 0 {
                        tr("All fields").to_string()
                    } else {
                        fname(cfg.field)
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut cfg.field, 0, tr("All fields"));
                        for i in 1..=nf {
                            ui.selectable_value(&mut cfg.field, i, fname(i));
                        }
                    });
                let wname = |i: usize| match waves.get(i - 1) {
                    Some(w) => trf("Wave {} {} µm", &[&i, &fmt_num(*w)]),
                    None => trf("Wave {}", &[&i]),
                };
                egui::ComboBox::from_id_salt(("wave", id))
                    .selected_text(if cfg.wave == 0 {
                        tr("Polychromatic").to_string()
                    } else {
                        wname(cfg.wave)
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut cfg.wave, 0, tr("Polychromatic"));
                        for i in 1..=waves.len() {
                            let w = waves[i - 1];
                            ui.selectable_value(
                                &mut cfg.wave,
                                i,
                                RichText::new(wname(i)).color(theme::wave_color(w)),
                            );
                        }
                    });
            });
            ui.checkbox(&mut cfg.live, tr("Auto"))
                .on_hover_text(tr("Recompute when the system changes"));
            if ui
                .button(tr("▶ Compute"))
                .on_hover_text(tr(
                    "Compute this analysis with the current settings (F5 computes all open analyses)",
                ))
                .clicked()
            {
                rerun = true;
            }
            rerun |= self.args_head(ui, cfg);
        });
        ui.separator();
        if cfg.args_open {
            let rect = ui.available_rect_before_wrap();
            if split::side_by_side(rect.width(), rect.height()) {
                // 横に広いパネル: 設定を左、結果を右に（境はドラッグ）
                let (mut a, mut b) = split::two(ui, ("ana-args", id), (0.4, 0.35), 220.0);
                egui::ScrollArea::vertical()
                    .id_salt(("args", id))
                    .auto_shrink([false, false])
                    .show(&mut a, |ui| rerun |= self.args_body(ui, id, cfg));
                self.result_ui(&mut b, id, cfg);
            } else {
                egui::ScrollArea::vertical()
                    .id_salt(("args", id))
                    .max_height(ui.available_height() * 0.45)
                    .show(ui, |ui| rerun |= self.args_body(ui, id, cfg));
                ui.separator();
                self.result_ui(ui, id, cfg);
            }
        } else {
            self.result_ui(ui, id, cfg);
        }
        // 設定を変えたら計算し直す（追加の引数は Enter か計算ボタン）
        if rerun
            || cfg.cmd != before.cmd
            || cfg.field != before.field
            || cfg.wave != before.wave
            || (cfg.view != before.view
                && (matches!(&*cfg.cmd, "spot" | "wavefront" | "interference" | "pop")
                    || crate::ext::get(&cfg.cmd).is_some_and(|a| a.figure_params.is_some())))
        {
            self.run(id, cfg);
        }
    }

    fn folders_window(&mut self, ctx: &egui::Context) {
        self.ask_folders();
        let mut open = self.folders_open;
        let mut changed = false;
        egui::Window::new(tr("Catalog folders"))
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                changed |= dir_list(
                    ui,
                    tr("Glass catalogs (folders of .AGF files)"),
                    &mut self.s.glass_dirs,
                    &self.folders.missing,
                );
                ui.separator();
                changed |= dir_list(
                    ui,
                    tr("Lens catalogs (folders of .zmx files; the folder name is the vendor, the file name the part number)"),
                    &mut self.s.lens_dirs,
                    &self.folders.missing,
                );
                ui.separator();
                ui.label(
                    RichText::new(tr("Used when opening systems and reading catalogs. Changing them reloads the system.")).small().weak(),
                );
                ui.separator();
                #[cfg(not(target_arch = "wasm32"))]
                {
                    crate::cad_store::cad_store_ui(ui, &mut self.s.cad_store);
                    ui.separator();
                }
                // 設定の保存先
                let at = crate::portable::location()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| tr("(not saved)").to_string());
                let kind = if crate::portable::base().is_some() {
                    tr("Settings file (portable; paths inside its folder are saved as relative)")
                } else {
                    tr("Settings file (OS location; put lensbench.portable next to the exe to make it portable)")
                };
                ui.label(RichText::new(format!("{kind}: {at}")).small().weak());
            });
        self.folders_open = open;
        if changed {
            self.refresh_catalog();
            self.reload();
        }
    }

    /// カタログに無いガラスで開けなかったときの案内（GUX-01）
    fn missing_glass_window(&mut self, ctx: &egui::Context) {
        let Some(m) = self.missing_glass.clone() else {
            return;
        };
        if m.model.is_some() {
            return;
        }
        let mut open = true;
        let mut act = None;
        egui::Window::new(tr("Glass not found"))
            .open(&mut open)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(trf(
                    "Cannot open {}: glass {} is not in the catalogs.",
                    &[
                        &m.file
                            .file_name()
                            .map(|x| x.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        &m.names.join(crate::i18n::tr(", ")),
                    ],
                ));
                ui.label(
                    RichText::new(tr(
                        "Add the folder of the maker's glass catalog (.AGF), or open with model glasses from the nd and νd written in the .zmx.",
                    ))
                    .small(),
                );
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr("Add catalog folders…")).clicked() {
                        act = Some(0);
                    }
                    let zmx = m
                        .file
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("zmx"));
                    if ui
                        .add_enabled(zmx, egui::Button::new(tr("Open with model glasses (nd, νd)")))
                        .on_hover_text(tr(
                            "Opens a copy in a temporary folder (the original file is unchanged; use Save As)",
                        ))
                        .on_disabled_hover_text(tr("Only .zmx files can be substituted"))
                        .clicked()
                    {
                        act = Some(1);
                    }
                });
            });
        match act {
            Some(0) => {
                self.folders_open = true;
                self.missing_glass = None;
            }
            Some(1) => self.open_model_glass(),
            _ if !open => self.missing_glass = None,
            _ => {}
        }
    }

    /// カタログに無いガラスをモデルガラスにした写しを一時フォルダに作って開く
    fn open_model_glass(&mut self) {
        let Some(m) = self.missing_glass.as_mut() else {
            return;
        };
        m.tries += 1;
        let src = m.model.clone().unwrap_or_else(|| m.file.clone());
        let text = match lensbench_core::vfs::read(&src) {
            Ok(b) => lensbench_core::glass::decode_text(&b),
            Err(e) => {
                self.fail(format!("{}: {e}", src.display()));
                self.missing_glass = None;
                return;
            }
        };
        let (out, done) = model_glass_zmx(&text, &m.names);
        if done.is_empty() {
            let text = trf(
                "Glass {} has no nd/νd in the file, so it cannot be substituted",
                &[&m.names.join(crate::i18n::tr(", "))],
            );
            self.fail(text);
            self.missing_glass = None;
            return;
        }
        let dir = if cfg!(target_arch = "wasm32") {
            PathBuf::from("/tmp/model-glass")
        } else {
            std::env::temp_dir().join("lensbench-model-glass")
        };
        let _ = lensbench_core::vfs::create_dir_all(&dir);
        let name = m
            .file
            .file_name()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_else(|| "system.zmx".into());
        let dst = dir.join(name);
        if let Err(e) = lensbench_core::vfs::write(&dst, out.as_bytes()) {
            self.fail(format!("{}: {e}", dst.display()));
            self.missing_glass = None;
            return;
        }
        m.model = Some(dst.clone());
        (self.loc.doc_path, self.loc.doc_from) = (None, None);
        self.open(&dst);
    }

    /// ユーザーカタログに保存する窓。保存は読み込んだ組合せの系から（解析と同じ worker）、一覧・変更は下書きの worker
    fn cat_save_window(&mut self, ctx: &egui::Context) {
        let ready = self.is_combo() && self.loc.built_combo.as_ref() == Some(&self.doc.combo);
        let req = self
            .p
            .cat_save
            .window(ctx, &self.doc.combo, &self.browser.lenses, ready);
        if let Some((method, p)) = req {
            if method == "catalog_add" {
                self.worker.send(Tag::UserCatalog, method, p);
            } else {
                self.preview.send(Tag::UserCatalog, method, p);
            }
        }
    }

    /// 未完のカタログ品を補う窓（品を足そうとしたら開く）。保存は下書きの worker
    fn fill_window(&mut self, ctx: &egui::Context) {
        if let Some(r) = self.browser.fill_req.take() {
            self.auto_fill.forget(&r.part);
            self.p.fill.start(r);
        }
        if let Some(e) = self.auto_fill.take_timing_error() {
            self.fail(e);
        }
        // 補う窓が開いている間は裏で読まない
        if !self.p.fill.open {
            let sel = self
                .browser
                .lens_sel
                .and_then(|i| self.browser.lenses.get(i))
                .cloned();
            match self.auto_fill.tick(ctx, sel.as_ref()) {
                Some(crate::fill::AutoOut::Save(p)) => {
                    self.preview.send(Tag::CatalogFill, "catalog_fill", p);
                }
                #[cfg(all(windows, feature = "webview"))]
                Some(crate::fill::AutoOut::Show(b)) => {
                    let (r, child, text) = *b;
                    self.p.fill.adopt(r, child, text);
                }
                None => {}
            }
        }
        if !self.p.fill.open {
            return;
        }
        if let Some(p) = self.p.fill.window(ctx) {
            self.preview.send(Tag::CatalogFill, "catalog_fill", p);
        }
    }
}

/// フォルダの並びを編集する。変えたら true
/// 組合せの系で、面の表から変えられない値の理由（開いた系は None）
/// 開いた系の最上位のブロック（玉・群）を、系の取り込みの群（面の範囲と名前）に
pub(crate) fn open_groups(blocks: &[lens_data::Block]) -> Vec<combo::ZGroup> {
    blocks
        .iter()
        .filter(|b| b.parent.is_none() && !b.name.trim().is_empty())
        .filter_map(|b| {
            let from = *b.surfs.iter().min()?;
            let to = *b.surfs.iter().max()?;
            (from >= 1).then(|| combo::ZGroup {
                from,
                to,
                name: b.name.clone(),
                ..Default::default()
            })
        })
        .collect()
}

/// 開いた系の面 `after` の後が、要素を差し込める空気の間隔か（物体面・像面の後、座標ブレーク、ガラスの中、鏡の後は不可）
pub(crate) fn open_gap_ok(rows: &[lens_data::Row], after: usize) -> bool {
    let Some(r) = rows.get(after) else {
        return false;
    };
    after >= 1
        && after + 1 < rows.len()
        && !r.is_cb()
        && (r.material.trim().is_empty() || r.material.eq_ignore_ascii_case("AIR"))
        && r.t.is_finite()
        && r.t > 0.0
}

/// 開いた系の取り込み `z`（面 1〜像面の手前）に要素 `p` を差し込んだ並び。面 `after` の後の空気の間隔に置くときは、
/// 取り込みを面 1〜after と after+1〜 に分け、間隔の空気を半分ずつ前後に分ける。差し込めない間隔・None は系の後ろ
pub(crate) fn split_open(
    z: combo::Pick,
    rows: &[lens_data::Row],
    after: Option<usize>,
    mut p: combo::Pick,
) -> Vec<combo::Pick> {
    let img = rows.len().saturating_sub(1);
    let Some(a) = after.filter(|&a| open_gap_ok(rows, a)) else {
        // 系の後ろ: 取り込みは像面の手前の面の厚さまで含む。足した要素の後の空気はその初めの値
        return vec![z, p];
    };
    let half = rows[a].t / 2.0;
    let mut z1 = z.clone();
    z1.range = [1, a];
    // 群は分けた範囲に収まるものだけ（間隔はブロックの外なので、跨ぐ群は無い）
    z1.zgroups.retain(|g| g.to <= a);
    // 取り込みの面 a（座標ブレークを除いて数えた番号）の厚さを半分に
    let rel = (1..a).filter(|&i| !rows[i].is_cb()).count();
    z1.set_edit(rel, "t", serde_json::json!(half));
    p.air = half;
    if a + 1 >= img {
        return vec![z1, p];
    }
    let mut z2 = z;
    z2.range = [a + 1, 0];
    z2.zgroups.retain(|g| g.from > a);
    vec![z1, p, z2]
}

/// missing は見つからないフォルダ（下書きの Worker が調べた分、[`Core::ask_folders`]）
fn dir_list(ui: &mut Ui, title: &str, dirs: &mut Vec<String>, missing: &[String]) -> bool {
    let mut changed = false;
    ui.label(RichText::new(title).strong());
    let mut remove = None;
    for (i, d) in dirs.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.button("✖").on_hover_text(tr("Remove")).clicked() {
                remove = Some(i);
            }
            ui.label(if !missing.contains(d) {
                RichText::new(d)
            } else {
                RichText::new(trf("{} (not found)", &[d])).color(theme::error(ui.visuals()))
            });
        });
    }
    if let Some(i) = remove {
        dirs.remove(i);
        changed = true;
    }
    let mut start = false;
    ui.horizontal(|ui| {
        start = ui.button(tr("➕ Add folder…")).clicked();
        // パスを打つ・貼る（GUX-01）
        let id = egui::Id::new(("dir-typed", title));
        let mut t: String = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
        let r = ui.add(
            egui::TextEdit::singleline(&mut t)
                .hint_text(tr("Or paste a path and press Enter"))
                .desired_width(260.0),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter && !t.trim().is_empty() {
            let p = t.trim().trim_matches('"').to_string();
            if !dirs.contains(&p) {
                dirs.push(p);
                changed = true;
            }
            t.clear();
        }
        ui.data_mut(|d| d.insert_temp(id, t));
    });
    {
        if let Some(p) = crate::files::open_folder(&format!("folder-{title}"), start, || {
            crate::files::FileDialog::new()
        }) {
            let p = p.display().to_string();
            if !dirs.contains(&p) {
                dirs.push(p);
                changed = true;
            }
        }
    }
    changed
}

/// egui_dock にパネルの中身を描かせる
struct Viewer<'a> {
    c: &'a mut Core,
    to_window: Vec<u64>,
    added: Vec<(SurfaceIndex, NodeIndex, Kind)>,
    /// このフレームでタブを引きずり始めたパネル
    dragging: Option<u64>,
}

impl TabViewer for Viewer<'_> {
    type Tab = Pane;

    fn on_tab_button(&mut self, tab: &mut Pane, response: &egui::Response) {
        // 引きずり始めると egui_dock が別の応答に差し替えるので、押した所で覚える
        if response.is_pointer_button_down_on() {
            self.dragging = Some(tab.id);
        }
    }

    fn title(&mut self, tab: &mut Pane) -> egui::WidgetText {
        tab.title().into()
    }

    fn id(&mut self, tab: &mut Pane) -> egui::Id {
        egui::Id::new(("pane", tab.id))
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Pane) {
        self.c.pane_ui(ui, tab);
    }

    fn context_menu(&mut self, ui: &mut Ui, tab: &mut Pane, _s: SurfaceIndex, _n: NodeIndex) {
        let label = menus::detach_label(cfg!(target_arch = "wasm32"));
        if ui.button(label).clicked() {
            self.to_window.push(tab.id);
            ui.close_menu();
        }
    }

    fn on_close(&mut self, tab: &mut Pane) -> bool {
        self.c.results.remove(&tab.id);
        true
    }

    fn add_popup(&mut self, ui: &mut Ui, surface: SurfaceIndex, node: NodeIndex) {
        ui.set_min_width(160.0);
        for k in add_kinds() {
            let label = Pane {
                id: 0,
                kind: k.clone(),
            }
            .title();
            if ui.button(label).clicked() {
                self.added.push((surface, node, k));
                ui.close_menu();
            }
        }
    }

    fn scroll_bars(&self, _tab: &Pane) -> [bool; 2] {
        [false, false]
    }
}

/// 版（git のハッシュ付き。+ は手元で変えた所あり）
fn version() -> String {
    let h = env!("LENSBENCH_GIT_HASH");
    if h.is_empty() {
        format!("lensbench {}", lensbench_core::VERSION)
    } else {
        format!("lensbench {} ({h})", lensbench_core::VERSION)
    }
}

/// 足せるパネル
fn add_kinds() -> Vec<Kind> {
    let mut v: Vec<Kind> = layout::TOOLS.iter().map(|t| t.kind.clone()).collect();
    v.extend(
        layout::analyses()
            .iter()
            .map(|a| Kind::Analysis(AnaCfg::new(a.0))),
    );
    v
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Args) -> App {
        let font = crate::fonts::install(&cc.egui_ctx);
        crate::files::set_ctx(&cc.egui_ctx);
        // 通し確認では保存した設定を使わない（引数のフォルダだけ）
        let mut s: Settings = if args.tour.is_some() {
            Settings::default()
        } else {
            cc.storage
                .and_then(|st| eframe::get_value(st, eframe::APP_KEY))
                .unwrap_or_default()
        };
        if args.tour.is_some() {
            crate::tour::prepare(&args, &mut s);
        } else {
            s.resolve_paths();
            // 初めてのポータブルで OS の場所の設定を取り込んだら、硝材・レンズのフォルダは exe の隣の catalogs/ に
            if matches!(args.notice, Some(crate::portable::Notice::Imported(_))) {
                if let Some(d) = crate::portable::exe_dir() {
                    crate::portable::imported_catalogs(&d, &mut s.glass_dirs, &mut s.lens_dirs);
                }
            }
            // 硝材・レンズのフォルダが無ければ、exe の隣の catalogs/ を既定に（リリースを展開してすぐ見える）
            if s.glass_dirs.is_empty() && s.lens_dirs.is_empty() {
                if let Some(d) = crate::portable::exe_dir() {
                    let (g, l) = crate::portable::default_catalogs(&d);
                    s.glass_dirs = g;
                    s.lens_dirs = l;
                }
            }
        }
        // Web 版: ブラウザに残したフォルダのうち、消えたもの（残したファイルを消した後など）は外す
        #[cfg(target_arch = "wasm32")]
        crate::web_store::prune_dirs(&mut s.glass_dirs, &mut s.lens_dirs);
        for d in &args.glass_dirs {
            if !s.glass_dirs.contains(d) {
                s.glass_dirs.push(d.clone());
            }
        }
        for d in &args.lens_dirs {
            if !s.lens_dirs.contains(d) {
                s.lens_dirs.push(d.clone());
            }
        }
        i18n::set(s.lang);
        theme::apply(&cc.egui_ctx, &s.look);
        s.cad_store.apply();
        // Web 版: 別のタブのパネル（?panel=）は計算の Web Worker を作らない
        #[cfg(target_arch = "wasm32")]
        let panel = crate::web_tabs::panel_of_location();
        #[cfg(target_arch = "wasm32")]
        let (worker, preview) = if panel.is_some() {
            (Worker::idle(), Worker::idle())
        } else {
            let ctx = cc.egui_ctx.clone();
            let ctx2 = cc.egui_ctx.clone();
            (
                Worker::new(None, move || ctx.request_repaint()),
                Worker::preview(move || ctx2.request_repaint()),
            )
        };
        #[cfg(not(target_arch = "wasm32"))]
        let (worker, preview) = {
            let ctx = cc.egui_ctx.clone();
            let ctx2 = cc.egui_ctx.clone();
            (
                Worker::new(None, move || ctx.request_repaint()),
                Worker::preview(move || ctx2.request_repaint()),
            )
        };
        let browser = Browser::default();
        let doc = crate::combo_doc::ComboDoc::new(s.combo.clone());
        // 系のファイルを指定して開いたときは、前の組合せを作り直さない（指定が無ければ前の組合せで系を作る）
        let built_combo = args.open.is_some().then(|| s.combo.clone());
        let lay = s.layout.take().unwrap_or_else(|| Preset::Analysis.build());
        let next_id = lay.max_id();
        let mut app = App {
            c: Core {
                s,
                samples: menus::samples(&args),
                worker,
                preview,
                load_stale: false,
                preview_req: None,
                status: Status {
                    text: args.notice.as_ref().map(|n| n.text()).unwrap_or_default(),
                    error: false,
                },
                status_err: None,
                sys: Default::default(),
                p: Panels::default(),
                results: HashMap::new(),
                new_param: String::new(),
                pending_set: Default::default(),
                rerun: None,
                combo_edit: None,
                combo_hist: Default::default(),
                docs: Default::default(),
                um_open: false,
                ld_glasses: Vec::new(),
                close_ask: false,
                combo_census: None,
                missing_glass: None,
                status_log: Vec::new(),
                last_set: None,
                shot: None,
                fig_rects: HashMap::new(),
                status_log_open: false,
                close_ok: false,
                close_pending: false,
                auto_fill: Default::default(),
                swap_ids: Vec::new(),
                swap_file: None,
                rev: 0,
                param_sys_cache: Default::default(),
                sys_gen: 0,
                browser,
                doc,
                folders_open: false,
                help_open: false,
                save_layout_open: false,
                ask: None,
                drawing_in: None,
                layout_name: String::new(),
                loc: DocLoc {
                    built_combo,
                    ..Default::default()
                },
                _work: doc_loc::WorkDir::new(),
                layout_data: None,
                overlay: Overlay::default(),
                rays_open: false,
                folders: Default::default(),
                combo_sel_prev: None,
                to_combo: None,
                open_insert: None,
                combo_len_prev: usize::MAX,
                font,
                next_id,
                cmds: Vec::new(),
            },
            dock: lay.dock,
            windows: lay.windows,
            palette: Palette::default(),
            win_init: HashMap::new(),
            dragging_tab: None,
            tour: None,
            decorations: None,
            #[cfg(target_arch = "wasm32")]
            tabs: None,
            #[cfg(target_arch = "wasm32")]
            #[cfg(target_arch = "wasm32")]
            viewer: None,
            #[cfg(target_arch = "wasm32")]
            hosts: HashMap::new(),
        };
        // 前に閉じたときの最適化・公差・差し替えの設定（GPN-14）
        let panels = app.c.s.panels.clone();
        app.c.set_panels(&panels);
        // レンズデータの見せ方は表の状態が持ち、設定には保存するときだけ写す（GPN-28）
        app.c.p.ld.tree = app.c.s.ld_tree;
        app.c.p.ld.detail = app.c.s.ld_detail;
        app.c.p.ld.by_surface = app.c.s.ld_by_surface;
        app.apply_dock_translations();
        // Web 版: 別の窓のパネルなら、計算しないで元のタブで描いた画面を受けて描く
        #[cfg(target_arch = "wasm32")]
        if let Some((id, sid)) = panel {
            app.viewer = Some(crate::web_tabs::Viewer::new(&cc.egui_ctx, id, sid));
            return app;
        }
        // Web 版: 別のタブ・窓に出していたパネルはドックに戻す（タブ・窓はクリックしないと開けない）
        #[cfg(target_arch = "wasm32")]
        if args.tour.is_none() {
            app.tabs = Some(crate::web_tabs::Tabs::new(&cc.egui_ctx));
            for w in std::mem::take(&mut app.windows) {
                crate::layout::redock(&mut app.dock, w.pane, w.home.as_ref());
            }
        }
        if let Some(d) = &args.tour {
            let _ = lensbench_core::vfs::create_dir_all(d);
            app.tour = Some(Tour::new(d.clone(), args.tour_timeout));
            if args.open.is_none() {
                app.tour
                    .as_mut()
                    .unwrap()
                    .errors
                    .push("--tour needs a system file".into());
            }
        }
        // Web 版: 前に開いていた、ブラウザに残したファイルがあればそれを開く（見本より先に）
        #[cfg(target_arch = "wasm32")]
        let reopen = app
            .tour
            .is_none()
            .then(|| app.c.s.recent.first().cloned())
            .flatten()
            .filter(|r| {
                crate::web_store::KEEP.iter().any(|k| r.starts_with(k))
                    && lensbench_core::vfs::is_file(r)
            })
            .map(PathBuf::from);
        #[cfg(not(target_arch = "wasm32"))]
        let reopen: Option<PathBuf> = None;
        if let Some(p) = reopen.as_ref().or(args.open.as_ref()) {
            app.c.open_doc(p);
        }
        app
    }

    fn apply_dock_translations(&mut self) {
        let mut t = egui_dock::Translations::english();
        t.tab_context_menu.close_button = tr("Close").into();
        t.tab_context_menu.eject_button = tr("Float").into();
        t.leaf.close_all_button = tr("Close window").into();
        t.leaf.minimize_button = tr("Collapse").into();
        self.dock.translations = t;
    }

    fn set_layout(&mut self, l: Layout) {
        self.c.results.clear();
        self.c.next_id = self.c.next_id.max(l.max_id());
        self.dock = l.dock;
        self.windows = l.windows;
        self.apply_dock_translations();
        self.c.rerun = Some(false);
    }

    fn layout(&self) -> Layout {
        Layout {
            dock: self.dock.clone(),
            windows: self.windows.clone(),
        }
    }

    /// 解析のパネル（id と設定）
    fn analyses(&self) -> Vec<(u64, AnaCfg)> {
        self.dock
            .iter_all_tabs()
            .map(|(_, p)| p)
            .chain(self.windows.iter().map(|w| &w.pane))
            .filter_map(|p| match &p.kind {
                Kind::Analysis(a) => Some((p.id, a.clone())),
                _ => None,
            })
            .collect()
    }

    fn run_all(&mut self, live_only: bool) {
        for (id, cfg) in self.analyses() {
            if !live_only || cfg.live {
                self.c.run(id, &cfg);
            }
        }
    }

    fn add_pane(&mut self, kind: Kind) -> u64 {
        let id = self.c.new_id();
        // いちばん広い葉にタブで足す（フォーカスのある小さな所に入って図が潰れない、GUX-04）
        let big = self
            .dock
            .main_surface()
            .iter()
            .enumerate()
            .filter(|(_, n)| n.is_leaf())
            .filter_map(|(i, n)| n.rect().map(|r| (i, r.area())))
            .filter(|(_, a)| a.is_finite() && *a > 0.0)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| NodeIndex(i));
        if let Some(n) = big {
            self.dock
                .set_focused_node_and_surface((SurfaceIndex::main(), n));
        }
        self.dock.push_to_focused_leaf(Pane { id, kind });
        id
    }

    /// パネルを表に出して選ぶ
    fn focus(&mut self, id: u64) {
        let probe = Pane {
            id,
            kind: Kind::System,
        };
        if let Some((s, n, t)) = self.dock.find_tab(&probe) {
            self.dock.set_active_tab((s, n, t));
            self.dock.set_focused_node_and_surface((s, n));
        }
    }

    /// 次（前）のパネルへ
    fn cycle(&mut self, back: bool) {
        let ids: Vec<u64> = self.dock.iter_all_tabs().map(|(_, p)| p.id).collect();
        if ids.is_empty() {
            return;
        }
        let cur = self
            .dock
            .find_active_focused()
            .map(|(_, p)| p.id)
            .and_then(|id| ids.iter().position(|&x| x == id))
            .unwrap_or(0);
        let k = if back {
            (cur + ids.len() - 1) % ids.len()
        } else {
            (cur + 1) % ids.len()
        };
        self.focus(ids[k]);
    }

    /// タブを本体の窓の外まで引きずって離したら、別の窓に出す（離した所に）
    fn drag_out(&mut self, ctx: &egui::Context, started: Option<u64>) {
        let (pos, screen, inner, down) = ctx.input(|i| {
            (
                i.pointer.latest_pos(),
                i.screen_rect(),
                i.viewport().inner_rect,
                i.pointer.any_down(),
            )
        });
        if let Some(id) = started {
            if self.dragging_tab.map(|t| t.0) != Some(id) {
                self.dragging_tab = pos.map(|p| (id, p));
            }
        }
        if down {
            if let (Some(t), Some(p)) = (self.dragging_tab.as_mut(), pos) {
                t.1 = p;
                if !screen.shrink(4.0).contains(p) {
                    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                }
            }
            return;
        }
        // 引きずり終えた: 最後の位置が本体の外なら
        let Some((id, p)) = self.dragging_tab.take() else {
            return;
        };
        // 外へ出たときの位置が届かない環境もあるので、縁まで来ていれば外とみなす
        if screen.shrink(4.0).contains(p) {
            return;
        }
        let base = inner.map(|o| o.min).unwrap_or(egui::Pos2::ZERO);
        self.detach_at(id, Some([base.x + p.x - 40.0, base.y + p.y - 12.0]));
    }

    fn detach(&mut self, id: u64) {
        self.detach_at(id, None);
    }

    fn detach_at(&mut self, id: u64, pos: Option<[f32; 2]>) {
        let probe = Pane {
            id,
            kind: Kind::System,
        };
        if let Some((s, n, t)) = self.dock.find_tab(&probe) {
            let home = crate::layout::home_of(&self.dock, (s, n, t));
            if let Some(p) = self.dock.remove_tab((s, n, t)) {
                self.windows.push(OsWin {
                    pane: p,
                    pos,
                    size: [720.0, 520.0],
                    home,
                });
            }
        }
    }

    fn save_layout_window(&mut self, ctx: &egui::Context) {
        if !self.c.save_layout_open {
            return;
        }
        let mut open = true;
        let mut done = false;
        egui::Window::new(tr("Save current workspace"))
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(tr("Name (an existing name is overwritten)"));
                let r = ui.text_edit_singleline(&mut self.c.layout_name);
                r.request_focus();
                let enter = ui.input(|i| i.key_pressed(Key::Enter));
                if (ui.button(tr("Save")).clicked() || enter)
                    && !self.c.layout_name.trim().is_empty()
                {
                    let name = self.c.layout_name.trim().to_string();
                    let l = self.layout();
                    if let Some(x) = self.c.s.layouts.iter_mut().find(|(n, _)| *n == name) {
                        x.1 = l;
                    } else {
                        self.c.s.layouts.push((name.clone(), l));
                    }
                    self.c.info(trf("Saved workspace \"{}\"", &[&name]));
                    done = true;
                }
            });
        if done || !open || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.c.save_layout_open = false;
        }
    }

    /// 本体の窓の外へ引きずって離したフロートのパネルを、その場所の別の OS の窓にする
    fn float_out(&mut self, ctx: &egui::Context) {
        let screen = ctx.screen_rect();
        let outer = ctx.input(|i| i.viewport().inner_rect);
        let dragging = ctx.input(|i| i.pointer.any_down());
        let mut out = Vec::new();
        for k in 1..self.dock.surfaces_count() {
            let si = egui_dock::SurfaceIndex(k);
            let Some(ws) = self.dock.get_window_state(si) else {
                continue;
            };
            let r = ws.rect();
            if dragging || ws.dragged() || !r.is_finite() || screen.contains(r.center()) {
                continue;
            }
            out.push((si, r));
        }
        for (si, r) in out.into_iter().rev() {
            let Some(surface) = self.dock.remove_surface(si) else {
                continue;
            };
            let egui_dock::Surface::Window(tree, _) = surface else {
                continue;
            };
            // 本体の窓の中の座標を画面の座標に
            let base = outer.map(|o| o.min).unwrap_or(egui::Pos2::ZERO);
            for (i, p) in tree.tabs().cloned().enumerate() {
                let off = 24.0 * i as f32;
                self.windows.push(OsWin {
                    pane: p,
                    pos: Some([base.x + r.min.x + off, base.y + r.min.y + off]),
                    size: [r.width().max(360.0), r.height().max(280.0)],
                    home: None,
                });
            }
        }
    }

    /// 別の OS の窓に出したパネル。閉じると本体に戻す
    fn os_windows(&mut self, ctx: &egui::Context) {
        let mut wins = std::mem::take(&mut self.windows);
        let mut back = Vec::new();
        for (k, w) in wins.iter_mut().enumerate() {
            // Web 版: 別のタブに出したパネルは描かない
            #[cfg(target_arch = "wasm32")]
            if self.tabs.as_ref().is_some_and(|t| t.is_open(w.pane.id)) {
                continue;
            }
            let vid = egui::ViewportId::from_hash_of(("lensbench-pane", w.pane.id));
            let (isize, ipos) = *self.win_init.entry(w.pane.id).or_insert((w.size, w.pos));
            let mut b = egui::ViewportBuilder::default()
                .with_title(format!("lensbench — {}", w.pane.title()))
                .with_icon(icon())
                .with_inner_size(isize);
            if let Some(p) = ipos {
                b = b.with_position(p);
            }
            let c = &mut self.c;
            let mut ret = false;
            let title = w.pane.title();
            let pid = w.pane.id;
            let size = w.size;
            let (pane, pos, wsize) = (&mut w.pane, &mut w.pos, &mut w.size);
            ctx.show_viewport_immediate(vid, b, |ctx, class| {
                let mut body = |ui: &mut Ui, c: &mut Core, ret: &mut bool| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&title).strong());
                        if ui.small_button(tr("Back to main window")).clicked() {
                            *ret = true;
                        }
                    });
                    ui.separator();
                    c.pane_ui(ui, pane);
                };
                if class == egui::ViewportClass::Embedded {
                    // 別の窓を作れない環境では本体の中の窓
                    let mut open = true;
                    egui::Window::new(title.clone())
                        .id(egui::Id::new(("embedded", pid)))
                        .open(&mut open)
                        .default_size(size)
                        .show(ctx, |ui| body(ui, c, &mut ret));
                    if !open {
                        ret = true;
                    }
                } else {
                    egui::CentralPanel::default().show(ctx, |ui| body(ui, c, &mut ret));
                    let (close, rect) =
                        ctx.input(|i| (i.viewport().close_requested(), i.viewport().outer_rect));
                    if close {
                        ret = true;
                    }
                    if let Some(r) = rect {
                        *pos = Some([r.min.x, r.min.y]);
                    }
                    if let Some(r) = ctx.input(|i| i.viewport().inner_rect) {
                        *wsize = [r.width(), r.height()];
                    }
                }
            });
            if ret {
                back.push(k);
            }
        }
        for k in back.into_iter().rev() {
            let w = wins.remove(k);
            #[cfg(target_arch = "wasm32")]
            if let Some(t) = &mut self.tabs {
                t.blocked.remove(&w.pane.id);
            }
            self.win_init.remove(&w.pane.id);
            crate::layout::redock(&mut self.dock, w.pane, w.home.as_ref());
        }
        // 窓を出している間に足された窓（パネルの右クリックなど）
        wins.append(&mut self.windows);
        self.windows = wins;
    }

    fn pane_of(&self, f: impl Fn(&Kind) -> bool) -> Option<u64> {
        self.dock
            .iter_all_tabs()
            .find(|(_, p)| f(&p.kind))
            .map(|(_, p)| p.id)
    }
}

impl App {
    /// 画面の写しが届いたら、頼んだ図の所を切り出して写す・PNG で保存する（GUX-18）
    fn take_shot(&mut self, ctx: &egui::Context) {
        let Some((id, copy, cmd)) = self.c.shot.clone() else {
            return;
        };
        let img = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(img) = img else {
            return;
        };
        self.c.shot = None;
        let Some(r) = self.c.fig_rects.get(&id).copied() else {
            return;
        };
        let Some(part) = crate::image_util::crop_image(&img, r, ctx.pixels_per_point()) else {
            return;
        };
        if copy {
            ctx.copy_image(part);
            self.c.info(tr("Copied the figure as an image"));
            return;
        }
        let Some(p) = crate::files::FileDialog::new()
            .add_filter("PNG", &["png"])
            .set_file_name(format!("{cmd}.png"))
            .save_file()
        else {
            return;
        };
        self.c.status = match crate::image_util::png_bytes(&part)
            .and_then(|b| lensbench_core::vfs::write(&p, b).map_err(|e| e.to_string()))
        {
            Ok(()) => Status {
                text: trf("Saved to {}", &[&p.display()]),
                error: false,
            },
            Err(e) => Status {
                text: format!("{}: {e}", p.display()),
                error: true,
            },
        };
    }

    /// 窓を閉じる前に、保存していない変更があれば「保存 / 保存しない / やめる」を聞く（GUX-02）
    fn close_guard(&mut self, ctx: &egui::Context) {
        if cfg!(target_arch = "wasm32") || self.tour.is_some() {
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.c.close_ok && self.c.close_loses()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.c.close_ask = true;
        }
        // 保存してから閉じる: 保存が終わって変更が無くなったら閉じる（失敗したらやめる）
        if self.c.close_pending {
            if self.c.status.error {
                self.c.close_pending = false;
            } else if !self.c.dirty() && !self.c.worker.busy() {
                self.c.close_pending = false;
                self.c.close_ok = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if !self.c.close_ask {
            return;
        }
        let mut choice = None;
        egui::Window::new(tr("Unsaved changes"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(tr("There are unsaved changes. Save before closing?"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(RichText::new(tr("Save and close")).strong())
                        .clicked()
                    {
                        choice = Some(0);
                    }
                    if ui.button(tr("Close without saving")).clicked() {
                        choice = Some(1);
                    }
                    if ui.button(tr("Cancel")).clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        choice = Some(2);
                    }
                });
            });
        match choice {
            Some(0) => {
                self.c.close_ask = false;
                self.c.status.error = false;
                self.c.save(false);
                // 保存の窓をやめたら閉じない（開いた系の保存は計算のスレッドで終わるのを待つ）
                self.c.close_pending = self.c.worker.busy() || !self.c.dirty();
            }
            Some(1) => {
                self.c.close_ask = false;
                self.c.close_ok = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(2) => self.c.close_ask = false,
            _ => {}
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(target_arch = "wasm32")]
        if let Some(v) = self.viewer.as_mut() {
            v.update(ctx);
            return;
        }
        self.logic(ctx);
        // 窓に落としたファイル
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(crate::files::dropped)
                .collect()
        });
        if let Some(p) = dropped.into_iter().next() {
            self.c.open_doc(&p);
        }
        // キー
        let (save_as, save) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S),
                i.consume_key(Modifiers::COMMAND, Key::S),
            )
        });
        if save_as || save {
            self.c.save(save_as);
        }
        let (new, open, palette, f5, reload, help, next, prev) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::N),
                i.consume_key(Modifiers::COMMAND, Key::O),
                i.consume_key(Modifiers::COMMAND, Key::K),
                i.consume_key(Modifiers::NONE, Key::F5),
                i.consume_key(Modifiers::COMMAND, Key::R),
                i.consume_key(Modifiers::NONE, Key::F1),
                i.consume_key(Modifiers::COMMAND, Key::Tab),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab),
            )
        });
        if new {
            self.c.new_system();
        }
        if open {
            self.c.pick_and_open();
        }
        if palette {
            if self.palette.open {
                self.palette.open = false;
            } else {
                self.palette.show();
            }
        }
        if f5 {
            self.run_all(false);
        }
        if reload {
            self.c.reload();
        }
        if help {
            self.c.help_open = !self.c.help_open;
        }
        if next || prev {
            self.cycle(prev);
        }
        // Ctrl+Z・Ctrl+Y（Ctrl+Shift+Z）: 系の変更を元に戻す・やり直す（入力欄の中では欄の文字の取り消し）
        if !ctx.wants_keyboard_input() {
            let (redo, undo) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::COMMAND, Key::Y)
                        || i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                    i.consume_key(Modifiers::COMMAND, Key::Z),
                )
            });
            if undo || redo {
                self.c.undo(undo, 1);
            }
        }
        // Ctrl+A・Ctrl+Shift+A: 多重構成の次・前の構成（入力欄にフォーカスがあるときは全選択に使う）
        if !ctx.wants_keyboard_input() {
            let (back, fwd) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::A),
                    i.consume_key(Modifiers::COMMAND, Key::A),
                )
            });
            if back || fwd {
                self.c.cycle_config(back);
            }
        }
        // 組合せの系は保存したプロジェクトの名前（一時ファイルの名前は出さない）
        let shown = self.c.loc.shown(self.c.is_combo());
        let mark = if self.c.dirty() { "● " } else { "" };
        let title = match shown {
            Some(f) => format!(
                "lensbench — {mark}{}",
                f.file_name()
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            None if !mark.is_empty() => format!("lensbench — {mark}{}", tr("(unsaved)")),
            None => "lensbench".into(),
        };
        // 変わったときだけ送る（送ると次のフレームを描くので、毎フレーム送ると何もしていなくても描き続ける）
        let changed = ctx.data_mut(|d| {
            let k = egui::Id::new("lensbench-title");
            let same = d.get_temp::<String>(k).is_some_and(|t| t == title);
            if !same {
                d.insert_temp(k, title.clone());
            }
            !same
        });
        if changed {
            // Web 版はブラウザのタブの題にする（起動確認 scripts/web_smoke.mjs が開いたファイルの名前を見る）
            #[cfg(target_arch = "wasm32")]
            crate::web_tabs::set_title(&title);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let os = self.c.s.os_titlebar || self.tour.is_some();
            if self.decorations != Some(os) {
                self.decorations = Some(os);
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(os));
            }
            if !os {
                crate::chrome::resize_edges(ctx);
            }
        }
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ctx, |ui| self.header(ctx, ui));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));
        let mut viewer = Viewer {
            c: &mut self.c,
            to_window: Vec::new(),
            added: Vec::new(),
            dragging: None,
        };
        let mut style = egui_dock::Style::from_egui(ctx.style().as_ref());
        style.tab_bar.corner_radius = egui::CornerRadius::same(6);
        style.tab.tab_body.inner_margin = egui::Margin::same(10);
        DockArea::new(&mut self.dock)
            .style(style)
            .show_add_buttons(true)
            .show_add_popup(true)
            .show_leaf_collapse_buttons(false)
            // フロートの窓を本体の外まで動かせるように（外に置くと別の OS の窓にする）
            .window_bounds(egui::Rect::EVERYTHING)
            .show(ctx, &mut viewer);
        let Viewer {
            to_window,
            added,
            dragging,
            ..
        } = viewer;
        self.drag_out(ctx, dragging);
        // カタログの行を引きずっている間は、ポインタの横に名前を出す
        if let (Some(p), Some(q)) = (
            egui::DragAndDrop::payload::<combo::DropPick>(ctx),
            ctx.pointer_latest_pos(),
        ) {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            egui::Area::new(egui::Id::new("lensbench-drop-pick"))
                .order(egui::Order::Tooltip)
                .fixed_pos(q + egui::vec2(14.0, 10.0))
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(trf("{} → layout", &[&p.0.label()]));
                    });
                });
        }
        for (s, n, k) in added {
            // 1 枚だけのパネルがもうあれば、2 枚目を作らず表に出す（GUX-16）
            if k.single() {
                let same = |x: &Kind| std::mem::discriminant(x) == std::mem::discriminant(&k);
                if let Some(id) = self.pane_of(same) {
                    self.focus(id);
                    continue;
                }
            }
            let id = self.c.new_id();
            self.dock.set_focused_node_and_surface((s, n));
            self.dock.push_to_focused_leaf(Pane { id, kind: k });
        }
        for id in to_window {
            self.detach(id);
        }
        self.float_out(ctx);
        #[cfg(target_arch = "wasm32")]
        self.web_tabs(ctx);
        self.os_windows(ctx);
        // 項目はパレットを開いているときだけ作る（GAR-09）
        if self.palette.open {
            let items = self.palette_items();
            if let Some(cmd) = self.palette.ui(ctx, &items) {
                self.exec(ctx, cmd);
            }
        }
        for cmd in std::mem::take(&mut self.c.cmds) {
            self.exec(ctx, cmd);
        }
        if self.c.folders_open {
            self.c.folders_window(ctx);
        }
        self.close_guard(ctx);
        self.take_shot(ctx);
        self.c.missing_glass_window(ctx);
        if self.c.help_open {
            self.c.help_window(ctx);
        }
        if self.c.p.cat_save.open {
            self.c.cat_save_window(ctx);
        }
        self.c.fill_window(ctx);
        self.save_layout_window(ctx);
        // 別の窓で出た確かめはその窓に出す（窓が閉じていれば本体に）
        #[cfg(target_arch = "wasm32")]
        let elsewhere = self.c.ask_from().is_some_and(|id| {
            self.windows.iter().any(|w| w.pane.id == id)
                && self.tabs.as_ref().is_some_and(|t| t.is_open(id))
        });
        #[cfg(not(target_arch = "wasm32"))]
        let elsewhere = false;
        if !elsewhere {
            self.c.ask_ui(ctx);
        }
        self.tour_frame(ctx);
    }

    /// Web 版は設定（最近のファイル・並びなど）をこまめに残す。eframe 0.31 の Web 版はページを閉じるときに
    /// 保存しない（"onbeforeunload" で待つので呼ばれない）
    #[cfg(target_arch = "wasm32")]
    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(2)
    }

    #[cfg(target_arch = "wasm32")]
    fn persist_egui_memory(&self) -> bool {
        self.viewer.is_none()
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.tour.is_some() {
            return;
        }
        // 別のタブのパネルは元のタブの設定を上書きしない
        #[cfg(target_arch = "wasm32")]
        if self.viewer.is_some() {
            return;
        }
        self.c.s.combo = self.c.doc.combo.clone();
        self.c.s.ld_tree = self.c.p.ld.tree;
        self.c.s.ld_detail = self.c.p.ld.detail;
        self.c.s.ld_by_surface = self.c.p.ld.by_surface;
        self.c.s.panels = self.c.panels_value();
        self.c.s.layout = Some(self.layout());
        // ポータブル: 設定のフォルダの中のパスは相対で書く
        if let Some(b) = crate::portable::base() {
            let keep = self.c.s.paths();
            self.c.s.map_paths(|p| crate::portable::to_stored(p, b));
            eframe::set_value(storage, eframe::APP_KEY, &self.c.s);
            self.c.s.set_paths(keep);
        } else {
            eframe::set_value(storage, eframe::APP_KEY, &self.c.s);
        }
        self.c.s.layout = None;
    }
}
/// レイアウト図に重ねる構成（.zmx の多重構成・プロジェクトの状態・BS の経路）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    /// 選べる構成（serve の layout の鍵 cfgs・states・paths と名前）と、重ねるか
    pub items: Vec<(&'static str, String, bool)>,
    /// 構成 k を k 倍ずらす量 [dz, dy] mm
    pub offset: [f64; 2],
}

impl Overlay {
    /// serve の configs の結果から。前に選んでいた名前は選んだまま
    /// 選べる構成を読み直す。選んでいたものが無くなったら true（レイアウトを頼み直す）
    pub fn update(&mut self, v: &Value) -> bool {
        let was: Vec<(&str, String)> = self
            .items
            .iter()
            .filter(|x| x.2)
            .map(|x| (x.0, x.1.clone()))
            .collect();
        let mut items = Vec::new();
        let n = v["cfgs"].as_u64().unwrap_or(1);
        if n > 1 {
            items.extend((1..=n).map(|k| ("cfgs", k.to_string())));
        }
        for key in ["states", "paths"] {
            let names: Vec<String> = v[key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if names.len() > 1 {
                items.extend(names.into_iter().map(|x| (key, x)));
            }
        }
        self.items = items
            .into_iter()
            .map(|(k, x)| {
                let on = was.iter().any(|w| w.0 == k && w.1 == x);
                (k, x, on)
            })
            .collect();
        self.items.iter().filter(|x| x.2).count() != was.len()
    }

    /// [`Self::params`] に見る面を足したもの。ずらす量の縦は見る面の縦（xz 面なら x）
    pub fn params_in(&self, plane: ViewPlane) -> Value {
        let mut v = self.params();
        if plane == ViewPlane::Xz {
            if let Some(o) = v.get_mut("offset") {
                *o = json!([self.offset[1], 0.0, self.offset[0]]);
            }
        }
        if plane != ViewPlane::Yz && plane != ViewPlane::Unfold {
            v["plane"] = json!(plane.key());
        }
        if plane == ViewPlane::Unfold {
            v["unfold"] = json!(true);
        }
        v
    }

    /// serve の layout の params（何も選ばなければ今の構成だけ、見る面は yz。BS の経路の鍵は beam_paths）
    pub fn params(&self) -> Value {
        let mut m = Map::new();
        for key in ["cfgs", "states", "paths"] {
            let on: Vec<Value> = self
                .items
                .iter()
                .filter(|x| x.0 == key && x.2)
                .map(|x| {
                    if key == "cfgs" {
                        json!(x.1.parse::<u64>().unwrap_or(1))
                    } else {
                        json!(x.1)
                    }
                })
                .collect();
            if !on.is_empty() {
                m.insert(key.replace("paths", "beam_paths"), Value::Array(on));
            }
        }
        if !m.is_empty() && self.offset != [0.0; 2] {
            m.insert(
                "offset".into(),
                json!([0.0, self.offset[1], self.offset[0]]),
            );
        }
        Value::Object(m)
    }

    /// 選ぶ欄（選べる構成が無ければ出さない）。変えたら true。vert は図の縦の軸の名前
    pub fn ui(&mut self, ui: &mut Ui, vert: &str) -> bool {
        if self.items.is_empty() {
            return false;
        }
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Overlay"));
            let mut last = "";
            for (key, name, on) in &mut self.items {
                if *key != last {
                    last = key;
                    ui.weak(match *key {
                        "cfgs" => tr("Config"),
                        "states" => tr("State"),
                        _ => tr("BS path"),
                    });
                }
                changed |= ui.checkbox(on, name.as_str()).changed();
            }
            if self.items.iter().any(|x| x.2) {
                ui.separator();
                ui.label(tr("Offset by"));
                for (i, l) in ["z", vert].into_iter().enumerate() {
                    ui.weak(l);
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.offset[i])
                                .speed(0.5)
                                .suffix(" mm"),
                        )
                        .changed();
                }
            }
        });
        changed
    }
}

/// 重ねる構成: configs の結果から欄を作り、選んだものだけ layout の params に。読み直しても選んだまま
#[test]
fn overlay_params() {
    let mut o = Overlay::default();
    o.update(&json!({ "cfgs": 1, "states": ["A"], "paths": ["TT", "RR"] }));
    assert_eq!(o.items.len(), 2);
    assert_eq!(o.params(), json!({}));
    o.items[1].2 = true;
    o.offset = [30.0, 0.0];
    assert_eq!(
        o.params(),
        json!({ "beam_paths": ["RR"], "offset": [0.0, 0.0, 30.0] })
    );
    o.update(&json!({ "cfgs": 1, "paths": ["RR", "TT", "TR"] }));
    assert_eq!(o.items.iter().filter(|x| x.2).count(), 1);
    assert_eq!(o.items[0].1, "RR");
    o.update(&json!({ "cfgs": 3 }));
    o.items[0].2 = true;
    o.items[2].2 = true;
    o.offset = [0.0; 2];
    assert_eq!(o.params(), json!({ "cfgs": [1, 3] }));
    // xz 面: plane を足し、ずらす量の縦は x
    o.offset = [10.0, 5.0];
    assert_eq!(
        o.params_in(ViewPlane::Xz),
        json!({ "cfgs": [1, 3], "offset": [5.0, 0.0, 10.0], "plane": "xz" })
    );
    assert_eq!(o.params_in(ViewPlane::Yz), o.params());
    // 展開: yz のまま unfold、3D は plane
    assert_eq!(
        o.params_in(ViewPlane::Unfold),
        json!({ "cfgs": [1, 3], "offset": [0.0, 5.0, 10.0], "unfold": true })
    );
    assert_eq!(o.params_in(ViewPlane::D3)["plane"], "3d");
    // 設定に保存する名前
    assert_eq!(serde_json::to_value(ViewPlane::D3).unwrap(), json!("3d"));
    assert_eq!(
        serde_json::to_value(ViewPlane::Unfold).unwrap(),
        json!("unfold")
    );
}

/// 解析の結果のメニューから頼むこと
enum ResOut {
    /// 図を画像にする（true は写す、false は PNG で保存）
    Shot(bool),
    /// 保存できなかった
    Error(String),
}

/// 解析の結果を写す・保存するメニュー（JSON・テキスト（タブ区切り）・CSV、#128。図の画像、GUX-18）
fn result_export_menu(ui: &mut Ui, cmd: &str, v: &Value, fig: bool) -> Option<ResOut> {
    use crate::export_table::{delimited, tables};
    let mut out = None;
    ui.menu_button(tr("Copy ▼"), |ui| {
        if fig && ui.button(tr("Figure as image")).clicked() {
            out = Some(ResOut::Shot(true));
            ui.close_menu();
        }
        if ui.button(tr("JSON")).clicked() {
            ui.ctx()
                .copy_text(serde_json::to_string_pretty(v).unwrap_or_default());
            ui.close_menu();
        }
        if ui
            .button(tr("Text (tab-separated)"))
            .on_hover_text(tr(
                "Pastes into a spreadsheet. Multiple tables are separated by \"# name\" lines",
            ))
            .clicked()
        {
            ui.ctx().copy_text(delimited(&tables(v), '\t'));
            ui.close_menu();
        }
        if ui.button(tr("CSV")).clicked() {
            ui.ctx().copy_text(delimited(&tables(v), ','));
            ui.close_menu();
        }
    });
    ui.menu_button(tr("Save ▼"), |ui| {
        for (label, ext) in [(tr("JSON…"), "json"), (tr("CSV…"), "csv")] {
            if ui.button(label).clicked() {
                ui.close_menu();
                if let Some(p) = crate::files::FileDialog::new()
                    .add_filter(ext.to_uppercase(), &[ext])
                    .set_file_name(format!("{cmd}.{ext}"))
                    .save_file()
                {
                    let text = if ext == "json" {
                        serde_json::to_string_pretty(v).unwrap_or_default()
                    } else {
                        delimited(&tables(v), ',')
                    };
                    if let Err(e) = lensbench_core::vfs::write(&p, text) {
                        out = Some(ResOut::Error(format!("{}: {e}", p.display())));
                    }
                }
            }
        }
        if fig && ui.button(tr("Figure as image (PNG)…")).clicked() {
            ui.close_menu();
            out = Some(ResOut::Shot(false));
        }
    });
    out
}

/// マニュアル（ドキュメントのサイト）の場所。非公開の版などは LENSBENCH_DOCS_URL で差し替える
const DOCS_URL: &str = match option_env!("LENSBENCH_DOCS_URL") {
    Some(u) => u,
    None => "https://masa-iwm.github.io/lensbench/",
};

/// マニュアルのページの URL
fn docs_url(page: &str) -> String {
    let base = DOCS_URL.trim_end_matches('/');
    if page.is_empty() {
        format!("{base}/")
    } else {
        format!("{base}/{page}")
    }
}

#[test]
fn docs_urls() {
    assert!(docs_url("").ends_with('/'));
    assert!(docs_url("gui.html").ends_with("/gui.html"));
    assert!(!docs_url("gui.html").contains("//gui"));
}

/// 見本の組合せ（examples/combos。Web 版の「見本」に出る、#134）
#[cfg(test)]
fn sample_combos() -> Vec<(&'static str, Combo)> {
    use crate::sysset::{ApKind, FieldKind, FieldPt};
    use combo::{ElemKind, Pick};
    let cat = |part: &str, air: f64, reversed: bool| Pick {
        vendor: "LB".into(),
        part: part.into(),
        air,
        reversed,
        locked: true,
        ..Pick::default()
    };
    let mut c1 = Combo {
        name: "C1 二枚のカタログレンズ".into(),
        epd: 8.0,
        stop_gap: 5.0,
        fields: [0.0, 2.0, 4.0].map(FieldPt::y).to_vec(),
        ..Default::default()
    };
    c1.picks = vec![cat("PCX-100", 3.0, false), cat("ACH-45", 30.0, true)];
    let mut c2 = Combo {
        name: "C2 折り曲げ鏡の後の玉".into(),
        epd: 8.0,
        stop_gap: 10.0,
        fields: [0.0, 2.0, 3.0].map(FieldPt::y).to_vec(),
        ..Default::default()
    };
    let mut m = Pick::new(ElemKind::Mirror);
    m.semi = 10.0;
    m.air = 20.0;
    c2.picks = vec![m, cat("BCX-50", 45.0, false)];
    let mut c3 = Combo {
        name: "C3 絞りを挟む 3 枚（有限物体）".into(),
        aperture: ApKind::Na,
        ap_value: 0.04,
        field_kind: FieldKind::ObjectHeight,
        fields: [0.0, 3.0, 6.0].map(FieldPt::y).to_vec(),
        object_distance: Some(100.0),
        ..Default::default()
    };
    let mut st = Pick::new(ElemKind::Stop);
    st.semi = 5.0;
    st.air = 4.0;
    c3.picks = vec![
        cat("BCX-50", 6.0, false),
        cat("BCC-32", 4.0, true),
        st,
        cat("MEN-75", 60.0, false),
    ];
    vec![
        ("C1_catalog_two", c1),
        ("C2_fold_mirror", c2),
        ("C3_stop_inside", c3),
    ]
}

/// examples/combos の見本は sample_combos から作ったもの（作り直すには LENSBENCH_UPDATE_SAMPLES=1 で cargo test）
#[test]
fn sample_combos_are_up_to_date() {
    let dir = std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/combos"
    ));
    let update = std::env::var_os("LENSBENCH_UPDATE_SAMPLES").is_some();
    for (name, c) in sample_combos() {
        assert!(c.settings_problems().is_empty(), "{name}");
        let want = serde_json::to_string_pretty(&doc_file::project_file(&c)).unwrap() + "\n";
        let p = dir.join(format!("{name}.json"));
        if update {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&p, &want).unwrap();
        }
        let got = std::fs::read_to_string(&p)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert_eq!(
            got, want,
            "{name}: examples/combos が古い。LENSBENCH_UPDATE_SAMPLES=1 cargo test -p lensbench-gui sample_combos で作り直す"
        );
        lensbench_project::Project::from_json(&got).unwrap();
        // 開くとブロックから同じ組合せ（ロックも）に戻る
        let v: Value = serde_json::from_str(&got).unwrap();
        let mut back = Combo::from_project(&v, &dir).unwrap();
        doc_file::apply_view(&mut back, v.get("gui"));
        for p in &mut back.picks {
            p.id.clear();
            p.air_id.clear();
        }
        assert_eq!(back, c, "{name}");
    }
}

#[cfg(test)]
mod open_insert_tests {
    use super::*;
    use combo::{ElemKind, Pick};

    /// 開いた系の空気の間隔に要素を差し込む: 取り込みを 2 つに分け、間隔を半分ずつ前後に分ける。
    /// 差し込んだ要素の前後の面は元の系と同じ（間隔の中の位置だけが変わる）
    #[test]
    fn split_open_inserts_in_air_gap() {
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let glass = vec![root.join("tests/glass")];
        let zmx = root.join("tests/refs/A0/system.zmx");
        let opt = lensbench_core::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: glass.clone(),
            ..Default::default()
        };
        let a = lensbench_core::zmx::load_with(&zmx, &opt).unwrap();
        let rows: Vec<lens_data::Row> = a
            .surfaces
            .iter()
            .map(|s| lens_data::Row {
                t: s.t,
                material: s.mat.clone(),
                ..Default::default()
            })
            .collect();
        let img = rows.len() - 1;
        // 物体面の後・像面の後・ガラスの中には差し込めない
        assert!(!open_gap_ok(&rows, 0) && !open_gap_ok(&rows, img));
        let gap = (1..img)
            .find(|&j| open_gap_ok(&rows, j) && open_gap_ok(&rows, j) && j + 1 < img)
            .expect("空気の間隔");
        let glass_j = (1..img).find(|&j| !rows[j].material.is_empty()).unwrap();
        assert!(!open_gap_ok(&rows, glass_j));

        let d = lensbench_project::edit_ops::system_data(&a).unwrap();
        let mut c = Combo::default();
        c.take_system_data(&d, a.surfaces[0].t).unwrap();
        let mut z = Pick::new(ElemKind::ZmxRange);
        z.path = zmx.display().to_string();
        z.air = 0.0;
        // 差し込む要素: 厚さ 2 mm の平板
        let mut p = Pick::new(ElemKind::Plate);
        p.t = 2.0;
        p.glass = "N-BK7".into();
        p.semi = 5.0;
        // 開いた系の玉（lens1 1–2、lens2 3–4、lens3 6–7）を群として残す
        let blk = |name: &str, surfs: Vec<usize>| lens_data::Block {
            name: name.into(),
            surfs,
            parent: None,
        };
        z.zgroups = open_groups(&[
            blk("lens1", vec![1, 2]),
            blk("lens2", vec![3, 4]),
            blk("lens3", vec![6, 7]),
        ]);
        let picks = split_open(z.clone(), &rows, Some(gap), p.clone());
        assert_eq!(picks.len(), 3);
        assert_eq!((picks[0].range, picks[2].range), ([1, gap], [gap + 1, 0]));
        c.picks = picks;
        let pj = lensbench_project::Project::from_json(&c.project().to_string()).unwrap();
        let env = lensbench_project::Env {
            base: root.clone(),
            glass_dirs: glass,
            ..Default::default()
        };
        let b = lensbench_project::expand(&pj, &env).unwrap();
        // 平板の 2 面が増える
        assert_eq!(b.surfaces.len(), a.surfaces.len() + 2);
        // 元の玉は取り込みのブロックの中の玉として残る
        let names: Vec<&str> = b.units.iter().map(|u| u.name.as_str()).collect();
        for n in ["lens1", "lens2", "lens3", "ZX1", "ZX2"] {
            assert!(names.contains(&n), "{n} が無い: {names:?}");
        }
        let half = a.surfaces[gap].t / 2.0;
        // 物体面の厚さは系の設定（物体の位置）なので比べない
        for j in 1..gap {
            assert_eq!(a.surfaces[j].c, b.surfaces[j].c);
            assert!((a.surfaces[j].t - b.surfaces[j].t).abs() < 1e-12, "面 {j}");
        }
        assert!((b.surfaces[gap].t - half).abs() < 1e-12);
        assert!((b.surfaces[gap + 1].t - 2.0).abs() < 1e-12);
        assert!((b.surfaces[gap + 2].t - half).abs() < 1e-12);
        for j in gap + 1..a.surfaces.len() {
            assert_eq!(a.surfaces[j].c, b.surfaces[j + 2].c, "面 {j}");
        }
        // 差し込めない間隔・None は系の後ろ
        assert_eq!(
            split_open(z.clone(), &rows, Some(glass_j), p.clone()).len(),
            2
        );
        assert_eq!(split_open(z, &rows, None, p).len(), 2);
    }
}
