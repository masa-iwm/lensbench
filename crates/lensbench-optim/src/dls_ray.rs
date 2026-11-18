//! 光線の項（[`TermKind::Ray`]）の、面ごとの記録から求める量: 大域の位置・方向、面の法線、入射角・射出角など

use super::*;
use lensbench_core::system::Trace;

/// 光線 t（面 1..=j を記録したもの）の、面 j での量 q
pub(super) fn extra(s: &System, t: &Trace, j: usize, q: RayQ) -> Result<f64> {
    let frames = s.frames();
    let fr = |k: usize| frames.surf[k].expect("traced surfaces have a position");
    let Some(at) = t.rec.iter().position(|r| r.surf == j) else {
        invalid!("surface {j} is ignored");
    };
    let e = t.rec[at].entry(0);
    let f = fr(j);
    // 面の後の方向（大域）と、面の前の方向（面 j の局所座標）
    let out = mv(&f.r, e.lmn);
    let before = if at == 0 {
        t.k0[0]
    } else {
        let p = t.rec[at - 1].entry(0);
        mv(&fr(p.surf).r, p.lmn)
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
    let rec = ps.is_some() || q.recorded();
    let t = s.try_trace(field, &[px], &[py], Some(w), Some(j), rec)?;
    let x = &t.rays[0];
    if x.status.surf().is_some() {
        invalid!(
            "the ray stopped (wavelength {w}, pupil ({px:.3}, {py:.3}), surface {})",
            x.status.code()
        );
    }
    // TRAX・TRAY・TRAR は主波長の主光線との差
    let tra = matches!(q, RayQ::TraX | RayQ::TraY | RayQ::TraR);
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
