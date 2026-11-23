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
    layers: &(dyn Fn(f64) -> Result<Vec<(C64, f64)>> + Sync),
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
    // 波長ごとに並列に計算する（厚い膜では数百万点になる）
    type Wave = (f64, Vec<[[C64; 4]; 2]>, Vec<[f64; 2]>);
    let rows: Vec<Wave> = {
        use rayon::prelude::*;
        wavelengths
            .par_iter()
            .map(crate::par::scoped(|wl: &f64| -> Result<Wave> {
                let wl = *wl;
                let ls = layers(wl)?;
                if ls.is_empty() {
                    invalid!("coating model from layers: no layers");
                }
                let d: f64 = ls.iter().map(|l| l.1).sum();
                let mut mw = Vec::with_capacity(betas.len());
                let mut pw = Vec::with_capacity(betas.len());
                for &b in betas {
                    let m = layer_matrix(&ls, wl, b);
                    let mut psi = 0.0;
                    for &(n, dj) in &ls {
                        let q = C64::new(n.re * n.re - b * b, 0.0).sqrt();
                        psi += dj * (C64::new(b * b, 0.0) / q).re;
                    }
                    if m.iter().flatten().any(|z| !(z.re.is_finite() && z.im.is_finite()))
                        || (b <= bmax && !psi.is_finite())
                    {
                        invalid!("coating model from layers: the characteristic matrix is not finite at {wl} µm, β {b}");
                    }
                    mw.push(m);
                    // BMAX より上の ψ は使わない（層の屈折率の上で発散する）。有限でなければ 0 にしておく
                    pw.push([psi, psi].map(|v| if v.is_finite() { v } else { 0.0 }));
                }
                Ok((d, mw, pw))
            }))
            .collect::<Result<_>>()?
    };
    md.d = rows[0].0;
    for (d, mw, pw) in rows {
        if (d - md.d).abs() > 1e-12 * d.max(1.0) {
            invalid!("coating model from layers: the total thickness changes with the wavelength");
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

/// 膜ファイルの膜からモデルを作るときの指定（[`model_from_coat`]）
#[derive(Debug, Clone)]
pub struct CoatGridSpec {
    /// 波長の範囲 [µm]
    pub wl: (f64, f64),
    /// 透過の β の上限（格子は β² を 0 からここまで等間隔）
    pub beta_max: f64,
    /// 補間の点の数（[`CoatModel::order`]、4〜12）
    pub order: usize,
    /// 格子の間で許す係数の差
    pub tol: f64,
    /// 鏡の表も作る（基準の媒質の屈折率。膜ファイルの鏡と同じく最後の層を基板とみる）
    pub mirror_ref: Option<f64>,
    /// 膜厚が波長の単位の層の基準波長 [µm]（OpticStudio の系の主波長）
    pub primary: f64,
    /// 格子を細かくし直す回数の上限
    pub max_refine: usize,
}

impl Default for CoatGridSpec {
    fn default() -> Self {
        CoatGridSpec {
            wl: (0.4, 0.7),
            beta_max: 1.4,
            order: 8,
            tol: 1e-5,
            mirror_ref: None,
            primary: 0.55,
            max_refine: 12,
        }
    }
}

/// [`model_from_coat`] の結果の格子と、格子の間で確かめた係数の差
#[derive(Debug, Clone, Default)]
pub struct CoatGridReport {
    pub wavelengths: usize,
    pub betas: usize,
    pub mirror_wavelengths: usize,
    pub mirror_betas: usize,
    /// 透過・屈折面の反射の、波長の格子の間・β の格子の間での最大の差
    pub err_wave: f64,
    pub err_beta: f64,
    /// 鏡の、波長の格子の間・β の格子の間での最大の差
    pub mirror_err_wave: f64,
    pub mirror_err_beta: f64,
    pub refined: usize,
    /// 差がすべて `tol` 以下になった（false なら `max_refine` 回細かくしても届かなかった）
    pub converged: bool,
    /// 材料の分散の折れ目（波長の補間がまたがない点）
    pub kinks: Vec<f64>,
}

/// 膜ファイルの膜の層（波長 wl での (複素屈折率, 厚さ [µm])）。材料は膜ファイルの MATE だけ（硝材の名前は使えない）
pub fn coat_layers(
    file: &CoatingFile,
    coat: &str,
    wl: f64,
    primary: f64,
) -> Result<Vec<(C64, f64)>> {
    let Some(ls) = file.coat.get(coat) else {
        invalid!("coating {coat} is not in the coating file (COAT)");
    };
    let n_at = |name: &str, w: f64| -> Result<C64> {
        let Some(m) = file.mate.get(name).filter(|m| !m.is_empty()) else {
            invalid!("coating material {name} is not in the coating file (MATE); glass materials can't be used here");
        };
        let xs: Vec<f64> = m.iter().map(|r| r[0]).collect();
        let n: Vec<f64> = m.iter().map(|r| r[1]).collect();
        let k: Vec<f64> = m.iter().map(|r| r[2]).collect();
        let kk = interp(w, &xs, &k);
        if kk > MATE_K_POS_TOL {
            invalid!("coating material {name} has a positive k ({kk} at {w} µm); write absorption as k < 0");
        }
        Ok(C64::new(interp(w, &xs, &n), kk))
    };
    ls.iter()
        .map(|l| {
            let n = n_at(&l.mat, wl)?;
            let d = if l.absolute {
                l.d
            } else {
                l.d * primary / n_at(&l.mat, primary)?.re
            };
            Ok((n, d))
        })
        .collect()
}

/// 1/λ を区間（折れ目の間）ごとに等間隔にした波長の格子（昇順）。間隔は ds 以下、区間ごとに p 点以上
fn wave_grid(w0: f64, w1: f64, kinks: &[f64], ds: f64, p: usize) -> Vec<f64> {
    let mut edges: Vec<f64> = vec![1.0 / w1];
    edges.extend(kinks.iter().rev().map(|k| 1.0 / k));
    edges.push(1.0 / w0);
    let mut s: Vec<f64> = Vec::new();
    for e in edges.windows(2) {
        let n = (((e[1] - e[0]) / ds).ceil() as usize + 1).max(p);
        for i in 0..n {
            let x = e[0] + (e[1] - e[0]) * i as f64 / (n - 1) as f64;
            if s.last().is_none_or(|&l| x > l * (1.0 + 1e-12)) {
                s.push(x);
            }
        }
    }
    let mut w: Vec<f64> = s.iter().map(|x| 1.0 / x).collect();
    w.reverse();
    // 折れ目と端は格子の点にちょうど置く（1/x の丸めで動かさない）
    for v in w.iter_mut() {
        for &k in kinks.iter().chain([w0, w1].iter()) {
            if (*v - k).abs() < 1e-12 * k {
                *v = k;
            }
        }
    }
    w
}

/// β² を 0 から top² まで n 点の等間隔にした β
fn beta_grid(top: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| (top * top * i as f64 / (n - 1) as f64).sqrt())
        .collect()
}

/// 膜ファイルの膜（COAT）から、層を見せないモデルを作る。格子は 1/λ（折れ目ごとの区間）と β² を等間隔にとり、
/// 格子の間の点（波長の中点・β² の中点）でモデルの係数と層からの直接計算を比べ、差が `tol` を超える方向を細かくし直す。
/// 比べる係数は、透過（空気→1.52、1.52→1.52、両向き）と屈折面の反射（両側）、鏡（空気・1.52 から）
pub fn model_from_coat(
    file: &CoatingFile,
    coat: &str,
    spec: &CoatGridSpec,
) -> Result<(CoatModel, CoatGridReport)> {
    use rayon::prelude::*;
    let (w0, w1) = spec.wl;
    if !(w0 > 0.0 && w0 < w1) {
        invalid!("coating model from layers: the wavelength range {w0} to {w1} µm is not valid");
    }
    if !MODEL_ORDER_RANGE.contains(&spec.order) {
        let p = spec.order;
        invalid!("coating model from layers: the interpolation order {p} is not 4 to 12");
    }
    let layers = |wl: f64| coat_layers(file, coat, wl, spec.primary);
    let l0 = layers(w0)?;
    // 折れ目: 使う材料の表の節（範囲の中）
    let mut kinks: Vec<f64> = Vec::new();
    for l in &file.coat[coat] {
        if let Some(m) = file.mate.get(&l.mat) {
            for r in m {
                if r[0] > w0 * (1.0 + 1e-9)
                    && r[0] < w1 * (1.0 - 1e-9)
                    && !kinks.iter().any(|&k| (k - r[0]).abs() < 1e-12)
                {
                    kinks.push(r[0]);
                }
            }
        }
    }
    kinks.sort_by(f64::total_cmp);
    // 層の屈折率の実部の最小（ψ が発散する β）と、群の光学的な厚さの目安
    let mut nmin = f64::INFINITY;
    let mut opt: f64 = 0.0;
    for wl in [w0, 0.5 * (w0 + w1), w1]
        .into_iter()
        .chain(kinks.iter().copied())
    {
        let ls = layers(wl)?;
        nmin = ls.iter().map(|l| l.0.re).fold(nmin, f64::min);
        opt = opt.max(ls.iter().map(|l| l.0.norm() * l.1).sum());
    }
    let dsum: f64 = l0.iter().map(|l| l.1).sum();
    let bmax = spec.beta_max.min(nmin - 1e-3).max(0.0);
    let p = spec.order;
    // 初めの格子: 1/λ の間隔は群の光学的な厚さの 1/16、β² の間隔は位相の変わり方 k Σ d / q から
    let mut ds = 1.0 / (12.0 * opt.max(0.05) * 1.15);
    let qmin = (nmin * nmin - spec.beta_max.min(nmin * 0.999).powi(2))
        .max(1e-4)
        .sqrt();
    let mut nb =
        ((spec.beta_max.powi(2) * 2.0 * PI / w0 * dsum / (2.0 * qmin) / 0.8).ceil() as usize + 1)
            .max(2 * p);
    let mirror_top = spec.mirror_ref.map(|n| n * 0.985);
    let mut mds = ds;
    let mut mnb = mirror_top.map_or(0, |t| {
        ((t * t * 2.0 * PI / w0 * dsum / (2.0 * qmin) / 0.8).ceil() as usize + 1).max(2 * p)
    });
    let media = [(1.0, 1.52), (1.52, 1.52), (1.52, 1.0)];
    let mut rep = CoatGridReport {
        kinks: kinks.clone(),
        ..Default::default()
    };
    for it in 0..=spec.max_refine {
        let wls = wave_grid(w0, w1, &kinks, ds, p);
        let betas = beta_grid(spec.beta_max, nb);
        let mg = spec.mirror_ref.map(|n_ref| MirrorGrid {
            n_ref,
            wavelengths: wave_grid(w0, w1, &kinks, mds, p),
            betas: beta_grid(mirror_top.unwrap_or(0.0), mnb),
        });
        let md = model_from_layers(&layers, &wls, &betas, &kinks, p, bmax, mg.as_ref())?;
        // 格子の間の点: 波長は隣り合う点の中点（1/λ）、β は β² の中点
        let mid_w = |w: &[f64]| -> Vec<f64> {
            w.windows(2)
                .map(|x| 2.0 / (1.0 / x[0] + 1.0 / x[1]))
                .collect()
        };
        let mid_b = |b: &[f64]| -> Vec<f64> {
            b.windows(2)
                .map(|x| (0.5 * (x[0] * x[0] + x[1] * x[1])).sqrt())
                .collect()
        };
        let step = |n: usize| (n / 24).max(1);
        // 透過・反射の差: 波長 wl の β の列で
        let trans_err = |wl: f64, bs: &[f64]| -> Result<f64> {
            let mut e: f64 = 0.0;
            for &(n1, n2) in &media {
                let keys = [("X|A|G", false), ("X|A|G|R", true)];
                for (key, _) in keys {
                    let key = if n2 == 1.0 {
                        key.replace("|G", "|AIR")
                    } else {
                        key.to_string()
                    };
                    let m = md.iface(&key, wl, n1, n2, PhaseConvention::Physical)?;
                    let s = layers_iface(&layers(wl)?, &key, wl, n1, n2);
                    for &b in bs {
                        if b >= n1.min(n2) * (1.0 - 1e-9) || b > spec.beta_max {
                            continue;
                        }
                        let th = (b / n1).asin();
                        let (a, c) = (m.eval(th)?, s.eval(th)?);
                        e = e.max((a[0] - c[0]).norm()).max((a[1] - c[1]).norm());
                    }
                }
            }
            Ok(e)
        };
        let ew: Vec<f64> = mid_w(&wls)
            .par_iter()
            .map(crate::par::scoped(|wl: &f64| {
                let bs: Vec<f64> = betas.iter().step_by(step(betas.len())).copied().collect();
                trans_err(*wl, &bs)
            }))
            .collect::<Result<_>>()?;
        let eb: Vec<f64> = wls
            .par_iter()
            .step_by(step(wls.len()))
            .map(crate::par::scoped(|wl: &f64| {
                trans_err(*wl, &mid_b(&betas))
            }))
            .collect::<Result<_>>()?;
        rep.err_wave = ew.iter().copied().fold(0.0, f64::max);
        rep.err_beta = eb.iter().copied().fold(0.0, f64::max);
        rep.wavelengths = wls.len();
        rep.betas = betas.len();
        rep.refined = it;
        let mut again = false;
        if rep.err_wave > spec.tol {
            ds /= 1.25;
            again = true;
        }
        if rep.err_beta > spec.tol {
            nb = (nb as f64 * 1.25).ceil() as usize;
            again = true;
        }
        if let Some(g) = &mg {
            let mir_err = |wl: f64, bs: &[f64]| -> Result<f64> {
                let mut e: f64 = 0.0;
                let mut ls = layers(wl)?;
                let ns = ls.pop().map_or(MIRROR_N, |l| l.0);
                for n1 in [1.0, 1.52] {
                    let m = md.iface("X|A|MIRROR", wl, n1, 1.0, PhaseConvention::OpticStudio)?;
                    let s = Iface {
                        kind: Kind::Stack {
                            layers: ls.clone(),
                            ns,
                            n_out: n1,
                            layer_phase: true,
                            front: true,
                        },
                        mirror: true,
                        n1,
                        wl,
                    };
                    for &b in bs {
                        if b >= n1 * (1.0 - 1e-9) {
                            continue;
                        }
                        let th = (b / n1).asin();
                        // 分岐点のそばの notimpl は数えない（膜ファイルでも分岐点は測った点に置けない）
                        let Ok(a) = m.eval(th) else { continue };
                        let c = s.eval(th)?;
                        e = e.max((a[0] - c[0]).norm()).max((a[1] - c[1]).norm());
                    }
                }
                Ok(e)
            };
            let bs_all: Vec<f64> = (0..48).map(|i| 1.5 * i as f64 / 47.0).collect();
            let mew: Vec<f64> = mid_w(&g.wavelengths)
                .par_iter()
                .map(crate::par::scoped(|wl: &f64| mir_err(*wl, &bs_all)))
                .collect::<Result<_>>()?;
            let mbs: Vec<f64> = mid_b(&g.betas)
                .into_iter()
                .map(|b| b * 1.0)
                .filter(|&b| b < 1.52)
                .collect();
            let meb: Vec<f64> = g
                .wavelengths
                .par_iter()
                .step_by(step(g.wavelengths.len()))
                .map(crate::par::scoped(|wl: &f64| mir_err(*wl, &mbs)))
                .collect::<Result<_>>()?;
            rep.mirror_err_wave = mew.iter().copied().fold(0.0, f64::max);
            rep.mirror_err_beta = meb.iter().copied().fold(0.0, f64::max);
            rep.mirror_wavelengths = g.wavelengths.len();
            rep.mirror_betas = g.betas.len();
            if rep.mirror_err_wave > spec.tol {
                mds /= 1.25;
                again = true;
            }
            if rep.mirror_err_beta > spec.tol {
                mnb = (mnb as f64 * 1.25).ceil() as usize;
                again = true;
            }
        }
        if !again || it == spec.max_refine {
            rep.converged = !again;
            return Ok((md, rep));
        }
    }
    unreachable!("the loop returns on its last pass")
}

/// 層から直接計算する境界（膜ファイルの層と同じ並べ方。[`Coatings`] の Stack と同じ）
fn layers_iface(layers: &[(C64, f64)], key: &str, wl: f64, n1: f64, n2: f64) -> Iface {
    let k = IfaceKey::parse(key);
    let reflect = k.is_reflect();
    let mut layers = layers.to_vec();
    let air = k.after_is_air();
    if air {
        layers.reverse();
    }
    let kind = Kind::Stack {
        layers,
        ns: C64::new(n2, 0.0),
        n_out: if reflect || !air { n1 } else { n2 },
        layer_phase: false,
        front: !air,
    };
    Iface {
        kind,
        mirror: reflect,
        n1,
        wl,
    }
}
