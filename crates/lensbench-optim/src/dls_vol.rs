//! 体積・質量の項（[`TermKind::Volume`]）: 面の間の要素の体積と、硝材の要素の質量

use super::surf::{surface_value, SurfQ};
use super::*;
use lensbench_core::surface::Surface;

/// 屈折率分布の媒質の密度 [g/cm³]（カタログに密度がないので、OpticStudio のマニュアルの仮定の値）
const GRIN_DENSITY: f64 = 3.6;

/// 半径の向きの Gauss-Legendre の節点の数（区間ごと）・角度の分割の数
const RADIAL: usize = 24;
const ANGULAR: usize = 32;

pub(super) fn check(i: usize, k: &TermKind, s: &System) -> Result<()> {
    let TermKind::Volume { surf, to, .. } = k else {
        unreachable!("not a volume term");
    };
    if *surf == 0 || surf > to || *to >= s.image() {
        invalid!(
            "term {i}: volume of surfaces {surf} to {to} (1 to {})",
            s.image() - 1
        );
    }
    term(s, k).map(|_| ())
}

pub(super) fn term(s: &System, k: &TermKind) -> Result<TermOut> {
    let TermKind::Volume {
        surf,
        to,
        mass,
        clear,
        target,
    } = k
    else {
        unreachable!("not a volume term");
    };
    // ミラーの後の向き・座標ブレークを挟む要素は OpticStudio で確かめていない
    if mirror_in(s, 1, *to + 1) {
        invalid!("volume of surfaces {surf} to {to}: systems with mirrors are not supported yet");
    }
    let mut v = 0.0;
    for j in *surf..=*to {
        let (a, b) = (&s.surfaces[j], &s.surfaces[j + 1]);
        if a.ignore || b.ignore || a.is_coord_break() || b.is_coord_break() {
            invalid!(
                "volume of surface {j}: elements next to ignored surfaces or coordinate breaks are not supported yet"
            );
        }
        let rho = if *mass {
            match density(s, j)? {
                Some(d) => d,
                None => continue,
            }
        } else {
            1.0
        };
        let rad = |i: usize| -> Result<f64> {
            if *clear {
                Ok(s.surfaces[i].semi)
            } else {
                Ok(surface_value(s, i, SurfQ::MechDiam, 0.0, 0.0, s.primary)? / 2.0)
            }
        };
        let r = rad(j)?.max(rad(j + 1)?);
        // mm³ → cm³
        v += rho * element(a, b, r) / 1000.0;
    }
    if !v.is_finite() {
        invalid!("the volume of surfaces {surf} to {to} is not finite");
    }
    Ok((vec![v - target], None))
}

/// 面 j の後の媒質の密度 [g/cm³]。空気は None（質量に数えない）
fn density(s: &System, j: usize) -> Result<Option<f64>> {
    use lensbench_core::material::Material;
    let f = &s.surfaces[j];
    if f.grin().is_some() {
        return Ok(Some(GRIN_DENSITY));
    }
    match &f.mat {
        m if m.is_air() || !m.is_medium() => Ok(None),
        Material::Glass(name) => {
            let Some(g) = s.glasses.get(name) else {
                invalid!("surface {j}: glass {name} is not in the catalogs");
            };
            match g.props.map(|p| p.density) {
                Some(d) if d > 0.0 => Ok(Some(d)),
                _ => invalid!("surface {j}: glass {name} has no density in the catalog"),
            }
        }
        _ => invalid!("surface {j}: the material has no density (only catalog glasses)"),
    }
}

/// 面 a・b の間の、半径 r の円柱の中の体積 [mm³]: ∫∫ (t + 矢高_b − 矢高_a) dA。面はクリア半径の外で縁の高さのまま
fn element(a: &Surface, b: &Surface, r: f64) -> f64 {
    let sag = |f: &Surface, x: f64, y: f64| {
        let h = x.hypot(y);
        let k = if h > f.semi && h > 0.0 {
            f.semi / h
        } else {
            1.0
        };
        f.shape().sag(x * k, y * k).0
    };
    // 半径の区間の境目（矢高はクリア半径で折れる）
    let mut cuts = vec![0.0, r];
    cuts.extend([a.semi, b.semi].into_iter().filter(|&h| h > 0.0 && h < r));
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let (gx, gw) = lensbench_core::opd::gauss_legendre(RADIAL);
    let mut v = 0.0;
    for w in cuts.windows(2) {
        let (h0, h1) = (w[0], w[1]);
        let (m, d) = ((h0 + h1) / 2.0, (h1 - h0) / 2.0);
        for (x, wx) in gx.iter().zip(&gw) {
            let h = m + d * x;
            let mut ring = 0.0;
            for i in 0..ANGULAR {
                let (sn, cs) = (std::f64::consts::TAU * i as f64 / ANGULAR as f64).sin_cos();
                let (px, py) = (h * cs, h * sn);
                ring += a.t + sag(b, px, py) - sag(a, px, py);
            }
            v += wx * d * h * ring * std::f64::consts::TAU / ANGULAR as f64;
        }
    }
    v
}
