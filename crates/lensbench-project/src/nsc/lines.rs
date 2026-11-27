//! 図に描く光線の折れ線: 光源の一部の光線を追い直して、検出器に届いた枝の通った点を出す（レイアウト図への重ね描き）

use super::{Branch, End, StartRay, Tracer};
use crate::raypath::EventKind;
use lensbench_core::math::V3;

/// 検出器に届いた枝の折れ線
#[derive(Debug, Clone, PartialEq)]
pub struct Polyline {
    /// (場面の面, 種類) の列（[`super::PathRow::events`] と同じ書き方）
    pub events: Vec<(usize, EventKind)>,
    /// 検出器で数えたパワー [W]
    pub power: f64,
    /// 光源の始点と出来事の点（大域）
    pub pts: Vec<V3>,
}

impl Tracer<'_> {
    /// 光源の光線のうち番号 ids（starts の番号。乱数もこの番号なので [`Tracer::run`] と同じ枝になる）だけを追い、
    /// 検出器に届いた枝の折れ線を ids の順・枝の終わった順に返す
    pub fn polylines(&self, starts: &[StartRay], ids: &[usize]) -> Vec<Polyline> {
        let parts = lensbench_core::par::map(ids, |&i| {
            let mut out: Vec<Branch> = Vec::new();
            if let Some(st) = starts.get(i) {
                self.trace(st, i as u64, false, &mut out);
            }
            out.into_iter()
                .filter(|b| matches!(b.end, End::Detected { .. }))
                .map(|b| Polyline {
                    events: b.hits.iter().map(|h| (h.face, h.kind)).collect(),
                    power: b.power,
                    pts: std::iter::once(starts[i].p)
                        .chain(b.hits.iter().map(|h| h.p))
                        .collect(),
                })
                .collect::<Vec<_>>()
        });
        parts.into_iter().flatten().collect()
    }
}

/// n 本の光源の光線から、だいたい k 本を等しい間隔で選ぶ（偏光なしの光源の 2 つの状態は並んでいるので、組の
/// 最初の番号に揃える: step は偶数）
pub fn spread(n: usize, k: usize) -> Vec<usize> {
    if k == 0 || n == 0 {
        return Vec::new();
    }
    let mut step = n.div_ceil(k).max(1);
    if step > 1 && step % 2 == 1 {
        step += 1;
    }
    (0..n).step_by(step).collect()
}

#[cfg(test)]
mod tests {
    use super::spread;

    #[test]
    fn spread_picks_evenly_and_keeps_pairs() {
        assert!(spread(10, 0).is_empty());
        assert_eq!(spread(4, 10), vec![0, 1, 2, 3]);
        assert_eq!(spread(100, 10), (0..100).step_by(10).collect::<Vec<_>>());
        assert_eq!(spread(90, 10), (0..90).step_by(10).collect::<Vec<_>>());
        assert!(spread(882, 100).iter().all(|i| i % 2 == 0));
    }
}
