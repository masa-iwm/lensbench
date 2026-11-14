//! 一括実行の枠組み: 公差・モンテカルロ・パラメータの掃引を、スレッドで並列に回し、結果を逐次ファイルに書き、
//! 止まっても途中から再開する。
//!
//! - 組の作り方 [`Plan`]: [`Sampler`]（分布・相関・乱数の種からの抽選。値は公称値からのずれ）と [`Grid`]（掃引の格子、値そのもの）。
//!   i 番目の組は (種, i) だけで決まるので、スレッド数・再開・実行の順によらず同じ値になる。
//! - タスクごとの種 [`sub_seed`]: (種, i) から作る。タスクの中の雑音などに使う。
//! - 実行 [`run`]: 未完了の番号を rayon のプール（[`crate::with_threads`]）で並列に回す。タスクの中の並列（追跡など）は
//!   入れ子のまま使える。結果は JSON の 1 行ずつ（JSONL）で、終わった順に書いて 1 行ごとに flush する。
//! - 再開: 同じ頭（1 行目）のファイルがあれば、書き終わった番号を飛ばす（途中で切れた最後の行は捨てる）。
//!   頭は呼び出し側が作る JSON（版・系の hash・種・組の作り方など）で、違えばエラー（別の計算を混ぜない）。
//!
//! ファイルの形:
//! ```text
//! {"lensbench_runs":1,"header":{…}}
//! {"i":0,"seed":123,"result":{…}}
//! {"i":2,"seed":456,"result":{…}}
//! ```

use crate::error::{Error, Result};
use crate::param::Param;
use crate::system::System;
use crate::{invalid, par};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use web_time::Instant;

// ------------------------------------------------------------------------------------------ 乱数
/// SplitMix64 の 1 段（種の混ぜ合わせと乱数列）
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 番号 i のタスクの種（組の抽選とは別の列）
pub fn sub_seed(seed: u64, i: usize) -> u64 {
    mix(mix(seed ^ 0x5EED_0000_0000_0001).wrapping_add(i as u64))
}

/// (種, 番号) で決まる乱数列（SplitMix64）
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64, i: usize) -> Rng {
        Rng(mix(mix(seed).wrapping_add(i as u64)))
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0.wrapping_sub(0x9E37_79B9_7F4A_7C15))
    }
    /// (0, 1) の一様乱数（0 と 1 は出ない）
    pub fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    /// 標準正規乱数（Box–Muller、1 回に 2 つ作って 1 つ使う。列が番号で決まれば値も決まる）
    pub fn normal(&mut self) -> f64 {
        let (u, v) = (self.uniform(), self.uniform());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

impl Rng {
    /// Poisson 乱数（平均 30 未満は積の方法で厳密、以上は正規分布で近似して丸める）。平均が正でなければ 0
    pub fn poisson(&mut self, lam: f64) -> f64 {
        if !(lam > 0.0) {
            return 0.0;
        }
        if lam < 30.0 {
            let l = (-lam).exp();
            let (mut k, mut p) = (0.0, 1.0);
            loop {
                p *= self.uniform();
                if p <= l {
                    return k;
                }
                k += 1.0;
            }
        }
        (lam + lam.sqrt() * self.normal()).round().max(0.0)
    }
}

// ------------------------------------------------------------------------------------------ 検出器の雑音
/// 検出器の雑音と AD 変換（検出器の露光で共通）。値は電子数 [e⁻]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SensorNoise {
    /// ショット雑音（Poisson）を足す
    pub shot: bool,
    /// 読み出し雑音 [e⁻ rms]
    pub read: f64,
    /// 飽和 [e⁻]（0 なら飽和しない）。AD 変換の全幅
    pub full_well: f64,
    /// AD 変換のビット数（0 なら丸めない）
    pub bits: u32,
}

impl SensorNoise {
    /// 何も足さない・丸めない
    pub fn is_quiet(&self) -> bool {
        !self.shot && self.read == 0.0 && self.full_well == 0.0
    }

    /// 値を確かめる
    pub fn check(&self) -> Result<()> {
        if !(self.read >= 0.0) || !(self.full_well >= 0.0) {
            invalid!(
                "読み出し雑音・飽和は 0 以上に（{}・{}）",
                self.read,
                self.full_well
            );
        }
        if self.bits > 0 && !(self.full_well > 0.0) {
            invalid!("AD 変換の bits には飽和 full_well が要る");
        }
        if self.bits > 32 {
            invalid!("AD 変換のビット数は 32 まで（{}）", self.bits);
        }
        Ok(())
    }

    /// 1 画素の露光。`mu` は平均の電子数（背景を含む）。ショット雑音は Poisson（[`Rng::poisson`]）、読み出し雑音は
    /// 正規分布。bits > 0 なら刻み q = full_well / 2^bits で round(e/q)（四捨五入）を 0〜2^bits − 1 に切って q 倍、
    /// bits = 0 で full_well > 0 なら 0〜full_well に切る。値でない `mu` はそのまま返す（乱数は使わない）
    pub fn expose(&self, mu: f64, rng: &mut Rng) -> f64 {
        if !mu.is_finite() {
            return mu;
        }
        let mut e = if self.shot { rng.poisson(mu) } else { mu };
        if self.read > 0.0 {
            e += self.read * rng.normal();
        }
        if self.bits > 0 {
            let levels = 2f64.powi(self.bits as i32);
            let q = self.full_well / levels;
            (e / q).round().clamp(0.0, levels - 1.0) * q
        } else if self.full_well > 0.0 {
            e.clamp(0.0, self.full_well)
        } else {
            e
        }
    }
}

/// 標準正規分布の累積分布関数
pub fn norm_cdf(x: f64) -> f64 {
    0.5 * erfc(-x / std::f64::consts::SQRT_2)
}

#[allow(clippy::excessive_precision)]
/// 相補誤差関数（Numerical Recipes 3 版の Chebyshev 展開、相対誤差 1.2e-16 程度）
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 2.0 / (2.0 + z);
    let ty = 4.0 * t - 2.0;
    const C: [f64; 28] = [
        -1.3026537197817094,
        6.419_697_923_564_902e-1,
        1.9476473204185836e-2,
        -9.561514786808631e-3,
        -9.46595344482036e-4,
        3.66839497852761e-4,
        4.2523324806907e-5,
        -2.0278578112534e-5,
        -1.624290004647e-6,
        1.303655835580e-6,
        1.5626441722e-8,
        -8.5238095915e-8,
        6.529054439e-9,
        5.059343495e-9,
        -9.91364156e-10,
        -2.27365122e-10,
        9.6467911e-11,
        2.394038e-12,
        -6.886027e-12,
        8.94487e-13,
        3.13092e-13,
        -1.12708e-13,
        3.81e-16,
        7.106e-15,
        -1.523e-15,
        -9.4e-17,
        1.21e-16,
        -2.8e-17,
    ];
    let (mut d, mut dd) = (0.0, 0.0);
    for &c in C.iter().skip(1).rev() {
        let tmp = d;
        d = ty * d - dd + c;
        dd = tmp;
    }
    let r = t * (-z * z + 0.5 * (C[0] + ty * d) - dd).exp();
    if x >= 0.0 {
        r
    } else {
        2.0 - r
    }
}

#[allow(clippy::excessive_precision)]
/// 標準正規分布の分位点（Acklam の近似を Newton 法 1 回で詰める、相対誤差 1e-15 程度）
pub fn norm_ppf(p: f64) -> f64 {
    if !(p > 0.0 && p < 1.0) {
        return if p <= 0.0 {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    const A: [f64; 6] = [
        -3.969683028665376e1,
        2.209460984245205e2,
        -2.759285104469687e2,
        1.383577518672690e2,
        -3.066479806614716e1,
        2.506628277459239,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e1,
        1.615858368580409e2,
        -1.556989798598866e2,
        6.680131188771972e1,
        -1.328068155288572e1,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-3,
        -3.223964580411365e-1,
        -2.400758277161838,
        -2.549732539343734,
        4.374664141464968,
        2.938163982698783,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-3,
        3.224671290700398e-1,
        2.445134137142996,
        3.754408661907416,
    ];
    let pl = 0.02425;
    let x = if p < pl {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - pl {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    // Newton（Halley）1 回
    let e = norm_cdf(x) - p;
    let u = e * (2.0 * std::f64::consts::PI).sqrt() * (x * x / 2.0).exp();
    x - u / (1.0 + x * u / 2.0)
}

// ------------------------------------------------------------------------------------------ 組の作り方
/// 分布（公称値からのずれ）
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Dist {
    /// 正規分布（平均 mean、標準偏差 sigma）
    Normal { mean: f64, sigma: f64 },
    /// 一様分布 [lo, hi]
    Uniform { lo: f64, hi: f64 },
    /// 平均 ± cut·sigma で切った正規分布（公差の ±値を cut σ と見るときなど）
    TruncNormal { mean: f64, sigma: f64, cut: f64 },
    /// 定数（ずれを固定する。比べるときに）
    Fixed(f64),
}

impl Dist {
    fn check(&self) -> Result<()> {
        let ok = match *self {
            Dist::Normal { mean, sigma } => mean.is_finite() && sigma.is_finite() && sigma >= 0.0,
            Dist::Uniform { lo, hi } => lo.is_finite() && hi.is_finite() && lo <= hi,
            Dist::TruncNormal { mean, sigma, cut } => {
                mean.is_finite() && sigma.is_finite() && sigma >= 0.0 && cut > 0.0
            }
            Dist::Fixed(v) => v.is_finite(),
        };
        if !ok {
            invalid!("分布 {self:?}");
        }
        Ok(())
    }

    /// 標準正規の値 z（相関を入れた後）を、この分布の値にする（ガウスのコピュラ）
    fn at_z(&self, z: f64) -> f64 {
        match *self {
            Dist::Normal { mean, sigma } => mean + sigma * z,
            Dist::Uniform { lo, hi } => lo + (hi - lo) * norm_cdf(z),
            Dist::TruncNormal { mean, sigma, cut } => {
                let a = norm_cdf(-cut);
                let p = a + (1.0 - 2.0 * a) * norm_cdf(z);
                mean + sigma * norm_ppf(p).clamp(-cut, cut)
            }
            Dist::Fixed(v) => v,
        }
    }
}

/// 組の作り方: i 番目の組のパラメータの値
pub trait Plan: Sync {
    /// 組の数（抽選なら None: 何番まででも作れる）
    fn count(&self) -> Option<usize>;
    /// i 番目の組の変更（系 base の公称値を使う）
    fn changes(&self, i: usize, base: &System) -> Result<Vec<(Param, f64)>>;
}

/// 抽選: パラメータごとの分布と相関、乱数の種。値は公称値 + ずれ
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Sampler {
    pub vars: Vec<(Param, Dist)>,
    /// 相関行列の下三角の Cholesky 分解（None は無相関）
    chol: Option<Vec<Vec<f64>>>,
    pub seed: u64,
}

impl Sampler {
    /// corr は相関行列（vars と同じ順、対称・対角 1・正定値）
    pub fn new(
        vars: Vec<(Param, Dist)>,
        corr: Option<Vec<Vec<f64>>>,
        seed: u64,
    ) -> Result<Sampler> {
        for (_, d) in &vars {
            d.check()?;
        }
        let n = vars.len();
        let chol = match corr {
            None => None,
            Some(c) => {
                if c.len() != n || c.iter().any(|r| r.len() != n) {
                    invalid!("相関行列の大きさ（{n} × {n}）");
                }
                for i in 0..n {
                    if (c[i][i] - 1.0).abs() > 1e-12 {
                        invalid!("相関行列の対角 {i} が 1 でない（{}）", c[i][i]);
                    }
                    for j in 0..i {
                        if (c[i][j] - c[j][i]).abs() > 1e-12 || c[i][j].abs() > 1.0 {
                            invalid!("相関行列 ({i}, {j}) が対称でないか |ρ| > 1");
                        }
                    }
                }
                let mut l = vec![vec![0.0; n]; n];
                for i in 0..n {
                    for j in 0..=i {
                        let s: f64 = (0..j).map(|k| l[i][k] * l[j][k]).sum();
                        if i == j {
                            let d = c[i][i] - s;
                            if d <= 1e-14 {
                                invalid!("相関行列が正定値でない");
                            }
                            l[i][j] = d.sqrt();
                        } else {
                            l[i][j] = (c[i][j] - s) / l[j][j];
                        }
                    }
                }
                Some(l)
            }
        };
        Ok(Sampler { vars, chol, seed })
    }

    /// i 番目の組のずれ（vars の順）
    pub fn draw(&self, i: usize) -> Vec<f64> {
        let mut r = Rng::new(self.seed, i);
        let u: Vec<f64> = self.vars.iter().map(|_| r.normal()).collect();
        let z: Vec<f64> = match &self.chol {
            None => u,
            Some(l) => (0..u.len())
                .map(|a| (0..=a).map(|b| l[a][b] * u[b]).sum())
                .collect(),
        };
        self.vars
            .iter()
            .zip(z)
            .map(|((_, d), z)| d.at_z(z))
            .collect()
    }
}

impl Plan for Sampler {
    fn count(&self) -> Option<usize> {
        None
    }
    fn changes(&self, i: usize, base: &System) -> Result<Vec<(Param, f64)>> {
        let d = self.draw(i);
        self.vars
            .iter()
            .zip(d)
            .map(|((p, _), x)| Ok((*p, base.get_param(*p)? + x)))
            .collect()
    }
}

/// 掃引の格子: 軸ごとのパラメータと値（値そのもの）。組の番号は最初の軸が最も遅く変わる順
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Grid {
    pub axes: Vec<(Param, Vec<f64>)>,
}

impl Grid {
    pub fn new(axes: Vec<(Param, Vec<f64>)>) -> Result<Grid> {
        if axes
            .iter()
            .any(|(_, v)| v.is_empty() || v.iter().any(|x| !x.is_finite()))
        {
            invalid!("掃引の軸に値がないか、有限でない値");
        }
        Ok(Grid { axes })
    }
    /// i 番目の組の値（axes の順）
    pub fn values(&self, i: usize) -> Vec<f64> {
        let mut out = vec![0.0; self.axes.len()];
        let mut k = i;
        for (a, (_, v)) in self.axes.iter().enumerate().rev() {
            out[a] = v[k % v.len()];
            k /= v.len();
        }
        out
    }
}

impl Plan for Grid {
    fn count(&self) -> Option<usize> {
        Some(self.axes.iter().map(|(_, v)| v.len()).product())
    }
    fn changes(&self, i: usize, _base: &System) -> Result<Vec<(Param, f64)>> {
        if let Some(n) = self.count() {
            if i >= n {
                invalid!("組 {i}（{n} 組）");
            }
        }
        Ok(self
            .axes
            .iter()
            .zip(self.values(i))
            .map(|((p, _), x)| (*p, x))
            .collect())
    }
}

// ------------------------------------------------------------------------------------------ 実行
/// 進み具合
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    /// 終わった数（再開で飛ばした分を含む）
    pub done: usize,
    pub total: usize,
    /// この実行で終えた数
    pub ran: usize,
    /// この実行の経過（秒）
    pub elapsed: f64,
    /// 残りの見込み（秒。この実行の速さから。まだ分からなければ None）
    pub eta: Option<f64>,
}

/// 実行の設定
#[derive(Default)]
pub struct RunOpts<'a> {
    /// スレッド数（[`crate::par::with_threads`] と同じ。None は既定、逐次の指定の中では逐次）
    pub threads: Option<usize>,
    /// 結果のファイル（JSONL）。None なら書かない
    pub out: Option<&'a Path>,
    /// 頭（JSON のオブジェクトの文字列）。再開のときは同じでなければならない
    pub header: String,
    /// 同じ頭のファイルがあれば続きから（false なら書き直す）
    pub resume: bool,
    /// 進み具合（タスクが終わるたびに、ワーカーのスレッドから呼ぶ）
    pub progress: Option<&'a (dyn Fn(Progress) + Sync)>,
    /// true になったら新しいタスクを始めない（走っているものは終わらせる）
    pub cancel: Option<&'a AtomicBool>,
}

/// 実行の結果
#[derive(Debug, Default)]
pub struct RunSummary {
    pub total: usize,
    /// 再開で飛ばした数
    pub skipped: usize,
    /// この実行で終えた数
    pub ran: usize,
    /// 止めたか（cancel）
    pub cancelled: bool,
}

const MAGIC: &str = "{\"lensbench_runs\":1,\"header\":";

/// 結果のファイルを読む: (頭, [(番号, タスクの種, 結果の JSON の文字列)])。番号の順。途中で切れた最後の行は捨てる
#[allow(clippy::type_complexity)]
pub fn read_results(path: &Path) -> Result<(String, Vec<(usize, u64, String)>)> {
    let txt =
        std::fs::read_to_string(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    let mut lines = complete_lines(&txt).into_iter();
    let head = match lines.next() {
        Some(l) => l,
        None => invalid!("{}: 空のファイル", path.display()),
    };
    let Some(h) = head.strip_prefix(MAGIC).and_then(|x| x.strip_suffix('}')) else {
        invalid!("{}: 一括実行の結果のファイルでない", path.display());
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for l in lines {
        if let Some(x) = parse_line(l) {
            if seen.insert(x.0) {
                out.push(x);
            }
        }
    }
    out.sort_by_key(|x| x.0);
    Ok((h.to_string(), out))
}

/// 改行で終わった行だけ（書き途中で切れた最後の行は捨てる。行は 1 回の write_all で改行まで書く）。
/// Windows のエディタで保存し直した CRLF のファイルも読めるよう、行末の \r は外す
fn complete_lines(txt: &str) -> Vec<&str> {
    let mut v: Vec<&str> = txt
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    v.pop(); // 最後の改行の後（空）か、改行のない切れた行
    v
}

/// 1 行を改行まで 1 回の write_all で書いて flush する
fn write_line(f: &mut std::fs::File, line: &str) -> std::io::Result<()> {
    let mut b = String::with_capacity(line.len() + 1);
    b.push_str(line);
    b.push('\n');
    f.write_all(b.as_bytes())?;
    f.flush()
}

/// {"i":N,"seed":S,"result":R} の 1 行（書いた形そのもの。切れていれば None）
fn parse_line(l: &str) -> Option<(usize, u64, String)> {
    let r = l.strip_prefix("{\"i\":")?;
    let (i, r) = r.split_once(",\"seed\":")?;
    let (s, r) = r.split_once(",\"result\":")?;
    let res = r.strip_suffix('}')?;
    Some((i.parse().ok()?, s.parse().ok()?, res.to_string()))
}

/// タスク task(i, タスクの種) を i = 0..n で回す（再開なら終わった番号を飛ばす）。結果は JSON の値の文字列で返す。
/// タスクが失敗したら新しいタスクを始めず、走っているものが終わってから最初のエラーを返す（書いた結果は残る）
pub fn run<F>(n: usize, seed: u64, opts: &RunOpts, task: F) -> Result<RunSummary>
where
    F: Fn(usize, u64) -> Result<String> + Sync,
{
    if opts.header.contains('\n') || opts.header.contains('\r') {
        invalid!("一括実行の頭に改行（1 行の JSON に）");
    }
    let mut done: HashSet<usize> = HashSet::new();
    let mut file: Option<std::fs::File> = None;
    if let Some(p) = opts.out {
        let mut keep = false;
        if opts.resume && p.exists() {
            let (h, rows) = read_results(p)?;
            if h != opts.header {
                invalid!(
                    "{}: 頭が違う（別の計算の結果。resume をやめるか別のファイルに）",
                    p.display()
                );
            }
            done = rows.into_iter().map(|r| r.0).filter(|&i| i < n).collect();
            keep = true;
            // 切れた行を除いて書き直す（行の境目から追記する）
            let txt = std::fs::read_to_string(p).map_err(|e| Error::Io(e.to_string()))?;
            let mut good = String::with_capacity(txt.len());
            for (k, l) in complete_lines(&txt).into_iter().enumerate() {
                if k == 0 || parse_line(l).is_some() {
                    good.push_str(l);
                    good.push('\n');
                }
            }
            if good != txt {
                std::fs::write(p, good).map_err(|e| Error::Io(e.to_string()))?;
            }
        }
        let f = if keep {
            std::fs::OpenOptions::new().append(true).open(p)
        } else {
            std::fs::File::create(p)
        }
        .map_err(|e| Error::Io(format!("{}: {e}", p.display())))?;
        file = Some(f);
        if !keep {
            let f = file.as_mut().unwrap();
            write_line(f, &format!("{MAGIC}{}}}", opts.header))
                .map_err(|e| Error::Io(e.to_string()))?;
        }
    }
    let todo: Vec<usize> = (0..n).filter(|i| !done.contains(i)).collect();
    let skipped = n - todo.len();
    let file = Mutex::new(file);
    let failed: Mutex<Option<Error>> = Mutex::new(None);
    let stop = AtomicBool::new(false);
    let ran = AtomicUsize::new(0);
    let t0 = Instant::now();
    let cancelled = || opts.cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    par::with_threads(opts.threads, || {
        let _ = par::map(&todo, |&i| {
            if stop.load(Ordering::Relaxed) || cancelled() {
                return;
            }
            let s = sub_seed(seed, i);
            match task(i, s) {
                Ok(res) => {
                    if res.contains('\n') {
                        stop.store(true, Ordering::Relaxed);
                        failed
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .get_or_insert(Error::Invalid(format!("組 {i}: 結果に改行")));
                        return;
                    }
                    let mut g = file.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(f) = g.as_mut() {
                        let w =
                            write_line(f, &format!("{{\"i\":{i},\"seed\":{s},\"result\":{res}}}"));
                        if let Err(e) = w {
                            stop.store(true, Ordering::Relaxed);
                            failed
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .get_or_insert(Error::Io(e.to_string()));
                            return;
                        }
                    }
                    let k = ran.fetch_add(1, Ordering::Relaxed) + 1;
                    drop(g);
                    if let Some(cb) = opts.progress {
                        let el = t0.elapsed().as_secs_f64();
                        let left = todo.len() - k.min(todo.len());
                        cb(Progress {
                            done: skipped + k,
                            total: n,
                            ran: k,
                            elapsed: el,
                            eta: (k > 0).then(|| el / k as f64 * left as f64),
                        });
                    }
                }
                Err(e) => {
                    stop.store(true, Ordering::Relaxed);
                    failed
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get_or_insert(Error::Invalid(format!("組 {i}: {e}")));
                }
            }
        });
    });
    if let Some(e) = failed.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return Err(e);
    }
    Ok(RunSummary {
        total: n,
        skipped,
        ran: ran.into_inner(),
        cancelled: cancelled(),
    })
}

#[cfg(test)]
mod noise_tests {
    use super::*;

    #[test]
    fn crlf_results_file_reads() {
        // Windows のエディタで保存し直した CRLF のファイル（切れた最後の行は捨てる）
        let dir = std::env::temp_dir().join(format!("lb_runs_crlf_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("r.jsonl");
        let txt = format!(
            "{MAGIC}{{\"a\":1}}}}\r\n{{\"i\":1,\"seed\":5,\"result\":2}}\r\n{{\"i\":0,\"seed\":4,\"result\":1}}\r\n{{\"i\":2,"
        );
        std::fs::write(&p, txt).unwrap();
        let (h, rows) = read_results(&p).unwrap();
        assert_eq!(h, "{\"a\":1}");
        assert_eq!(rows, vec![(0, 4, "1".into()), (1, 5, "2".into())]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn poisson_has_the_right_mean_and_variance() {
        for &lam in &[0.7, 12.0, 400.0] {
            let mut r = Rng::new(7, 0);
            let n = 40000;
            let (mut s, mut s2) = (0.0, 0.0);
            for _ in 0..n {
                let v = r.poisson(lam);
                assert_eq!(v, v.round());
                s += v;
                s2 += v * v;
            }
            let m = s / n as f64;
            let var = s2 / n as f64 - m * m;
            assert!(
                (m - lam).abs() < 4.0 * (lam / n as f64).sqrt(),
                "{lam}: {m}"
            );
            assert!((var / lam - 1.0).abs() < 0.05, "{lam}: {var}");
        }
    }

    #[test]
    fn adc_rounds_half_away_from_zero_and_clips() {
        let q = 20000.0 / 4096.0;
        let a = SensorNoise {
            full_well: 20000.0,
            bits: 12,
            ..Default::default()
        };
        let mut r = Rng::new(1, 0);
        assert_eq!(a.expose(2.5 * q, &mut r), 3.0 * q);
        assert_eq!(a.expose(2.4 * q, &mut r), 2.0 * q);
        assert_eq!(a.expose(-3.0, &mut r), 0.0);
        assert_eq!(a.expose(1e9, &mut r), 4095.0 * q);
        assert!(a.expose(f64::NAN, &mut r).is_nan());
        let fw = SensorNoise {
            full_well: 4.0,
            ..Default::default()
        };
        assert_eq!(fw.expose(5.0, &mut r), 4.0);
        assert!(SensorNoise::default().is_quiet());
        assert!(SensorNoise {
            bits: 8,
            ..Default::default()
        }
        .check()
        .is_err());
    }
}
