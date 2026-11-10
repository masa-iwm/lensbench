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
            Kind::Layout => tr("組合せ").into(),
            Kind::LensData => tr("レンズデータ").into(),
            Kind::MultiConfig => tr("多重構成").into(),
            Kind::Optimize => tr("最適化").into(),
            Kind::Swap => tr("差し替え探索").into(),
            Kind::Tolerance => tr("公差").into(),
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
    /// 系・スポット・MTF・数値
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
                let spot = pane(ana("spot"));
                let catalog = pane(Kind::Catalog);
                let lens = pane(Kind::LensData);
                let mce = pane(Kind::MultiConfig);
                let mut dock = DockState::new(vec![spot, catalog, lens, mce]);
                let sys = pane(Kind::System);
                let mtf = pane(ana("mtf"));
                let wf = pane(ana("wavefront"));
                let m = dock.main_surface_mut();
                let [center, _] = m.split_left(NodeIndex::root(), 0.23, vec![sys]);
                let [center, _] = m.split_right(center, 0.55, vec![mtf]);
                m.split_below(center, 0.68, vec![wf]);
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
                    },
                    OsWin {
                        pane: pane(ana("mtf")),
                        pos: None,
                        size: [720.0, 480.0],
                    },
                ];
                Layout { dock, windows }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
