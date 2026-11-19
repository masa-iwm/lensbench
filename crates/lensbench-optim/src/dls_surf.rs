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
    /// 面のパワー (n′ − n) c（POWR）
    Power,
    /// 近軸周辺光線の y · n · i（高さ · 面の前の屈折率 · 入射角、YNIP）
    Ynip,
    /// 面の後の媒質の屈折率（INDX）
    Index,
    /// 半直径（範囲の項 [`TermKind::SurfEach`] の MNSD・MXSD）
    Semi,
    /// 面の局所座標の z 軸のグローバル座標での向き（GLCA・GLCB・GLCC）
    Ga,
    Gb,
    Gc,
    /// 面の後のガラスの d 線（587.5618 nm）の屈折率・アッベ数 νd（カタログの温度、分散式から）・ΔPgF・相対価格・熱膨張係数
    /// −30〜70 °C [10⁻⁶/K]（GCOS・GTCE、範囲の項で MNIN〜MXPD）。空気は 0
    Nd,
    Vd,
    Dpgf,
    Cost,
    Tce,
}

impl SurfQ {
    pub const ALL: [SurfQ; 23] = [
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
        SurfQ::Power,
        SurfQ::Ynip,
        SurfQ::Index,
        SurfQ::Semi,
        SurfQ::Ga,
        SurfQ::Gb,
        SurfQ::Gc,
        SurfQ::Nd,
        SurfQ::Vd,
        SurfQ::Dpgf,
        SurfQ::Cost,
        SurfQ::Tce,
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
            SurfQ::Power => "power",
            SurfQ::Ynip => "ynip",
            SurfQ::Index => "index",
            SurfQ::Semi => "semi",
            SurfQ::Ga => "ga",
            SurfQ::Gb => "gb",
            SurfQ::Gc => "gc",
            SurfQ::Nd => "nd",
            SurfQ::Vd => "vd",
            SurfQ::Dpgf => "dpgf",
            SurfQ::Cost => "cost",
            SurfQ::Tce => "tce",
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
        !matches!(
            self,
            SurfQ::Curv
                | SurfQ::Conic
                | SurfQ::Power
                | SurfQ::Index
                | SurfQ::Ga
                | SurfQ::Gb
                | SurfQ::Gc
                | SurfQ::Nd
                | SurfQ::Vd
                | SurfQ::Dpgf
                | SurfQ::Cost
                | SurfQ::Tce
        )
    }
    /// 波長を選べる量
    pub fn has_wave(self) -> bool {
        matches!(self, SurfQ::Power | SurfQ::Ynip | SurfQ::Index)
    }
}

/// ガラス（空気でない媒質）か
fn glass(s: &System, i: usize) -> bool {
    let m = &s.surfaces[i].mat;
    m.is_medium() && !m.is_air()
}

/// 面 j の量 q（(x, y) は Sag・Thick の点、w は Power・Ynip・Index の波長）
pub(super) fn surface_value(
    s: &System,
    j: usize,
    q: SurfQ,
    x: f64,
    y: f64,
    w: usize,
) -> Result<f64> {
    let f = &s.surfaces[j];
    // 面の前の屈折率（物体面は物体空間、無視する面・座標ブレークを飛ばす）
    let n_before = || s.index_before(j, w);
    // ミラーの面のパワー・ミラーの後の符号は OpticStudio で確かめていない
    if (q == SurfQ::Power && f.is_mirror()) || (q == SurfQ::Ynip && super::mirror_in(s, 1, j)) {
        invalid!(
            "{} of surface {j}: systems with mirrors are not supported yet",
            q.name()
        );
    }
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
            // 無視した面は無いものとして玉をつなぐ（OpticStudio 26.1 の多重構成の IGNR で確かめた）
            let ign = |i: usize| s.surfaces[i].ignore;
            let (mut a, mut b) = (j, j);
            while a > 1 && (glass(s, a - 1) || ign(a - 1)) {
                a -= 1;
            }
            while b < s.image() && (glass(s, b) || ign(b)) {
                b += 1;
            }
            // 円形の開口の面は開口の最大の半径を縁にする（OpticStudio 26.1 の多重構成の APMX で、半直径より大きくても
            // 小さくても開口の半径を使った）
            let edge = |i: usize| match s.surfaces[i].ap {
                lensbench_core::surface::Aperture::Circular { max, .. } => max,
                _ => s.surfaces[i].semi,
            };
            let semi = (a..=b).filter(|&i| !ign(i)).map(edge);
            2.0 * semi.fold(0.0, f64::max)
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
        SurfQ::Power => (s.n(j)[w] - n_before()) * f.shape().paraxial_curv(),
        SurfQ::Ynip => {
            let p = s.paraxial();
            let (ym, um) = s.ptrace(p.marginal.0[1], p.marginal.1[0], w, Some(j));
            let i = um[j - 1] + ym[j] * f.shape().paraxial_curv();
            ym[j] * n_before() * i
        }
        SurfQ::Index => s.n(j)[w],
        SurfQ::Semi => f.semi,
        SurfQ::Ga | SurfQ::Gb | SurfQ::Gc => {
            let Some(fr) = s.frames().surf[j] else {
                invalid!("surface {j} has no position (ignored, or the object at infinity)");
            };
            fr.r[match q {
                SurfQ::Ga => 0,
                SurfQ::Gb => 1,
                _ => 2,
            }][2]
        }
        SurfQ::Nd | SurfQ::Vd | SurfQ::Dpgf | SurfQ::Cost | SurfQ::Tce => {
            let g = glass_values(s, j)?;
            match q {
                SurfQ::Nd => g[0],
                SurfQ::Vd => g[1],
                SurfQ::Dpgf => g[2],
                SurfQ::Cost => g[3],
                _ => g[4],
            }
        }
    };
    if !v.is_finite() {
        invalid!("{} of surface {j} is not finite", q.name());
    }
    Ok(v)
}

/// 面 j の後のガラスの [nd, νd, ΔPgF, 相対価格, 熱膨張係数]（カタログの温度の分散式から。空気・ミラーは 0）
fn glass_values(s: &System, j: usize) -> Result<[f64; 5]> {
    use lensbench_core::material::Material;
    const D: f64 = 0.5875618;
    const F: f64 = 0.4861327;
    const C: f64 = 0.6562725;
    Ok(match &s.surfaces[j].mat {
        Material::Glass(name) => {
            let Some(g) = s.glasses.get(name) else {
                invalid!("surface {j}: glass {name} is not in the catalogs");
            };
            let nd = g.n_cat(D);
            let p = g.props.unwrap_or_default();
            [
                nd,
                (nd - 1.0) / (g.n_cat(F) - g.n_cat(C)),
                p.dpgf,
                p.cost,
                p.tce,
            ]
        }
        Material::Model { nd, vd, dpgf } => [*nd, *vd, *dpgf, 0.0, 0.0],
        Material::Index(n) => [*n, 0.0, 0.0, 0.0, 0.0],
        _ => [0.0; 5],
    })
}

pub(super) fn check(
    i: usize,
    surf: usize,
    q: SurfQ,
    wave: Option<usize>,
    s: &System,
) -> Result<()> {
    let last = if q == SurfQ::Thick {
        s.image() - 1
    } else {
        s.image()
    };
    let first = usize::from(q == SurfQ::Ynip);
    if surf < first || surf > last {
        invalid!("term {i}: surface {surf} ({first} to {last})");
    }
    if let Some(w) = wave {
        if !q.has_wave() {
            invalid!(
                "term {i}: wave goes with power, ynip and index, not {}",
                q.name()
            );
        }
        if w >= s.wavelengths.len() {
            invalid!("term {i}: wavelength {w} (of {})", s.wavelengths.len());
        }
    }
    let w = wave.unwrap_or(s.primary);
    surface_value(s, surf, q, 0.0, 0.0, w).map(|_| ())
}

pub(super) fn term(s: &System, k: &TermKind) -> Result<TermOut> {
    let TermKind::Surface {
        surf,
        q,
        x,
        y,
        wave,
        target,
    } = k
    else {
        unreachable!("not a surface term");
    };
    let w = wave.unwrap_or(s.primary);
    Ok((vec![surface_value(s, *surf, *q, *x, *y, w)? - target], None))
}

/// 半直径を読む項（自動の半直径は変数とともに動く）
pub(super) fn reads_semi(k: &TermKind) -> bool {
    match k {
        TermKind::Surface { q, .. } => {
            matches!(
                q,
                SurfQ::Diam | SurfQ::MechDiam | SurfQ::SagX | SurfQ::SagY | SurfQ::Semi
            )
        }
        TermKind::Thickness { edge, .. } => edge.is_some(),
        TermKind::SurfEach { q, .. } => *q == SurfQ::Semi,
        TermKind::Calc { args, .. } => args.iter().any(|a| match a {
            CalcArg::Term(k) => reads_semi(k),
            _ => false,
        }),
        _ => false,
    }
}

pub(super) fn semi_check(i: usize, k: &TermKind, s: &System) -> Result<()> {
    let TermKind::SurfEach { q, surf, to, .. } = k else {
        unreachable!("not a surface range term");
    };
    if *surf == 0 || surf > to || *to >= s.image() {
        invalid!(
            "term {i}: {} of surfaces {surf} to {to} (1 to {})",
            q.name(),
            s.image() - 1
        );
    }
    if !matches!(
        q,
        SurfQ::Semi | SurfQ::Nd | SurfQ::Vd | SurfQ::Dpgf | SurfQ::Cost | SurfQ::Tce
    ) {
        invalid!("term {i}: {} has no range (to)", q.name());
    }
    for j in *surf..=*to {
        surface_value(s, j, *q, 0.0, 0.0, s.primary)?;
    }
    Ok(())
}

pub(super) fn semi_term(s: &System, k: &TermKind) -> Result<TermOut> {
    let TermKind::SurfEach {
        q,
        surf,
        to,
        upper,
        target,
    } = k
    else {
        unreachable!("not a surface range term");
    };
    // ガラスの量はガラスの面だけ（OpticStudio 26.1 の MNIN〜MXPD で、空気の面は数えない）
    let counts = |i: usize| *q == SurfQ::Semi || glass(s, i);
    let mut excess = 0.0;
    for i in (*surf..=*to).filter(|&i| !s.surfaces[i].ignore && counts(i)) {
        let d = surface_value(s, i, *q, 0.0, 0.0, s.primary)? - target;
        excess += if *upper { d.max(0.0) } else { d.min(0.0) };
    }
    if !excess.is_finite() {
        invalid!("the {} of surfaces {surf} to {to} are not finite", q.name());
    }
    Ok((vec![excess], None))
}
