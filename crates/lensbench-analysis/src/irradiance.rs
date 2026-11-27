//! 光線を数える検出器（インコヒーレント）: 追跡の結果の光線が最後の面に当たる点で、光線の重みを画素に足す。
//!
//! 重みは [`Trace::weight`]（重みのある光源 [`System::trace_source`] の重み、重みの無い追跡では 1）に、光線ごとの
//! 掛ける数（光源の全パワーを分けたパワー [W] など、[`DetectorOpts::scale`]）を掛けたもの。画素の値の和は、検出面に
//! 届いた光線の重みの和になる（足し方は光線の順で決まり、スレッド数に依らない）。
//!
//! 順次の追跡の結果は膜・吸収の透過率を重みに含まない（[`Trace::w`]）。透過率を入れるなら、偏光追跡の結果の
//! 透過率を掛けた数を scale に渡す。非順次の追跡（境界ごとに分かれる）では、分かれるたびに重みに掛けたものが
//! 光線の重みになる。
//!
//! 画素の約束は干渉の検出器（[`crate::interference::detector`]）と同じ [`Grid2`]（座標は画素の中心 [mm]、行 = y、
//! 最後の面の局所座標）。画素 (i, j) は中心から ±dx/2 の範囲（下の端を含み、上の端を含まない）。
//!
//! [`System::trace_source`]: lensbench_core::System::trace_source

use lensbench_core::grid::Grid2;
use lensbench_core::system::Trace;
use lensbench_core::{invalid, Result};

/// 検出器の設定
#[derive(Debug, Clone, PartialEq)]
pub struct DetectorOpts {
    /// 画素の数（一辺）
    pub npx: usize,
    /// 検出面の中心 (x, y) [mm]（None なら届いた光線の範囲の中心）
    pub center: Option<(f64, f64)>,
    /// 検出面の一辺 [mm]（None なら届いた光線を全部覆う正方形）
    pub width: Option<f64>,
    /// 光線ごとの重みに掛ける数（光線の数と同じ長さ。None なら 1）
    pub scale: Option<Vec<f64>>,
}

impl Default for DetectorOpts {
    fn default() -> Self {
        DetectorOpts {
            npx: 64,
            center: None,
            width: None,
            scale: None,
        }
    }
}

/// 検出器の結果
#[derive(Debug, Clone)]
pub struct Detector {
    /// 画素の間隔 [mm]
    pub dx: f64,
    /// 画素ごとの重みの和（パワーを渡せば W）
    pub power: Grid2,
    /// 画素ごとの照度（power / 画素の面積、W/mm²）
    pub irradiance: Grid2,
    /// 画素ごとの光線の数
    pub hits: Grid2<usize>,
    /// 検出面に届いた重みの和（画素の値の和）
    pub detected: f64,
    /// 最後の面まで届いた光線の重みの和（検出面の外に落ちたものも）
    pub arrived: f64,
    /// 全部の光線の重みの和（届かなかったものも）
    pub launched: f64,
}

/// 追跡の結果 t の光線を検出器に数える
pub fn detect(t: &Trace, o: &DetectorOpts) -> Result<Detector> {
    let n = o.npx;
    if n < 1 {
        invalid!("the number of pixels must be 1 or more");
    }
    let m = t.rays.len();
    if let Some(s) = &o.scale {
        if s.len() != m {
            invalid!("{} scale factors for {m} rays", s.len());
        }
        if let Some((i, x)) = s
            .iter()
            .enumerate()
            .find(|(_, x)| !(x.is_finite() && **x >= 0.0))
        {
            invalid!("ray {i} has the scale factor {x} (must be finite and not negative)");
        }
    }
    let wt = |i: usize| t.weight(i) * o.scale.as_ref().map_or(1.0, |s| s[i]);
    let ok: Vec<usize> = (0..m).filter(|&i| t.ok(i)).collect();
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &i in &ok {
        let p = t.rays[i].xyz;
        x0 = x0.min(p[0]);
        x1 = x1.max(p[0]);
        y0 = y0.min(p[1]);
        y1 = y1.max(p[1]);
    }
    let c = match o.center {
        Some(c) => c,
        None if ok.is_empty() => invalid!("no ray reaches the detector"),
        None => (0.5 * (x0 + x1), 0.5 * (y0 + y1)),
    };
    let width = match o.width {
        Some(w) => w,
        None if ok.is_empty() => invalid!("no ray reaches the detector"),
        None => {
            let h = (x1 - c.0)
                .abs()
                .max((x0 - c.0).abs())
                .max((y1 - c.1).abs())
                .max((y0 - c.1).abs());
            // 上の端の光線も入るように少し広げる（1 点に集まるときは 1 µm 角）
            if h > 0.0 {
                2.0 * h * (1.0 + 1e-9)
            } else {
                1e-3
            }
        }
    };
    if !(width > 0.0 && width.is_finite()) || !(c.0.is_finite() && c.1.is_finite()) {
        invalid!("detector width {width} mm or center {c:?}");
    }
    let dx = width / n as f64;
    let (lx, ly) = (c.0 - 0.5 * width, c.1 - 0.5 * width);
    let mut power = vec![0.0; n * n];
    let mut hits = vec![0usize; n * n];
    let mut arrived = 0.0;
    for &i in &ok {
        let w = wt(i);
        arrived += w;
        let p = t.rays[i].xyz;
        let (u, v) = ((p[0] - lx) / dx, (p[1] - ly) / dx);
        if !(u >= 0.0 && v >= 0.0 && u < n as f64 && v < n as f64) {
            continue;
        }
        let q = v as usize * n + u as usize;
        power[q] += w;
        hits[q] += 1;
    }
    let detected = power.iter().sum();
    let launched = (0..m).map(wt).sum();
    let xs: Vec<f64> = (0..n).map(|i| lx + (i as f64 + 0.5) * dx).collect();
    let ys: Vec<f64> = (0..n).map(|j| ly + (j as f64 + 0.5) * dx).collect();
    let a = dx * dx;
    Ok(Detector {
        dx,
        irradiance: Grid2::new(
            xs.clone(),
            ys.clone(),
            power.iter().map(|p| p / a).collect(),
        ),
        power: Grid2::new(xs.clone(), ys.clone(), power),
        hits: Grid2::new(xs, ys, hits),
        detected,
        arrived,
        launched,
    })
}

/// 重み付きのスポットの重心と RMS 半径（最後の面の局所座標 [mm]。届いた光線の重みで平均）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpotMoments {
    /// 重心 (x, y)
    pub centroid: (f64, f64),
    /// 重心のまわりの RMS 半径
    pub rms: f64,
    /// 届いた光線の重みの和
    pub weight: f64,
}

/// 追跡の結果 t の届いた光線の重み付きの重心と RMS 半径（重みの和が 0 なら None）
pub fn spot_moments(t: &Trace) -> Option<SpotMoments> {
    let (mut sw, mut sx, mut sy) = (0.0, 0.0, 0.0);
    for i in (0..t.rays.len()).filter(|&i| t.ok(i)) {
        let (w, p) = (t.weight(i), t.rays[i].xyz);
        sw += w;
        sx += w * p[0];
        sy += w * p[1];
    }
    if !(sw > 0.0) {
        return None;
    }
    let c = (sx / sw, sy / sw);
    let s2: f64 = (0..t.rays.len())
        .filter(|&i| t.ok(i))
        .map(|i| {
            let p = t.rays[i].xyz;
            t.weight(i) * ((p[0] - c.0).powi(2) + (p[1] - c.1).powi(2))
        })
        .sum();
    Some(SpotMoments {
        centroid: c,
        rms: (s2 / sw).sqrt(),
        weight: sw,
    })
}
