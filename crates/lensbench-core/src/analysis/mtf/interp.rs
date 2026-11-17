//! 瞳の OPD の格子と 6 点補間（[`super::MtfMethod::Interp`]）、求積の点数の見積もり

use super::*;

/// 瞳の OPD の傾きの最大 [波 / 正規化瞳の半径]。一辺 `Tuning::slope_grid` 点（[-1, 1] を `slope_grid` − 1 区間）の
/// 格子の隣り合う点の差で見積もる（けられた点は飛ばす）
pub fn opd_slope_max(s: &System, field: FieldSel, w: usize) -> Result<f64> {
    Ok(PupilInterp::new(s, field, w, T.slope_grid - 1, &MtfFrame::IDENTITY)?.slope_max())
}

/// MTF の求積の点数 n（None なら OPD の傾きから自動で決める）と、傾きを見積もった格子（`Tuning::slope_grid`）。
/// 傾きは向きによらず正規化瞳の x・y の格子で見積もる（組を回すと格子の点が変わり、n が変わりうる）ので、
/// 組 fr が恒等のときだけ、その格子を補間の格子（[`mtf_interp`]）の最初の候補として返す（同じ大きさなら
/// そのまま使い、同じ点を 2 度追わない）
pub(super) fn pupil_points(
    s: &System,
    field: FieldSel,
    wi: usize,
    n: Option<usize>,
    fr: &MtfFrame,
) -> Result<(usize, Option<PupilInterp>)> {
    Ok(match n {
        Some(n) => (n, None),
        None => {
            let ip = PupilInterp::new(s, field, wi, T.slope_grid - 1, &MtfFrame::IDENTITY)?;
            let n = mtf_points(ip.slope_max());
            (n, fr.is_identity().then_some(ip))
        }
    })
}

/// MTF の補間（求積の点数 n）。格子の一辺を min(n, [`Tuning::interp_grid_start`]) から始め、区間の中点の
/// 標本で補間と直接追跡の差が [`Tuning::interp_tol`] を超えれば 2 倍にする（上限 n。n 以下の格子は従来と同じ）。
/// 格子 n でも差が [`Tuning::interp_mask_tol`] を超えるとき（外挿しない Zernike 面の正規化半径の縁など、OPD に
/// 段差がある系）は、全区間の中点で確かめ、差がそれを超える区間の角の格子点を NaN にする。足にその点を
/// 含む求積点は直接追跡になる。中点の標本（一辺およそ 16 点）の半分以上で超えるときは、格子をすべて NaN にして
/// 全部直接追跡
pub(super) fn mtf_interp(
    s: &System,
    field: FieldSel,
    wi: usize,
    n: usize,
    fr: &MtfFrame,
    mut first: Option<PupilInterp>,
) -> Result<PupilInterp> {
    let n = n.max(T.points_min);
    let mut g = n.min(T.interp_grid_start);
    loop {
        let mut ip = match first.take() {
            Some(ip) if ip.cells == g => ip,
            _ => PupilInterp::new(s, field, wi, g, fr)?,
        };
        let cells = ((2.0 / ip.h).round() as usize).max(1);
        if g < n {
            let e = interp_errors(s, field, wi, &ip, cells.div_ceil(48).max(1))?
                .iter()
                .fold(0.0_f64, |m, &(_, _, e)| m.max(e));
            if e <= T.interp_tol {
                return Ok(ip);
            }
            g = (2 * g).min(n);
            continue;
        }
        // 格子 n: 段差・粗すぎる格子を一辺およそ 16 点の中点で探す（安く済ませる。見つからなければ従来と同じ）
        let errs = interp_errors(s, field, wi, &ip, cells.div_ceil(16).max(1))?;
        let nb = errs.iter().filter(|x| x.2 > T.interp_mask_tol).count();
        if 2 * nb >= errs.len() && nb > 0 {
            // 瞳の半分以上で補間が効かない（格子の面など、OPD が格子の線ごとに折れる系）:
            // 全区間の確かめを省き、すべて直接追跡にする
            ip.w.fill(f64::NAN);
        } else if nb > 0 {
            let bad: Vec<(usize, usize)> = interp_errors(s, field, wi, &ip, 1)?
                .into_iter()
                .filter(|&(_, _, e)| e > T.interp_mask_tol)
                .map(|(ix, iy, _)| (ix, iy))
                .collect();
            ip.mask_cells(&bad);
        }
        return Ok(ip);
    }
}

/// 補間の格子 ip の区間の中点（step 区間おき）で、補間と直接追跡の差 [波]: (区間の番号 ix, iy, 差)。
/// 中点が瞳（半径 1 に区間 1 つ分を足した円）の外の区間と、補間・直接追跡のどちらかができない点は除く
pub(super) fn interp_errors(
    s: &System,
    field: FieldSel,
    wi: usize,
    ip: &PupilInterp,
    step: usize,
) -> Result<Vec<(usize, usize, f64)>> {
    let cells = ((2.0 / ip.h).round() as usize).max(1);
    let lim = if step == 1 { (1.0 + ip.h).powi(2) } else { 1.0 };
    let (mut px, mut py, mut id) = (Vec::new(), Vec::new(), Vec::new());
    for iy in (0..cells).step_by(step) {
        for ix in (0..cells).step_by(step) {
            let x = -1.0 + (ix as f64 + 0.5) * ip.h;
            let y = -1.0 + (iy as f64 + 0.5) * ip.h;
            if x * x + y * y < lim {
                px.push(x);
                py.push(y);
                id.push((ix, iy));
            }
        }
    }
    let o = frame_opd(s, field, wi, &ip.fr, &px, &py)?;
    let mut out = Vec::new();
    for k in 0..px.len() {
        if let (Some(v), true) = (ip.eval(px[k], py[k]), o.ok(k) && o.opd[k].is_finite()) {
            out.push((id[k].0, id[k].1, (v - o.opd[k]).abs()));
        }
    }
    Ok(out)
}

/// 組 fr の座標 (u, v)（u は球欠 s、v は子午 t の向き。[`MtfFrame::pupil_xy`]）の点の OPD。正規化瞳の座標に
/// 直して追跡する（恒等なら直さない）
pub(super) fn frame_opd(
    s: &System,
    field: FieldSel,
    w: usize,
    fr: &MtfFrame,
    u: &[f64],
    v: &[f64],
) -> Result<crate::analysis::Opd> {
    if fr.is_identity() {
        return opd(s, field, u, v, Some(w), true);
    }
    let (px, py): (Vec<f64>, Vec<f64>) = u.iter().zip(v).map(|(&a, &b)| fr.pupil_xy(a, b)).unzip();
    opd(s, field, &px, &py, Some(w), true)
}

/// 組 fr の座標 (u, v) の正方格子で追跡した OPD の補間（各軸 6 点の Lagrange、5 次）。格子が子午・球欠の向きに
/// 沿うので、どちらかの向きにずらした点の列は格子の線に乗る（[`PupilInterp::eval_lines`]）
pub(super) struct PupilInterp {
    /// 格子の点数（一辺）・[-1, 1] の区間の数・間隔・始点
    m: usize,
    cells: usize,
    pub(super) h: f64,
    x0: f64,
    /// OPD [波]（けられ・止まった点と、追わない円の外の点（[`PupilInterp::sampled`]）は NaN）、行優先 [iy][ix]
    pub(super) w: Vec<f64>,
    /// 格子の向きの組
    pub(super) fr: MtfFrame,
}

pub(super) const LAG: usize = 6;

impl PupilInterp {
    /// 一辺 n 区間で [-1, 1] を覆い、補間の足が届くよう外側に 3 点ずつ足す。追うのは [`PupilInterp::sampled`] の点だけ
    fn new(s: &System, field: FieldSel, w: usize, n: usize, fr: &MtfFrame) -> Result<PupilInterp> {
        crate::par::check_cancel()?;
        let h = 2.0 / n as f64;
        let pad = LAG / 2;
        let m = n + 1 + 2 * pad;
        let x0 = -1.0 - pad as f64 * h;
        let mut ip = PupilInterp {
            m,
            cells: n,
            h,
            x0,
            w: vec![f64::NAN; m * m],
            fr: *fr,
        };
        let idx: Vec<usize> = (0..m * m).filter(|&i| ip.sampled(i)).collect();
        let px: Vec<f64> = idx.iter().map(|&i| ip.node(i).0).collect();
        let py: Vec<f64> = idx.iter().map(|&i| ip.node(i).1).collect();
        let o = frame_opd(s, field, w, fr, &px, &py)?;
        for (k, &i) in idx.iter().enumerate() {
            if o.ok(k) && o.opd[k].is_finite() {
                ip.w[i] = o.opd[k];
            }
        }
        Ok(ip)
    }

    /// 格子点 i の座標
    #[inline]
    fn node(&self, i: usize) -> (f64, f64) {
        (
            self.x0 + (i % self.m) as f64 * self.h,
            self.x0 + (i / self.m) as f64 * self.h,
        )
    }

    /// 格子点 i を追うか: 中心から 1 + 5.5h 以内。補間するのは半径 1 + h 以内の点（瞳の中の求積点と、区間の中点の
    /// 確かめ）だけで、その足（各軸 ±3 区間）は中心から 1 + (1 + 3√2)h ≈ 1 + 5.25h 以内に収まる。正方形の
    /// 隅の点（全体の 2 割ほど）は使わないので追わない
    #[inline]
    fn sampled(&self, i: usize) -> bool {
        let (x, y) = self.node(i);
        x.hypot(y) <= 1.0 + 5.5 * self.h
    }

    /// 追った点がどれも有限（けられ・止まった点が無い）か
    pub(super) fn all_finite(&self) -> bool {
        (0..self.w.len()).all(|i| !self.sampled(i) || self.w[i].is_finite())
    }

    /// 単位円（と縁の許し）の中の隣り合う格子点の OPD の差から、OPD の傾きの最大 [波 / 正規化瞳の半径]
    /// （けられた点は飛ばす）
    fn slope_max(&self) -> f64 {
        let m = self.m;
        let ok = |i: usize| {
            let (x, y) = self.node(i);
            x.hypot(y) <= 1.0 + crate::tol::PUPIL_RIM && self.w[i].is_finite()
        };
        let mut g: f64 = 0.0;
        for iy in 0..m {
            for ix in 0..m {
                let k = iy * m + ix;
                if !ok(k) {
                    continue;
                }
                if ix + 1 < m && ok(k + 1) {
                    g = g.max((self.w[k + 1] - self.w[k]).abs() / self.h);
                }
                if iy + 1 < m && ok(k + m) {
                    g = g.max((self.w[k + m] - self.w[k]).abs() / self.h);
                }
            }
        }
        g
    }

    /// 1 点の補間。足の 36 点のどれかが NaN（けられ）なら None
    fn eval(&self, x: f64, y: f64) -> Option<f64> {
        let (wx, ix) = self.weights(x)?;
        let (wy, iy) = self.weights(y)?;
        let mut acc = 0.0;
        for (a, &cy) in wy.iter().enumerate() {
            let o = (iy + a) * self.m + ix;
            let row: &[f64; LAG] = self.w[o..o + LAG].try_into().expect("slice of LAG");
            let mut r = 0.0;
            for k in 0..LAG {
                r += row[k] * wx[k];
            }
            acc += r * cy;
        }
        acc.is_finite().then_some(acc)
    }

    /// 1 軸の Lagrange の重みと足の最初の番号
    fn weights(&self, x: f64) -> Option<([f64; LAG], usize)> {
        let t = (x - self.x0) / self.h;
        if !t.is_finite() {
            return None;
        }
        // floor（libm を呼ばない）
        let fl = t as isize - ((t as isize) as f64 > t) as isize;
        let i = fl - (LAG as isize / 2 - 1);
        if i < 0 || i as usize + LAG > self.m {
            return None;
        }
        let u = t - i as f64; // 足の点 0..LAG に対する位置
        let wv = crate::interp::lagrange::<LAG>(u);
        Some((wv, i as usize))
    }
}

impl PupilInterp {
    /// 区間 (ix, iy)（[-1, 1] を区切った区間の番号）の 4 隅の格子点を NaN にする（補間させず直接追跡させる）
    fn mask_cells(&mut self, cells: &[(usize, usize)]) {
        let pad = LAG / 2;
        for &(ix, iy) in cells {
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (gx, gy) = (pad + ix + dx, pad + iy + dy);
                if gx < self.m && gy < self.m {
                    self.w[gy * self.m + gx] = f64::NAN;
                }
            }
        }
    }
}

/// 組 fr の座標の点 (x, y) の OPD [波]。補間できない点は直接追跡する。けられ・止まった点は NaN
pub(super) fn opd_at(
    s: &System,
    field: FieldSel,
    w: usize,
    fr: &MtfFrame,
    ip: Option<&PupilInterp>,
    x: &[f64],
    y: &[f64],
) -> Result<Vec<f64>> {
    let mut out = vec![f64::NAN; x.len()];
    let mut miss = Vec::new();
    match ip {
        Some(ip) => {
            // 点ごとに独立（並列でも値は同じ）
            use rayon::prelude::*;
            let f = |(i, o): (usize, &mut f64)| {
                if let Some(v) = ip.eval(x[i], y[i]) {
                    *o = v;
                }
            };
            if x.len() >= 4096 && !crate::par::sequential() {
                out.par_iter_mut()
                    .enumerate()
                    .with_min_len(1024)
                    .for_each(scoped(f));
            } else {
                out.iter_mut().enumerate().for_each(f);
            }
            miss.extend((0..x.len()).filter(|&i| out[i].is_nan()));
        }
        None => miss.extend(0..x.len()),
    }
    if !miss.is_empty() {
        let mx: Vec<f64> = miss.iter().map(|&i| x[i]).collect();
        let my: Vec<f64> = miss.iter().map(|&i| y[i]).collect();
        let o = frame_opd(s, field, w, fr, &mx, &my)?;
        for (k, &i) in miss.iter().enumerate() {
            if o.ok(k) && o.opd[k].is_finite() {
                out[i] = o.opd[k];
            }
        }
    }
    Ok(out)
}

impl PupilInterp {
    /// 1 軸が一定の点の列の補間（行ごとに固定の軸を先に縮約するので [`PupilInterp::eval`] より速い）。
    /// fixed_x なら点は (c, v[i])、そうでなければ (v[i], c)。2 本の列 ca・cb を同じ v で求め、補間できない点は
    /// NaN のまま。fixed_x の値は [`PupilInterp::eval`] とビットまで同じ（足し算の順が同じ）
    #[allow(clippy::too_many_arguments)]
    pub(super) fn eval_lines(
        &self,
        fixed_x: bool,
        ca: f64,
        cb: f64,
        v: &[f64],
        oa: &mut [f64],
        ob: &mut [f64],
        col: &mut [Vec<f64>; 2],
    ) {
        let m = self.m;
        let mut have = [false; 2];
        for (j, c) in [ca, cb].into_iter().enumerate() {
            let Some((wc, ic)) = self.weights(c) else {
                continue;
            };
            have[j] = true;
            let cj = &mut col[j];
            cj.clear();
            if fixed_x {
                cj.extend((0..m).map(|iy| {
                    let o = iy * m + ic;
                    let row: &[f64; LAG] = self.w[o..o + LAG].try_into().expect("slice of LAG");
                    let mut r = 0.0;
                    for k in 0..LAG {
                        r += row[k] * wc[k];
                    }
                    r
                }));
            } else {
                cj.resize(m, 0.0);
                for (a, &wa) in wc.iter().enumerate() {
                    let row = &self.w[(ic + a) * m..(ic + a + 1) * m];
                    for (o, &r) in cj.iter_mut().zip(row) {
                        *o += r * wa;
                    }
                }
            }
        }
        if !have[0] && !have[1] {
            return;
        }
        for (i, &x) in v.iter().enumerate() {
            let Some((wv, iv)) = self.weights(x) else {
                continue;
            };
            for (j, o) in [&mut oa[i], &mut ob[i]].into_iter().enumerate() {
                if !have[j] {
                    continue;
                }
                let c = &col[j][iv..iv + LAG];
                let mut acc = 0.0;
                for a in 0..LAG {
                    acc += c[a] * wv[a];
                }
                if acc.is_finite() {
                    *o = acc;
                }
            }
        }
    }

    /// 格子の OPD が x ↔ y の入れ替えと y の反転で変わらない（|差| ≤ 1e-9 波、けられの形も同じ）か。
    /// 軸上の回転対称な系では子午と球欠の MTF が同じになる
    fn symmetric(&self) -> bool {
        let m = self.m;
        let same = |a: f64, b: f64| (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-9;
        (0..m).all(|iy| {
            (0..m).all(|ix| {
                let v = self.w[iy * m + ix];
                same(v, self.w[ix * m + iy]) && same(v, self.w[(m - 1 - iy) * m + ix])
            })
        })
    }
}

/// 子午と球欠の MTF が同じになるか（軸上の回転対称な系）: 格子の OPD が対称（[`PupilInterp::symmetric`]）で、
/// 瞳の縮尺 J が x・y で同じ
pub(super) fn axis_symmetric(jm: &[[f64; 2]; 2], ip: &PupilInterp) -> bool {
    let a = jm[0][0].abs() * crate::tol::JACOBIAN_REL;
    (jm[0][0] - jm[1][1]).abs() <= a && jm[0][1].abs() <= a && jm[1][0].abs() <= a && ip.symmetric()
}

/// 直接追跡の点（補間できなかった点、組 fr の座標）を埋める
pub(super) fn fill_direct(
    s: &System,
    field: FieldSel,
    w: usize,
    fr: &MtfFrame,
    x: &[f64],
    y: &[f64],
    out: &mut [f64],
) -> Result<()> {
    let miss: Vec<usize> = (0..x.len()).filter(|&i| out[i].is_nan()).collect();
    if !miss.is_empty() {
        let mx: Vec<f64> = miss.iter().map(|&i| x[i]).collect();
        let my: Vec<f64> = miss.iter().map(|&i| y[i]).collect();
        let o = frame_opd(s, field, w, fr, &mx, &my)?;
        for (k, &i) in miss.iter().enumerate() {
            if o.ok(k) && o.opd[k].is_finite() {
                out[i] = o.opd[k];
            }
        }
    }
    Ok(())
}

/// 周波数・向きごとの求積の点数（[`MtfMethod::Auto`]・[`MtfMethod::Interp`] で点数が自動のとき。上限は自動の n）。
/// 求積するのは位相差 D(p) = W(p + s/2) − W(p − s/2)（s はずらし (dx, dy)）の位相因子なので、要る点数は W ではなく
/// D の傾きで決まる。D の傾きの最大 g_D [波 / 半径] を、重なりの中の [`Tuning::shift_grid`]² 点で補間した D の隣り合う
/// 点の差で見積もり、n_D = [`Tuning::shift_fac`]·g_D + [`Tuning::points_pad`]（[`Tuning::points_min`]〜[`Tuning::points_max`]）にする。自動の n（[`mtf_points`]）は W の傾き g から D の
/// 傾きの上限 2g を見込むが、低い周波数（ずらしが小さい）では D ≈ s·∇W で傾きはずっと小さい（OPD の傾きが
/// 600 波 / 半径を超える系の低い周波数で、g_D は g の 1/15 ほど）。補間の格子がすべて有限（けられなし）のときだけ呼ぶ
pub(super) fn shift_points(ip: &PupilInterp, dx: f64, dy: f64) -> usize {
    let m = T.shift_grid;
    let d = dx.hypot(dy);
    let a = 1.0 - 0.5 * d;
    if a <= 0.0 {
        return T.points_min;
    }
    let (ex, ey) = if d > 0.0 {
        (dx / d, dy / d)
    } else {
        (1.0, 0.0)
    };
    // 重なり（レンズ形）の中の m×m 点: ずらす向きの座標 t は区間の中心、直交する座標 u は各 t の弦を m 等分した
    // 区間の中心（[`lens_quad`] と同じ形の領域を覆う。周波数が高いと領域が細く、正方格子では点が足りない）
    let mut dv = vec![f64::NAN; m * m];
    let mut pt = vec![(0.0, 0.0); m * m];
    for i in 0..m {
        let t = a * (-1.0 + (2 * i + 1) as f64 / m as f64);
        let x = (1.0 - (t.abs() + 0.5 * d).powi(2)).max(0.0).sqrt();
        for j in 0..m {
            let u = x * (-1.0 + (2 * j + 1) as f64 / m as f64);
            let (px, py) = (t * ex - u * ey, t * ey + u * ex);
            pt[i * m + j] = (px, py);
            if let (Some(va), Some(vb)) = (
                ip.eval(px + 0.5 * dx, py + 0.5 * dy),
                ip.eval(px - 0.5 * dx, py - 0.5 * dy),
            ) {
                dv[i * m + j] = va - vb;
            }
        }
    }
    let mut g: f64 = 0.0;
    let mut slope = |k: usize, k2: usize| {
        let (v, v2) = (dv[k], dv[k2]);
        let dist = (pt[k2].0 - pt[k].0).hypot(pt[k2].1 - pt[k].1);
        if v.is_finite() && v2.is_finite() && dist > 0.0 {
            g = g.max((v2 - v).abs() / dist);
        }
    };
    for i in 0..m {
        for j in 0..m {
            let k = i * m + j;
            if j + 1 < m {
                slope(k, k + 1);
            }
            if i + 1 < m {
                slope(k, k + m);
            }
        }
    }
    ((T.shift_fac * g).ceil() as usize + T.points_pad).clamp(T.points_min, T.points_max)
}
