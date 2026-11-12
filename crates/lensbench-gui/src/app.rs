//! 窓の骨格: 上の帯（ファイル・探す・計算）、パネル（ドッキング・タブ・フロート・別の OS の窓）、
//! 下の状態の行（焦点距離・F 値など）。
//!
//! 計算はすべて serve と同じ method（load・analyze・get・set・write・trace・glasses・lenses）を [`Worker`] に頼む。
//! 系のブロック編集とレイアウト図は、系の編集・保存形式の上にパネルとして足す。

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use egui::{Color32, Key, Modifiers, RichText, Stroke, Ui};
use egui_dock::{DockArea, NodeIndex, SurfaceIndex, TabViewer};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::argform;
use crate::catalog::{self, Browser, Combo};
use crate::catsave::CatSave;
use crate::combo;
use crate::config_panel;
use crate::figures;
use crate::i18n::{self, tr, trf, Lang};
use crate::json_view::{self, fmt_num};
use crate::layout::{self, AnaCfg, Kind, Layout, OsWin, Pane, Preset, View};
use crate::lens_data;
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
    pub cad_store: CadStore,
}

/// 品の STEP の共有の置き場（`lensbench_cli::cad::Store`）。取るときは配布元より先に見て、配布元から取った STEP を上げる
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CadStore {
    /// 口の URL（例 `https://<プロジェクト>.pages.dev/cad`）
    pub url: String,
    /// Cloudflare Access のサービストークン（Client ID・Client Secret）。空なら送らない
    pub id: String,
    pub secret: String,
}

impl CadStore {
    /// 取るときに使う置き場に決める（アプリ版。URL が空なら環境変数か組み込みの置き場）
    pub fn apply(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        lensbench_cli::cad::set_store(Some(lensbench_cli::cad::Store {
            url: self.url.trim().to_string(),
            id: self.id.trim().to_string(),
            secret: self.secret.trim().to_string(),
        }));
    }
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
            ViewPlane::Unfold => tr("展開"),
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
            ld_tree: false,
            ld_detail: false,
            ld_by_surface: false,
            rays: Default::default(),
            lens_hide: Vec::new(),
            os_titlebar: false,
            cad_store: CadStore::default(),
        }
    }
}

#[derive(Default)]
struct Status {
    text: String,
    error: bool,
}

/// 解析パネルの結果
#[derive(Default)]
struct AnaRes {
    cmd: String,
    value: Option<Value>,
    /// 値を変える前の結果（同じ解析のとき）
    prev: Option<Value>,
    ms: f64,
    rays: Vec<SpotRays>,
    error: Option<String>,
}

/// 組合せの元に戻す・やり直す段（組合せの系の変更は GUI の組合せのデータなので、ここで積む）。段ごとに変更の名前
#[derive(Default)]
struct ComboHist {
    undo: Vec<(Combo, String)>,
    redo: Vec<(Combo, String)>,
    /// 最後に段に積んだ（落ち着いた）組合せ
    last: Option<Combo>,
    /// 前の画面の組合せと、最後に変わった時刻（入力・ドラッグの途中は積まない）
    seen: Option<Combo>,
    changed_at: Option<web_time::Instant>,
}

/// 元に戻せる段の数
const UNDO_DEPTH: usize = 100;

/// 新規の前の状態（新規を元に戻す。serve には前の系が残っているので、GUI の状態だけ戻す）
struct NewSnap {
    combo: Combo,
    combo_hist: ComboHist,
    file: Option<PathBuf>,
    summary: Option<Value>,
    built: Option<Value>,
    built_combo: Option<Combo>,
    combo_path: Option<PathBuf>,
    doc_path: Option<PathBuf>,
    ld_dirty: bool,
    hist: (usize, usize),
    hist_list: (Vec<Value>, Vec<Value>),
}

/// serve の段の変更の名前（set はパラメータの説明、edit は操作）
fn hist_label(v: &Value) -> String {
    if let Some(f) = v["open"].as_str() {
        return trf("{} を開く", &[&f]);
    }
    if let Some(f) = v["reload"].as_str() {
        return trf("{} を読み直す", &[&f]);
    }
    if let Some(names) = v["set"].as_array() {
        let d: Vec<String> = names
            .iter()
            .filter_map(Value::as_str)
            .map(|n| {
                let d = crate::param_pick::param_desc(n);
                if d.is_empty() {
                    n.to_string()
                } else {
                    d
                }
            })
            .collect();
        return match d.len() {
            0 => tr("値の変更").into(),
            1..=2 => d.join("、"),
            n => trf("{} ほか {} 個", &[&d[0], &(n - 1)]),
        };
    }
    let ops = v["edit"].as_array().cloned().unwrap_or_default();
    let one = |o: &Value| -> String {
        let s = o["surface"].as_u64().map(|x| x.to_string());
        let op = o["op"].as_str().unwrap_or_default();
        let what = match op {
            "insert_surface" => tr("面を挿入"),
            "delete_surface" => tr("面を削除"),
            "set_surface" | "set_surface_data" => tr("面の値"),
            "set_surface_type" => tr("面の種類"),
            "set_material" => tr("硝材"),
            "set_stop" => tr("絞り"),
            "add_pickup" | "remove_pickup" => tr("ピックアップ"),
            "add_solve" | "remove_solve" => tr("ソルブ"),
            "set_system_data" => tr("系の設定"),
            "set_param" => tr("値の変更"),
            "define_unit" | "set_unit" | "remove_unit" => tr("玉・群"),
            "add_mech" | "set_mech" | "remove_mech" => tr("機構部品"),
            "set_config" => tr("構成の切り替え"),
            o if o.contains("config") => tr("多重構成"),
            _ => tr("編集"),
        };
        // 面の値の 1 項目なら、その項目の名前（面 8 の厚さ など）
        if let (Some(s), Some([f])) = (&s, o["fields"].as_array().map(Vec::as_slice)) {
            let short = match f.as_str().unwrap_or_default() {
                "thickness" | "t" => "t",
                "radius" | "r" => "r",
                "curv" | "c" => "c",
                "conic" | "k" => "k",
                _ => "",
            };
            if !short.is_empty() {
                return crate::param_pick::param_desc(&format!("{s}.{short}"));
            }
        }
        match s {
            Some(s) => trf("{}（面 {}）", &[&what, &s]),
            None => what.to_string(),
        }
    };
    match ops.len() {
        0 => tr("編集").into(),
        1 => one(&ops[0]),
        n => trf("{} ほか {} 件", &[&one(&ops[0]), &(n - 1)]),
    }
}

/// 組合せの変更の名前（要素の追加・削除・どの要素か・系の設定）
fn combo_change(a: &Combo, b: &Combo) -> String {
    use std::cmp::Ordering;
    match b.picks.len().cmp(&a.picks.len()) {
        Ordering::Greater => {
            let k = (0..a.picks.len())
                .find(|&k| a.picks[k] != b.picks[k])
                .unwrap_or(a.picks.len());
            return trf("要素を足す（{}）", &[&b.picks[k].label()]);
        }
        Ordering::Less => {
            let k = (0..b.picks.len())
                .find(|&k| a.picks[k] != b.picks[k])
                .unwrap_or(b.picks.len());
            return trf("要素を消す（{}）", &[&a.picks[k].label()]);
        }
        Ordering::Equal => {}
    }
    let diff: Vec<usize> = (0..a.picks.len())
        .filter(|&k| a.picks[k] != b.picks[k])
        .collect();
    match diff.as_slice() {
        [] => tr("組合せの設定").into(),
        [k] => trf("要素 {} を変える（{}）", &[&(k + 1), &b.picks[*k].label()]),
        _ => trf("{} 個の要素を変える", &[&diff.len()]),
    }
}

impl ComboHist {
    /// 画面ごとに呼ぶ。組合せが変わって落ち着いたら（ポインタを離し、入力欄を離れるか 0.8 秒たったら）前を積む
    fn track(&mut self, cur: &Combo, busy: bool) {
        if self.seen.as_ref() != Some(cur) {
            self.seen = Some(cur.clone());
            self.changed_at = Some(web_time::Instant::now());
        }
        let Some(last) = &self.last else {
            self.last = Some(cur.clone());
            return;
        };
        if last == cur {
            return;
        }
        let quiet = self
            .changed_at
            .is_some_and(|t| t.elapsed().as_secs_f64() > 0.8);
        if !busy || quiet {
            self.commit(cur);
        }
    }

    /// 前の落ち着いた組合せを積む
    fn commit(&mut self, cur: &Combo) {
        if let Some(last) = self.last.replace(cur.clone()) {
            if &last != cur {
                let what = combo_change(&last, cur);
                self.undo.push((last, what));
                if self.undo.len() > UNDO_DEPTH {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
        }
    }

    fn can(&self, cur: &Combo, back: bool) -> bool {
        if back {
            !self.undo.is_empty() || self.last.as_ref().is_some_and(|l| l != cur)
        } else {
            !self.redo.is_empty()
        }
    }

    /// 段の名前（元に戻す段は新しい順、やり直す段は次の順）
    fn list(&self, back: bool) -> Vec<String> {
        let v = if back { &self.undo } else { &self.redo };
        v.iter().rev().map(|x| x.1.clone()).collect()
    }

    /// n 段戻す（back）・やり直す。戻した後の組合せ
    fn step(&mut self, cur: &Combo, back: bool, n: usize) -> Option<Combo> {
        self.commit(cur);
        let mut c = cur.clone();
        let mut moved = false;
        for _ in 0..n.max(1) {
            let (from, to) = if back {
                (&mut self.undo, &mut self.redo)
            } else {
                (&mut self.redo, &mut self.undo)
            };
            let Some((x, what)) = from.pop() else {
                break;
            };
            to.push((std::mem::replace(&mut c, x), what));
            moved = true;
        }
        if !moved {
            return None;
        }
        self.last = Some(c.clone());
        self.seen = Some(c.clone());
        Some(c)
    }
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
}

/// パネルの外の状態（パネルの中から触る）
struct Core {
    s: Settings,
    /// ファイルメニューの「見本」（Web 版の同梱の系）
    samples: Vec<String>,
    /// 硝材のフォルダ（の並び）と、そこにある AGF の名前（系の設定の硝材カタログの候補）
    agf_names: (Vec<String>, Vec<String>),
    worker: Worker,
    /// 組合せのレイアウト図だけを先に作る（解析の順番を待たない）
    preview: Worker,
    /// 系の読み込みを待っている間に組合せが変わった（読み込みが空いたら最新を読む）
    load_stale: bool,
    /// カタログの玉の断面を頼んだ行
    preview_req: Option<usize>,
    status: Status,
    /// 読み込んだ系のファイル
    file: Option<PathBuf>,
    /// load の結果（系の概要）
    summary: Option<Value>,
    /// info の結果
    info: Option<Value>,
    results: HashMap<u64, AnaRes>,
    /// パラメータの今の値
    param_vals: HashMap<String, f64>,
    new_param: String,
    /// まだ送っていない値の変更（計算中に動かした分はまとめて送る）
    pending_set: std::collections::BTreeMap<String, f64>,
    /// 開いている解析をすべて計算し直す（true は live の解析だけ）
    rerun: Option<bool>,
    browser: Browser,
    folders_open: bool,
    help_open: bool,
    save_layout_open: bool,
    layout_name: String,
    /// 最後に組合せから作ったプロジェクト
    built: Option<Value>,
    /// 今の系（組合せ）を保存するファイル（開いた・保存したプロジェクトの .json。Ctrl+S の書き先）
    doc_path: Option<PathBuf>,
    /// 最後に系を作った組合せ（今の組合せと違えば作り直す）
    built_combo: Option<Combo>,
    /// 組合せの系のファイル（開いている系が組合せか）
    combo_path: Option<PathBuf>,
    /// 組合せのレイアウト図（serve の layout）
    layout_data: Option<combo::LayoutData>,
    /// レイアウト図に重ねる構成
    overlay: Overlay,
    /// レイアウトの光線・書き出しの設定の欄を次の描画で開く（通し確認）
    rays_open: bool,
    combo_view: combo::View,
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
    /// 開いた系の元に戻せる段・やり直せる段の数（serve の set・edit・undo・redo の結果）
    hist: (usize, usize),
    /// その段の変更（serve の undo_list・redo_list）
    hist_list: (Vec<Value>, Vec<Value>),
    /// 組合せの元に戻す・やり直す
    combo_hist: ComboHist,
    /// 新規の前の状態（新規を元に戻す）と、新規をやり直せるか
    new_undo: Option<NewSnap>,
    /// 新規を戻したときの serve のやり直す段の数（それより下の段は新規の前のもので、やり直さない）
    new_redo: Option<usize>,
    /// serve のやり直しの後に新規をやり直す（一覧で新規より先まで選んだ）
    redo_new_pending: bool,
    /// 面の表: 状態、面の値、玉・群、木
    ld: lens_data::LensData,
    ld_rows: Vec<lens_data::Row>,
    ld_units: Vec<lens_data::Unit>,
    /// 開いた系の玉・群・機構部品の欄
    um: crate::unit_mech::Edits,
    /// 玉・群・機構部品の欄を開く（通し確認）
    um_open: bool,
    ld_blocks: Vec<lens_data::Block>,
    /// レンズデータの硝材の候補（ガラスカタログを読んだときに作る）
    ld_glasses: Vec<lens_data::GlassChoice>,
    /// 開いた系を面の表で変えて、まだ保存していない
    ld_dirty: bool,
    /// 多重構成の表
    mc: config_panel::State,
    /// 開いた系の系の設定
    oss: open_sysset::State,
    /// 組合せの要素をユーザーカタログに保存する窓
    cat_save: CatSave,
    /// 未完のカタログ品を補う窓
    fill: crate::fill::FillWin,
    /// 未完のカタログ品を、行を選んだら裏で補う
    auto_fill: crate::fill::AutoFill,
    /// 最適化のパネル
    opt: crate::optim_panel::OptPanel,
    /// 差し替え探索のパネルと、回したときの組合せの id
    swap: crate::swap_panel::SwapPanel,
    swap_ids: Vec<String>,
    /// 開いたプロジェクトの玉・間隔（ファイルと、その中の候補）
    swap_file: Option<(PathBuf, crate::swap_panel::Cands)>,
    /// モンテカルロ公差のパネル
    tol: crate::tol_panel::TolPanel,
    zoom: crate::zoom_panel::ZoomPanel,
    /// 結果・系の概要を受け取った回数（Web 版の別のタブに送り直す目安）
    rev: u64,
    /// Web 版の別のタブのパネル: 計算は元のタブに頼む（Some(頼むか)）。None はふつうのパネル
    relay: Option<bool>,
    /// 元のタブが計算中か（別のタブのパネル）
    relay_busy: bool,
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
    /// Web 版: 別のタブに出した解析のパネル（元のタブ）
    #[cfg(target_arch = "wasm32")]
    tabs: Option<crate::web_tabs::Tabs>,
    /// Web 版: このタブが別のタブのパネルなら、その口と、届いたパネル
    #[cfg(target_arch = "wasm32")]
    child: Option<(crate::web_tabs::Child, Option<Pane>)>,
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

/// フォルダの並びにある AGF の名前（拡張子なし、重なりは前のもの、名前の順）
fn agf_names(dirs: &[String]) -> Vec<String> {
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
fn agf_names_of_test_glass() {
    let d = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/glass").to_string();
    let v = agf_names(&[d.clone(), d]);
    assert!(v.iter().any(|x| x == "SCHOTT"), "{v:?}");
    let mut w = v.clone();
    w.dedup();
    assert_eq!(v, w);
}

/// 組合せを同梱したプロジェクト（`gui.combo` に組合せ。系の計算は残りのプロジェクトで、CLI でも開ける）
fn project_with_combo(c: &Combo) -> Value {
    let mut v = c.project();
    if let Ok(cv) = serde_json::to_value(c) {
        v["gui"] = json!({ "combo": cv });
    }
    v
}

/// プロジェクトに同梱した組合せ
fn combo_of_project(v: &Value) -> Option<Combo> {
    serde_json::from_value(v.get("gui")?.get("combo")?.clone()).ok()
}

impl Core {
    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// 利用者が選んだ系のファイルを開く（最近のファイルに足す）。組合せを同梱したプロジェクト（`gui.combo`）は
    /// 組合せに戻して系を作る（#129）
    fn open_doc(&mut self, p: &Path) {
        let file = p.display().to_string();
        self.s.recent.retain(|x| x != &file);
        self.s.recent.insert(0, file.clone());
        self.s.recent.truncate(10);
        let is_json = p
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"));
        let v = is_json
            .then(|| lensbench_core::vfs::read(p).ok())
            .flatten()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        {
            if let Some(c) = v.as_ref().and_then(combo_of_project) {
                if !self.browser.combo.picks.is_empty()
                    && self.browser.combo != c
                    && self.built_combo.as_ref() != Some(&self.browser.combo)
                    && !crate::files::confirm(
                        tr("開く"),
                        tr("今の組合せを消して、ファイルの組合せを開きますか？"),
                    )
                {
                    return;
                }
                self.browser.combo = c;
                // 別の組合せ: 元に戻す段は空から
                self.combo_hist = Default::default();
                self.combo_view = combo::View::default();
                self.doc_path = Some(p.to_path_buf());
                self.build_combo(self.browser.combo.project());
                return;
            }
        }
        // 系ファイル（面の並び）だけはその場に上書きする。ブロックで書いたプロジェクトは、保存すると面の並びに
        // なるので書き先を選ばせる
        self.doc_path = v
            .is_some_and(|v| v.get("blocks").is_none())
            .then(|| p.to_path_buf());
        self.open(p);
    }

    /// 系のファイル（.zmx・プロジェクトの .json）を読む
    fn open(&mut self, p: &Path) {
        let file = p.display().to_string();
        let mut params = self.load_params(p);
        // 開く前の系は serve の段に積む（開くを元に戻せる）。組合せの作り直しは組合せの段で戻すので積まない
        params["history"] = json!(
            if self.is_combo() && self.combo_path.as_deref() == Some(p) {
                "keep"
            } else {
                "push"
            }
        );
        self.worker.send(Tag::Load, "load", params);
        self.status = Status {
            text: trf("{} を読んでいます…", &[&file]),
            error: false,
        };
    }

    fn load_params(&self, p: &Path) -> Value {
        let mut v = json!({
            "file": p.display().to_string(),
            "glass_dir": dirs(&self.s.glass_dirs),
            "lens_catalog": dirs(&self.s.lens_dirs),
        });
        // 組合せの今の状態
        if self.combo_path.as_deref() == Some(p) {
            if let Some(st) = self.browser.combo.active_state() {
                v["state"] = json!(st);
            }
        }
        v
    }

    /// 新規: 組合せと開いている系を空にする（前の状態は新規を元に戻す段に。確かめずに空にし、元に戻せる）
    fn new_system(&mut self) {
        // すでに空なら何もしない（新規の前の状態を空で上書きしない）
        if self.summary.is_none() && self.browser.combo.picks.is_empty() {
            return;
        }
        self.new_undo = Some(NewSnap {
            combo: std::mem::take(&mut self.browser.combo),
            combo_hist: {
                // 新規の前のやり直す段は捨てる（新規の後にやり直すと順がおかしくなる）
                let mut h = std::mem::take(&mut self.combo_hist);
                h.redo.clear();
                h
            },
            file: self.file.take(),
            summary: self.summary.take(),
            built: self.built.take(),
            built_combo: self.built_combo.take(),
            combo_path: self.combo_path.take(),
            doc_path: self.doc_path.take(),
            ld_dirty: self.ld_dirty,
            hist: std::mem::take(&mut self.hist),
            hist_list: std::mem::take(&mut self.hist_list),
        });
        self.new_redo = None;
        self.combo_view = combo::View::default();
        self.summary = None;
        self.info = None;
        self.rev += 1;
        self.built = None;
        self.built_combo = None;
        self.combo_path = None;
        self.doc_path = None;
        self.layout_data = None;
        self.overlay = Overlay::default();
        self.load_stale = false;
        self.ld.reset();
        self.ld_rows.clear();
        self.ld_units.clear();
        self.ld_blocks.clear();
        self.ld_dirty = false;
        self.mc = Default::default();
        self.oss = Default::default();
        self.param_vals.clear();
        self.pending_set.clear();
        for r in self.results.values_mut() {
            r.value = None;
            r.prev = None;
            r.error = None;
        }
        self.status = Status {
            text: tr("新規").to_string(),
            error: false,
        };
    }

    /// 読み直す。開いた系は前の系を段に積む（読み直すを元に戻せる）。組合せの系は組合せから作り直す
    fn reload(&mut self) {
        let Some(f) = self.file.clone() else {
            return;
        };
        if self.is_combo() {
            self.open(&f);
            return;
        }
        self.worker
            .send(Tag::Load, "reload", json!({ "history": "push" }));
        self.status = Status {
            text: trf("{} を読んでいます…", &[&f.display()]),
            error: false,
        };
    }

    /// 新規を元に戻す（back）・やり直す
    fn undo_new(&mut self, back: bool) {
        if !back {
            self.new_system();
            self.status = Status {
                text: tr("やり直した").into(),
                error: false,
            };
            return;
        }
        let Some(n) = self.new_undo.take() else {
            return;
        };
        self.browser.combo = n.combo;
        self.combo_hist = n.combo_hist;
        self.combo_len_prev = self.browser.combo.picks.len();
        self.built = n.built;
        self.built_combo = n.built_combo;
        self.combo_path = n.combo_path;
        self.doc_path = n.doc_path;
        self.hist = n.hist;
        self.hist_list = n.hist_list;
        self.new_redo = Some(self.hist.1);
        if let Some(sum) = n.summary {
            // serve には新規の前の系が残っている（読み直さずに表・解析を出し直す）
            self.on_loaded(sum);
            self.file = n.file;
            self.ld_dirty = n.ld_dirty;
        }
        self.status = Status {
            text: tr("元に戻した").into(),
            error: false,
        };
    }

    /// 系のファイルを選んで開く。start のフレームで選び始める（Web 版は選び終わった後のフレームで開くので、
    /// 毎フレーム start = false で呼ぶ）
    fn pick_and_open_when(&mut self, start: bool) {
        if let Some(p) = crate::files::open("open-system", start, || {
            crate::files::FileDialog::new()
                .add_filter(tr("系（.zmx・プロジェクト .json）"), &["zmx", "json"])
        }) {
            self.open_doc(&p);
        }
    }

    fn pick_and_open(&mut self) {
        self.pick_and_open_when(true);
    }

    fn refresh_catalog(&mut self) {
        // 系の解析の順番を待たない（下書きの worker で読む）
        self.preview.send(
            Tag::Glasses,
            "glasses",
            json!({ "glass_dir": dirs(&self.s.glass_dirs) }),
        );
        self.preview.send(
            Tag::Lenses,
            "lenses",
            json!({
                "lens_catalog": dirs(&self.s.lens_dirs),
                "glass_dir": dirs(&self.s.glass_dirs),
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
        self.summary
            .as_ref()
            .and_then(|s| s["fields"].as_array().map(Vec::len))
            .unwrap_or(0)
    }

    /// パラメータの名前の欄の補完・階層選択の候補（面・玉・群・波長の数）
    fn param_sys(&self) -> crate::param_pick::Sys {
        crate::param_pick::Sys {
            surfs: self
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
            units: self.ld_units.iter().map(|u| u.name.clone()).collect(),
            waves: self.waves_um().len(),
        }
    }

    fn waves_um(&self) -> Vec<f64> {
        self.summary
            .as_ref()
            .and_then(|s| s["wavelengths_um"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_f64)
            .collect()
    }

    fn primary(&self) -> usize {
        self.summary
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
                return Err(
                    tr("追加の引数は JSON のオブジェクト（例 {\"freqs\": [10, 20]}）").into(),
                );
            }
            let v: Value = serde_json::from_str(t).map_err(|e| trf("追加の引数: {}", &[&e]))?;
            if let Value::Object(o) = v {
                m.extend(o);
            }
        }
        figures::figure_params(&cfg.cmd, &mut m, cutoff, cfg.view == View::Figure);
        Ok(Value::Object(m))
    }

    /// 解析パネルを計算する（スポットの図なら光線も追跡する）
    fn run(&mut self, id: u64, cfg: &AnaCfg) {
        if self.summary.is_none() {
            return;
        }
        // 別のタブのパネル: 元のタブに頼む
        if let Some(r) = &mut self.relay {
            *r = true;
            return;
        }
        match Self::analysis_params(cfg, self.cutoff()) {
            Ok(p) => {
                self.worker.send(Tag::Analyze(id), "analyze", p);
                self.status = Status {
                    text: trf("{} を計算しています…", &[&layout::analysis_label(&cfg.cmd)]),
                    error: false,
                };
            }
            Err(e) => {
                self.results.entry(id).or_default().error = Some(e.clone());
                self.status = Status {
                    text: e,
                    error: true,
                };
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
        // 組合せを変えたら、新規はやり直さない
        self.new_redo = None;
        let dir = lensbench_core::vfs::temp_dir().join("lensbench-gui");
        let name: String = self
            .browser
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
        self.built_combo = Some(self.browser.combo.clone());
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
                self.built = Some(project);
                self.combo_path = Some(path.clone());
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
            Err(e) => {
                self.status = Status {
                    text: format!("{}: {e}", path.display()),
                    error: true,
                }
            }
        }
    }

    /// 保存（Ctrl+S）・名前を付けて保存（Ctrl+Shift+S）。組合せの系は組合せを同梱したプロジェクトの .json に、
    /// 開いた系は系ファイル（.json）に書く。書き先は開いた・前に保存した .json（.zmx から開いた系・Web 版・
    /// as_new は書き先を選ぶ）（#129）
    fn save(&mut self, as_new: bool) {
        let combo =
            self.is_combo() || (self.file.is_none() && !self.browser.combo.picks.is_empty());
        if !combo && self.file.is_none() {
            return;
        }
        let keep = if as_new || cfg!(target_arch = "wasm32") {
            None
        } else {
            self.doc_path.clone()
        };
        let path = match keep {
            Some(p) => p,
            None => {
                let name = self
                    .doc_path
                    .as_ref()
                    .or(if combo { None } else { self.file.as_ref() })
                    .and_then(|f| f.file_stem())
                    .map(|x| x.to_string_lossy().into_owned())
                    .filter(|x| !x.is_empty())
                    .or_else(|| {
                        let n = self.browser.combo.name.trim();
                        (combo && !n.is_empty()).then(|| n.to_string())
                    })
                    .unwrap_or_else(|| "project".into());
                let Some(p) = crate::files::FileDialog::new()
                    .add_filter(tr("プロジェクト"), &["json"])
                    .set_file_name(format!("{name}.json"))
                    .save_file()
                else {
                    return;
                };
                p
            }
        };
        if combo {
            let v = project_with_combo(&self.browser.combo);
            let r = lensbench_core::vfs::write(
                &path,
                serde_json::to_string_pretty(&v).unwrap_or_default(),
            );
            self.status = match r {
                Ok(()) => {
                    self.doc_path = Some(path.clone());
                    let f = path.display().to_string();
                    self.s.recent.retain(|x| x != &f);
                    self.s.recent.insert(0, f);
                    self.s.recent.truncate(10);
                    Status {
                        text: trf("{} に保存", &[&path.display()]),
                        error: false,
                    }
                }
                Err(e) => Status {
                    text: format!("{}: {e}", path.display()),
                    error: true,
                },
            };
        } else {
            self.doc_path = Some(path.clone());
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
            .add_filter(tr("2D の図"), &["svg", "png", "dxf"])
            .add_filter(
                tr("3D の立体"),
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

    fn take(&mut self, d: Done) {
        self.rev += 1;
        let ms = d.ms;
        if let Some((method, e)) = d.error() {
            let text = format!("{method}: {e}");
            match &d.tag {
                Tag::Analyze(id) | Tag::Rays(id) => {
                    self.results.entry(*id).or_default().error = Some(e.to_string());
                }
                Tag::Get => {
                    if let Some(p) = d.replies[0].params["param"].as_str() {
                        self.param_vals.remove(p);
                    }
                }
                // 断られた変更は欄から消す（系の値を読み直す）
                Tag::Edit => {
                    self.oss = Default::default();
                    self.ld.insp.stale();
                }
                Tag::UserCatalog => self.cat_save.note = Some((e.to_string(), true)),
                Tag::CatalogFill => self.fill.failed(e),
                Tag::Optimize => {
                    self.opt.running = false;
                    self.opt.error = Some(e.to_string());
                }
                Tag::Swap => {
                    self.swap.running = false;
                    self.swap.error = Some(e.to_string());
                }
                Tag::Tolerance => {
                    self.tol.running = false;
                    self.tol.error = Some(e.to_string());
                }
                Tag::Zoom => {
                    self.zoom.running = false;
                    self.zoom.error = Some(e.to_string());
                }
                _ => {}
            }
            self.status = Status { text, error: true };
            return;
        }
        let mut text = trf(
            "{} 完了（{} ms）",
            &[&d.replies[0].method, &fmt_num((ms * 10.0).round() / 10.0)],
        );
        let (_, params, v) = d.first();
        let v = v.cloned().unwrap_or(Value::Null);
        let params = params.clone();
        match d.tag {
            Tag::Load => {
                // 開く・読み直すで段を積んだときは段の数と一覧が付く（無ければ段は空）
                self.hist = (0, 0);
                self.hist_list = Default::default();
                self.take_history(&v);
                self.new_undo = None;
                self.new_redo = None;
                self.on_loaded(v);
            }
            Tag::Info => self.info = Some(v),
            Tag::Analyze(id) => {
                let cmd = params["cmd"].as_str().unwrap_or_default().to_string();
                let r = self.results.entry(id).or_default();
                r.prev = if r.cmd == cmd { r.value.take() } else { None };
                r.cmd = cmd;
                r.value = Some(v);
                r.ms = ms;
                r.error = None;
            }
            Tag::Rays(id) => {
                let rays = d
                    .replies
                    .iter()
                    .filter_map(|r| {
                        let (_, a) = r.result.as_ref().ok()?;
                        let f = r.params["field"].as_u64()? as usize;
                        let w = r.params["wave"].as_u64()? as usize;
                        Some(plots::rays_from(f, w, a))
                    })
                    .collect();
                self.results.entry(id).or_default().rays = rays;
                return;
            }
            Tag::Glasses => {
                self.browser.glasses = catalog::parse_glasses(&v);
                self.browser.glass_sel = None;
                self.ld_glasses = self
                    .browser
                    .glasses
                    .iter()
                    .map(|g| lens_data::GlassChoice {
                        name: g.name.clone(),
                        desc: format!("{}  nd {:.5}  νd {:.2}", g.catalog, g.nd, g.vd),
                    })
                    .collect();
                text = trf("ガラス {} 種", &[&self.browser.glasses.len()]);
            }
            Tag::Lenses => {
                self.browser.lenses = catalog::parse_lenses(&v);
                self.browser.lens_sel = None;
                // 前のカタログの断面は捨てる（同じ番号の行でも別の玉。作りかけの結果も受け取らない）
                self.browser.preview = None;
                self.preview_req = None;
                let bad = self
                    .browser
                    .lenses
                    .iter()
                    .filter(|l| l.error.is_some())
                    .count();
                text = if bad > 0 {
                    trf(
                        "カタログレンズ {} 枚、うち {} 枚は読めない（{} ms）",
                        &[&self.browser.lenses.len(), &bad, &fmt_num(ms.round())],
                    )
                } else {
                    trf(
                        "カタログレンズ {} 枚（{} ms）",
                        &[&self.browser.lenses.len(), &fmt_num(ms.round())],
                    )
                };
            }
            Tag::Get => {
                if let (Some(p), Some(x)) = (params["param"].as_str(), v["value"].as_f64()) {
                    self.param_vals.insert(p.to_string(), x);
                    if !self.s.params.iter().any(|q| q == p) {
                        self.s.params.push(p.to_string());
                    }
                }
                return;
            }
            Tag::HistoryDepth => {
                self.take_history(&v);
                return;
            }
            Tag::Set | Tag::Edit | Tag::History => {
                self.take_history(&v);
                if d.tag != Tag::History {
                    self.new_redo = None;
                }
                if d.tag == Tag::History && std::mem::take(&mut self.redo_new_pending) {
                    self.new_system();
                    return;
                }
                // 開く・読み直すを戻した: 戻した系を開いたときと同じく表・解析を出し直す。読んだままの系でなければ
                // 面の表は変更あり
                if let Some(l) = v.get("loaded") {
                    self.on_loaded(l.clone());
                    let top = &self.hist_list.0.first();
                    self.ld_dirty =
                        top.is_some_and(|t| t.get("open").is_none() && t.get("reload").is_none());
                    self.status = Status {
                        text: tr(if params["back"].as_bool() == Some(true) {
                            "元に戻した"
                        } else {
                            "やり直した"
                        })
                        .into(),
                        error: false,
                    };
                    return;
                }
                if d.tag == Tag::History {
                    text = tr(if params["back"].as_bool() == Some(true) {
                        "元に戻した"
                    } else {
                        "やり直した"
                    })
                    .into();
                }
                if d.tag != Tag::Set {
                    self.ld_dirty = true;
                }
                // 値を変えたら要約と、開いている解析を計算し直す
                self.worker.send(Tag::Surfaces, "surface", json!({}));
                self.worker
                    .send(Tag::Info, "analyze", json!({ "cmd": "info" }));
                self.worker
                    .send(Tag::Layout, "layout", self.layout_params());
                self.rerun = Some(true);
            }
            Tag::Write => {}
            Tag::LayoutExport => {
                text = trf(
                    "レイアウトを {} に書き出した",
                    &[&v["written"].as_str().unwrap_or_default()],
                );
            }
            Tag::Surfaces => {
                // 面が変われば多重構成の表も読み直す
                self.worker
                    .send(Tag::ConfigTable, "config_table", json!({}));
                self.oss.stale();
                // 要約の面の数も今の系に（読み込み時のままにしない）
                if let (Some(sum), Some(a)) = (self.summary.as_mut(), v["surfaces"].as_array()) {
                    sum["surfaces"] = json!(a.len());
                }
                let (rows, units) = lens_data::parse(&v);
                self.um.set_mechs(&v["mechanics"]);
                self.ld_blocks = lens_data::tree(&units);
                self.ld.insp.stale();
                self.ld_rows = rows;
                self.ld_units = units;
                return;
            }
            Tag::SurfData(j) => {
                self.ld.insp.set(j, v);
                return;
            }
            Tag::SysData => {
                // 要約の視野・波長も今の系に
                if let Some(sum) = self.summary.as_mut() {
                    let d = &v["data"];
                    if let Some(f) = d["fields"].as_array() {
                        sum["fields"] = json!(f
                            .iter()
                            .map(|f| [f["x"].as_f64(), f["y"].as_f64()])
                            .collect::<Vec<_>>());
                    }
                    if d["wavelengths"].is_array() {
                        sum["wavelengths_um"] = d["wavelengths"].clone();
                    }
                    if let Some(p) = d["primary"].as_u64() {
                        sum["primary"] = json!(p + 1);
                    }
                }
                self.oss.set(&v);
                return;
            }
            Tag::ConfigTable => {
                self.mc.table = Some(config_panel::parse(&v));
                return;
            }
            Tag::ConfigDiff => {
                self.mc.notes = v["notes"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let ops = v["ops"].clone();
                self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
                text = trf("多重構成の表に {} 行を取り込んだ", &[&v["rows"]]);
            }
            Tag::SaveRange => {
                text = trf(
                    "{} に書き出し",
                    &[&params["path"].as_str().unwrap_or_default()],
                );
            }
            Tag::Save => {
                self.ld_dirty = false;
                text = trf("{} に保存", &[&params["path"].as_str().unwrap_or_default()]);
            }
            Tag::Layout => {
                self.layout_data = Some(combo::LayoutData::parse(&v));
                return;
            }
            Tag::LensPreview(_) => return,
            Tag::Focus => {
                self.focus_from(&d);
                return;
            }
            Tag::UserCatalog => {
                let (method, _, _) = d.first();
                self.cat_save.take(&v);
                text = match method {
                    "catalog_add" => trf(
                        "{} に {} を保存",
                        &[
                            &v["catalog"].as_str().unwrap_or_default(),
                            &v["added"].as_str().unwrap_or_default(),
                        ],
                    ),
                    "catalog_edit" => trf(
                        "{} を書き換え",
                        &[&v["catalog"].as_str().unwrap_or_default()],
                    ),
                    _ => text,
                };
                // Web 版: ブラウザに残すユーザーカタログのフォルダは、最初に保存したときにレンズのフォルダに足す
                #[cfg(target_arch = "wasm32")]
                if method == "catalog_add" {
                    let d = crate::web_store::USER_CATALOGS.to_string();
                    if v["catalog"].as_str().is_some_and(|c| c.starts_with(&d))
                        && !self.s.lens_dirs.contains(&d)
                    {
                        self.s.lens_dirs.push(d);
                    }
                }
                if method != "catalog_items" {
                    self.cat_save.note = Some((text.clone(), false));
                    // カタログブラウザに並ぶカタログなら一覧を読み直す
                    self.refresh_catalog();
                }
            }
            Tag::CatalogFill => {
                self.fill.done(&v);
                text = trf(
                    "{} {} を補って {} に保存",
                    &[
                        &v["vendor"].as_str().unwrap_or_default(),
                        &v["item"].as_str().unwrap_or_default(),
                        &v["catalog"].as_str().unwrap_or_default(),
                    ],
                );
                // 補足のカタログのフォルダをレンズのフォルダに足し、一覧を読み直す
                if let Some(d) = crate::fill::fill_dir() {
                    let d = d.display().to_string();
                    if !self.s.lens_dirs.contains(&d) {
                        self.s.lens_dirs.push(d);
                    }
                }
                self.refresh_catalog();
            }
            Tag::Configs => {
                self.overlay.update(&v);
                return;
            }
            Tag::Optimize => {
                self.opt.running = false;
                self.opt.error = None;
                self.opt.undo = None;
                // 結果を系に当てたら元に戻す段が増える
                self.worker.send(Tag::HistoryDepth, "history", json!({}));
                text = trf(
                    "最適化 完了（{} ms）",
                    &[&fmt_num((ms * 10.0).round() / 10.0)],
                );
                self.opt.result = Some(v);
            }
            Tag::Swap => {
                self.swap.running = false;
                self.swap.error = None;
                self.swap.inserted = None;
                if v.get("results").is_some() {
                    text = trf(
                        "差し替え探索 完了（{} ms）",
                        &[&fmt_num((ms * 10.0).round() / 10.0)],
                    );
                }
                self.swap.result = Some(v);
            }
            Tag::Tolerance => {
                self.tol.running = false;
                self.tol.error = None;
                text = trf(
                    "公差 完了（{} ms）",
                    &[&fmt_num((ms * 10.0).round() / 10.0)],
                );
                self.tol.result = Some(v);
            }
            Tag::Zoom => {
                self.zoom.running = false;
                self.zoom.error = None;
                text = trf(
                    "ズーム軌跡 完了（{} ms）",
                    &[&fmt_num((ms * 10.0).round() / 10.0)],
                );
                self.zoom.result = Some(v);
            }
        }
        self.status = Status { text, error: false };
    }

    /// 値の変更を送る（計算中なら終わってからまとめて）
    fn flush_set(&mut self) {
        if self.worker.busy() {
            return;
        }
        if !self.pending_set.is_empty() {
            let values = std::mem::take(&mut self.pending_set);
            self.worker
                .send(Tag::Set, "set", json!({ "values": values }));
        }
    }

    fn info_num(&self, k: &str) -> Option<f64> {
        self.info.as_ref().and_then(|i| i[k].as_f64())
    }

    /// Web 版: 別のタブのパネルに送る設定・系の概要・結果
    #[cfg(target_arch = "wasm32")]
    fn tab_snapshot(&self, id: u64, cfg: &AnaCfg) -> Value {
        let res = self.results.get(&id).map(|r| {
            json!({
                "cmd": r.cmd,
                "value": r.value,
                "prev": r.prev,
                "ms": r.ms,
                "error": r.error,
                "rays": r.rays.iter().map(|x| json!([x.field, x.wave, x.x, x.y])).collect::<Vec<_>>(),
            })
        });
        json!({
            "cfg": cfg,
            "file": self.file,
            "summary": self.summary,
            "info": self.info,
            "res": res,
            "busy": self.worker.busy(),
        })
    }

    /// Web 版: 元のタブから届いた系の概要・結果を当てる（別のタブのパネル）
    #[cfg(target_arch = "wasm32")]
    fn apply_snapshot(&mut self, id: u64, v: &Value) {
        let some = |x: &Value| (!x.is_null()).then(|| x.clone());
        self.file = v["file"].as_str().map(PathBuf::from);
        self.summary = some(&v["summary"]);
        self.info = some(&v["info"]);
        self.relay_busy = v["busy"].as_bool().unwrap_or(false);
        let r = &v["res"];
        if r.is_null() {
            self.results.remove(&id);
            return;
        }
        let rays = r["rays"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|x| {
                        let f = |k: usize| -> Vec<f64> {
                            serde_json::from_value(x[k].clone()).unwrap_or_default()
                        };
                        plots::SpotRays {
                            field: x[0].as_u64().unwrap_or(1) as usize,
                            wave: x[1].as_u64().unwrap_or(1) as usize,
                            x: f(2),
                            y: f(3),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.results.insert(
            id,
            AnaRes {
                cmd: r["cmd"].as_str().unwrap_or_default().to_string(),
                value: some(&r["value"]),
                prev: some(&r["prev"]),
                ms: r["ms"].as_f64().unwrap_or(0.0),
                rays,
                error: r["error"].as_str().map(String::from),
            },
        );
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
        match (&self.combo_path, self.is_combo()) {
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

    /// 元に戻す（back）・やり直すことができるか。組合せの系は組合せの段、開いた系は serve の段
    fn can_undo(&self, back: bool) -> bool {
        if let Some(n) = self.new_undo_step(back) {
            return n;
        }
        if self.is_combo() && self.combo_hist.can(&self.browser.combo, back) {
            return true;
        }
        if self.summary.is_some() {
            if back {
                self.hist.0 > 0
            } else {
                self.serve_redo() > 0
            }
        } else {
            false
        }
    }

    /// 新規の直後（元に戻す）・新規を戻した直後（やり直す）なら Some（戻せるか）。それ以外は None
    fn new_undo_step(&self, back: bool) -> Option<bool> {
        if self.summary.is_none() && self.browser.combo.picks.is_empty() {
            return Some(back && self.new_undo.is_some());
        }
        (!back
            && self.new_redo.is_some()
            && self.serve_redo() == 0
            && !self.combo_hist.can(&self.browser.combo, false))
        .then_some(true)
    }

    /// serve のやり直せる段の数（新規を戻した後は、新規の前の段を除く）
    fn serve_redo(&self) -> usize {
        self.hist.1.saturating_sub(self.new_redo.unwrap_or(0))
    }

    /// 系を読んだ（開いた・読み直した・それを戻した）後: ファイル名・概要を置き、面の表・解析を読み直す
    fn on_loaded(&mut self, v: Value) {
        let new_file = v["file"].as_str().map(PathBuf::from);
        if new_file != self.file {
            // 別の系: 前との差は出さない
            for r in self.results.values_mut() {
                r.value = None;
                r.prev = None;
            }
        }
        // 読み直した系には面の表の変更は無い。別の系なら階層・選択・ロックも戻す（組合せの作り直しは同じ系）
        self.ld_dirty = false;
        if new_file != self.file {
            self.ld.reset();
        }
        self.file = new_file;
        self.summary = Some(v);
        self.worker.send(Tag::Surfaces, "surface", json!({}));
        self.info = None;
        self.worker
            .send(Tag::Info, "analyze", json!({ "cmd": "info" }));
        self.read_params();
        // 組合せの図は下書きで作る（ここで作ると、動かしている途中の古い図に戻ることがある）
        if !self.is_combo() {
            self.worker
                .send(Tag::Layout, "layout", self.layout_params());
        }
        self.worker.send(Tag::Configs, "configs", json!({}));
        self.rerun = Some(false);
    }

    /// serve の結果の段の数と一覧を覚える
    fn take_history(&mut self, v: &Value) {
        if let (Some(u), Some(r)) = (v["undo"].as_u64(), v["redo"].as_u64()) {
            self.hist = (u as usize, r as usize);
            let list = |k: &str| v[k].as_array().cloned().unwrap_or_default();
            self.hist_list = (list("undo_list"), list("redo_list"));
        }
    }

    /// 元に戻す・やり直す段の名前（元に戻す段は新しい順、やり直す段は次の順）
    fn undo_list(&self, back: bool) -> Vec<String> {
        match self.new_undo_step(back) {
            Some(true) => return vec![tr("新規").into()],
            Some(false) => return Vec::new(),
            None => {}
        }
        // 組合せの系は組合せの段の後に、組合せを作る前の serve の段（開く など）
        let mut out = if self.is_combo() {
            self.combo_hist.list(back)
        } else {
            Vec::new()
        };
        let l = if back {
            &self.hist_list.0
        } else {
            &self.hist_list.1
        };
        if self.is_combo() && !out.is_empty() && !back {
            return out;
        }
        let m = if back { l.len() } else { self.serve_redo() };
        out.extend(l.iter().take(m).map(hist_label));
        if !back && self.new_redo.is_some() {
            out.push(tr("新規").into());
        }
        out
    }

    /// n 段元に戻す（back）・やり直す（Ctrl+Z・Ctrl+Y、編集メニューの一覧）
    fn undo(&mut self, back: bool, n: usize) {
        if self.new_undo_step(back) == Some(true) {
            self.undo_new(back);
            return;
        }
        // 組合せの段を使い切ったら serve の段（組合せを作る前に開いた系など）
        let k = if self.is_combo() {
            self.combo_hist.list(back).len()
        } else {
            0
        };
        if k > 0 {
            if n > k && k > 0 && back {
                // 組合せの段を越えて選んだ: 組合せの段は捨てて、残りを serve で戻す
                self.combo_hist = Default::default();
                self.undo(back, n - k);
                return;
            }
            match self.combo_hist.step(&self.browser.combo, back, n) {
                Some(c) => {
                    self.browser.combo = c;
                    if !self.s.auto_build {
                        self.build_combo(self.browser.combo.project());
                    }
                    self.status = Status {
                        text: tr(if back {
                            "元に戻した"
                        } else {
                            "やり直した"
                        })
                        .into(),
                        error: false,
                    };
                }
                None => {
                    self.status = Status {
                        text: tr(if back {
                            "元に戻す変更がない"
                        } else {
                            "やり直す変更がない"
                        })
                        .into(),
                        error: false,
                    }
                }
            }
            return;
        }
        if self.summary.is_some() {
            self.flush_set();
            let mut n = n;
            if !back && self.new_redo.is_some() && n > self.serve_redo() {
                // serve の段の後に新規までやり直す
                n = self.serve_redo();
                self.redo_new_pending = true;
            }
            let m = if back { "undo" } else { "redo" };
            self.worker
                .send(Tag::History, m, json!({ "back": back, "steps": n }));
        }
    }

    /// 開いた系（組合せでない系）にカタログの要素を差し込む: 開いた系を組合せ（系の取り込み）にし、面 `after` の後の
    /// 空気の間隔（None は系の後ろ）に置く。今の組合せに要素があれば置き換えてよいか聞く。差し込むことにしたら true
    fn insert_into_open(&mut self, after: Option<usize>, pick: combo::Pick, n_old: usize) -> bool {
        if n_old > 0
            && !crate::files::confirm(
                tr("開いた系に足す"),
                &trf(
                    "開いた系を組合せにして {} を足します。今の組合せ（{} 個の要素）は置き換わります。よいですか？",
                    &[&pick.label(), &n_old],
                ),
            )
        {
            return false;
        }
        self.open_insert = Some((after, pick));
        self.to_combo = Some(false);
        true
    }

    /// レンズデータで面 j の行を選ぶ: 最上位のブロック（玉・群）の中なら、そのブロックの行（今のセルはブロックの最後の面）
    fn select_ld_surface(&mut self, j: usize) {
        self.ld.scope.clear();
        let top = self
            .ld_blocks
            .iter()
            .find(|b| b.parent.is_none() && b.surfs.contains(&j));
        match top {
            Some(b) => {
                self.ld.sel = Some(b.name.clone());
                // ブロックの最後の面（たたんだ行の鍵。続けて足すとブロックの後に入る）
                let last = b.surfs.last().copied().unwrap_or(j);
                self.ld.cur = Some((last, lens_data::Col::R));
            }
            None => {
                self.ld.sel = None;
                self.ld.cur = Some((j, lens_data::Col::R));
            }
        }
    }

    fn is_combo(&self) -> bool {
        self.file.is_some() && self.file == self.combo_path
    }

    /// 最適化
    fn optimize_ui(&mut self, ui: &mut Ui) {
        use crate::optim_panel::{self as op, Action};
        if self.summary.is_none() {
            ui.add_space(8.0);
            ui.label(tr("系を開くか、組合せで系を作ると最適化できます。"));
            return;
        }
        let is_combo = self.is_combo();
        let cands = op::Cands {
            units: self.ld_units.iter().map(|u| u.name.clone()).collect(),
            combo: is_combo,
            sys: self.param_sys(),
        };
        let progress = if self.opt.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
            crate::worker::progress::optimize()
        } else {
            None
        };
        let acts = op::ui(ui, &mut self.opt, &cands, progress.as_deref());
        let mut load = false;
        for a in acts {
            match a {
                Action::Run(p) => {
                    self.opt.running = true;
                    self.opt.result = None;
                    self.worker.send(Tag::Optimize, "optimize", p);
                }
                Action::Stop => crate::worker::progress::stop("optimize"),
                Action::Apply => {
                    let Some(r) = self.opt.result.clone() else {
                        continue;
                    };
                    if self.apply_values(&op::result_values(&r, false)) {
                        self.opt.undo = Some(op::result_values(&r, true));
                    }
                }
                Action::Undo => {
                    if let Some(u) = self.opt.undo.take() {
                        self.apply_values(&u);
                    }
                }
                Action::Save(v) => {
                    if let Some(path) = crate::files::FileDialog::new()
                        .add_filter(tr("最適化の設定"), &["json"])
                        .set_file_name("optimize.json")
                        .save_file()
                    {
                        let txt = serde_json::to_string_pretty(&v).unwrap_or_default();
                        self.status = match lensbench_core::vfs::write(&path, txt + "\n") {
                            Ok(()) => Status {
                                text: trf("{} に保存", &[&path.display()]),
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
            crate::files::FileDialog::new().add_filter(tr("最適化の設定"), &["json"])
        }) {
            let r = lensbench_core::vfs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                .and_then(|v| self.opt.set_params(&v));
            if let Err(e) = r {
                self.status = Status {
                    text: format!("{}: {e}", path.display()),
                    error: true,
                };
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
        if self.summary.is_none() {
            ui.add_space(8.0);
            ui.label(tr("系を開くとズーム軌跡を解けます。"));
            return;
        }
        let cands = self.zoom_cands();
        let sys = self.param_sys();
        if let Some(p) = crate::zoom_panel::ui(ui, &mut self.zoom, &cands, &sys) {
            self.zoom.running = true;
            self.zoom.result = None;
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
                    text: trf("{} に保存", &[&path.display()]),
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
        if self.summary.is_none() {
            ui.add_space(8.0);
            ui.label(tr("系を開くか、組合せで系を作ると公差を回せます。"));
            return;
        }
        let cands = crate::optim_panel::Cands {
            units: self.ld_units.iter().map(|u| u.name.clone()).collect(),
            // 補償子は系の中で動かすだけ（組合せに当てない）ので、面の値の種類を絞らない
            combo: false,
            sys: self.param_sys(),
        };
        let progress = if self.tol.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
            Some(crate::worker::progress::tolerance().unwrap_or((0, 0)))
        } else {
            None
        };
        let acts = tp::ui(ui, &mut self.tol, &cands, progress);
        let mut load = false;
        for a in acts {
            match a {
                Action::Run(p) => {
                    self.tol.running = true;
                    self.tol.result = None;
                    self.worker.send(Tag::Tolerance, "tolerance", p);
                }
                Action::Stop => crate::worker::progress::stop("tolerance"),
                Action::Save(v) => {
                    let txt = serde_json::to_string_pretty(&v).unwrap_or_default() + "\n";
                    self.save_text(tr("公差の設定"), "json", "tolerance.json", txt);
                }
                Action::SaveResult(v) => {
                    let txt = serde_json::to_string_pretty(&v).unwrap_or_default() + "\n";
                    self.save_text(tr("公差の結果"), "json", "tolerance_result.json", txt);
                }
                Action::SaveCsv(txt) => {
                    self.save_text("CSV", "csv", "tolerance_compensators.csv", txt);
                }
                Action::Load => load = true,
            }
        }
        // 設定の読み込み（Web 版は選び終わった後のフレームで返るので毎フレーム見る）
        if let Some(path) = crate::files::open("tol-load", load, || {
            crate::files::FileDialog::new().add_filter(tr("公差の設定"), &["json"])
        }) {
            let r = lensbench_core::vfs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                .and_then(|v| self.tol.set_spec(&v));
            if let Err(e) = r {
                self.status = Status {
                    text: format!("{}: {e}", path.display()),
                    error: true,
                };
            }
        }
    }

    /// 差し替え探索
    fn swap_ui(&mut self, ui: &mut Ui) {
        use crate::swap_panel::{self as sp, Action};
        if self.summary.is_none() {
            ui.add_space(8.0);
            ui.label(tr(
                "組合せで系を作るか、玉のブロックを持つプロジェクトを開くと差し替え探索ができます。",
            ));
            return;
        }
        let is_combo = self.is_combo();
        let (project, mut cands) = if is_combo {
            let (p, c) = sp::combo_project(&self.browser.combo);
            (Some(p), c)
        } else {
            let f = self.file.clone().unwrap_or_default();
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
            self.browser.combo.states.clone()
        } else {
            self.overlay
                .items
                .iter()
                .filter(|x| x.0 == "states")
                .map(|x| x.1.clone())
                .collect()
        };
        let progress = if self.swap.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
            crate::worker::progress::swap()
        } else {
            None
        };
        cands.sys = self.param_sys();
        let acts = sp::ui(ui, &mut self.swap, &cands, progress);
        if !is_combo {
            if let Some(x) = self.swap_file.as_mut() {
                x.1 = cands;
            }
        }
        let mut load = false;
        for a in acts {
            let count = matches!(a, Action::Count(_));
            match a {
                Action::Run(_) | Action::Count(_) if self.swap.running => {}
                Action::Run(mut v) | Action::Count(mut v) => {
                    if count {
                        v["count_only"] = json!(true);
                    }
                    if let Some(p) = &project {
                        v["project"] = p.clone();
                    }
                    v["top"] = json!(self.swap.top);
                    self.swap_ids = self.browser.combo.ids();
                    self.swap.running = true;
                    self.swap.result = None;
                    self.worker.send(Tag::Swap, "swap", v);
                }
                Action::Stop => crate::worker::progress::stop("swap"),
                Action::Insert(i) => {
                    let Some(row) = self.swap.result.as_ref().and_then(|r| r["results"].get(i))
                    else {
                        continue;
                    };
                    let row = row.clone();
                    let before = self.browser.combo.clone();
                    match sp::insert_row(&mut self.browser.combo, &self.swap_ids, &row) {
                        Ok(()) => {
                            if self.swap.undo.is_none() {
                                self.swap.undo = Some(before);
                            }
                            self.swap.inserted = Some(i);
                            if !self.s.auto_build {
                                self.build_combo(self.browser.combo.project());
                            }
                        }
                        Err(e) => {
                            self.status = Status {
                                text: e,
                                error: true,
                            }
                        }
                    }
                }
                Action::Undo => {
                    if let Some(c) = self.swap.undo.take() {
                        self.browser.combo = c;
                        self.swap.inserted = None;
                        if !self.s.auto_build {
                            self.build_combo(self.browser.combo.project());
                        }
                    }
                }
                Action::Save(v) => {
                    if let Some(path) = crate::files::FileDialog::new()
                        .add_filter(tr("差し替え探索の設定"), &["json"])
                        .set_file_name("swap.json")
                        .save_file()
                    {
                        let txt = serde_json::to_string_pretty(&v).unwrap_or_default();
                        self.status = match lensbench_core::vfs::write(&path, txt + "\n") {
                            Ok(()) => Status {
                                text: trf("{} に保存", &[&path.display()]),
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
            crate::files::FileDialog::new().add_filter(tr("差し替え探索の設定"), &["json"])
        }) {
            let r = lensbench_core::vfs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                .and_then(|v| self.swap.set_spec(&v));
            if let Err(e) = r {
                self.status = Status {
                    text: format!("{}: {e}", path.display()),
                    error: true,
                };
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
                    self.status = Status {
                        text: trf(
                            "変数 {} は組合せに当てられない（面の厚さ・曲率半径・コーニックだけ）",
                            &[&n],
                        ),
                        error: true,
                    };
                    return false;
                }
            }
        }
        let ids = self.browser.combo.span_ids();
        let lead = self.browser.combo.lead_stop();
        let spans = lens_data::elem_spans(&ids, &self.ld_units, lead, &self.ld_rows);
        for (j, key, value) in edits {
            if !self.combo_surface_edit(&spans, j, key, value) {
                return false;
            }
        }
        if !self.s.auto_build {
            self.build_combo(self.browser.combo.project());
        }
        true
    }

    /// 組合せの系の設定
    fn sysset_ui(&mut self, ui: &mut Ui) {
        if self.agf_names.0 != self.s.glass_dirs {
            self.agf_names = (self.s.glass_dirs.clone(), agf_names(&self.s.glass_dirs));
        }
        if self.file.is_some() && !self.is_combo() {
            // 開いた系の設定。組合せの設定はその下にたたむ
            if self.oss.want() {
                self.worker.send(Tag::SysData, "system_data", json!({}));
            }
            let ns = self.ld_rows.len();
            egui::ScrollArea::vertical()
                .id_salt("sysset_scroll")
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(tr(
                            "開いている系の設定です。変えた値は系に入り、「系ファイルに保存…」で残せます。",
                        ))
                        .small()
                        .weak(),
                    );
                    if let Some(op) = open_sysset::ui(ui, &mut self.oss, ns) {
                        self.worker.send(Tag::Edit, "edit", json!({ "ops": [op] }));
                    }
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new(tr("組合せの設定（組合せの系を作ると使われる）"))
                        .id_salt("combo_sysset")
                        .default_open(false)
                        .show(ui, |ui| {
                            crate::sysset::ui(ui, &mut self.browser.combo, &self.agf_names.1)
                        });
                });
            return;
        }
        let cats = &self.agf_names.1;
        egui::ScrollArea::vertical()
            .id_salt("sysset_scroll")
            .show(ui, |ui| {
                crate::sysset::ui(ui, &mut self.browser.combo, cats)
            });
    }

    /// 組合せのレイアウト図と要素の操作
    fn layout_ui(&mut self, ui: &mut Ui) {
        let is_combo = self.is_combo();
        // 開いた系の図の下の欄（図と分けたとき）
        let mut below: Option<Ui> = None;
        let accent = theme::accent(&self.s.look);
        if self.file.is_some() {
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label(tr("見る面"));
                for p in ViewPlane::ALL {
                    let r = ui.selectable_value(&mut self.s.view_plane, p, p.label());
                    let r = match p {
                        ViewPlane::Unfold => r.on_hover_text(tr(
                            "鏡・BS で折れた後の区間を、折れる前の光軸の延長に並べ直した図（光軸に沿う操作だけ）",
                        )),
                        ViewPlane::D3 => r.on_hover_text(tr(
                            "ドラッグで回す・Shift か右ドラッグで動かす・ホイールで拡大・ダブルクリックで戻す",
                        )),
                        _ => r,
                    };
                    changed |= r.changed();
                }
                ui.separator();
                if ui
                    .button(tr("書き出し…"))
                    .on_hover_text(tr(
                        "今の見る面・重ねた構成で書き出す。形式は拡張子で: 2D は svg・png・dxf、3D は step・glb・gltf・obj・stl",
                    ))
                    .clicked()
                {
                    self.export_layout();
                }
            });
            changed |= self.overlay.ui(ui, self.s.view_plane.vert());
            let (nf, nw) = (self.n_fields(), self.waves_um().len());
            let head = if self.s.rays.is_default() {
                tr("光線・書き出しの設定").to_string()
            } else {
                format!("{} ●", tr("光線・書き出しの設定"))
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
        if self.file.is_some() && !is_combo {
            ui.horizontal_wrapped(|ui| {
                ui.weak(tr(
                    "開いている系は組合せから作ったものではありません（図は見るだけ）。",
                ));
                if ui
                    .button(tr("この系を組合せにする"))
                    .on_hover_text(tr(
                        "開いた系を組合せの要素（系の取り込み 1 つ）にし、系の設定（開口・視野・波長など）も移す。前後にカタログの玉などを足せる",
                    ))
                    .clicked()
                    && (self.browser.combo.picks.is_empty()
                        || crate::files::confirm(
                            tr("組合せにする"),
                            tr("今の組合せを、開いた系を取り込んだ組合せに置き換えますか？"),
                        ))
                {
                    self.to_combo = Some(false);
                }
                if !self.browser.combo.picks.is_empty()
                    && ui.button(tr("▶ 組合せの系を作る")).clicked()
                {
                    self.build_combo(self.browser.combo.project());
                }
            });
            // 開いている系のレイアウト図（組合せと同じ描き方）
            // 組合せの要素のパレット・図はその下に
            // 図と下の組合せの欄に残りを分ける（パネルが低くても下が押し出されないように）
            if let Some(d) = &self.layout_data {
                let (mut top, b) = split::two(ui, "open_layout", (0.55, 0.55), 80.0);
                let h = top.available_height();
                let hl = self.ld.cur.map(|x| x.0);
                let clicked = combo::view_only(
                    &mut top,
                    d,
                    h,
                    accent,
                    Some(egui::Id::new("open_layout_nav")),
                    hl,
                );
                match clicked {
                    // 図でクリックした玉の行をレンズデータで選ぶ
                    Some(combo::OpenAct::Clicked([a, _])) => self.select_ld_surface(a),
                    // カタログの行を落とした間隔に差し込む（開いた系を組合せにして）
                    Some(combo::OpenAct::Drop { after, pick }) => {
                        let n = self.browser.combo.picks.len();
                        self.insert_into_open(Some(after), *pick, n);
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
        self.combo_view.hl_surf = if is_combo {
            self.ld.cur.map(|x| x.0)
        } else {
            None
        };
        let act = combo::ui(
            ui,
            &mut self.browser.combo,
            data,
            &mut self.combo_view,
            &mut self.s.auto_build,
            self.worker.busy(),
            accent,
            &paths,
        );
        match act {
            Some(combo::Act::Catalog) => self.cmds.push(Cmd::FocusKind(Kind::Catalog)),
            Some(combo::Act::Build) => self.build_combo(self.browser.combo.project()),
            Some(combo::Act::Focus) => self.request_focus(),
            Some(combo::Act::SaveToCatalog(k)) => {
                self.cat_save.open_for(k, &self.browser.combo);
                let c = self.cat_save.catalog.trim().to_string();
                if !c.is_empty() && self.cat_save.listed != c {
                    self.preview
                        .send(Tag::UserCatalog, "catalog_items", json!({ "catalog": c }));
                }
            }
            Some(combo::Act::Open(k)) => {
                if let Some(id) = self.browser.combo.ids().get(k) {
                    // 面の表がまだ前の系なら、木に無い名前は入れない（最上位で選ぶ）
                    self.ld.scope.clear();
                    self.ld.enter(&self.ld_blocks, id);
                    if self.ld.scope.is_empty() {
                        self.ld.sel = Some(id.clone());
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
        if self.summary.is_none() || self.ld_rows.is_empty() {
            ui.add_space(8.0);
            ui.label(tr("系を開くか、組合せで系を作ると、面の表が出ます。"));
            return;
        }
        let is_combo = self.is_combo();
        let ids = if is_combo {
            self.browser.combo.ids()
        } else {
            Vec::new()
        };
        // 組合せ: 図で選んだ要素を表でも選ぶ（最上位のとき）
        if is_combo && self.ld.scope.is_empty() {
            if let Some(k) = self.combo_view.sel {
                self.ld.sel = ids.get(k).cloned();
            }
        }
        // 今の経路に沿った要素（分岐の無い組合せは並びそのもの）
        let picks = &self.browser.combo.chain_picks();
        let locked: BTreeSet<String> = if is_combo {
            ids.iter()
                .zip(picks)
                .filter(|(_, p)| p.locked)
                .map(|(i, _)| i.clone())
                .collect()
        } else {
            self.ld.locked.clone()
        };
        let lead = self.browser.combo.lead_stop();
        let spans = if is_combo {
            let live = self.browser.combo.span_ids();
            lens_data::elem_spans(&live, &self.ld_units, lead, &self.ld_rows)
        } else {
            Vec::new()
        };
        // 図で別の要素を選んだら、その要素の外の今の面は外す（図の目立たせと表を合わせる）
        if is_combo && self.combo_view.sel != self.combo_sel_prev {
            if let (Some(k), Some((j, _))) = (self.combo_view.sel, self.ld.cur) {
                let inside = spans
                    .get(k)
                    .copied()
                    .flatten()
                    .is_some_and(|s| s[0] <= j && j <= s[1]);
                if !inside {
                    self.ld.cur = None;
                }
            }
        }
        self.combo_sel_prev = self.combo_view.sel;
        let rows = &self.ld_rows;
        let read_only = |j: usize, col: Col| -> Option<String> {
            combo_read_only(is_combo, rows, &spans, picks, lead, j, col)
        };
        let cfg_rows: Vec<String> = self
            .mc
            .table
            .as_ref()
            .map(|t| t.rows.iter().map(|r| r.0.clone()).collect())
            .unwrap_or_default();
        let ctx = lens_data::Ctx {
            config_n: self.mc.table.as_ref().map_or(1, |t| t.n),
            config_rows: &cfg_rows,
            glasses: &self.ld_glasses,
            glasses_loaded: self.browser.loaded && !self.browser.glasses_pending,
            rows,
            blocks: &self.ld_blocks,
            locked: &locked,
            read_only: &read_only,
            combo: is_combo,
            unit_mech: (!is_combo).then_some(self.um_open),
            note: is_combo.then(|| tr(
                "組合せの系: 変えた値は組合せの要素に残ります（後の空気は要素の間隔、ほかは面の上書き）。偏心・傾き・BS は要素の設定で。",
            )),
        };
        self.ld.tree = self.s.ld_tree;
        self.ld.detail = self.s.ld_detail;
        self.ld.by_surface = self.s.ld_by_surface;
        let (acts, mut bottom) = lens_data::ui(ui, &mut self.ld, &ctx);
        self.s.ld_tree = self.ld.tree;
        self.s.ld_detail = self.ld.detail;
        self.s.ld_by_surface = self.ld.by_surface;
        for a in acts {
            match a {
                Act::Select(name) => {
                    if is_combo && self.ld.scope.is_empty() {
                        if let Some(k) = name.and_then(|n| ids.iter().position(|i| *i == n)) {
                            self.combo_view.select(k);
                        }
                    }
                }
                Act::Lock(name, on) => {
                    if is_combo {
                        match ids.iter().position(|i| *i == name) {
                            Some(k) => {
                                if let Some(p) = self.browser.combo.el_mut(k) {
                                    p.locked = on;
                                }
                            }
                            None => {
                                self.status = Status {
                                    text: tr("組合せでは要素ごとにロックする").into(),
                                    error: true,
                                }
                            }
                        }
                    } else if on {
                        self.ld.locked.insert(name);
                    } else {
                        self.ld.locked.remove(&name);
                    }
                }
                Act::ExportRange([a, b]) => {
                    let stem = self
                        .file
                        .as_ref()
                        .and_then(|f| f.file_stem())
                        .map(|x| x.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "system".into());
                    if let Some(p) = crate::files::FileDialog::new()
                        .add_filter(tr("系ファイル"), &["json"])
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
                    if self.um_open && self.um.new_spans.trim().is_empty() {
                        if let Some((j, _)) = self.ld.cur {
                            self.um.new_spans = j.to_string();
                        }
                    }
                }
                Act::Refused(why) => {
                    self.status = Status {
                        text: why,
                        error: true,
                    }
                }
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
                                    self.build_combo(self.browser.combo.project());
                                }
                            }
                            Err(e) => {
                                self.status = Status {
                                    text: e,
                                    error: true,
                                }
                            }
                        }
                    } else {
                        self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
                    }
                }
                Act::DropPick { after, pick } => {
                    if is_combo {
                        // 組合せの系: 面 after を持つ要素の後（要素の外の面なら、その面より前の要素の後）
                        let at = spans
                            .iter()
                            .rposition(|s| s.is_some_and(|s| s[0] <= after))
                            .map_or(0, |k| k + 1);
                        let k = combo::insert_pick(&mut self.browser.combo, at, *pick);
                        self.combo_view.select(k);
                    } else if self.file.is_some() {
                        if open_gap_ok(&self.ld_rows, after) {
                            let n = self.browser.combo.picks.len();
                            self.insert_into_open(Some(after), *pick, n);
                        } else {
                            self.status = Status {
                                text: trf(
                                    "面 {} の後は空気の間隔でないので差し込めない",
                                    &[&after],
                                ),
                                error: true,
                            };
                        }
                    }
                }
                Act::NeedGlasses => {
                    if !self.browser.glasses_pending {
                        self.refresh_catalog();
                    }
                }
                Act::NeedData(j) => {
                    self.worker
                        .send(Tag::SurfData(j), "surface_data", json!({ "surface": j }));
                }
            }
        }
        // ブロックの設定（組合せは要素の設定、開いた系は玉・群の偏心・傾き・移動・dn・disp）と、
        // 開いた系の玉・群を足す・機構部品の欄（同じスクロールの中）
        let view = self.ld.block_view.clone();
        if view.is_some() {
            egui::ScrollArea::vertical()
                .id_salt("block_edit")
                .auto_shrink([false, false])
                .show(&mut bottom, |ui| {
                    if let Some(name) = &view {
                        if is_combo {
                            match ids.iter().position(|i| i == name) {
                                Some(k) => combo::block_settings(ui, &mut self.browser.combo, k),
                                None => {
                                    ui.weak(trf(
                                        "{} は組合せの要素でない（設定は要素を選んで）",
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
            let n = self.ld_rows.len();
            let mut ops = Vec::new();
            egui::Window::new(tr("玉・群・機構部品"))
                .id(egui::Id::new("unit_mech_win"))
                .open(&mut open)
                .default_width(380.0)
                .vscroll(true)
                .show(ui.ctx(), |ui| {
                    ops = crate::unit_mech::ui(ui, &mut self.um, n);
                });
            if !ops.is_empty() {
                self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
            }
            self.um_open = open;
        }
    }

    /// 多重構成の次（back なら前）の構成を系に入れる（Ctrl+A・Ctrl+Shift+A、最後の次は最初）
    fn cycle_config(&mut self, back: bool) {
        let Some(t) = self.mc.table.as_ref().filter(|t| t.n > 1) else {
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
        self.status = Status {
            text: trf("構成 {} / {}", &[&k, &t.n]),
            error: false,
        };
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
        for a in config_panel::ui(ui, &mut self.mc, combo, &cands) {
            match a {
                config_panel::Act::Ops(ops) => {
                    self.mc.notes.clear();
                    self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
                }
                config_panel::Act::Import => {
                    self.worker.send(Tag::ConfigDiff, "config_diff", json!({}));
                }
                config_panel::Act::Refused(why) => {
                    self.status = Status {
                        text: why,
                        error: true,
                    }
                }
            }
        }
    }

    /// 開いた系の玉・群の設定（serve の set の `名前.dx` など）
    fn unit_settings(&mut self, ui: &mut Ui, name: &str) {
        // 名前・範囲を変える・外す（serve の edit）
        if let Some(u) = self.ld_units.iter().find(|u| u.name == name) {
            let moved = u.motion.iter().any(|&x| x != 0.0) || u.shift != 0.0;
            let spans = u.spans.clone();
            let ops = crate::unit_mech::unit_ui(ui, &mut self.um, name, &spans, moved);
            for op in &ops {
                match op["op"].as_str() {
                    Some("remove_unit") => self.ld.block_view = None,
                    _ => {
                        if let Some(n) = op["name"].as_str() {
                            self.ld.block_view = Some(n.to_string());
                            self.ld.sel = Some(n.to_string());
                        }
                    }
                }
            }
            if !ops.is_empty() {
                self.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
            }
        }
        let Some(u) = self.ld_units.iter_mut().find(|u| u.name == name) else {
            return;
        };
        let weak = theme::weak_text(ui.visuals());
        let locked = self
            .ld_blocks
            .iter()
            .position(|b| b.name == name)
            .is_some_and(|i| lens_data::block_locked(&self.ld_blocks, &self.ld.locked, i));
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(name).strong());
            let spans = u
                .spans
                .iter()
                .map(|s| format!("{}–{}", s[0], s[1]))
                .collect::<Vec<_>>()
                .join(", ");
            ui.label(RichText::new(trf("面 {}", &[&spans])).color(weak));
            if locked {
                ui.label(RichText::new(trf("{} はロック中", &[&name])).color(weak));
            }
        });
        let mut set = Vec::new();
        // ロックは中身（dn・disp）だけを止める。偏心・傾き・移動は変えられる
        {
            ui.horizontal_wrapped(|ui| {
                let labels = [
                    (tr("偏心 x"), " mm"),
                    (tr("偏心 y"), " mm"),
                    (tr("傾き x"), "°"),
                    (tr("傾き y"), "°"),
                    (tr("傾き z"), "°"),
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
                ui.label(RichText::new(tr("移動")).color(weak))
                    .on_hover_text(tr(
                        "群の移動（光の進む向きが正）。前の間隔に足し、後の間隔から引く",
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
                                "媒質が無いか、面ごとに値が違う（面の設定で変える）",
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
            self.build_combo(self.browser.combo.project());
        }
    }

    /// 組合せの系の面 j（系の面の番号）の要素と、要素の面の番号（座標ブレークを除く）
    fn combo_face(&self, spans: &[Option<[usize; 2]>], j: usize) -> Option<(usize, usize)> {
        let k = spans
            .iter()
            .position(|s| s.is_some_and(|s| s[0] <= j && j <= s[1]))?;
        let a = spans[k]?[0];
        let rows = &self.ld_rows;
        if !rows.get(j).is_some_and(|r| !r.is_cb()) {
            return None;
        }
        Some((k, (a..j).filter(|&i| !rows[i].is_cb()).count()))
    }

    /// 組合せの系の絞り・ソルブ・ピックアップ: serve の edit の操作を組合せの要素の surface_ops にする
    fn combo_ops(
        &mut self,
        spans: &[Option<[usize; 2]>],
        ids: &[String],
        ops: &[Value],
    ) -> Result<(), String> {
        let lead0 = self.browser.combo.lead_stop();
        let r = self.combo_ops_inner(spans, ids, ops);
        // 前の絞りが出入りすると面の番号がずれるので、選んだ面も合わせる
        let lead1 = self.browser.combo.lead_stop();
        if lead0 != lead1 {
            if let Some((j, c)) = self.ld.cur {
                self.ld.cur = Some(if lead1 {
                    (j + 1, c)
                } else {
                    (j.saturating_sub(1), c)
                });
            }
        }
        r
    }

    fn combo_ops_inner(
        &mut self,
        spans: &[Option<[usize; 2]>],
        ids: &[String],
        ops: &[Value],
    ) -> Result<(), String> {
        let outside = || tr("組合せの要素の外の面").to_string();
        for op in ops {
            let name = op["op"].as_str().unwrap_or_default();
            let face = |key: &str| -> Result<(usize, usize), String> {
                let (k, i) = op[key]
                    .as_u64()
                    .and_then(|j| self.combo_face(spans, j as usize))
                    .ok_or_else(outside)?;
                if self.browser.combo.el(k).is_some_and(|p| p.locked) {
                    return Err(tr("ロックした要素は変えられない").into());
                }
                Ok((k, i))
            };
            let other = |key: &str| -> Result<Option<Value>, String> {
                match op.get(key) {
                    None | Some(Value::Null) => Ok(None),
                    Some(v) => {
                        let (k, i) = v
                            .as_u64()
                            .and_then(|j| self.combo_face(spans, j as usize))
                            .ok_or_else(outside)?;
                        Ok(Some(json!({ "block": ids[k], "surface": i })))
                    }
                }
            };
            let is_curv = |o: &Value| {
                lensbench_core::edit::solve_kind_parse(
                    o["kind"].as_str().unwrap_or("marginal_height"),
                    0.0,
                    Some(0),
                )
                .is_ok_and(|k| k.is_curv())
            };
            match name {
                "set_stop" => {
                    let j = op["surface"].as_u64().unwrap_or(0) as usize;
                    let c = &self.browser.combo;
                    let lead = c
                        .all()
                        .iter()
                        .all(|l| !c.get(l).is_some_and(|p| p.kind == combo::ElemKind::Stop));
                    match self.combo_face(spans, j) {
                        Some((k, i)) => {
                            let c = &mut self.browser.combo;
                            if c.el(k).is_some_and(|p| p.kind == combo::ElemKind::Stop) {
                                // 絞りの要素そのもの
                                c.set_stop_face(None);
                            } else {
                                let loc = c.chain()[k].loc.clone();
                                c.set_stop_face(Some((loc, i)));
                            }
                        }
                        // 前の絞り（要素の外）に戻す
                        None if lead
                            && self.ld_rows.get(j).is_some_and(|r| r.comment == "STOP") =>
                        {
                            self.browser.combo.set_stop_face(None)
                        }
                        None => return Err(outside()),
                    }
                }
                "unset_stop" => self.browser.combo.set_stop_face(None),
                "add_solve" => {
                    let (k, i) = face("surface")?;
                    let mut o = op.clone();
                    o["surface"] = json!(i);
                    if let Some(x) = other("other")? {
                        o["other"] = x;
                    }
                    let c = is_curv(&o);
                    let Some(p) = self.browser.combo.el_mut(k) else {
                        return Err(outside());
                    };
                    p.ops.retain(|q| {
                        !(q["op"] == "add_solve" && q["surface"] == json!(i) && is_curv(q) == c)
                    });
                    p.ops.push(o);
                }
                "remove_solve" => {
                    let (k, i) = face("surface")?;
                    let c = op["what"] == "c";
                    self.browser
                        .combo
                        .el_mut(k)
                        .ok_or_else(outside)?
                        .ops
                        .retain(|q| {
                            !(q["op"] == "add_solve" && q["surface"] == json!(i) && is_curv(q) == c)
                        });
                }
                "add_pickup" => {
                    let (k, i) = face("dst")?;
                    let mut o = op.clone();
                    o["dst"] = json!(i);
                    o["src"] = other("src")?.ok_or_else(outside)?;
                    let Some(p) = self.browser.combo.el_mut(k) else {
                        return Err(outside());
                    };
                    p.ops.retain(|q| {
                        !(q["op"] == "add_pickup" && q["dst"] == json!(i) && q["what"] == o["what"])
                    });
                    p.ops.push(o);
                }
                "remove_pickup" => {
                    let (k, i) = face("dst")?;
                    self.browser
                        .combo
                        .el_mut(k)
                        .ok_or_else(outside)?
                        .ops
                        .retain(|q| {
                            !(q["op"] == "add_pickup"
                                && q["dst"] == json!(i)
                                && q["what"] == op["what"])
                        });
                }
                _ => return Err(trf("組合せの系では {} は使えない", &[&name])),
            }
        }
        Ok(())
    }

    /// 組合せの系の面 j の値 key を、組合せの要素に当てる（要素の最後の面の厚さは後の空気、ほかは面の上書き）
    fn combo_surface_edit(
        &mut self,
        spans: &[Option<[usize; 2]>],
        j: usize,
        key: &str,
        value: Value,
    ) -> bool {
        let rows = &self.ld_rows;
        let k = spans
            .iter()
            .position(|s| s.is_some_and(|s| s[0] <= j && j <= s[1]));
        let x = value.as_f64();
        match (k, key, x) {
            // 前の絞りの厚さ = 絞りから最初の要素まで
            (None, "t", Some(x)) if x >= 0.0 && rows[j].stop => self.browser.combo.stop_gap = x,
            (Some(k), "t", Some(x)) if spans[k].is_some_and(|s| s[1] == j) => {
                // 要素の最後の面の厚さ = 後の空気（鏡の後は符号が逆）
                let air = if rows[j].t < 0.0 { -x } else { x };
                if air < 0.0 {
                    self.status = Status {
                        text: tr("間隔は 0 以上").into(),
                        error: true,
                    };
                    return false;
                }
                match self.browser.combo.gap_after(k) {
                    Some(v) => *v = air,
                    None => {
                        self.status = Status {
                            text: tr("この値は変えられない").into(),
                            error: true,
                        };
                        return false;
                    }
                }
            }
            (Some(k), _, _) => {
                let a = spans[k].map(|s| s[0]).unwrap_or(j);
                let rel = (a..j).filter(|&i| !rows[i].is_cb()).count();
                if let Some(p) = self.browser.combo.el_mut(k) {
                    p.set_edit(rel, key, value);
                }
            }
            _ => {
                self.status = Status {
                    text: tr("この値は変えられない").into(),
                    error: true,
                };
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
        let n = self.browser.combo.picks.len();
        if self.file.is_some()
            && !self.is_combo()
            && self.to_combo.is_none()
            && self.combo_len_prev == 0
            && n > 0
            && !self
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
            if let Some(p) = self.combo_path.clone() {
                self.worker.drop_queued();
                self.open(&p);
            }
        }
        if !self.s.auto_build
            || self.preview.busy()
            || self.browser.combo.picks.is_empty()
            || self.built_combo.as_ref() == Some(&self.browser.combo)
            || !self.browser.combo.settings_problems().is_empty()
        {
            return;
        }
        self.combo_edit = Some(web_time::Instant::now());
        self.build_combo(self.browser.combo.project());
    }

    /// 開いた系を組合せにする: 系の取り込み（zmx ブロック、.zmx も lensbench の .json も）1 つと、系の設定。keep は今の
    /// 組合せの要素をその後ろに残す。系の設定がまだ無ければ読みに行き、届いた後の画面でもう一度呼ぶ（#135）
    fn open_to_combo(&mut self, keep: bool) {
        let Some(f) = self.file.clone().filter(|_| !self.is_combo()) else {
            self.to_combo = None;
            self.open_insert = None;
            return;
        };
        let Some(d) = self.oss.data.clone() else {
            if self.oss.want() {
                self.worker.send(Tag::SysData, "system_data", json!({}));
            }
            return;
        };
        self.to_combo = None;
        if self.ld_dirty
            && !crate::files::confirm(
                tr("組合せにする"),
                tr("開いた系の保存していない変更は組合せに入りません（ファイルから取り込みます）。続けますか？"),
            )
        {
            if keep {
                // 足した要素は残し、開いた系はそのまま
                self.built_combo = Some(self.browser.combo.clone());
            }
            self.open_insert = None;
            return;
        }
        let obj_t = self.ld_rows.first().map_or(f64::INFINITY, |r| r.t);
        let mut c = Combo {
            name: f
                .file_stem()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_default(),
            ..Default::default()
        };
        if let Err(e) = c.take_system_data(&Value::Object(d), obj_t) {
            self.status = Status {
                text: e,
                error: true,
            };
            if keep {
                self.built_combo = Some(self.browser.combo.clone());
            }
            self.open_insert = None;
            return;
        }
        let mut z = combo::Pick::new(combo::ElemKind::ZmxRange);
        // 組合せは保存先から相対に読むので、開いた系は絶対パスで持つ（Web 版の置き場は初めから絶対）
        z.path = std::path::absolute(&f).unwrap_or(f).display().to_string();
        // 取り込みは像面の手前の面の厚さまで含むので、後の空気は 0
        z.air = 0.0;
        if let Some(cfg) = self
            .summary
            .as_ref()
            .and_then(|s| s["cfg"].as_u64())
            .filter(|&n| n > 1)
        {
            z.config = cfg as usize;
        }
        // 開いた系の玉・群（最上位のブロック）を取り込みの中の群として残す（取り込み全体が 1 つのブロックになり、
        // その中に元の玉が並ぶ）
        z.zgroups = open_groups(&self.ld_blocks);
        match self.open_insert.take() {
            Some((after, p)) => c.picks.extend(split_open(z, &self.ld_rows, after, p)),
            None => c.picks.push(z),
        }
        if keep {
            let old = std::mem::take(&mut self.browser.combo);
            c.picks.extend(old.picks);
            c.mechanics = old.mechanics;
        }
        self.browser.combo = c;
        self.combo_view = combo::View::default();
        self.combo_len_prev = self.browser.combo.picks.len();
        self.doc_path = None;
        self.status = Status {
            text: tr("開いた系を組合せの要素にした（系の取り込み）").into(),
            error: false,
        };
        self.build_combo(self.browser.combo.project());
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
            self.status = Status {
                text: tr("軸上の光線が像面の前の面まで届かない").into(),
                error: true,
            };
            return;
        }
        match combo::paraxial_focus(y, z, m, n) {
            Some(t) => {
                if let Some(v) = self.browser.combo.image_gap() {
                    *v = t;
                }
                self.status = Status {
                    text: trf(
                        "像面を近軸の焦点へ（最後の空気 {} mm）",
                        &[&fmt_num((t * 1e4).round() / 1e4)],
                    ),
                    error: false,
                };
                if !self.s.auto_build {
                    self.build_combo(self.browser.combo.project());
                }
            }
            None => {
                self.status = Status {
                    text: tr("軸上の光線が軸と交わらない（焦点が無い）").into(),
                    error: true,
                }
            }
        }
    }

    fn catalog_ui(&mut self, ui: &mut Ui) {
        if !self.browser.loaded {
            self.refresh_catalog();
        }
        // ダブルクリックで足す玉は、組合せで選んでいる要素の直後へ
        self.browser.insert_after = self
            .combo_view
            .sel
            .filter(|&k| k < self.browser.combo.len());
        self.browser.lens_hide.clone_from(&self.s.lens_hide);
        let n_old = self.browser.combo.picks.len();
        let act = self
            .browser
            .ui(ui, self.s.glass_dirs.len(), self.s.lens_dirs.len());
        self.s.lens_hide.clone_from(&self.browser.lens_hide);
        if let Some(k) = self.browser.added.take() {
            if self.file.is_some() && !self.is_combo() {
                // 組合せでない系を開いているときは開いた系に足す: レンズデータで選んでいる面の後（空気の間隔なら）、
                // 無ければ系の後ろ。以前は前の組合せが残っていると、足した玉は前の組合せに入り、開いた系が前の組合せの
                // 系に置き換わっていた
                if let Some(p) = self.browser.combo.el(k).cloned() {
                    let after = self
                        .ld
                        .cur
                        .map(|(j, _)| j)
                        .filter(|&j| open_gap_ok(&self.ld_rows, j));
                    if self.insert_into_open(after, p, n_old) {
                        self.browser.combo.remove_chain(k);
                    } else {
                        // 置き換えない: 前の組合せに足したまま、開いた系は作り直さない
                        self.built_combo = Some(self.browser.combo.clone());
                        self.combo_view.select(k);
                        self.status = Status {
                            text: tr("組合せに足した（開いた系はそのまま。「▶ 組合せの系を作る」で作る）").into(),
                            error: false,
                        };
                    }
                }
            } else {
                self.combo_view.select(k);
            }
        }
        self.lens_preview();
        match act {
            Some(catalog::Action::Refresh) => self.refresh_catalog(),
            Some(catalog::Action::Settings) => self.folders_open = true,
            Some(catalog::Action::SysSettings) => self.cmds.push(Cmd::FocusKind(Kind::SysSettings)),
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
        let path = lensbench_core::vfs::temp_dir()
            .join("lensbench-gui")
            .join("catalog-preview.json");
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
        let Some(sum) = self.summary.clone() else {
            ui.add_space(8.0);
            ui.label(tr(
                "系のファイル（.zmx・プロジェクトの .json）を開くか、窓に落とします。",
            ));
            ui.label(tr(
                "カタログレンズを組み合わせるときは「カタログ」パネルへ。",
            ));
            if ui.button(tr("開く…（Ctrl+O）")).clicked() {
                self.pick_and_open();
            }
            if !self.s.recent.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(tr("最近のファイル")).small().color(weak));
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
                    (tr("焦点距離"), self.info_num("effl"), "mm", 1e3),
                    (tr("作動 F 値"), self.info_num("wfno"), "", 1e3),
                ],
                [
                    (tr("像空間 NA"), self.info_num("image_na"), "", 1e4),
                    (tr("全長"), self.info_num("total_track"), "mm", 1e3),
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
                    let mut row = |k: &str, v: RichText| {
                        ui.label(RichText::new(k).color(weak));
                        ui.label(v);
                        ui.end_row();
                    };
                    let n = |x: Option<f64>| {
                        x.map(|x| fmt_num((x * 1e4).round() / 1e4))
                            .unwrap_or("—".into())
                    };
                    row(tr("入射瞳径 [mm]"), RichText::new(n(self.info_num("epd"))));
                    row(
                        tr("近軸像高 [mm]"),
                        RichText::new(n(self.info_num("paraxial_image_height"))),
                    );
                    row(tr("面の数"), RichText::new(format!("{}", sum["surfaces"])));
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
                    row(tr("視野"), RichText::new(fields.join(" ")));
                });
            // 波長（色付き）
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("波長 [µm]")).color(weak));
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
            ui.label(RichText::new(tr("パラメータ")).small().strong().color(weak));
            ui.label(
                RichText::new(tr(
                    "値を変えると、開いている解析をすぐ計算し直します（↑↓ キーでも動かせます）",
                ))
                .small()
                .color(weak),
            );
            let mut remove = None;
            for (i, p) in self.s.params.clone().iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(p).monospace());
                    match self.param_vals.get(p).copied() {
                        Some(mut x) => {
                            let step = (x.abs() * 1e-3).max(1e-3);
                            let r =
                                ui.add(egui::DragValue::new(&mut x).speed(step).max_decimals(8));
                            if r.changed() {
                                self.param_vals.insert(p.clone(), x);
                                self.pending_set.insert(p.clone(), x);
                            }
                        }
                        None => {
                            ui.weak("—");
                        }
                    }
                    if ui.small_button("✖").on_hover_text(tr("外す")).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                let p = self.s.params.remove(i);
                self.param_vals.remove(&p);
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
                if (ui.button(tr("＋ パラメータを足す")).clicked() || enter)
                    && !self.new_param.trim().is_empty()
                {
                    let p = self.new_param.trim().to_string();
                    self.worker.send(Tag::Get, "get", json!({ "param": p }));
                    self.new_param.clear();
                }
            });
            ui.label(
                RichText::new(tr(
                    "名前は Python・serve と同じ（3.t、STOP.r、G1.shift など）",
                ))
                .small()
                .color(weak),
            );
            if ui.button(tr("元に戻す（読み直す）")).clicked() {
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
            .filter(|s| s.name != "save" && !object.is_some_and(|o| s.name == o.arg))
            .filter(|s| !(sel && (s.name == "field" || s.name == "wave")))
            .filter(|s| {
                !(fig
                    && matches!(
                        (cfg.cmd.as_str(), s.name.as_str()),
                        ("wavefront", "map") | ("fringe" | "pop", "grid")
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
        let head = if set > 0 {
            trf("⚙ 設定（{} 個を変更）", &[&set])
        } else {
            tr("⚙ 設定").to_string()
        };
        let mut rerun = false;
        if ui
            .selectable_label(cfg.args_open, head)
            .on_hover_text(tr("解析の設定の欄を開く・閉じる"))
            .clicked()
        {
            cfg.args_open = !cfg.args_open;
        }
        if set > 0
            && ui
                .small_button("↺")
                .on_hover_text(tr("すべて既定に"))
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
            ui.weak(tr("この解析に設定はありません"));
        } else if argform::form(ui, ("a", id), &shown, &mut cfg.args) {
            rerun = true;
        }
        if let Some(o) = object {
            if self.object_ui(ui, id, cfg, &o) {
                rerun = true;
            }
        }
        let extra_set = !cfg.extra.trim().is_empty();
        egui::CollapsingHeader::new(RichText::new(tr("詳細（JSON で引数を足す）")).small().color(weak))
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
                        "CLI と同じ引数を JSON で（キーの _ は -。docs/serve.md の analyze）。欄の値より優先",
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
                .small_button(tr("JSON を読む…"))
                .on_hover_text(tr("欄と同じ項目の JSON"))
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
                                self.status = Status {
                                    text: trf(
                                        "{}: 知らない項目 {}",
                                        &[&p.display(), &unknown.join(", ")],
                                    ),
                                    error: true,
                                };
                            }
                        }
                        Err(e) => {
                            self.status = Status {
                                text: format!("{}: {e}", p.display()),
                                error: true,
                            }
                        }
                    }
                }
            }
            if ui.small_button(tr("JSON に保存…")).clicked() {
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
                                self.status = Status {
                                    text: format!("{}: {e}", p.display()),
                                    error: true,
                                };
                            }
                        }
                    }
                    Err(e) => {
                        self.status = Status {
                            text: e,
                            error: true,
                        }
                    }
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
        groups.push((String::new(), tr("その他"), rest));
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
                        .hint_text(tr("絞り込む…"))
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
                    ui.weak(tr("合う解析がありません"));
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
        if self.summary.is_none() {
            ui.weak(tr("系を開くと解析できます。"));
            return;
        }
        let nf = self.n_fields();
        let waves = self.waves_um();
        let fields: Vec<String> = self
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
                    Some(f) => trf("視野 {} {}", &[&i, f]),
                    None => trf("視野 {}", &[&i]),
                };
                egui::ComboBox::from_id_salt(("field", id))
                    .selected_text(if cfg.field == 0 {
                        tr("全視野").to_string()
                    } else {
                        fname(cfg.field)
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut cfg.field, 0, tr("全視野"));
                        for i in 1..=nf {
                            ui.selectable_value(&mut cfg.field, i, fname(i));
                        }
                    });
                let wname = |i: usize| match waves.get(i - 1) {
                    Some(w) => trf("波長 {} {} µm", &[&i, &fmt_num(*w)]),
                    None => trf("波長 {}", &[&i]),
                };
                egui::ComboBox::from_id_salt(("wave", id))
                    .selected_text(if cfg.wave == 0 {
                        tr("多色").to_string()
                    } else {
                        wname(cfg.wave)
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut cfg.wave, 0, tr("多色"));
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
            ui.checkbox(&mut cfg.live, tr("自動"))
                .on_hover_text(tr("系の値が変わったら計算し直す"));
            if ui
                .button(tr("▶ 計算"))
                .on_hover_text(tr(
                    "この解析を今の設定で計算する（F5 は開いている解析すべて）",
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
                && (matches!(cfg.cmd.as_str(), "spot" | "wavefront" | "fringe" | "pop")
                    || crate::ext::get(&cfg.cmd).is_some_and(|a| a.figure_params.is_some())))
        {
            self.run(id, cfg);
        }
    }

    /// 解析の結果（見せ方の選択・時間・書き出しと、図・表・JSON）
    fn result_ui(&mut self, ui: &mut Ui, id: u64, cfg: &mut AnaCfg) {
        let weak = theme::weak_text(ui.visuals());
        let view_row = |ui: &mut Ui, cfg: &mut AnaCfg| {
            ui.selectable_value(&mut cfg.view, View::Figure, tr("図"))
                .on_hover_text(tr("結果を図で見る（図の無い解析は表）"));
            ui.selectable_value(&mut cfg.view, View::Table, tr("表"))
                .on_hover_text(tr("結果を表と木で見る"));
            ui.selectable_value(&mut cfg.view, View::Json, "JSON")
                .on_hover_text(tr("結果の JSON をそのまま見る（CLI の --json と同じ形）"));
        };
        let Some(r) = self.results.get(&id) else {
            ui.horizontal(|ui| view_row(ui, cfg));
            if !self.worker.busy() && !self.relay_busy {
                self.run(id, cfg);
            }
            return;
        };
        if let Some(e) = &r.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        let Some(v) = &r.value else {
            ui.horizontal(|ui| view_row(ui, cfg));
            ui.spinner();
            return;
        };
        ui.horizontal(|ui| {
            view_row(ui, cfg);
            ui.separator();
            ui.label(
                RichText::new(format!("{} ms", fmt_num((r.ms * 10.0).round() / 10.0)))
                    .small()
                    .color(weak),
            );
            if r.cmd != cfg.cmd {
                ui.label(RichText::new(tr("（前の解析の結果）")).small().color(weak));
            }
            result_export_menu(ui, &r.cmd, v);
        });
        let fig = cfg.view == View::Figure && figures::has_figure(&r.cmd);
        // 図はパネルの残りに合わせるが最低の高さがあるので、パネルが低いときは縦にスクロールして見る
        if fig && (r.cmd == "mtf" || r.cmd == "huygens-mtf") {
            egui::ScrollArea::vertical()
                .id_salt(("fig", id))
                .auto_shrink([false, false])
                .show(ui, |ui| plots::mtf(ui, v, r.prev.as_ref(), id));
            return;
        }
        // スポット以外の図（描けない結果なら表）
        if fig && r.cmd != "spot" {
            let shown = egui::ScrollArea::vertical()
                .id_salt(("fig", id))
                .auto_shrink([false, false])
                .show(ui, |ui| figures::show(ui, id, &r.cmd, v, r.prev.as_ref()))
                .inner;
            if shown {
                return;
            }
        }
        egui::ScrollArea::both()
            .id_salt(("res", id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if fig && r.cmd == "spot" {
                    let waves = self.waves_um();
                    let sys = SysInfo {
                        waves_um: &waves,
                        primary: self.primary(),
                        wfno: self.info_num("wfno"),
                    };
                    plots::spot(ui, &r.rays, v, r.prev.as_ref(), &sys);
                    ui.separator();
                    json_view::show(ui, v);
                } else if cfg.view == View::Json {
                    let mut t = serde_json::to_string_pretty(v).unwrap_or_default();
                    ui.add(
                        egui::TextEdit::multiline(&mut t)
                            .code_editor()
                            .desired_width(f32::INFINITY),
                    );
                } else {
                    json_view::show(ui, v);
                }
            });
    }

    fn folders_window(&mut self, ctx: &egui::Context) {
        let mut open = self.folders_open;
        let mut changed = false;
        egui::Window::new(tr("カタログのフォルダ"))
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                changed |= dir_list(
                    ui,
                    tr("ガラスカタログ（.AGF のフォルダ）"),
                    &mut self.s.glass_dirs,
                );
                ui.separator();
                changed |= dir_list(
                    ui,
                    tr("レンズカタログ（.zmx を入れたフォルダ。メーカーはフォルダの名前、型番はファイル名）"),
                    &mut self.s.lens_dirs,
                );
                ui.separator();
                ui.label(
                    RichText::new(tr("系を開くとき・カタログを読むときに使います。変えたら系は読み直します。")).small().weak(),
                );
                ui.separator();
                #[cfg(not(target_arch = "wasm32"))]
                {
                    cad_store_ui(ui, &mut self.s.cad_store);
                    ui.separator();
                }
                // 設定の保存先
                let at = crate::portable::location()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| tr("（保存しない）").to_string());
                let kind = if crate::portable::base().is_some() {
                    tr("設定の保存先（ポータブル。フォルダの中のパスは相対で保存）")
                } else {
                    tr("設定の保存先（OS の場所。exe の隣に lensbench.portable を置くとポータブル）")
                };
                ui.label(RichText::new(format!("{kind}: {at}")).small().weak());
            });
        self.folders_open = open;
        if changed {
            self.refresh_catalog();
            self.reload();
        }
    }

    /// ユーザーカタログに保存する窓。保存は読み込んだ組合せの系から（解析と同じ worker）、一覧・変更は下書きの worker
    fn cat_save_window(&mut self, ctx: &egui::Context) {
        let ready = self.is_combo() && self.built_combo.as_ref() == Some(&self.browser.combo);
        let req = self
            .cat_save
            .window(ctx, &self.browser.combo, &self.browser.lenses, ready);
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
            self.fill.start(r);
        }
        if let Some(e) = self.auto_fill.take_timing_error() {
            self.status = Status {
                text: e,
                error: true,
            };
        }
        // 補う窓が開いている間は裏で読まない
        if !self.fill.open {
            let sel = self
                .browser
                .lens_sel
                .and_then(|i| self.browser.lenses.get(i))
                .cloned();
            match self.auto_fill.tick(ctx, sel.as_ref()) {
                Some(crate::fill::AutoOut::Save(p)) => {
                    self.preview.send(Tag::CatalogFill, "catalog_fill", p);
                }
                #[cfg(windows)]
                Some(crate::fill::AutoOut::Show(b)) => {
                    let (r, child, text) = *b;
                    self.fill.adopt(r, child, text);
                }
                None => {}
            }
        }
        if !self.fill.open {
            return;
        }
        if let Some(p) = self.fill.window(ctx) {
            self.preview.send(Tag::CatalogFill, "catalog_fill", p);
        }
    }

    fn help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.help_open;
        egui::Window::new(tr("キーボード操作"))
            .open(&mut open)
            .default_width(460.0)
            .show(ctx, |ui| {
                egui::Grid::new("keys").striped(true).show(ui, |ui| {
                    for (k, v) in [
                        ("Ctrl+K", tr("コマンドパレット（操作・解析・パネル・並びを名前で探す）")),
                        ("Ctrl+N", tr("新規（組合せを空にする）")),
                        ("Ctrl+O", tr("系のファイルを開く")),
                        ("F5", tr("開いている解析をすべて計算し直す")),
                        ("Ctrl+R", tr("系を読み直す（変えた値を捨てる）")),
                        ("Ctrl+Z", tr("系の変更を元に戻す（100 段まで。入力欄の中では欄の文字）")),
                        ("Ctrl+Y / Ctrl+Shift+Z", tr("元に戻した変更をやり直す")),
                        ("Ctrl+Tab", tr("次のパネルへ（Shift で前へ）")),
                        ("Tab / Shift+Tab", tr("パネルの中の入力欄・ボタンを順に移る")),
                        ("Enter / Space", tr("ボタンを押す・選ぶ")),
                        ("↑ ↓", tr("数の欄で値を増減（パラメータ・解析の設定は解析をすぐ計算し直す）")),
                        ("Ctrl+A", tr("多重構成の次の構成（Shift で前へ）")),
                        ("Enter", tr("解析の設定の欄: 打った値で計算し直す")),
                        (tr("文字"), tr("解析の種類の一覧: 打って絞り込む（Enter で最初の候補）")),
                        ("F1", tr("この一覧")),
                        ("Esc", tr("パレット・窓を閉じる")),
                    ] {
                        ui.label(RichText::new(k).monospace().strong());
                        ui.label(v);
                        ui.end_row();
                    }
                });
                for (title, keys) in [
                    (
                        tr("レンズデータ"),
                        vec![
                            ("← → ↑ ↓ / Tab", tr("セルを移る（Shift+↑ ↓・Shift+クリックで行の範囲）")),
                            ("Enter / F2", tr("書き換え（文字を打っても始まる）。Enter で決め、Esc で取り消し")),
                            ("Insert", tr("今の面の前に面を挿入")),
                            ("Alt+Insert", tr("今の面の後に面を挿入")),
                            ("Delete", tr("選んだ面を消す")),
                            ("Ctrl+C", tr("選んだ行をタブ区切りでコピー")),
                            ("Ctrl+V", tr("今のセルから上書きで貼り付け")),
                            ("Ctrl+Shift+V", tr("写した行の数だけ面を挿入して貼り付け")),
                            ("Shift+F10", tr("右クリックの欄")),
                            ("Ctrl+L", tr("ブロックのロック")),
                            ("Enter / Backspace", tr("ブロックの中へ・外へ")),
                        ],
                    ),
                    (
                        tr("多重構成"),
                        vec![
                            ("← → ↑ ↓ / Tab", tr("セルを移る")),
                            ("Enter / F2", tr("書き換え（文字を打っても始まる）")),
                            ("Delete", tr("選んだ行を消す（列の見出しを選んでいれば構成を消す）")),
                            ("Ctrl+C / Ctrl+V", tr("行のコピー・上書きで貼り付け")),
                            ("Shift+F10", tr("右クリックの欄")),
                        ],
                    ),
                ] {
                    ui.separator();
                    ui.strong(title);
                    egui::Grid::new(("keys", title)).striped(true).show(ui, |ui| {
                        for (k, v) in keys {
                            ui.label(RichText::new(k).monospace().strong());
                            ui.label(v);
                            ui.end_row();
                        }
                    });
                }
                ui.separator();
                ui.label(
                    RichText::new(tr("パネルはタブを引きずって並べ替え・分割・フロートできます。タブを右クリックすると別の窓（別モニター）に出せます。")).small(),
                );
            });
        self.help_open = open;
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
                role: String::new(),
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

fn combo_read_only(
    is_combo: bool,
    rows: &[lens_data::Row],
    spans: &[Option<[usize; 2]>],
    picks: &[combo::Pick],
    lead: bool,
    j: usize,
    col: lens_data::Col,
) -> Option<String> {
    use lens_data::Col;
    if !is_combo {
        return None;
    }
    let img = rows.len().saturating_sub(1);
    if j == 0 || j >= img {
        return Some(tr("物体面・像面は組合せの設定で").into());
    }
    let Some(k) = spans
        .iter()
        .position(|s| s.is_some_and(|s| s[0] <= j && j <= s[1]))
    else {
        if lead && rows[j].stop && col == Col::T {
            return None;
        }
        return Some(tr("組合せの要素の外の面（前の絞りは厚さだけ）").into());
    };
    if picks.get(k).is_some_and(|p| p.fixed())
        && col == Col::T
        && spans[k].is_some_and(|s| s[1] == j)
    {
        return Some(tr("位置もロックした要素の後の間隔は変えられない").into());
    }
    if picks
        .get(k)
        .is_some_and(|p| p.kind == combo::ElemKind::Splitter)
    {
        return Some(tr("BS は組合せの要素の設定で").into());
    }
    let end = spans[k].map(|s| s[1]).unwrap_or(j);
    if col == Col::T && j == end {
        return None;
    }
    if rows[j].is_cb() {
        return Some(tr("偏心・傾きは組合せの要素の設定で").into());
    }
    None
}

/// 品の STEP の共有の置き場の欄（カタログのフォルダの窓）。変えたらすぐ使う
#[cfg(not(target_arch = "wasm32"))]
fn cad_store_ui(ui: &mut Ui, st: &mut CadStore) {
    ui.label(tr("品の STEP の置き場（共有）"));
    let before = st.clone();
    egui::Grid::new("cad_store").num_columns(2).show(ui, |ui| {
        ui.label(tr("URL"));
        ui.add(
            egui::TextEdit::singleline(&mut st.url)
                .hint_text("https://….pages.dev/cad")
                .desired_width(360.0),
        );
        ui.end_row();
        ui.label(tr("Client ID"));
        ui.add(egui::TextEdit::singleline(&mut st.id).desired_width(360.0));
        ui.end_row();
        ui.label(tr("Client Secret"));
        ui.add(
            egui::TextEdit::singleline(&mut st.secret)
                .password(true)
                .desired_width(360.0),
        );
        ui.end_row();
    });
    ui.label(
        RichText::new(tr(
            "品の STEP を取るとき、配布元より先にここを見ます。配布元から取った STEP はここにも上げます。Client ID・Secret は Cloudflare Access のサービストークン（守っていない口なら空）。空の URL は環境変数 LENSBENCH_CAD_STORE を使います。",
        ))
        .small()
        .weak(),
    );
    if lensbench_cli::cad::has_default_store() && st.url.trim().is_empty() {
        ui.label(
            RichText::new(tr("空のときは、この版に組み込んだ置き場を使います。"))
                .small()
                .weak(),
        );
    }
    if *st != before {
        st.apply();
    }
}

fn dir_list(ui: &mut Ui, title: &str, dirs: &mut Vec<String>) -> bool {
    let mut changed = false;
    ui.label(RichText::new(title).strong());
    let mut remove = None;
    for (i, d) in dirs.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.button("✖").on_hover_text(tr("外す")).clicked() {
                remove = Some(i);
            }
            let ok = lensbench_core::vfs::is_dir(d);
            ui.label(if ok {
                RichText::new(d)
            } else {
                RichText::new(trf("{}（見つからない）", &[d])).color(Color32::RED)
            });
        });
    }
    if let Some(i) = remove {
        dirs.remove(i);
        changed = true;
    }
    let start = ui.button(tr("➕ フォルダを足す…")).clicked();
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
        let label = detach_label(cfg!(target_arch = "wasm32"));
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
    let mut v = vec![
        Kind::System,
        Kind::Catalog,
        Kind::Layout,
        Kind::SysSettings,
        Kind::LensData,
        Kind::MultiConfig,
        Kind::Optimize,
        Kind::Swap,
        Kind::Tolerance,
        Kind::Zoom,
    ];
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
            s.lang = Lang::Ja;
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
        let browser = Browser {
            combo: s.combo.clone(),
            ..Default::default()
        };
        // 系のファイルを指定して開いたときは、前の組合せを作り直さない（指定が無ければ前の組合せで系を作る）
        let built_combo = args.open.is_some().then(|| s.combo.clone());
        let lay = s.layout.take().unwrap_or_else(|| Preset::Analysis.build());
        let next_id = lay.max_id();
        let mut app = App {
            c: Core {
                s,
                samples: args.samples.clone(),
                worker,
                preview,
                load_stale: false,
                preview_req: None,
                status: Status {
                    text: args.notice.as_ref().map(|n| n.text()).unwrap_or_default(),
                    error: false,
                },
                file: None,
                summary: None,
                info: None,
                results: HashMap::new(),
                param_vals: HashMap::new(),
                new_param: String::new(),
                pending_set: Default::default(),
                rerun: None,
                combo_edit: None,
                hist: (0, 0),
                hist_list: Default::default(),
                combo_hist: Default::default(),
                new_undo: None,
                new_redo: None,
                redo_new_pending: false,
                ld: Default::default(),
                ld_rows: Vec::new(),
                ld_units: Vec::new(),
                um: Default::default(),
                um_open: false,
                ld_blocks: Vec::new(),
                ld_glasses: Vec::new(),
                ld_dirty: false,
                mc: Default::default(),
                oss: Default::default(),
                cat_save: CatSave::default(),
                fill: Default::default(),
                auto_fill: Default::default(),
                opt: crate::optim_panel::OptPanel::new(),
                swap: Default::default(),
                tol: Default::default(),
                zoom: Default::default(),
                swap_ids: Vec::new(),
                swap_file: None,
                rev: 0,
                relay: None,
                relay_busy: false,
                browser,
                folders_open: false,
                help_open: false,
                save_layout_open: false,
                layout_name: String::new(),
                built: None,
                doc_path: None,
                built_combo,
                combo_path: None,
                layout_data: None,
                overlay: Overlay::default(),
                rays_open: false,
                agf_names: Default::default(),
                combo_view: Default::default(),
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
            child: None,
            #[cfg(target_arch = "wasm32")]
            viewer: None,
            #[cfg(target_arch = "wasm32")]
            hosts: HashMap::new(),
        };
        app.apply_dock_translations();
        // Web 版: 別のタブのパネルなら、計算しないで元のタブの結果を描く
        #[cfg(target_arch = "wasm32")]
        if let Some((id, sid)) = panel {
            if crate::web_tabs::remote_of_location() {
                app.viewer = Some(crate::web_tabs::Viewer::new(&cc.egui_ctx, id, sid));
                return app;
            }
            app.c.relay = Some(false);
            app.child = Some((crate::web_tabs::Child::new(&cc.egui_ctx, id, sid), None));
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
            app.tour = Some(Tour::new(d.clone()));
            if args.open.is_none() {
                app.tour
                    .as_mut()
                    .unwrap()
                    .errors
                    .push("--tour には系のファイルが要る".into());
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
        t.tab_context_menu.close_button = tr("閉じる").into();
        t.tab_context_menu.eject_button = tr("フロートにする").into();
        t.leaf.close_all_button = tr("窓を閉じる").into();
        t.leaf.minimize_button = tr("たたむ").into();
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

    /// Web 版（元のタブ）: 別の窓に出した解析のパネルを別のタブで開き、設定・結果をやりとりする。
    /// タブを閉じたらドックに戻す
    #[cfg(target_arch = "wasm32")]
    fn web_tabs(&mut self, ctx: &egui::Context) {
        use crate::web_tabs::Ev;
        let Some(t) = self.tabs.as_mut() else {
            return;
        };
        t.retain(&self.windows.iter().map(|w| w.pane.id).collect::<Vec<_>>());
        self.hosts
            .retain(|id, _| self.windows.iter().any(|w| w.pane.id == *id));
        for w in &self.windows {
            let id = w.pane.id;
            let remote = !matches!(w.pane.kind, Kind::Analysis(_));
            if !t.is_open(id) && !t.blocked.contains(&id) && !t.open(id, remote, w.size) {
                self.c.status = Status {
                    text: tr("ブラウザが別のタブ・窓を開くのを止めました（ポップアップを許すと、パネルを別のタブ・窓に出せます）。本体の中の窓に出します").into(),
                    error: true,
                };
            }
        }
        for e in t.events() {
            match e {
                Ev::Hello(id) => {
                    self.hosts.insert(id, crate::web_tabs::Host::new(ctx));
                }
                Ev::Input(id, v) => {
                    self.hosts
                        .entry(id)
                        .or_insert_with(|| crate::web_tabs::Host::new(ctx))
                        .input(&v);
                }
                Ev::Cfg(id, cfg, run) => {
                    if let Some(w) = self.windows.iter_mut().find(|w| w.pane.id == id) {
                        w.pane.kind = Kind::Analysis(cfg.clone());
                        if run {
                            self.c.run(id, &cfg);
                        }
                    }
                }
                Ev::Gone(id) => {
                    self.hosts.remove(&id);
                    if let Some(k) = self.windows.iter().position(|w| w.pane.id == id) {
                        let w = self.windows.remove(k);
                        self.win_init.remove(&id);
                        crate::layout::redock(&mut self.dock, w.pane, w.home.as_ref());
                    }
                }
            }
        }
        let busy = self.c.worker.busy();
        for w in &self.windows {
            if let Kind::Analysis(cfg) = &w.pane.kind {
                if t.stale(w.pane.id, self.c.rev, busy) {
                    let snap = self.c.tab_snapshot(w.pane.id, cfg);
                    t.send(w.pane.id, self.c.rev, busy, snap);
                }
            }
        }
        // 解析以外のパネル: 元のタブで描いて、変わった画面を送る
        let App {
            c, windows, hosts, ..
        } = self;
        for w in windows.iter_mut() {
            let id = w.pane.id;
            let Some(h) = hosts.get_mut(&id) else {
                continue;
            };
            if !t.is_open(id) || matches!(w.pane.kind, Kind::Analysis(_)) {
                continue;
            }
            let pane = &mut w.pane;
            h.title = pane.title();
            let (frame, out) = h.frame(ctx, |ui| c.pane_ui(ui, pane));
            if let Some(f) = frame {
                t.send_frame(id, &f);
            }
            if let Some(o) = out {
                t.send_out(id, o);
            }
        }
        // 子のタブを閉じたのに気づく
        if t.any_open() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// Web 版（別のタブのパネル）: 元のタブから届いた結果を描き、設定の変更・計算の依頼を返す
    #[cfg(target_arch = "wasm32")]
    fn child_update(&mut self, ctx: &egui::Context) {
        let App { c, child, .. } = self;
        let Some((ch, pane)) = child.as_mut() else {
            return;
        };
        if let Some(v) = ch.poll(ctx, pane.is_some()) {
            if pane.is_none() {
                if let Ok(cfg) = serde_json::from_value::<AnaCfg>(v["cfg"].clone()) {
                    ch.got(&cfg);
                    *pane = Some(Pane {
                        id: ch.id,
                        kind: Kind::Analysis(cfg),
                    });
                }
            }
            c.apply_snapshot(ch.id, &v);
        }
        let title = pane.as_ref().map(|p| p.title()).unwrap_or_default();
        crate::web_tabs::set_title(&format!("lensbench — {title}"));
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&title).strong());
                if let Some(f) = c.file.as_ref().and_then(|f| f.file_name()) {
                    ui.label(RichText::new(f.to_string_lossy()).small().weak());
                }
                if ui.small_button(tr("元のタブに戻す")).clicked() {
                    crate::web_tabs::close_this();
                }
            });
            if ch.closed {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    tr("元のタブを閉じたので、このパネルは計算できません。この窓を閉じて、元のページを開き直してください。"),
                );
            }
            if !ch.closed && ch.stalled(ctx) {
                ui.label(
                    RichText::new(tr("元のタブの返事を待っています。元のタブが見えていないと、ブラウザはそのタブを止めます。元のタブとこの窓を並べて使ってください。"))
                        .small()
                        .color(ui.visuals().warn_fg_color),
                );
            }
            ui.separator();
            match pane.as_mut() {
                Some(p) => c.pane_ui(ui, p),
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr("元のタブを待っています…"));
                    });
                }
            }
        });
        if let Some(Pane {
            kind: Kind::Analysis(cfg),
            ..
        }) = pane.as_ref()
        {
            let run = c.relay.replace(false) == Some(true);
            if !ch.closed && ch.send(cfg, run) && run {
                c.relay_busy = true;
            }
        }
    }

    fn exec(&mut self, ctx: &egui::Context, cmd: Cmd) {
        match cmd {
            Cmd::New => self.c.new_system(),
            Cmd::Undo(back, n) => self.c.undo(back, n),
            Cmd::Open => self.c.pick_and_open(),
            Cmd::Recent(f) => self.c.open_doc(Path::new(&f)),
            Cmd::Reload => self.c.reload(),
            Cmd::RunAll => self.run_all(false),
            Cmd::WriteZmx => self.c.write_zmx(),
            Cmd::Save(as_new) => self.c.save(as_new),
            Cmd::Add(k) => {
                let id = self.add_pane(k);
                self.focus(id);
            }
            Cmd::Focus(id) => self.focus(id),
            Cmd::FocusKind(k) => {
                let same = |x: &Kind| std::mem::discriminant(x) == std::mem::discriminant(&k);
                let id = match self.pane_of(same) {
                    Some(id) => id,
                    None => self.add_pane(k),
                };
                self.focus(id);
            }
            Cmd::ToWindow(id) => self.detach(id),
            Cmd::Preset(p) => self.set_layout(p.build()),
            Cmd::Layout(i) => {
                if let Some((_, l)) = self.c.s.layouts.get(i) {
                    let l = l.clone();
                    self.set_layout(l);
                }
            }
            Cmd::SaveLayout => {
                self.c.save_layout_open = true;
            }
            Cmd::Theme(m) => {
                self.c.s.look.mode = m;
                theme::apply(ctx, &self.c.s.look);
            }
            Cmd::Accent(i) => {
                self.c.s.look.accent = theme::ACCENTS[i % theme::ACCENTS.len()];
                theme::apply(ctx, &self.c.s.look);
            }
            Cmd::Lang(l) => {
                self.c.s.lang = l;
                i18n::set(l);
                self.apply_dock_translations();
            }
            Cmd::Folders => self.c.folders_open = true,
            Cmd::Help => self.c.help_open = true,
        }
    }

    /// パレットの項目
    fn palette_items(&self) -> Vec<Item<Cmd>> {
        let mut v = Vec::new();
        let mut push = |label: String, hint: &str, cmd: Cmd| {
            v.push(Item {
                label,
                hint: hint.into(),
                cmd,
            })
        };
        push(
            tr("新規（組合せを空にする）").into(),
            "new ctrl+n",
            Cmd::New,
        );
        push(
            tr("系のファイルを開く…").into(),
            "open file ctrl+o",
            Cmd::Open,
        );
        push(
            tr("元に戻す（Ctrl+Z）").into(),
            "undo ctrl+z",
            Cmd::Undo(true, 1),
        );
        push(
            tr("やり直す（Ctrl+Y）").into(),
            "redo ctrl+y ctrl+shift+z",
            Cmd::Undo(false, 1),
        );
        push(tr("系を読み直す").into(), "reload", Cmd::Reload);
        push(
            tr("開いている解析をすべて計算し直す").into(),
            "run all f5",
            Cmd::RunAll,
        );
        push(
            tr(".zmx に書き出す…").into(),
            "write zmx export",
            Cmd::WriteZmx,
        );
        push(tr("保存（Ctrl+S）").into(), "save ctrl+s", Cmd::Save(false));
        push(
            tr("名前を付けて保存…（Ctrl+Shift+S）").into(),
            "save as ctrl+shift+s",
            Cmd::Save(true),
        );
        for k in add_kinds() {
            let t = Pane {
                id: 0,
                kind: k.clone(),
            }
            .title();
            let hint = match &k {
                Kind::Analysis(a) => format!("add panel {}", a.cmd),
                Kind::System => "add panel system".into(),
                Kind::Catalog => "add panel catalog".into(),
                Kind::Layout => "add panel layout combo".into(),
                Kind::LensData => "add panel lens data surfaces table".into(),
                Kind::MultiConfig => "add panel multi configuration editor mce".into(),
                Kind::Optimize => "add panel optimize optimization merit function dls".into(),
                Kind::Swap => "add panel swap search catalog lens replace".into(),
                Kind::Tolerance => "add panel tolerance monte carlo sensitivity".into(),
                Kind::Zoom => "add panel zoom cam track states focal length".into(),
                Kind::SysSettings => {
                    "add panel system settings aperture field wavelength aiming".into()
                }
            };
            push(trf("パネルを足す: {}", &[&t]), &hint, Cmd::Add(k));
        }
        for (_, p) in self.dock.iter_all_tabs() {
            let name = match &p.kind {
                Kind::Analysis(a) => a.cmd.clone(),
                Kind::System => "system".into(),
                Kind::Catalog => "catalog".into(),
                Kind::Layout => "layout".into(),
                Kind::LensData => "lens data".into(),
                Kind::MultiConfig => "multi configuration".into(),
                Kind::Optimize => "optimize".into(),
                Kind::Swap => "swap".into(),
                Kind::Tolerance => "tolerance".into(),
                Kind::Zoom => "zoom".into(),
                Kind::SysSettings => "settings".into(),
            };
            push(
                trf("パネルへ移る: {}", &[&p.title()]),
                &format!("go focus panel {name}"),
                Cmd::Focus(p.id),
            );
            push(
                format!(
                    "{}: {}",
                    detach_label(cfg!(target_arch = "wasm32")),
                    p.title()
                ),
                &format!("window monitor detach {name}"),
                Cmd::ToWindow(p.id),
            );
        }
        for p in Preset::ALL {
            push(
                trf("並び: {}", &[&p.label()]),
                "layout preset",
                Cmd::Preset(p),
            );
        }
        for (i, (n, _)) in self.c.s.layouts.iter().enumerate() {
            push(
                trf("保存した並び: {}", &[n]),
                "layout saved",
                Cmd::Layout(i),
            );
        }
        push(tr("今の並びを保存…").into(), "save layout", Cmd::SaveLayout);
        push(
            tr("配色: 暗い").into(),
            "theme dark",
            Cmd::Theme(Mode::Dark),
        );
        push(
            tr("配色: 明るい").into(),
            "theme light",
            Cmd::Theme(Mode::Light),
        );
        push(
            tr("配色: OS に合わせる").into(),
            "theme system",
            Cmd::Theme(Mode::System),
        );
        for i in 0..theme::ACCENTS.len() {
            push(
                trf("アクセント色 {}", &[&(i + 1)]),
                "accent color",
                Cmd::Accent(i),
            );
        }
        for l in Lang::ALL {
            push(trf("言語: {}", &[&l.label()]), "language", Cmd::Lang(l));
        }
        push(
            tr("カタログのフォルダ…").into(),
            "folders settings",
            Cmd::Folders,
        );
        push(tr("キーボード操作の一覧").into(), "help keys f1", Cmd::Help);
        for f in &self.c.s.recent {
            push(
                trf("最近のファイル: {}", &[f]),
                "recent",
                Cmd::Recent(f.clone()),
            );
        }
        v
    }

    /// 上の帯: ロゴ、ファイル、メニュー、探す、元に戻す、計算
    fn header(&mut self, ctx: &egui::Context, ui: &mut Ui) {
        let accent = theme::accent(&self.c.s.look);
        let weak = theme::weak_text(ui.visuals());
        let mut cmd = None;
        ui.horizontal(|ui| {
            ui.set_min_height(36.0);
            logo(ui, 26.0, accent);
            ui.label(RichText::new("lensbench").strong().size(15.0).color(accent));
            {
                ui.menu_button(tr("ファイル"), |ui| {
                    if ui.button(tr("新規（Ctrl+N）")).clicked() {
                        ui.close_menu();
                        cmd = Some(Cmd::New);
                    }
                    if ui.button(tr("開く…（Ctrl+O）")).clicked() {
                        ui.close_menu();
                        cmd = Some(Cmd::Open);
                    }
                    if !self.c.samples.is_empty() {
                        ui.menu_button(tr("見本"), |ui| {
                            // 組合せ・プロジェクト・.zmx の系の間に区切り
                            let group = |f: &str| {
                                if f.contains("/examples/combos/") {
                                    0
                                } else if f.ends_with(".json") {
                                    1
                                } else {
                                    2
                                }
                            };
                            let mut last = None;
                            for f in &self.c.samples {
                                let g = group(f);
                                if last.is_some_and(|l| l != g) {
                                    ui.separator();
                                }
                                if last != Some(g) && g == 0 {
                                    ui.weak(tr("組合せ（開くと組合せに戻る）"));
                                }
                                last = Some(g);
                                let name = sample_name(f);
                                if ui.button(name).on_hover_text(f).clicked() {
                                    ui.close_menu();
                                    cmd = Some(Cmd::Recent(f.clone()));
                                }
                            }
                        });
                    }
                    ui.menu_button(tr("最近のファイル"), |ui| {
                        if self.c.s.recent.is_empty() {
                            ui.label(tr("（なし）"));
                        }
                        for f in &self.c.s.recent {
                            if ui.button(f).clicked() {
                                ui.close_menu();
                                cmd = Some(Cmd::Recent(f.clone()));
                            }
                        }
                    });
                    if ui
                        .add_enabled(
                            self.c.file.is_some(),
                            egui::Button::new(tr("読み直す（Ctrl+R）")),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        cmd = Some(Cmd::Reload);
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.c.summary.is_some(),
                            egui::Button::new(tr(".zmx に書き出す…")),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        cmd = Some(Cmd::WriteZmx);
                    }
                    let can_save = self.c.file.is_some() || !self.c.browser.combo.picks.is_empty();
                    if ui
                        .add_enabled(can_save, egui::Button::new(tr("保存（Ctrl+S）")))
                        .on_hover_text(tr(
                            "組合せの系は組合せごとプロジェクトの .json に保存（開くと組合せに戻る）。開いた系は系ファイルに",
                        ))
                        .clicked()
                    {
                        ui.close_menu();
                        cmd = Some(Cmd::Save(false));
                    }
                    if ui
                        .add_enabled(
                            can_save,
                            egui::Button::new(tr("名前を付けて保存…（Ctrl+Shift+S）")),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        cmd = Some(Cmd::Save(true));
                    }
                    ui.separator();
                    if ui.button(tr("カタログのフォルダ…")).clicked() {
                        ui.close_menu();
                        cmd = Some(Cmd::Folders);
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        let (n, b) = crate::web_store::usage();
                        if ui
                            .add_enabled(
                                n > 0,
                                egui::Button::new(trf(
                                    "ブラウザに残したファイルを消す（{} 個、{} MB）…",
                                    &[&n, &format!("{:.1}", b as f64 / 1e6)],
                                )),
                            )
                            .on_hover_text(tr(
                                "読み込んだファイルとユーザーカタログは、ページを閉じてもブラウザに残る",
                            ))
                            .clicked()
                        {
                            ui.close_menu();
                            if crate::files::confirm(
                                "lensbench",
                                tr("ブラウザに残した、読み込んだファイルとユーザーカタログを全部消します。よろしいですか？"),
                            ) {
                                crate::web_store::clear();
                                crate::web_store::prune_dirs(
                                    &mut self.c.s.glass_dirs,
                                    &mut self.c.s.lens_dirs,
                                );
                                self.c.refresh_catalog();
                            }
                        }
                    }
                    // ブラウザ（Web 版）では閉じられないので出さない
                    if cfg!(not(target_arch = "wasm32")) {
                        ui.separator();
                        if ui.button(tr("終了")).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                });
                ui.menu_button(tr("編集"), |ui| {
                    let (u, r) = (self.c.can_undo(true), self.c.can_undo(false));
                    if ui
                        .add_enabled(u, egui::Button::new(tr("元に戻す（Ctrl+Z）")))
                        .clicked()
                    {
                        ui.close_menu();
                        cmd = Some(Cmd::Undo(true, 1));
                    }
                    if ui
                        .add_enabled(r, egui::Button::new(tr("やり直す（Ctrl+Y）")))
                        .clicked()
                    {
                        ui.close_menu();
                        cmd = Some(Cmd::Undo(false, 1));
                    }
                    // 段の一覧: 選んだ段まで（その段を含めて）まとめて戻す・やり直す
                    for (back, title) in [(true, tr("ここまで元に戻す")), (false, tr("ここまでやり直す"))] {
                        let list = self.c.undo_list(back);
                        ui.add_enabled_ui(!list.is_empty(), |ui| {
                            ui.menu_button(title, |ui| {
                                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                                    for (i, l) in list.iter().enumerate() {
                                        if ui.button(format!("{}. {l}", i + 1)).clicked() {
                                            ui.close_menu();
                                            cmd = Some(Cmd::Undo(back, i + 1));
                                        }
                                    }
                                });
                            });
                        });
                    }
                    ui.label(
                        RichText::new(tr(
                            "系の変更（面の表・多重構成・最適化の結果・組合せ）と、系を開く・読み直す・新規を 100 段まで",
                        ))
                        .small()
                        .weak(),
                    );
                });
                ui.menu_button(tr("パネル"), |ui| {
                    for k in add_kinds() {
                        let t = Pane {
                            id: 0,
                            kind: k.clone(),
                        }
                        .title();
                        if ui.button(trf("＋ {}", &[&t])).clicked() {
                            ui.close_menu();
                            cmd = Some(Cmd::Add(k));
                        }
                    }
                });
                ui.menu_button(tr("並び"), |ui| {
                    ui.label(RichText::new(tr("初期の並び")).small().color(weak));
                    for p in Preset::ALL {
                        if ui.button(p.label()).clicked() {
                            ui.close_menu();
                            cmd = Some(Cmd::Preset(p));
                        }
                    }
                    ui.separator();
                    ui.label(RichText::new(tr("保存した並び")).small().color(weak));
                    if self.c.s.layouts.is_empty() {
                        ui.label(tr("（なし）"));
                    }
                    let mut del = None;
                    for (i, (n, _)) in self.c.s.layouts.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.button(n).clicked() {
                                ui.close_menu();
                                cmd = Some(Cmd::Layout(i));
                            }
                            if ui.small_button("✖").on_hover_text(tr("消す")).clicked() {
                                del = Some(i);
                            }
                        });
                    }
                    if let Some(i) = del {
                        self.c.s.layouts.remove(i);
                    }
                    ui.separator();
                    if ui.button(tr("今の並びを保存…")).clicked() {
                        ui.close_menu();
                        cmd = Some(Cmd::SaveLayout);
                    }
                });
                ui.menu_button(tr("表示"), |ui| {
                    ui.label(RichText::new(tr("配色")).small().color(weak));
                    for (m, l) in [
                        (Mode::Dark, tr("暗い")),
                        (Mode::Light, tr("明るい")),
                        (Mode::System, tr("OS に合わせる")),
                    ] {
                        if ui.radio(self.c.s.look.mode == m, l).clicked() {
                            cmd = Some(Cmd::Theme(m));
                        }
                    }
                    ui.label(RichText::new(tr("アクセント色")).small().color(weak));
                    ui.horizontal(|ui| {
                        for (i, a) in theme::ACCENTS.iter().enumerate() {
                            let c = Color32::from_rgb(a[0], a[1], a[2]);
                            let on = self.c.s.look.accent == *a;
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(if on { "●" } else { "○" })
                                            .color(c)
                                            .size(18.0),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                cmd = Some(Cmd::Accent(i));
                            }
                        }
                        let mut c = Color32::from_rgb(
                            self.c.s.look.accent[0],
                            self.c.s.look.accent[1],
                            self.c.s.look.accent[2],
                        );
                        if ui.color_edit_button_srgba(&mut c).changed() {
                            self.c.s.look.accent = [c.r(), c.g(), c.b()];
                            theme::apply(ctx, &self.c.s.look);
                        }
                    });
                    if cfg!(not(target_arch = "wasm32")) {
                        ui.checkbox(&mut self.c.s.os_titlebar, tr("OS のタイトルバーを使う"))
                            .on_hover_text(tr(
                                "外すと上の帯をタイトルバーにする（ドラッグで移動、ダブルクリックで最大化、縁で大きさを変える）",
                            ));
                    }
                    ui.separator();
                    ui.label(RichText::new(tr("言語")).small().color(weak));
                    for l in Lang::ALL {
                        if ui.radio(self.c.s.lang == l, l.label()).clicked() {
                            cmd = Some(Cmd::Lang(l));
                        }
                    }
                    ui.separator();
                    if ui.button(tr("キーボード操作の一覧（F1）")).clicked() {
                        ui.close_menu();
                        cmd = Some(Cmd::Help);
                    }
                });
                ui.menu_button(tr("ヘルプ"), |ui| {
                    for (label, page, tip) in HELP_PAGES {
                        if ui
                            .button(tr(label))
                            .on_hover_text(tr(tip))
                            .clicked()
                        {
                            ui.close_menu();
                            ctx.open_url(egui::OpenUrl::new_tab(docs_url(page)));
                        }
                    }
                    ui.separator();
                    if ui.button(tr("キーボード操作の一覧（F1）")).clicked() {
                        ui.close_menu();
                        cmd = Some(Cmd::Help);
                    }
                    ui.separator();
                    ui.label(RichText::new(version()).small().color(weak));
                });
            }
            if let Some(f) = &self.c.file {
                ui.label(
                    RichText::new(
                        f.file_name()
                            .map(|x| x.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    )
                    .strong(),
                )
                .on_hover_text(f.display().to_string());
            }
            let custom = crate::chrome::custom(self.c.s.os_titlebar);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if custom {
                    crate::chrome::window_buttons(ui);
                }
                let run = egui::Button::new(
                    RichText::new(tr("▶ 計算（F5）"))
                        .strong()
                        .color(Color32::from_rgb(6, 18, 28)),
                )
                .fill(accent)
                .min_size(egui::vec2(0.0, 28.0));
                if ui.add_enabled(self.c.summary.is_some(), run).clicked() {
                    cmd = Some(Cmd::RunAll);
                }
                if ui
                    .add_enabled(
                        self.c.file.is_some(),
                        egui::Button::new(tr("元に戻す")).min_size(egui::vec2(0.0, 28.0)),
                    )
                    .on_hover_text(tr("系を読み直す（変えた値を捨てる）"))
                    .clicked()
                {
                    cmd = Some(Cmd::Reload);
                }
                let search = egui::Button::new(
                    RichText::new(format!("🔍  {}    Ctrl K", tr("解析・パネル・操作を探す")))
                        .color(weak),
                )
                .min_size(egui::vec2(300.0, 28.0));
                let r = ui.add(search);
                self.palette.anchor = Some(r.rect);
                if r.clicked() {
                    self.palette.show();
                }
                if custom {
                    crate::chrome::drag_area(ui);
                }
            });
        });
        if let Some(c) = cmd {
            self.exec(ctx, c);
        }
    }

    /// 下の状態の行: 状態、面・視野・波長の数、焦点距離・F 値・NA・全長・像高、版
    fn status_bar(&mut self, ui: &mut Ui) {
        let weak = theme::weak_text(ui.visuals());
        ui.horizontal(|ui| {
            let busy = self.c.worker.busy();
            let (dot, col) = if busy {
                ("●", Color32::from_rgb(0xF2, 0xC9, 0x4C))
            } else if self.c.status.error {
                ("●", Color32::from_rgb(0xFF, 0x7A, 0x6B))
            } else {
                ("●", Color32::from_rgb(0x8F, 0xD1, 0x9E))
            };
            ui.label(RichText::new(dot).color(col));
            if busy {
                ui.spinner();
            }
            let text = if self.c.status.text.is_empty() {
                tr("準備完了").to_string()
            } else {
                self.c.status.text.clone()
            };
            if self.c.status.error {
                ui.colored_label(Color32::from_rgb(0xFF, 0x7A, 0x6B), text);
            } else {
                ui.label(RichText::new(text).color(weak));
            }
            if let Some(s) = &self.c.summary {
                ui.separator();
                ui.label(
                    RichText::new(trf(
                        "{} 面 · {} 視野 · {} 波長",
                        &[&s["surfaces"], &self.c.n_fields(), &self.c.waves_um().len()],
                    ))
                    .color(weak),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Label::new(RichText::new(version()).small().color(weak))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text(tr("クリックで版をコピー"))
                    .clicked()
                {
                    ui.ctx().copy_text(version());
                }
                if self.c.font.is_none() && i18n::is_ja() {
                    ui.label(
                        RichText::new(tr("日本語のフォントが見つからない"))
                            .small()
                            .color(weak),
                    );
                }
                if self.c.info.is_some() {
                    let n = |k: &str, d: f64| {
                        self.c
                            .info_num(k)
                            .map(|x| fmt_num((x * d).round() / d))
                            .unwrap_or("—".into())
                    };
                    let items = [
                        (tr("像高"), n("paraxial_image_height", 1e3), "mm"),
                        (tr("全長"), n("total_track", 1e2), "mm"),
                        ("NA", n("image_na", 1e4), ""),
                        ("F/#", n("wfno", 1e2), ""),
                        ("EFFL", n("effl", 1e3), "mm"),
                    ];
                    for (k, v, u) in items {
                        ui.label(RichText::new(format!("{v} {u}")).monospace());
                        ui.label(RichText::new(k).color(weak));
                        ui.add_space(6.0);
                    }
                }
            });
        });
    }

    fn save_layout_window(&mut self, ctx: &egui::Context) {
        if !self.c.save_layout_open {
            return;
        }
        let mut open = true;
        let mut done = false;
        egui::Window::new(tr("今の並びを保存"))
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(tr("名前（同じ名前は上書き）"));
                let r = ui.text_edit_singleline(&mut self.c.layout_name);
                r.request_focus();
                let enter = ui.input(|i| i.key_pressed(Key::Enter));
                if (ui.button(tr("保存")).clicked() || enter)
                    && !self.c.layout_name.trim().is_empty()
                {
                    let name = self.c.layout_name.trim().to_string();
                    let l = self.layout();
                    if let Some(x) = self.c.s.layouts.iter_mut().find(|(n, _)| *n == name) {
                        x.1 = l;
                    } else {
                        self.c.s.layouts.push((name.clone(), l));
                    }
                    self.c.status = Status {
                        text: trf("並び「{}」を保存", &[&name]),
                        error: false,
                    };
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
                        if ui.small_button(tr("本体に戻す")).clicked() {
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

    /// 通し確認を 1 フレーム進める
    fn tour_frame(&mut self, ctx: &egui::Context) {
        let Some(mut t) = self.tour.take() else {
            return;
        };
        ctx.request_repaint();
        let busy = self.c.worker.busy();
        if t.timed_out() {
            t.errors.push(format!("{} で時間切れ", t.step.name()));
            t.step = Step::Done;
        }
        if t.step != Step::Done {
            if t.take_shot(ctx) {
                // 撮れた。次の段へ
            } else if t.ready(busy) {
                if self.c.status.error {
                    t.errors
                        .push(format!("{}: {}", t.step.name(), self.c.status.text));
                    self.c.status.error = false;
                }
                if !t.acted {
                    t.acted = true;
                    self.tour_act(ctx, t.step);
                } else {
                    t.request_shot(ctx);
                }
            }
        }
        if t.step == Step::Done {
            t.finish();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.tour = Some(t);
        }
    }

    fn pane_of(&self, f: impl Fn(&Kind) -> bool) -> Option<u64> {
        self.dock
            .iter_all_tabs()
            .find(|(_, p)| f(&p.kind))
            .map(|(_, p)| p.id)
    }

    /// 通し確認の各段の操作
    fn tour_act(&mut self, ctx: &egui::Context, step: Step) {
        let spot = |k: &Kind| matches!(k, Kind::Analysis(a) if a.cmd == "spot");
        match step {
            Step::Open | Step::Done => {}
            Step::Analysis => self.run_all(false),
            Step::States => {
                // 干渉計の組合せに状態を足し、状態だけの系の値（物体距離）と経路（参照の腕）にする
                let c = &mut self.c.browser.combo;
                c.states = vec!["参照".into()];
                c.state = Some("参照".into());
                let mut s = c.snap();
                s.object_distance = Some(200.0);
                c.state_sys.insert("参照".into(), s);
                c.state_path.insert("参照".into(), "RR".into());
                let p = c.project();
                self.c.build_combo(p);
                self.exec(ctx, Cmd::FocusKind(Kind::SysSettings));
            }
            Step::Coatings => {
                // 膜（フィルタ）の定義: バンドパスを定義して BS の膜にし、系の設定の膜の欄を開いて見せる
                let c = &mut self.c.browser.combo;
                c.coatings.insert(
                    "BP633".into(),
                    lensbench_project::filter::CoatSpec::Bandpass {
                        center_nm: 633.0,
                        fwhm_nm: 10.0,
                        ideal: lensbench_project::filter::Ideal {
                            t: 0.9,
                            od: 4.0,
                            width_nm: None,
                            neff: 2.0,
                            absorptive: false,
                        },
                    },
                );
                let p = c.project();
                self.c.build_combo(p);
                crate::coat_defs::focus(ctx);
                self.exec(ctx, Cmd::FocusKind(Kind::SysSettings));
            }
            Step::Units => {
                // 開いた系（P17）に群 G23（群 2・3）を足し、機構部品（筒）を置いて、群 G2 の設定と欄を見せる
                let u = |n: &str| {
                    self.c
                        .ld_units
                        .iter()
                        .find(|u| u.name == n)
                        .map(|u| u.spans.clone())
                };
                let mut ops = Vec::new();
                if let (Some(a), Some(b)) = (u("G2"), u("G3")) {
                    ops.push(json!({ "op": "define_unit", "name": "G23", "spans": [[a[0][0], b[0][1]]] }));
                }
                ops.push(json!({ "op": "add_mech", "part": {
                    "name": "tube", "surface": 1, "offset": [0.0, 0.0, -2.0],
                    "bores": [{ "z0": 0.0, "z1": 40.0, "radius": 15.0 }], "outer_radius": 18.0,
                }}));
                self.c.worker.send(Tag::Edit, "edit", json!({ "ops": ops }));
                self.c.ld.scope.clear();
                self.c.ld.sel = Some("G2".into());
                self.c.ld.block_view = Some("G2".into());
                self.c.um_open = true;
                self.exec(ctx, Cmd::FocusKind(Kind::LensData));
            }
            Step::Zoom => {
                // 3 群のズーム（例の P17）を開き、群 2・3 の移動で焦点距離 140・160・175 mm に合わせ、間を 2 点ずつたどる
                self.c
                    .open(Path::new("examples/projects/P17_zoom_three_groups.json"));
                let z = &mut self.c.zoom;
                z.vars = ["G2.shift", "G3.shift"]
                    .map(|n| crate::zoom_panel::VarRow {
                        name: n.into(),
                        ..Default::default()
                    })
                    .to_vec();
                z.steps = 2;
                let cands: Vec<crate::zoom_panel::Cand> =
                    [("wide", "140"), ("mid", "160"), ("tele", "175")]
                        .map(|(n, t)| {
                            z.states.insert(
                                n.into(),
                                crate::zoom_panel::StateRow {
                                    on: true,
                                    target: t.into(),
                                },
                            );
                            (false, n.to_string())
                        })
                        .to_vec();
                match z.params(&cands) {
                    Ok(p) => {
                        z.running = true;
                        self.c.worker.send(Tag::Zoom, "zoom", p);
                    }
                    Err(e) => {
                        self.c.status = Status {
                            text: e,
                            error: true,
                        }
                    }
                }
                self.exec(ctx, Cmd::FocusKind(Kind::Zoom));
            }
            Step::Pop => {
                // 簡易 POP のパネルを足して計算する（像面の前の面から像面まで）
                let cfg = AnaCfg::new("pop");
                let id = self.add_pane(Kind::Analysis(cfg.clone()));
                self.focus(id);
                self.c.run(id, &cfg);
            }
            Step::Catalog => {
                if let Some(id) = self.pane_of(|k| matches!(k, Kind::Catalog)) {
                    self.focus(id);
                }
                self.c.browser.tab = catalog::Tab::Lenses;
                self.c.refresh_catalog();
            }
            Step::Combo => {
                // 正のレンズを焦点距離の短い順に 2 枚
                let b = &mut self.c.browser;
                let mut pos: Vec<catalog::LensRow> = b
                    .lenses
                    .iter()
                    .filter(|l| l.error.is_none() && l.efl > 0.0)
                    .cloned()
                    .collect();
                pos.sort_by(|a, b| a.efl.total_cmp(&b.efl));
                b.combo.picks.clear();
                for l in pos.iter().take(2) {
                    b.add_pick(l);
                }
                if let Some(p) = b.combo.picks.first_mut() {
                    p.air = 3.0;
                }
                b.lens_sel = b.lenses.iter().position(|l| Some(l) == pos.first());
            }
            Step::Build => {
                let p = self.c.browser.combo.project();
                self.c.build_combo(p);
            }
            Step::Layout => {
                // レイアウト図: 2 枚目を選び、1 枚目の後の空気の寸法を選ぶ
                self.exec(ctx, Cmd::FocusKind(Kind::Layout));
                // 1 枚目の後の空気の寸法を、その場で書き換えるところ
                self.c.combo_view.start_edit(&mut self.c.browser.combo, 0);
            }
            Step::Layout3d => {
                // 3D 表示: 2 枚目を選んだまま
                self.c.combo_view.select(1);
                self.c.s.view_plane = ViewPlane::D3;
                self.c.request_layout();
            }
            Step::LensData => {
                // 面の表: 1 枚目の玉の前の面の半径を表から変え（組合せの面の上書きで作り直す）、その玉に入る
                self.c.combo_view.select(0);
                let ids = self.c.browser.combo.span_ids();
                let spans = lens_data::elem_spans(
                    &ids,
                    &self.c.ld_units,
                    self.c.browser.combo.lead_stop(),
                    &self.c.ld_rows,
                );
                if let Some(Some([a, _])) = spans.first().copied() {
                    self.c.combo_surface_edit(&spans, a, "semi", json!(6.0));
                    if !self.c.s.auto_build {
                        let p = self.c.browser.combo.project();
                        self.c.build_combo(p);
                    }
                }
                // 最上位で 1 枚目の玉（ブロック）を選び、ブロックの設定（要素の設定）を出す
                self.c.s.ld_detail = true;
                if let Some(id) = ids.first() {
                    self.c.ld.scope.clear();
                    self.c.ld.sel = Some(id.clone());
                    self.c.ld.cur = None;
                }
                self.exec(ctx, Cmd::FocusKind(Kind::LensData));
            }
            Step::LensDataTree => {
                // 折りたたみ表示: 1 枚目の玉をその場で開き、中の面を選ぶ
                let ids = self.c.browser.combo.span_ids();
                self.c.s.ld_tree = true;
                self.c.ld.scope.clear();
                if let Some(id) = ids.first() {
                    self.c.ld.open.insert(id.clone());
                    self.c.ld.sel = Some(id.clone());
                    let first = self
                        .c
                        .ld_blocks
                        .iter()
                        .find(|b| &b.name == id)
                        .and_then(|b| b.surfs.first().copied());
                    self.c.ld.cur = first.map(|j| (j, lens_data::Col::R));
                }
                self.exec(ctx, Cmd::FocusKind(Kind::LensData));
            }
            Step::ComboSpot => {
                // 面の表の表示・見る面を戻す
                self.c.s.ld_tree = false;
                self.c.s.ld_detail = false;
                self.c.ld.open.clear();
                self.c.s.view_plane = ViewPlane::Yz;
                self.c.request_layout();
                if let Some(id) = self.pane_of(spot) {
                    self.focus(id);
                }
            }
            Step::Optimize => {
                // 最適化: 像面の前の間隔を変数に、既定の RMS スポットと縁の光線の項で回す
                self.exec(ctx, Cmd::FocusKind(Kind::Optimize));
                let img = self.c.ld_rows.len().saturating_sub(1);
                let o = &mut self.c.opt;
                o.vars = vec![crate::optim_panel::VarRow {
                    name: format!("{}.t", img.saturating_sub(1)),
                    lo: "0".into(),
                    ..Default::default()
                }];
                let mut t = crate::optim_panel::TermRow::new("ray");
                t.weight = "0.1".into();
                o.terms = vec![t];
                if let Ok(p) = o.params() {
                    o.running = true;
                    self.c.worker.send(Tag::Optimize, "optimize", p);
                }
            }
            Step::OptApply => {
                // 結果を組合せに当てる（後の空気が変わり、系を作り直して解析も計算し直す）
                match self.c.opt.result.clone() {
                    Some(r) => {
                        let v = crate::optim_panel::result_values(&r, false);
                        if self.c.apply_values(&v) {
                            self.c.opt.undo = Some(crate::optim_panel::result_values(&r, true));
                        }
                    }
                    None => {
                        self.c.status = Status {
                            text: "最適化の結果がない".into(),
                            error: true,
                        }
                    }
                }
            }
            Step::Swap => {
                // 差し替え探索: 2 枚目をカタログの全部（裏返しも）で差し替え、間の空気と像面までを最適化
                self.exec(ctx, Cmd::FocusKind(Kind::Swap));
                let (proj, cands) = crate::swap_panel::combo_project(&self.c.browser.combo);
                let sw = &mut self.c.swap;
                sw.slots = cands
                    .blocks
                    .iter()
                    .skip(1)
                    .take(1)
                    .map(|b| crate::swap_panel::SlotRow {
                        block: b.0.clone(),
                        flip: true,
                        ..Default::default()
                    })
                    .collect();
                let n = self.c.browser.combo.picks.len();
                sw.vars = [(1, "0.5"), (n, "1")]
                    .iter()
                    .map(|(k, lo)| crate::swap_panel::VarRow {
                        name: format!("d{k}.t"),
                        lo: lo.to_string(),
                        hi: String::new(),
                    })
                    .collect();
                if let Ok(mut v) = sw.spec() {
                    v["project"] = proj;
                    v["top"] = json!(sw.top);
                    sw.running = true;
                    self.c.swap_ids = self.c.browser.combo.ids();
                    self.c.worker.send(Tag::Swap, "swap", v);
                }
            }
            Step::SwapInsert => {
                // 1 位の組を組合せに入れる（系を作り直して解析も計算し直す）
                let row = self
                    .c
                    .swap
                    .result
                    .as_ref()
                    .and_then(|r| r["results"].get(0))
                    .cloned();
                let before = self.c.browser.combo.clone();
                let r = row
                    .ok_or_else(|| "差し替え探索の結果がない".to_string())
                    .and_then(|row| {
                        crate::swap_panel::insert_row(
                            &mut self.c.browser.combo,
                            &self.c.swap_ids,
                            &row,
                        )
                    });
                match r {
                    Ok(()) => {
                        self.c.swap.undo = Some(before);
                        self.c.swap.inserted = Some(0);
                        let p = self.c.browser.combo.project();
                        self.c.build_combo(p);
                    }
                    Err(e) => {
                        self.c.status = Status {
                            text: e,
                            error: true,
                        }
                    }
                }
            }
            Step::Tolerance => {
                // モンテカルロ公差: 精密の組、像面の前の間隔を補償子に、30 回
                self.exec(ctx, Cmd::FocusKind(Kind::Tolerance));
                let img = self.c.ld_rows.len().saturating_sub(1);
                let t = &mut self.c.tol;
                t.trials = 30;
                t.preset = "precision".into();
                // カタログの玉の公差: どのメーカーも中心厚 ±0.05 mm、外径 10 mm 未満は焦点距離 ±1%
                t.catalog_tols = crate::tol_catalog::from_json(&serde_json::json!({
                    "*": { "thickness": 0.05, "rules": [{ "dia_lt": 10, "focal_rel": 0.01 }] }
                }))
                .unwrap_or_default();
                t.percentiles = "50, 90, 98".into();
                t.opt.vars = vec![crate::optim_panel::VarRow {
                    name: format!("{}.t", img.saturating_sub(1)),
                    ..Default::default()
                }];
                match t.spec() {
                    Ok(p) => {
                        t.running = true;
                        self.c.worker.send(Tag::Tolerance, "tolerance", p);
                    }
                    Err(e) => {
                        self.c.status = Status {
                            text: e,
                            error: true,
                        }
                    }
                }
            }
            Step::Param => {
                // 1 枚目の後の空気を少し動かす（解析は自動で計算し直し、前との差が出る）
                self.c.s.params = vec!["1.t".into()];
                self.c.read_params();
                self.c.pending_set.insert("1.t".into(), 2.5);
            }
            Step::Glasses => {
                if let Some(id) = self.pane_of(|k| matches!(k, Kind::Catalog)) {
                    self.focus(id);
                }
                let b = &mut self.c.browser;
                b.tab = catalog::Tab::Glasses;
                b.glass_sel = (!b.glasses.is_empty()).then_some(0);
                // アッベ図は選んだガラスの辺りを 3 倍に（目盛りが範囲に合うかを見る）
                b.glass_view = catalog::MapView::FULL;
                if let Some(g) = b.glass_sel.and_then(|i| b.glasses.get(i)) {
                    if g.vd.is_finite() && g.nd.is_finite() {
                        let (vd, nd) = (g.vd, g.nd);
                        b.glass_view.zoom(vd, nd, 3.0);
                    }
                }
            }
            Step::Window => {
                // MTF を別の窓に出す（本体の画面では MTF が消え、スポットが表に出る）
                if let Some(id) = self.pane_of(|k| matches!(k, Kind::Analysis(a) if a.cmd == "mtf"))
                {
                    self.detach(id);
                }
                if let Some(id) = self.pane_of(spot) {
                    self.focus(id);
                }
            }
            Step::Palette => {
                self.palette.show();
                self.palette.query = "spot".into();
            }
            Step::English => {
                self.palette.open = false;
                self.exec(ctx, Cmd::Lang(Lang::En));
                self.exec(ctx, Cmd::Theme(Mode::Light));
            }
            Step::Settings => {
                // 系の設定のパネル: 視野に重み、波長を 1 つ足し、エイミングを実光線に（系は自動で作り直す）
                self.exec(ctx, Cmd::Lang(Lang::Ja));
                self.exec(ctx, Cmd::Theme(Mode::Dark));
                self.exec(ctx, Cmd::FocusKind(Kind::SysSettings));
                let c = &mut self.c.browser.combo;
                if let Some(f) = c.fields.last_mut() {
                    f.w = 0.5;
                }
                c.waves.push(crate::sysset::WavePt { um: 0.55, w: 2.0 });
                c.aiming = crate::sysset::Aiming::Real;
            }
            Step::Interferometer => {
                // BS の分岐で組んだ Twyman-Green。経路 TR（試験の腕を往復して口 D へ）で、BS を選ぶ
                let mut c = combo::Combo::twyman_green();
                c.path = "TR".into();
                c.object_distance = Some(100.0);
                c.field_kind = crate::sysset::FieldKind::ObjectHeight;
                c.fields = [0.0, 0.5].map(crate::sysset::FieldPt::y).to_vec();
                c.aperture = crate::sysset::ApKind::Na;
                c.ap_value = 0.02;
                c.waves = vec![crate::sysset::WavePt::um(0.6328)];
                c.primary = 1;
                self.c.browser.combo = c;
                // 参照の腕（RR）も重ねて描く。光線は視野 1 の十字で、光線・書き出しの設定の欄も開いて見せる
                self.c.overlay.items = ["TR", "RR"]
                    .map(|x| ("paths", x.to_string(), true))
                    .to_vec();
                self.c.s.rays.fields = vec![1];
                self.c.s.rays.fan = "cross".into();
                self.c.rays_open = true;
                let p = self.c.browser.combo.project();
                self.c.build_combo(p);
                self.c.combo_view.select(1);
                self.exec(ctx, Cmd::FocusKind(Kind::Layout));
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(target_arch = "wasm32")]
        if self.child.is_some() {
            self.child_update(ctx);
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(v) = self.viewer.as_mut() {
            v.update(ctx);
            return;
        }
        // Web 版: 選び終わったファイルを開く・書かれたファイルをダウンロードにする
        self.c.pick_and_open_when(false);
        crate::files::flush_downloads();
        // Web 版: 読み込んだファイル・ユーザーカタログをブラウザに残す
        #[cfg(target_arch = "wasm32")]
        crate::web_store::flush();
        let mut redo = Vec::new();
        while let Some(d) = self.c.worker.poll() {
            if !d.skipped {
                self.c.take(d);
            } else if let Tag::Analyze(id) | Tag::Rays(id) = d.tag {
                redo.push(id);
            }
        }
        // 系が変わって捨てた解析: 自動のパネルは系の変更のあとにまとめて回し直すので、自動でないパネル
        // （頼まれて回していたもの）だけ新しい系で頼み直す
        if !redo.is_empty() {
            redo.sort_unstable();
            redo.dedup();
            for (id, cfg) in self.analyses() {
                if !cfg.live && redo.contains(&id) {
                    self.c.run(id, &cfg);
                }
            }
        }
        while let Some(d) = self.c.preview.poll() {
            // カタログの一覧は系の要求と同じに扱う（状態の行・失敗の表示も）
            if matches!(
                d.tag,
                Tag::Glasses | Tag::Lenses | Tag::UserCatalog | Tag::CatalogFill
            ) {
                // 失敗しても読み込み中は終わる
                match d.tag {
                    Tag::Glasses => self.c.browser.glasses_pending = false,
                    Tag::Lenses => self.c.browser.lenses_pending = false,
                    _ => {}
                }
                self.c.take(d);
                continue;
            }
            // 失敗は系の読み込みの方で出す
            match (&d.tag, d.first().2) {
                (Tag::Layout, Ok(v)) => self.c.layout_data = Some(combo::LayoutData::parse(v)),
                // カタログの玉の断面（読めなければ空の図）
                // 頼んだ行の結果だけ受け取る（カタログを読み直す前に頼んだ分は捨てる）
                (Tag::LensPreview(i), r) if self.c.preview_req == Some(*i) => {
                    self.c.preview_req = None;
                    let mut data = r.ok().map(combo::LayoutData::parse).unwrap_or_default();
                    // 負の単レンズの断面は虚物体（前側焦点へ集まる光）。入る光を玉の前から描く
                    data.virtual_object_from_front();
                    self.c.browser.preview = Some((*i, data));
                }
                _ => {}
            }
        }
        // 組合せを動かしている間は解析を待たせ、系の作り直しとレイアウト図を先に回す
        let settle = std::time::Duration::from_millis(350);
        let moving = self.c.is_combo() && self.c.combo_edit.is_some_and(|t| t.elapsed() < settle);
        if moving && self.c.rerun.is_some() {
            ctx.request_repaint_after(settle);
        } else if let Some(live) = self.c.rerun.take() {
            self.run_all(live);
        }
        self.c.flush_set();
        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
        self.c.combo_hist.track(&self.c.browser.combo, busy);
        self.c.auto_build();
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
        let shown = if self.c.is_combo() {
            self.c.doc_path.as_ref()
        } else {
            self.c.file.as_ref()
        };
        let title = match shown {
            Some(f) => format!(
                "lensbench — {}",
                f.file_name()
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
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
                        ui.label(trf("{} をレイアウト図へ", &[&p.0.label()]));
                    });
                });
        }
        for (s, n, k) in added {
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
        let items = self.palette_items();
        if let Some(cmd) = self.palette.ui(ctx, &items) {
            self.exec(ctx, cmd);
        }
        for cmd in std::mem::take(&mut self.c.cmds) {
            self.exec(ctx, cmd);
        }
        if self.c.folders_open {
            self.c.folders_window(ctx);
        }
        if self.c.help_open {
            self.c.help_window(ctx);
        }
        if self.c.cat_save.open {
            self.c.cat_save_window(ctx);
        }
        self.c.fill_window(ctx);
        self.save_layout_window(ctx);
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
        self.child.is_none()
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.tour.is_some() {
            return;
        }
        // 別のタブのパネルは元のタブの設定を上書きしない
        #[cfg(target_arch = "wasm32")]
        if self.child.is_some() || self.viewer.is_some() {
            return;
        }
        self.c.s.combo = self.c.browser.combo.clone();
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
    pub fn update(&mut self, v: &Value) {
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

    /// serve の layout の params（何も選ばなければ今の構成だけ、見る面は yz）
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
                m.insert(key.into(), Value::Array(on));
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
            ui.label(tr("重ねる構成"));
            let mut last = "";
            for (key, name, on) in &mut self.items {
                if *key != last {
                    last = key;
                    ui.weak(match *key {
                        "cfgs" => tr("構成"),
                        "states" => tr("状態"),
                        _ => tr("BS の経路"),
                    });
                }
                changed |= ui.checkbox(on, name.as_str()).changed();
            }
            if self.items.iter().any(|x| x.2) {
                ui.separator();
                ui.label(tr("ずらす"));
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
        json!({ "paths": ["RR"], "offset": [0.0, 0.0, 30.0] })
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

/// パネルを外に出す操作の名前。Web 版は解析のパネルはブラウザの別のタブに、ほかは別の窓（ポップアップ）に出す。
/// 解析以外は元のタブで描いた画面を送って、別の窓で操作する（#137）
fn detach_label(web: bool) -> &'static str {
    if web {
        tr("別の窓に出す")
    } else {
        tr("別の窓に出す（別モニターへ）")
    }
}

#[test]
fn detach_label_web() {
    assert_eq!(detach_label(true), "別の窓に出す");
    assert_eq!(detach_label(false), "別の窓に出す（別モニターへ）");
}

/// 解析の結果を写す・保存するメニュー（JSON・テキスト（タブ区切り）・CSV、#128）
fn result_export_menu(ui: &mut Ui, cmd: &str, v: &Value) {
    use crate::export_table::{delimited, tables};
    ui.menu_button(tr("コピー ▾"), |ui| {
        if ui.button(tr("JSON")).clicked() {
            ui.ctx()
                .copy_text(serde_json::to_string_pretty(v).unwrap_or_default());
            ui.close_menu();
        }
        if ui
            .button(tr("テキスト（タブ区切り）"))
            .on_hover_text(tr("表計算に貼れる形。表が複数あれば「# 名前」の行で区切る"))
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
    ui.menu_button(tr("保存 ▾"), |ui| {
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
                        eprintln!("{}: {e}", p.display());
                    }
                }
            }
        }
    });
}

/// マニュアル（ドキュメントのサイト）の場所。非公開の版などは LENSBENCH_DOCS_URL で差し替える
const DOCS_URL: &str = match option_env!("LENSBENCH_DOCS_URL") {
    Some(u) => u,
    None => "https://masa-iwm.github.io/lensbench/",
};

/// ヘルプのメニューのページ（名前、マニュアルの中のページ、説明）
const HELP_PAGES: &[(&str, &str, &str)] = &[
    ("マニュアル", "", "使い方の目次（ブラウザで開く）"),
    ("GUI の使い方", "gui.html", "画面・パネルの使い方（図つき）"),
    ("解析と CLI", "cli.html", "解析の種類と引数、出力の例"),
    (
        "プロジェクトファイル",
        "project-file.html",
        "ブロックで系を組む JSON（状態・群・BS）",
    ),
    ("最適化", "optimize.html", "評価関数・変数・DLS"),
    ("公差", "tolerance.html", "モンテカルロ公差の設定"),
];

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

/// 組合せを同梱したプロジェクト: 系のプロジェクトとして読め、組合せに戻せる（#129）
#[test]
fn project_keeps_combo() {
    let mut c = Combo {
        name: "two".into(),
        ..Default::default()
    };
    c.picks.push(combo::Pick::new(combo::ElemKind::Plate));
    let v = project_with_combo(&c);
    let txt = serde_json::to_string_pretty(&v).unwrap();
    let p = lensbench_project::Project::from_json(&txt).unwrap();
    assert_eq!(
        p.blocks.len(),
        c.project()["blocks"].as_array().unwrap().len()
    );
    assert_eq!(combo_of_project(&v), Some(c));
    assert_eq!(combo_of_project(&json!({ "blocks": [] })), None);
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
        let want = serde_json::to_string_pretty(&project_with_combo(&c)).unwrap() + "\n";
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
    }
}

#[cfg(test)]
mod hist_tests {
    use super::*;

    /// 組合せの段: 入力・ドラッグの途中は積まず、落ち着いたら 1 段。戻す・やり直す、戻した後に変えるとやり直す段は消える
    #[test]
    fn combo_history_steps() {
        let mut h = ComboHist::default();
        let mut c = Combo::default();
        let name0 = c.name.clone();
        h.track(&c, false);
        assert!(!h.can(&c, true));
        c.name = "a".into();
        h.track(&c, true);
        c.name = "ab".into();
        h.track(&c, true);
        assert!(h.undo.is_empty() && h.can(&c, true));
        h.track(&c, false);
        assert_eq!(h.undo.len(), 1);
        c.name = "x".into();
        h.track(&c, false);
        assert_eq!(h.list(true).len(), 2);
        assert_eq!(h.list(true)[0], "組合せの設定");
        let b = h.step(&c, true, 1).unwrap();
        assert_eq!(b.name, "ab");
        let b = h.step(&b, true, 1).unwrap();
        assert_eq!(b.name, name0);
        assert!(h.step(&b, true, 1).is_none());
        let f = h.step(&b, false, 1).unwrap();
        assert_eq!(f.name, "ab");
        let mut g = f.clone();
        g.name = "y".into();
        h.track(&g, false);
        assert!(!h.can(&g, false));
        assert_eq!(h.step(&g, true, 1).unwrap().name, "ab");
        // まとめて 2 段戻す
        assert_eq!(h.step(&f, true, 2).map(|c| c.name), Some(name0.clone()));
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
            catalog: None,
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
