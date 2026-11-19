//! 包囲エネルギー: 像面の上の円・x の帯・y の帯・正方形の中の光量の割合と、割合を与えたときの大きさ
//!
//! 光量の分布は重みつきの点の集まり（[`Spots`]）で表す。幾何（瞳の格子の光線の像面の点）と回折（PSF の格子の点）のどちらも
//! 同じ形にする。中心からの距離（円は半径、x・y の帯は半幅、正方形は半幅 max(|dx|, |dy|)）で並べた累積の割合を、点の
//! 距離の間で直線に補う（大きさに対して連続）。

use crate::psf::Psf;
use lensbench_core::error::Result;
use lensbench_core::invalid;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::Waves;

/// 光量を数える形
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnergyShape {
    /// 円（半径）
    #[default]
    Circle,
    /// x の帯 |dx| ≤ r（半幅）
    X,
    /// y の帯 |dy| ≤ r（半幅）
    Y,
    /// 正方形 max(|dx|, |dy|) ≤ r（半幅）
    Square,
}

/// 形の中心
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnergyCenter {
    /// 主波長の主光線の像点
    #[default]
    Chief,
    /// 光量の重心
    Centroid,
    /// 像面の局所座標の原点
    Vertex,
    /// 点の広がりの外接長方形の中心
    Middle,
}

/// 像面の上の重みつきの点（像面の局所座標）と、全光量（点の重みの和より大きいことがある: PSF の格子の外の光）
#[derive(Debug, Clone)]
pub struct Spots {
    pts: Vec<[f64; 3]>,
    total: f64,
    /// 主波長の主光線の像点
    chief: (f64, f64),
    /// PSF の升の大きさ (x, y)。Some なら各点の光量を升の中に一様に広げて数える（升の幅より細かい大きさでも連続）
    cell: Option<(f64, f64)>,
}

impl Spots {
    /// 瞳の n × n の格子の升の中心のうち瞳の中の点の光線（波長 w、None なら全部の波長を波長の重みで）の像面の点。
    /// 波長ごとに、届いた光線の重みの和を波長の重みにそろえる。けられ係数は系のとおり
    pub fn geometric(s: &System, field: FieldSel, w: Waves, n: usize) -> Result<Spots> {
        if n < 4 {
            invalid!("geometric encircled energy needs a pupil grid of 4 or more ({n})");
        }
        let mut px = Vec::new();
        let mut py = Vec::new();
        for i in 0..n {
            for j in 0..n {
                let x = -1.0 + (2 * i + 1) as f64 / n as f64;
                let y = -1.0 + (2 * j + 1) as f64 / n as f64;
                if x * x + y * y <= 1.0 {
                    px.push(x);
                    py.push(y);
                }
            }
        }
        let ws: Vec<usize> = match w.one() {
            Some(w) => vec![w],
            None => (0..s.wavelengths.len()).collect(),
        };
        let mut pts = Vec::new();
        for &wi in &ws {
            let t = s.try_trace(field, &px, &py, Some(wi), None, false)?;
            let ok: Vec<_> = (0..t.rays.len()).filter(|&i| t.ok(i)).collect();
            if ok.is_empty() {
                continue;
            }
            let wt = if ws.len() == 1 { 1.0 } else { s.weights[wi] } / ok.len() as f64;
            pts.extend(ok.iter().map(|&i| [t.rays[i].xyz[0], t.rays[i].xyz[1], wt]));
        }
        if pts.is_empty() {
            invalid!("no ray of the field reaches the image");
        }
        let total = pts.iter().map(|p| p[2]).sum();
        let c = s.try_trace(field, &[0.0], &[0.0], Some(s.primary), None, false)?;
        if !c.ok(0) {
            invalid!("the chief ray doesn't reach the image");
        }
        Ok(Spots {
            pts,
            total,
            chief: (c.rays[0].xyz[0], c.rays[0].xyz[1]),
            cell: None,
        })
    }

    /// PSF の格子の点（強度 × 升の面積、全光量は格子の中の光量 / [`Psf::enclosed`]）。格子は像面の上（[`crate::psf::PsfPlane::Image`]）
    pub fn psf(p: &Psf) -> Result<Spots> {
        if p.plane != crate::psf::PsfPlane::Image {
            invalid!("encircled energy needs a PSF on the image surface");
        }
        let (xs, ys) = (p.i.x(), p.i.y());
        let mut pts = Vec::with_capacity(xs.len() * ys.len());
        let mut sum = 0.0;
        for (iy, row) in p.i.rows().enumerate() {
            for (ix, &v) in row.iter().enumerate() {
                if !v.is_finite() {
                    invalid!("the PSF intensity is undefined at some image grid points");
                }
                pts.push([p.center.0 + xs[ix], p.center.1 + ys[iy], v]);
                sum += v;
            }
        }
        if !(sum > 0.0) || !(p.enclosed > 0.0) {
            invalid!("the PSF has no light");
        }
        Ok(Spots {
            pts,
            total: sum / p.enclosed.min(1.0),
            chief: p.center,
            cell: Some(p.pixel),
        })
    }

    /// 中心の点
    pub fn center(&self, c: EnergyCenter) -> (f64, f64) {
        match c {
            EnergyCenter::Chief => self.chief,
            EnergyCenter::Vertex => (0.0, 0.0),
            EnergyCenter::Centroid => {
                let w: f64 = self.pts.iter().map(|p| p[2]).sum();
                let x = self.pts.iter().map(|p| p[0] * p[2]).sum::<f64>() / w;
                let y = self.pts.iter().map(|p| p[1] * p[2]).sum::<f64>() / w;
                (x, y)
            }
            EnergyCenter::Middle => {
                let lit = self.pts.iter().filter(|p| p[2] > 0.0);
                let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
                for p in lit {
                    x0 = x0.min(p[0]);
                    x1 = x1.max(p[0]);
                    y0 = y0.min(p[1]);
                    y1 = y1.max(p[1]);
                }
                ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
            }
        }
    }

    /// 中心 at からの距離の累積の割合（[`Curve`]、升に広げない）
    pub fn curve(&self, at: (f64, f64), shape: EnergyShape) -> Curve {
        let (cx, cy) = at;
        let mut d: Vec<(f64, f64)> = self
            .pts
            .iter()
            .map(|p| {
                let (dx, dy) = ((p[0] - cx).abs(), (p[1] - cy).abs());
                (dist(shape, dx, dy), p[2])
            })
            .collect();
        d.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out = Vec::with_capacity(d.len() + 1);
        out.push((0.0, 0.0));
        let mut acc = 0.0;
        for (r, w) in d {
            acc += w;
            out.push((r, acc / self.total));
        }
        Curve(out)
    }

    /// 中心 at、大きさ r（半径・半幅）の形の中の光量の割合
    pub fn fraction(&self, at: (f64, f64), shape: EnergyShape, r: f64) -> f64 {
        match self.cell {
            Some(cell) => self.fraction_cells(at, shape, r, cell),
            None => self.curve(at, shape).fraction(r),
        }
    }

    /// 升の中に一様に広げた光量の割合: 升の中心の距離 d の前後 ±h に一様に広がるとして数える（h は升の半幅の、距離の
    /// 向きへの射影。x・y の帯では厳密、円と正方形では近似）
    fn fraction_cells(&self, at: (f64, f64), shape: EnergyShape, r: f64, cell: (f64, f64)) -> f64 {
        let (cx, cy) = at;
        let (hx, hy) = (cell.0 / 2.0, cell.1 / 2.0);
        let mut acc = 0.0;
        for p in &self.pts {
            let (dx, dy) = ((p[0] - cx).abs(), (p[1] - cy).abs());
            let (d, h) = match shape {
                EnergyShape::X => (dx, hx),
                EnergyShape::Y => (dy, hy),
                EnergyShape::Square if dx >= dy => (dx, hx),
                EnergyShape::Square => (dy, hy),
                EnergyShape::Circle => {
                    let d = dx.hypot(dy);
                    if d > 0.0 {
                        (d, (dx * hx + dy * hy) / d)
                    } else {
                        (0.0, hx.max(hy))
                    }
                }
            };
            let (lo, hi) = (d - h, d + h);
            let f = if r >= hi {
                1.0
            } else if lo < 0.0 {
                // 中心をまたぐ升（距離は 0 から hi まで）
                r.max(0.0) / hi
            } else if r <= lo {
                0.0
            } else {
                (r - lo) / (2.0 * h)
            };
            acc += p[2] * f;
        }
        acc / self.total
    }

    /// 中心 at で光量の割合 frac（0 < frac < 1）を含む形の大きさ。届かなければ誤り
    pub fn size(&self, at: (f64, f64), shape: EnergyShape, frac: f64) -> Result<f64> {
        let r = self.curve(at, shape).size(frac)?;
        let Some(cell) = self.cell else {
            return Ok(r);
        };
        // 升に広げた割合（増える関数）を二分法で。点の曲線の値の前後 2 升分から
        let w = cell.0.max(cell.1);
        let (mut lo, mut hi) = ((r - 2.0 * w).max(0.0), r + 2.0 * w);
        while lo > 0.0 && self.fraction(at, shape, lo) > frac {
            lo = (lo - 2.0 * w).max(0.0);
        }
        while self.fraction(at, shape, hi) < frac {
            hi += 2.0 * w;
        }
        Ok(bisect(lo, hi, |m| self.fraction(at, shape, m) >= frac))
    }
}

/// 形の大きさの向きの距離
fn dist(shape: EnergyShape, dx: f64, dy: f64) -> f64 {
    match shape {
        EnergyShape::Circle => dx.hypot(dy),
        EnergyShape::X => dx,
        EnergyShape::Y => dy,
        EnergyShape::Square => dx.max(dy),
    }
}

/// 距離の昇順の (距離, 累積の割合)（最初は (0, 0)）。割合は点の距離の間で直線に補う
#[derive(Debug, Clone)]
pub struct Curve(Vec<(f64, f64)>);

impl Curve {
    /// 大きさ r の中の割合
    pub fn fraction(&self, r: f64) -> f64 {
        let cv = &self.0;
        let k = cv.partition_point(|p| p.0 <= r);
        if k >= cv.len() {
            return cv[cv.len() - 1].1;
        }
        let (a, b) = (cv[k - 1], cv[k]);
        a.1 + (b.1 - a.1) * (r - a.0) / (b.0 - a.0)
    }

    /// 割合 frac（0 < frac < 1）になる大きさ
    pub fn size(&self, frac: f64) -> Result<f64> {
        if !(frac > 0.0 && frac < 1.0) {
            invalid!("fraction {frac} (between 0 and 1)");
        }
        let cv = &self.0;
        let k = cv.partition_point(|p| p.1 < frac);
        if k >= cv.len() {
            invalid!(
                "the energy reaches only {:.4} of the total, not {frac}",
                cv[cv.len() - 1].1
            );
        }
        let (a, b) = (cv[k - 1], cv[k]);
        Ok(a.0 + (b.0 - a.0) * (frac - a.1) / (b.1 - a.1))
    }
}

/// 幾何の円の包囲エネルギーに、波長ごとにエアリー像の円の割合をかけたもの（回折をおおまかに入れる）:
/// Σ ω_w F_w(r) A(π r / (λF)_w)。F_w は波長 w の光線の円の割合、ω_w は波長の重み、(λF)_w は視野のその波長の回折の
/// 遮断周波数（子午・球欠の相乗平均）の逆数（[`crate::psf::geometric_mtf`] の回折限界と同じ遮断周波数）
#[derive(Debug, Clone)]
pub struct AiryEnergy {
    parts: Vec<(f64, Curve, f64)>,
}

impl AiryEnergy {
    /// 視野 field・波長 w（None は全部の波長）の瞳の n × n の格子の光線で、中心 c（重心・広がりの中心は全部の波長の点で）
    pub fn new(
        s: &System,
        field: FieldSel,
        w: Waves,
        n: usize,
        c: EnergyCenter,
    ) -> Result<AiryEnergy> {
        let at = Spots::geometric(s, field, w, n)?.center(c);
        let ws: Vec<usize> = match w.one() {
            Some(w) => vec![w],
            None => (0..s.wavelengths.len())
                .filter(|&i| s.weights[i] > 0.0)
                .collect(),
        };
        let wsum: f64 = ws.iter().map(|&i| s.weights[i]).sum();
        let mut parts = Vec::with_capacity(ws.len());
        for wi in ws {
            let sp = Spots::geometric(s, field, Waves::all_or(Some(wi)), n)?;
            let cut = crate::analysis::mtf_cutoff(s, field, wi)?;
            let lam_f = 1.0 / (cut.tan * cut.sag).sqrt();
            let om = if w.one().is_some() {
                1.0
            } else {
                s.weights[wi] / wsum
            };
            parts.push((om, sp.curve(at, EnergyShape::Circle), lam_f));
        }
        Ok(AiryEnergy { parts })
    }

    /// 半径 r の中の割合
    pub fn fraction(&self, r: f64) -> f64 {
        self.parts
            .iter()
            .map(|(om, cv, lf)| om * cv.fraction(r) * airy_energy(std::f64::consts::PI * r / lf))
            .sum()
    }

    /// 割合 frac になる半径（増える関数の二分法）
    pub fn size(&self, frac: f64) -> Result<f64> {
        if !(frac > 0.0 && frac < 1.0) {
            invalid!("fraction {frac} (between 0 and 1)");
        }
        let mut hi = self.parts.iter().map(|p| p.2).fold(0.0, f64::max);
        let mut n = 0;
        while self.fraction(hi) < frac {
            hi *= 2.0;
            n += 1;
            if n > 60 {
                invalid!("the energy doesn't reach {frac}");
            }
        }
        Ok(bisect(0.0, hi, |m| self.fraction(m) >= frac))
    }
}

/// 増える条件 ok の境目（lo で偽・hi で真）を二分法で
fn bisect(mut lo: f64, mut hi: f64, ok: impl Fn(f64) -> bool) -> f64 {
    for _ in 0..100 {
        let m = 0.5 * (lo + hi);
        if ok(m) {
            hi = m;
        } else {
            lo = m;
        }
        if hi - lo <= 1e-15 * hi {
            break;
        }
    }
    0.5 * (lo + hi)
}

/// ストレール比: 像面の上の FFT の回折 PSF（波長 w、瞳の格子 pupil、None は自動）の最大の値（収差のない同じ瞳の PSF の
/// 中心を 1）。格子の間隔は既定の半分（使う波長の回折の遮断周波数の最大の 4 倍の逆数）で、最大の点の周りを 2 次式で補う
pub fn strehl(s: &System, field: FieldSel, w: Waves, pupil: Option<usize>) -> Result<f64> {
    let ws: Vec<usize> = match w.one() {
        Some(w) => vec![w],
        None => (0..s.wavelengths.len()).collect(),
    };
    let mut top = 0.0f64;
    for wi in ws {
        let c = crate::analysis::mtf_cutoff(s, field, wi)?;
        top = top.max(c.tan).max(c.sag);
    }
    if !(top > 0.0 && top.is_finite()) {
        invalid!("the diffraction cutoff of the field is not finite");
    }
    let o = crate::psf::PsfOpts {
        pupil,
        image: 128,
        pixel: Some(0.25 / top),
        plane: crate::psf::PsfPlane::Image,
    };
    let p = crate::psf::psf(s, field, w, o)?;
    let (nx, ny) = (p.i.nx(), p.i.ny());
    let mut best = (0, 0, f64::NEG_INFINITY);
    for iy in 0..ny {
        for ix in 0..nx {
            let v = *p.i.get(ix, iy);
            if v > best.2 {
                best = (ix, iy, v);
            }
        }
    }
    let (ix, iy, v) = best;
    if !v.is_finite() {
        invalid!("the PSF has no finite peak");
    }
    if ix == 0 || iy == 0 || ix + 1 == nx || iy + 1 == ny {
        return Ok(v);
    }
    // 頂点の 2 次式（x・y 別）の最大
    let g = |a: usize, b: usize| *p.i.get(a, b);
    let up = |m: f64, c: f64, q: f64| -> f64 {
        let (d1, d2) = ((q - m) / 2.0, q - 2.0 * c + m);
        if d2 < 0.0 {
            -d1 * d1 / (2.0 * d2)
        } else {
            0.0
        }
    };
    Ok(v + up(g(ix - 1, iy), v, g(ix + 1, iy)) + up(g(ix, iy - 1), v, g(ix, iy + 1)))
}

/// 円形の無収差の瞳の像（エアリー像）の、半径 v = π r / (λ F) の円の中の光量の割合 1 − J0(v)² − J1(v)²
pub fn airy_energy(v: f64) -> f64 {
    let v = v.abs();
    if v > 400.0 {
        // 漸近形（J0² + J1² ≈ 2 / (π v)、v = 400 で相対 1e-6 以下の差）
        return 1.0 - 2.0 / (std::f64::consts::PI * v);
    }
    let (j0, j1) = bessel01(v);
    1.0 - j0 * j0 - j1 * j1
}

/// J0(x)・J1(x)（積分表示 Jn(x) = (1/π) ∫₀^π cos(nτ − x sin τ) dτ の台形則。被積分関数が周期的で滑らかなので点の数が
/// x + 40 を超えれば丸めの誤差まで収束する）
fn bessel01(x: f64) -> (f64, f64) {
    let m = (x as usize + 40) * 2;
    let (mut j0, mut j1) = (0.0, 0.0);
    for k in 0..m {
        let t = std::f64::consts::PI * (k as f64 + 0.5) / m as f64;
        let ph = x * t.sin();
        j0 += ph.cos();
        j1 += (t - ph).cos();
    }
    (j0 / m as f64, j1 / m as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn airy_energy_known_values() {
        // 暗輪（J1 の零点）では 1 − J0²: J0 = −0.402 759 395 702 553・0.300 115 752 526 133
        let ring = |j0: f64| 1.0 - j0 * j0;
        assert!((airy_energy(3.831_705_970_207_512) - ring(-0.402_759_395_702_553)).abs() < 1e-13);
        assert!((airy_energy(7.015_586_669_815_619) - ring(0.300_115_752_526_133)).abs() < 1e-13);
        assert!(airy_energy(0.0).abs() < 1e-15);
        let (j0, j1) = bessel01(10.0);
        assert!((j0 - (-0.245_935_764_451_348_3)).abs() < 1e-14);
        assert!((j1 - 0.043_472_746_168_861_44).abs() < 1e-14);
    }
}
