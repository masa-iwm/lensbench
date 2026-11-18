//! 評価関数が使う瞳のガウス求積の点の使い回し（core の analysis から移した。使うのはこのクレートだけ）

use crate::Quad;
use lensbench_core::analysis::{pupil_gauss, pupil_gauss_os};
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
                })
            })
            .clone()
    })
}

/// 求積の点の数
pub(crate) fn quad_len(rings: usize, quad: Quad) -> usize {
    match quad {
        Quad::Lensbench => rings * (4 * rings + 2),
        Quad::OpticStudio => rings * 2 * rings,
    }
}

/// [`pupil_shared`] の点と、系のアポダイゼーションの強度 A²（amplitude なら振幅 A。波面の項）を掛けた重み
/// （解析と同じ: [`lensbench_core::analysis::apodized`]・[`lensbench_core::analysis::apodized_amplitude`]）
pub(crate) fn apodized_gauss(
    s: &lensbench_core::System,
    field: lensbench_core::system::FieldSel,
    rings: usize,
    quad: Quad,
    amplitude: bool,
) -> lensbench_core::error::Result<(Rc<PupilPts>, Vec<f64>)> {
    let pg = pupil_shared(rings, quad);
    let f = if amplitude {
        lensbench_core::analysis::apodized_amplitude
    } else {
        lensbench_core::analysis::apodized
    };
    let w = f(s, field, &pg.0, &pg.1, &pg.2)?;
    Ok((pg, w))
}
