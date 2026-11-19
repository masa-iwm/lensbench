//! ザイデル収差係数の項（[`TermKind::Seidel`]）、実光線の歪曲の項（[`TermKind::Distortion`]）、
//! 縦収差・軸上色収差の項（[`TermKind::Longitudinal`]）と倍率色収差の項（[`TermKind::LateralColor`]）

use super::*;
use lensbench_core::seidel::{
    chief_height, distortion_against, distortion_max, distortion_point, distortion_ref,
    ftheta_distortion, longitudinal, seidel, DistRef,
};

/// ザイデル収差係数（波面収差の係数 [波]、[`lensbench_core::seidel::Seidel::waves`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeidelQ {
    /// 球面収差 W040 = S_I/8（SPHA）
    Spha,
    /// コマ W131 = S_II/2（COMA）
    Coma,
    /// 非点収差 W222 = S_III/2（ASTI）
    Asti,
    /// 像面湾曲 W220 = S_IV/4（FCUR）
    Fcur,
    /// 歪曲 W311 = S_V/2（DIST の面の寄与）
    Dist,
}

impl SeidelQ {
    pub const ALL: [SeidelQ; 5] = [
        SeidelQ::Spha,
        SeidelQ::Coma,
        SeidelQ::Asti,
        SeidelQ::Fcur,
        SeidelQ::Dist,
    ];
    pub fn name(self) -> &'static str {
        match self {
            SeidelQ::Spha => "spha",
            SeidelQ::Coma => "coma",
            SeidelQ::Asti => "asti",
            SeidelQ::Fcur => "fcur",
            SeidelQ::Dist => "dist",
        }
    }
    pub fn parse(s: &str) -> Result<SeidelQ> {
        let t = s.trim().to_ascii_lowercase();
        match SeidelQ::ALL.iter().find(|q| q.name() == t) {
            Some(q) => Ok(*q),
            None => invalid!(
                "Seidel quantity {s} ({})",
                SeidelQ::ALL.map(|q| q.name()).join(", ")
            ),
        }
    }
    fn index(self) -> usize {
        SeidelQ::ALL.iter().position(|q| *q == self).unwrap_or(0)
    }
}

pub(super) fn check(i: usize, k: &TermKind, s: &System) -> Result<()> {
    let (wave, target) = match k {
        TermKind::Seidel {
            surf, wave, target, ..
        } => {
            if *surf > s.image() {
                invalid!("term {i}: surface {surf} (0 to {})", s.image());
            }
            (wave, target)
        }
        TermKind::Distortion { wave, target, .. } => (wave, target),
        TermKind::Longitudinal {
            wave,
            wave2,
            zone,
            target,
        } => {
            if !(0.0..=1.0).contains(zone) {
                invalid!("term {i}: zone {zone} (0 to 1)");
            }
            if let Some(w) = wave2 {
                if *w >= s.wavelengths.len() {
                    invalid!("term {i}: wavelength {w} (of {})", s.wavelengths.len());
                }
            }
            (wave, target)
        }
        TermKind::LateralColor {
            wave,
            wave2,
            target,
        } => {
            if *wave2 >= s.wavelengths.len() {
                invalid!("term {i}: wavelength {wave2} (of {})", s.wavelengths.len());
            }
            (wave, target)
        }
        _ => unreachable!("not a Seidel term"),
    };
    if !target.is_finite() {
        invalid!("term {i}: target {target}");
    }
    if let Some(w) = wave {
        if *w >= s.wavelengths.len() {
            invalid!("term {i}: wavelength {w} (of {})", s.wavelengths.len());
        }
    }
    value(s, k).map(|_| ())
}

fn value(s: &System, k: &TermKind) -> Result<f64> {
    match k {
        TermKind::Seidel { surf, q, wave, .. } => {
            let w = wave.unwrap_or(s.primary);
            Ok(seidel(s, w)?.waves(*surf)[q.index()])
        }
        TermKind::Distortion {
            wave,
            absolute,
            field,
            reference,
            mode,
            abcd,
            component,
            ..
        } => {
            let w = wave.unwrap_or(s.primary);
            let norm = |f: &Option<FieldSel>, d: (f64, f64)| -> Result<(f64, f64)> {
                match f {
                    Some(f) => s.field_sel_norm(*f, w),
                    None => Ok(d),
                }
            };
            let h = norm(field, (0.0, 1.0))?;
            // 軸上の視野の基準は近軸主光線の倍率（数値の微分より正確）
            let r = match reference {
                Some(r) => Some(norm(&Some(*r), (0.0, 0.0))?).filter(|&h| h != (0.0, 0.0)),
                None => None,
            };
            let base = || -> Result<DistRef> {
                match abcd {
                    Some([a, b, c, d]) => {
                        let mut x = distortion_point(s, w, r)?;
                        x.j = [[*a, *b], [*c, *d]];
                        Ok(x)
                    }
                    None => distortion_ref(s, w, r),
                }
            };
            let (d, pct) = match mode {
                DistMode::At => distortion_against(s, w, h, &base()?, *component)?,
                DistMode::Max => {
                    let m = distortion_max(s, w, h, *absolute)?;
                    (m, m)
                }
                DistMode::FTheta => ftheta_distortion(s, w)?,
                DistMode::Coef(k) => {
                    let j = distortion_ref(s, w, r)?.j;
                    let v = j[k / 2][k % 2];
                    (v, v)
                }
            };
            Ok(if *absolute { d } else { pct })
        }
        TermKind::Longitudinal {
            wave, wave2, zone, ..
        } => {
            let w = wave.unwrap_or(s.primary);
            let a = longitudinal(s, w, *zone)?;
            Ok(match wave2 {
                Some(w2) => longitudinal(s, *w2, *zone)? - a,
                None => a,
            })
        }
        TermKind::LateralColor { wave, wave2, .. } => {
            let w = wave.unwrap_or(s.primary);
            let v = chief_height(s, *wave2) - chief_height(s, w);
            if !v.is_finite() {
                invalid!("the lateral color is not finite");
            }
            Ok(v)
        }
        _ => unreachable!("not a Seidel term"),
    }
}

pub(super) fn term(s: &System, k: &TermKind) -> Result<TermOut> {
    let t = k.target().unwrap_or(0.0);
    Ok((vec![value(s, k)? - t], None))
}
