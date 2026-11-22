//! 重ね描き・多重構成の書き出しで並べる構成の選び方（`--cfgs`・`--states`・`--beam-paths`・`--beams`）

use clap::Args;
use lensbench_core::System;

use crate::Common;

/// 重ね描き・多重構成の書き出しで並べる構成
#[derive(Args, Clone, Default)]
pub(crate) struct ConfigSel {
    /// .zmx multi-configurations (from 1, comma-separated, all for every one)
    #[arg(long)]
    pub(crate) cfgs: Option<String>,
    /// Project states (comma-separated names, all for every one)
    #[arg(long)]
    pub(crate) states: Option<String>,
    /// Beam splitter paths (comma-separated strings of T and R, all for every possible path)
    #[arg(long = "beam-paths")]
    pub(crate) paths: Option<String>,
    /// Project beams (comma-separated names, all for every beam not marked off)
    #[arg(long)]
    pub(crate) beams: Option<String>,
}

impl ConfigSel {
    pub(crate) fn overlay(&self) -> lensbench_engine::load::Overlay {
        lensbench_engine::load::Overlay {
            cfgs: self.cfgs.clone(),
            states: self.states.clone(),
            paths: self.paths.clone(),
            beams: self.beams.clone(),
        }
    }
}

/// 重ね描きする構成の系と名前。重ね描きでなければ今の系 1 つ（名前は空）。読んだときの注意は標準エラーへ
pub(crate) fn overlay_systems(
    c: &Common,
    l: &ConfigSel,
) -> lensbench_core::Result<Vec<(String, System)>> {
    let (systems, notes) = lensbench_engine::load::overlay_systems(&c.spec(), &l.overlay())?;
    for w in notes {
        eprintln!("{}", lensbench_core::i18n::warning(&w));
    }
    Ok(systems)
}

/// 最適化する構成の系と、構成ごとのプロジェクトのブロックの面の範囲（変数 `ブロック:面.種類`）。構成を選ばなければ
/// 今の系 1 つ。読んだときの注意は標準エラーへ
pub(crate) fn optimize_systems(
    c: &Common,
    l: &ConfigSel,
) -> lensbench_core::Result<(Vec<System>, Vec<Vec<lensbench_project::BlockSpans>>)> {
    let (v, notes) = lensbench_engine::load::overlay_systems_spans(&c.spec(), &l.overlay())?;
    for w in notes {
        eprintln!("{}", lensbench_core::i18n::warning(&w));
    }
    Ok(v.into_iter().unzip())
}
