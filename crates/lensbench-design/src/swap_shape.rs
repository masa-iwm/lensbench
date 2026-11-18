//! 差し替え探索の同じ形の候補（置いた玉の面が同じ選び方）をまとめる（[`super`]）

use super::Pick;
use lensbench_core::catalog::LensCatalog;
use lensbench_core::elements::{self, Kind as EKind};
use lensbench_core::{par, zmx};
use std::collections::HashMap;
use std::path::PathBuf;

/// 玉の .zmx を置いたときの面の形の鍵（向きのまま, 裏返し）。面の説明（comment）は除き、ほかの欄（膜を含む）が
/// すべて同じなら同じ鍵。読めない玉は None（まとめない）
fn lens_shape(zmx_text: &str, glass_dirs: &[PathBuf]) -> Option<(String, String)> {
    let raw = zmx::parse_text(zmx_text).ok()?;
    let s = lensbench_project::catalog_system(&raw, glass_dirs, None).ok()?;
    let e = elements::split(&s)
        .ok()?
        .into_iter()
        .find(|e| e.kind == EKind::Lens)?;
    let key = |e: &elements::Element| {
        let v: Vec<lensbench_core::surface::Surface> = e
            .surfaces
            .iter()
            .map(|q| lensbench_core::surface::Surface {
                comment: String::new(),
                ..q.clone()
            })
            .collect();
        format!("{v:?}")
    };
    let r = elements::reverse(&e).ok()?;
    Some((key(&e), key(&r)))
}

/// 所ごとの選び方の代表（[`super::SwapSearch::same`]）。鍵が作れない選び方は自分
pub(super) fn same_shapes(
    cat: &LensCatalog,
    picks: &[Vec<Pick>],
    glass_dirs: &[PathBuf],
) -> Vec<Vec<usize>> {
    let mut parts: Vec<(&str, &str)> = picks
        .iter()
        .flatten()
        .map(|p| (p.lens.vendor.as_str(), p.lens.part.as_str()))
        .collect();
    parts.sort_unstable();
    parts.dedup();
    let keys = par::map(&parts, |(v, p)| {
        cat.get(v, p).and_then(|l| lens_shape(&l.zmx, glass_dirs))
    });
    let shape: HashMap<(&str, &str), (String, String)> = parts
        .into_iter()
        .zip(keys)
        .filter_map(|(k, v)| Some((k, v?)))
        .collect();
    picks
        .iter()
        .map(|v| {
            let mut first: HashMap<&String, usize> = HashMap::new();
            v.iter()
                .enumerate()
                .map(
                    |(c, p)| match shape.get(&(p.lens.vendor.as_str(), p.lens.part.as_str())) {
                        Some((f, r)) => *first.entry(if p.reversed { r } else { f }).or_insert(c),
                        None => c,
                    },
                )
                .collect()
        })
        .collect()
}
