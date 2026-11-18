//! 評価関数が使う瞳のガウス求積の点の使い回し（core の analysis から移した。使うのはこのクレートだけ）

use lensbench_core::analysis::pupil_gauss;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// 瞳のガウス求積の点（px, py, 重み）
pub(crate) type PupilPts = (Vec<f64>, Vec<f64>, Vec<f64>);

/// [`pupil_gauss`]（arms は既定）を、スレッドごとに覚えて使い回す。
/// 評価関数（最適化・差し替え探索）は項ごと・反復ごとに同じ点を何度も使うため。値は [`pupil_gauss`] と同じ
pub(crate) fn pupil_gauss_shared(rings: usize) -> Rc<PupilPts> {
    thread_local! {
        static CACHE: RefCell<HashMap<usize, Rc<PupilPts>>> = RefCell::new(HashMap::new());
    }
    CACHE.with(|c| {
        c.borrow_mut()
            .entry(rings)
            .or_insert_with(|| Rc::new(pupil_gauss(rings, None)))
            .clone()
    })
}

/// [`pupil_gauss_shared`] の点と、系のアポダイゼーションの強度 A²（amplitude なら振幅 A。波面の項）を掛けた重み
/// （解析と同じ: [`lensbench_core::analysis::apodized`]・[`lensbench_core::analysis::apodized_amplitude`]）
pub(crate) fn apodized_gauss(
    s: &lensbench_core::System,
    field: lensbench_core::system::FieldSel,
    rings: usize,
    amplitude: bool,
) -> lensbench_core::error::Result<(Rc<PupilPts>, Vec<f64>)> {
    let pg = pupil_gauss_shared(rings);
    let f = if amplitude {
        lensbench_core::analysis::apodized_amplitude
    } else {
        lensbench_core::analysis::apodized
    };
    let w = f(s, field, &pg.0, &pg.1, &pg.2)?;
    Ok((pg, w))
}
