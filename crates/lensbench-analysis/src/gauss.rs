//! 近軸ガウスビーム（OpticStudio の Paraxial Gaussian Beam に当たる）。
//!
//! 物体空間で与えたウエスト（半径 w₀・面 1 からの距離・M²）の複素ビームパラメータ q を、近軸の ABCD 行列で各面へ
//! 運ぶ。ABCD は近軸光線 2 本（[`System::ptrace`]、高さ 1・傾き 0 と、高さ 0・換算傾き 1）から作る。光線は
//! (y, n u)（換算傾き）で表し、q も換算した q̂ = q / n で運ぶ（q̂′ = (A q̂ + B) / (C q̂ + D)）。与えるウエストは
//! 埋め込みのガウスビーム（基本モード）のもので、M² の混合モードのビームは基本モードの √M² 倍の径・ウエスト・広がり角、
//! 同じレイリー長・曲率半径・位置を持つ（OpticStudio の Paraxial Gaussian Beam と同じ約束。実測で確かめた）。
//!
//! 位置（ウエストから面へ）・曲率半径は面の局所 z の向きで測る（奇数回の反射の後は符号が逆。OpticStudio と同じ）。
//! 輪郭の位置 z は展開した向き（鏡の後も光の進む向きが正）。λ は系の波長（空気に対する相対、屈折率と同じ基準）。

use lensbench_core::{Error, Result, System};
use rustfft::num_complex::Complex64;

/// 入射のビーム
#[derive(Debug, Clone, Copy)]
pub struct Beam {
    /// 波長の番号（0 始まり）
    pub wave: usize,
    /// 埋め込みのガウスビーム（基本モード）のウエストの半径（1/e² の強度）。混合モードのビームのウエストは √M² 倍
    pub waist: f64,
    /// 面 1 の頂点からウエストまでの距離（物体空間、光の進む向きが正。負ならウエストは面 1 より前）
    pub surf1_to_waist: f64,
    /// M²（1 以上）
    pub m2: f64,
}

/// 面の後の空間のビーム（面の頂点の平面で）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeamAt {
    /// 面の番号
    pub surface: usize,
    /// 面の頂点でのビームの半径（1/e²、基本モード）
    pub size: f64,
    /// ウエストの半径（基本モード）
    pub waist: f64,
    /// ウエストから面の頂点までの距離（面の局所 z の向き。反射が偶数回なら、負は面がウエストより前＝ウエストはこの先。
    /// OpticStudio の Position と同じ）
    pub position: f64,
    /// 面の頂点での波面の曲率半径（ウエストの上なら無限大。面の局所 z の向きで、反射が偶数回なら正は発散）
    pub radius: f64,
    /// 漸近線の半角 atan(w₀ / z_R) [rad]（媒質の中、基本モード）
    pub divergence: f64,
    /// レイリー長（媒質の中）
    pub rayleigh: f64,
    /// 面の後の媒質の屈折率
    pub index: f64,
    /// 面の頂点の、展開した面 1 の頂点からの位置
    pub z: f64,
}

/// ガウスビームを各面へ運んだ結果
#[derive(Debug, Clone)]
pub struct GaussOut {
    /// 面 1 から像面までの各面の後（座標ブレーク・無視する面は直前の面と同じ値）
    pub rows: Vec<BeamAt>,
    /// 光軸に沿ったビームの半径の輪郭: 面と面の間ごとの (展開した面 1 からの距離, 半径) の並び。GRIN の媒質・
    /// 無限の間隔・厚さ 0 の間は入れない
    pub profile: Vec<Vec<[f64; 2]>>,
}

/// 媒質の中の q（ウエストからの距離 z + i z_R）から、面の後のビーム
fn beam_at(surface: usize, q: Complex64, n: f64, lam: f64, zpos: f64) -> BeamAt {
    let (z, zr) = (q.re, q.im);
    // 基本モード: z_R = π w₀² n / λ
    let waist = (zr * lam / (std::f64::consts::PI * n)).sqrt();
    let size = waist * (1.0 + (z / zr).powi(2)).sqrt();
    let radius = if z == 0.0 {
        f64::INFINITY
    } else {
        z * (1.0 + (zr / z).powi(2))
    };
    BeamAt {
        surface,
        size,
        waist,
        position: z,
        radius,
        // 漸近線の角度（OpticStudio と同じく atan。w₀/z_R との差は 3 次）
        divergence: (waist / zr).atan(),
        rayleigh: zr,
        index: n,
        z: zpos,
    }
}

/// 面 j の後の屈折率（頂点で。GRIN の媒質は頂点の屈折率）
fn index_after(s: &System, j: usize, w: usize) -> f64 {
    let d = &s.surfaces[j];
    if !d.is_mirror() {
        if let Some(n) = d.grin().and_then(|g| g.vertex_index(s.wavelengths[w])) {
            return n;
        }
    }
    s.index_before(j + 1, w)
}

/// 近軸ガウスビームを面 1 から像面まで運ぶ
pub fn gauss(s: &System, b: &Beam) -> Result<GaussOut> {
    if b.wave >= s.wavelengths.len() {
        return Err(Error::Invalid(format!("no wavelength {}", b.wave + 1)));
    }
    if !(b.waist > 0.0 && b.waist.is_finite()) {
        return Err(Error::Invalid(format!(
            "the waist radius must be positive ({})",
            b.waist
        )));
    }
    if !(b.m2 >= 1.0 && b.m2.is_finite()) {
        return Err(Error::Invalid(format!("M² must be 1 or more ({})", b.m2)));
    }
    if !b.surf1_to_waist.is_finite() {
        return Err(Error::Invalid(
            "the distance to the waist must be finite".into(),
        ));
    }
    let w = b.wave;
    let img = s.image();
    let lam = s.wave(w).mm();
    let n0 = s.index_before(1, w);
    // 換算傾き 1 は物体空間の傾き 1 / n0
    let (ya, ua) = s.ptrace(1.0, 0.0, w, None);
    let (yb, ub) = s.ptrace(0.0, 1.0 / n0, w, None);
    let pi = std::f64::consts::PI;
    // 面 1 の頂点（屈折の前）の q̂ = q / n0。面 1 はウエストから −surf1_to_waist の所
    let q1 = Complex64::new(-b.surf1_to_waist, pi * b.waist * b.waist * n0 / lam) / n0;
    let mut rows = Vec::with_capacity(img);
    let mut profile = Vec::new();
    let (mut zpos, mut sgn) = (0.0, 1.0);
    for j in 1..=img {
        let d = &s.surfaces[j];
        let n = index_after(s, j, w);
        let (a, bb) = (ya[j], yb[j]);
        let (c, dd) = (n * ua[j], n * ub[j]);
        let qh = (q1 * a + bb) / (q1 * c + dd);
        if !(qh.re.is_finite() && qh.im.is_finite() && qh.im > 0.0) {
            return Err(Error::Invalid(format!(
                "the paraxial trace fails at surface {j} (the beam can't be carried)"
            )));
        }
        let q = qh * n;
        if !d.ignore && d.is_mirror() {
            sgn = -sgn;
        }
        // 位置・曲率半径は面の局所 z の向きで（奇数回の反射の後は光が −z へ進むので符号が逆。OpticStudio と同じ）
        let mut r = beam_at(j, q, n, lam, zpos);
        r.position *= sgn;
        r.radius *= sgn;
        rows.push(r);
        if j < img && !d.ignore {
            let t = sgn * d.t;
            let in_grin = d.grin().is_some() && !d.is_mirror();
            if t.is_finite() && t != 0.0 && !in_grin {
                // 面と面の間の自由な伝搬: q(s) = q + s（媒質の中の距離）
                const STEPS: usize = 32;
                profile.push(
                    (0..=STEPS)
                        .map(|k| {
                            let ds = t * k as f64 / STEPS as f64;
                            [zpos + ds, beam_at(j, q + ds, n, lam, 0.0).size]
                        })
                        .collect(),
                );
            }
            if t.is_finite() {
                zpos += t;
            }
        }
    }
    Ok(GaussOut { rows, profile })
}
