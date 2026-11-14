//! 横軸を振る解析: スルーフォーカス MTF、フィールド MTF、RMS の各種（視野・フォーカス・波長）。
//!
//! どれも [`crate::analysis`] の MTF・スポット・OPD を点ごとに呼ぶ。像面のずらしは像面の手前の面の厚さを
//! 変える（OpticStudio の Through Focus・RMS vs Focus と同じ。厚さのソルブは解き直さない）。視野の刻みは正規化
//! 視野で 0 から 1 まで（ケラレ係数のない系か、定義済みの視野の点だけ）。波長を振るときは、その波長を系の波長の
//! 末尾に重み 0 で足して単色で計算する（主波長・視野の定義は変えない）。
//!
//! 横軸のある点で計算できない（瞳の縁がけられる・光線が届かないなど）ときは、その点を NaN にして warnings に
//! 理由を残し、ほかの点は続ける。

use crate::analysis::{self, mtf, opd, pupil_gauss, spot};
use crate::error::Result;
use crate::glass;
use crate::invalid;
use crate::surface::SurfType;
use crate::system::{FieldSel, FieldType, System};
use crate::Waves;

// ------------------------------------------------------------------------------------------ 系の変更
/// 像面を dz [レンズ単位] ずらした系（像面の手前の面の厚さに dz を足す。+ は光の進む向きに遠ざける）
pub fn defocus(s: &System, dz: f64) -> Result<System> {
    if !dz.is_finite() {
        invalid!("像面のずらし {dz}");
    }
    let mut d = s.clone();
    if dz != 0.0 {
        let j = d.image() - 1;
        d.surfaces[j].t += dz;
        d.changed()?;
    }
    Ok(d)
}

/// 面 j の後の媒質の、波長 lam [µm] での屈折率（dn・disp をかける前）。prev は前の媒質の値（鏡・座標ブレーク用）。
/// 系の主波長の値と比べて、ガラスの温度の扱い（adjust）を系の値の作り方に合わせる
fn base_index_at(s: &System, j: usize, prev: f64, lam: f64) -> Result<f64> {
    let d = &s.surfaces[j];
    if j > 0 && (d.mirror || d.typ == SurfType::CoordBrk) {
        return Ok(prev);
    }
    if let Some(g) = &d.grin {
        return g.vertex_index(lam).ok_or_else(|| {
            crate::error::Error::Invalid(format!(
                "面 {j}: GRIN の頂点が分布の範囲の外（波長 {lam} µm）"
            ))
        });
    }
    let mat = d.mat.trim();
    if mat.is_empty() || mat.eq_ignore_ascii_case("AIR") {
        return Ok(1.0);
    }
    let n0 = s.base_index(j, s.primary);
    if let Some(g) = s.glasses.get(mat) {
        let (t, p) = (s.temperature, s.pressure);
        let wp = s.wavelengths[s.primary];
        // 系の値と同じ作り方を選ぶ（.zmx は Adjust Index の設定、プロジェクトはカタログの値のまま）
        for adjust in [s.adjust_index, !s.adjust_index] {
            if (g.index(wp, t, p, adjust) - n0).abs() <= 1e-12 {
                return Ok(g.index(lam, t, p, adjust));
            }
        }
        invalid!("面 {j}: ガラス {mat} の屈折率を波長 {lam} µm で作り直せない（系の値と合わない）");
    }
    // モデルガラス "nd,vd[,dpgf]"（vd = 0 は分散なし）
    let v: Vec<f64> = mat
        .split(',')
        .map(|x| x.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .unwrap_or_default();
    if (2..=3).contains(&v.len()) {
        if v[1] == 0.0 {
            return Ok(n0);
        }
        return Ok(glass::model_index(
            v[0],
            v[1],
            v.get(2).copied().unwrap_or(0.0),
            lam,
        ));
    }
    let nb = |w: usize| s.base_index(j, w);
    if (0..s.wavelengths.len()).all(|w| nb(w) == n0) {
        return Ok(n0);
    }
    invalid!("面 {j}: 材料 {mat} の屈折率を波長 {lam} µm で作れない");
}

impl System {
    /// 波長 i を lam [µm] に変える（(i, lam) の並び）。全媒質の屈折率（dn・disp をかける前の値と、複屈折材の異常光）を
    /// 新しい波長で作り直す。作り方は元の系の値に合わせる（[`base_index_at`]）。dn・disp の反映
    /// （[`System::update_indices`]）とキャッシュの扱いは呼ぶ側（[`System::set_params`]）で
    pub(crate) fn set_wavelengths(&mut self, waves: &[(usize, f64)]) -> Result<()> {
        let ns = self.surfaces.len();
        let mut new: Vec<(usize, Vec<f64>)> = Vec::with_capacity(waves.len());
        for &(i, lam) in waves {
            if i >= self.wavelengths.len() || !(lam > 0.0) {
                invalid!(
                    "波長 {} = {lam} µm（波長は {} 個）",
                    i + 1,
                    self.wavelengths.len()
                );
            }
            let mut prev = 1.0;
            let mut col = Vec::with_capacity(ns);
            for j in 0..ns {
                let n = base_index_at(self, j, prev, lam)?;
                col.push(n);
                if !self.surfaces[j].ignore || j == 0 {
                    prev = n;
                }
            }
            new.push((i, col));
        }
        for (i, col) in new {
            for (j, n) in col.into_iter().enumerate() {
                let d = &mut self.surfaces[j];
                let v = if d.n_base.is_empty() {
                    &mut d.n
                } else {
                    &mut d.n_base
                };
                if let Some(x) = v.get_mut(i) {
                    *x = n;
                }
            }
        }
        for &(i, lam) in waves {
            self.wavelengths[i] = lam;
        }
        // 複屈折材の異常光の屈折率は材料（名前-E）から作り直す
        let mut bire = false;
        for d in self.surfaces.iter_mut() {
            if let Some(crate::surface::Bire::In { ne, .. }) = &mut d.bire {
                ne.clear();
                bire = true;
            }
        }
        if bire {
            self.refresh_media()?;
        }
        Ok(())
    }
}

/// 波長 lam [µm] を末尾に重み 0 で足した系と、その波長の番号（0 始まり）。主波長・視野・エイミングは変えない
pub fn with_wavelength(s: &System, lam: f64) -> Result<(System, usize)> {
    if !(lam > 0.0) {
        invalid!("波長 {lam} µm");
    }
    let mut d = s.clone();
    let mut prev = 1.0;
    let mut add = Vec::with_capacity(d.surfaces.len());
    for j in 0..d.surfaces.len() {
        let n = base_index_at(s, j, prev, lam)?;
        add.push(n);
        if !s.surfaces[j].ignore || j == 0 {
            prev = n;
        }
    }
    for (j, n) in add.into_iter().enumerate() {
        let sf = &mut d.surfaces[j];
        if sf.n_base.is_empty() {
            sf.n.push(n);
        } else {
            sf.n_base.push(n);
            sf.n.push(f64::NAN);
        }
    }
    d.wavelengths.push(lam);
    d.weights.push(0.0);
    d.update_indices();
    d.changed()?;
    Ok((d, s.wavelengths.len()))
}

// ------------------------------------------------------------------------------------------ 刻み
/// 0 から 1 までの n 点（n = 1 なら [0]）
pub fn steps01(n: usize) -> Vec<f64> {
    if n <= 1 {
        return vec![0.0];
    }
    (0..n).map(|k| k as f64 / (n - 1) as f64).collect()
}

/// −range から +range までの n 点（n = 1 なら [0]）
pub fn steps_sym(range: f64, n: usize) -> Vec<f64> {
    if n <= 1 {
        return vec![0.0];
    }
    (0..n)
        .map(|k| -range + 2.0 * range * k as f64 / (n - 1) as f64)
        .collect()
}

/// 視野を振る向き
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FieldAxis {
    X,
    #[default]
    Y,
}

impl FieldAxis {
    pub fn sel(self, h: f64) -> FieldSel {
        match self {
            FieldAxis::X => FieldSel::Norm(h, 0.0),
            FieldAxis::Y => FieldSel::Norm(0.0, h),
        }
    }
    /// 正規化視野 h の視野の値（系の視野の単位: 角度 [°] か高さ [レンズ単位]）
    pub fn value(self, s: &System, h: f64) -> f64 {
        let (nx, ny) = s.field_norm_values();
        match self {
            FieldAxis::X => h * nx,
            FieldAxis::Y => h * ny,
        }
    }
}

// ------------------------------------------------------------------------------------------ MTF
/// 横軸を振った MTF。tan・sag は [周波数][横軸の点]
#[derive(Debug, Clone)]
pub struct MtfSweep {
    pub x: Vec<f64>,
    pub freqs: Vec<f64>,
    pub tan: Vec<Vec<f64>>,
    pub sag: Vec<Vec<f64>>,
    pub warnings: Vec<String>,
}

fn mtf_sweep(
    x: &[f64],
    freqs: &[f64],
    label: impl Fn(f64) -> String,
    one: impl Fn(f64) -> Result<analysis::Mtf> + Sync + Send,
) -> MtfSweep {
    let nf = freqs.len();
    let mut tan = vec![vec![f64::NAN; x.len()]; nf];
    let mut sag = tan.clone();
    let mut warnings = Vec::new();
    // 点ごとに独立（周波数が少ないと 1 点の中の並列（周波数×向き）だけではコアが余るので、点でも並列にする。値は同じ）
    let ms = crate::par::map(x, |&xi| one(xi));
    for ((i, &xi), m) in x.iter().enumerate().zip(ms) {
        match m {
            Ok(m) => {
                for k in 0..nf {
                    tan[k][i] = m.tan[k];
                    sag[k][i] = m.sag[k];
                }
            }
            Err(e) => warnings.push(format!("{}: {e}", label(xi))),
        }
    }
    MtfSweep {
        x: x.to_vec(),
        freqs: freqs.to_vec(),
        tan,
        sag,
        warnings,
    }
}

/// スルーフォーカス MTF（OpticStudio の FFT Through Focus MTF 相当）: 像面を dz ずつずらした回折 MTF。
/// w None なら多色、n は [`analysis::mtf`] の求積の点の数（None で自動）
pub fn through_focus_mtf(
    s: &System,
    field: FieldSel,
    w: Waves,
    n: Option<usize>,
    freqs: &[f64],
    dz: &[f64],
) -> MtfSweep {
    let w = w.one();
    mtf_sweep(
        dz,
        freqs,
        |z| format!("ずらし {z}"),
        |z| mtf(&defocus(s, z)?, field, Waves::all_or(w), n, freqs),
    )
}

/// 視野の掃引用: ケラレ係数を 0 にした系（OpticStudio の MTF vs Field・RMS vs Field は定義済みの視野の点でも
/// ケラレ係数を使わない。A6 で確かめた）。係数があれば警告を返す
fn without_vig(s: &System) -> (std::borrow::Cow<'_, System>, Option<String>) {
    let has = s
        .fields
        .iter()
        .any(|f| f.vdx != 0.0 || f.vdy != 0.0 || f.vcx != 0.0 || f.vcy != 0.0 || f.van != 0.0);
    if !has {
        return (std::borrow::Cow::Borrowed(s), None);
    }
    let mut c = s.clone();
    for f in c.fields.iter_mut() {
        (f.vdx, f.vdy, f.vcx, f.vcy, f.van) = (0.0, 0.0, 0.0, 0.0, 0.0);
    }
    (
        std::borrow::Cow::Owned(c),
        Some("ケラレ係数は使わない（視野の掃引では OpticStudio と同じく無視）".into()),
    )
}

/// フィールド MTF（OpticStudio の FFT MTF vs Field 相当）: 正規化視野 h（axis の向き）ごとの回折 MTF。ケラレ係数は使わない（OpticStudio と同じ）
pub fn field_mtf(
    s: &System,
    axis: FieldAxis,
    h: &[f64],
    w: Waves,
    n: Option<usize>,
    freqs: &[f64],
) -> MtfSweep {
    let w = w.one();
    let (s, warn) = without_vig(s);
    let mut r = mtf_sweep(
        h,
        freqs,
        |h| format!("視野 {h}"),
        |h| mtf(&s, axis.sel(h), Waves::all_or(w), n, freqs),
    );
    r.warnings.splice(0..0, warn);
    r
}

// ------------------------------------------------------------------------------------------ RMS
/// RMS の基準
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RmsRef {
    /// 主光線（スポットは主光線の像点、波面はピストンだけ除く）
    Chief,
    /// 重心（スポットは光線の重心、波面はピストンと、参照点を像面で動かした分の傾きを除く）
    #[default]
    Centroid,
}

/// RMS の設定
#[derive(Debug, Clone, Copy)]
pub struct RmsOpts {
    /// 瞳のガウス求積のリング数（腕は 4 rings + 2）
    pub rings: usize,
    pub reference: RmsRef,
    /// 単色の視野・波長の掃引で、実像高（と条件で定める視野）の視野を解析の波長の主光線で解く
    /// （OpticStudio の単色の RMS vs Field・RMS vs Wavelength と同じ。既定 false は主波長で解き、
    /// スポット・演算子・多色と同じ）
    pub field_wave: bool,
}

impl Default for RmsOpts {
    fn default() -> Self {
        RmsOpts {
            rings: 6,
            reference: RmsRef::Centroid,
            field_wave: false,
        }
    }
}

/// 1 点の RMS
#[derive(Debug, Clone, Copy)]
pub struct Rms {
    /// スポット半径の RMS（√(x² + y²)）・x 方向・y 方向 [レンズ単位]
    pub spot: f64,
    pub spot_x: f64,
    pub spot_y: f64,
    /// 波面収差の RMS [波]（多色は波長ごとの分散を重みで平均した平方根。基準の取り方は [`wfe_var`]）
    pub wfe: f64,
    /// ストレール比の近似 exp(−(2π·wfe)²)（多色は波長ごとの値の重み付き平均。OpticStudio は多色の値を出さない）
    pub strehl: f64,
}

impl Rms {
    const NAN: Rms = Rms {
        spot: f64::NAN,
        spot_x: f64::NAN,
        spot_y: f64::NAN,
        wfe: f64::NAN,
        strehl: f64::NAN,
    };
}

/// 波面の RMS [波] を波長ごとに（ガウス求積、けられた光線は除く）。OpticStudio の RMS vs Field などと同じ定義
/// （26.1 の値と 1e-7 で一致を確かめた）:
/// - 単色（poly = false、ws は 1 波長）: その波長自身の主光線を基準にした OPD。chief はピストンだけ除く。centroid は
///   ピストンと、像空間の方向余弦の主光線からのずれ (L − Lc, M − Mc) に比例する項（参照点を像面で動かしたときの
///   OPD の変化と同じ形）を重み付き最小二乗で除く（瞳座標 px・py の傾きではない）。
/// - 多色（poly = true）: 全波長の OPD を主波長の主光線を基準に。chief は波長ごとにピストンを除く。centroid は
///   波長ごとのピストンと、全波長に共通の (L − Lc, M − Mc) の係数 [波]（Lc は主波長の主光線）を一緒に当てはめて除く。
/// - アフォーカル（参照が平面波）の centroid は (L − Lc, M − Mc) の代わりに瞳座標 (px, py)（参照の平面を傾けた分）。
///
/// 返すのは波長ごとの分散（ws の順）
#[allow(clippy::too_many_arguments)]
fn wfe_var(
    s: &System,
    field: FieldSel,
    ws: &[usize],
    poly: bool,
    px: &[f64],
    py: &[f64],
    wt: &[f64],
    r: RmsRef,
) -> Result<Vec<f64>> {
    let nw = ws.len();
    // 波長ごとの (OPD, L − Lc, M − Mc, 重み)（重みは波長の中で和 1）
    let mut rows: Vec<Vec<(f64, f64, f64, f64)>> = Vec::with_capacity(nw);
    for &w in ws {
        let o = opd(s, field, px, py, Some(w), poly)?;
        let cw = if poly { s.primary } else { w };
        let c = s.try_trace(field, &[0.0], &[0.0], Some(cw), None, false)?;
        if !c.traced(0) {
            invalid!("主光線が像面に届かない");
        }
        let (lc, mc) = (c.rays[0].lmn[0], c.rays[0].lmn[1]);
        let idx: Vec<usize> = (0..px.len())
            .filter(|&i| o.ok(i) && o.opd[i].is_finite())
            .collect();
        let sw: f64 = idx.iter().map(|&i| wt[i]).sum();
        if idx.is_empty() || !(sw > 0.0) {
            invalid!("有効な光線がない（すべてけられた）");
        }
        rows.push(
            idx.iter()
                .map(|&i| {
                    // アフォーカルは参照が平面波なので、参照の向きを傾けた分は瞳の位置に比例（瞳座標で）
                    if s.afocal {
                        return (o.opd[i], px[i], py[i], wt[i] / sw);
                    }
                    let k = o.trace.rays[i].lmn;
                    (o.opd[i], k[0] - lc, k[1] - mc, wt[i] / sw)
                })
                .collect(),
        );
    }
    // 未知数: 波長ごとのピストン nw 個と、centroid なら傾きの係数 2 個（多色は全波長で共通、単色は 1 波長なので同じ）
    let nt = if r == RmsRef::Centroid { 2 } else { 0 };
    let m = nw + nt;
    let mut g = vec![vec![0.0; m]; m];
    let mut v = vec![0.0; m];
    for (j, rw) in rows.iter().enumerate() {
        for &(y, dl, dm, q) in rw {
            let mut b = vec![0.0; m];
            b[j] = 1.0;
            if nt == 2 {
                b[nw] = dl;
                b[nw + 1] = dm;
            }
            for p in 0..m {
                if b[p] == 0.0 {
                    continue;
                }
                v[p] += q * b[p] * y;
                for t in 0..m {
                    g[p][t] += q * b[p] * b[t];
                }
            }
        }
    }
    let c = solve(g, v);
    Ok(rows
        .iter()
        .enumerate()
        .map(|(j, rw)| {
            rw.iter()
                .map(|&(y, dl, dm, q)| {
                    let mut e = y - c[j];
                    if nt == 2 {
                        e -= c[nw] * dl + c[nw + 1] * dm;
                    }
                    q * e * e
                })
                .sum()
        })
        .collect())
}

/// 小さな連立一次方程式（部分ピボットの消去）。ピボットが 0 の未知数は 0 にする（傾きが決まらないとき）
pub(crate) fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    let scale = a
        .iter()
        .flatten()
        .fold(0.0f64, |x, y| x.max(y.abs()))
        .max(1e-300);
    let mut skip = vec![false; n];
    for c in 0..n {
        let p = (c..n)
            .max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))
            .unwrap();
        if !(a[p][c].abs() > 1e-14 * scale) {
            skip[c] = true;
            continue;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..n {
            let f = a[r][c] / a[c][c];
            if f == 0.0 {
                continue;
            }
            for k in c..n {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for c in (0..n).rev() {
        if skip[c] {
            continue;
        }
        let mut t = b[c];
        for k in c + 1..n {
            t -= a[c][k] * x[k];
        }
        x[c] = t / a[c][c];
    }
    x
}

/// 1 点の RMS（スポット半径・x・y、波面、ストレール比）。w None なら多色
pub fn rms(s: &System, field: FieldSel, w: Waves, o: RmsOpts) -> Result<Rms> {
    let w = w.one();
    if o.rings == 0 {
        invalid!("リング数は 1 以上");
    }
    let sp = spot(s, field, Waves::all_or(w), o.rings)?;
    let (rx, ry) = match o.reference {
        RmsRef::Chief => sp.chief,
        RmsRef::Centroid => sp.centroid,
    };
    let sw: f64 = sp.weight.iter().sum();
    let mom = |f: &dyn Fn(usize) -> f64| {
        ((0..sp.x.len()).map(|i| f(i) * sp.weight[i]).sum::<f64>() / sw).sqrt()
    };
    let sx = mom(&|i| (sp.x[i] - rx).powi(2));
    let sy = mom(&|i| (sp.y[i] - ry).powi(2));
    let ws: Vec<usize> = match w {
        None => (0..s.wavelengths.len()).collect(),
        Some(w) => vec![w],
    };
    let (px, py, wt) = pupil_gauss(o.rings, None);
    let ks: Vec<(usize, f64)> = ws
        .iter()
        .map(|&wi| (wi, if ws.len() == 1 { 1.0 } else { s.weights[wi] }))
        .filter(|x| x.1 > 0.0)
        .collect();
    let wsum: f64 = ks.iter().map(|x| x.1).sum();
    if !(wsum > 0.0) {
        invalid!("重みのある波長がない");
    }
    let wl: Vec<usize> = ks.iter().map(|x| x.0).collect();
    let vars = wfe_var(s, field, &wl, w.is_none(), &px, &py, &wt, o.reference)?;
    let (mut var, mut st) = (0.0, 0.0);
    for (&(_, k), &v) in ks.iter().zip(&vars) {
        var += k * v;
        st += k * (-(2.0 * std::f64::consts::PI).powi(2) * v).exp();
    }
    Ok(Rms {
        spot: sx.hypot(sy),
        spot_x: sx,
        spot_y: sy,
        wfe: (var / wsum).sqrt(),
        strehl: st / wsum,
    })
}

/// 横軸を振った RMS
#[derive(Debug, Clone)]
pub struct RmsSweep {
    pub x: Vec<f64>,
    pub rms: Vec<Rms>,
    pub warnings: Vec<String>,
}

fn rms_sweep(
    x: &[f64],
    label: impl Fn(f64) -> String,
    one: impl Fn(f64) -> Result<Rms> + Sync + Send,
) -> RmsSweep {
    let mut warnings = Vec::new();
    // 点ごとに独立なので並列（警告は点の順）
    let rms = crate::par::map(x, |&v| one(v))
        .into_iter()
        .zip(x)
        .map(|(r, &v)| {
            r.unwrap_or_else(|e| {
                warnings.push(format!("{}: {e}", label(v)));
                Rms::NAN
            })
        })
        .collect();
    RmsSweep {
        x: x.to_vec(),
        rms,
        warnings,
    }
}

/// o.field_wave のとき、実像高・条件で定める視野を波長 w の主光線で解く系（主波長・エイミングはそのまま）
fn field_at_wave(s: &System, w: usize, o: RmsOpts) -> Result<std::borrow::Cow<'_, System>> {
    let by_ray = matches!(s.field_type, FieldType::RealH | FieldType::Solve(_));
    if !o.field_wave || !by_ray || w == s.primary {
        return Ok(std::borrow::Cow::Borrowed(s));
    }
    let mut d = s.clone();
    d.field_wave = Some(w);
    d.changed()?;
    Ok(std::borrow::Cow::Owned(d))
}

/// RMS vs Field: 正規化視野 h（axis の向き）ごと。ケラレ係数は使わない（OpticStudio と同じ）
pub fn rms_vs_field(s: &System, axis: FieldAxis, h: &[f64], w: Waves, o: RmsOpts) -> RmsSweep {
    let w = w.one();
    let (s, warn) = without_vig(s);
    let s = match w {
        Some(w) => match field_at_wave(&s, w, o) {
            Ok(d) => std::borrow::Cow::Owned(d.into_owned()),
            Err(e) => {
                let mut r = rms_sweep(h, |h| format!("視野 {h}"), |_| Err(e.clone()));
                r.warnings.splice(0..0, warn);
                return r;
            }
        },
        None => s,
    };
    let mut r = rms_sweep(
        h,
        |h| format!("視野 {h}"),
        |h| rms(&s, axis.sel(h), Waves::all_or(w), o),
    );
    r.warnings.splice(0..0, warn);
    r
}

/// RMS vs Focus: 像面のずらし dz ごと
pub fn rms_vs_focus(s: &System, field: FieldSel, dz: &[f64], w: Waves, o: RmsOpts) -> RmsSweep {
    let w = w.one();
    rms_sweep(
        dz,
        |z| format!("ずらし {z}"),
        |z| rms(&defocus(s, z)?, field, Waves::all_or(w), o),
    )
}

/// RMS vs Wavelength: 波長 lams [µm] ごとの単色（主波長・視野の定義は系のまま。o.field_wave で実像高の視野を各波長で解く）
pub fn rms_vs_wavelength(s: &System, field: FieldSel, lams: &[f64], o: RmsOpts) -> RmsSweep {
    rms_sweep(
        lams,
        |l| format!("波長 {l} µm"),
        |l| {
            let (d, w) = with_wavelength(s, l)?;
            let d2: std::borrow::Cow<'_, System> = field_at_wave(&d, w, o)?;
            rms(&d2, field, Waves::One(w), o)
        },
    )
}

/// 系の波長の最小から最大までの n 点 [µm]
pub fn wavelength_steps(s: &System, n: usize) -> Vec<f64> {
    let lo = s.wavelengths.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = s.wavelengths.iter().cloned().fold(0.0, f64::max);
    steps01(n).iter().map(|t| lo + (hi - lo) * t).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve_singular_returns_weighted_mean() {
        // 点がすべて x = 0 の線上にあると特異。傾き x は 0 に、ピストンと y の傾きは当てはまる
        let pts = [(0.0, 3.0, 1.0), (0.0, 4.0, 2.0), (0.0, 5.0, 0.5)];
        let mut g = vec![vec![0.0; 3]; 3];
        let mut v = vec![0.0; 3];
        for &(x, y, w) in &pts {
            let f = [1.0, x, y];
            let z: f64 = 1.5 + 2.0 * y;
            for p in 0..3 {
                v[p] += w * f[p] * z;
                for q in 0..3 {
                    g[p][q] += w * f[p] * f[q];
                }
            }
        }
        let r = solve(g, v);
        assert!(
            (r[0] - 1.5).abs() < 1e-12 && r[1] == 0.0 && (r[2] - 2.0).abs() < 1e-12,
            "{r:?}"
        );
        // 傾きの列がすべて 0 ならピストンは重み付き平均
        let r = solve(
            vec![vec![2.0, 0.0, 0.0], vec![0.0; 3], vec![0.0; 3]],
            vec![3.0, 0.0, 0.0],
        );
        assert_eq!(r, vec![1.5, 0.0, 0.0]);
    }

    #[test]
    fn solve_recovers_a_plane() {
        let pts = [(0.1, 0.2), (-0.5, 0.3), (0.7, -0.4), (0.0, 0.0), (0.3, 0.9)];
        let (a, b, c) = (0.3, -1.2, 2.5);
        let mut g = vec![vec![0.0; 3]; 3];
        let mut v = vec![0.0; 3];
        for &(x, y) in &pts {
            let f = [1.0, x, y];
            let z = a + b * x + c * y;
            for p in 0..3 {
                v[p] += f[p] * z;
                for q in 0..3 {
                    g[p][q] += f[p] * f[q];
                }
            }
        }
        let r = solve(g, v);
        assert!((r[0] - a).abs() < 1e-12 && (r[1] - b).abs() < 1e-12 && (r[2] - c).abs() < 1e-12);
    }

    #[test]
    fn steps() {
        assert_eq!(steps01(3), vec![0.0, 0.5, 1.0]);
        assert_eq!(steps_sym(0.1, 3), vec![-0.1, 0.0, 0.1]);
        assert_eq!(steps_sym(0.1, 1), vec![0.0]);
    }
}
