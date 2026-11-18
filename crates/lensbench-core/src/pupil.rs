//! 瞳の格子（[`PupilGrid`]）と瞳関数（[`PupilFunction`]）。
//!
//! 解析ごとに書いていた「格子を作る → 単位円で切る → 光線と OPD を求める → 止まった点を除く」を 1 か所にまとめる
//! 。格子の式と単位円の判定はここにだけあり、波面マップ・Zernike・FFT PSF・Huygens PSF・干渉縞・RMS 波面は
//! [`PupilFunction::trace`] の結果を使う。格子の種類は OpticStudio の解析ごとの格子に合わせてあり、互いに 1 点ずれる
//! など値が違う（同じ一辺でも点が同じとは限らない）ので、種類は型で区別する。
//!
//! 標本の並びはどの格子も行 = y が外、x が内（[`PupilSamples`]）。

use crate::analysis::{opd_chief, to_ref_plane_point};
use crate::error::Result;
use crate::invalid;
use crate::math::V3;
use crate::system::{FieldSel, RayBuf, System, Trace};
use std::sync::{Arc, Mutex};

/// 単位円の判定（境界の点の扱い）。格子によって決まっていて、変えると縁の点が増減する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rim {
    /// x² + y² ≤ 1
    Closed,
    /// x² + y² ≤ 1 + [`crate::tol::PUPIL_RIM`]（丸めで縁の点を落とさない）
    Padded,
}

impl Rim {
    /// 点 (x, y) が単位円内か
    #[inline]
    pub fn inside(self, x: f64, y: f64) -> bool {
        match self {
            Rim::Closed => x * x + y * y <= 1.0,
            Rim::Padded => x * x + y * y <= 1.0 + crate::tol::PUPIL_RIM,
        }
    }
}

/// 瞳の格子の種類（正規化瞳の座標）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PupilGrid {
    /// OpticStudio の Wavefront Map・Zernike の格子 p_i = (i − n/2)/(n/2 − 1)（n は 4 以上の偶数、[`Rim::Closed`]）
    WavefrontMap(usize),
    /// OpticStudio の Interferogram の格子 p_i = (i − n/2 + 1)/(n/2 − 1)（n は 4 以上の偶数、[`Rim::Closed`]）。
    /// Wavefront Map の格子とは 1 点ずれる（OpticStudio 2026 R1 で確認）
    Interferogram(usize),
    /// −1〜1 を n − 1 等分した p_i = −1 + 2i/(n − 1)（n は 2 以上）
    Linear { n: usize, rim: Rim },
    /// 直径を n 区間に分けた区間の中心 c_i = −1 + h(i + 0.5)、h = 2/n（FFT PSF・Huygens PSF）。単位円にかかる区間だけを
    /// 使い、光線は区間の円内の部分の中心（近似）で追う。重みは区間の円内の面積の割合
    /// （[`crate::psf::cell_frac`]、8×8 の小区間で数える）
    Cells(usize),
    /// [`PupilGrid::Cells`] の区間の中心 c を p = q c で写した格子（像面の上の FFT PSF で、J に非対角の項があるとき。
    /// [`crate::psf::image_shear`]）。重みは写した区間の円内の面積の割合
    Sheared { n: usize, q: [[f64; 2]; 2] },
    /// 瞳のガウス求積: r² に Gauss-Legendre（rings）× 等間隔の角度（arms、None で 4 rings + 2）。重みの和は 1
    Gauss { rings: usize, arms: Option<usize> },
}

/// 格子の標本（行 = y が外、x が内の順。Gauss はリングが外、腕が内）
#[derive(Debug, Clone, Default)]
pub struct PupilSamples {
    pub px: Vec<f64>,
    pub py: Vec<f64>,
    /// 格子の番号 (ix, iy)（Gauss は (腕, リング)）
    pub cell: Vec<(usize, usize)>,
    /// 重み（Cells は区間の円内の面積の割合、Gauss は求積の重み、ほかは 1）
    pub wt: Vec<f64>,
}

impl PupilSamples {
    pub fn len(&self) -> usize {
        self.px.len()
    }
    pub fn is_empty(&self) -> bool {
        self.px.is_empty()
    }
}

impl PupilGrid {
    /// 一辺の点の数（Gauss は None）
    pub fn side(&self) -> Option<usize> {
        match *self {
            PupilGrid::WavefrontMap(n)
            | PupilGrid::Interferogram(n)
            | PupilGrid::Linear { n, .. }
            | PupilGrid::Cells(n)
            | PupilGrid::Sheared { n, .. } => Some(n),
            PupilGrid::Gauss { .. } => None,
        }
    }

    /// 大きさを確かめる（WavefrontMap・Interferogram は 4 以上の偶数、Linear は 2 以上、Cells・Gauss は 1 以上）
    pub fn check(&self) -> Result<()> {
        match *self {
            PupilGrid::WavefrontMap(n) | PupilGrid::Interferogram(n) => {
                if n < 4 || !n.is_multiple_of(2) {
                    invalid!("grid side {n} must be an even number 4 or more");
                }
            }
            PupilGrid::Linear { n, .. } => {
                if n < 2 {
                    invalid!("the pupil grid must be 2 or more");
                }
            }
            PupilGrid::Cells(n) | PupilGrid::Sheared { n, .. } => {
                if n < 1 {
                    invalid!("the pupil grid must be 1 or more");
                }
            }
            PupilGrid::Gauss { rings, .. } => {
                if rings == 0 {
                    invalid!("the number of rings must be 1 or more");
                }
            }
        }
        Ok(())
    }

    /// 格子の x・y 共通の座標（番号 i の点、Cells・Sheared は区間の中心 c。Gauss は None）
    pub fn axis(&self) -> Option<Vec<f64>> {
        Some(match *self {
            PupilGrid::WavefrontMap(n) => {
                let h = (n / 2) as f64;
                (0..n).map(|i| (i as f64 - h) / (h - 1.0)).collect()
            }
            PupilGrid::Interferogram(n) => {
                let h = (n / 2) as f64 - 1.0;
                (0..n).map(|i| (i as f64 - h) / h).collect()
            }
            PupilGrid::Linear { n, .. } => (0..n)
                .map(|i| -1.0 + 2.0 * i as f64 / (n - 1) as f64)
                .collect(),
            PupilGrid::Cells(n) | PupilGrid::Sheared { n, .. } => {
                let h = 2.0 / n as f64;
                (0..n).map(|i| -1.0 + h * (i as f64 + 0.5)).collect()
            }
            PupilGrid::Gauss { .. } => return None,
        })
    }

    /// 単位円内の標本
    pub fn samples(&self) -> PupilSamples {
        let mut s = PupilSamples::default();
        match *self {
            PupilGrid::WavefrontMap(_) | PupilGrid::Interferogram(_) | PupilGrid::Linear { .. } => {
                let rim = match *self {
                    PupilGrid::Linear { rim, .. } => rim,
                    _ => Rim::Closed,
                };
                let g = self.axis().unwrap_or_default();
                for (iy, &y) in g.iter().enumerate() {
                    for (ix, &x) in g.iter().enumerate() {
                        if rim.inside(x, y) {
                            s.px.push(x);
                            s.py.push(y);
                            s.cell.push((ix, iy));
                            s.wt.push(1.0);
                        }
                    }
                }
            }
            PupilGrid::Cells(n) | PupilGrid::Sheared { n, .. } => {
                let h = 2.0 / n as f64;
                let g = self.axis().unwrap_or_default();
                for (iy, &cy) in g.iter().enumerate() {
                    for (ix, &cx) in g.iter().enumerate() {
                        let (f, qx, qy) = match *self {
                            PupilGrid::Sheared { q, .. } => {
                                crate::psf::cell_frac_mapped(q, cx, cy, h)
                            }
                            _ => crate::psf::cell_frac(cx, cy, h),
                        };
                        if f > 0.0 {
                            s.px.push(qx);
                            s.py.push(qy);
                            s.cell.push((ix, iy));
                            s.wt.push(f);
                        }
                    }
                }
            }
            PupilGrid::Gauss { rings, arms } => {
                let arms = arms.unwrap_or(4 * rings + 2);
                let (u, wu) = crate::analysis::gauss_legendre(rings);
                for i in 0..rings {
                    let r = ((u[i] + 1.0) / 2.0).sqrt();
                    for a in 0..arms {
                        let ang = (a as f64 + 0.5) * 2.0 * std::f64::consts::PI / arms as f64;
                        s.px.push(r * ang.cos());
                        s.py.push(r * ang.sin());
                        s.cell.push((a, i));
                        s.wt.push(wu[i] / 2.0 / arms as f64);
                    }
                }
            }
        }
        s
    }
}

/// OPD の基準の面。選び方は [`RefSurface::of`] の 1 か所（OPD の値・最適化の微分・Huygens PSF が同じものを使う）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RefSurface {
    /// 射出瞳の参照球: 中心（参照の主光線の像点）、半径、σ（球が像点の前なら +1、後ろなら −1）
    Sphere { center: V3, radius: f64, sign: f64 },
    /// 主光線に垂直な平面（ROPD「無限遠」・射出瞳が無限遠、アフォーカル像空間）: 平面の上の点と法線（主光線の向き）
    Plane { point: V3, normal: V3 },
}

/// 射出瞳の位置 expp（像面から）が有限で、参照球を置ける近さか（|expp| ≤ [`crate::tol::EXPP_FAR`]）
#[inline]
pub fn exit_pupil_near(expp: f64) -> bool {
    expp.is_finite() && expp.abs() <= crate::tol::EXPP_FAR
}

/// OPD の基準の設定（ROPD）が扱えるか: 射出瞳（2）か無限遠（1）、またはアフォーカル像空間
pub fn check_ropd(s: &System) -> Result<()> {
    if !s.afocal && s.ropd != 1 && s.ropd != 2 {
        crate::notimpl!("OPD reference {} (exit pupil or infinity only)", s.ropd);
    }
    Ok(())
}

/// OPD を求められるか: [`check_ropd`] に加え、射出瞳を基準にするなら射出瞳の位置が求まっていること
pub fn check_opd_reference(s: &System) -> Result<()> {
    check_ropd(s)?;
    if !s.afocal && s.ropd != 1 && s.exit_pupil().is_nan() {
        invalid!(
            "the exit pupil can't be found (the pupil helper rays can't be solved); no OPD reference"
        );
    }
    Ok(())
}

impl RefSurface {
    /// アフォーカルでない系で、基準が主光線に垂直な平面か（ROPD が無限遠、または射出瞳が遠い・無限遠）
    pub fn plane_for(s: &System) -> bool {
        !s.afocal && (s.ropd == 1 || !exit_pupil_near(s.exit_pupil()))
    }

    /// 参照の主光線の像点 pc・向き kc から
    pub fn of(s: &System, pc: V3, kc: V3) -> RefSurface {
        let expp = s.exit_pupil();
        if s.afocal {
            return RefSurface::Plane {
                point: to_ref_plane_point(expp, pc, kc),
                normal: kc,
            };
        }
        if RefSurface::plane_for(s) {
            return RefSurface::Plane {
                point: pc,
                normal: kc,
            };
        }
        let sc = (expp - pc[2]) / kc[2];
        RefSurface::Sphere {
            center: pc,
            radius: sc.abs(),
            sign: if sc < 0.0 { 1.0 } else { -1.0 },
        }
    }

    /// 像面の点 q・向き k・光路長 opl の光線を、像空間（屈折率 ni）で基準の面まで延ばした光路長。基準の球に
    /// 届かない光線は NaN（接点に丸めない）
    #[inline]
    pub fn opl(&self, ni: f64, q: V3, k: V3, opl: f64) -> f64 {
        use crate::math::{dot, sub};
        match *self {
            RefSurface::Plane { point, normal } => {
                opl + ni * dot(sub(point, q), normal) / dot(k, normal)
            }
            RefSurface::Sphere {
                center,
                radius,
                sign,
            } => {
                let dd = sub(q, center);
                let b = dot(dd, k);
                let c = dot(dd, dd) - radius * radius;
                let disc = (b * b - c).sqrt();
                let t = if sign > 0.0 { -b - disc } else { -b + disc };
                opl + ni * t
            }
        }
    }
}

/// 1 つの視野・波長の瞳関数: 格子の標本ごとの光線と OPD [波]
#[derive(Debug)]
pub struct PupilFunction {
    pub field: FieldSel,
    /// 波長の番号
    pub wave: usize,
    /// 参照球の中心が主波長の主光線か（false なら波長 wave 自身の主光線）
    pub ref_primary: bool,
    pub grid: PupilGrid,
    pub samples: PupilSamples,
    /// 標本ごとの OPD [波]（(参照の主光線の光路 − 光線の光路) / λ。使えない標本の値は意味を持たない。[`Self::mask`]）
    pub opd: Vec<f64>,
    /// 使える標本（像面に届き、OPD が有限）
    pub mask: Vec<bool>,
    /// 標本の光線の列（波長 wave、[`crate::system::Cols::WAVE`]）
    pub rays: RayBuf,
    /// 参照球の中心に使った主光線の追跡（ref_primary なら主波長、でなければ波長 wave）
    pub chief: Trace,
    /// OPD の基準の面
    pub reference: RefSurface,
    /// 標本ごとの振幅（系のアポダイゼーション × 選んだ膜・偏光の振幅、[`WaveAmp`]。どこでも 1 なら全部 1.0）。
    /// 区間の面積の割合などの重み（samples.wt）は含まない
    pub amp: Vec<f64>,
}

impl PupilFunction {
    /// 視野 field・波長 wave（None で主波長）の格子 grid の光線を追い、OPD を求める。解析が瞳の格子の光線を追うのは
    /// ここだけ。結果は系の中にしまい（[`PupilCache`]）、同じ系・同じ引数ならもう 1 度は追わない（系を直すと捨てる）
    pub fn trace(
        s: &System,
        field: FieldSel,
        wave: Option<usize>,
        grid: PupilGrid,
        ref_primary: bool,
    ) -> Result<Arc<PupilFunction>> {
        Self::trace_amp(s, field, wave, grid, ref_primary, None)
    }

    /// [`Self::trace`] に、膜・偏光の振幅 extra（[`PupilAmp`]）を足す。extra があるときはしまわない
    pub fn trace_amp(
        s: &System,
        field: FieldSel,
        wave: Option<usize>,
        grid: PupilGrid,
        ref_primary: bool,
        extra: Option<&dyn PupilAmp>,
    ) -> Result<Arc<PupilFunction>> {
        grid.check()?;
        if extra.is_some() {
            return Ok(Arc::new(Self::trace_new(
                s,
                field,
                wave,
                grid,
                ref_primary,
                extra,
            )?));
        }
        let key = PupilKey {
            field,
            wave: wave.unwrap_or(s.primary),
            grid,
            ref_primary,
        };
        let cache = s.pupil_cache();
        if let Some(p) = cache.get(&key) {
            return Ok(p);
        }
        let p = Arc::new(Self::trace_new(s, field, wave, grid, ref_primary, None)?);
        cache.put(key, &p);
        Ok(p)
    }

    /// 瞳の格子の光線を追う（しまわない）
    fn trace_new(
        s: &System,
        field: FieldSel,
        wave: Option<usize>,
        grid: PupilGrid,
        ref_primary: bool,
        extra: Option<&dyn PupilAmp>,
    ) -> Result<PupilFunction> {
        let samples = grid.samples();
        let (op, chief) = opd_chief(s, field, &samples.px, &samples.py, wave, ref_primary)?;
        let mask = (0..samples.len())
            .map(|q| op.ok(q) && op.opd[q].is_finite())
            .collect();
        let reference = RefSurface::of(s, chief.rays[0].xyz, chief.rays[0].lmn);
        let wa = WaveAmp::new(s, field, wave.unwrap_or(s.primary), extra)?;
        let amp = if wa.is_uniform() {
            vec![1.0; samples.len()]
        } else {
            (0..samples.len())
                .map(|q| wa.at(samples.px[q], samples.py[q]))
                .collect()
        };
        Ok(PupilFunction {
            amp,
            field,
            wave: wave.unwrap_or(s.primary),
            ref_primary,
            grid,
            samples,
            opd: op.opd,
            mask,
            rays: op.rays,
            chief,
            reference,
        })
    }

    /// 標本の数
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// 使える標本の OPD（使えなければ None）
    #[inline]
    pub fn get(&self, q: usize) -> Option<f64> {
        self.mask[q].then(|| self.opd[q])
    }

    /// 格子の上の OPD [iy][ix]（瞳の外・使えない標本は NaN）。Gauss の格子では空
    pub fn map(&self) -> Vec<Vec<f64>> {
        let Some(n) = self.grid.side() else {
            return Vec::new();
        };
        let mut g = vec![vec![f64::NAN; n]; n];
        for (q, &(ix, iy)) in self.samples.cell.iter().enumerate() {
            if self.mask[q] {
                g[iy][ix] = self.opd[q];
            }
        }
        g
    }
}

/// [`PupilCache`] の鍵
#[derive(Debug, Clone, Copy, PartialEq)]
struct PupilKey {
    field: FieldSel,
    wave: usize,
    grid: PupilGrid,
    ref_primary: bool,
}

/// 系にしまう瞳関数（[`PupilFunction::trace`]）。系を直すと系のほかのキャッシュと一緒に捨てる。同じ視野・波長・
/// 格子で波面・Zernike・PSF などを続けて求めるときに、格子の光線を 1 度だけ追う。しまうのは標本が
/// [`PupilCache::MAX_SAMPLES`] 以下の格子だけで、全部で [`PupilCache::MAX_TOTAL`] 標本・[`PupilCache::MAX_ENTRIES`] 個を超えたら
/// 古いものから捨てる
#[derive(Default)]
pub(crate) struct PupilCache {
    entries: Mutex<Vec<(PupilKey, Arc<PupilFunction>)>>,
}

impl std::fmt::Debug for PupilCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.entries.lock().map_or(0, |e| e.len());
        write!(f, "PupilCache({n})")
    }
}

impl PupilCache {
    /// しまう格子の標本の数の上限（512 × 512）
    const MAX_SAMPLES: usize = 1 << 18;
    /// しまう標本の合計の上限（1 標本 120 バイトほど。ネイティブで約 120 MB、Web 版はメモリが狭いので約 30 MB）
    #[cfg(not(target_arch = "wasm32"))]
    const MAX_TOTAL: usize = 1 << 20;
    #[cfg(target_arch = "wasm32")]
    const MAX_TOTAL: usize = 1 << 18;
    /// しまう数の上限
    const MAX_ENTRIES: usize = 8;

    fn get(&self, k: &PupilKey) -> Option<Arc<PupilFunction>> {
        let mut e = self.entries.lock().ok()?;
        let i = e.iter().position(|(q, _)| q == k)?;
        // 最後に使ったものを末尾に（捨てるのは先頭から）
        let hit = e.remove(i);
        let p = hit.1.clone();
        e.push(hit);
        Some(p)
    }

    fn put(&self, k: PupilKey, p: &Arc<PupilFunction>) {
        if p.len() > Self::MAX_SAMPLES {
            return;
        }
        let Ok(mut e) = self.entries.lock() else {
            return;
        };
        e.retain(|(q, _)| *q != k);
        e.push((k, p.clone()));
        while e.len() > Self::MAX_ENTRIES
            || e.iter().map(|(_, p)| p.len()).sum::<usize>() > Self::MAX_TOTAL
        {
            e.remove(0);
        }
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut e) = self.entries.lock() {
            e.clear();
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 振幅

/// 瞳のアポダイゼーション（系の設定。.zmx の系の行 GFAC 係数 種類、OpticStudio の System Explorer の Aperture の
/// Apodization Type・Apodization Factor）。解析の瞳関数の振幅に常に入る（FFT PSF・Huygens PSF・回折 MTF）。
///
/// 式は OpticStudio のヘルプ（System Explorer > Aperture）の定義に合わせる:
/// - Gaussian: 「瞳の上で振幅がガウス形に変わる。アポダイゼーション係数は振幅が瞳の半径の座標で減る速さ」。
///   振幅 A(ρ) = exp(−G ρ²)（ρ は正規化瞳の半径、G が係数。瞳の中心で 1）。G = 1 で縁の振幅が 1/e（強度 1/e²）、
///   強度が 1/e² になる半径は ρ₀ = 1/√G（ヘルプの例「係数 3 なら入射瞳の直径が 1/e² の幅の √3 倍」と同じ）。
/// - Cosine Cubed: 「点光源が平面を照らすときの強度の落ち方」。強度 I(θ) = cos³θ（θ は z 軸と、入射瞳と交わる
///   光線の角）を瞳の中心で 1 に正規化（I/I_中心）。振幅は √I。θ は物点から近軸の入射瞳の面の点
///   (ρ_x r, ρ_y r)（r は入射瞳の半径）への向きで求める。無限遠物体ではどの光線も同じ向きなので一様になる。
///
/// OpticStudio 26.1 と照合済み（Cooke 40 の Strehl が OS の FFT PSF 瞳 512 と 0.3% 以内。試験は解析的に確かめたもの）
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(schemars::JsonSchema)]
pub enum Apodization {
    /// 一様（.zmx の GFAC _ 0）
    #[default]
    Uniform,
    /// ガウス A(ρ) = exp(−G ρ²)（GFAC G 1）
    Gaussian(f64),
    /// cos³θ の強度（GFAC _ 2）
    CosineCubed,
}

impl Apodization {
    /// .zmx の値から: kind は種類（0 一様・1 ガウス・2 Cosine Cubed。名前 UNIFORM・GAUSSIAN・COSINE も受ける）、g はガウスの
    /// 係数 G。OpticStudio の .zmx は系の行 GFAC 係数 種類
    pub fn from_zmx(kind: &str, g: f64) -> crate::Result<Apodization> {
        let t = kind.to_ascii_uppercase();
        Ok(match t.as_str() {
            "0" | "UNIFORM" => Apodization::Uniform,
            "1" | "GAUSSIAN" => {
                if !g.is_finite() || g < 0.0 {
                    crate::invalid!(
                        "zmx apodization factor {g}: the Gaussian factor must be 0 or more"
                    );
                }
                Apodization::Gaussian(g)
            }
            "2" | "COSINE" | "COSINE_CUBED" => Apodization::CosineCubed,
            _ => crate::notimpl!("zmx apodization type {t} (0, 1 or 2)"),
        })
    }

    pub fn is_uniform(&self) -> bool {
        match *self {
            Apodization::Uniform => true,
            Apodization::Gaussian(g) => g == 0.0,
            Apodization::CosineCubed => false,
        }
    }
}

/// 解析が選んで足す瞳の上の振幅（膜の透過率・偏光。OpticStudio の「Use Polarization」に当たる）。
/// 波長 w・正規化瞳 (px, py) の振幅（強度の透過率の平方根）。有限でなければその点は光を通さない
pub trait PupilAmp: Sync {
    fn amp(&self, w: usize, px: f64, py: f64) -> f64;
}

/// 1 つの視野でのアポダイゼーションの振幅
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ApodMap {
    kind: ApodKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ApodKind {
    Uniform,
    Gauss(f64),
    /// 物点から入射瞳の面までの距離 l、入射瞳の半径 r、入射瞳の中心から見た物点の横のずれ (ox, oy)、
    /// 瞳の中心の cos θ
    Cos3 {
        l: f64,
        r: f64,
        ox: f64,
        oy: f64,
        c0: f64,
    },
}

impl ApodMap {
    /// 系 s の視野 field のアポダイゼーション（Cosine Cubed で有限物体なら主波長の主光線を追う）
    pub fn new(s: &System, field: FieldSel) -> Result<ApodMap> {
        let kind = match s.apodization {
            a if a.is_uniform() => ApodKind::Uniform,
            Apodization::Gaussian(g) => ApodKind::Gauss(g),
            Apodization::CosineCubed if s.infinite() => ApodKind::Uniform,
            _ => {
                let p = s.paraxial();
                let l = (p.enpp + s.surfaces[0].t).abs();
                let c = s.try_trace(field, &[0.0], &[0.0], Some(s.primary), None, false)?;
                if !c.traced(0) {
                    invalid!("cosine cubed apodization: the chief ray can't be traced");
                }
                // 主光線は物点から入射瞳の中心へ向かう（近軸）ので、物点は中心から −l (k_x, k_y) / k_z
                let k = c.k0[0];
                let (ox, oy) = (-l * k[0] / k[2], -l * k[1] / k[2]);
                let c0 = l / (ox * ox + oy * oy + l * l).sqrt();
                if !(l > 0.0 && c0 > 0.0 && p.epd > 0.0) {
                    invalid!("cosine cubed apodization: the object or entrance pupil position is undefined");
                }
                ApodKind::Cos3 {
                    l,
                    r: 0.5 * p.epd,
                    ox,
                    oy,
                    c0,
                }
            }
        };
        Ok(ApodMap { kind })
    }

    /// 振幅がどこでも 1 か
    pub fn is_uniform(&self) -> bool {
        self.kind == ApodKind::Uniform
    }

    /// 正規化瞳 (px, py) の振幅（瞳の中心で 1）
    pub fn amp(&self, px: f64, py: f64) -> f64 {
        match self.kind {
            ApodKind::Uniform => 1.0,
            ApodKind::Gauss(g) => (-g * (px * px + py * py)).exp(),
            ApodKind::Cos3 { l, r, ox, oy, c0 } => {
                let (dx, dy) = (px * r - ox, py * r - oy);
                let c = l / (dx * dx + dy * dy + l * l).sqrt();
                // 強度 (cos θ / cos θ₀)³ の平方根
                (c / c0).powf(1.5)
            }
        }
    }
}

/// 1 つの視野・波長の瞳の振幅（アポダイゼーション × 選んだ膜・偏光の振幅）。有限でない値は 0
#[derive(Clone, Copy)]
pub struct WaveAmp<'a> {
    pub apod: ApodMap,
    pub extra: Option<&'a dyn PupilAmp>,
    pub wave: usize,
}

impl<'a> WaveAmp<'a> {
    pub fn new(
        s: &System,
        field: FieldSel,
        wave: usize,
        extra: Option<&'a dyn PupilAmp>,
    ) -> Result<WaveAmp<'a>> {
        Ok(WaveAmp {
            apod: ApodMap::new(s, field)?,
            extra,
            wave,
        })
    }

    /// どこでも 1 か（膜・偏光を選ばず、アポダイゼーションが一様）
    pub fn is_uniform(&self) -> bool {
        self.extra.is_none() && self.apod.is_uniform()
    }

    /// 振幅が瞳の中心のまわりに回転対称か（膜・偏光を選ばず、アポダイゼーションが一様かガウス）
    pub fn rotationally_symmetric(&self) -> bool {
        self.extra.is_none() && matches!(self.apod.kind, ApodKind::Uniform | ApodKind::Gauss(_))
    }

    /// 正規化瞳 (px, py) の振幅
    #[inline]
    pub fn at(&self, px: f64, py: f64) -> f64 {
        let a = match self.extra {
            Some(e) => self.apod.amp(px, py) * e.amp(self.wave, px, py),
            None => self.apod.amp(px, py),
        };
        if a.is_finite() {
            a
        } else {
            0.0
        }
    }
}

/// [`wave_weight`] で透過率を平均する瞳の格子の一辺（膜・偏光の振幅は瞳の上で滑らかなので粗くてよい）
pub const TRANSMISSION_GRID: usize = 33;

/// 多色の PSF・MTF で波長 w に掛ける重み。一色（multi が false）は 1、多色は系の波長の重みに、膜・偏光の振幅
/// extra（なければ 1）の光量の透過率を掛けたもの。
///
/// 各波長の PSF・OTF は瞳を通った全光量で揃えてから重みで足すので、膜の透過率が波長で違うときの像面に届く光量の比
/// （重み × 透過率）はここで入れる。透過率は瞳の上（一辺 [`TRANSMISSION_GRID`] の区間の中心）で、系の
/// アポダイゼーションの強度を重みにした extra² の平均。extra が有限でない点（偏光の追跡で光線が届かない）は分母にも
/// 数えない（ケラレは振幅なしの多色と同じく波長ごとに揃えたまま）。どの点も届かなければ 0（その波長は足さない）
pub fn wave_weight(
    s: &System,
    field: FieldSel,
    w: usize,
    multi: bool,
    extra: Option<&dyn PupilAmp>,
) -> Result<f64> {
    if !multi {
        return Ok(1.0);
    }
    let k = s.weights[w];
    let Some(e) = extra else {
        return Ok(k);
    };
    if !(k > 0.0) {
        return Ok(k);
    }
    let apod = ApodMap::new(s, field)?;
    let sm = PupilGrid::Cells(TRANSMISSION_GRID).samples();
    let (mut num, mut den) = (0.0, 0.0);
    for q in 0..sm.len() {
        let (px, py) = (sm.px[q], sm.py[q]);
        let t = e.amp(w, px, py);
        if t.is_finite() {
            let fa = sm.wt[q] * apod.amp(px, py).powi(2);
            num += fa * t * t;
            den += fa;
        }
    }
    Ok(if den > 0.0 { k * num / den } else { 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_keep_their_formulas() {
        // Wavefront Map は中心が i = n/2、Interferogram は i = n/2 − 1（1 点ずれる）
        let w = PupilGrid::WavefrontMap(8).axis().unwrap();
        let f = PupilGrid::Interferogram(8).axis().unwrap();
        assert_eq!(w[4], 0.0);
        assert_eq!(f[3], 0.0);
        assert_eq!(w[1], -1.0);
        assert_eq!(f[0], -1.0);
        let l = PupilGrid::Linear {
            n: 5,
            rim: Rim::Closed,
        }
        .axis()
        .unwrap();
        assert_eq!(l, vec![-1.0, -0.5, 0.0, 0.5, 1.0]);
        let c = PupilGrid::Cells(4).axis().unwrap();
        assert_eq!(c, vec![-0.75, -0.25, 0.25, 0.75]);
        // 区間の重みの和は円の面積 / h²
        let s = PupilGrid::Cells(64).samples();
        let h = 2.0 / 64.0;
        let a: f64 = s.wt.iter().sum::<f64>() * h * h;
        assert!((a - std::f64::consts::PI).abs() < 2e-4, "{a}");
        // Gauss の重みの和は 1
        let g = PupilGrid::Gauss {
            rings: 6,
            arms: None,
        }
        .samples();
        assert_eq!(g.len(), 6 * 26);
        assert!((g.wt.iter().sum::<f64>() - 1.0).abs() < 1e-14);
    }

    #[test]
    fn rim_padding_keeps_points_rounded_off_the_circle() {
        // 一辺 11 の格子の (−0.8, 0.6) は −1 + 2·8/10 = 0.6000000000000001 で、丸めで 1 をわずかに超える
        let l = PupilGrid::Linear {
            n: 11,
            rim: Rim::Closed,
        }
        .axis()
        .unwrap();
        let (x, y) = (l[1], l[8]);
        assert!(x * x + y * y > 1.0);
        assert!(!Rim::Closed.inside(x, y) && Rim::Padded.inside(x, y));
        let n = |rim| PupilGrid::Linear { n: 11, rim }.samples().len();
        assert_eq!(n(Rim::Padded), n(Rim::Closed) + 4);
        assert!(Rim::Closed.inside(1.0, 0.0) && Rim::Padded.inside(1.0, 0.0));
    }

    #[test]
    fn grid_sizes_are_checked() {
        assert!(PupilGrid::WavefrontMap(6).check().is_ok());
        assert!(PupilGrid::WavefrontMap(5).check().is_err());
        assert!(PupilGrid::Interferogram(2).check().is_err());
        assert!(PupilGrid::Linear {
            n: 1,
            rim: Rim::Padded
        }
        .check()
        .is_err());
        assert!(PupilGrid::Gauss {
            rings: 0,
            arms: None
        }
        .check()
        .is_err());
    }
}
