//! Zernike の当てはめ（OPD の最小二乗。OpticStudio の Zernike Coefficients）。最小二乗は [`lensbench_core::linalg`]。

use super::*;
use lensbench_core::linalg::{dot4, lstsq_cols, Cholesky};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZernKind {
    Fringe,
    Standard,
}

/// 項 1..nterms の (n, m, cos なら true)
pub fn zern_nm(kind: ZernKind, nterms: usize) -> Vec<(usize, usize, bool)> {
    let mut out = Vec::new();
    match kind {
        ZernKind::Fringe => {
            assert!(nterms <= 37, "Fringe Zernike has at most 37 terms");
            for g in 0..6usize {
                for m in (0..=g).rev() {
                    let n = 2 * g - m;
                    if m > 0 {
                        out.push((n, m, true));
                        out.push((n, m, false));
                    } else {
                        out.push((n, 0, true));
                    }
                }
            }
            out.push((12, 0, true));
        }
        ZernKind::Standard => {
            let (mut j, mut n) = (1usize, 0usize);
            while out.len() < nterms {
                let mut m = n % 2;
                while m <= n {
                    if m == 0 {
                        out.push((n, 0, true));
                        j += 1;
                    } else {
                        for _ in 0..2 {
                            out.push((n, m, j % 2 == 0));
                            j += 1;
                        }
                    }
                    m += 2;
                }
                n += 1;
            }
        }
    }
    out.truncate(nterms);
    out
}

pub(super) fn fact(n: usize) -> f64 {
    (1..=n).map(|v| v as f64).product()
}

/// 動径の番号と、それを使う項（番号, cos なら true）
type RadUsers = (usize, Vec<(usize, bool)>);

/// 項ごとに前もって求めた Zernike 多項式。動径は R_n^m(r) = r^m · P(r²)（P は r² の多項式、正規化の係数込み）で、
/// cos と sin の組は同じ P を使う。r^m cos(mθ)・r^m sin(mθ) は (x + iy)^m の実部・虚部（平方根・割り算・三角関数なし）
pub struct ZernBasis {
    /// 異なる (n, m) ごとの P の係数（r² の高次から、Horner 用）を続けて置いたもの
    coef: Vec<f64>,
    /// 異なる (n, m) ごとの (coef の始まり, 長さ, m)
    rad: Vec<(usize, usize, usize)>,
    /// 項ごとの (rad の番号, cos なら true)
    terms: Vec<(usize, bool)>,
    mmax: usize,
    /// m ごとの、その m の動径の番号と、それを使う項（番号, cos なら true）。eval_cols で m の小さい順に回す
    by_m: Vec<Vec<RadUsers>>,
}

impl ZernBasis {
    pub fn new(kind: ZernKind, nm: &[(usize, usize, bool)]) -> ZernBasis {
        assert!(nm.iter().all(|t| t.0 < 64), "Zernike orders go up to 63");
        let mut coef = Vec::new();
        let mut rad: Vec<(usize, usize, usize)> = Vec::new();
        let mut keys: Vec<(usize, usize)> = Vec::new();
        let terms: Vec<(usize, bool)> = nm
            .iter()
            .map(|&(n, m, c)| {
                let ri = match keys.iter().position(|&k| k == (n, m)) {
                    Some(i) => i,
                    None => {
                        let norm = if kind == ZernKind::Standard {
                            (((n + 1) * if m > 0 { 2 } else { 1 }) as f64).sqrt()
                        } else {
                            1.0
                        };
                        // r^(n-2k) = r^m (r²)^((n-m)/2-k): k = 0 が r² の最高次
                        let start = coef.len();
                        for k in 0..=(n - m) / 2 {
                            let sgn = if k % 2 == 0 { 1.0 } else { -1.0 };
                            let ck = sgn * fact(n - k)
                                / (fact(k) * fact((n + m) / 2 - k) * fact((n - m) / 2 - k));
                            coef.push(ck * norm);
                        }
                        keys.push((n, m));
                        rad.push((start, coef.len() - start, m));
                        rad.len() - 1
                    }
                };
                (ri, c)
            })
            .collect();
        let mmax = nm.iter().map(|t| t.1).max().unwrap_or(0);
        let mut by_m: Vec<Vec<RadUsers>> = vec![Vec::new(); mmax + 1];
        for (i, r) in rad.iter().enumerate() {
            let users = terms
                .iter()
                .enumerate()
                .filter(|t| t.1 .0 == i)
                .map(|(j, t)| (j, t.1))
                .collect();
            by_m[r.2].push((i, users));
        }
        ZernBasis {
            coef,
            rad,
            terms,
            mmax,
            by_m,
        }
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.terms.len()
    }

    /// 点 (px, py) での各項を out に書く（stride おきに、out[0], out[stride], ...）
    pub fn eval(&self, px: f64, py: f64, out: &mut [f64], stride: usize) {
        let r2 = px * px + py * py;
        // (x + iy)^m = r^m (cos mθ + i sin mθ)
        let mut cm = [0.0f64; 64];
        let mut sm = [0.0f64; 64];
        cm[0] = 1.0;
        for m in 1..=self.mmax {
            cm[m] = cm[m - 1] * px - sm[m - 1] * py;
            sm[m] = sm[m - 1] * px + cm[m - 1] * py;
        }
        // cos と sin の組は隣り合うので、直前の項と同じ動径なら使い回す
        let (mut last, mut h) = (usize::MAX, 0.0);
        for (j, &(ri, cos)) in self.terms.iter().enumerate() {
            let (st, len, m) = self.rad[ri];
            if ri != last {
                let c = &self.coef[st..st + len];
                h = c[0];
                for &ck in &c[1..] {
                    h = h * r2 + ck;
                }
                last = ri;
            }
            out[j * stride] = h * if cos { cm[m] } else { sm[m] };
        }
    }
}

impl ZernBasis {
    /// 点の並び (px[i], py[i]) での各項を列優先で out に書く（項 j が out[j*ld..j*ld+len]、len = px.len() ≤ ld。
    /// 列の残り ld − len 個には触れない）。点についての演算は eval と同じ順なので、値は eval とビットまで同じ。
    /// 点の向きに回すのでベクトル化が効く
    pub fn eval_cols(&self, px: &[f64], py: &[f64], out: &mut [f64], ld: usize) {
        let len = px.len().min(py.len());
        assert!(len <= ld && out.len() >= ld * self.terms.len());
        let (px, py) = (&px[..len], &py[..len]);
        let r2: Vec<f64> = px.iter().zip(py).map(|(x, y)| x * x + y * y).collect();
        let mut cm = vec![1.0; len];
        let mut sm = vec![0.0; len];
        let mut h = vec![0.0; len];
        for (m, rads) in self.by_m.iter().enumerate() {
            if m > 0 {
                for i in 0..len {
                    let (c, s) = (cm[i], sm[i]);
                    cm[i] = c * px[i] - s * py[i];
                    sm[i] = s * px[i] + c * py[i];
                }
            }
            for (ri, users) in rads {
                let (st, ln, _) = self.rad[*ri];
                let c = &self.coef[st..st + ln];
                h.iter_mut().for_each(|v| *v = c[0]);
                for &ck in &c[1..] {
                    for (v, &q) in h.iter_mut().zip(&r2) {
                        *v = *v * q + ck;
                    }
                }
                for &(j, cos) in users {
                    let col = &mut out[j * ld..j * ld + len];
                    let a = if cos { &cm } else { &sm };
                    for ((o, &hv), &av) in col.iter_mut().zip(&h).zip(a) {
                        *o = hv * av;
                    }
                }
            }
        }
    }
}

/// 瞳の点での Zernike 項（Standard は OpticStudio の正規化、Fringe は正規化なし）
pub fn zernike_row(px: f64, py: f64, kind: ZernKind, nm: &[(usize, usize, bool)]) -> Vec<f64> {
    let b = ZernBasis::new(kind, nm);
    let mut out = vec![0.0; nm.len()];
    b.eval(px, py, &mut out, 1);
    out
}

/// Zernike の当てはめで点を分ける塊の大きさ
const ZCH: usize = 256;

/// 列優先の塊 a（列の長さ ld は 4 の倍数、列の数 np は 4 の倍数で、足りない所は 0）の AᵀA の下三角
/// （i < n、j ≤ i）を g（n×n、行優先）に書く。各要素は、点を 4 つずつに分けた 4 本の和を最後に
/// (s0 + s1) + (s2 + s3) で足す（和の順は列の組の大きさや CPU によらず同じ）。列の組（TU 列 × TW 列）ごとに
/// 足すので、読み込み 1 回あたりの積和が多い
#[inline(always)]
pub(super) fn gram_tiles<const TU: usize, const TW: usize>(
    a: &[f64],
    ld: usize,
    np: usize,
    n: usize,
    g: &mut [f64],
) {
    const L: usize = 4;
    let h = ld / L;
    let col = |k: usize| -> &[[f64; L]] { a[k * ld..(k + 1) * ld].as_chunks::<L>().0 };
    for bi in (0..np).step_by(TU) {
        let ci: [&[[f64; L]]; TU] = std::array::from_fn(|k| col(bi + k));
        for bj in (0..bi + TU).step_by(TW) {
            let cj: [&[[f64; L]]; TW] = std::array::from_fn(|k| col(bj + k));
            let mut acc = [[[0.0f64; L]; TW]; TU];
            // 長さを h に切りそろえておくと、下の添字の範囲の検査が消えてコンパイラが並べる（無いと 2 倍遅い）
            let (ci, cj) = (ci.map(|c| &c[..h]), cj.map(|c| &c[..h]));
            for q in 0..h {
                for u in 0..TU {
                    let x = &ci[u][q];
                    for w in 0..TW {
                        let y = &cj[w][q];
                        for l in 0..L {
                            acc[u][w][l] += x[l] * y[l];
                        }
                    }
                }
            }
            for (u, au) in acc.iter().enumerate() {
                let i = bi + u;
                if i >= n {
                    break;
                }
                for (w, s) in au.iter().enumerate() {
                    let j = bj + w;
                    if j > i {
                        break;
                    }
                    g[i * n + j] = (s[0] + s[1]) + (s[2] + s[3]);
                }
            }
        }
    }
}

/// [`gram_tiles`]。x86_64 で AVX があれば、同じ関数を AVX を使ってよいとしてコンパイルしたもの（4 列 × 2 列の組。
/// コンパイラが 256 ビットのレジスタに並べる。wasm の simd128 はビルドの設定で同じく並ぶ）。積と和は別の命令で
/// 順も同じなので、値はどれでも同じ
pub(super) fn gram_block(a: &[f64], ld: usize, np: usize, n: usize, g: &mut [f64]) {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx") {
            // 安全: AVX があることを上で確かめた
            return unsafe { gram_block_avx(a, ld, np, n, g) };
        }
    }
    gram_tiles::<2, 2>(a, ld, np, n, g)
}

/// [`gram_block`] の AVX を使ってよいとしてコンパイルしたもの
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
pub(super) fn gram_block_avx(a: &[f64], ld: usize, np: usize, n: usize, g: &mut [f64]) {
    gram_tiles::<4, 2>(a, ld, np, n, g)
}

/// Zernike の当てはめの 1 回分の和: 点の塊（ZCH 点）ごとに項を列優先で求め、残差 r = v − A c（c が None なら
/// r = v）と Aᵀr、c が None なら AᵀA も足す。塊の分け方と足す順は固定なので、値はスレッド数によらない。
/// A は塊の中だけに置く（全体を持って使い回すより、ページの確保がない分だけ速い。項を求める時間は小さい）
pub(super) fn zern_pass(
    basis: &ZernBasis,
    px: &[f64],
    py: &[f64],
    v: &[f64],
    c: Option<&[f64]>,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    use rayon::prelude::*;
    let n = basis.len();
    // 列の数と長さを 4 の倍数にして、足りない所は 0（積和に効かない）
    let np = n.div_ceil(4) * 4;
    let gram = c.is_none();
    let part = |k: usize| {
        let lo = k * ZCH;
        let hi = (lo + ZCH).min(v.len());
        let len = hi - lo;
        let ld = len.div_ceil(4) * 4;
        let mut a = vec![0.0; ld * np];
        basis.eval_cols(&px[lo..hi], &py[lo..hi], &mut a, ld);
        let mut r = v[lo..hi].to_vec();
        if let Some(c) = c {
            for (col, &cj) in a.chunks_exact(ld).zip(c) {
                r.iter_mut().zip(col).for_each(|(ri, z)| *ri -= z * cj);
            }
        }
        let atr: Vec<f64> = a
            .chunks_exact(ld)
            .take(n)
            .map(|col| dot4(col, &r))
            .collect();
        let mut g = Vec::new();
        if gram {
            g = vec![0.0; n * n];
            gram_block(&a, ld, np, n, &mut g);
        }
        (g, atr, r)
    };
    let nch = v.len().div_ceil(ZCH);
    let parts: Vec<_> = if lensbench_core::par::sequential() {
        (0..nch).map(part).collect()
    } else {
        (0..nch).into_par_iter().map(scoped(part)).collect()
    };
    let mut g = if gram { vec![0.0; n * n] } else { Vec::new() };
    let mut atr = vec![0.0; n];
    let mut r = Vec::with_capacity(v.len());
    for (pg, pa, pr) in parts {
        g.iter_mut().zip(&pg).for_each(|(x, y)| *x += y);
        atr.iter_mut().zip(&pa).for_each(|(x, y)| *x += y);
        r.extend(pr);
    }
    if gram {
        for i in 0..n {
            for j in 0..i {
                g[j * n + i] = g[i * n + j];
            }
        }
    }
    (g, atr, r)
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Zernike {
    pub coef: Vec<f64>,
    pub kind: ZernKind,
    pub pv: f64,
    pub pv_centroid: f64,
    pub rms_fit: f64,
    pub max_fit: f64,
    pub rms_chief: f64,
    pub rms_centroid: f64,
    pub variance: f64,
    pub strehl: f64,
}

/// OPD の Zernike 当てはめ [波]（波面マップと同じ n×n の格子で最小二乗。OpticStudio の Zernike Coefficients）
pub fn zernike(
    s: &System,
    field: FieldSel,
    w: Option<usize>,
    kind: ZernKind,
    nterms: usize,
    n: usize,
    ref_primary: bool,
) -> Result<Zernike> {
    let grid = PupilGrid::WavefrontMap(n);
    grid.check()?;
    if nterms == 0 || (kind == ZernKind::Fringe && nterms > 37) || nterms > 2016 {
        invalid!("{nterms} terms (Fringe 1 to 37, Standard 1 to 2016)");
    }
    let r = PupilFunction::trace(s, field, w, grid, ref_primary)?;
    let nm = zern_nm(kind, nterms);
    let basis = ZernBasis::new(kind, &nm);
    let (mut qx, mut qy, mut v) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..r.len() {
        if let Some(o) = r.get(i) {
            qx.push(r.samples.px[i]);
            qy.push(r.samples.py[i]);
            v.push(o);
        }
    }
    if v.is_empty() {
        invalid!(code = lensbench_core::error::code::ALL_VIGNETTED; "no valid rays (all vignetted)");
    }
    if v.len() < nm.len().max(3) {
        invalid!("{} valid rays are fewer than {} terms", v.len(), nm.len());
    }
    // 正規方程式 (AᵀA) c = Aᵀv を点の塊ごとに足し（塊の分け方と足す順は固定なので、値はスレッド数によらない）、
    // Cholesky で解いて残差で 1 回補正する（Zernike の項は円の上でほぼ直交なので条件数は小さい）
    let nt = nm.len();
    let m = v.len();
    let (g, atv, _) = zern_pass(&basis, &qx, &qy, &v, None);
    // 正規方程式は条件数の 2 乗で効くので、Cholesky が通らない・補正が大きい（項数が格子に比べて多いなど）
    // ときは QR で解き直す
    let normal = Cholesky::new(&g, nt).and_then(|ch| {
        let mut c = ch.solve(&atv);
        let (_, atr, _) = zern_pass(&basis, &qx, &qy, &v, Some(&c));
        let dc = ch.solve(&atr);
        let cn = c.iter().map(|x| x * x).sum::<f64>().sqrt();
        let dn = dc.iter().map(|x| x * x).sum::<f64>().sqrt();
        c.iter_mut().zip(&dc).for_each(|(x, d)| *x += d);
        (dn.is_finite() && dn <= 1e-6 * cn.max(1e-300)).then_some(c)
    });
    let c = match normal {
        Some(c) => c,
        None => {
            let mut q = vec![0.0; m * nt];
            let mut row = vec![0.0; nt];
            for (i, (&x, &y)) in qx.iter().zip(&qy).enumerate() {
                basis.eval(x, y, &mut row, 1);
                for (j, &z) in row.iter().enumerate() {
                    q[j * m + i] = z;
                }
            }
            lstsq_cols(q, m, &v)
        }
    };
    let (_, _, res) = zern_pass(&basis, &qx, &qy, &v, Some(&c));
    // 重心基準: 1・px・py の最小二乗（QR、列優先）
    let q3: Vec<f64> = std::iter::repeat_n(1.0, m)
        .chain(qx.iter().copied())
        .chain(qy.iter().copied())
        .collect();
    let t = lstsq_cols(q3, m, &v);
    let vt: Vec<f64> = qx
        .iter()
        .zip(&qy)
        .zip(&v)
        .map(|((&x, &y), vi)| vi - (t[0] + x * t[1] + y * t[2]))
        .collect();
    let nn = v.len() as f64;
    let ptp = |x: &[f64]| {
        x.iter().cloned().fold(f64::MIN, f64::max) - x.iter().cloned().fold(f64::MAX, f64::min)
    };
    let mean = v.iter().sum::<f64>() / nn;
    let rms_c = (vt.iter().map(|x| x * x).sum::<f64>() / nn).sqrt();
    Ok(Zernike {
        coef: c,
        kind,
        pv: ptp(&v),
        pv_centroid: ptp(&vt),
        rms_fit: (res.iter().map(|x| x * x).sum::<f64>() / nn).sqrt(),
        max_fit: res.iter().map(|x| x.abs()).fold(0.0, f64::max),
        rms_chief: (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nn).sqrt(),
        rms_centroid: rms_c,
        variance: rms_c * rms_c,
        strehl: (-(2.0 * std::f64::consts::PI * rms_c).powi(2)).exp(),
    })
}
