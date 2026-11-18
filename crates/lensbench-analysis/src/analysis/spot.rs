//! スポット（RMS 半径・重心・主光線）

use super::pupil_gauss;
use lensbench_core::error::Result;
use lensbench_core::system::{Cols, FieldSel, RayBuf, System, Trace};
use lensbench_core::trace::RayCounts;
use lensbench_core::Waves;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Spot {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub wave: Vec<usize>,
    pub weight: Vec<f64>,
    pub centroid: (f64, f64),
    pub chief: (f64, f64),
    pub rms_centroid: f64,
    pub rms_chief: f64,
    pub geo: f64,
    /// 追跡した光線の本数と、除いた本数の理由（波長の分を足したもの。主光線は数えない）
    pub rays: RayCounts,
}

/// スポット（w None なら全波長を重み付きで）。RMS は OpticStudio の RSCE・RSCH と同じ（ガウス求積、けられた光線は除く）。
/// 系のアポダイゼーション（[`System::apodization`]）の強度を光線の重みに掛ける（重心・RMS。OpticStudio の RSCE・CENY・
/// スポット図の RMS と同じ）。腕の数は 4 rings + 2（[`spot_arms`] で指定できる）
pub fn spot(s: &System, field: FieldSel, w: Waves, rings: usize) -> Result<Spot> {
    let w = w.one();
    spot_arms(s, field, Waves::all_or(w), rings, None)
}

/// [`spot`] で、求積の腕（角度の分割）の数 arms を指定する（None なら 4 rings + 2）。OpticStudio の RSCE・RSCH は
/// 腕が 2 rings で、回転対称でない系（Zernike・Grid Sag など）では角度の求積が収束しきらず、腕を増やした値と
/// 相対 1e-5〜1e-4 違う（Some(2 * rings) で OpticStudio と丸め誤差まで合う）
pub fn spot_arms(
    s: &System,
    field: FieldSel,
    w: Waves,
    rings: usize,
    arms: Option<usize>,
) -> Result<Spot> {
    spot_pupil(s, field, w, &pupil_gauss(rings, arms))
}

/// [`spot`] を瞳の点 (px, py, 重み) で求める（[`super::pupil_gauss_os`] など）
pub fn spot_pupil(
    s: &System,
    field: FieldSel,
    w: Waves,
    pupil: &(Vec<f64>, Vec<f64>, Vec<f64>),
) -> Result<Spot> {
    let w = w.one();
    let (px, py, wts) = pupil;
    let ws: Vec<usize> = match w {
        None => (0..s.wavelengths.len()).collect(),
        Some(w) => vec![w],
    };
    let mut rs = Vec::with_capacity(ws.len());
    for &wi in &ws {
        let mut b = RayBuf::with_cols(Cols::XYZ | Cols::LMN);
        s.try_trace_into(field, px, py, Some(wi), None, &mut b)?;
        rs.push((wi, b));
    }
    let rc = s.try_trace(
        field,
        &[0.0],
        &[0.0],
        Some(w.unwrap_or(s.primary)),
        None,
        false,
    )?;
    let rs: Vec<(usize, &RayBuf)> = rs.iter().map(|(wi, r)| (*wi, r)).collect();
    let wts = apodized(s, field, px, py, wts)?;
    Ok(spot_of(s, &rs, &wts, &rc))
}

/// 求積の重み wts に、系のアポダイゼーションの強度 A(px, py)² を掛ける（一様なら wts のまま）。光線の着地点の
/// 平均・広がり（スポット・重心・幾何 MTF）は光の量で重みを付ける
pub fn apodized(
    s: &System,
    field: FieldSel,
    px: &[f64],
    py: &[f64],
    wts: &[f64],
) -> Result<Vec<f64>> {
    apodized_pow(s, field, px, py, wts, 2)
}

/// 求積の重み wts に、系のアポダイゼーションの振幅 A(px, py) を掛ける（一様なら wts のまま）。波面の RMS の重み:
/// 収差の無い瞳で規格化したストレール比 |∫A e^{2πiW}|² / (∫A)² を W の 2 次まで展開すると 1 − (2π)² Var(W) で、
/// その分散の重みは A（OpticStudio のガウス求積の RWCE・RWRE も A で、26.1 で 1e-5 で一致）
pub fn apodized_amplitude(
    s: &System,
    field: FieldSel,
    px: &[f64],
    py: &[f64],
    wts: &[f64],
) -> Result<Vec<f64>> {
    apodized_pow(s, field, px, py, wts, 1)
}

fn apodized_pow(
    s: &System,
    field: FieldSel,
    px: &[f64],
    py: &[f64],
    wts: &[f64],
    pow: i32,
) -> Result<Vec<f64>> {
    let a = lensbench_core::pupil::ApodMap::new(s, field)?;
    if a.is_uniform() {
        return Ok(wts.to_vec());
    }
    Ok(wts
        .iter()
        .zip(px.iter().zip(py))
        .map(|(&w, (&x, &y))| w * a.amp(x, y).powi(pow))
        .collect())
}

/// 波長ごとの追跡 rs（求積の点、重み wts。アポダイゼーションの強度は呼ぶ側で掛ける）と主光線の追跡 rc からスポットを作る
/// （2 波長以上なら波長の重みもかける）
pub(crate) fn spot_of(s: &System, rs: &[(usize, &RayBuf)], wts: &[f64], rc: &Trace) -> Spot {
    let (mut x, mut y, mut wt, mut wv) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut rays = RayCounts::default();
    for &(wi, r) in rs {
        rays.merge(&r.counts());
        for (i, &wq) in wts.iter().enumerate() {
            if r.ok(i) {
                let (a, b) = s.image_point(r.xyz[i], r.lmn[i]);
                x.push(a);
                y.push(b);
                // 単色は重みによらない（重み 0 の波長でも求める）
                wt.push(wq * if rs.len() == 1 { 1.0 } else { s.weights[wi] });
                wv.push(wi);
            }
        }
    }
    let chief = if rc.traced(0) {
        s.image_point(rc.rays[0].xyz, rc.rays[0].lmn)
    } else {
        (f64::NAN, f64::NAN)
    };
    let sw: f64 = wt.iter().sum();
    if x.is_empty() || !(sw > 0.0) {
        return Spot {
            x,
            y,
            wave: wv,
            weight: wt,
            centroid: (f64::NAN, f64::NAN),
            chief,
            rms_centroid: f64::NAN,
            rms_chief: f64::NAN,
            geo: f64::NAN,
            rays,
        };
    }
    let avg = |v: &dyn Fn(usize) -> f64| (0..x.len()).map(|i| v(i) * wt[i]).sum::<f64>() / sw;
    let cx = avg(&|i| x[i]);
    let cy = avg(&|i| y[i]);
    let (hx, hy) = chief;
    let r2c = |i: usize| (x[i] - cx).powi(2) + (y[i] - cy).powi(2);
    let rms_c = avg(&r2c).sqrt();
    let rms_h = avg(&|i| (x[i] - hx).powi(2) + (y[i] - hy).powi(2)).sqrt();
    let geo = (0..x.len()).map(r2c).fold(0.0, f64::max).sqrt();
    Spot {
        centroid: (cx, cy),
        chief,
        rms_centroid: rms_c,
        rms_chief: rms_h,
        geo,
        rays,
        x,
        y,
        wave: wv,
        weight: wt,
    }
}
