//! FFT の自己相関による OTF（[`super::MtfMethod::Fft`]）

use super::*;

/// ずらす向きが軸に沿うときの重なりの求積（[`lens_quad`] と同じ点と重み）。[`lens_quad`] の行（t が一定）
/// ごとに、補間の固定の軸を先に縮約する（[`PupilInterp::eval_lines`]）。和は行ごとに取って行の順に足し、
/// 補間できなかった点は最後に直接追跡して足す（スレッド数によらず同じ値）
#[allow(clippy::too_many_arguments)]
pub(super) fn shifted_rows(
    s: &System,
    field: FieldSel,
    wi: usize,
    ip: &PupilInterp,
    gl: &LensGl,
    d: f64,
    (dx, dy): (f64, f64),
    (ex, ey): (f64, f64),
    amp: Option<&FrameAmp>,
) -> Result<C64> {
    let a = 1.0 - 0.5 * d;
    let mut acc = C64::new(0.0, 0.0);
    if a <= 0.0 {
        return Ok(acc);
    }
    let fixed_x = ey == 0.0;
    let nr = gl.gt.len() * 2;
    let row = |r: usize, buf: &mut [Vec<f64>; 5]| -> (C64, Vec<Miss>) {
        let (z, wz) = (gl.gt[r / 2], gl.wt[r / 2]);
        let sgn = if r.is_multiple_of(2) { 1.0 } else { -1.0 };
        let tau = 0.5 * (z + 1.0);
        let y = a * (1.0 - tau * tau);
        let dyq = 2.0 * a * tau * 0.5 * wz;
        let x = (1.0 - (y + 0.5 * d).powi(2)).max(0.0).sqrt();
        let t = sgn * y;
        let [var, oa, ob, ca, cb] = buf;
        // 点 (t, u)（u = x·g）を回した (X, Y) = (t ex − u ey, t ey + u ex)。行の中で一定の軸は t だけで決まる
        var.clear();
        let (fa, fb) = if fixed_x {
            var.extend(gl.gu.iter().map(|&g| (x * g) * ex));
            let xx = t * ex;
            (xx + 0.5 * dx, xx - 0.5 * dx)
        } else {
            var.extend(gl.gu.iter().map(|&g| -((x * g) * ey)));
            let yy = t * ey;
            (yy + 0.5 * dy, yy - 0.5 * dy)
        };
        let nu = var.len();
        oa.clear();
        oa.resize(nu, f64::NAN);
        ob.clear();
        ob.resize(nu, f64::NAN);
        let mut col = [std::mem::take(ca), std::mem::take(cb)];
        ip.eval_lines(fixed_x, fa, fb, var, oa, ob, &mut col);
        [*ca, *cb] = col;
        let mut miss = Vec::new();
        let mut ws: Vec<f64> = gl.wu.iter().map(|&wg| dyq * x * wg).collect();
        if let Some(am) = amp {
            for (i, w) in ws.iter_mut().enumerate() {
                let (xa, ya, xb, yb) = if fixed_x {
                    (fa, var[i], fb, var[i])
                } else {
                    (var[i], fa, var[i], fb)
                };
                *w *= am.at(xa, ya) * am.at(xb, yb);
            }
        }
        for (i, &w) in ws.iter().enumerate() {
            let (va, vb) = (oa[i], ob[i]);
            if va.is_nan() || vb.is_nan() {
                let (xa, ya, xb, yb) = if fixed_x {
                    (fa, var[i], fb, var[i])
                } else {
                    (var[i], fa, var[i], fb)
                };
                miss.push((xa, ya, xb, yb, w, va, vb));
            }
        }
        // 補間できた点（NaN でない点）の和。oa・ob は補間できないと NaN、できれば有限
        let sum = phasor_sum(oa, ob, &ws);
        (sum, miss)
    };
    let new_buf = || -> [Vec<f64>; 5] { Default::default() };
    let rows: Vec<(C64, Vec<Miss>)> =
        if nr * gl.gu.len() >= 4096 && !lensbench_core::par::sequential() {
            use rayon::prelude::*;
            let cx = lensbench_core::par::current();
            (0..nr)
                .into_par_iter()
                .with_min_len((1024 / gl.gu.len()).max(1))
                .map_init(new_buf, |b, r| {
                    lensbench_core::par::with_ctx(&cx, || row(r, b))
                })
                .collect()
        } else {
            let mut b = new_buf();
            (0..nr).map(|r| row(r, &mut b)).collect()
        };
    let mut miss: Vec<Miss> = Vec::new();
    for (sum, m) in rows {
        acc += sum;
        miss.extend(m);
    }
    if !miss.is_empty() {
        let mut va: Vec<f64> = miss.iter().map(|m| m.5).collect();
        let mut vb: Vec<f64> = miss.iter().map(|m| m.6).collect();
        let (xa, ya): (Vec<f64>, Vec<f64>) = miss.iter().map(|m| (m.0, m.1)).unzip();
        let (xb, yb): (Vec<f64>, Vec<f64>) = miss.iter().map(|m| (m.2, m.3)).unzip();
        fill_direct(s, field, wi, &ip.fr, &xa, &ya, &mut va)?;
        fill_direct(s, field, wi, &ip.fr, &xb, &yb, &mut vb)?;
        for (i, m) in miss.iter().enumerate() {
            if va[i].is_finite() && vb[i].is_finite() {
                acc += phasor(va[i] - vb[i]) * m.4;
            }
        }
    }
    Ok(acc)
}

/// 瞳の格子の行ごとの自己相関の和 r[s] = Σ_行 Σ_i P[i + s] P*[i]（s = 0..L、負の s は L + s）。
/// line(k, buf) が行 k の P（長さ ns）を buf に書く（全面を持たずに済む）。L ≥ 2 ns で巡回の重なりなし
pub(super) fn row_autocorr(
    ns: usize,
    l: usize,
    line: impl Fn(usize, &mut [C64]) + Sync,
) -> Vec<C64> {
    let fft = rustfft::FftPlanner::<f64>::new().plan_fft_forward(l);
    let zero = C64::new(0.0, 0.0);
    // 行を決まった大きさの塊に分けて塊ごとに |F|² を足し、塊の順に足す（スレッド数によらず同じ値）
    const CH: usize = 16;
    let groups: Vec<usize> = (0..ns.div_ceil(CH)).collect();
    let parts = lensbench_core::par::map(&groups, |&g| {
        let mut buf = vec![zero; l];
        let mut scratch = vec![zero; fft.get_inplace_scratch_len()];
        let mut acc = vec![0.0; l];
        for k in g * CH..((g + 1) * CH).min(ns) {
            line(k, &mut buf[..ns]);
            if buf[..ns].iter().all(|c| c.re == 0.0 && c.im == 0.0) {
                continue;
            }
            buf[ns..].fill(zero);
            fft.process_with_scratch(&mut buf, &mut scratch);
            for (a, f) in acc.iter_mut().zip(&buf) {
                *a += f.norm_sqr();
            }
        }
        acc
    });
    let mut sp = vec![zero; l];
    for q in parts {
        for (a, v) in sp.iter_mut().zip(q) {
            a.re += v;
        }
    }
    let ifft = rustfft::FftPlanner::<f64>::new().plan_fft_inverse(l);
    ifft.process(&mut sp);
    let k = 1.0 / l as f64;
    sp.iter().map(|c| c * k).collect()
}

/// 自己相関 r（[`row_autocorr`]）の非整数のずれ s での値（4 点の Lagrange 補間、r[−s] = r[s]*）。
/// 誤差は標本の間隔の 4 乗で減り、線形補間（2 乗）より 1 桁ほど合う
pub(super) fn autocorr_at(r: &[C64], s: f64) -> C64 {
    let l = r.len() as isize;
    let at = |i: isize| -> C64 {
        if i >= 0 {
            r[i as usize]
        } else {
            r[(l + i) as usize]
        }
    };
    let i0 = s.floor();
    let u = s - i0;
    let i0 = i0 as isize;
    if u == 0.0 {
        return at(i0);
    }
    // 点 i0−1, i0, i0+1, i0+2（節点の番号 0..4 で u + 1 の位置）
    let w = lensbench_core::interp::lagrange::<4>(u + 1.0);
    (0..4).map(|k| at(i0 - 1 + k as isize) * w[k]).sum()
}

/// 1 波長の複素 OTF を FFT の自己相関で求める（[`MtfMethod::Fft`]）。返す値は [`wave_otf`] と同じ形。
/// 瞳の標本の OPD は [`PupilInterp`] の補間。球欠は行ごと、子午は列ごとの 1 次元の自己相関の和で、
/// 周波数のずれ（標本の間隔の非整数倍）は自己相関を 4 点で補間する
#[allow(clippy::too_many_arguments)]
pub(super) fn wave_otf_fft(
    s: &System,
    field: FieldSel,
    wi: usize,
    n: usize,
    first: Option<PupilInterp>,
    freqs: &[f64],
    amp: Option<&FrameAmp>,
    fr: &MtfFrame,
) -> Result<WaveOtf> {
    let jm = fr.jacobian(s, field, wi)?;
    let ip = mtf_interp(s, field, wi, n, fr, first)?;
    let lam = s.wave(wi).mm();
    // 瞳の標本（直径 ns 点、区間の中心）。円の縁は中心が内側かどうかで 0・1 にする（面積の割合を振幅にすると
    // 縁どうしの重なりが割合の積になって h に比例してずれる）。補間との差は ns² に反比例
    // する。標本は求積の点数の 2 倍にする（直径に 2n ≈ 6g 点で、OPD の 1 波あたり 3 点ほど）。
    // OPD の傾きが 600 波 / 半径を超える系では n が 2000 を超えるので、上限は 4096（2048 では高い周波数で MTF が
    // 1.7e-3 浮いた）
    let ns = (2 * n).clamp(T.fft_min, T.fft_max);
    let h = 2.0 / ns as f64;
    let cx: Vec<f64> = (0..ns).map(|i| -1.0 + h * (i as f64 + 0.5)).collect();
    let inside = |i: usize| {
        let (x, y) = (cx[i % ns], cx[i / ns]);
        x * x + y * y <= 1.0
    };
    // 標本 i の組の座標（行は球欠 s の向き、列は子午 t の向き。恒等なら行が x）。補間の格子も組の座標なので
    // 行は格子の線に乗る
    let pt = |i: usize| (cx[i % ns], cx[i / ns]);
    // 各行の OPD（行ごとに補間、補間できない点は直接追跡）
    let mut wv = vec![f64::NAN; ns * ns];
    {
        use rayon::prelude::*;
        let f = |(iy, row): (usize, &mut [f64])| {
            let mut tmp = vec![f64::NAN; ns];
            let mut col = [Vec::new(), Vec::new()];
            ip.eval_lines(false, cx[iy], cx[iy], &cx, row, &mut tmp, &mut col);
        };
        if !lensbench_core::par::sequential() {
            wv.par_chunks_mut(ns).enumerate().for_each(scoped(f));
        } else {
            wv.chunks_mut(ns).enumerate().for_each(f);
        }
    }
    let need: Vec<usize> = (0..ns * ns)
        .filter(|&i| wv[i].is_nan() && inside(i))
        .collect();
    if !need.is_empty() {
        let mx: Vec<f64> = need.iter().map(|&i| pt(i).0).collect();
        let my: Vec<f64> = need.iter().map(|&i| pt(i).1).collect();
        let mut v = vec![f64::NAN; need.len()];
        fill_direct(s, field, wi, fr, &mx, &my, &mut v)?;
        for (k, &i) in need.iter().enumerate() {
            wv[i] = v[k];
        }
    }
    let zero = C64::new(0.0, 0.0);
    let ok = |i: usize| inside(i) && wv[i].is_finite();
    // 振幅（一様なら 1 で、光量は標本の数）
    let am = |i: usize| {
        amp.map_or(1.0, |a| {
            let (x, y) = pt(i);
            a.at(x, y)
        })
    };
    let e0 = match amp {
        None => (0..ns * ns).filter(|&i| ok(i)).count() as f64,
        Some(_) => (0..ns * ns).filter(|&i| ok(i)).map(|i| am(i).powi(2)).sum(),
    };
    if !(e0 > 0.0) {
        invalid!("MTF: no valid rays (all vignetted)");
    }
    // 瞳の標本の位相因子は行（列）ごとに作る（ns = 4096 で複素数の全面は 268 MB になるので持たない）
    let at = |i: usize| match (ok(i), amp) {
        (false, _) => zero,
        (true, None) => phasor(wv[i]),
        (true, Some(_)) => phasor(wv[i]) * am(i),
    };
    let l = (2 * ns).next_power_of_two();
    // 球欠: 行（s の向き）の自己相関、子午: 列（t の向き）
    let rs = row_autocorr(ns, l, |iy, b| {
        for (ix, v) in b.iter_mut().enumerate() {
            *v = at(iy * ns + ix);
        }
    });
    let rt = if axis_symmetric(&jm, &ip) && amp.is_none_or(|a| a.rotationally_symmetric()) {
        rs.clone()
    } else {
        row_autocorr(ns, l, |ix, b| {
            for (iy, v) in b.iter_mut().enumerate() {
                *v = at(iy * ns + ix);
            }
        })
    };
    drop(wv);
    let (mut sag, mut tan) = (Vec::new(), Vec::new());
    let (sx, _) = pupil_shift(&jm, lam, 0);
    let (_, sy) = pupil_shift(&jm, lam, 1);
    for &f in freqs {
        let (dx, dy) = (sx * f, sy * f);
        sag.push(if dx.abs() < 2.0 {
            autocorr_at(&rs, dx / h)
        } else {
            zero
        });
        tan.push(if dy.abs() < 2.0 {
            autocorr_at(&rt, dy / h)
        } else {
            zero
        });
    }
    Ok((sag, tan, (e0, e0)))
}
