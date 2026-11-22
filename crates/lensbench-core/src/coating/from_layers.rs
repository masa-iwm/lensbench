//! 層のわかる膜から特性行列モデル（[`CoatModel`]）を作る（層を見せずに配るため）

use super::*;

/// 鏡の表を作る格子（[`model_from_layers`]）
#[derive(Debug, Clone)]
pub struct MirrorGrid {
    /// 基準の媒質の屈折率（[`MirrorModel::n_ref`]）
    pub n_ref: f64,
    pub wavelengths: Vec<f64>,
    pub betas: Vec<f64>,
}

/// 層（膜の外側から基板の側へ、(複素屈折率, 厚さ [µm])）の特性行列 M（偏光 s, p ごとに M00 M10 M01 M11）。
/// 層の行列の成分は q = sqrt(n² − β²) の偶関数なので、q の枝によらない（β が層の屈折率を超えても使える）
pub fn layer_matrix(layers: &[(C64, f64)], wl: f64, b: f64) -> [[C64; 4]; 2] {
    let k = 2.0 * PI / wl;
    let one = C64::new(1.0, 0.0);
    let zero = C64::new(0.0, 0.0);
    let i = C64::new(0.0, 1.0);
    std::array::from_fn(|pol| {
        let mut m = [[one, zero], [zero, one]];
        for &(n, d) in layers {
            let q = (n * n - b * b).sqrt();
            let e = if pol == 0 { q } else { n * n / q };
            let dl = q * (k * d);
            let (c, s) = (dl.cos(), dl.sin());
            let l = [[c, i * s / e], [i * e * s, c]];
            m = [
                [
                    m[0][0] * l[0][0] + m[0][1] * l[1][0],
                    m[0][0] * l[0][1] + m[0][1] * l[1][1],
                ],
                [
                    m[1][0] * l[0][0] + m[1][1] * l[1][0],
                    m[1][0] * l[0][1] + m[1][1] * l[1][1],
                ],
            ];
        }
        [m[0][0], m[1][0], m[0][1], m[1][1]]
    })
}

/// 層のわかる膜から、特性行列モデルを作る。
///
/// `layers` は波長 [µm] からその波長の層（膜の外側から基板の側へ、(複素屈折率, 厚さ [µm])。COATING.DAT の COAT と同じ順）。
/// 透過は波長 `wavelengths`・β `betas` の格子で M̃（層の特性行列そのもの。det = 1）と
/// ψ = Σ d_j Re(β² / q_j)（q_j = sqrt(Re(n_j)² − β²)）を持つ。モデルは M = exp(i k ψ) M̃ で t を割るので、
/// 係数には [`stack_coef`] と同じ位相 exp(−i k Σ d_j Re(β² / q_j)) が掛かる。D は層の厚さの和。
/// `bmax` は透過で OpticStudio の位相を使える β の上限（ψ は層の屈折率の実部で発散するので、それより少し下にする）。
/// `kinks` は層の材料の分散の折れ目（波長の補間がまたがない点。格子にも入れておく）。
/// `order` は補間の点の数（[`CoatModel::order`]。0 なら従来の補間）。
/// `mirror` があれば鏡の表も作る（膜ファイルの鏡と同じく、最後の層を半無限の基板とみて、残りの層の反射を基準の媒質から計算する）
#[allow(clippy::too_many_arguments)]
pub fn model_from_layers(
    layers: &dyn Fn(f64) -> Result<Vec<(C64, f64)>>,
    wavelengths: &[f64],
    betas: &[f64],
    kinks: &[f64],
    order: usize,
    bmax: f64,
    mirror: Option<&MirrorGrid>,
) -> Result<CoatModel> {
    if order != 0 && !MODEL_ORDER_RANGE.contains(&order) {
        invalid!("coating model from layers: the interpolation order {order} is not 0 or 4 to 12");
    }
    let asc = |v: &[f64]| v.windows(2).all(|x| x[0] < x[1]);
    if !asc(wavelengths) || !asc(betas) || betas.len() < 4 || wavelengths.is_empty() {
        invalid!("coating model from layers: the wavelengths or β are not ascending, or there are fewer than 4 β points");
    }
    let mut md = CoatModel {
        bmax,
        wavelengths: wavelengths.to_vec(),
        betas: betas.to_vec(),
        kinks: kinks.to_vec(),
        order,
        ..Default::default()
    };
    md.kinks.sort_by(f64::total_cmp);
    for (iw, &wl) in wavelengths.iter().enumerate() {
        let ls = layers(wl)?;
        if ls.is_empty() {
            invalid!("coating model from layers: no layers");
        }
        let d: f64 = ls.iter().map(|l| l.1).sum();
        if iw == 0 {
            md.d = d;
        } else if (d - md.d).abs() > 1e-12 * d.max(1.0) {
            invalid!("coating model from layers: the total thickness changes with the wavelength");
        }
        let mut mw = Vec::with_capacity(betas.len());
        let mut pw = Vec::with_capacity(betas.len());
        for &b in betas {
            let m = layer_matrix(&ls, wl, b);
            let mut psi = 0.0;
            for &(n, dj) in &ls {
                let q = C64::new(n.re * n.re - b * b, 0.0).sqrt();
                psi += dj * (C64::new(b * b, 0.0) / q).re;
            }
            if m.iter()
                .flatten()
                .any(|z| !(z.re.is_finite() && z.im.is_finite()))
                || (b <= bmax && !psi.is_finite())
            {
                invalid!("coating model from layers: the characteristic matrix is not finite at {wl} µm, β {b}");
            }
            mw.push(m);
            // BMAX より上の ψ は使わない（層の屈折率の上で発散する）。有限でなければ 0 にしておく
            pw.push([psi, psi].map(|v| if v.is_finite() { v } else { 0.0 }));
        }
        md.m.push(mw);
        md.psi.push(pw);
    }
    if let Some(g) = mirror {
        if !asc(&g.wavelengths) || !asc(&g.betas) || g.betas.len() < 4 || g.wavelengths.is_empty() {
            invalid!("coating model from layers: the mirror wavelengths or β are not ascending, or there are fewer than 4 β points");
        }
        let mut r = Vec::with_capacity(g.wavelengths.len());
        let mut dm = 0.0;
        for &wl in &g.wavelengths {
            let mut ls = layers(wl)?;
            // 膜ファイルの鏡と同じく、最後の層が基板（半無限）
            let ns = match ls.pop() {
                Some((n, _)) => n,
                None => MIRROR_N,
            };
            dm = ls.iter().map(|l| l.1).sum();
            let mut rw = Vec::with_capacity(g.betas.len());
            for &b in &g.betas {
                let s0 = b / g.n_ref;
                if s0 >= 1.0 {
                    invalid!("coating model from layers: the mirror β {b} is not below the reference index {}", g.n_ref);
                }
                let c0 = (1.0 - s0 * s0).sqrt();
                let (rs, rp, _, _) = stack_sc(g.n_ref, &ls, ns, wl, s0, c0);
                rw.push([rs, rp]);
            }
            r.push(rw);
        }
        md.mirror = Some(MirrorModel {
            n_ref: g.n_ref,
            d: dm,
            wavelengths: g.wavelengths.clone(),
            betas: g.betas.clone(),
            r,
            ..Default::default()
        });
    }
    Ok(md)
}
