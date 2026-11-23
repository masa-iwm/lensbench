//! パネルと並び（レイアウト）。パネルは egui_dock でドッキング・タブ・フロートでき、別の OS の窓（別モニター）にも
//! 出せる。並びは名前を付けて保存・呼び出しでき、初期の並びをいくつか用意する。
//!
//! 保存するのはパネルの種類と設定（解析の種類・視野・波長など）だけで、計算結果は保存しない。

use std::collections::BTreeMap;

use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};

use crate::i18n::{n_, tr};

/// 解析の種類（serve の analyze の cmd）、表示名、視野・波長を選べるか
pub(crate) const ANALYSES: &[(&str, &str, bool)] = &[
    ("spot", n_("Spot"), true),
    ("mtf", n_("Diffraction MTF"), true),
    ("wavefront", n_("Wavefront Map"), true),
    ("zernike", n_("Zernike coefficients"), true),
    ("rms", "RMS", true),
    ("psf", n_("Diffraction PSF"), true),
    ("geo-mtf", n_("Geometric MTF"), true),
    ("huygens-mtf", "Huygens MTF", true),
    ("mtf-field", n_("MTF vs field"), false),
    ("mtf-focus", n_("Through-focus MTF"), true),
    ("interference", n_("Two-beam fringes"), true),
    ("pop", n_("Simple POP"), true),
    ("image-sim", n_("Image simulation"), false),
    ("rays", n_("Ray table"), false),
    ("coatings", n_("Coating interfaces"), false),
    ("info", n_("System data"), false),
    ("elements", n_("Element mode"), false),
    ("ghost", n_("Ghosts"), false),
    ("mech-check", n_("Mechanical vignetting"), false),
    ("ray-fan", n_("Ray fan"), true),
    ("opd-fan", n_("OPD fan"), true),
    ("pupil-fan", n_("Pupil aberration fan"), true),
    ("field-curvature", n_("Field curvature / distortion"), false),
    ("longitudinal", n_("Longitudinal aberration"), false),
    ("lateral-color", n_("Lateral color"), false),
    ("focal-shift", n_("Chromatic focal shift"), false),
    ("grid-distortion", n_("Grid distortion"), false),
    ("seidel", n_("Seidel coefficients"), false),
    ("footprint", n_("Footprint diagram"), true),
    ("relative-illumination", n_("Relative illumination"), false),
    ("vignetting", n_("Vignetting plot"), false),
    ("gaussian-beam", n_("Paraxial Gaussian beam"), false),
];

/// 解析の一覧（同梱の [`ANALYSES`] と拡張の解析 [`crate::ext`]）: (cmd, 表示名（日本語）, 視野・波長を選べるか)
pub(crate) fn analyses() -> Vec<(&'static str, &'static str, bool)> {
    let mut v = ANALYSES.to_vec();
    v.extend(
        crate::ext::list()
            .iter()
            .map(|a| (a.cmd, a.label, a.select)),
    );
    v
}

/// 解析の説明（ツールチップ、日本語。表示は tr で）。無ければ None
pub(crate) fn analysis_help(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "spot" => "Spot diagram: ray positions on the image and RMS/GEO radius",
        "mtf" => "Diffraction MTF: modulation per frequency (tangential/sagittal), from the pupil autocorrelation",
        "wavefront" => "Wavefront: OPD map referenced to the chief ray, with P-V and RMS [waves]",
        "zernike" => "Zernike coefficients [waves] (fit of the wavefront)",
        "rms" => "RMS (spot radius, wavefront, approximate Strehl) versus field, focus or wavelength",
        "psf" => "Diffraction PSF: DFT of the pupil function, normalized to the aberration-free peak",
        "geo-mtf" => "Geometric MTF: MTF from the ray position distribution",
        "huygens-mtf" => "Huygens MTF: Fourier transform of the Huygens PSF (frequencies on the image surface)",
        "mtf-field" => "Field MTF: MTF versus normalized field 0–1 (per frequency)",
        "mtf-focus" => "Through-focus MTF: MTF as the image plane shifts",
        "interference" => "Two-beam fringes: overlap of two configurations or states",
        "pop" => "Simple POP: irradiance of a field built from rays and propagated",
        "image-sim" => "Image simulation: a source image through the diffraction PSF, distortion and relative illumination",
        "rays" => "Ray table: intersections, directions and optical path per surface",
        "coatings" => "Coating boundaries: every boundary (coating, media) and how it is handled",
        "info" => "System data: focal length, F-number, pupils, image height, etc.",
        "elements" => "Lens mode: the system split into lenses, mirrors, coordinate breaks and air gaps",
        "ghost" => "Ghosts: paraxial focus, blur at the image and intensity ratio of double bounces",
        "mech-check" => "Mechanical vignetting: share of rays outside the mechanical apertures",
        "ray-fan" => "Ray fan: transverse ray error versus pupil coordinate (tangential and sagittal) per field and wavelength",
        "opd-fan" => "OPD fan: optical path difference [waves] versus pupil coordinate (tangential and sagittal)",
        "pupil-fan" => "Pupil aberration fan: where real rays aimed at the paraxial entrance pupil cross the stop, against the paraxial point [% of the stop radius]",
        "field-curvature" => "Field curvature and distortion: tangential and sagittal focus and distortion % versus field",
        "longitudinal" => "Longitudinal aberration: where on-axis rays cross the axis versus pupil height",
        "lateral-color" => "Lateral color: chief ray height of each wavelength minus the primary wavelength versus field",
        "focal-shift" => "Chromatic focal shift: paraxial focus versus wavelength relative to the primary wavelength",
        "grid-distortion" => "Grid distortion: real chief ray points of a field grid against the paraxial ideal grid",
        "seidel" => "Seidel coefficients: third-order aberration sums and first-order color per surface",
        "footprint" => "Footprint diagram: where the beam of each field crosses a surface, against its aperture",
        "relative-illumination" => "Relative illumination: image irradiance versus field relative to the axis",
        "vignetting" => "Vignetting plot: fraction of unvignetted rays versus field",
        "gaussian-beam" => "Paraxial Gaussian beam: beam size, waist, divergence and Rayleigh range after each surface",
        _ => return None,
    })
}

/// 解析の群の並び
pub(crate) const ANALYSIS_GROUPS: &[&str] = &[
    n_("Image quality"),
    n_("MTF / PSF"),
    n_("Aberrations"),
    n_("Interference / propagation"),
    n_("Tables / system"),
    n_("Extra analyses"),
];

/// 解析の群（一覧の見出し、日本語。表示は tr で）。拡張の解析は「追加の解析」
pub(crate) fn analysis_group(cmd: &str) -> &'static str {
    match cmd {
        "spot"
        | "wavefront"
        | "zernike"
        | "rms"
        | "footprint"
        | "relative-illumination"
        | "vignetting" => "Image quality",
        "mtf" | "geo-mtf" | "huygens-mtf" | "mtf-field" | "mtf-focus" | "psf" | "image-sim" => {
            "MTF / PSF"
        }
        "interference" | "pop" | "gaussian-beam" => "Interference / propagation",
        "ray-fan" | "opd-fan" | "pupil-fan" | "field-curvature" | "longitudinal"
        | "lateral-color" | "focal-shift" | "grid-distortion" | "seidel" => "Aberrations",
        "rays" | "coatings" | "info" | "elements" | "ghost" | "mech-check" => "Tables / system",
        _ => "Extra analyses",
    }
}

/// cmd の解析（同梱、無ければ拡張）。一覧を作らずに引く（タブの名前などで毎フレーム呼ぶ）
fn find_analysis(cmd: &str) -> Option<(&'static str, &'static str, bool)> {
    ANALYSES
        .iter()
        .find(|a| a.0 == cmd)
        .copied()
        .or_else(|| crate::ext::get(cmd).map(|a| (a.cmd, a.label, a.select)))
}

/// 解析の表示名（今の言語）
pub(crate) fn analysis_label(cmd: &str) -> String {
    find_analysis(cmd)
        .map(|a| tr(a.1).to_string())
        .unwrap_or_else(|| cmd.to_string())
}

pub(crate) fn takes_sel(cmd: &str) -> bool {
    find_analysis(cmd).is_some_and(|a| a.2)
}

/// 解析の結果の見せ方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) enum View {
    /// 図（figures::has_figure の解析）。図の無い解析は表
    #[default]
    Figure,
    Table,
    Json,
}

/// 解析パネルの設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AnaCfg {
    pub cmd: String,
    /// 0 は全視野
    pub field: usize,
    /// 0 は多色
    pub wave: usize,
    /// 追加の引数（JSON のオブジェクト。欄の値より後に当てる、上級者向け）
    pub extra: String,
    /// 引数の欄の値（名前 → 文字列。空の欄は持たない）
    pub args: BTreeMap<String, String>,
    /// 設定のオブジェクトの欄の値（拡張の解析の [`crate::ext::ObjectArg`]。入れ子を「.」でつないだ名前）
    pub sensor: BTreeMap<String, String>,
    /// 引数の欄を開いているか
    pub args_open: bool,
    pub view: View,
    /// 系の値が変わったら計算し直す
    pub live: bool,
    /// 図の見せ方（干渉縞・対数・選んだ視野・拡大など。大きいので箱に）
    pub fig: Box<crate::figures::FigState>,
}

impl Default for AnaCfg {
    fn default() -> Self {
        AnaCfg {
            cmd: "spot".into(),
            field: 0,
            wave: 0,
            extra: String::new(),
            args: BTreeMap::new(),
            sensor: BTreeMap::new(),
            args_open: false,
            view: View::Figure,
            live: true,
            fig: Default::default(),
        }
    }
}

impl AnaCfg {
    pub(crate) fn new(cmd: &str) -> AnaCfg {
        AnaCfg {
            cmd: cmd.into(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum Kind {
    /// 系の要約とパラメータ
    System,
    /// カタログ（レンズ・ガラス）と組合せ
    Catalog,
    /// 組合せのレイアウト図と要素の操作
    Layout,
    /// 組合せの系の設定（開口・視野・波長など）
    SysSettings,
    Analysis(AnaCfg),
    /// 面の表（Lens Data 風）
    LensData,
    /// 多重構成の表（Multi-Configuration Editor 風）
    MultiConfig,
    /// 最適化
    Optimize,
    /// カタログレンズの差し替え探索
    Swap,
    /// モンテカルロ公差
    Tolerance,
    /// ズーム軌跡
    Zoom,
    /// 操作の履歴（元に戻す・やり直す段の一覧）
    History,
}

impl Kind {
    /// 1 枚だけ置くパネル（解析のほかは全部）。足す操作は、あれば表に出す
    pub(crate) fn single(&self) -> bool {
        !matches!(self, Kind::Analysis(_))
    }

    /// 解析でないパネルの定義（解析は None）
    pub(crate) fn tool(&self) -> Option<&'static Tool> {
        TOOLS.iter().find(|t| t.kind == *self)
    }
}

/// 解析でないパネル（1 枚だけ）の定義。メニュー・パレット・タブの名前はここから作る
pub(crate) struct Tool {
    pub kind: Kind,
    /// 名前（tr の鍵）
    pub title: &'static str,
    /// パレットで引く英語の語（パネルを出す）
    pub keywords: &'static str,
    /// パレットで引く英語の名前（開いているパネルへ移る）
    pub name: &'static str,
}

/// 解析でないパネルの一覧（メニューの順）
pub(crate) const TOOLS: &[Tool] = &[
    Tool {
        kind: Kind::System,
        title: n_("System"),
        keywords: "system",
        name: "system",
    },
    Tool {
        kind: Kind::Catalog,
        title: n_("Catalog"),
        keywords: "catalog",
        name: "catalog",
    },
    Tool {
        kind: Kind::Layout,
        title: n_("Layout"),
        keywords: "layout combo",
        name: "layout",
    },
    Tool {
        kind: Kind::SysSettings,
        title: n_("System settings"),
        keywords: "system settings aperture field wavelength aiming",
        name: "settings",
    },
    Tool {
        kind: Kind::LensData,
        title: n_("Lens Data"),
        keywords: "lens data surfaces table",
        name: "lens data",
    },
    Tool {
        kind: Kind::MultiConfig,
        title: n_("Multi-Config"),
        keywords: "multi configuration editor mce",
        name: "multi configuration",
    },
    Tool {
        kind: Kind::Optimize,
        title: n_("Optimize"),
        keywords: "optimize optimization merit function dls",
        name: "optimize",
    },
    Tool {
        kind: Kind::Swap,
        title: n_("Swap search"),
        keywords: "swap search catalog lens replace",
        name: "swap",
    },
    Tool {
        kind: Kind::Tolerance,
        title: n_("Tolerance"),
        keywords: "tolerance monte carlo sensitivity",
        name: "tolerance",
    },
    Tool {
        kind: Kind::Zoom,
        title: n_("Zoom track"),
        keywords: "zoom cam track states focal length",
        name: "zoom",
    },
    Tool {
        kind: Kind::History,
        title: n_("History"),
        keywords: "history undo redo steps",
        name: "history",
    },
];

/// 1 つのパネル。id で見分ける
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Pane {
    pub id: u64,
    pub kind: Kind,
}

impl PartialEq for Pane {
    fn eq(&self, o: &Pane) -> bool {
        self.id == o.id
    }
}

impl Pane {
    pub(crate) fn title(&self) -> String {
        match &self.kind {
            Kind::Analysis(a) => analysis_label(&a.cmd),
            k => k
                .tool()
                .map(|t| tr(t.title).to_string())
                .unwrap_or_default(),
        }
    }
}

/// 別の OS の窓に出したパネル
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OsWin {
    pub pane: Pane,
    /// 画面上の位置（論理座標。None は OS に任せる）
    pub pos: Option<[f32; 2]>,
    pub size: [f32; 2],
    /// ドックのどこから出したか（戻すときに同じ所へ）
    #[serde(default)]
    pub home: Option<Home>,
}

/// 別の窓に出す前のドックの位置。ドックの番号は並べ替えで変わるので、近くのタブの id で覚える
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) enum Home {
    /// 同じタブの並びに残ったタブと、その並びの何番目にあったか
    With { tab: u64, index: usize },
    /// 並びに 1 つだけだったとき: 隣の側のタブ、そのタブから何段上が隣の側の根か、分け方（0 左・1 右・2 上・3 下）と割合
    Beside {
        tab: u64,
        up: usize,
        side: u8,
        fraction: f32,
    },
}

fn node_at(t: &egui_dock::Tree<Pane>, k: NodeIndex) -> Option<&egui_dock::Node<Pane>> {
    (k.0 < t.len()).then(|| &t[k])
}

fn probe(id: u64) -> Pane {
    Pane {
        id,
        kind: Kind::System,
    }
}

/// ドックの (面, ノード, タブ) にあるパネルの、戻す先を覚える（取り出す前に呼ぶ）
pub(crate) fn home_of(
    dock: &DockState<Pane>,
    egui_dock::TabPath {
        surface: s,
        node: n,
        tab: t,
    }: egui_dock::TabPath,
) -> Option<Home> {
    let tree = dock.get_surface(s)?.node_tree()?;
    let tabs = node_at(tree, n)?.tabs()?;
    if let Some(other) = tabs.iter().enumerate().find(|(k, _)| *k != t.0) {
        return Some(Home::With {
            tab: other.1.id,
            index: t.0,
        });
    }
    let parent = n.parent()?;
    let (horizontal, fraction) = match node_at(tree, parent)? {
        egui_dock::Node::Horizontal(sp) => (true, sp.fraction),
        egui_dock::Node::Vertical(sp) => (false, sp.fraction),
        _ => return None,
    };
    let sib = if n.is_left() {
        parent.right()
    } else {
        parent.left()
    };
    // 隣の側の一番浅いタブ
    let mut stack = vec![(sib, 0usize)];
    let mut best: Option<(u64, usize)> = None;
    while let Some((k, d)) = stack.pop() {
        match node_at(tree, k) {
            Some(node) if node.is_leaf() => {
                if let Some(p) = node.tabs().and_then(|t| t.first()) {
                    if !best.is_some_and(|b| b.1 <= d) {
                        best = Some((p.id, d));
                    }
                }
            }
            Some(node) if node.is_parent() => {
                stack.push((k.left(), d + 1));
                stack.push((k.right(), d + 1));
            }
            _ => {}
        }
    }
    let (tab, up) = best?;
    let side = match (horizontal, n.is_left()) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (false, false) => 3,
    };
    Some(Home::Beside {
        tab,
        up,
        side,
        fraction,
    })
}

/// 別の窓から戻したパネルを、覚えた所（無ければ選んでいる所）に入れる
pub(crate) fn redock(dock: &mut DockState<Pane>, pane: Pane, home: Option<&Home>) {
    match home {
        Some(Home::With { tab, index }) => {
            if let Some(egui_dock::TabPath {
                surface: s,
                node: n,
                ..
            }) = dock.find_tab(&probe(*tab))
            {
                if let Some(node) = dock
                    .get_surface_mut(s)
                    .and_then(|x| x.node_tree_mut())
                    .filter(|t| n.0 < t.len())
                    .map(|t| &mut t[n])
                {
                    let i = (*index).min(node.tabs_count());
                    node.insert_tab(egui_dock::TabIndex(i), pane);
                    dock.set_focused_node_and_surface(egui_dock::NodePath {
                        surface: s,
                        node: n,
                    });
                    return;
                }
            }
        }
        Some(Home::Beside {
            tab,
            up,
            side,
            fraction,
        }) => {
            if let Some(egui_dock::TabPath {
                surface: s,
                node: mut n,
                ..
            }) = dock.find_tab(&probe(*tab))
            {
                for _ in 0..*up {
                    n = n.parent().unwrap_or(n);
                }
                let (split, f) = match side {
                    0 => (egui_dock::Split::Left, *fraction),
                    1 => (egui_dock::Split::Right, *fraction),
                    2 => (egui_dock::Split::Above, *fraction),
                    _ => (egui_dock::Split::Below, *fraction),
                };
                let [_, new] = dock.split(
                    egui_dock::NodePath {
                        surface: s,
                        node: n,
                    },
                    split,
                    f.clamp(0.05, 0.95),
                    egui_dock::Node::leaf(pane),
                );
                dock.set_focused_node_and_surface(egui_dock::NodePath {
                    surface: s,
                    node: new,
                });
                return;
            }
        }
        None => {}
    }
    dock.push_to_focused_leaf(pane);
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Layout {
    pub dock: DockState<Pane>,
    pub windows: Vec<OsWin>,
}

impl Layout {
    /// パネルの id の最大
    pub(crate) fn max_id(&self) -> u64 {
        self.dock
            .iter_all_tabs()
            .map(|(_, p)| p.id)
            .chain(self.windows.iter().map(|w| w.pane.id))
            .max()
            .unwrap_or(0)
    }
}

/// 初期の並び
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Preset {
    /// 系・レイアウト図・レンズデータ・スポット・MTF
    Analysis,
    /// 組合せのレイアウト図を大きく、横にカタログと解析
    Catalog,
    /// 2 画面: 本体に系とカタログ、解析は別の窓
    TwoScreens,
}

impl Preset {
    pub(crate) const ALL: [Preset; 3] = [Preset::Analysis, Preset::Catalog, Preset::TwoScreens];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Preset::Analysis => tr("Analysis-focused"),
            Preset::Catalog => tr("Catalog and combination"),
            Preset::TwoScreens => tr("Two screens (analyses in separate windows)"),
        }
    }

    pub(crate) fn build(self) -> Layout {
        let mut id = 0;
        let mut pane = |kind: Kind| {
            id += 1;
            Pane { id, kind }
        };
        let ana = |c: &str| Kind::Analysis(AnaCfg::new(c));
        match self {
            Preset::Analysis => {
                // 左: 系（カタログはタブ）、中央上: レイアウト図、中央下: レンズデータ（多重構成はタブ）、
                // 右上: スポット、右下: MTF（波面はタブ）
                let lay = pane(Kind::Layout);
                let mut dock = DockState::new(vec![lay]);
                let sys = pane(Kind::System);
                let catalog = pane(Kind::Catalog);
                let spot = pane(ana("spot"));
                let mtf = pane(ana("mtf"));
                let wf = pane(ana("wavefront"));
                let lens = pane(Kind::LensData);
                let mce = pane(Kind::MultiConfig);
                let m = dock.main_surface_mut();
                let [center, _] = m.split_left(NodeIndex::root(), 0.22, vec![sys, catalog]);
                let [center, right] = m.split_right(center, 0.6, vec![spot]);
                m.split_below(right, 0.5, vec![mtf, wf]);
                // レイアウト図を広く（既定の窓で図が潰れない）
                m.split_below(center, 0.62, vec![lens, mce]);
                Layout {
                    dock,
                    windows: Vec::new(),
                }
            }
            Preset::Catalog => {
                let lay = pane(Kind::Layout);
                let mut dock = DockState::new(vec![lay]);
                let catalog = pane(Kind::Catalog);
                let sys = pane(Kind::System);
                let settings = pane(Kind::SysSettings);
                let spot = pane(ana("spot"));
                let mtf = pane(ana("mtf"));
                let lens = pane(Kind::LensData);
                let m = dock.main_surface_mut();
                let [center, _] =
                    m.split_left(NodeIndex::root(), 0.3, vec![catalog, sys, settings]);
                m.split_below(center, 0.68, vec![lens]);
                let [_, right] = m.split_right(center, 0.62, vec![spot]);
                m.split_below(right, 0.5, vec![mtf]);
                Layout {
                    dock,
                    windows: Vec::new(),
                }
            }
            Preset::TwoScreens => {
                let catalog = pane(Kind::Catalog);
                let mut dock = DockState::new(vec![catalog]);
                let sys = pane(Kind::System);
                dock.main_surface_mut()
                    .split_left(NodeIndex::root(), 0.25, vec![sys]);
                let windows = vec![
                    OsWin {
                        pane: pane(ana("spot")),
                        pos: None,
                        size: [720.0, 560.0],
                        home: None,
                    },
                    OsWin {
                        pane: pane(ana("mtf")),
                        pos: None,
                        size: [720.0, 480.0],
                        home: None,
                    },
                ];
                Layout { dock, windows }
            }
        }
    }
}

/// egui_dock 0.16（lensbench 0.9 まで）で保存した並びの RON を、egui_dock 0.22 の形に書き換える。
/// 0.22 はノードの `Horizontal(rect: …)`・`Vertical(…)`・`Leaf(…)` を中の構造体で包む（`Leaf((rect: …))`）。
/// 欄の名前は同じで、増えた欄（`tab_bar_hidden`）は既定の値で読め、消えた欄（`translations`）は読み飛ばされる。
/// 書き換える所が無ければ None。文字列（パネルの設定の文字）の中は触らない
pub(crate) fn migrate_dock_016(s: &str) -> Option<String> {
    fn wrap(s: &str, changed: &mut bool) -> String {
        let b = s.as_bytes();
        let mut out = String::with_capacity(s.len() + 64);
        let mut i = 0;
        let mut last = 0;
        while i < b.len() {
            match b[i] {
                b'"' => i = skip_str(b, i),
                b'H' | b'V' | b'L' if i == 0 || !is_ident(b[i - 1]) => {
                    let name = ["Horizontal", "Vertical", "Leaf"]
                        .into_iter()
                        .find(|n| s[i..].starts_with(&format!("{n}(rect:")));
                    let Some(name) = name else {
                        i += 1;
                        continue;
                    };
                    let open = i + name.len();
                    let Some(close) = matching(b, open) else {
                        i += 1;
                        continue;
                    };
                    out.push_str(&s[last..open]);
                    out.push_str("((");
                    out.push_str(&wrap(&s[open + 1..close], changed));
                    out.push_str("))");
                    *changed = true;
                    i = close + 1;
                    last = i;
                }
                _ => i += 1,
            }
        }
        out.push_str(&s[last..]);
        out
    }
    fn is_ident(c: u8) -> bool {
        c.is_ascii_alphanumeric() || c == b'_'
    }
    /// b[i] は `"`。閉じの `"` の次
    fn skip_str(b: &[u8], mut i: usize) -> usize {
        i += 1;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 2,
                b'"' => return i + 1,
                _ => i += 1,
            }
        }
        i
    }
    /// b[open] は `(`。対になる `)` の位置
    fn matching(b: &[u8], open: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut i = open;
        while i < b.len() {
            match b[i] {
                b'"' => {
                    i = skip_str(b, i);
                    continue;
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return (b[i] == b')').then_some(i);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        None
    }
    let mut changed = false;
    let out = wrap(s, &mut changed);
    changed.then_some(out)
}

#[cfg(test)]
mod tests {

    /// 並べる解析はどれも設定の項目が引ける（欄が消えない。serve の analysis_args と同じ）
    #[test]
    fn every_listed_analysis_has_args() {
        for (cmd, _, _) in ANALYSES {
            assert!(
                lensbench_engine::params::analysis_args(cmd).is_some(),
                "{cmd}"
            );
        }
    }

    use super::*;

    /// 別の窓から戻したパネルは、出す前と同じ所（並びの何番目・分けた側と割合）に戻る
    #[test]
    fn redock_returns_to_the_same_place() {
        let mut d = DockState::new(vec![probe(1), probe(2), probe(3)]);
        let [_, r] = d
            .main_surface_mut()
            .split_right(NodeIndex::root(), 0.3, vec![probe(4)]);
        d.main_surface_mut().split_below(r, 0.6, vec![probe(5)]);
        let shape = |d: &DockState<Pane>| -> Vec<Vec<u64>> {
            d.main_surface()
                .iter()
                .filter_map(|n| n.tabs().map(|t| t.iter().map(|p| p.id).collect()))
                .collect()
        };
        let before = shape(&d);
        for id in [2, 1, 3, 4, 5] {
            let at = d.find_tab(&probe(id)).unwrap();
            let home = home_of(&d, at);
            let p = d.remove_tab(at).unwrap();
            assert_ne!(shape(&d), before);
            redock(&mut d, p, home.as_ref());
            assert_eq!(shape(&d), before, "{id}");
        }
        // 4 は上 0.6 の側
        let at = d.find_tab(&probe(4)).unwrap();
        assert!(matches!(
            home_of(&d, at),
            Some(Home::Beside {
                tab: 5,
                side: 2,
                ..
            })
        ));
    }

    /// egui_dock 0.16 で保存した並び（lensbench 0.9 までの設定）を書き換えて読める
    #[test]
    fn dock_016_layout_migrates() {
        let old = concat!(
            r#"(dock:(surfaces:[Main((nodes:[Horizontal(rect:(min:(x:inf,y:inf),max:(x:-inf,y:-inf)),"#,
            r#"fraction:0.22,fully_collapsed:false,collapsed_leaf_count:0),Leaf(rect:(min:(x:inf,y:inf),"#,
            r#"max:(x:-inf,y:-inf)),viewport:(min:(x:inf,y:inf),max:(x:-inf,y:-inf)),tabs:[(id:2,kind:System),"#,
            r#"(id:3,kind:Catalog)],active:(0),scroll:0.0,collapsed:false),Vertical(rect:(min:(x:inf,y:inf),"#,
            r#"max:(x:-inf,y:-inf)),fraction:0.5,fully_collapsed:false,collapsed_leaf_count:0),Empty,Empty,"#,
            r#"Leaf(rect:(min:(x:inf,y:inf),max:(x:-inf,y:-inf)),viewport:(min:(x:inf,y:inf),max:(x:-inf,"#,
            r#"y:-inf)),tabs:[(id:1,kind:Layout)],active:(0),scroll:0.0,collapsed:false),Leaf(rect:(min:"#,
            r#"(x:inf,y:inf),max:(x:-inf,y:-inf)),viewport:(min:(x:inf,y:inf),max:(x:-inf,y:-inf)),"#,
            r#"tabs:[(id:4,kind:LensData)],active:(0),scroll:0.0,collapsed:false)],focused_node:Some((6)),"#,
            r#"collapsed:false,collapsed_leaf_count:0))],focused_surface:None,translations:(tab_context_menu:"#,
            r#"(close_button:"Close (x)",eject_button:"Eject")))"#,
            r#",windows:[])"#
        );
        assert!(ron::from_str::<Layout>(old).is_err());
        let new = migrate_dock_016(old).expect("old nodes");
        let l: Layout = ron::from_str(&new).unwrap();
        assert_eq!(l.max_id(), 4);
        assert_eq!(l.dock.iter_all_tabs().count(), 4);
        // 今の形はそのまま（書き換えない）
        let now = ron::to_string(&Preset::Analysis.build()).unwrap();
        assert_eq!(migrate_dock_016(&now), None);
    }

    #[test]
    fn presets_have_unique_ids_and_roundtrip() {
        for p in Preset::ALL {
            let l = p.build();
            let mut ids: Vec<u64> = l
                .dock
                .iter_all_tabs()
                .map(|(_, t)| t.id)
                .chain(l.windows.iter().map(|w| w.pane.id))
                .collect();
            let n = ids.len();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), n, "{p:?}");
            assert!(n >= 3);
            assert_eq!(l.max_id(), n as u64);
            // 保存して読み直せる（eframe の保存と同じ RON）
            let s = ron::to_string(&l).unwrap();
            let back: Layout = ron::from_str(&s).unwrap();
            assert_eq!(back.max_id(), l.max_id());
            assert_eq!(back.windows.len(), l.windows.len());
        }
    }
}
