//! 膜の透過率・偏光による瞳の振幅。OpticStudio の解析の「Use Polarization」に当たる選択肢で、既定は使わない。
//!
//! 波長ごとに粗い瞳の格子で偏光追跡し（[`PolGrid`]、伝搬の位相なし）、格子の Jones 行列を補間して、入射の偏光の
//! 強度の透過率 |E_出|² / |E_入|²（OpticStudio の偏光追跡と同じ約束で |E|² が強度）の平方根を瞳の振幅にする
//! （[`PupilAmp`]）。偏光なし（既定）は x・y の偏光の透過率の平均。振幅だけで、偏光による位相・成分の違い
//! （偏光ごとの PSF の重ね合わせ）は入れない。多色では、波長ごとに瞳の上で平均した光量の透過率を波長の重みに掛ける
//! （[`lensbench_core::pupil::wave_weight`]。膜の透過率が波長で違えば、像面に届く光量の比で足す）。
//!
//! 使い方: [`PolAmp::new`] で作り、[`crate::psf::psf_with`]・[`crate::huygens::huygens_psf_with`]・
//! [`crate::analysis::mtf_amp`] に渡す。系のアポダイゼーション（[`lensbench_core::pupil::Apodization`]）は
//! これとは別に常に入る。

use crate::polgrid::PolGrid;
use lensbench_core::coating::{Coatings, C64};
use lensbench_core::pupil::PupilAmp;
use lensbench_core::system::{FieldSel, Launch, System};
use lensbench_core::{invalid, Result};

/// 偏光の格子の一辺の既定（Jones 行列は瞳の上で滑らかなので粗くてよい）
pub const POL_GRID: usize = 17;

/// 膜・偏光の振幅（波長ごとの偏光の格子）
#[derive(Debug, Clone)]
pub struct PolAmp {
    /// 波長の番号ごと（追跡しなかった波長は None）
    grids: Vec<Option<PolGrid>>,
    /// 入射の偏光 (Ex, Ey)（None で偏光なし）
    pol: Option<[C64; 2]>,
}

impl PolAmp {
    /// 視野 field の波長 waves を、膜 coat で一辺 n の格子で偏光追跡する。pol は入射の偏光 (Ex, Ey)（物体の座標系、
    /// None で偏光なし = x・y の平均）
    pub fn new(
        s: &System,
        field: FieldSel,
        waves: &[usize],
        coat: &Coatings,
        n: usize,
        pol: Option<[C64; 2]>,
    ) -> Result<PolAmp> {
        if let Some(j) = pol {
            if !(j[0].norm_sqr() + j[1].norm_sqr() > 0.0) {
                invalid!("the input polarization (Ex, Ey) must not be zero");
            }
        }
        let mut grids = vec![None; s.wavelengths.len()];
        for &w in waves {
            // 視野の物点・ケラレ係数は主光線の追跡と同じもの
            let c = s.try_trace(field, &[0.0], &[0.0], Some(w), None, false)?;
            let (hx, hy) = c.hx_hy;
            let l = Launch::Pupil {
                spec: s.object_spec(hx, hy)?,
                signed: false,
                px: &[],
                py: &[],
                vig: Some(s.vig_factors(hx, hy)?),
                hxy: Some((hx, hy)),
            };
            grids[w] = Some(PolGrid::trace(s, &l, w, coat, n, n)?);
        }
        Ok(PolAmp { grids, pol })
    }
}

impl PupilAmp for PolAmp {
    fn amp(&self, w: usize, px: f64, py: f64) -> f64 {
        let Some(Some(g)) = self.grids.get(w) else {
            return f64::NAN;
        };
        let Some(v) = g.eval(px, py) else {
            return f64::NAN;
        };
        // v = [xx, xy, yx, yy, z_x, z_y]（行 = 出射の x・y、列 = 入射の x・y）
        let t = |j: [C64; 2]| -> f64 {
            let ex = v[0] * j[0] + v[1] * j[1];
            let ey = v[2] * j[0] + v[3] * j[1];
            let ez = v[4] * j[0] + v[5] * j[1];
            (ex.norm_sqr() + ey.norm_sqr() + ez.norm_sqr()) / (j[0].norm_sqr() + j[1].norm_sqr())
        };
        let one = C64::new(1.0, 0.0);
        let zero = C64::new(0.0, 0.0);
        let tr = match self.pol {
            Some(j) => t(j),
            None => 0.5 * (t([one, zero]) + t([zero, one])),
        };
        tr.sqrt()
    }
}
