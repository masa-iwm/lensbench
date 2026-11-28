//! 面の散乱と仕上げ（非順次の追跡の段 3）: 散乱のモデル（Lambert・Gauss・ABg）と、面・玉の縁・機構部品・絞りの板に
//! 付ける性質（[`Finish`]）。
//!
//! 散乱の分布は方向余弦の空間（面の接平面への射影 β、|β| < 1）で書く。BSDF を f(β) とすると、散乱する光の割合は
//! ∫ f cosθ dΩ = ∫ f d²β（単位円の中）。鏡面の方向（反射・屈折した光線の方向）の射影を β0 とする。
//!
//! - Lambert: f は一定（放射輝度が一様、強度は cosθ に比例）。β は単位円の中で一様
//! - Gauss: f ∝ exp(−|β − β0|² / σ²)。単位円の外へはみ出す分は円の中で規格化する
//! - ABg: f = A / (B + |β − β0|^g)。散乱の割合（TIS）は f を単位円の中で積分した値で、入射角（|β0|）で変わる。
//!   `fraction` は使わない
//!
//! Lambert・Gauss の散乱の割合は `fraction`（入射角に依らない）。どのモデルも散乱しない残りは鏡面の方向へ進む。

use crate::{invalid, Result};
use serde::{Deserialize, Serialize};

/// 散乱のモデル
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ScatterModel {
    /// 放射輝度が一様（強度は cosθ に比例）
    #[default]
    Lambert,
    /// 鏡面の方向のまわりの Gauss（`sigma`）
    Gauss,
    /// ABg（`a`・`b`・`g`）
    Abg,
}

/// 散乱する側
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ScatterSide {
    /// 反射と透過の両方
    #[default]
    Both,
    /// 反射した光だけ
    Reflect,
    /// 透過した光だけ
    Transmit,
}

impl ScatterSide {
    fn is_both(&self) -> bool {
        *self == ScatterSide::Both
    }
}

fn one() -> f64 {
    1.0
}
fn is_one(x: &f64) -> bool {
    *x == 1.0
}
fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

/// 面の散乱
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scatter {
    pub model: ScatterModel,
    /// 散乱する割合（Lambert・Gauss。0〜1、省略で 1）。ABg は使わない（BSDF の積分で決まる）
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub fraction: f64,
    /// Gauss の幅 σ（方向余弦の空間）
    #[serde(default, skip_serializing_if = "is_zero")]
    pub sigma: f64,
    /// ABg の A・B・g（BSDF = A / (B + |β − β0|^g) [1/sr]）
    #[serde(default, skip_serializing_if = "is_zero")]
    pub a: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub b: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub g: f64,
    /// 散乱する側（省略で反射と透過の両方）
    #[serde(default, skip_serializing_if = "ScatterSide::is_both")]
    pub on: ScatterSide,
    /// 散乱した光線が偏光を失うか（true: 向きが一様な直線偏光、false: 鏡面の方向の電場を散乱した方向に垂直な面へ
    /// 射影して保つ）。省略で Lambert は失い、Gauss・ABg は保つ
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depolarize: Option<bool>,
}

impl Default for Scatter {
    fn default() -> Self {
        Scatter {
            model: ScatterModel::Lambert,
            fraction: 1.0,
            sigma: 0.0,
            a: 0.0,
            b: 0.0,
            g: 0.0,
            on: ScatterSide::Both,
            depolarize: None,
        }
    }
}

impl Scatter {
    /// Lambert（散乱の割合 fraction）
    pub fn lambert(fraction: f64) -> Scatter {
        Scatter {
            fraction,
            ..Default::default()
        }
    }
    /// Gauss（幅 sigma、散乱の割合 fraction）
    pub fn gauss(sigma: f64, fraction: f64) -> Scatter {
        Scatter {
            model: ScatterModel::Gauss,
            sigma,
            fraction,
            ..Default::default()
        }
    }
    /// ABg
    pub fn abg(a: f64, b: f64, g: f64) -> Scatter {
        Scatter {
            model: ScatterModel::Abg,
            a,
            b,
            g,
            ..Default::default()
        }
    }

    /// 散乱した光線が偏光を失うか（[`Scatter::depolarize`]、省略で Lambert だけ）
    pub fn depolarizes(&self) -> bool {
        self.depolarize
            .unwrap_or(self.model == ScatterModel::Lambert)
    }

    /// 反射（reflect）・透過した光に効くか
    pub fn applies(&self, reflect: bool) -> bool {
        match self.on {
            ScatterSide::Both => true,
            ScatterSide::Reflect => reflect,
            ScatterSide::Transmit => !reflect,
        }
    }

    /// 値の確かめ
    pub fn check(&self) -> Result<()> {
        let f = self.fraction;
        if !(0.0..=1.0).contains(&f) {
            invalid!("scatter: fraction {f} (0 to 1)");
        }
        match self.model {
            ScatterModel::Lambert => {}
            ScatterModel::Gauss => {
                let s = self.sigma;
                if !(s.is_finite() && s > 0.0) {
                    invalid!("scatter: gauss sigma {s} (must be positive)");
                }
            }
            ScatterModel::Abg => {
                let (a, b, g) = (self.a, self.b, self.g);
                if !(a.is_finite() && a >= 0.0) {
                    invalid!("scatter: abg A {a} (0 or more)");
                }
                if !(b.is_finite() && b > 0.0) {
                    invalid!("scatter: abg B {b} (must be positive)");
                }
                if !(g.is_finite() && (0.0..=10.0).contains(&g)) {
                    invalid!("scatter: abg g {g} (0 to 10)");
                }
            }
        }
        Ok(())
    }

    /// 散乱の分布を用意する（ABg は散乱の割合の表を作る）。ABg の割合がどこかの入射角で 1 を超えれば誤り
    pub fn prepare(&self) -> Result<Bsdf> {
        self.check()?;
        Ok(match self.model {
            ScatterModel::Lambert => Bsdf::Lambert {
                fraction: self.fraction,
            },
            ScatterModel::Gauss => Bsdf::Gauss {
                fraction: self.fraction,
                sigma: self.sigma,
            },
            ScatterModel::Abg => {
                let t = AbgTable::new(self.a, self.b, self.g);
                let worst = t.tis.iter().copied().fold(0.0, f64::max);
                if worst > 1.0 {
                    let (a, b, g) = (self.a, self.b, self.g);
                    invalid!("scatter: abg A {a} B {b} g {g} scatters {worst:.4} of the light (more than all of it)");
                }
                Bsdf::Abg(Box::new(t))
            }
        })
    }
}

/// 面などの仕上げ: 散乱と、光を通さない物の反射率。対象は系の面（`surface`）、その面を持つ玉の縁（`edge`）、機構部品
/// （`mech`、名前）のどれか 1 つ
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Finish {
    /// 系の面の番号（光学面。空気中の絞りの面なら開口の外の板）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<usize>,
    /// この番号の面を持つ玉の縁（縁の輪と円柱）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<usize>,
    /// 機構部品の名前（壁）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mech: Option<String>,
    /// 光を通さない面の反射率（0〜1。残りは吸収）。機構部品・絞りの板は省略で 0（黒）。玉の縁に書くと縁は光を
    /// 通さない（塗った縁）。光学面には書けない（反射は膜込みの Fresnel）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflect: Option<f64>,
    /// 散乱（光学面・透明な縁では反射と透過の光、光を通さない面では反射した光）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scatter: Option<Scatter>,
}

/// 仕上げの対象
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishAt<'a> {
    Surface(usize),
    Edge(usize),
    Mech(&'a str),
}

impl Finish {
    /// 対象（ちょうど 1 つでなければ誤り）
    pub fn at(&self) -> Result<FinishAt<'_>> {
        let n = usize::from(self.surface.is_some())
            + usize::from(self.edge.is_some())
            + usize::from(self.mech.is_some());
        if n != 1 {
            invalid!("finish: give one of surface, edge and mech ({n} given)");
        }
        Ok(match (self.surface, self.edge, &self.mech) {
            (Some(j), _, _) => FinishAt::Surface(j),
            (_, Some(j), _) => FinishAt::Edge(j),
            (_, _, Some(m)) => FinishAt::Mech(m),
            _ => unreachable!("one target"),
        })
    }

    /// 値の確かめ
    pub fn check(&self) -> Result<()> {
        self.at()?;
        if let Some(r) = self.reflect {
            if !(0.0..=1.0).contains(&r) {
                invalid!("finish: reflect {r} (0 to 1)");
            }
        }
        if let Some(s) = &self.scatter {
            s.check()?;
        }
        if self.reflect.is_none() && self.scatter.is_none() && self.mech.is_none() {
            invalid!("finish: give reflect or scatter");
        }
        Ok(())
    }
}

/// 用意した散乱の分布（[`Scatter::prepare`]）
#[derive(Debug, Clone)]
pub enum Bsdf {
    Lambert { fraction: f64 },
    Gauss { fraction: f64, sigma: f64 },
    Abg(Box<AbgTable>),
}

/// 単位円からはみ出した点を引き直す回数の上限
const MAX_TRIES: u32 = 64;

impl Bsdf {
    /// 鏡面の方向の射影の大きさ b0 = |β0| での散乱の割合
    pub fn tis(&self, b0: f64) -> f64 {
        match self {
            Bsdf::Lambert { fraction } | Bsdf::Gauss { fraction, .. } => *fraction,
            Bsdf::Abg(t) => t.tis_at(b0),
        }
    }

    /// 散乱の方向の射影 β を引く（鏡面の方向の射影 beta0、[0, 1) の一様な数を返す u）。単位円の外の点は引き直し、
    /// 上限まで外なら None
    pub fn sample(&self, beta0: [f64; 2], u: &mut dyn FnMut() -> f64) -> Option<[f64; 2]> {
        for _ in 0..MAX_TRIES {
            let b = match self {
                Bsdf::Lambert { .. } => {
                    let r = u().sqrt();
                    let p = std::f64::consts::TAU * u();
                    return Some([r * p.cos(), r * p.sin()]);
                }
                Bsdf::Gauss { sigma, .. } => {
                    // exp(−r²/σ²) は軸ごとの標準偏差 σ/√2 の正規分布（Box–Muller）
                    let s = sigma / std::f64::consts::SQRT_2;
                    let r = s * (-2.0 * (1.0 - u()).ln()).sqrt();
                    let p = std::f64::consts::TAU * u();
                    [beta0[0] + r * p.cos(), beta0[1] + r * p.sin()]
                }
                Bsdf::Abg(t) => {
                    let b0 = beta0[0].hypot(beta0[1]);
                    let r = t.radius(u(), 1.0 + b0);
                    let p = std::f64::consts::TAU * u();
                    [beta0[0] + r * p.cos(), beta0[1] + r * p.sin()]
                }
            };
            if b[0] * b[0] + b[1] * b[1] < 1.0 {
                return Some(b);
            }
        }
        None
    }

    /// BSDF の値 [1/sr]（散乱の方向の射影 beta、鏡面の方向の射影 beta0、単位円の外は 0）。Lambert・Gauss は散乱の
    /// 割合で規格化した値
    pub fn value(&self, beta: [f64; 2], beta0: [f64; 2]) -> f64 {
        if beta[0] * beta[0] + beta[1] * beta[1] >= 1.0 {
            return 0.0;
        }
        let d = (beta[0] - beta0[0]).hypot(beta[1] - beta0[1]);
        match self {
            Bsdf::Lambert { fraction } => fraction / std::f64::consts::PI,
            Bsdf::Gauss { fraction, sigma } => {
                let b0 = beta0[0].hypot(beta0[1]);
                fraction * (-(d * d) / (sigma * sigma)).exp() / gauss_norm(*sigma, b0)
            }
            Bsdf::Abg(t) => t.a / (t.b + d.powf(t.g)),
        }
    }
}

/// exp(−|β − β0|²/σ²) の単位円の中の積分（|β0| = b0）
pub fn gauss_norm(sigma: f64, b0: f64) -> f64 {
    let f = |r: f64| 0.5 * sigma * sigma * (1.0 - (-(r * r) / (sigma * sigma)).exp());
    circle_integral(&f, b0)
}

/// 中心から半径 r までの積分が F(r)（角度ごと、2π 倍しない）の分布を、中心 b0 から単位円の中で積分する:
/// ∫₀^{2π} F(r_max(φ)) dφ（r_max は中心から単位円までの距離）
fn circle_integral(f: &dyn Fn(f64) -> f64, b0: f64) -> f64 {
    if b0 <= 0.0 {
        return std::f64::consts::TAU * f(1.0);
    }
    // φ ∈ [0, π] を 2 倍（対称）。Simpson
    let n = 512;
    let h = std::f64::consts::PI / n as f64;
    let rmax = |p: f64| {
        let (s, c) = p.sin_cos();
        -b0 * c + (1.0 - b0 * b0 * s * s).max(0.0).sqrt()
    };
    let mut sum = f(rmax(0.0)) + f(rmax(std::f64::consts::PI));
    for i in 1..n {
        let w = if i % 2 == 1 { 4.0 } else { 2.0 };
        sum += w * f(rmax(i as f64 * h));
    }
    2.0 * sum * h / 3.0
}

/// ABg の表: 中心からの距離 r までの積分 F(r) = ∫₀^r s·A/(B + s^g) ds と、入射の |β0| ごとの散乱の割合
#[derive(Debug, Clone)]
pub struct AbgTable {
    pub a: f64,
    pub b: f64,
    pub g: f64,
    r: Vec<f64>,
    cdf: Vec<f64>,
    /// |β0| = i / (n − 1) の散乱の割合
    pub tis: Vec<f64>,
}

/// F(r) の表の点の数
const ABG_POINTS: usize = 4096;
/// 散乱の割合の表の点の数
const ABG_TIS_POINTS: usize = 257;

impl AbgTable {
    pub fn new(a: f64, b: f64, g: f64) -> AbgTable {
        let f = |s: f64| s * a / (b + s.powf(g));
        // 0 の近くは f ≈ s·A/B。目の細かさは分布の幅 B^(1/g) より十分小さい所から
        let scale = if g > 0.0 { b.powf(1.0 / g) } else { 1.0 };
        let r0 = (1e-4 * scale).clamp(1e-300, 1e-6);
        let rmax = 2.0;
        let mut r = Vec::with_capacity(ABG_POINTS + 1);
        let mut cdf = Vec::with_capacity(ABG_POINTS + 1);
        r.push(0.0);
        cdf.push(0.0);
        r.push(r0);
        cdf.push(0.5 * a / (b + r0.powf(g)) * r0 * r0);
        let q = (rmax / r0).powf(1.0 / (ABG_POINTS - 1) as f64);
        let mut x = r0;
        for i in 1..ABG_POINTS {
            let y = if i == ABG_POINTS - 1 { rmax } else { x * q };
            let m = 0.5 * (x + y);
            let c = cdf[cdf.len() - 1] + (y - x) / 6.0 * (f(x) + 4.0 * f(m) + f(y));
            r.push(y);
            cdf.push(c);
            x = y;
        }
        let mut t = AbgTable {
            a,
            b,
            g,
            r,
            cdf,
            tis: Vec::new(),
        };
        t.tis = (0..ABG_TIS_POINTS)
            .map(|i| {
                let b0 = i as f64 / (ABG_TIS_POINTS - 1) as f64;
                circle_integral(&|x: f64| t.f(x), b0.min(1.0 - 1e-12))
            })
            .collect();
        t
    }

    /// F(r)（表の間は直線）
    pub fn f(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, self.r[self.r.len() - 1]);
        let i = self
            .r
            .partition_point(|&v| v <= x)
            .clamp(1, self.r.len() - 1);
        let (x0, x1) = (self.r[i - 1], self.r[i]);
        let (c0, c1) = (self.cdf[i - 1], self.cdf[i]);
        if x1 <= x0 {
            return c0;
        }
        c0 + (c1 - c0) * (x - x0) / (x1 - x0)
    }

    /// F の逆: F(r) = u·F(rmax) の r
    fn radius(&self, u: f64, rmax: f64) -> f64 {
        let target = u * self.f(rmax);
        let i = self
            .cdf
            .partition_point(|&v| v <= target)
            .clamp(1, self.cdf.len() - 1);
        let (c0, c1) = (self.cdf[i - 1], self.cdf[i]);
        let (x0, x1) = (self.r[i - 1], self.r[i]);
        if c1 <= c0 {
            return x0;
        }
        (x0 + (x1 - x0) * (target - c0) / (c1 - c0)).min(rmax)
    }

    /// |β0| = b0 の散乱の割合（表の間は直線）
    pub fn tis_at(&self, b0: f64) -> f64 {
        let n = self.tis.len() - 1;
        let x = b0.clamp(0.0, 1.0) * n as f64;
        let i = (x as usize).min(n - 1);
        let w = x - i as f64;
        self.tis[i] * (1.0 - w) + self.tis[i + 1] * w
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::CounterRng;

    #[test]
    fn abg_tis_matches_the_closed_form_for_g_2() {
        // g = 2、垂直入射: TIS = π A ln(1 + 1/B)
        for (a, b) in [(1e-3, 1e-4), (0.05, 0.01), (0.2, 1.0)] {
            let t = AbgTable::new(a, b, 2.0);
            let want = std::f64::consts::PI * a * (1.0 + 1.0 / b).ln();
            let got = t.tis_at(0.0);
            assert!((got / want - 1.0).abs() < 1e-6, "{a} {b}: {got} {want}");
        }
    }

    #[test]
    fn abg_tis_off_axis_by_brute_force() {
        let t = AbgTable::new(0.01, 0.003, 1.5);
        for b0 in [0.3, 0.7, 0.95] {
            // 単位円の中の格子で積分
            let n = 2000;
            let h = 2.0 / n as f64;
            let mut s = 0.0;
            for i in 0..n {
                for j in 0..n {
                    let x = -1.0 + (i as f64 + 0.5) * h;
                    let y = -1.0 + (j as f64 + 0.5) * h;
                    if x * x + y * y < 1.0 {
                        let d = (x - b0).hypot(y);
                        s += 0.01 / (0.003 + d.powf(1.5)) * h * h;
                    }
                }
            }
            let got = t.tis_at(b0);
            assert!((got / s - 1.0).abs() < 2e-3, "{b0}: {got} {s}");
        }
    }

    #[test]
    fn too_much_abg_is_an_error() {
        assert!(Scatter::abg(1.0, 0.01, 2.0).prepare().is_err());
        assert!(Scatter::abg(1e-3, 1e-4, 2.0).prepare().is_ok());
        assert!(Scatter::gauss(0.0, 0.5).prepare().is_err());
        assert!(Scatter::lambert(1.5).prepare().is_err());
    }

    /// 引いた点の分布が BSDF に比例する（半径の帯ごとの数を BSDF の積分と比べる）
    fn check_samples(bsdf: &Bsdf, beta0: [f64; 2]) {
        let rng = CounterRng::new(5);
        let n = 400_000u64;
        let nb = 8;
        let mut bins = vec![0u64; nb];
        let mut miss = 0;
        for i in 0..n {
            let mut k = 0;
            let mut u = || {
                k += 1;
                rng.uniform(i, 0, k)
            };
            match bsdf.sample(beta0, &mut u) {
                Some(b) => {
                    let r = b[0].hypot(b[1]);
                    assert!(r < 1.0);
                    bins[((r * nb as f64) as usize).min(nb - 1)] += 1;
                }
                None => miss += 1,
            }
        }
        assert_eq!(miss, 0);
        // BSDF の帯ごとの積分（格子）
        let m = 1200;
        let h = 2.0 / m as f64;
        let mut want = vec![0.0; nb];
        for i in 0..m {
            for j in 0..m {
                let b = [-1.0 + (i as f64 + 0.5) * h, -1.0 + (j as f64 + 0.5) * h];
                let r = b[0].hypot(b[1]);
                if r < 1.0 {
                    want[((r * nb as f64) as usize).min(nb - 1)] += bsdf.value(b, beta0) * h * h;
                }
            }
        }
        let tot: f64 = want.iter().sum();
        for k in 0..nb {
            let p = want[k] / tot;
            let got = bins[k] as f64 / n as f64;
            let sigma = (p * (1.0 - p) / n as f64).sqrt();
            assert!(
                (got - p).abs() < 4.0 * sigma + 2e-3 * p,
                "band {k}: {got} vs {p} (σ {sigma})"
            );
        }
    }

    #[test]
    fn samples_follow_the_bsdf() {
        check_samples(&Scatter::lambert(1.0).prepare().unwrap(), [0.0, 0.0]);
        check_samples(&Scatter::gauss(0.3, 1.0).prepare().unwrap(), [0.5, 0.2]);
        check_samples(
            &Scatter::abg(0.02, 0.01, 2.0).prepare().unwrap(),
            [0.4, -0.3],
        );
        check_samples(
            &Scatter::abg(0.01, 0.002, 1.3).prepare().unwrap(),
            [0.0, 0.0],
        );
    }

    #[test]
    fn normalized_values_integrate_to_the_fraction() {
        for (s, b0) in [
            (Scatter::lambert(0.3), [0.2, 0.0]),
            (Scatter::gauss(0.4, 0.6), [0.7, 0.1]),
        ] {
            let bsdf = s.prepare().unwrap();
            let m = 1500;
            let h = 2.0 / m as f64;
            let mut tot = 0.0;
            for i in 0..m {
                for j in 0..m {
                    let b = [-1.0 + (i as f64 + 0.5) * h, -1.0 + (j as f64 + 0.5) * h];
                    tot += bsdf.value(b, b0) * h * h;
                }
            }
            let b = b0[0].hypot(b0[1]);
            assert!((tot - bsdf.tis(b)).abs() < 3e-3, "{s:?}: {tot}");
        }
    }

    #[test]
    fn finish_needs_one_target() {
        let f = Finish {
            surface: Some(2),
            edge: Some(2),
            scatter: Some(Scatter::lambert(0.1)),
            ..Default::default()
        };
        assert!(f.check().is_err());
        let f = Finish {
            mech: Some("tube".into()),
            ..Default::default()
        };
        assert!(f.check().is_ok());
        let f: Finish = serde_json::from_str(
            r#"{"surface": 3, "scatter": {"model": "abg", "a": 1e-3, "b": 1e-4, "g": 2}}"#,
        )
        .unwrap();
        assert_eq!(f.at().unwrap(), FinishAt::Surface(3));
        assert!(serde_json::from_str::<Finish>(r#"{"surface": 3, "colour": 1}"#).is_err());
    }
}
