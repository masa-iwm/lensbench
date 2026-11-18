//! 面の量の項（[`TermKind::Surface`]）: 曲率・コーニック・矢高・その点の厚み・径・グローバル座標

use super::*;

/// 面の量（[`TermKind::Surface`]）。長さはレンズ単位、(x, y) は面の局所座標の点
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfQ {
    /// 曲率（OpticStudio の CVVA）
    Curv,
    /// コーニック（COVA）
    Conic,
    /// 点 (x, y) の矢高（SSAG）
    Sag,
    /// 半直径の点 (半直径, 0)・(0, 半直径) の矢高（SAGX・SAGY）
    SagX,
    SagY,
    /// 点 (x, y) での次の面までの厚み: 中心の厚み − 矢高 + 次の面の矢高（STHI）
    Thick,
    /// 直径（半直径の 2 倍、DMVA の Mode 1）
    Diam,
    /// 玉の直径: ガラスでつながった面の半直径の最大の 2 倍（DMVA の Mode 0、OpticStudio の Mech Semi-Dia）
    MechDiam,
    /// 頂点のグローバル座標（グローバル座標の基準の面から、GLCX・GLCY・GLCZ）
    Gx,
    Gy,
    Gz,
}

impl SurfQ {
    pub const ALL: [SurfQ; 11] = [
        SurfQ::Curv,
        SurfQ::Conic,
        SurfQ::Sag,
        SurfQ::SagX,
        SurfQ::SagY,
        SurfQ::Thick,
        SurfQ::Diam,
        SurfQ::MechDiam,
        SurfQ::Gx,
        SurfQ::Gy,
        SurfQ::Gz,
    ];
    pub fn name(self) -> &'static str {
        match self {
            SurfQ::Curv => "curv",
            SurfQ::Conic => "conic",
            SurfQ::Sag => "sag",
            SurfQ::SagX => "sagx",
            SurfQ::SagY => "sagy",
            SurfQ::Thick => "thick",
            SurfQ::Diam => "diam",
            SurfQ::MechDiam => "mech_diam",
            SurfQ::Gx => "gx",
            SurfQ::Gy => "gy",
            SurfQ::Gz => "gz",
        }
    }
    pub fn parse(s: &str) -> Result<SurfQ> {
        let t = s.trim().to_ascii_lowercase();
        match SurfQ::ALL.iter().find(|q| q.name() == t) {
            Some(q) => Ok(*q),
            None => invalid!(
                "surface quantity {s} ({})",
                SurfQ::ALL.map(|q| q.name()).join(", ")
            ),
        }
    }
    /// 長さの量か（目標をレンズ単位から mm に直す）
    pub fn is_length(self) -> bool {
        !matches!(self, SurfQ::Curv | SurfQ::Conic)
    }
}

/// ガラス（空気でない媒質）か
fn glass(s: &System, i: usize) -> bool {
    let m = &s.surfaces[i].mat;
    m.is_medium() && !m.is_air()
}

/// 面 j の量 q（(x, y) は Sag・Thick の点）
pub(super) fn surface_value(s: &System, j: usize, q: SurfQ, x: f64, y: f64) -> Result<f64> {
    let f = &s.surfaces[j];
    let sag = |f: &lensbench_core::surface::Surface, x: f64, y: f64| f.shape().sag(x, y).0;
    let v = match q {
        SurfQ::Curv => f.c,
        SurfQ::Conic => f.k,
        SurfQ::Sag => sag(f, x, y),
        SurfQ::SagX => sag(f, f.semi, 0.0),
        SurfQ::SagY => sag(f, 0.0, f.semi),
        SurfQ::Thick => f.t - sag(f, x, y) + sag(&s.surfaces[j + 1], x, y),
        SurfQ::Diam => 2.0 * f.semi,
        SurfQ::MechDiam => {
            let (mut a, mut b) = (j, j);
            while a > 1 && glass(s, a - 1) {
                a -= 1;
            }
            while b < s.image() && glass(s, b) {
                b += 1;
            }
            2.0 * (a..=b).map(|i| s.surfaces[i].semi).fold(0.0, f64::max)
        }
        SurfQ::Gx | SurfQ::Gy | SurfQ::Gz => {
            let Some(fr) = s.frames().surf[j] else {
                invalid!("surface {j} has no position (ignored, or the object at infinity)");
            };
            fr.o[match q {
                SurfQ::Gx => 0,
                SurfQ::Gy => 1,
                _ => 2,
            }]
        }
    };
    if !v.is_finite() {
        invalid!("{} of surface {j} is not finite", q.name());
    }
    Ok(v)
}

pub(super) fn check(i: usize, surf: usize, q: SurfQ, s: &System) -> Result<()> {
    let last = if q == SurfQ::Thick {
        s.image() - 1
    } else {
        s.image()
    };
    if surf > last {
        invalid!("term {i}: surface {surf} (0 to {last})");
    }
    surface_value(s, surf, q, 0.0, 0.0).map(|_| ())
}

pub(super) fn term(s: &System, k: &TermKind) -> Result<TermOut> {
    let TermKind::Surface {
        surf,
        q,
        x,
        y,
        target,
    } = k
    else {
        unreachable!("not a surface term");
    };
    Ok((vec![surface_value(s, *surf, *q, *x, *y)? - target], None))
}

/// 半直径を読む項（自動の半直径は変数とともに動く）
pub(super) fn reads_semi(k: &TermKind) -> bool {
    match k {
        TermKind::Surface { q, .. } => {
            matches!(q, SurfQ::Diam | SurfQ::MechDiam | SurfQ::SagX | SurfQ::SagY)
        }
        TermKind::Thickness { edge, .. } => edge.is_some(),
        TermKind::Calc { args, .. } => args.iter().any(|a| match a {
            CalcArg::Term(k) => reads_semi(k),
            _ => false,
        }),
        _ => false,
    }
}
