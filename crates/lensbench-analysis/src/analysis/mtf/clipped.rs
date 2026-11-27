//! 瞳が開口・遮蔽で切れているときの線ごとの求積

use super::*;

/// 向き dir（0 球欠、1 子午）の線に沿う座標 a・線の位置 v の点の組の座標 (u, v)
#[inline]
fn line_xy(dir: usize, a: f64, v: f64) -> (f64, f64) {
    if dir == 0 {
        (a, v)
    } else {
        (v, a)
    }
}

/// 瞳を向き dir の線（組の座標 (u, v) で、dir 0: v が一定の線で u（球欠）の向き、1: u が一定の線で v（子午）の
/// 向き）で切ったときの、光線が通る区間（線に沿う座標の昇順）。単位円の弦を等間隔の点（fine の線は [`Tuning::seg_scan_fine`]、他は [`Tuning::seg_scan`]）で
/// 走査し、通る・通らないが変わる所を二分法で詰める。走査の間隔より狭い穴・島は見落とす（区間は区間の数が
/// 変わる所でだけ 0 に縮むので、その近くの線は細かく走査する）
pub(super) fn pupil_segments(
    s: &System,
    field: FieldSel,
    wi: usize,
    dir: usize,
    vs: &[f64],
    fine: &[bool],
    fr: &MtfFrame,
) -> Result<Vec<Vec<(f64, f64)>>> {
    let xy = |a: f64, v: f64| line_xy(dir, a, v);
    let valid = |pts: &[(f64, f64)]| -> Result<Vec<bool>> {
        let (px, py): (Vec<f64>, Vec<f64>) = pts.iter().copied().unzip();
        let o = frame_opd(s, field, wi, fr, &px, &py)?;
        Ok((0..pts.len()).map(|k| o[k].is_finite()).collect())
    };
    let m: Vec<usize> = fine
        .iter()
        .map(|&f| if f { T.seg_scan_fine } else { T.seg_scan })
        .collect();
    let mut off = vec![0usize; vs.len() + 1];
    for i in 0..vs.len() {
        off[i + 1] = off[i] + m[i];
    }
    // 弦の端は単位円のわずかに内側（丸めで外に出ない）
    let chord: Vec<f64> = vs
        .iter()
        .map(|&v| (1.0 - v * v).max(0.0).sqrt() * (1.0 - 1e-12))
        .collect();
    let at = |i: usize, j: usize| chord[i] * (-1.0 + 2.0 * j as f64 / (m[i] - 1) as f64);
    let mut pts = Vec::with_capacity(off[vs.len()]);
    for (i, &v) in vs.iter().enumerate() {
        for j in 0..m[i] {
            pts.push(xy(at(i, j), v));
        }
    }
    let ok = valid(&pts)?;
    // 変わり目 (線, 走査の番号, 通る側, 通らない側) を二分法で詰める
    let mut edges: Vec<(usize, usize, f64, f64)> = Vec::new();
    for i in 0..vs.len() {
        for j in 0..m[i] - 1 {
            let (a, b) = (ok[off[i] + j], ok[off[i] + j + 1]);
            if a != b {
                let (lo, hi) = if a {
                    (at(i, j), at(i, j + 1))
                } else {
                    (at(i, j + 1), at(i, j))
                };
                edges.push((i, j, lo, hi));
            }
        }
    }
    for _ in 0..44 {
        if edges.is_empty() {
            break;
        }
        let mid: Vec<(f64, f64)> = edges
            .iter()
            .map(|e| xy(0.5 * (e.2 + e.3), vs[e.0]))
            .collect();
        let g = valid(&mid)?;
        for (e, &good) in edges.iter_mut().zip(&g) {
            let c = 0.5 * (e.2 + e.3);
            if good {
                e.2 = c;
            } else {
                e.3 = c;
            }
        }
    }
    let mut segs = vec![Vec::new(); vs.len()];
    let mut ei = 0;
    for i in 0..vs.len() {
        let mut start = ok[off[i]].then(|| at(i, 0));
        for j in 0..m[i] - 1 {
            if ei < edges.len() && edges[ei].0 == i && edges[ei].1 == j {
                let x = 0.5 * (edges[ei].2 + edges[ei].3);
                match start.take() {
                    Some(a) => segs[i].push((a, x)),
                    None => start = Some(x),
                }
                ei += 1;
            }
        }
        if let Some(a) = start {
            segs[i].push((a, at(i, m[i] - 1)));
        }
    }
    Ok(segs)
}

/// ずらさない瞳の [`lens_quad`]（d = 0、点数は gl）で、けられた点（o0 が false の点）が覆うけられの面積（正規化瞳の
/// 単位、単位円で π）。点の行（t が一定）ごとに、けられた点の続きの両側の縁を隣の点（行の端なら弦の端）との間の
/// 二分法で詰め、縁の間の長さに行の重みを掛けて足す。点の当たり外れではなく縁の位置で測るので、求積点より細い
/// けられ（面の定義域の縁で光線が落ちる帯など）は点に当たってもその幅ほどの面積にしかならない。点が当たらない
/// 行のけられは数えない
pub(super) fn clipped_area(
    s: &System,
    field: FieldSel,
    wi: usize,
    fr: &MtfFrame,
    gl: &LensGl,
    o0: &[bool],
) -> Result<f64> {
    let nu = gl.gu.len();
    // 縁 (行の t, けられた側の u, 通る側の u)、けられの続き (行の重み, 左の縁, 右の縁)
    let mut edges: Vec<(f64, f64, f64)> = Vec::new();
    let mut runs: Vec<(f64, usize, usize)> = Vec::new();
    for (r, row) in o0.chunks(nu).enumerate() {
        if row.iter().all(|&v| v) {
            continue;
        }
        // [`lens_quad`] の d = 0 の行 r
        let (z, wz) = (gl.gt[r / 2], gl.wt[r / 2]);
        let tau = 0.5 * (z + 1.0);
        let y = 1.0 - tau * tau;
        let dy = tau * wz;
        let x = (1.0 - y * y).max(0.0).sqrt();
        let t = if r % 2 == 0 { y } else { -y };
        let end = x * (1.0 - 1e-12);
        let mut i = 0;
        while i < nu {
            if row[i] {
                i += 1;
                continue;
            }
            let mut j = i;
            while j + 1 < nu && !row[j + 1] {
                j += 1;
            }
            let left = if i > 0 { x * gl.gu[i - 1] } else { -end };
            let right = if j + 1 < nu { x * gl.gu[j + 1] } else { end };
            runs.push((dy, edges.len(), edges.len() + 1));
            edges.push((t, x * gl.gu[i], left));
            edges.push((t, x * gl.gu[j], right));
            i = j + 1;
        }
    }
    for _ in 0..44 {
        if edges.is_empty() {
            break;
        }
        let (px, py): (Vec<f64>, Vec<f64>) = edges.iter().map(|e| (0.5 * (e.1 + e.2), e.0)).unzip();
        let o = frame_opd(s, field, wi, fr, &px, &py)?;
        for (k, e) in edges.iter_mut().enumerate() {
            let c = 0.5 * (e.1 + e.2);
            if o[k].is_finite() {
                e.2 = c;
            } else {
                e.1 = c;
            }
        }
    }
    let at = |k: usize| 0.5 * (edges[k].1 + edges[k].2);
    Ok(runs.iter().map(|&(w, a, b)| w * (at(b) - at(a))).sum())
}

/// [`clipped_otf`] の線の位置と重み（向き dir、[`pupil_segments`] と同じ線）。v = sin θ と置いて θ で
/// Gauss-Legendre、全部で約 n 本。光線が通る区間の数が変わる θ（開口の縁が線と平行な所、遮蔽の端など。
/// 線ごとの光量が跳ぶ・折れる）を [`Tuning::line_scan`] 本の走査と二分法で求め、そこで θ の範囲を分ける
pub(super) fn line_nodes(
    s: &System,
    field: FieldSel,
    wi: usize,
    dir: usize,
    n: usize,
    fr: &MtfFrame,
) -> Result<(Vec<f64>, Vec<f64>, Vec<bool>)> {
    use std::f64::consts::{FRAC_PI_2, PI};
    let xy = |a: f64, v: f64| line_xy(dir, a, v);
    // θ の線ごとの、通る区間の数（弦の T.count_scan 点の走査で数える。小さい区間が見える所まで
    // 変わり目がずれないよう、区間を求める走査より細かい）
    let counts = |th: &[f64]| -> Result<Vec<usize>> {
        let (mut px, mut py) = (Vec::new(), Vec::new());
        for &t in th {
            let v = t.sin();
            let c = (1.0 - v * v).max(0.0).sqrt() * (1.0 - 1e-12);
            for j in 0..T.count_scan {
                let (x, y) = xy(c * (-1.0 + 2.0 * j as f64 / (T.count_scan - 1) as f64), v);
                px.push(x);
                py.push(y);
            }
        }
        let o = frame_opd(s, field, wi, fr, &px, &py)?;
        let ok = |k: usize| o[k].is_finite();
        Ok((0..th.len())
            .map(|i| {
                (0..T.count_scan)
                    .filter(|&j| {
                        ok(i * T.count_scan + j) && (j == 0 || !ok(i * T.count_scan + j - 1))
                    })
                    .count()
            })
            .collect())
    };
    let th: Vec<f64> = (0..T.line_scan)
        .map(|j| -FRAC_PI_2 + PI * j as f64 / (T.line_scan - 1) as f64)
        .collect();
    let c = counts(&th)?;
    let mut br: Vec<(f64, f64, usize, usize)> = (0..T.line_scan - 1)
        .filter(|&j| c[j] != c[j + 1])
        .map(|j| (th[j], th[j + 1], c[j], c[j + 1]))
        .collect();
    for _ in 0..24 {
        if br.is_empty() {
            break;
        }
        let mid: Vec<f64> = br.iter().map(|b| 0.5 * (b.0 + b.1)).collect();
        let cm = counts(&mid)?;
        for (b, (&m, &k)) in br.iter_mut().zip(mid.iter().zip(&cm)) {
            // 区間の数が両端のどちらとも違うなら（変わり目が 2 つ）、左の変わり目を追う
            if k == b.2 {
                b.0 = m;
            } else {
                b.1 = m;
                b.3 = k;
            }
        }
    }
    let mut cuts = vec![-FRAC_PI_2];
    cuts.extend(br.iter().map(|b| 0.5 * (b.0 + b.1)));
    cuts.push(FRAC_PI_2);
    let (mut vs, mut wv, mut near) = (Vec::new(), Vec::new(), Vec::new());
    // 変わり目から走査の線 1 本分の間の線は細かく走査する（[`pupil_segments`]）
    let band = PI / (T.line_scan - 1) as f64;
    let inner = &cuts[1..cuts.len() - 1];
    for p in cuts.windows(2) {
        let (a, b) = (p[0], p[1]);
        if !(b > a) {
            continue;
        }
        let m = ((n as f64 * (b - a) / PI).ceil() as usize).max(4);
        let (gz, gw) = gauss_legendre(m);
        let (c, r) = (0.5 * (a + b), 0.5 * (b - a));
        for (&z, &w) in gz.iter().zip(&gw) {
            let t = c + r * z;
            vs.push(t.sin());
            wv.push(r * w * t.cos());
            near.push(inner.iter().any(|&q| (t - q).abs() < band));
        }
    }
    Ok((vs, wv, near))
}

/// 区間の列 a と b（それぞれ昇順で重ならない）の共通部分
pub(super) fn intersect_segs(a: &[(f64, f64)], b: &[(f64, f64)], out: &mut Vec<(f64, f64)>) {
    out.clear();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let lo = a[i].0.max(b[j].0);
        let hi = a[i].1.min(b[j].1);
        if hi > lo {
            out.push((lo, hi));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
}

/// 瞳が開口・遮蔽で切れているときの 1 波長の複素 OTF（[`WaveOtf`]）。[`lens_quad`] の点は瞳の縁の
/// 不連続をまたぐので、ずらす向きの縁で値が跳び、求積の誤差が周波数ごとにばらつく（周波数 0 でも 1 から
/// 1e-2 ずれ、点を増やしても収束が遅い）。ここでは瞳をずらす向きの線で切り、各線で光線が通る区間
/// （[`pupil_segments`]）を ±δ/2 ずらした共通部分を Gauss-Legendre で積分する。線の位置は [`line_nodes`]。
/// 光量は同じ線の区間の長さの和（周波数 0 で 1 になる）
#[allow(clippy::too_many_arguments)]
pub(super) fn clipped_otf(
    s: &System,
    field: FieldSel,
    wi: usize,
    ip: Option<&PupilInterp>,
    n: usize,
    jm: &[[f64; 2]; 2],
    lam: f64,
    freqs: &[f64],
    sym: bool,
    amp: Option<&FrameAmp>,
    fr: &MtfFrame,
) -> Result<WaveOtf> {
    let (gz, gw) = gauss_legendre(n.max(8));
    let dirs: &[usize] = if sym { &[0] } else { &[0, 1] };
    let mut segs = [Vec::new(), Vec::new()];
    let mut lines = [(Vec::new(), Vec::new()), (Vec::new(), Vec::new())];
    let mut e0 = [0.0; 2];
    for &dir in dirs {
        let (vs, wv, near) = line_nodes(s, field, wi, dir, n.max(T.lines_min), fr)?;
        segs[dir] = pupil_segments(s, field, wi, dir, &vs, &near, fr)?;
        lines[dir] = (vs, wv);
        let (vs, wv) = &lines[dir];
        e0[dir] = match amp {
            None => segs[dir]
                .iter()
                .zip(wv)
                .map(|(sg, &w)| w * sg.iter().map(|(a, b)| b - a).sum::<f64>())
                .sum(),
            // 振幅つき: 区間ごとに振幅の 2 乗を Gauss-Legendre で
            Some(am) => {
                let xy = |a: f64, v: f64| line_xy(dir, a, v);
                let mut e = 0.0;
                for (i, sg) in segs[dir].iter().enumerate() {
                    for &(a, b) in sg {
                        let (c, r) = (0.5 * (a + b), 0.5 * (b - a));
                        for (&g, &wg) in gz.iter().zip(&gw) {
                            let (x, y) = xy(c + r * g, vs[i]);
                            e += wv[i] * r * wg * am.at(x, y).powi(2);
                        }
                    }
                }
                e
            }
        };
        if !(e0[dir] > 0.0) {
            invalid!(code = lensbench_core::error::code::ALL_VIGNETTED; "MTF: no valid rays (all vignetted)");
        }
    }
    if sym {
        e0[1] = e0[0];
    }
    let one = |(k, dir): (usize, usize)| -> Result<C64> {
        // GUI で系が変わったら途中でやめる（周波数・向きごとに確かめる）
        lensbench_core::par::check_cancel()?;
        let (sx, sy) = pupil_shift(jm, lam, dir);
        let dl = if dir == 0 { sx } else { sy } * freqs[k];
        let h = 0.5 * dl.abs();
        let xy = |a: f64, v: f64| line_xy(dir, a, v);
        let (vs, wv) = &lines[dir];
        let (mut xa, mut ya, mut xb, mut yb, mut w) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut ov = Vec::new();
        for (i, sg) in segs[dir].iter().enumerate() {
            let lo: Vec<(f64, f64)> = sg.iter().map(|&(a, b)| (a - h, b - h)).collect();
            let hi: Vec<(f64, f64)> = sg.iter().map(|&(a, b)| (a + h, b + h)).collect();
            intersect_segs(&lo, &hi, &mut ov);
            for &(a, b) in &ov {
                let (c, r) = (0.5 * (a + b), 0.5 * (b - a));
                for (&g, &wg) in gz.iter().zip(&gw) {
                    let t = c + r * g;
                    let (x1, y1) = xy(t + 0.5 * dl, vs[i]);
                    let (x2, y2) = xy(t - 0.5 * dl, vs[i]);
                    xa.push(x1);
                    ya.push(y1);
                    xb.push(x2);
                    yb.push(y2);
                    w.push(wv[i] * r * wg);
                }
            }
        }
        let mut acc = C64::new(0.0, 0.0);
        if w.is_empty() {
            return Ok(acc);
        }
        let oa = opd_at(s, field, wi, fr, ip, &xa, &ya)?;
        let ob = opd_at(s, field, wi, fr, ip, &xb, &yb)?;
        weigh(amp, &mut w, &xa, &ya, &xb, &yb);
        acc += phasor_sum(&oa, &ob, &w);
        Ok(acc)
    };
    let jobs: Vec<(usize, usize)> = (0..freqs.len())
        .flat_map(|k| dirs.iter().map(move |&d| (k, d)))
        .collect();
    let accs: Vec<Result<C64>> = if jobs.len() > 1 && !lensbench_core::par::sequential() {
        use rayon::prelude::*;
        jobs.par_iter().copied().map(scoped(one)).collect()
    } else {
        jobs.iter().map(|&j| one(j)).collect()
    };
    let zero = C64::new(0.0, 0.0);
    let (mut sag, mut tan) = (vec![zero; freqs.len()], vec![zero; freqs.len()]);
    for (&(k, dir), acc) in jobs.iter().zip(accs) {
        let acc = acc?;
        if dir == 0 {
            sag[k] = acc;
            if sym {
                tan[k] = acc;
            }
        } else {
            tan[k] = acc;
        }
    }
    Ok((sag, tan, (e0[0], e0[1])))
}
