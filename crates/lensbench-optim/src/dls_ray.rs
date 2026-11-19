//! 光線の項（[`TermKind::Ray`]）の、面ごとの記録から求める量: 大域の位置・方向、面の法線、入射角・射出角など

use super::*;
use lensbench_core::system::{FieldType, Trace};

/// 光線 t（面 1..=j を記録したもの）の、面 j での量 q
pub(super) fn extra(s: &System, t: &Trace, j: usize, q: RayQ) -> Result<f64> {
    let frames = s.frames();
    let fr = |k: usize| -> Result<_> {
        match frames.surf[k] {
            Some(f) => Ok(f),
            None => invalid!("surface {k} has no position (ignored)"),
        }
    };
    let Some(at) = t.rec.iter().position(|r| r.surf == j) else {
        invalid!("surface {j} is ignored");
    };
    let e = t.rec[at].entry(0);
    let f = fr(j)?;
    // 面の後の方向（大域）と、面の前の方向（面 j の局所座標）
    let out = mv(&f.r, e.lmn);
    let before = if at == 0 {
        t.k0[0]
    } else {
        let p = t.rec[at - 1].entry(0);
        mv(&fr(p.surf)?.r, p.lmn)
    };
    let din = normalize(mtv(&f.r, before));
    let (_, zx, zy) = s.surfaces[j].shape().sag(e.xyz[0], e.xyz[1]);
    // 法線は −z 寄り（OpticStudio の RENA〜RENC の向き）
    let nrm = normalize([zx, zy, -1.0]);
    // 小さい角でも精度が落ちないよう atan2(|d × n|, |d · n|)
    let angle = |d: V3| norm(cross(d, nrm)).atan2(dot(d, nrm).abs());
    let v = match q {
        RayQ::Gx => e.p[0],
        RayQ::Gy => e.p[1],
        RayQ::Gz => e.p[2],
        RayQ::Gl => out[0],
        RayQ::Gm => out[1],
        RayQ::Gn => out[2],
        RayQ::R => e.xyz[0].hypot(e.xyz[1]),
        RayQ::Ang => e.lmn[0].hypot(e.lmn[1]).atan2(e.lmn[2].abs()),
        RayQ::TanX => e.lmn[0] / e.lmn[2],
        RayQ::TanY => e.lmn[1] / e.lmn[2],
        RayQ::Nl => nrm[0],
        RayQ::Nm => nrm[1],
        RayQ::Nn => nrm[2],
        RayQ::Aoi => angle(din).to_degrees(),
        RayQ::Aoe => angle(e.lmn).to_degrees(),
        RayQ::CosI => dot(din, nrm).abs(),
        RayQ::CosE => dot(e.lmn, nrm).abs(),
        _ => unreachable!("not a recorded ray quantity"),
    };
    if !v.is_finite() {
        invalid!("{} of the ray at surface {j} is not finite", q.name());
    }
    Ok(v)
}

/// 光線の量の項
#[allow(clippy::too_many_arguments)]
pub(super) fn ray_term(
    s: &System,
    field: FieldSel,
    wave: Option<usize>,
    px: f64,
    py: f64,
    surf: Option<usize>,
    q: RayQ,
    target: f64,
    ps: Option<&[Param]>,
) -> Result<TermOut> {
    let w = wave.unwrap_or(s.primary);
    let j = surf.unwrap_or(s.image());
    if matches!(q, RayQ::FcgT | RayQ::FcgS) {
        return Ok((vec![field_curvature(s, field, w, j, q)? - target], None));
    }
    if matches!(q, RayQ::DxDx | RayQ::DxDy | RayQ::DyDx | RayQ::DyDy) {
        return Ok((vec![pupil_slope(s, field, w, px, py, j, q)? - target], None));
    }
    let rec = ps.is_some() || q.recorded();
    let t = s.try_trace(field, &[px], &[py], Some(w), Some(j), rec)?;
    let x = &t.rays[0];
    if x.status.surf().is_some() {
        invalid!(
            "the ray stopped (wavelength {w}, pupil ({px:.3}, {py:.3}), surface {})",
            x.status.code()
        );
    }
    // TRAX・TRAY・TRAR・ANAX・ANAY・ANAR は主波長の主光線との差
    let tra = matches!(
        q,
        RayQ::TraX | RayQ::TraY | RayQ::TraR | RayQ::AnaX | RayQ::AnaY | RayQ::AnaR
    );
    let tc = if tra {
        let tc = s.try_trace(
            field,
            &[0.0],
            &[0.0],
            Some(s.primary),
            Some(j),
            ps.is_some(),
        )?;
        if tc.rays[0].status.surf().is_some() {
            invalid!(
                "the chief ray stopped (surface {})",
                tc.rays[0].status.code()
            );
        }
        Some(tc)
    } else {
        None
    };
    let (dx, dy) = match &tc {
        Some(tc) => (x.xyz[0] - tc.rays[0].xyz[0], x.xyz[1] - tc.rays[0].xyz[1]),
        None => (0.0, 0.0),
    };
    let r = dx.hypot(dy);
    let v = match q {
        RayQ::X => x.xyz[0],
        RayQ::Y => x.xyz[1],
        RayQ::Z => x.xyz[2],
        RayQ::L => x.lmn[0],
        RayQ::M => x.lmn[1],
        RayQ::N => x.lmn[2],
        RayQ::TraX => dx,
        RayQ::TraY => dy,
        RayQ::TraR => r,
        RayQ::AnaX | RayQ::AnaY | RayQ::AnaR => {
            let c = &tc.as_ref().expect("the chief ray is traced").rays[0];
            let (dl, dm) = (x.lmn[0] - c.lmn[0], x.lmn[1] - c.lmn[1]);
            match q {
                RayQ::AnaX => dl,
                RayQ::AnaY => dm,
                _ => dl.hypot(dm),
            }
        }
        RayQ::Opth if s.infinite() => {
            // 平行な光線は光線に垂直な同じ面から出るので、主光線の面 1 までの光路長を引けば OpticStudio の基準になる
            let c = s.try_trace(field, &[0.0], &[0.0], Some(w), Some(1), false)?;
            if c.rays[0].status.surf().is_some() {
                invalid!(
                    "the chief ray stopped (surface {})",
                    c.rays[0].status.code()
                );
            }
            x.opl - c.rays[0].opl
        }
        RayQ::Opth => x.opl,
        q => extra(s, &t, j, q)?,
    };
    let jac = match ps {
        Some(ps) if !q.by_diff() => {
            let g = s.land_grad(&t, w, ps, None)?;
            let gc = match &tc {
                Some(tc) => s.land_grad(tc, s.primary, ps, None)?,
                None => vec![0.0; g.len()],
            };
            let d: Vec<f64> = (0..ps.len())
                .map(|k| {
                    let g = &g[k * 4..k * 4 + 4];
                    let (gx, gy) = (g[0] - gc[k * 4], g[1] - gc[k * 4 + 1]);
                    match q {
                        RayQ::X => g[0],
                        RayQ::Y => g[1],
                        RayQ::L => g[2],
                        RayQ::M => g[3],
                        RayQ::TraX => gx,
                        RayQ::TraY => gy,
                        // 長さ 0 では向きが決まらないので 0
                        RayQ::TraR if r > 0.0 => (dx * gx + dy * gy) / r,
                        RayQ::TraR => 0.0,
                        // l² + m² + n² = 1
                        _ => -(x.lmn[0] * g[2] + x.lmn[1] * g[3]) / x.lmn[2],
                    }
                })
                .collect();
            if d.iter().any(|v| !v.is_finite()) {
                invalid!("can't find the ray derivatives");
            }
            Some(d)
        }
        Some(_) => invalid!("{} of a ray is by differences", q.name()),
        None => None,
    };
    Ok((vec![v - target], jac))
}

/// 面 j での x・y の、正規化瞳座標での微分（中心差分。DXDX・DXDY・DYDX・DYDY）
fn pupil_slope(
    s: &System,
    field: FieldSel,
    w: usize,
    px: f64,
    py: f64,
    j: usize,
    q: RayQ,
) -> Result<f64> {
    // OpticStudio 26.1 と同じ刻み（中心差分、刻み 1e-3 で値が 1e-11 まで合う）
    const H: f64 = 1e-3;
    let (ax, by_x) = match q {
        RayQ::DxDx => (0, true),
        RayQ::DxDy => (0, false),
        RayQ::DyDx => (1, true),
        _ => (1, false),
    };
    let (dx, dy) = if by_x { (H, 0.0) } else { (0.0, H) };
    let t = s.try_trace(
        field,
        &[px - dx, px + dx],
        &[py - dy, py + dy],
        Some(w),
        Some(j),
        false,
    )?;
    if let Some(r) = t.rays.iter().find(|r| r.status.surf().is_some()) {
        invalid!(
            "the ray stopped (wavelength {w}, pupil ({px:.3}, {py:.3}), surface {})",
            r.status.code()
        );
    }
    Ok((t.rays[1].xyz[ax] - t.rays[0].xyz[ax]) / (2.0 * H))
}

/// 主光線の近くの光線の焦点の、面 j からの z の距離（[`RayQ::FcgT`]・[`RayQ::FcgS`]）。正規化瞳座標を y（FcgT）か
/// x（FcgS）の向きに ±0.001 ずらした 2 本の光線が最も近づく z（横の位置の差 Δp と傾きの差 Δs から −Δp·Δs / |Δs|²。
/// 回転対称でない系でも決まる）。OpticStudio 26.1 は主光線と +0.001 の光線の片側の差分で求め、ずらす向きが視野の向きと
/// 垂直なら中心差分と 1e-10 で合い、そうでなければ 1e-4 ほど違う（刻みを 0 に近づけた値は中心差分の側）
fn field_curvature(s: &System, field: FieldSel, w: usize, j: usize, q: RayQ) -> Result<f64> {
    const H: f64 = 1e-3;
    let (px, py) = if q == RayQ::FcgT {
        ([0.0, 0.0], [-H, H])
    } else {
        ([-H, H], [0.0, 0.0])
    };
    let t = s.try_trace(field, &px, &py, Some(w), Some(j), false)?;
    if let Some(r) = t.rays.iter().find(|r| r.status.surf().is_some()) {
        invalid!(
            "the ray near the chief ray stopped (wavelength {w}, surface {})",
            r.status.code()
        );
    }
    let (a, b) = (&t.rays[0], &t.rays[1]);
    let dp = [b.xyz[0] - a.xyz[0], b.xyz[1] - a.xyz[1]];
    let ds = [
        b.lmn[0] / b.lmn[2] - a.lmn[0] / a.lmn[2],
        b.lmn[1] / b.lmn[2] - a.lmn[1] / a.lmn[2],
    ];
    let z = -(dp[0] * ds[0] + dp[1] * ds[1]) / (ds[0] * ds[0] + ds[1] * ds[1]);
    if !z.is_finite() {
        invalid!("the field curvature is not finite (wavelength {w})");
    }
    Ok(z)
}

pub(super) fn check_path(
    i: usize,
    s: &System,
    surf: usize,
    to: usize,
    q: RayQ,
    upper: Option<bool>,
) -> Result<()> {
    // 光路長は物体面から測れる（OPTH と同じ基準）
    let first = usize::from(q != RayQ::Opth);
    if surf < first || surf > to || to > s.image() {
        invalid!(
            "term {i}: surfaces {surf} to {to} ({first} to {})",
            s.image()
        );
    }
    match (q, upper) {
        (RayQ::Opth, None) | (RayQ::Aoi | RayQ::Aoe, Some(_)) => Ok(()),
        _ => invalid!("term {i}: a ray term with to is opth, or aoi or aoe with min or max"),
    }
}

/// [`TermKind::RayPath`] の値
pub(super) fn path_term(s: &System, k: &TermKind) -> Result<TermOut> {
    let &TermKind::RayPath {
        field,
        wave,
        px,
        py,
        surf,
        to,
        q,
        upper,
        target,
    } = k
    else {
        unreachable!("not a ray path term");
    };
    let w = wave.unwrap_or(s.primary);
    let t = s.try_trace(field, &[px], &[py], Some(w), Some(to), true)?;
    if t.rays[0].status.surf().is_some() {
        invalid!(
            "the ray stopped (wavelength {w}, pupil ({px:.3}, {py:.3}), surface {})",
            t.rays[0].status.code()
        );
    }
    let opl = |j: usize| t.rec.iter().find(|r| r.surf == j).map(|r| r.opl[0]);
    let v = match upper {
        None => {
            // 物体面からは OPTH と同じ: 無限遠物体では主光線が面 1 を通る点を通り光線に垂直な面から
            let a = match surf {
                0 if s.infinite() => {
                    let c = s.try_trace(field, &[0.0], &[0.0], Some(w), Some(1), false)?;
                    if c.rays[0].status.surf().is_some() {
                        invalid!(
                            "the chief ray stopped (surface {})",
                            c.rays[0].status.code()
                        );
                    }
                    Some(c.rays[0].opl)
                }
                0 => Some(0.0),
                j => opl(j),
            };
            match (a, opl(to)) {
                (Some(a), Some(b)) => b - a,
                _ => invalid!("surface {surf} or {to} is ignored"),
            }
        }
        Some(up) => {
            let mut v = target;
            for r in t.rec.iter().filter(|r| (surf..=to).contains(&r.surf)) {
                let d = extra(s, &t, r.surf, q)? - target;
                v += if up { d.max(0.0) } else { d.min(0.0) };
            }
            v
        }
    };
    if !v.is_finite() {
        invalid!(
            "{} of the ray from surface {surf} to {to} is not finite",
            q.name()
        );
    }
    Ok((vec![v - target], None))
}

/// [`TermKind::ParaxRay`] の値: 近軸光線（x・y を別々に近軸で追う）の面 surf での量。位置は頂点面、方向は傾き
/// (u_x, u_y, 1) を規格化したもの、法線は頂点面の位置での面の法線（OpticStudio の PARX〜PATY）
pub(super) fn parax_term(s: &System, k: &TermKind) -> Result<TermOut> {
    let &TermKind::ParaxRay {
        field,
        wave,
        px,
        py,
        surf,
        q,
        target,
    } = k
    else {
        unreachable!("not a paraxial ray term");
    };
    let w = wave.unwrap_or(s.primary);
    let (hx, hy) = match field {
        FieldSel::Index(i) if i < s.fields.len() => s.hxhy(i),
        FieldSel::Index(i) => invalid!("field number {} ({} in all)", i + 1, s.fields.len()),
        FieldSel::Norm(a, b) => (a, b),
    };
    let (nx, ny) = s.field_norm_values();
    let p = s.paraxial();
    let t0 = s.surfaces[0].t;
    // 物体空間の傾きと面 1 の頂点面での高さ（入射瞳の点 (px, py) · EPD/2 を通る）
    let launch = |h: f64, pp: f64| -> Result<(f64, f64)> {
        let e = pp * p.epd / 2.0;
        Ok(match s.field_type {
            FieldType::Angle if s.infinite() => {
                let u = h.to_radians().tan();
                (e - u * p.enpp, u)
            }
            FieldType::ObjH if !s.infinite() => {
                let u = (e - h) / (p.enpp + t0);
                (h + u * t0, u)
            }
            _ => invalid!(
                "a paraxial ray needs an angle field (object at infinity) or an object height field"
            ),
        })
    };
    let (x1, ux0) = launch(hx * nx, px)?;
    let (y1, uy0) = launch(hy * ny, py)?;
    let (xs, uxs) = s.ptrace(x1, ux0, w, Some(surf));
    let (ys, uys) = s.ptrace(y1, uy0, w, Some(surf));
    let (x, y, ux, uy) = (xs[surf], ys[surf], uxs[surf], uys[surf]);
    let d = normalize([ux, uy, 1.0]);
    let (_, zx, zy) = s.surfaces[surf].shape().sag(x, y);
    let nrm = normalize([zx, zy, -1.0]);
    let v = match q {
        RayQ::X => x,
        RayQ::Y => y,
        RayQ::Z => 0.0,
        RayQ::L => d[0],
        RayQ::M => d[1],
        RayQ::N => d[2],
        RayQ::R => x.hypot(y),
        RayQ::Nl => nrm[0],
        RayQ::Nm => nrm[1],
        RayQ::Nn => nrm[2],
        RayQ::TanX => ux,
        RayQ::TanY => uy,
        _ => invalid!(
            "paraxial ray quantity {} (x, y, z, l, m, n, r, nl, nm, nn, tanx, tany)",
            q.name()
        ),
    };
    if !v.is_finite() {
        invalid!(
            "{} of the paraxial ray at surface {surf} is not finite",
            q.name()
        );
    }
    Ok((vec![v - target], None))
}

/// 実光線の F ナンバー: 主光線と、瞳の縁 (0, ±1)（sag なら (±1, 0)）の 2 本の光線の像空間での角度 θ± から
/// 1 / (n′ (sin θ₊ + sin θ₋))（OpticStudio 26.1 の TFNO・SFNO と 1e-14 で一致）
pub(super) fn fnum_value(s: &System, field: FieldSel, w: usize, sag: bool) -> Result<f64> {
    let e = [0.0, 1.0, -1.0];
    let z = [0.0; 3];
    let (pxs, pys) = if sag { (&e, &z) } else { (&z, &e) };
    let t = s.try_trace(field, pxs, pys, Some(w), None, false)?;
    for (i, x) in t.rays.iter().enumerate() {
        if x.status.surf().is_some() {
            let (px, py) = (pxs[i], pys[i]);
            invalid!(
                "the ray stopped (wavelength {w}, pupil ({px:.3}, {py:.3}), surface {})",
                x.status.code()
            );
        }
    }
    // 縁の 2 本が主光線の同じ側に来たら 2 本目の正弦を負にして、和を開き角に合わせる
    let k0 = t.rays[0].lmn;
    let a = cross(t.rays[1].lmn, k0);
    let b = cross(k0, t.rays[2].lmn);
    let sb = if dot(a, b) < 0.0 { -norm(b) } else { norm(b) };
    let v = 1.0 / (s.image_index(w).abs() * (norm(a) + sb));
    if !v.is_finite() {
        invalid!("the F-number is not finite (wavelength {w})");
    }
    Ok(v)
}

/// F ナンバーの項
pub(super) fn fnum_term(s: &System, k: &TermKind) -> Result<TermOut> {
    let TermKind::FNum {
        field,
        wave,
        sag,
        target,
    } = k
    else {
        unreachable!("not an F-number term");
    };
    let w = wave.unwrap_or(s.primary);
    Ok((vec![fnum_value(s, *field, w, *sag)? - target], None))
}
