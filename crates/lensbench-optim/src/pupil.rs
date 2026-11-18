//! 評価関数が使う瞳のガウス求積の点の使い回し（core の analysis から移した。使うのはこのクレートだけ）

use crate::Quad;
use lensbench_analysis::analysis::{pupil_gauss, pupil_gauss_os};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// 瞳のガウス求積の点（px, py, 重み）
pub(crate) type PupilPts = (Vec<f64>, Vec<f64>, Vec<f64>);

/// 瞳のガウス求積の点（置き方 quad）を、スレッドごとに
/// 覚えて使い回す。評価関数（最適化・差し替え探索）は項ごと・反復ごとに同じ点を何度も使うため。値は [`pupil_gauss`]・
/// [`pupil_gauss_os`] と同じ
pub(crate) fn pupil_shared(rings: usize, quad: Quad) -> Rc<PupilPts> {
    thread_local! {
        static CACHE: RefCell<HashMap<(usize, Quad), Rc<PupilPts>>> = RefCell::new(HashMap::new());
    }
    CACHE.with(|c| {
        c.borrow_mut()
            .entry((rings, quad))
            .or_insert_with(|| {
                Rc::new(match quad {
                    Quad::Lensbench => pupil_gauss(rings, None),
                    Quad::OpticStudio => pupil_gauss_os(rings),
                    Quad::Arms(arms) => {
                        // pupil_gauss の腕は (a + 0.5)·360°/arms。半分の刻みだけ戻す
                        let (px, py, w) = pupil_gauss(rings, Some(arms));
                        let (s, c) = (std::f64::consts::PI / arms as f64).sin_cos();
                        let rx = px.iter().zip(&py).map(|(x, y)| c * x + s * y).collect();
                        let ry = px.iter().zip(&py).map(|(x, y)| c * y - s * x).collect();
                        (rx, ry, w)
                    }
                    Quad::Grid(n) => pupil_grid(n),
                })
            })
            .clone()
    })
}

/// 瞳を n × n の升に分けた升の中心のうち瞳の中（r ≤ 1）の点、重みは等しく和 1
fn pupil_grid(n: usize) -> PupilPts {
    let c = |i: usize| (2 * i + 1) as f64 / n as f64 - 1.0;
    let (mut px, mut py) = (Vec::new(), Vec::new());
    for i in 0..n {
        for k in 0..n {
            let (x, y) = (c(k), c(i));
            if x * x + y * y <= 1.0 + 1e-12 {
                px.push(x);
                py.push(y);
            }
        }
    }
    let w = vec![1.0 / px.len().max(1) as f64; px.len()];
    (px, py, w)
}

/// 求積の点の数
pub(crate) fn quad_len(rings: usize, quad: Quad) -> usize {
    match quad {
        Quad::Lensbench => rings * (4 * rings + 2),
        Quad::OpticStudio => rings * 2 * rings,
        Quad::Arms(arms) => rings * arms,
        Quad::Grid(n) => pupil_grid(n).0.len(),
    }
}

/// [`pupil_shared`] の点と、系のアポダイゼーションの強度 A²（amplitude なら振幅 A。波面の項）を掛けた重み
/// （解析と同じ: [`lensbench_analysis::analysis::apodized`]・[`lensbench_analysis::analysis::apodized_amplitude`]）
pub(crate) fn apodized_gauss(
    s: &lensbench_core::System,
    field: lensbench_core::system::FieldSel,
    rings: usize,
    quad: Quad,
    amplitude: bool,
) -> lensbench_core::error::Result<(Rc<PupilPts>, Vec<f64>)> {
    let pg = pupil_shared(rings, quad);
    let f = if amplitude {
        lensbench_analysis::analysis::apodized_amplitude
    } else {
        lensbench_analysis::analysis::apodized
    };
    let w = f(s, field, &pg.0, &pg.1, &pg.2)?;
    Ok((pg, w))
}
