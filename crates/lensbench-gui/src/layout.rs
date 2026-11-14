//! パネルと並び（レイアウト）。パネルは egui_dock でドッキング・タブ・フロートでき、別の OS の窓（別モニター）にも
//! 出せる。並びは名前を付けて保存・呼び出しでき、初期の並びをいくつか用意する。
//!
//! 保存するのはパネルの種類と設定（解析の種類・視野・波長など）だけで、計算結果は保存しない。

use std::collections::BTreeMap;

use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};

use crate::i18n::tr;

/// 解析の種類（serve の analyze の cmd）、表示名、視野・波長を選べるか
pub const ANALYSES: &[(&str, &str, bool)] = &[
    ("spot", "スポット", true),
    ("mtf", "回折 MTF", true),
    ("wavefront", "波面収差", true),
    ("zernike", "Zernike 係数", true),
    ("rms", "RMS", true),
    ("psf", "回折 PSF", true),
    ("geo-mtf", "幾何 MTF", true),
    ("huygens-mtf", "Huygens MTF", true),
    ("mtf-field", "フィールド MTF", false),
    ("mtf-focus", "スルーフォーカス MTF", true),
    ("fringe", "干渉縞（2 光束）", true),
    ("pop", "簡易 POP", true),
    ("rays", "光線の表", false),
    ("coatings", "膜の境界", false),
    ("info", "システムデータ", false),
    ("elements", "玉モード", false),
    ("ghost", "ゴースト", false),
    ("mech-check", "機構のケラレ", false),
];

/// 解析の一覧（同梱の [`ANALYSES`] と拡張の解析 [`crate::ext`]）: (cmd, 表示名（日本語）, 視野・波長を選べるか)
pub fn analyses() -> Vec<(&'static str, &'static str, bool)> {
    let mut v = ANALYSES.to_vec();
    v.extend(
        crate::ext::list()
            .iter()
            .map(|a| (a.cmd, a.label, a.select)),
    );
    v
}

/// 解析の説明（ツールチップ、日本語。表示は tr で）。無ければ None
pub fn analysis_help(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "spot" => "スポットダイアグラム: 像面での光線の位置と RMS・GEO 半径",
        "mtf" => "回折 MTF: 周波数ごとの変調度（子午・球欠）。瞳関数の自己相関から",
        "wavefront" => "波面収差: 主光線基準の OPD の地図と P-V・RMS [波]",
        "zernike" => "Zernike 係数 [波]（波面を Zernike 多項式で当てはめた係数）",
        "rms" => "RMS（スポット半径・波面・Strehl の近似）を視野・フォーカス・波長に対して",
        "psf" => "回折 PSF: 瞳関数の DFT。強度は収差のない同じ瞳の中心を 1 とする",
        "geo-mtf" => "幾何 MTF: 光線の位置の分布から求めた MTF",
        "huygens-mtf" => "Huygens MTF: Huygens PSF のフーリエ変換（像面の上の周波数）",
        "mtf-field" => "フィールド MTF: 正規化視野 0〜1 に対する MTF（周波数ごと）",
        "mtf-focus" => "スルーフォーカス MTF: 像面をずらしたときの MTF",
        "fringe" => "干渉縞（2 光束）: 2 つの構成か状態の光を重ねた縞",
        "pop" => "簡易 POP: 光線から作った場を伝えた照度",
        "rays" => "光線の表: 面ごとの交点・方向・光路長",
        "coatings" => "膜の境界: 全構成の境界（膜・前後の媒質）と扱い",
        "info" => "システムデータ: 焦点距離・F 値・瞳・像高など",
        "elements" => "玉モード: 系を玉・鏡・座標ブレーク・空気間隔に分けて示す",
        "ghost" => "ゴースト: 2 回反射の近軸の焦点の位置・像面でのぼけと強度の比",
        "mech-check" => "機構のケラレ: 機構部品の穴の外に出た光線の割合",
        _ => return None,
    })
}

/// 解析の群の並び
pub const ANALYSIS_GROUPS: &[&str] = &["像の質", "MTF・PSF", "干渉・伝搬", "表・系", "追加の解析"];

/// 解析の群（一覧の見出し、日本語。表示は tr で）。拡張の解析は「追加の解析」
pub fn analysis_group(cmd: &str) -> &'static str {
    match cmd {
        "spot" | "wavefront" | "zernike" | "rms" => "像の質",
        "mtf" | "geo-mtf" | "huygens-mtf" | "mtf-field" | "mtf-focus" | "psf" => "MTF・PSF",
        "fringe" | "pop" => "干渉・伝搬",
        "rays" | "coatings" | "info" | "elements" | "ghost" | "mech-check" => "表・系",
        _ => "追加の解析",
    }
}

/// 解析の表示名（今の言語）
pub fn analysis_label(cmd: &str) -> String {
    analyses()
        .iter()
        .find(|a| a.0 == cmd)
        .map(|a| tr(a.1).to_string())
        .unwrap_or_else(|| cmd.to_string())
}

pub fn takes_sel(cmd: &str) -> bool {
    analyses().iter().find(|a| a.0 == cmd).is_some_and(|a| a.2)
}

/// 解析の結果の見せ方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum View {
    /// 図（figures::has_figure の解析）。図の無い解析は表
    #[default]
    Figure,
    Table,
    Json,
}

/// 解析パネルの設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnaCfg {
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
        }
    }
}

impl AnaCfg {
    pub fn new(cmd: &str) -> AnaCfg {
        AnaCfg {
            cmd: cmd.into(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Kind {
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
}

/// 1 つのパネル。id で見分ける
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pane {
    pub id: u64,
    pub kind: Kind,
}

impl PartialEq for Pane {
    fn eq(&self, o: &Pane) -> bool {
        self.id == o.id
    }
}

impl Pane {
    pub fn title(&self) -> String {
        match &self.kind {
            Kind::System => tr("系").into(),
            Kind::Catalog => tr("カタログ").into(),
            Kind::Layout => tr("レイアウト").into(),
            Kind::LensData => tr("レンズデータ").into(),
            Kind::MultiConfig => tr("多重構成").into(),
            Kind::Optimize => tr("最適化").into(),
            Kind::Swap => tr("差し替え探索").into(),
            Kind::Tolerance => tr("公差").into(),
            Kind::Zoom => tr("ズーム軌跡").into(),
            Kind::SysSettings => tr("系の設定").into(),
            Kind::Analysis(a) => analysis_label(&a.cmd),
        }
    }
}

/// 別の OS の窓に出したパネル
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsWin {
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
pub enum Home {
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
pub fn home_of(
    dock: &DockState<Pane>,
    (s, n, t): (egui_dock::SurfaceIndex, NodeIndex, egui_dock::TabIndex),
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
        egui_dock::Node::Horizontal { fraction, .. } => (true, *fraction),
        egui_dock::Node::Vertical { fraction, .. } => (false, *fraction),
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
pub fn redock(dock: &mut DockState<Pane>, pane: Pane, home: Option<&Home>) {
    match home {
        Some(Home::With { tab, index }) => {
            if let Some((s, n, _)) = dock.find_tab(&probe(*tab)) {
                if let Some(node) = dock
                    .get_surface_mut(s)
                    .and_then(|x| x.node_tree_mut())
                    .filter(|t| n.0 < t.len())
                    .map(|t| &mut t[n])
                {
                    let i = (*index).min(node.tabs_count());
                    node.insert_tab(egui_dock::TabIndex(i), pane);
                    dock.set_focused_node_and_surface((s, n));
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
            if let Some((s, mut n, _)) = dock.find_tab(&probe(*tab)) {
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
                    (s, n),
                    split,
                    f.clamp(0.05, 0.95),
                    egui_dock::Node::leaf(pane),
                );
                dock.set_focused_node_and_surface((s, new));
                return;
            }
        }
        None => {}
    }
    dock.push_to_focused_leaf(pane);
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Layout {
    pub dock: DockState<Pane>,
    pub windows: Vec<OsWin>,
}

impl Layout {
    /// パネルの id の最大
    pub fn max_id(&self) -> u64 {
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
pub enum Preset {
    /// 系・レイアウト図・レンズデータ・スポット・MTF
    Analysis,
    /// 組合せのレイアウト図を大きく、横にカタログと解析
    Catalog,
    /// 2 画面: 本体に系とカタログ、解析は別の窓
    TwoScreens,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Analysis, Preset::Catalog, Preset::TwoScreens];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Analysis => tr("解析中心"),
            Preset::Catalog => tr("カタログと組合せ"),
            Preset::TwoScreens => tr("2 画面（解析を別の窓に）"),
        }
    }

    pub fn build(self) -> Layout {
        let mut id = 0;
        let mut pane = |kind: Kind| {
            id += 1;
            Pane { id, kind }
        };
        let ana = |c: &str| Kind::Analysis(AnaCfg::new(c));
        match self {
            Preset::Analysis => {
                // 左: 系（カタログはタブ）、中央上: レイアウト図、中央下: レンズデータ（多重構成はタブ）、
                // 右上: スポット、右下: MTF（波面はタブ）（GUX-04）
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
                m.split_below(center, 0.5, vec![lens, mce]);
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
