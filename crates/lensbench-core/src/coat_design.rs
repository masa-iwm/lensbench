//! 分光の曲線（反射率・透過率）に合う多層膜を当てはめる（暗号化された膜の代わりの「近似の膜」を作る）。
//!
//! 材料（複素屈折率の表）を外側から順に繰り返し並べ、層の厚さだけを Levenberg–Marquardt で合わせる。
//! 層の数ごとに乱数の初期値を何通りか試し、いちばん合うものを残す。同じ曲線を出す設計は一つではないので、
//! 合わせた曲線（波長・角度・偏光）の外の量（位相、別の角度、硝材の側から入るとき）は元の膜と違いうる。
//!
//! 係数は [`crate::coating::stack`]（OpticStudio の約束、パワー規格化）で、R = |r|²、T = |t|²。

use crate::coating::{interp, stack, C64};
use crate::error::{Error, Result};

/// 合わせる量
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quantity {
    R,
    T,
}

/// 偏光（Avg は s と p の平均）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pol {
    S,
    P,
    Avg,
}

/// 曲線の 1 点
#[derive(Debug, Clone, Copy)]
pub struct Target {
    /// 波長 [µm]
    pub wl: f64,
    /// 入射角 [deg]（入射側の媒質で）
    pub angle_deg: f64,
    pub pol: Pol,
    pub q: Quantity,
    /// 値（0〜1）
    pub value: f64,
    /// 重み
    pub weight: f64,
}

/// 膜の材料: 名前と (波長 [µm], n, k) の表（波長の昇順）
#[derive(Debug, Clone)]
pub struct Material {
    pub name: String,
    pub rows: Vec<[f64; 3]>,
}

impl Material {
    pub fn index(&self, wl: f64) -> C64 {
        let xs: Vec<f64> = self.rows.iter().map(|r| r[0]).collect();
        let n: Vec<f64> = self.rows.iter().map(|r| r[1]).collect();
        let k: Vec<f64> = self.rows.iter().map(|r| r[2]).collect();
        C64::new(interp(wl, &xs, &n), interp(wl, &xs, &k))
    }
}

/// 当てはめの設定
#[derive(Debug, Clone)]
pub struct Options {
    /// 入射側の媒質の屈折率（空気 1.0）
    pub n0: f64,
    /// 材料（外側＝入射側から順に繰り返す。例: [低屈折率, 高屈折率]）
    pub materials: Vec<Material>,
    /// 試す層の数（最小、最大）
    pub layers: (usize, usize),
    /// 層の数ごとの初期値の数
    pub starts: usize,
    /// 層の厚さの上限 [µm]
    pub d_max: f64,
    /// 乱数の種
    pub seed: u64,
    /// LM の反復の上限
    pub iters: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            n0: 1.0,
            materials: Vec::new(),
            layers: (1, 10),
            starts: 20,
            d_max: 0.6,
            seed: 1,
            iters: 200,
        }
    }
}

/// 当てはめた多層膜
#[derive(Debug, Clone)]
pub struct Design {
    /// (材料の番号, 厚さ [µm])、外側から
    pub layers: Vec<(usize, f64)>,
    /// 重み付きの残差の RMS
    pub rms: f64,
    /// 残差の絶対値の最大（重みなし）
    pub max_err: f64,
}

/// 基板の屈折率（波長 [µm] → n）
pub type Substrate<'a> = &'a dyn Fn(f64) -> C64;

/// 1 点の値
fn eval(t: &Target, layers: &[(C64, f64)], ns: C64, n0: f64) -> f64 {
    let (rs, rp, ts, tp) = stack(n0, layers, ns, t.wl, t.angle_deg.to_radians());
    let (s, p) = match t.q {
        Quantity::R => (rs.norm_sqr(), rp.norm_sqr()),
        Quantity::T => (ts.norm_sqr(), tp.norm_sqr()),
    };
    match t.pol {
        Pol::S => s,
        Pol::P => p,
        Pol::Avg => 0.5 * (s + p),
    }
}

/// 点ごとの材料・基板の屈折率を先に求めておく
struct Prep {
    /// [点][材料]
    n: Vec<Vec<C64>>,
    ns: Vec<C64>,
}

fn residuals(ts: &[Target], pre: &Prep, mats: &[usize], d: &[f64], n0: f64, out: &mut [f64]) {
    let mut layers: Vec<(C64, f64)> = vec![(C64::new(1.0, 0.0), 0.0); d.len()];
    for (i, t) in ts.iter().enumerate() {
        for (j, l) in layers.iter_mut().enumerate() {
            *l = (pre.n[i][mats[j]], d[j]);
        }
        out[i] = t.weight.sqrt() * (eval(t, &layers, pre.ns[i], n0) - t.value);
    }
}

/// 小さな連立一次方程式（ガウスの消去、部分ピボット）。解けなければ None
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() < 1e-300 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..n {
            let f = a[r][c] / a[c][c];
            for k in c..n {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|k| a[r][k] * x[k]).sum();
        x[r] = (b[r] - s) / a[r][r];
    }
    Some(x)
}

/// 厚さ d から LM で合わせる（d は [0, d_max] に収める）→ (d, 残差の二乗和)
fn lm(ts: &[Target], pre: &Prep, mats: &[usize], mut d: Vec<f64>, o: &Options) -> (Vec<f64>, f64) {
    let (m, n) = (ts.len(), d.len());
    let mut r = vec![0.0; m];
    residuals(ts, pre, mats, &d, o.n0, &mut r);
    let mut cost: f64 = r.iter().map(|x| x * x).sum();
    let mut lam = 1e-3;
    let mut jac = vec![vec![0.0; n]; m];
    let mut rr = vec![0.0; m];
    for _ in 0..o.iters {
        // 数値微分（前進差分）
        for j in 0..n {
            let h = 1e-6;
            let mut dd = d.clone();
            dd[j] += h;
            residuals(ts, pre, mats, &dd, o.n0, &mut rr);
            for i in 0..m {
                jac[i][j] = (rr[i] - r[i]) / h;
            }
        }
        let mut jtj = vec![vec![0.0; n]; n];
        let mut jtr = vec![0.0; n];
        for i in 0..m {
            for a in 0..n {
                jtr[a] -= jac[i][a] * r[i];
                for b in 0..n {
                    jtj[a][b] += jac[i][a] * jac[i][b];
                }
            }
        }
        let mut improved = false;
        for _ in 0..12 {
            let mut a = jtj.clone();
            for k in 0..n {
                a[k][k] += lam * (jtj[k][k] + 1e-12);
            }
            let Some(step) = solve(a, jtr.clone()) else {
                lam *= 10.0;
                continue;
            };
            let nd: Vec<f64> = d
                .iter()
                .zip(&step)
                .map(|(x, s)| (x + s).clamp(0.0, o.d_max))
                .collect();
            residuals(ts, pre, mats, &nd, o.n0, &mut rr);
            let c: f64 = rr.iter().map(|x| x * x).sum();
            if c < cost {
                let rel = (cost - c) / cost.max(1e-300);
                d = nd;
                std::mem::swap(&mut r, &mut rr);
                cost = c;
                lam = (lam * 0.3).max(1e-12);
                improved = rel > 1e-12;
                break;
            }
            lam *= 10.0;
        }
        if !improved {
            break;
        }
    }
    (d, cost)
}

/// 決まった乱数（xorshift）
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// 層の数ごとの最良（層の数の昇順）。最後の要素がいちばん層の多いもの
pub fn fit_all(targets: &[Target], substrate: Substrate, o: &Options) -> Result<Vec<Design>> {
    if targets.is_empty() {
        return Err(Error::Invalid("合わせる点がない".into()));
    }
    if o.materials.is_empty() {
        return Err(Error::Invalid("材料がない".into()));
    }
    let (lo, hi) = (o.layers.0.max(1), o.layers.1.max(o.layers.0.max(1)));
    let pre = Prep {
        n: targets
            .iter()
            .map(|t| o.materials.iter().map(|m| m.index(t.wl)).collect())
            .collect(),
        ns: targets.iter().map(|t| substrate(t.wl)).collect(),
    };
    let wsum: f64 = targets.iter().map(|t| t.weight).sum::<f64>().max(1e-300);
    // 主な波長（中央）での 1/4 波長の厚さを初期値の目安にする
    let mut wls: Vec<f64> = targets.iter().map(|t| t.wl).collect();
    wls.sort_by(f64::total_cmp);
    let wc = wls[wls.len() / 2];
    let mut rng = Rng(o.seed.max(1).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut out: Vec<Design> = Vec::new();
    // 層の数ごとの最良の厚さ（層の数 → d）。次の層の数の初期値に使う（針の挿入）
    let mut prev: Vec<(usize, Vec<f64>)> = Vec::new();
    for nl in lo..=hi {
        let mats: Vec<usize> = (0..nl).map(|j| j % o.materials.len()).collect();
        let qw: Vec<f64> = mats
            .iter()
            .map(|&k| wc / (4.0 * o.materials[k].index(wc).re))
            .collect();
        let mut seeds: Vec<Vec<f64>> = vec![qw.clone()];
        for _ in 1..o.starts.max(1) {
            seeds.push(
                qw.iter()
                    .map(|q| (q * (0.1 + 2.4 * rng.next())).min(o.d_max))
                    .collect(),
            );
        }
        let eps = 0.002;
        // 1 層少ない最良に、基板の側へ薄い層を足す
        if let Some((_, d)) = prev.iter().find(|(n, _)| *n + 1 == nl) {
            let mut x = d.clone();
            x.push(eps);
            seeds.push(x);
        }
        // 2 層少ない最良の層を 1 つ割って、間に別の材料の薄い層（針）を入れる（材料の並びは変わらない）
        if o.materials.len() == 2 {
            if let Some((_, d)) = prev.iter().find(|(n, _)| *n + 2 == nl) {
                for j in 0..d.len() {
                    for f in [0.25, 0.5, 0.75] {
                        let mut x = d[..j].to_vec();
                        x.extend([d[j] * f, eps, d[j] * (1.0 - f)]);
                        x.extend_from_slice(&d[j + 1..]);
                        seeds.push(x);
                    }
                }
            }
        }
        let mut best: Option<(Vec<f64>, f64)> = None;
        for d0 in seeds {
            let (d, c) = lm(targets, &pre, &mats, d0, o);
            if best.as_ref().map_or(true, |b| c < b.1) {
                best = Some((d, c));
            }
        }
        let (d, c) = best.unwrap();
        prev.push((nl, d.clone()));
        let mut r = vec![0.0; targets.len()];
        residuals(targets, &pre, &mats, &d, o.n0, &mut r);
        let max_err = r
            .iter()
            .zip(targets)
            .map(|(x, t)| {
                if t.weight > 0.0 {
                    x.abs() / t.weight.sqrt()
                } else {
                    0.0
                }
            })
            .fold(0.0, f64::max);
        out.push(Design {
            layers: mats.into_iter().zip(d).collect(),
            rms: (c / wsum).sqrt(),
            max_err,
        });
    }
    Ok(out)
}

/// 層の数の中から選ぶ: いちばん合うものの RMS の (1 + slack) 倍以内で、層のいちばん少ないもの
pub fn pick(all: &[Design], slack: f64) -> Option<&Design> {
    let best = all.iter().map(|d| d.rms).fold(f64::INFINITY, f64::min);
    all.iter().find(|d| d.rms <= best * (1.0 + slack) + 1e-15)
}

/// 膜ファイル（COATING.DAT）の形式で書く: `COAT name` と層（絶対の厚さ [µm]）。
/// with_mate の材料は `MATE` の表も書く（膜ファイルにない材料）
pub fn format_lbcs(
    name: &str,
    d: &Design,
    mats: &[Material],
    with_mate: &[usize],
    note: &str,
) -> String {
    let mut s = String::new();
    for l in note.lines() {
        s += &format!("! {l}\n");
    }
    s += &format!("COAT {name}\n");
    for &(k, t) in &d.layers {
        if t > 0.0 {
            s += &format!("{} {:.9} 1\n", mats[k].name, t);
        }
    }
    for &k in with_mate {
        s += &format!("MATE {}\n", mats[k].name);
        for r in &mats[k].rows {
            s += &format!("{} {} {}\n", r[0], r[1], r[2]);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(name: &str, n: f64) -> Material {
        Material {
            name: name.into(),
            rows: vec![[0.3, n, 0.0], [2.0, n, 0.0]],
        }
    }

    #[test]
    fn recovers_a_two_layer_ar_from_its_reflectance() {
        // 既知の 2 層の膜の反射率（0° と 45° の s・p）から、同じ曲線の膜を当てはめる
        let (l, h) = (flat("L", 1.38), flat("H", 2.1));
        let truth = [(C64::new(1.38, 0.0), 0.1), (C64::new(2.1, 0.0), 0.065)];
        let ns = C64::new(1.52, 0.0);
        let mut ts = Vec::new();
        for i in 0..=30 {
            let wl = 0.4 + 0.3 * i as f64 / 30.0;
            for (a, pol) in [(0.0, Pol::Avg), (45.0, Pol::S), (45.0, Pol::P)] {
                let t = Target {
                    wl,
                    angle_deg: a,
                    pol,
                    q: Quantity::R,
                    value: 0.0,
                    weight: 1.0,
                };
                ts.push(Target {
                    value: eval(&t, &truth, ns, 1.0),
                    ..t
                });
            }
        }
        let o = Options {
            materials: vec![l.clone(), h.clone()],
            layers: (1, 3),
            starts: 8,
            ..Default::default()
        };
        let all = fit_all(&ts, &|_| ns, &o).unwrap();
        let d = pick(&all, 0.5).unwrap();
        assert_eq!(d.layers.len(), 2, "{all:?}");
        assert!(d.max_err < 1e-6, "{d:?}");
        assert!(
            (d.layers[0].1 - 0.1).abs() < 1e-4 && (d.layers[1].1 - 0.065).abs() < 1e-4,
            "{d:?}"
        );
        // 書いた膜ファイルを読むと同じ層になる
        let txt = format_lbcs("X", d, &o.materials, &[0, 1], "test");
        let f = crate::coating::CoatingFile::parse(&txt).unwrap();
        assert_eq!(f.coat["X"].len(), 2);
        assert!(f.coat["X"][0].absolute && f.mate.contains_key("H"));
    }
}
