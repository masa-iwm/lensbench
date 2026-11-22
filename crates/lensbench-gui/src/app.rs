//! 窓の骨格: 上の帯（ファイル・探す・計算）、パネル（ドッキング・タブ・フロート・別の OS の窓）、
//! 下の状態の行（焦点距離・F 値など）。
//!
//! 計算はすべて serve と同じ method（load・analyze・get・set・write・trace・glasses・lenses）を [`Worker`] に頼む。
//! 系のブロック編集とレイアウト図は、系の編集・保存形式の上にパネルとして足す。

mod ask;
use export_menu::{copy_menu, result_export_menu, ResOut};
mod doc_file;
mod doc_loc;
mod edit;
mod export_menu;
mod folders;
mod help;
mod history;
mod ld_combo;
use history::{ComboHist, DocHist};
mod frame;
mod menus;
mod msg;
mod panel_msgs;
use msg::Msg;
mod open_doc;
mod panels;
mod record;
mod result;
mod save_as;
mod status;
mod sys_copy;
mod thermal_ui;
mod tick;
use doc_loc::DocLoc;
use panels::Panels;
use result::AnaRes;
use sys_copy::SysCopy;
mod analysis_ui;
mod calc_panel;
mod lens_data_ui;
mod take;
#[cfg(any(target_arch = "wasm32", test))]
mod test_state;
mod tool_ui;
mod tour_steps;
#[cfg(test)]
mod ui_tests;

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
use crate::combo_doc::Snap;
use crate::figures;
use crate::i18n::{self, n_, tr, trf, Lang};
use crate::json_view::{self, fmt_num};
use crate::layout::{self, AnaCfg, Kind, Layout, OsWin, Pane, Preset, View};
use crate::lens_data;
use crate::missing_glass::{missing_glass_of, model_glass_zmx};
use crate::palette::{Item, Palette};
use crate::plots::{self, SpotRays, SysInfo};
use crate::split;
use crate::theme::{self, Look, Mode};
use crate::tour::{Args, Step, Tour};
use crate::worker::{
    get_param, into_params, load_file, AnalyzeReq, Call, CatOp, Done, GetReq, LayoutReq, Tag,
    TraceReq, Worker,
};

/// 覚えておく好み（次に開いたときも使う）。開いていた文書と画面の様子は [`Session`] に分ける
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub glass_dirs: Vec<String>,
    pub lens_dirs: Vec<String>,
    pub recent: Vec<String>,
    pub look: Look,
    pub lang: Lang,
    /// 名前を付けて保存した並び
    pub layouts: Vec<(String, Layout)>,
    /// 組合せを変えたらすぐ系を作り直す
    pub auto_build: bool,
    /// レイアウト図・書き出しの光線と書き出しの設定
    pub rays: crate::layout_rays::RaySet,
    /// カタログレンズの一覧で隠す列（[`catalog::LensCol::key`]）
    pub lens_hide: Vec<String>,
    /// アプリ版で OS のタイトルバーを使う（false は上の帯をタイトルバーにする）
    pub os_titlebar: bool,
    /// 品の STEP の共有の置き場（アプリ版。空なら環境変数 LENSBENCH_CAD_STORE、なければ配布元だけ）
    pub cad_store: crate::cad_store::CadStore,
}

impl Settings {
    /// 設定の中のパス（硝材・レンズのフォルダ、最近のファイル、組合せの .zmx）をそれぞれ f で変える
    pub(crate) fn map_paths(&mut self, f: impl Fn(&str) -> String) {
        for p in self
            .glass_dirs
            .iter_mut()
            .chain(self.lens_dirs.iter_mut())
            .chain(self.recent.iter_mut())
        {
            *p = f(p);
        }
    }

    fn paths(&self) -> Settings {
        Settings {
            glass_dirs: self.glass_dirs.clone(),
            lens_dirs: self.lens_dirs.clone(),
            recent: self.recent.clone(),
            ..Default::default()
        }
    }

    fn set_paths(&mut self, k: Settings) {
        self.glass_dirs = k.glass_dirs;
        self.lens_dirs = k.lens_dirs;
        self.recent = k.recent;
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
pub(crate) enum ViewPlane {
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
    pub(crate) const ALL: [ViewPlane; 4] = [
        ViewPlane::Yz,
        ViewPlane::Xz,
        ViewPlane::Unfold,
        ViewPlane::D3,
    ];

    /// serve の layout の plane
    pub(crate) fn view(self) -> lensbench_engine::layout::View {
        use lensbench_engine::layout::View;
        match self {
            ViewPlane::Yz | ViewPlane::Unfold => View::YZ,
            ViewPlane::Xz => View::XZ,
            ViewPlane::D3 => View::D3,
        }
    }
    /// 図の縦の軸の名前
    pub(crate) fn vert(self) -> &'static str {
        match self {
            ViewPlane::Xz => "x",
            _ => "y",
        }
    }
    /// 見る面の選択の表示
    pub(crate) fn label(self) -> &'static str {
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
            look: Look::default(),
            lang: Lang::Auto,
            layouts: Vec::new(),
            auto_build: true,
            rays: Default::default(),
            lens_hide: Vec::new(),
            os_titlebar: false,
            cad_store: Default::default(),
        }
    }
}

/// 次に開いたときに戻す、開いていた文書と画面の様子（好みの [`Settings`] と分ける）。
/// 保存するときだけ今の文書と画面から作る（毎フレーム写さない）
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Session {
    pub combo: Combo,
    /// 今の並び
    pub layout: Option<Layout>,
    /// 系のパネルで見るパラメータ（3.t、G1.shift など）
    pub params: Vec<String>,
    /// レイアウト図の見る面
    pub view_plane: ViewPlane,
    /// レンズデータの表をブロックごとに折りたたむ表示（false はブロックに入って中を見る表示）
    pub ld_tree: bool,
    /// レンズデータの表の下に詳細設定（開口・傾き偏心・形のデータ・ブロックの設定）を出す
    pub ld_detail: bool,
    /// レンズデータの表を面単位で選ぶ（ブロックを自動で選ばない）
    pub ld_by_surface: bool,
    /// 最適化・公差・差し替え・ズームのパネルの設定（serve と同じ形。次に開いたときも残す）
    pub panels: Value,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            combo: Combo::default(),
            layout: None,
            params: Vec::new(),
            view_plane: ViewPlane::Yz,
            // 既定は折りたたみ表示で全部開く（曲率半径・硝材がすぐ見える）
            ld_tree: true,
            ld_detail: false,
            ld_by_surface: false,
            panels: Value::Null,
        }
    }
}

impl Session {
    /// 読んだ組合せの相対パスを、設定のフォルダから戻す（ポータブル・--settings のとき）
    fn resolve_paths(&mut self) {
        if let Some(b) = crate::portable::base() {
            self.combo
                .map_paths(&|p: &str| crate::portable::from_stored(p, b));
        }
    }
}

/// 保存先での [`Session`] の鍵（好みは eframe::APP_KEY）
const SESSION_KEY: &str = "lensbench_session";

#[derive(Default)]
struct Status {
    text: String,
    error: bool,
}

impl Core {
    /// 注意を下の行の履歴（直近 20 件）に足す（系を読んだときの注意。下の行には出さない）
    fn note_history(&mut self, notes: Vec<String>) {
        self.status_log
            .extend(notes.into_iter().map(|t| (t, false, None)));
        let n = self.status_log.len();
        if n > 20 {
            self.status_log.drain(..n - 20);
        }
    }

    /// 下の行に知らせを出す（利用者の操作の結果）
    fn info(&mut self, text: impl Into<String>) {
        self.status = Status {
            text: text.into(),
            error: false,
        };
    }

    /// 下の行に誤りを出す
    fn fail(&mut self, text: impl Into<String>) {
        self.fails += 1;
        self.status = Status {
            text: lensbench_core::i18n::tr(&text.into()), // 計算側の英語の文を訳す
            error: true,
        };
    }
}

/// 結果の JSON の表示で見せる長さ（字数。これより長い分は切る）
const JSON_VIEW_MAX: usize = 200_000;

/// serve の要求の名前を画面の言葉に（状態の行）
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

/// カタログに無いガラスで開けなかった系
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

/// パネルの外の状態（パネルの中から触る）
struct Core {
    /// パネルの状態
    p: Panels,
    s: Settings,
    /// 系を変える要求の返事に付けた読み直しを受け取っている間（[`Core::refresh`] で頼み直さない）
    bundled_refresh: bool,
    /// 開いていた文書と画面の様子のうち、画面で使い続けるもの（見るパラメータ・図の面・ロック）。組合せ・並び・
    /// パネルの設定・表の見せ方は開いたときに取り出し、保存するときだけ今のものを入れる
    ses: Session,
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
    /// 下の行の誤りを出した要求とその文（同じ要求がうまくいったら消す）
    status_err: Option<(Tag, String)>,
    /// 文書の所在（開いた系のファイル・保存先・最後に作った組合せ）
    loc: DocLoc,
    _work: doc_loc::WorkDir,
    /// serve の系の写し（概要・面の表・元に戻す段など）
    sys: SysCopy,
    results: HashMap<u64, AnaRes>,
    new_param: String,
    /// まだ送っていない値の変更（計算中に動かした分はまとめて送る）
    pending_set: std::collections::BTreeMap<String, f64>,
    /// 開いている解析をすべて計算し直す（true は live の解析だけ）
    rerun: Option<bool>,
    browser: Browser,
    /// 組合せ（GUI が持つ文書）と、カタログから足す位置
    doc: crate::combo_doc::ComboDoc,
    folders_open: bool,
    /// 熱の構成を作って書き出す窓（開いていれば）
    thermal: Option<thermal_ui::ThermalDlg>,
    help_open: bool,
    save_layout_open: bool,
    /// 答えを待っている「はい・いいえ」の確かめ（[`Core::ask`]）
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
    /// 次の [`Core::open_doc_unlisted`] は、開いている段（元のファイルを開いたとき）を使う（モデルガラスの写し）
    reopen_same: bool,
    font: Option<String>,
    next_id: u64,
    /// パネルの中から頼まれた操作（パネルを描いた後に当てる）
    msgs: Vec<msg::Msg>,
    /// 操作の記録（記録中だけ）
    rec: Option<record::Recording>,
    /// 流し直している記録（[`record::Replaying`]）
    replay: Option<record::Replaying>,
    /// 誤りを出した数（流し直しで当たらなかった操作を見つける）
    fails: u64,
    /// まだ当てていない外への作用（計算の要求など）
    fx: msg::Fx,
    /// Web 版（元のタブ）: 解析のパネルを出した別の窓に写す系
    popup_doc: calc_panel::PopupDoc,
    /// 最後に組合せを変えて系を作り直した時刻（動かしている間は解析を待たせる）
    combo_edit: Option<web_time::Instant>,
    /// 組合せの元に戻す・やり直す
    combo_hist: ComboHist,
    /// 文書を替えた段（新規・開く）の元に戻す・やり直す
    docs: DocHist,
    /// レンズデータの硝材の候補（ガラスカタログを読んだときに作る）
    ld_glasses: std::sync::Arc<Vec<lens_data::GlassChoice>>,
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
    /// 状態の行の履歴（文、誤りか、誤りの案内）と、その窓を開いているか
    status_log: Vec<(String, bool, Option<String>)>,
    /// 今の誤りの文への案内（誤りの文、案内と符号）。文が変われば出さない
    status_hint: Option<(String, String)>,
    status_log_open: bool,
    /// 開けなかった系の、カタログに無いガラス（案内の窓）
    missing_glass: Option<MissingGlass>,
    /// 未保存の確認の後に閉じる（もう聞かない）
    close_ok: bool,
    /// 保存が終わったら閉じる
    close_pending: bool,
    /// 未完のカタログ品を、行を選んだら裏で補う
    auto_fill: crate::fill::AutoFill,
    swap_ids: Vec<String>,
    /// 結果・系の概要を受け取った回数（Web 版の別のタブに送り直す目安）
    rev: u64,
    /// [`Core::param_sys`] の写し（rev が変わったときだけ作り直す）
    param_sys_cache: std::cell::RefCell<Option<(u64, std::rc::Rc<crate::param_pick::Sys>)>>,
    /// [`Core::field_labels`] の写し
    field_labels_cache: std::cell::RefCell<analysis_ui::FieldLabels>,
    /// 系を変える要求（読込・値の変更・編集・元に戻す）の返事を受け取った回数（最適化の結果を当ててよいか）
    sys_gen: u64,
    saves: save_as::Saves,
}

pub(crate) struct App {
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
    /// Web 版: このタブが別の窓に出した解析以外のパネルなら、元のタブで描いた画面を受けて描く口
    #[cfg(target_arch = "wasm32")]
    viewer: Option<crate::web_tabs::Viewer>,
    /// Web 版（元のタブ）: 別の窓に出した解析以外のパネルを描く画面（パネル id ごと）
    #[cfg(target_arch = "wasm32")]
    hosts: HashMap<u64, crate::web_tabs::Host>,
    /// Web 版: このタブが別の窓に出した解析のパネルなら、元のタブから系を受けて自分で計算する口
    #[cfg(target_arch = "wasm32")]
    calc: Option<crate::web_tabs::Calc>,
    /// Web 版（解析のパネルの別の窓）: 今の系は平たい系ファイルで受けた（読めなくても送り直しを頼まない）
    #[cfg(target_arch = "wasm32")]
    calc_flat: bool,
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
pub(crate) fn icon() -> std::sync::Arc<egui::IconData> {
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

impl Core {
    /// 最適化・公差・差し替えのパネルの設定（変数・項・公差の表。serve の optimize・tolerance・swap と同じ形）。
    /// 空のパネルは入れない
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

    /// 保存していない変更がある（組合せが最後に保存・開いたものと違う）
    fn dirty(&self) -> bool {
        if self.loc.saved_combo.is_none() {
            !self.doc.combo.picks.is_empty()
        } else {
            !self.loc.saved_combo.same(&self.doc)
        }
    }

    /// 閉じると失う変更がある。一度もファイルに保存・ファイルから開いていない組合せは設定に残り、次に起動したときに
    /// 戻るので失わない（毎回の閉じる確認を出さない）
    fn close_loses(&self) -> bool {
        if self.loc.saved_combo.is_none() {
            return false;
        }
        self.dirty()
    }

    /// 最近のファイルの先頭に足す。Web 版の保存先（`/downloads/`、ダウンロードにして置き場から消える）は
    /// 開き直せないので足さない
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

    /// 読み直す（組合せから作り直す）
    fn reload(&mut self) {
        if let Some(f) = self.loc.file.clone() {
            self.open(&f);
        }
    }

    fn refresh_catalog(&mut self) {
        self.folders.stale();
        // 系の解析の順番を待たない（下書きの worker で読む）
        let paths = |v: &[String]| v.iter().map(std::path::PathBuf::from).collect::<Vec<_>>();
        let glass_dirs = paths(&self.s.glass_dirs);
        self.draft_call(
            Tag::Glasses,
            Call::Glasses(lensbench_engine::BrowseReq {
                glass_dirs: glass_dirs.clone(),
                lens_catalogs: Vec::new(),
            }),
        );
        self.draft_call(
            Tag::Lenses,
            Call::Lenses(lensbench_engine::BrowseReq {
                glass_dirs,
                lens_catalogs: paths(&self.s.lens_dirs),
            }),
        );
        self.browser.loaded = true;
        self.browser.glasses_pending = true;
        self.browser.lenses_pending = true;
    }

    /// serve の layout の params（重ねる構成・見る面と光線の設定）
    fn layout_params(&self) -> LayoutReq {
        let mut r = self.overlay.req_in(self.ses.view_plane);
        self.s
            .rays
            .apply(&mut r, self.n_fields(), self.waves_um().len());
        r
    }

    fn n_fields(&self) -> usize {
        self.sys
            .summary
            .as_ref()
            .and_then(|s| s["fields"].as_array().map(Vec::len))
            .unwrap_or(0)
    }

    /// パラメータの名前の欄の補完・階層選択の候補（面・玉・群・波長の数）
    /// 変数名の補完の候補を作る系の面・玉・群（6 つのパネルが毎フレーム呼ぶので rev ごとに 1 回だけ作る）
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
        // 配列を複製せずに読む
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
                let specs =
                    argform::object_specs(&o, o.chosen_plugin(&cfg.cmd, &cfg.args).as_deref());
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
            Ok(mut p) => {
                let args = p.as_object_mut().map(std::mem::take).unwrap_or_default();
                let mut r = AnalyzeReq::new(cfg.cmd.clone(), args);
                r.args.remove("cmd");
                self.call(Tag::Analyze(id), Call::Analyze(Box::new(r)));
                // 下の行は操作の結果だけ。計算中は要求の数を横に出す
                self.ana_sent(id);
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
                    calls.push(Call::Trace(Box::new(TraceReq {
                        px: Some(px.clone()),
                        py: Some(py.clone()),
                        field: Some(f),
                        wave: Some(w),
                        ..Default::default()
                    })));
                }
            }
            self.call_each(Tag::Rays(id), calls);
        }
    }

    /// 見るパラメータの値を 1 つの要求でまとめて読む（途中の誤りで止まれば残りは take が 1 つずつ頼み直す）
    fn read_params(&mut self) {
        let calls = self
            .ses
            .params
            .iter()
            .map(|p| Call::Get(GetReq::One(p.clone())))
            .collect();
        self.call_each(Tag::Gets, calls);
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
        self.loc.built_combo = Snap::new(Some(self.doc.combo.clone()));
        // 作業フォルダはメモリ（work_fs）: 書きかけを読まれることも、読み中で置き換えられないことも無い
        let res = crate::work_fs::with(|| {
            lensbench_core::vfs::create_dir_all(&dir).and_then(|_| {
                lensbench_core::vfs::write(
                    &path,
                    serde_json::to_string_pretty(&project).unwrap_or_default(),
                )
            })
        });
        match res {
            Ok(()) => {
                self.loc.built = Some(project);
                self.loc.combo_path = Some(path.clone());
                // 図は下書きで先に。系の読み込みは、前の読み込みが終わってから最新のファイルを読む
                self.draft_call(
                    Tag::Preview,
                    Call::Preview {
                        load: Box::new(self.load_req(&path)),
                        layout: Box::new(self.layout_params()),
                    },
                );
                if self.busy_core() {
                    self.load_stale = true;
                } else {
                    self.drop_queued();
                    self.open(&path);
                }
            }
            Err(e) => self.fail(format!("{}: {e}", path.display())),
        }
    }

    /// 保存（Ctrl+S）・名前を付けて保存（Ctrl+Shift+S）。組合せをプロジェクトの .json に書く。書き先は開いた・
    /// 前に保存した .json（.zmx から開いた組合せ・Web 版・as_new は書き先を選ぶ）
    fn save(&mut self, as_new: bool) {
        if self.loc.file.is_none() && self.doc.combo.picks.is_empty() {
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
                    .or(self.loc.shown())
                    .and_then(|f| f.file_stem())
                    .map(|x| x.to_string_lossy().into_owned())
                    .filter(|x| !x.is_empty())
                    .or_else(|| {
                        let n = self.doc.combo.name.trim();
                        (!n.is_empty()).then(|| n.to_string())
                    })
                    .unwrap_or_else(|| "project".into());
                let d = crate::files::FileDialog::new()
                    .add_filter(tr("Project"), &["json"])
                    .set_file_name(format!("{name}.json"));
                self.save_as(d, move |c, p| c.save_to(p));
                return;
            }
        };
        self.save_to(path);
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
        let ext = if self.ses.view_plane == ViewPlane::D3 {
            "step"
        } else {
            "svg"
        };
        let d = crate::files::FileDialog::new()
            .add_filter(tr("2D drawing"), &["svg", "png", "dxf"])
            .add_filter(
                tr("3D solids"),
                &["step", "stp", "glb", "gltf", "obj", "stl"],
            )
            .set_file_name(format!("{base}.{ext}"));
        self.save_as(d, |c, p| {
            let mut layout = c.overlay.req_in(c.ses.view_plane);
            c.s.rays
                .apply(&mut layout, c.n_fields(), c.waves_um().len());
            let r = c.s.rays.export_req(layout, p.display().to_string());
            c.call(Tag::LayoutExport, Call::LayoutExport(Box::new(r)));
        });
    }

    /// 値の変更を送る（計算中なら終わってからまとめて）
    /// 値の変更を送る。解析でない要求（読み込み・前の値の変更）を待っている間だけ待つ。解析が回っている間は
    /// 送れば中断する（Set は系を変える）が、動かし続けると解析が 1 つも出ないので 250 ms に 1 回まで
    fn flush_set(&mut self) {
        if self.pending_set.is_empty() || self.busy_core() {
            return;
        }
        if self.busy_short()
            && self
                .last_set
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(250))
        {
            return;
        }
        let values = std::mem::take(&mut self.pending_set).into_iter().collect();
        self.call_refreshing(Tag::Set, Some(Call::Set { values }));
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
            Kind::Layout => self.layout_ui(ui, id),
            Kind::LensData => self.lens_data_ui(ui),
            Kind::MultiConfig => self.config_ui(ui),
            Kind::Optimize => self.optimize_ui(ui),
            Kind::Swap => self.swap_ui(ui),
            Kind::Tolerance => self.tolerance_ui(ui),
            Kind::Zoom => self.zoom_ui(ui),
            Kind::History => self.history_ui(ui),
            Kind::SysSettings => self.sysset_ui(ui),
            Kind::Analysis(cfg) => self.analysis_ui(ui, id, cfg),
        }
    }

    /// レイアウト図を描き直す（重ねる構成を変えたとき）。組合せの系は下書きで
    fn request_layout(&mut self) {
        match &self.loc.combo_path {
            Some(path) => {
                let load = Box::new(self.load_req(path));
                let layout = Box::new(self.layout_params());
                self.draft_call(Tag::Preview, Call::Preview { load, layout });
            }
            _ => self.call(Tag::Layout, Call::Layout(Box::new(self.layout_params()))),
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
                r.typed = None;
                r.prev_typed = None;
            }
        }
        // 別の系なら階層・選択も戻す（組合せの作り直しは同じ系）
        if new_file != self.loc.file {
            self.p.ld.reset();
        }
        self.loc.file = new_file;
        self.sys.elem_blocks = crate::combo::block_spans(&v);
        self.sys.summary = Some(v);
        self.sys.info = None;
        self.refresh();
        self.read_params();
        // 組合せの図は下書きで作る（ここで作ると、動かしている途中の古い図に戻ることがある）。組合せの無い系（Web 版の
        // 解析のパネルの別の窓が受けた写し）だけここで
        if self.loc.combo_path.is_none() {
            self.call(Tag::Layout, Call::Layout(Box::new(self.layout_params())));
        }
        self.call(Tag::Configs, Call::Configs);
        self.rerun = Some(false);
    }

    /// 面の表と要約を 1 つの要求で読み直す（途中の状態を見せない）。系を変える要求に
    /// 付けて読み直す途中（[`Core::call_refreshing`]）なら頼まない
    fn refresh(&mut self) {
        if self.bundled_refresh {
            return;
        }
        self.calc_refreshing(Tag::Refresh, Value::Null);
    }

    /// 組合せのレイアウト図と要素の操作（id はパネル。図を画像でコピーするときの切り出しに使う）
    fn layout_ui(&mut self, ui: &mut Ui, id: u64) {
        let accent = theme::accent(&self.s.look);
        if self.loc.file.is_some() {
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label(tr("View plane"));
                for p in ViewPlane::ALL {
                    let r = ui.selectable_value(&mut self.ses.view_plane, p, p.label());
                    let r = match p {
                        ViewPlane::Unfold => r.on_hover_text(tr(
                            "Sections after mirror/BS folds laid out along the original axis (moves along the axis only)",
                        )),
                        ViewPlane::D3 => r.on_hover_text(tr(
                            "Drag to rotate, Shift- or right-drag to pan, Ctrl (⌘ on Mac) + wheel or pinch to zoom, double-click to reset",
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
                // 解析のグラフと同じコピーメニュー（図の画像と、図の元の数値）
                if let Some(d) = &self.layout_data {
                    if copy_menu(ui, &d.json, true) {
                        self.shot = Some((id, true, "layout".into()));
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                    }
                }
            });
            changed |= self.overlay.ui(ui, self.ses.view_plane.vert());
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
        let data = self.layout_data.as_ref();
        let paths: Vec<String> = self
            .overlay
            .items
            .iter()
            .filter(|x| x.0 == "paths")
            .map(|x| x.1.clone())
            .collect();
        // 面の表で選んだ面を図で目立たせる
        self.p.combo_view.hl_surf = self.p.ld.cur.map(|x| x.0);
        let busy = self.busy_short();
        let act = edit::stage(&mut self.doc.combo, &mut self.msgs, |c| {
            combo::ui(
                ui,
                c,
                data,
                &mut self.p.combo_view,
                &mut self.s.auto_build,
                busy,
                accent,
                &paths,
            )
        });
        if let Some(r) = self.p.combo_view.fig_rect {
            self.fig_rects.insert(id, r);
        }
        if let Some(a) = act {
            self.msgs.push(Msg::Combo(a));
        }
    }

    /// 組合せが変わっていて系の読み込みが空いていれば、系を作り直す。待っている解析は捨てる
    /// （作り直した後にまとめて回す）
    fn auto_build(&mut self) {
        if self.load_stale && !self.busy_core() {
            self.load_stale = false;
            if let Some(p) = self.loc.combo_path.clone() {
                self.drop_queued();
                self.open(&p);
            }
        }
        if !self.s.auto_build
            || self.preview_busy()
            || self.doc.combo.picks.is_empty()
            || self.loc.built_combo.same(&self.doc)
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
        let r = TraceReq {
            px: Some(vec![0.0]),
            py: Some(vec![1e-3]),
            hx: Some(0.0),
            hy: Some(0.0),
            last: Some(img - 1),
            ..Default::default()
        };
        self.call(Tag::Focus, Call::Trace(Box::new(r)));
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
        self.doc.insert_after = self
            .p
            .combo_view
            .sel()
            .filter(|&k| k < self.doc.combo.len());
        self.browser.lens_hide.clone_from(&self.s.lens_hide);
        // カタログから足した玉も欄の書き換えとして当てる（足した番号・像面合わせの印は文書の外の印なのでそのまま）
        let before = self.doc.combo.clone();
        let act = self.browser.ui(
            ui,
            &mut self.doc,
            self.s.glass_dirs.len(),
            self.s.lens_dirs.len(),
        );
        edit::unstage(before, &mut self.doc.combo, &mut self.msgs);
        self.s.lens_hide.clone_from(&self.browser.lens_hide);
        if let Some(k) = self.doc.added.take() {
            self.p.combo_view.select(k);
        }
        self.lens_preview();
        if let Some(a) = act {
            self.msgs.push(Msg::Catalog(a));
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
        if self.preview_busy() {
            return;
        }
        let r = &b.lenses[i];
        if !r.placeable() {
            self.browser.preview = Some((i, combo::LayoutData::default()));
            return;
        }
        let project = catalog::preview_project(r);
        let path = doc_loc::work_dir().join("catalog-preview.json");
        let ok = crate::work_fs::with(|| {
            lensbench_core::vfs::create_dir_all(path.parent().unwrap_or(&path))
                .and_then(|_| lensbench_core::vfs::write(&path, project.to_string()))
        });
        if ok.is_err() {
            return;
        }
        self.preview_req = Some(i);
        let layout = LayoutReq {
            rays: Some(5),
            ..Default::default()
        };
        self.draft_call(
            Tag::LensPreview(i),
            Call::Preview {
                load: Box::new(self.load_req(&path)),
                layout: Box::new(layout),
            },
        );
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

    /// カタログに無いガラスで開けなかったときの案内
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
        // 組合せとして開く（まだ無いガラスがあれば、系として読んだ誤りでもう一度ここへ来る）
        self.reopen_same = true;
        self.open_doc_unlisted(&dst);
    }

    /// ユーザーカタログに保存する窓。保存は読み込んだ組合せの系から（解析と同じ worker）、一覧・変更は下書きの worker
    fn cat_save_window(&mut self, ctx: &egui::Context) {
        let ready = self.loc.file.is_some() && self.loc.built_combo.same(&self.doc);
        let req = self
            .p
            .cat_save
            .window(ctx, &self.doc.combo, &self.browser.lenses, ready);
        if let Some((op, p)) = req {
            if op == CatOp::Add {
                self.call(Tag::UserCatalog(op), p);
            } else {
                self.draft_call(Tag::UserCatalog(op), p);
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
                    self.draft_call(Tag::CatalogFill, Call::CatalogFill(p));
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
            self.draft_call(Tag::CatalogFill, Call::CatalogFill(Box::new(p)));
        }
    }
}

/// フォルダの並びを編集する。変えたら true
/// 組合せの系で、面の表から変えられない値の理由（開いた系は None）
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
        // パスを打つ・貼る
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
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, args: Args) -> App {
        let font = crate::fonts::install(&cc.egui_ctx);
        crate::files::set_ctx(&cc.egui_ctx);
        #[cfg(not(target_arch = "wasm32"))]
        crate::files::set_owner(cc);
        let (s, mut ses) = load_settings(cc, &args);
        i18n::set(s.lang);
        crate::fonts::follow_lang(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, &s.look);
        s.cad_store.apply();
        let (worker, preview) = make_workers(cc);
        let lay = ses
            .layout
            .take()
            .unwrap_or_else(|| Preset::Analysis.build());
        let next_id = lay.max_id();
        let c = Core::new(&args, s, ses, (worker, preview), font, next_id);
        let mut app = App {
            c,
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
            viewer: None,
            #[cfg(target_arch = "wasm32")]
            hosts: HashMap::new(),
            #[cfg(target_arch = "wasm32")]
            calc: None,
            #[cfg(target_arch = "wasm32")]
            calc_flat: false,
        };
        app.start(cc, &args);
        app
    }

    /// 作った後: 前に閉じたときのパネルの設定を戻し、通し確認・Web 版の別の窓を始め、指定のファイルを開く
    #[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
    fn start(&mut self, cc: &eframe::CreationContext<'_>, args: &Args) {
        let app = self;
        #[cfg(target_arch = "wasm32")]
        let panel = crate::web_tabs::panel_of_location();
        // 前に閉じたときの最適化・公差・差し替えの設定
        let panels = std::mem::take(&mut app.c.ses.panels);
        app.c.set_panels(&panels);
        // レンズデータの見せ方は表の状態が持ち、保存するときだけ写す
        app.c.p.ld.tree = app.c.ses.ld_tree;
        app.c.p.ld.detail = app.c.ses.ld_detail;
        app.c.p.ld.by_surface = app.c.ses.ld_by_surface;
        app.apply_dock_translations();
        // Web 版: 別の窓のパネルなら、計算しないで元のタブで描いた画面を受けて描く
        #[cfg(target_arch = "wasm32")]
        if let Some((id, sid)) = panel {
            if crate::web_tabs::calc_panel() {
                app.start_calc(&cc.egui_ctx, id, sid);
            } else {
                app.viewer = Some(crate::web_tabs::Viewer::new(&cc.egui_ctx, id, sid));
            }
            return;
        }
        // Web 版: 別のタブ・窓に出していたパネルはドックに戻す（タブ・窓はクリックしないと開けない）
        #[cfg(target_arch = "wasm32")]
        if args.tour.is_none() {
            app.tabs = Some(crate::web_tabs::Tabs::new(&cc.egui_ctx));
            for w in std::mem::take(&mut app.windows) {
                crate::layout::redock(&mut app.dock, w.pane, w.home.as_ref());
            }
        }
        // 通し確認で系のファイルを渡さなければ、exe に埋めた例の系（ズームの例）を開く（配る版の確認）
        let tour_open = args.tour.as_ref().map(|d| {
            let _ = lensbench_core::vfs::create_dir_all(d);
            app.tour = Some(Tour::new(d.clone(), args.tour_timeout));
            crate::tour::zoom_sample(d)
        });
        let tour_open = tour_open.filter(|_| args.open.is_none());
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
        if let Some(p) = reopen
            .as_ref()
            .or(args.open.as_ref())
            .or(tour_open.as_ref())
        {
            app.c.open_doc(p);
        }
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
            .chain(
                self.windows
                    .iter()
                    .map(|w| &w.pane)
                    .filter(|p| !self.calc_window(p.id)),
            )
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
        // いちばん広い葉にタブで足す（フォーカスのある小さな所に入って図が潰れない）
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
    /// 画面の写しが届いたら、頼んだ図の所を切り出して写す・PNG で保存する
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
        let d = crate::files::FileDialog::new()
            .add_filter("PNG", &["png"])
            .set_file_name(format!("{cmd}.png"));
        self.c.save_as(d, move |c, p| {
            match crate::image_util::png_bytes(&part)
                .and_then(|b| lensbench_core::vfs::write(&p, b).map_err(|e| e.to_string()))
            {
                Ok(()) => c.info(trf("Saved to {}", &[&p.display()])),
                Err(e) => c.fail(format!("{}: {e}", p.display())),
            }
        });
    }

    /// 窓を閉じる前に、保存していない変更があれば「保存 / 保存しない / やめる」を聞く
    fn close_guard(&mut self, ctx: &egui::Context) {
        if cfg!(target_arch = "wasm32") || self.tour.is_some() {
            return;
        }
        // 未保存の変更は版で見るので、この画面で変えた分も数える
        self.c.doc.sync();
        if ctx.input(|i| i.viewport().close_requested()) && !self.c.close_ok && self.c.close_loses()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.c.close_ask = true;
        }
        // 保存してから閉じる: 保存が終わって変更が無くなったら閉じる（失敗したらやめる）
        if self.c.close_pending {
            if self.c.status.error {
                self.c.close_pending = false;
            } else if !self.c.dirty() && !self.c.busy() {
                self.c.close_pending = false;
                self.c.close_ok = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if !self.c.busy() && !self.c.saving() {
                // 保存の窓をやめた
                self.c.close_pending = false;
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
                // 保存の窓を選び終わるのと、開いた系の保存が計算のスレッドで終わるのを待つ（窓をやめたら閉じない）
                self.c.close_pending = true;
            }
            Some(1) => {
                self.c.close_ask = false;
                self.c.close_ok = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(2) => {
                // 保存して閉じるのを待っている間にやめた: 保存が終わっても閉じない
                self.c.close_ask = false;
                self.c.close_pending = false;
            }
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
        #[cfg(target_arch = "wasm32")]
        if self.calc.is_some() {
            self.calc_frame(ctx);
            return self.flush_fx(ctx);
        }
        self.logic(ctx);
        self.c.share_glasses(ctx);
        self.handle_input(ctx);
        self.sync_window_chrome(ctx);
        self.show_panels(ctx);
        self.handle_late_input(ctx);
        self.show_dialogs(ctx);
        self.tour_frame(ctx);
        self.flush_fx(ctx);
    }

    /// Web 版は設定（最近のファイル・並びなど）をこまめに残す。eframe 0.31 の Web 版はページを閉じるときに
    /// 保存しない（"onbeforeunload" で待つので呼ばれない）
    #[cfg(target_arch = "wasm32")]
    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(2)
    }

    #[cfg(target_arch = "wasm32")]
    fn persist_egui_memory(&self) -> bool {
        self.viewer.is_none() && self.calc.is_none()
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.tour.is_some() {
            return;
        }
        // 別のタブのパネルは元のタブの設定を上書きしない
        #[cfg(target_arch = "wasm32")]
        if self.viewer.is_some() || self.calc.is_some() {
            return;
        }
        // 好み（設定）と、開いていた文書と画面の様子（Session）を別の鍵に書く
        let mut ses = Session {
            combo: self.c.doc.combo.clone(),
            layout: Some(self.layout()),
            params: self.c.ses.params.clone(),
            view_plane: self.c.ses.view_plane,
            ld_tree: self.c.p.ld.tree,
            ld_detail: self.c.p.ld.detail,
            ld_by_surface: self.c.p.ld.by_surface,
            panels: self.c.panels_value(),
        };
        // ポータブル: 設定のフォルダの中のパスは相対で書く
        if let Some(b) = crate::portable::base() {
            let keep = self.c.s.paths();
            self.c.s.map_paths(|p| crate::portable::to_stored(p, b));
            eframe::set_value(storage, eframe::APP_KEY, &self.c.s);
            self.c.s.set_paths(keep);
            ses.combo
                .map_paths(&|p: &str| crate::portable::to_stored(p, b));
        } else {
            eframe::set_value(storage, eframe::APP_KEY, &self.c.s);
        }
        eframe::set_value(storage, SESSION_KEY, &ses);
    }
}
/// レイアウト図に重ねる構成（.zmx の多重構成・プロジェクトの状態・BS の経路）
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Overlay {
    /// 選べる構成（serve の layout の鍵 cfgs・states・paths と名前）と、重ねるか
    pub items: Vec<(&'static str, String, bool)>,
    /// 構成 k を k 倍ずらす量 [dz, dy] mm
    pub offset: [f64; 2],
}

impl Overlay {
    /// serve の configs の結果から。前に選んでいた名前は選んだまま
    /// 選べる構成を読み直す。選んでいたものが無くなったら true（レイアウトを頼み直す）
    pub(crate) fn update(&mut self, v: &Value) -> bool {
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

    /// [`Self::req`] に見る面を足したもの。ずらす量の縦は見る面の縦（xz 面なら x）
    pub(crate) fn req_in(&self, plane: ViewPlane) -> LayoutReq {
        let mut r = self.req();
        if plane == ViewPlane::Xz && !r.offset.is_empty() {
            r.offset = vec![self.offset[1], 0.0, self.offset[0]];
        }
        r.plane = plane.view();
        r.unfold = plane == ViewPlane::Unfold;
        r
    }

    /// serve の layout の要求（何も選ばなければ今の構成だけ、見る面は yz）
    pub(crate) fn req(&self) -> LayoutReq {
        let on = |key: &str| -> Option<String> {
            let v: Vec<String> = self
                .items
                .iter()
                .filter(|x| x.0 == key && x.2)
                .map(|x| {
                    if key == "cfgs" {
                        x.1.parse::<u64>().unwrap_or(1).to_string()
                    } else {
                        x.1.clone()
                    }
                })
                .collect();
            (!v.is_empty()).then(|| v.join(","))
        };
        let mut r = LayoutReq {
            cfgs: on("cfgs"),
            states: on("states"),
            beam_paths: on("paths"),
            ..Default::default()
        };
        let any = r.cfgs.is_some() || r.states.is_some() || r.beam_paths.is_some();
        if any && self.offset != [0.0; 2] {
            r.offset = vec![0.0, self.offset[1], self.offset[0]];
        }
        r
    }

    /// 選ぶ欄（選べる構成が無ければ出さない）。変えたら true。vert は図の縦の軸の名前
    pub(crate) fn ui(&mut self, ui: &mut Ui, vert: &str) -> bool {
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
                        .add(crate::widgets::Num::new(&mut self.offset[i]).suffix(" mm"))
                        .changed();
                }
            }
        });
        changed
    }
}

/// 重ねる構成: configs の結果から欄を作り、選んだものだけ layout の要求に。読み直しても選んだまま
#[test]
fn overlay_params() {
    let mut o = Overlay::default();
    o.update(&json!({ "cfgs": 1, "states": ["A"], "paths": ["TT", "RR"] }));
    assert_eq!(o.items.len(), 2);
    assert_eq!(o.req(), LayoutReq::default());
    o.items[1].2 = true;
    o.offset = [30.0, 0.0];
    assert_eq!(
        o.req(),
        LayoutReq {
            beam_paths: Some("RR".into()),
            offset: vec![0.0, 0.0, 30.0],
            ..Default::default()
        }
    );
    o.update(&json!({ "cfgs": 1, "paths": ["RR", "TT", "TR"] }));
    assert_eq!(o.items.iter().filter(|x| x.2).count(), 1);
    assert_eq!(o.items[0].1, "RR");
    o.update(&json!({ "cfgs": 3 }));
    o.items[0].2 = true;
    o.items[2].2 = true;
    o.offset = [0.0; 2];
    let cfgs = LayoutReq {
        cfgs: Some("1,3".into()),
        ..Default::default()
    };
    assert_eq!(o.req(), cfgs);
    // xz 面: plane を足し、ずらす量の縦は x
    o.offset = [10.0, 5.0];
    use lensbench_engine::layout::View;
    let r = o.req_in(ViewPlane::Xz);
    assert_eq!(
        (r.offset.as_slice(), r.plane),
        (&[5.0, 0.0, 10.0][..], View::XZ)
    );
    assert_eq!(o.req_in(ViewPlane::Yz), o.req());
    // 展開: yz のまま unfold、3D は plane
    let r = o.req_in(ViewPlane::Unfold);
    assert_eq!(
        (r.offset.as_slice(), r.plane, r.unfold),
        (&[0.0, 5.0, 10.0][..], View::YZ, true)
    );
    assert_eq!(o.req_in(ViewPlane::D3).plane, View::D3);
    // 設定に保存する名前
    assert_eq!(serde_json::to_value(ViewPlane::D3).unwrap(), json!("3d"));
    assert_eq!(
        serde_json::to_value(ViewPlane::Unfold).unwrap(),
        json!("unfold")
    );
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

/// 見本の組合せ（examples/combos。Web 版の「見本」に出る）
#[cfg(test)]
fn sample_combos() -> Vec<(&'static str, Combo)> {
    use crate::sysset::{ApertureKind, FieldKind, FieldPt};
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
        aperture: ApertureKind::Na,
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
        let want =
            lensbench_project::schema::pretty_with_schema(&doc_file::project_file(&c), "project")
                + "\n";
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

impl Core {
    /// 設定・開いていた文書と計算の順番待ちから作る（パネルの設定は [`App::start`] が戻す）
    fn new(
        args: &Args,
        s: Settings,
        mut ses: Session,
        (worker, preview): (Worker, Worker),
        font: Option<String>,
        next_id: u64,
    ) -> Core {
        let browser = Browser::default();
        let doc = crate::combo_doc::ComboDoc::new(std::mem::take(&mut ses.combo));
        // 系のファイルを指定して開いたときは、前の組合せを作り直さない（指定が無ければ前の組合せで系を作る）
        let built_combo = args.open.is_some().then(|| doc.combo.clone());
        Core {
            s,
            ses,
            bundled_refresh: false,
            samples: menus::samples(args),
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
            ld_glasses: Default::default(),
            close_ask: false,
            combo_census: None,
            missing_glass: None,
            status_log: Vec::new(),
            status_hint: None,
            last_set: None,
            shot: None,
            fig_rects: HashMap::new(),
            status_log_open: false,
            close_ok: false,
            close_pending: false,
            auto_fill: Default::default(),
            swap_ids: Vec::new(),
            rev: 0,
            param_sys_cache: Default::default(),
            field_labels_cache: Default::default(),
            sys_gen: 0,
            saves: Default::default(),
            browser,
            doc,
            folders_open: false,
            thermal: None,
            help_open: false,
            save_layout_open: false,
            ask: None,
            drawing_in: None,
            layout_name: String::new(),
            loc: DocLoc {
                built_combo: Snap::new(built_combo),
                ..Default::default()
            },
            _work: doc_loc::WorkDir::new(),
            layout_data: None,
            overlay: Overlay::default(),
            rays_open: false,
            folders: Default::default(),
            reopen_same: false,
            font,
            next_id,
            msgs: Vec::new(),
            rec: None,
            replay: None,
            fails: 0,
            fx: Default::default(),
            popup_doc: Default::default(),
        }
    }
}

/// 保存した好みと開いていた文書（通し確認では使わない）に、引数のカタログのフォルダを足す
fn load_settings(cc: &eframe::CreationContext<'_>, args: &Args) -> (Settings, Session) {
    // 通し確認では保存した設定を使わない（引数のフォルダだけ）
    let mut s: Settings = if args.tour.is_some() {
        Settings::default()
    } else {
        cc.storage
            .and_then(|st| eframe::get_value(st, eframe::APP_KEY))
            .unwrap_or_default()
    };
    // 開いていた文書と画面の様子。前の版は好みと同じ値に入れていたので、鍵が無ければそこから一度だけ読む
    // （同じ欄の名前。次に保存すると分かれる。この読み替えはほかの移行と一緒に消す）
    let mut ses: Session = if args.tour.is_some() {
        Session::default()
    } else {
        cc.storage
            .and_then(|st| {
                eframe::get_value(st, SESSION_KEY)
                    .or_else(|| eframe::get_value(st, eframe::APP_KEY))
            })
            .unwrap_or_default()
    };
    // Web 版: 解析のパネルを出した別の窓は、開いていた文書・並びを使わない（元のタブから受ける）
    #[cfg(target_arch = "wasm32")]
    if crate::web_tabs::calc_panel() {
        ses = Session::default();
    }
    if args.tour.is_some() {
        crate::tour::prepare(args, &mut s);
    } else {
        s.resolve_paths();
        ses.resolve_paths();
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
    (s, ses)
}

/// 計算の順番待ち（本体と下書き）
fn make_workers(cc: &eframe::CreationContext<'_>) -> (Worker, Worker) {
    // Web 版: 別のタブのパネル（?panel=）は計算の Web Worker を作らない（解析のパネルは自分で計算するので作る）
    #[cfg(target_arch = "wasm32")]
    if crate::web_tabs::panel_of_location().is_some() && !crate::web_tabs::calc_panel() {
        return (Worker::idle(), Worker::idle());
    }
    let ctx = cc.egui_ctx.clone();
    let ctx2 = cc.egui_ctx.clone();
    (
        Worker::new(None, move || ctx.request_repaint()),
        Worker::preview(move || ctx2.request_repaint()),
    )
}

#[cfg(test)]
mod session_tests {
    use super::*;

    /// 好みと開いていた様子は別の値: 好みに組合せ・並びが入らず、Session は読み直して同じ
    #[test]
    fn gar17_settings_and_session_are_separate() {
        let mut ses = Session {
            params: vec!["1.t".into()],
            view_plane: ViewPlane::Xz,
            ..Session::default()
        };
        ses.combo.insert_plane(0);
        let t = ron::to_string(&ses).unwrap();
        let back: Session = ron::from_str(&t).unwrap();
        assert_eq!(back.combo, ses.combo);
        assert_eq!(back.params, ses.params);
        assert_eq!(back.view_plane, ViewPlane::Xz);
        let st = ron::to_string(&Settings::default()).unwrap();
        for k in ["combo", "layout:", "params", "panels", "locked", "ld_tree"] {
            assert!(!st.contains(k), "{k} in {st}");
        }
    }

    /// 前の版の設定（好みと様子が 1 つの値）を Session として読める（一度だけの読み替え）
    #[test]
    fn gar17_old_settings_read_as_session() {
        let ses = Session {
            params: vec!["G1.shift".into()],
            ld_tree: false,
            ..Session::default()
        };
        // 前の版の形: 好みの欄と様子の欄が同じ構造体に並ぶ（eframe は RON の構造体で書く）
        let a = ron::to_string(&Settings::default()).unwrap();
        let b = ron::to_string(&ses).unwrap();
        let old = format!("({},{})", &a[1..a.len() - 1], &b[1..b.len() - 1]);
        let back: Session = ron::from_str(&old).unwrap();
        assert_eq!(back.params, ses.params);
        assert!(!back.ld_tree);
    }
}
