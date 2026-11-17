//! 実光線による解析: 瞳の標本、スポット、OPD（射出瞳の参照球）、波面マップ、Zernike。
//!
//! OPD は OpticStudio と同じ（ROPD「射出瞳」）: 参照球の中心は主波長の主光線の像点、主光線が射出瞳面
//! （像面から EXPP）と交わる点を通る。各光線を像面から参照球まで戻し、(主光線の光路 − 光線の光路) を
//! その光線の波長の波数で表す（主光線はその波長のもの）。ROPD「無限遠」は主光線に垂直な平面。

use crate::coating::C64;
use crate::error::Result;
use crate::math::*;
use crate::pupil::{PupilAmp, PupilFunction, PupilGrid, Rim, WaveAmp};
use crate::system::{Cols, FieldSel, RayBuf, System, Trace};
use crate::{invalid, notimpl, par::scoped, TanSag, Waves};

mod mtf;
mod mtf_frame;
mod spot;
mod zernike;
pub use mtf::*;
pub use mtf_frame::MtfAxes;
use mtf_frame::MtfFrame;
pub(crate) use spot::spot_of;
pub use spot::{apodized, spot, spot_arms, Spot};
pub use zernike::*;

/// n×n の格子のうち単位円内の点（[`PupilGrid::Linear`] の [`Rim::Padded`]）
pub fn pupil_grid(n: usize) -> (Vec<f64>, Vec<f64>) {
    let s = PupilGrid::Linear {
        n,
        rim: Rim::Padded,
    }
    .samples();
    (s.px, s.py)
}

/// Gauss-Legendre の節点と重み（[-1, 1]）
pub fn gauss_legendre(n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut x = vec![0.0; n];
    let mut w = vec![0.0; n];
    for i in 0..n {
        let mut z = (std::f64::consts::PI * (i as f64 + 0.75) / (n as f64 + 0.5)).cos();
        let mut dp = 0.0;
        for _ in 0..100 {
            let (mut p0, mut p1) = (1.0, z);
            for k in 2..=n {
                let p2 = ((2 * k - 1) as f64 * z * p1 - (k - 1) as f64 * p0) / k as f64;
                p0 = p1;
                p1 = p2;
            }
            let pn = if n == 1 { z } else { p1 };
            let pm = if n == 1 { 1.0 } else { p0 };
            dp = n as f64 * (z * pn - pm) / (z * z - 1.0);
            let dz = pn / dp;
            z -= dz;
            if dz.abs() < 1e-16 {
                break;
            }
        }
        x[n - 1 - i] = z;
        w[n - 1 - i] = 2.0 / ((1.0 - z * z) * dp * dp);
    }
    (x, w)
}

/// 瞳のガウス求積（[`PupilGrid::Gauss`]）→ px, py, 重み（和 1）
pub fn pupil_gauss(rings: usize, arms: Option<usize>) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let s = PupilGrid::Gauss { rings, arms }.samples();
    (s.px, s.py, s.wt)
}

/// OPD [波] の結果（光線と一緒に）
pub struct Opd {
    /// 光線の列（位置・方向・光路長・理想レンズの位相と状態・けられ、[`Cols::WAVE`]）
    pub rays: RayBuf,
    pub opd: Vec<f64>,
}

impl Opd {
    pub fn ok(&self, i: usize) -> bool {
        self.rays.ok(i)
    }
}

/// アフォーカル像空間の基準の平面の点: 射出瞳の位置で主光線に垂直な平面（OpticStudio の平面波基準）。
/// 射出瞳が無限遠なら像面の主光線の点で（平面波なので面の位置は定数の違いだけ）
pub(crate) fn to_ref_plane_point(expp: f64, pc: V3, kc: V3) -> V3 {
    if crate::pupil::exit_pupil_near(expp) {
        add(pc, scale(kc, (expp - pc[2]) / kc[2]))
    } else {
        pc
    }
}

#[doc(hidden)]
pub fn to_ref(s: &System, w: usize, pc: V3, kc: V3, q: V3, k: V3, opl: f64) -> f64 {
    crate::pupil::RefSurface::of(s, pc, kc).opl(s.image_index(w), q, k, opl)
}

/// 視野の光線 (px, py) の OPD [波]。ref_primary: 参照球の中心を主波長の主光線に（OpticStudio の光線の OPD）、
/// false なら波長 w 自身の主光線に（Zernike、波面マップ）
pub fn opd(
    s: &System,
    field: FieldSel,
    px: &[f64],
    py: &[f64],
    w: Option<usize>,
    ref_primary: bool,
) -> Result<Opd> {
    Ok(opd_chief(s, field, px, py, w, ref_primary)?.0)
}

/// [`opd`] と、参照球の中心に使った主光線の追跡（ref_primary なら主波長、でなければ波長 w）
pub(crate) fn opd_chief(
    s: &System,
    field: FieldSel,
    px: &[f64],
    py: &[f64],
    w: Option<usize>,
    ref_primary: bool,
) -> Result<(Opd, Trace)> {
    crate::pupil::check_opd_reference(s)?;
    let w = w.unwrap_or(s.primary);
    let mut r = RayBuf::with_cols(Cols::WAVE);
    s.try_trace_into(field, px, py, Some(w), None, &mut r)?;
    let rc = s.try_trace(field, &[0.0], &[0.0], Some(w), None, false)?;
    let rp = if w == s.primary || !ref_primary {
        rc.clone()
    } else {
        s.try_trace(field, &[0.0], &[0.0], Some(s.primary), None, false)?
    };
    for (t, wi) in [(&rc, w), (&rp, s.primary)] {
        let c = &t.rays[0];
        if let Some(surf) = c.status.surf() {
            let why = c.status.why();
            invalid!(
                "the chief ray (wavelength {}) doesn't reach the image (surface {}, {why}); no OPD reference",
                wi + 1,
                surf
            );
        }
    }
    let (pc, kc) = (rp.rays[0].xyz, rp.rays[0].lmn);
    let c = &rc.rays[0];
    let lc = to_ref(s, w, pc, kc, c.xyz, c.lmn, c.opl) + c.lens_opd;
    let wl = s.wave(w).mm();
    let opd = (0..r.len())
        .map(|i| (lc - (to_ref(s, w, pc, kc, r.xyz[i], r.lmn[i], r.opl[i]) + r.lens_opd[i])) / wl)
        .collect();
    Ok((Opd { rays: r, opd }, rp))
}

/// 波面マップの格子 p_i = (i − n/2)/(n/2 − 1)（[`PupilGrid::WavefrontMap`]、OpticStudio の Wavefront Map と同じ）
pub fn wavefront_grid(n: usize) -> Vec<f64> {
    PupilGrid::WavefrontMap(n).axis().unwrap_or_default()
}

pub struct Wavefront {
    /// 波面 [波]（行優先、行 = y。座標は正規化瞳、x・y 共通の [`wavefront_grid`]。瞳の外・けられは NaN）
    pub w: crate::grid::Grid2,
    pub pv: f64,
    pub rms: f64,
}

pub fn wavefront(
    s: &System,
    field: FieldSel,
    w: Option<usize>,
    n: usize,
    ref_primary: bool,
) -> Result<Wavefront> {
    let grid = PupilGrid::WavefrontMap(n);
    let r = PupilFunction::trace(s, field, w, grid, ref_primary)?;
    let g = wavefront_grid(n);
    let ww = r.map();
    let v: Vec<f64> = (0..r.len()).filter_map(|q| r.get(q)).collect();
    if v.is_empty() {
        invalid!("no valid rays (all vignetted)");
    }
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    let rms = (v.iter().map(|a| (a - mean).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
    let pv =
        v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min);
    Ok(Wavefront {
        w: crate::grid::Grid2::from_rows(g.clone(), g, ww),
        pv,
        rms,
    })
}

/// 像空間の方向余弦 (L, M) の、正規化瞳 (px, py) に対する傾き J = ∂(L, M)/∂(px, py)（2×2、行が L・M）。
/// 瞳の縁の光線の差で見積もる。これらは基準の光線なので開口・遮蔽は無視する（OpticStudio と同じ）。
/// 縁の光線が像面まで届かない（面を外れるなど）視野では、軸ごとに内側の点（±0.75, ±0.5, ±0.25）の
/// 差で、それも届かないなら主光線との片側の差で見積もる
#[doc(hidden)]
pub fn pupil_jacobian(s: &System, field: FieldSel, w: usize) -> Result<[[f64; 2]; 2]> {
    if s.afocal {
        return afocal_jacobian(s, field, w);
    }
    let r = s.try_trace(
        field,
        &[1.0, -1.0, 0.0, 0.0],
        &[0.0, 0.0, 1.0, -1.0],
        Some(w),
        None,
        false,
    )?;
    let jm = if (0..4).all(|i| r.traced(i)) {
        let d = |i: usize, j: usize, c: usize| 0.5 * (r.rays[i].lmn[c] - r.rays[j].lmn[c]);
        [[d(0, 1, 0), d(2, 3, 0)], [d(0, 1, 1), d(2, 3, 1)]]
    } else {
        pupil_jacobian_inner(s, field, w)?
    };
    if !(jm[0][0].abs() > crate::tol::JACOBIAN_MIN && jm[1][1].abs() > crate::tol::JACOBIAN_MIN) {
        notimpl!("MTF: the image space aperture is 0 (afocal?)");
    }
    Ok(jm)
}

/// アフォーカル像空間の [`pupil_jacobian`]: 像の座標は角度 θ [単位]、ビームの幅 X [mm] とすると
/// 位相は 2π X θ u / λ（u = 1 単位のラジアン）なので、−∂X/∂p · u を ∂L/∂p の代わりにする。X は OPD の基準と同じ
/// 射出瞳の位置の主光線に垂直な平面での位置。符号は像面の前に焦点距離 F の理想レンズを置いた
/// 系（L′ = L − X/F、x = F θ）と同じ向き。けられて縁の光線が通らないときは内側の点で差をとる
fn afocal_jacobian(s: &System, field: FieldSel, w: usize) -> Result<[[f64; 2]; 2]> {
    const A: [f64; 4] = [1.0, 0.75, 0.5, 0.25];
    let (mut px, mut py) = (vec![0.0], vec![0.0]);
    for axis in 0..2 {
        for a in A {
            for v in [a, -a] {
                let (x, y) = if axis == 0 { (v, 0.0) } else { (0.0, v) };
                px.push(x);
                py.push(y);
            }
        }
    }
    let r = s.try_trace(field, &px, &py, Some(w), None, false)?;
    if !r.traced(0) {
        invalid!(code = crate::error::code::PUPIL_VIGNETTED; "MTF: the chief ray doesn't reach the image (the pupil size can't be set)");
    }
    let (q0, k0) = (r.rays[0].xyz, r.rays[0].lmn);
    // 射出瞳の位置で主光線に垂直な平面（OPD の基準の平面、[`to_ref`]）まで光線を延ばした点の、主光線からの差。
    // 射出瞳が無限遠なら像面の主光線の点を通る平面
    let expp = s.exit_pupil();
    let pe = if crate::pupil::exit_pupil_near(expp) {
        add(q0, scale(k0, (expp - q0[2]) / k0[2]))
    } else {
        q0
    };
    // 平面の向き: e2 = ŷ から主光線の成分を除いたもの、e1 = e2 × k0（主光線が y・z 面で傾くとき、e2 の向きの
    // 1 rad が atan(m/n) の 1 rad）
    let e2 = {
        let v = sub([0.0, 1.0, 0.0], scale(k0, k0[1]));
        scale(v, 1.0 / dot(v, v).sqrt())
    };
    let e1 = [
        e2[1] * k0[2] - e2[2] * k0[1],
        e2[2] * k0[0] - e2[0] * k0[2],
        e2[0] * k0[1] - e2[1] * k0[0],
    ];
    let perp = |i: usize| {
        let (q, k) = (r.rays[i].xyz, r.rays[i].lmn);
        let x = add(q, scale(k, dot(sub(pe, q), k0) / dot(k, k0)));
        let d = sub(x, pe);
        [dot(d, e1), dot(d, e2)]
    };
    let u = s.afocal_unit.radians();
    let mut col = [[0.0; 2]; 2];
    for (axis, c) in col.iter_mut().enumerate() {
        let k = |ia: usize, neg: bool| 1 + axis * 2 * A.len() + 2 * ia + neg as usize;
        let Some(ia) = (0..A.len()).find(|&ia| r.traced(k(ia, false)) && r.traced(k(ia, true)))
        else {
            invalid!(code = crate::error::code::PUPIL_VIGNETTED; "MTF: rays on the pupil axes are vignetted (the pupil size can't be set)");
        };
        let (p, m) = (perp(k(ia, false)), perp(k(ia, true)));
        let h = 2.0 * A[ia];
        *c = [-(p[0] - m[0]) / h * u, -(p[1] - m[1]) / h * u];
    }
    let jm = [[col[0][0], col[1][0]], [col[0][1], col[1][1]]];
    if !(jm[0][0].abs() > crate::tol::JACOBIAN_MIN && jm[1][1].abs() > crate::tol::JACOBIAN_MIN) {
        notimpl!("MTF: the beam width of the afocal image space is 0");
    }
    Ok(jm)
}

/// [`pupil_jacobian`] のフォールバック: 瞳の縁が届かないとき、軸ごとに届く内側の点で差をとる
fn pupil_jacobian_inner(s: &System, field: FieldSel, w: usize) -> Result<[[f64; 2]; 2]> {
    const A: [f64; 3] = [0.75, 0.5, 0.25];
    // 主光線、x の軸の ±a、y の軸の ±a の順
    let (mut px, mut py) = (vec![0.0], vec![0.0]);
    for axis in 0..2 {
        for a in A {
            for v in [a, -a] {
                let (x, y) = if axis == 0 { (v, 0.0) } else { (0.0, v) };
                px.push(x);
                py.push(y);
            }
        }
    }
    let r = s.try_trace(field, &px, &py, Some(w), None, false)?;
    let lm = |i: usize| [r.rays[i].lmn[0], r.rays[i].lmn[1]];
    let mut col = [[0.0; 2]; 2];
    for (axis, c) in col.iter_mut().enumerate() {
        let k = |ia: usize, neg: bool| 1 + axis * 2 * A.len() + 2 * ia + neg as usize;
        let sym = (0..A.len()).find(|&ia| r.traced(k(ia, false)) && r.traced(k(ia, true)));
        let d = if let Some(ia) = sym {
            let (p, m) = (lm(k(ia, false)), lm(k(ia, true)));
            let h = 2.0 * A[ia];
            [(p[0] - m[0]) / h, (p[1] - m[1]) / h]
        } else {
            if !r.traced(0) {
                invalid!(code = crate::error::code::PUPIL_VIGNETTED; "MTF: the chief ray doesn't reach the image (the pupil size can't be set)");
            }
            let o = lm(0);
            let one = (0..A.len())
                .flat_map(|ia| [(ia, false), (ia, true)])
                .find(|&(ia, neg)| r.traced(k(ia, neg)));
            let Some((ia, neg)) = one else {
                invalid!(code = crate::error::code::PUPIL_VIGNETTED; "MTF: rays inside the pupil are vignetted too (the pupil size can't be set)");
            };
            let p = lm(k(ia, neg));
            let h = if neg { -A[ia] } else { A[ia] };
            [(p[0] - o[0]) / h, (p[1] - o[1]) / h]
        };
        *c = d;
    }
    // col[axis] = ∂(L, M)/∂p_axis → 行が L・M
    Ok([[col[0][0], col[1][0]], [col[0][1], col[1][1]]])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gauss_legendre_integrates() {
        let (x, w) = gauss_legendre(5);
        let i: f64 = x.iter().zip(&w).map(|(x, w)| x.powi(8) * w).sum();
        assert!((i - 2.0 / 9.0).abs() < 1e-14);
    }
    fn radial(n: usize, m: usize, r: f64) -> f64 {
        (0..=(n - m) / 2)
            .map(|k| {
                let sgn = if k % 2 == 0 { 1.0 } else { -1.0 };
                sgn * fact(n - k) / (fact(k) * fact((n + m) / 2 - k) * fact((n - m) / 2 - k))
                    * r.powi((n - 2 * k) as i32)
            })
            .sum()
    }

    #[test]
    fn zernike_terms_match_the_definition() {
        // 漸化式・前計算の係数による項が、階乗と atan2 による定義どおりの値と一致する
        for kind in [ZernKind::Standard, ZernKind::Fringe] {
            let nm = zern_nm(kind, if kind == ZernKind::Fringe { 37 } else { 120 });
            for &(px, py) in &[
                (0.0, 0.0),
                (0.3, -0.2),
                (-0.7, 0.7),
                (1.0, 0.0),
                (0.05, 0.99),
            ] {
                let z = zernike_row(px, py, kind, &nm);
                let (r, a) = (f64::hypot(px, py), f64::atan2(py, px));
                for (k, &(n, m, c)) in nm.iter().enumerate() {
                    let ang = if c {
                        (m as f64 * a).cos()
                    } else {
                        (m as f64 * a).sin()
                    };
                    let nrm = if kind == ZernKind::Standard {
                        (((n + 1) * if m > 0 { 2 } else { 1 }) as f64).sqrt()
                    } else {
                        1.0
                    };
                    let want = radial(n, m, r) * ang * nrm;
                    assert!(
                        (z[k] - want).abs() < 1e-9,
                        "{kind:?} {n} {m} {px} {py}: {} {want}",
                        z[k]
                    );
                }
            }
        }
    }

    #[test]
    fn pupil_jacobian_falls_back_when_the_rim_is_vignetted() {
        // 絞りの面に絞りの半径の 0.9 倍の開口を置くと瞳の縁の光線がけられる。内側の点の差で見積もった J は
        // 縁の差の J と瞳の収差の分しか違わず、MTF も計算できる
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let opt = crate::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s = crate::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap();
        for k in 0..s.fields.len() {
            let f = FieldSel::Index(k);
            let j0 = pupil_jacobian(&s, f, s.primary).unwrap();
            // けられないときもフォールバックの見積もりは近い
            let ji = pupil_jacobian_inner(&s, f, s.primary).unwrap();
            let mut v = s.clone();
            let st = v.stop();
            let rmax = 0.9 * v.stop_radius().0.abs();
            v.edit_raw(|p| {
                p.surfaces[st].ap = crate::surface::Aperture::Circular {
                    min: 0.0,
                    max: rmax,
                    dx: 0.0,
                    dy: 0.0,
                };
                Ok(())
            })
            .unwrap();
            let r = v
                .try_trace(
                    f,
                    &[1.0, -1.0, 0.0, 0.0],
                    &[0.0, 0.0, 1.0, -1.0],
                    Some(v.primary),
                    None,
                    false,
                )
                .unwrap();
            assert!((0..4).any(|i| !r.ok(i)), "field {k}: 縁がけられていない");
            let j1 = pupil_jacobian(&v, f, v.primary).unwrap();
            for (a, b) in [(j0, j1), (j0, ji)] {
                for r in 0..2 {
                    for c in 0..2 {
                        let tol = 0.02 * j0[r][r].abs().max(j0[c][c].abs());
                        assert!((a[r][c] - b[r][c]).abs() < tol, "field {k}: {a:?} {b:?}");
                    }
                }
            }
            assert!(
                mtf(&v, f, Waves::One(v.primary), Some(16), &[0.0, 20.0]).is_ok(),
                "field {k}"
            );
        }
    }

    #[test]
    fn column_basis_and_gram_match_the_row_forms() {
        // eval_cols は eval とビットまで同じ。AᵀA は AVX の有無・列の組の大きさ（gram_block と gram_tiles）で同じで、
        // 1 点ずつの外積の和と丸めの範囲で一致する
        let mut st = 7u64;
        let mut rnd = || {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            (st >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
        };
        for (kind, nt) in [
            (ZernKind::Fringe, 37),
            (ZernKind::Standard, 45),
            (ZernKind::Standard, 7),
        ] {
            let nm = zern_nm(kind, nt);
            let b = ZernBasis::new(kind, &nm);
            let len: usize = 203;
            let (px, py): (Vec<f64>, Vec<f64>) =
                (0..len).map(|_| (rnd() * 0.7, rnd() * 0.7)).unzip();
            let ld = len.div_ceil(4) * 4;
            let np = nt.div_ceil(4) * 4;
            let mut a = vec![0.0; ld * np];
            b.eval_cols(&px, &py, &mut a, ld);
            let mut row = vec![0.0; nt];
            let mut want = vec![0.0; nt * nt];
            for i in 0..len {
                b.eval(px[i], py[i], &mut row, 1);
                for j in 0..nt {
                    assert_eq!(
                        a[j * ld + i].to_bits(),
                        row[j].to_bits(),
                        "{kind:?} {i} {j}"
                    );
                    for k in 0..=j {
                        want[j * nt + k] += row[j] * row[k];
                    }
                }
            }
            let (mut g1, mut g2) = (vec![0.0; nt * nt], vec![0.0; nt * nt]);
            gram_block(&a, ld, np, nt, &mut g1);
            gram_tiles::<2, 2>(&a, ld, np, nt, &mut g2);
            for j in 0..nt {
                for k in 0..=j {
                    let (x, y, w) = (g1[j * nt + k], g2[j * nt + k], want[j * nt + k]);
                    assert_eq!(x.to_bits(), y.to_bits(), "{kind:?} {j} {k}");
                    assert!(
                        (x - w).abs() <= 1e-13 * (1.0 + w.abs()),
                        "{kind:?} {j} {k}: {x} {w}"
                    );
                }
            }
        }
    }

    #[test]
    fn zernike_fit_is_the_same_for_any_thread_count() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let opt = crate::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s = crate::zmx::load_with(&root.join("tests/refs/C0/system.zmx"), &opt).unwrap();
        let f = FieldSel::Index(1);
        let run = |t| {
            crate::with_threads(Some(t), || {
                zernike(&s, f, None, ZernKind::Fringe, 37, 96, false).unwrap()
            })
        };
        let (a, b) = (run(1), run(3));
        for (x, y) in a.coef.iter().zip(&b.coef) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
        assert_eq!(a.rms_fit.to_bits(), b.rms_fit.to_bits());
    }

    #[test]
    fn many_terms_on_a_coarse_grid_fall_back_to_qr() {
        // 格子 32・Standard 496 項（次数 30）は正規方程式では解けない（条件数が大きい）ので QR で解く。
        // 行列を zernike_row で作って lstsq で解いた値と比べる
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let opt = crate::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s = crate::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap();
        let f = FieldSel::Index(1);
        let (nt, n) = (496, 32);
        let z = zernike(&s, f, None, ZernKind::Standard, nt, n, false).unwrap();
        let g = wavefront_grid(n);
        let (mut px, mut py) = (Vec::new(), Vec::new());
        for &y in &g {
            for &x in &g {
                if x * x + y * y <= 1.0 {
                    px.push(x);
                    py.push(y);
                }
            }
        }
        let o = opd(&s, f, &px, &py, None, false).unwrap();
        let nm = zern_nm(ZernKind::Standard, nt);
        let (mut a, mut v) = (Vec::new(), Vec::new());
        for i in 0..px.len() {
            if o.ok(i) && o.opd[i].is_finite() {
                a.push(zernike_row(px[i], py[i], ZernKind::Standard, &nm));
                v.push(o.opd[i]);
            }
        }
        let c = crate::linalg::lstsq(&a, &v);
        let e = c
            .iter()
            .zip(&z.coef)
            .map(|(p, q)| (p - q).abs())
            .fold(0.0, f64::max);
        assert!(e < 1e-9, "{e}");
    }

    #[test]
    fn lstsq_exact() {
        let a = vec![vec![1.0, 0.0], vec![1.0, 1.0], vec![1.0, 2.0]];
        let x = crate::linalg::lstsq(&a, &[1.0, 3.0, 5.0]);
        assert!((x[0] - 1.0).abs() < 1e-14 && (x[1] - 2.0).abs() < 1e-14);
    }

    #[test]
    fn a_point_at_the_front_focus_leaves_a_plane_wave() {
        // 物体点が理想レンズの前側焦点: 像空間は平行光（像は無限遠）。OPD は 0（以前は理想レンズの位相が NaN）
        let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 2 1 0 0 0\n\
                   XFLN 0 0\nYFLN 0 2\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\n\
                   SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 100\n\
                   SURF 1\n  STOP\n  TYPE PARAXIAL\n  CURV 0.0\n  PARM 1 100\n  PARM 2 1\n  DISZ 50\n\
                   SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
        let raw = crate::zmx::parse_text(txt).unwrap();
        let s = crate::zmx::build(&raw, &Default::default(), &[], None).unwrap();
        let (px, py) = pupil_grid(9);
        for f in 0..2 {
            let t = s
                .try_trace(FieldSel::Index(f), &px, &py, None, None, false)
                .unwrap();
            let l: Vec<f64> = (0..px.len())
                .filter(|&i| t.ok(i))
                .map(|i| {
                    // 平行光の波面（主光線に垂直な面）までの光路
                    let k = t.rays[0].lmn;
                    let d = crate::math::sub(t.rays[i].xyz, t.rays[0].xyz);
                    t.opl_os[i] + t.rays[i].lens_opd - crate::math::dot(d, k)
                })
                .collect();
            let (lo, hi) = l
                .iter()
                .fold((f64::MAX, f64::MIN), |a, &v| (a.0.min(v), a.1.max(v)));
            assert!(hi - lo < 1e-9, "視野 {f}: {lo} {hi}");
        }
    }
}
