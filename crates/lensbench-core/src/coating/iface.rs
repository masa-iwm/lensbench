//! 境界の係数の用意（[`Coatings::prepare`]）と評価（[`Iface`]）

use super::*;

impl Coatings {
    /// 境界 key の波長 wl [µm] での係数を用意する（n1・n2 は前後の屈折率、鏡では n2 は使わない）
    ///
    /// 屈折面での反射（鍵 `膜|入射側|反対側|R`、ゴーストの経路）は鏡と同じ約束（p = s × k を反射の後で反転）の r を返す。
    /// n2 は反対側の媒質の屈折率。膜の向き（基板の側）は透過と同じく、反対側が空気でなければ反対側が基板、空気なら
    /// 入射側が基板。位相は基板の面を面の頂点に置いた値（膜の側から入るときは鏡と同じく exp(2 i k n1 cosθ1 Σ d_j)
    /// を掛け、基板の側から入るときは掛けない）。暗号化された膜は特性行列モデルの M から r を求める（透過の実測表
    /// だけでは求められない）
    pub fn prepare(&self, sys: &System, key: &str, wl: f64, n1: f64, n2: f64) -> Result<Iface> {
        let k = IfaceKey::parse(key);
        let coat = k.coat;
        let reflect = k.is_reflect();
        let to_mirror = k.after == After::Mirror;
        let mirror = to_mirror || reflect;
        let kind = match self.resolve(&k, key) {
            CoatKind::Table => {
                let tab = &self.file.table[coat];
                let angs: Vec<f64> = tab.iter().map(|a| a.0).collect();
                let mut vals: [Vec<f64>; 8] = Default::default();
                for (a, rows) in tab {
                    if rows.is_empty() {
                        invalid!("coating table {coat} has no row for angle {a}");
                    }
                    let xs: Vec<f64> = rows.iter().map(|r| r[0]).collect();
                    for c in 0..8 {
                        let mut ys: Vec<f64> = rows.iter().map(|r| r[c + 1]).collect();
                        if c >= 4 {
                            unwrap_deg(&mut ys);
                        }
                        vals[c].push(interp(wl, &xs, &ys));
                    }
                }
                let step = self.table_normal == TableNormal::OpticStudio;
                if !step && angs[0] == 0.0 {
                    // 段差なし: 0° の行の s を p で置き換えて補間する
                    for c in [0, 2, 4, 6] {
                        vals[c][0] = vals[c + 1][0];
                    }
                }
                // 位相（度）は ±180° の折り返しをほどいてから補間する（+179° と −179° の間を 0° にしない）
                for v in &mut vals[4..] {
                    unwrap_deg(v);
                }
                Kind::Table {
                    name: coat.to_string(),
                    step,
                    xc: angs.iter().map(|a| -a.to_radians().cos()).collect(),
                    angs,
                    vals: Box::new(vals),
                    out: (!mirror && n1 > n2 && n2 > 0.0).then_some(n1 / n2),
                }
            }
            CoatKind::Bare => Kind::Fresnel {
                ns: if to_mirror {
                    MIRROR_N
                } else {
                    C64::new(n2, 0.0)
                },
            },
            CoatKind::Ideal => {
                // I.xx: 透過率 .xx。I.s=Ts,p=Tp: s・p で別の透過率（理想の偏光ビームスプリッター）。
                // 反射（鏡・屈折面での反射）は −√(1 − T)
                let [ts, tp] = ideal_t(coat)?;
                let v = |t: f64| C64::new(if mirror { -(1.0 - t).sqrt() } else { t.sqrt() }, 0.0);
                Kind::Ideal([v(ts), v(tp)])
            }
            CoatKind::Stack => {
                let ls = &self.file.coat[coat];
                let wl_ref = sys.wavelengths[sys.primary];
                let mut layers = Vec::with_capacity(ls.len());
                for l in ls {
                    let n = self.mat_n(sys, &l.mat, wl)?;
                    let d = if l.absolute {
                        l.d
                    } else {
                        l.d * wl_ref / self.mat_n(sys, &l.mat, wl_ref)?.re
                    };
                    layers.push((n, d));
                }
                // OpticStudio の約束（COATING.DAT の膜で、空気・硝材どうし・両向き・鏡を偏光追跡と照合）:
                // - 層は膜の外側（基板と反対側）から順。透過で後ろの媒質が空気なら、基板は前の媒質（層を逆に通る）
                // - 鏡の基板は最後の層の材料（半無限）。最後の層より上だけが薄膜
                // - 係数に位相 exp(i k Σ d_j (n_o / cos_o − β² / q_j)) が掛かる（n_o・cos_o は膜の外側の媒質、
                //   β = n1 sinθ1、q_j = sqrt(Re(n_j)² − β²)）。鏡では exp(2 i k n1 cosθ1 Σ d_j)
                if to_mirror {
                    let ns = match layers.pop() {
                        Some((n, _)) => n,
                        None => MIRROR_N,
                    };
                    Kind::Stack {
                        layers,
                        ns,
                        n_out: n1,
                        layer_phase: true,
                        front: true,
                    }
                } else if reflect {
                    // 反対側が空気なら基板は入射側（層を逆に、基板の側から入る）
                    let front = !k.after_is_air();
                    if !front {
                        layers.reverse();
                    }
                    Kind::Stack {
                        layers,
                        ns: C64::new(n2, 0.0),
                        n_out: n1,
                        layer_phase: false,
                        front,
                    }
                } else {
                    if k.after_is_air() {
                        layers.reverse();
                    }
                    Kind::Stack {
                        layers,
                        ns: C64::new(n2, 0.0),
                        n_out: if k.after_is_air() { n2 } else { n1 },
                        layer_phase: self.phase == PhaseConvention::OpticStudio,
                        front: false,
                    }
                }
            }
            // モデル（なければ理由を返す）: 暗号化された膜
            CoatKind::Model | CoatKind::Missing => {
                if reflect && !self.models.contains_key(coat) {
                    notimpl!("coating {coat} (interface {key}): reflection coefficients need a characteristic matrix model (a measured table is transmission only)");
                }
                return match self.models.get(coat) {
                    Some(md) => Ok(Iface {
                        kind: prepare_model(
                            coat,
                            md,
                            to_mirror,
                            reflect,
                            k.after_is_air(),
                            wl,
                            n2,
                            self.phase,
                        )?,
                        mirror,
                        n1,
                        wl,
                    }),
                    None => notimpl!(
                        "coating {coat} (interface {key}): not in the coating file, and no measured table or model (an encrypted coating?)"
                    ),
                };
            }
            CoatKind::Measured => {
                let m = &self.measured[key];
                let Some(w) = m.wavelengths.iter().position(|&v| (v - wl).abs() <= 1e-9) else {
                    let have = &m.wavelengths;
                    notimpl!("measured coating table {key} has no wavelength {wl} µm (the table has {have:?})");
                };
                // 屈折角の余弦のスプライン（0 付近で角度に偶、全反射の平方根の振舞いもこれに線形）。鏡では入射角の余弦
                let r = if mirror { 1.0 } else { n1 / n2 };
                let mut xs = Vec::new();
                let mut ys: [Vec<f64>; 4] = Default::default();
                for (a, v) in m.angles_deg.iter().zip(&m.data[w]) {
                    if !(v[0].re.is_finite() && v[0].im.is_finite()) {
                        continue;
                    }
                    xs.push(xcos(r, a.to_radians()));
                    ys[0].push(v[0].re);
                    ys[1].push(v[0].im);
                    ys[2].push(v[1].re);
                    ys[3].push(v[1].im);
                }
                let sp = [0, 1, 2, 3].map(|c| Spline::not_a_knot(&xs, &ys[c]));
                let sp = match sp {
                    [Some(a), Some(b), Some(c), Some(d)] => [a, b, c, d],
                    _ => invalid!("measured coating table {key}: too few points to interpolate"),
                };
                Kind::Measured {
                    key: key.to_string(),
                    r,
                    max_deg: *m.angles_deg.last().unwrap(),
                    cmax: m.angles_deg.last().unwrap().to_radians().cos(),
                    sp: Box::new(sp),
                }
            }
        };
        Ok(Iface {
            kind,
            mirror,
            n1,
            wl,
        })
    }
}

/// 実測表の横軸: −cos(屈折角)（増加）
pub(in crate::coating) fn xcos(r: f64, th: f64) -> f64 {
    xcos_s(r, th.sin())
}

/// [`xcos`] を入射角の sin で
pub(in crate::coating) fn xcos_s(r: f64, s0: f64) -> f64 {
    let s = r * s0;
    -(1.0 - s * s).max(0.0).sqrt()
}

/// 理想膜の名前から (s, p) の強度透過率: `I.xx`（s・p 共通、`I.98` は 0.98）、`I.s=Ts,p=Tp`（s・p で別）
pub(crate) fn ideal_t(coat: &str) -> Result<[f64; 2]> {
    let bad = || Error::Invalid(format!("ideal coating {coat} (I.xx or I.s=Ts,p=Tp)"));
    let rest = coat.strip_prefix("I.").ok_or_else(bad)?;
    let v = if rest.contains('=') {
        let (mut ts, mut tp) = (None, None);
        for part in rest.split(',') {
            let (k, x) = part.split_once('=').ok_or_else(bad)?;
            let x: f64 = x.trim().parse().map_err(|_| bad())?;
            match k.trim() {
                "s" => ts = Some(x),
                "p" => tp = Some(x),
                _ => return Err(bad()),
            }
        }
        [ts.ok_or_else(bad)?, tp.ok_or_else(bad)?]
    } else {
        let t: f64 = coat[1..].parse().map_err(|_| bad())?;
        [t, t]
    };
    if v.iter().any(|t| !(0.0..=1.0).contains(t)) {
        return Err(bad());
    }
    Ok(v)
}

#[derive(Debug, Clone)]
pub(in crate::coating) enum Kind {
    Fresnel {
        ns: C64,
    },
    /// 理想膜の (s, p) の係数
    Ideal([C64; 2]),
    Stack {
        layers: Vec<(C64, f64)>,
        ns: C64,
        /// 膜の外側（基板と反対側）の媒質の屈折率（位相の約束に使う）
        n_out: f64,
        /// 透過で層ごとの位相を含める（[`PhaseConvention::OpticStudio`]）
        layer_phase: bool,
        /// 反射で、膜の側から入る（位相 exp(2 i k n1 cosθ1 Σ d_j) を掛ける。基板が頂点にある）
        front: bool,
    },
    Table {
        name: String,
        /// 垂直入射（1 − cosθ < TABLE_NORMAL）だけ s に p を使う（[`TableNormal::OpticStudio`]）
        step: bool,
        angs: Vec<f64>,
        /// −cos(角度)（補間の横軸、増加）
        xc: Vec<f64>,
        /// 量ごと（Rs Rp Ts Tp と位相）の角度に対する値
        vals: Box<[Vec<f64>; 8]>,
        /// 透過で屈折率の高い側から入るとき n1 / n2（表の角度は屈折率の低い側の角度で引く。OpticStudio と同じ）
        out: Option<f64>,
    },
    Measured {
        key: String,
        r: f64,
        max_deg: f64,
        /// cos(max_deg)
        cmax: f64,
        sp: Box<[Spline; 4]>,
    },
    /// モデルの透過: β² のスプライン（偏光ごとに M̃ の実部・虚部 8 本と ψ）
    Model {
        name: String,
        /// 層ごとの位相（ψ）を含める（[`PhaseConvention::OpticStudio`]）
        layer_phase: bool,
        /// 屈折面での反射 r を返す（M から。ψ は打ち消し合うので使わない）
        reflect: bool,
        reverse: bool,
        n2: f64,
        d: f64,
        bmax: f64,
        b2: (f64, f64),
        sp: Box<[[Spline; 9]; 2]>,
    },
    /// モデルの鏡: 基準の媒質での r（この波長、β² の点ごと、偏光ごと）。β² の有理関数で補間する
    /// （高い β では導波の共鳴で、β 方向にも鋭く変わる）
    ModelMirror {
        name: String,
        n_ref: f64,
        d: f64,
        b2: Vec<f64>,
        r: Box<[Vec<C64>; 2]>,
        /// β² 方向の分岐点（膜の外の媒質で全反射が始まる β²）。補間の点はこれをまたがない
        branch: Option<Box<Branch>>,
        /// 値が決まらない β² の区間（両端を含む）。ここでは notimpl
        band: Option<(f64, f64)>,
    },
}

/// 膜のモデルを波長 wl で用意する
#[allow(clippy::too_many_arguments)]
pub(in crate::coating) fn prepare_model(
    name: &str,
    md: &CoatModel,
    mirror: bool,
    reflect: bool,
    reverse: bool,
    wl: f64,
    n2: f64,
    phase: PhaseConvention,
) -> Result<Kind> {
    let range = |b: &[f64]| (b[0] * b[0], b[b.len() - 1] * b[b.len() - 1]);
    if mirror {
        let Some(m) = &md.mirror else {
            notimpl!("coating model {name} has no mirror data");
        };
        let Some((s0, e0)) = wave_segment(&m.wavelengths, wl, &md.kinks) else {
            let why = out_of_waves(&m.wavelengths, wl);
            notimpl!("coating model {name} (mirror): wavelength {wl} µm {why}");
        };
        // 波数の昇順に並べた点で補間する（波長は昇順なので逆順）
        let xs: Vec<f64> = (s0..e0).rev().map(|i| 1.0 / m.wavelengths[i]).collect();
        let b2: Vec<f64> = m.betas.iter().map(|b| b * b).collect();
        let brb = m.branches(&b2);
        let brs: Vec<Option<f64>> = brb.iter().map(|v| v.map(|v| v.xb)).collect();
        // 分岐点の位置は波長の滑らかな関数（基板の屈折率の 2 乗）なので、近い 4 点の 1/λ の 3 次式で補間する
        // （区間のどの波長にもあるときだけ使う）
        let branch = if brs[s0..e0].iter().all(Option::is_some) {
            let bx: Vec<f64> = (s0..e0).rev().map(|i| brs[i].unwrap_or(0.0)).collect();
            // どの波長でも同じ（分散のない媒質。格子の点に寄せたときなど）なら、補間の丸めで動かさない
            if bx.iter().all(|&v| v == bx[0]) {
                Some(bx[0])
            } else {
                let n = xs.len().min(4);
                let i = xs
                    .partition_point(|&x| x < 1.0 / wl)
                    .saturating_sub(n / 2)
                    .min(xs.len() - n);
                let q = 1.0 / wl;
                Some(
                    (i..i + n)
                        .map(|a| {
                            let w: f64 = (i..i + n)
                                .filter(|&c| c != a)
                                .map(|c| (q - xs[c]) / (xs[a] - xs[c]))
                                .product();
                            w * bx[a]
                        })
                        .sum::<f64>(),
                )
            }
        } else {
            None
        };
        let branch = branch.map(|xb| Box::new(Branch::new(&b2, xb)));
        // 補間に使いそうな波長（近い RATIONAL_N 点と、その両側に 2 点ずつ）で分岐点が点 j の β² の両側にあるなら、
        // β² を固定すると波長の補間が分岐点をまたぐ。その点は、各波長の表を分岐点からの β² の差 u = β² − xb で
        // 揃えてから波長で補間する。ほかの点は β² を固定して補間する（測った点で値が厳密に合う）
        // rational_at が選ぶ波長の点（端では内側へ寄せる）と、その両側 2 点
        let (w0, w1) = {
            let n = xs.len();
            let m = RATIONAL_N.min(n - n % 2).max(n.min(4));
            let st = xs
                .partition_point(|&x| x < 1.0 / wl)
                .saturating_sub(m / 2)
                .min(n - m);
            // xs の k 番目は波長の e0 − 1 − k 番目
            (
                (e0 - (st + m)).saturating_sub(2).max(s0),
                (e0 - st + 2).min(e0),
            )
        };
        // 分岐点が測った点の間にある波長では、そのそば（全反射の手前の最後の点 k − 1 から、全反射の 2 点目 k + 1 まで）は
        // r が急に変わるのに測った点が足りず、値が r のわずかな違いで大きく変わり、OpticStudio とも合わない
        // （細かく測った OpticStudio の値と 2.6e-2 違った。この範囲を外すと 8e-6 以下）。
        // 補間に使う波長のその範囲を合わせた区間では notimpl にする（測った点の上にある分岐点は除く）
        let band = if branch.is_some() {
            (w0..w1)
                .filter_map(|w| brb[w].filter(|v| !v.on_grid))
                .map(|v| (b2[v.k - 1], b2[(v.k + 1).min(b2.len() - 1)]))
                .reduce(|a, c| (a.0.min(c.0), a.1.max(c.1)))
        } else {
            None
        };
        let near = |j: usize| {
            branch.is_some() && {
                let above = |w: usize| brs[w].is_some_and(|xb| xb > b2[j]);
                let a = above(w0);
                (w0..w1).any(|w| above(w) != a)
            }
        };
        let side: Vec<Option<Branch>> = (s0..e0)
            .map(|i| {
                brs[i]
                    .filter(|_| branch.is_some())
                    .map(|xb| Branch::new(&b2, xb))
            })
            .collect();
        let mut r: [Vec<C64>; 2] = [Vec::new(), Vec::new()];
        for (pol, rp) in r.iter_mut().enumerate() {
            *rp = (0..m.betas.len())
                .map(|j| {
                    if near(j) {
                        let u = b2[j] - branch.as_ref().map_or(0.0, |bb| bb.xb);
                        rational_at(&xs, 0, xs.len(), 1.0 / wl, &|k| {
                            let w = e0 - 1 - k;
                            let bk = side[w - s0].as_ref().expect("分岐点");
                            bk.eval(bk.xb + u, &|jj| m.r[w][jj][pol])
                        })
                    } else {
                        rational_at(&xs, 0, xs.len(), 1.0 / wl, &|k| m.r[e0 - 1 - k][j][pol])
                    }
                })
                .collect();
        }
        return Ok(Kind::ModelMirror {
            name: name.to_string(),
            n_ref: m.n_ref,
            d: m.d,
            b2,
            r: Box::new(r),
            branch,
            band,
        });
    }
    if !md.has_transmit() {
        notimpl!("coating model {name} has no transmission data");
    }
    let Some((i, w)) = wave_weights(&md.wavelengths, wl, &md.kinks) else {
        let why = out_of_waves(&md.wavelengths, wl);
        notimpl!("coating model {name}: wavelength {wl} µm {why}");
    };
    let b2: Vec<f64> = md.betas.iter().map(|b| b * b).collect();
    let mut sp: Vec<[Spline; 9]> = Vec::new();
    for pol in 0..2 {
        let mut v: Vec<Spline> = Vec::new();
        for c in 0..9 {
            let ys: Vec<f64> = (0..md.betas.len())
                .map(|j| {
                    w.iter()
                        .enumerate()
                        .map(|(a, wa)| {
                            let k = i + a;
                            wa * if c == 8 {
                                md.psi[k][j][pol]
                            } else {
                                let z = md.m[k][j][pol][c / 2];
                                if c % 2 == 0 {
                                    z.re
                                } else {
                                    z.im
                                }
                            }
                        })
                        .sum()
                })
                .collect();
            if c == 8 {
                // ψ は BMAX までの点だけで（その先は層の屈折率で発散する）
                let m = md.betas.partition_point(|&b| b <= md.bmax + 1e-12);
                if m < 4 {
                    invalid!("coating model {name}: too few β points up to BMAX (4 or more)");
                }
                v.push(beta_spline(name, &b2[..m], &ys[..m])?);
            } else {
                v.push(beta_spline(name, &b2, &ys)?);
            }
        }
        sp.push(v.try_into().unwrap());
    }
    let [s0, s1]: [[Spline; 9]; 2] = sp.try_into().unwrap();
    Ok(Kind::Model {
        name: name.to_string(),
        layer_phase: phase == PhaseConvention::OpticStudio,
        reflect,
        reverse,
        n2,
        d: md.d,
        bmax: md.bmax,
        b2: range(&md.betas),
        sp: Box::new([s0, s1]),
    })
}

/// ある波長で用意した境界の係数
#[derive(Debug, Clone)]
pub struct Iface {
    pub(in crate::coating) kind: Kind,
    pub(in crate::coating) mirror: bool,
    pub(in crate::coating) n1: f64,
    pub(in crate::coating) wl: f64,
}

impl Iface {
    /// 入射角の cos（0〜1）での (s, p)。屈折率が実数の境界（膜なし）は角度を経由せず実数で計算する
    /// （偏光追跡の面ごとの処理で、acos・複素数の計算を省く）。ほかは [`Self::eval`] と同じ
    pub fn eval_cos(&self, c0: f64) -> Result<[C64; 2]> {
        if let Kind::Fresnel { ns } = &self.kind {
            if ns.im == 0.0 && self.n1 > 0.0 {
                let (n0, ns) = (self.n1, ns.re);
                let s0 = ((1.0 - c0) * (1.0 + c0)).max(0.0).sqrt();
                let q = n0 * s0 / ns;
                let cc = 1.0 - q * q;
                if cc >= 0.0 && c0 > 0.0 {
                    let cs = cc.sqrt();
                    // s: η = n cosθ、p: η = n / cosθ（stack と同じ約束、パワー規格化）
                    let rt = |e0: f64, es: f64| {
                        let r = (e0 - es) / (e0 + es);
                        let t = 2.0 * e0 / (e0 + es) * (es / e0).sqrt();
                        (r, t)
                    };
                    let (rs, ts) = rt(n0 * c0, ns * cs);
                    let (rp, tp) = rt(n0 / c0, ns / cs);
                    return Ok(if self.mirror {
                        [C64::new(rs, 0.0), C64::new(rp, 0.0)]
                    } else {
                        [C64::new(ts, 0.0), C64::new(tp, 0.0)]
                    });
                }
            }
        }
        let c0 = c0.clamp(0.0, 1.0);
        self.eval_sc(((1.0 - c0) * (1.0 + c0)).sqrt(), c0)
    }

    /// 複屈折材の板の境界で、板の側の屈折率を `n` に置き換えた [`Self::eval_cos`]（公開 #52: 異常光の成分の係数）。
    /// `entering` = true は板へ入る面（後ろの媒質が板）、false は板から出る面（前の媒質が板）。
    /// 屈折率で決まる係数（膜なし・膜ファイルの層）だけ置き換え、理想膜・実測表・表・モデル・鏡は [`Self::eval_cos`] と同じ
    pub fn eval_cos_crystal(&self, c0: f64, entering: bool, n: f64) -> Result<[C64; 2]> {
        if entering {
            self.eval_cos_media(c0, None, Some(n))
        } else {
            self.eval_cos_media(c0, Some(n), None)
        }
    }

    /// 前の媒質の屈折率を `n1`、後ろの媒質の屈折率を `n2` に置き換えた [`Self::eval_cos`]（None は置き換えない）。
    /// 複屈折材の板の境界（公開 #52）と、板どうしの貼り合わせの境界（前の板の o・e → 次の板の o・e、公開 #73）に使う。
    /// 置き換えるのは [`Self::eval_cos_crystal`] と同じく屈折率で決まる係数だけ
    pub fn eval_cos_media(&self, c0: f64, n1: Option<f64>, n2: Option<f64>) -> Result<[C64; 2]> {
        if self.mirror {
            return self.eval_cos(c0);
        }
        match &self.kind {
            Kind::Fresnel { ns } => {
                // 膜なしの境界はヒープを持たないので、置き換えた境界を作って同じ計算に通す
                let f = Iface {
                    kind: Kind::Fresnel {
                        ns: n2.map_or(*ns, |n| C64::new(n, 0.0)),
                    },
                    mirror: false,
                    n1: n1.unwrap_or(self.n1),
                    wl: self.wl,
                };
                f.eval_cos(c0)
            }
            Kind::Stack {
                layers,
                ns,
                n_out,
                layer_phase,
                front,
            } => {
                // n_out（位相の約束の外側の媒質）が板なら、それも置き換える（判定は置き換える前の値で）
                let (out_is_2, out_is_1) = (ns.im == 0.0 && *n_out == ns.re, *n_out == self.n1);
                let mut m = StackMedia {
                    n1: self.n1,
                    ns: *ns,
                    n_out: *n_out,
                };
                if let Some(n) = n2 {
                    if out_is_2 {
                        m.n_out = n;
                    }
                    m.ns = C64::new(n, 0.0);
                }
                if let Some(n) = n1 {
                    if out_is_1 {
                        m.n_out = n;
                    }
                    m.n1 = n;
                }
                // eval_cos と同じ cos の丸め
                let c0 = c0.clamp(0.0, 1.0);
                let s0 = ((1.0 - c0) * (1.0 + c0)).sqrt();
                stack_coef(m, layers, *layer_phase, *front, false, self.wl, s0, c0)
            }
            _ => self.eval_cos(c0),
        }
    }

    /// 入射角 theta [rad] での (s, p): 透過、鏡では反射
    pub fn eval(&self, theta: f64) -> Result<[C64; 2]> {
        let (s0, c0) = theta.sin_cos();
        self.eval_sc(s0, c0)
    }

    /// 入射角の sin・cos での (s, p)（角度を経由しない）
    fn eval_sc(&self, s0: f64, c0: f64) -> Result<[C64; 2]> {
        let mirror = self.mirror;
        Ok(match &self.kind {
            Kind::Ideal(v) => *v,
            Kind::Fresnel { ns } => {
                let (rs, rp, ts, tp) = stack_sc(self.n1, &[], *ns, self.wl, s0, c0);
                if mirror {
                    [rs, rp]
                } else {
                    [ts, tp]
                }
            }
            Kind::Stack {
                layers,
                ns,
                n_out,
                layer_phase,
                front,
            } => {
                let m = StackMedia {
                    n1: self.n1,
                    ns: *ns,
                    n_out: *n_out,
                };
                stack_coef(m, layers, *layer_phase, *front, mirror, self.wl, s0, c0)?
            }
            Kind::Table {
                name,
                step,
                angs,
                xc,
                vals,
                out,
            } => {
                // 屈折率の高い側から入る透過（硝材 → 空気など）は、出る側（低い側）の屈折角で表を引く
                // （OpticStudio と同じ。tests/filter_os の裏面の膜で照合）
                let c0 = match out {
                    Some(r) => {
                        let so = r * s0;
                        ((1.0 - so) * (1.0 + so)).max(0.0).sqrt()
                    }
                    None => c0,
                };
                // 角度ではなく cosθ について線形（OpticStudio と同じ。偏光追跡で照合）
                let x = -c0;
                // 範囲の検査: 横軸 −cosθ が表の範囲の中なら角度も中（外れそうなときだけ角度にして調べる）
                if !(x >= xc[0] && x <= xc[xc.len() - 1]) {
                    let deg = c0.clamp(-1.0, 1.0).acos().to_degrees();
                    let (lo, hi) = (angs[0], *angs.last().unwrap());
                    if deg.is_finite() && (deg > hi + 1e-9 || deg < lo - 1e-9) {
                        let what = if out.is_some() {
                            "exit refraction angle"
                        } else {
                            "incidence angle"
                        };
                        notimpl!("coating table {name}: {what} {deg:.4} deg is outside the table range {lo} to {hi}");
                    }
                }
                let v = |k: usize| interp(x, xc, &vals[k]);
                let (a, b) = if mirror { (0, 4) } else { (2, 6) };
                let p = C64::from_polar(v(a + 1).sqrt(), v(b + 1).to_radians());
                // 垂直入射では s・p の区別がないので p の値を使う。OpticStudio と同じく、ごく小さい角度
                // （1 − cosθ < TABLE_NORMAL）だけを垂直入射とみなす（その外は表の s に段差で戻る）
                if *step && 1.0 - c0 < TABLE_NORMAL {
                    return Ok([p, p]);
                }
                [C64::from_polar(v(a).sqrt(), v(b).to_radians()), p]
            }
            Kind::Measured {
                key,
                r,
                max_deg,
                cmax,
                sp,
            } => {
                // 範囲の検査: cosθ が表の最大角の cos より十分大きければ範囲内（境目の近くだけ角度にして調べる）
                if !(c0 > cmax + 1e-12) {
                    let deg = c0.clamp(-1.0, 1.0).acos().to_degrees();
                    if deg.is_finite() && deg > max_deg + 1e-12_f64.to_degrees() {
                        notimpl!(
                            "coating {key}: incidence angle {deg:.4} deg is outside the measured table range (0 to {max_deg} deg)"
                        );
                    }
                }
                let x = xcos_s(*r, s0);
                // 4 本のスプラインは同じ節なので、区間は 1 回だけ探す
                let i = sp[0].locate(x);
                [
                    C64::new(sp[0].eval_at(i, x), sp[1].eval_at(i, x)),
                    C64::new(sp[2].eval_at(i, x), sp[3].eval_at(i, x)),
                ]
            }
            Kind::Model {
                name,
                layer_phase,
                reflect,
                reverse,
                n2,
                d,
                bmax,
                b2,
                sp,
            } => {
                let b = self.n1 * s0;
                let x = b * b;
                if *reflect {
                    if x < b2.0 - 1e-12 || x > b2.1 + 1e-12 {
                        let top = b2.1.sqrt();
                        notimpl!("coating model {name} (reflection): β = {b:.4} is outside the model range (β ≤ {top:.4})");
                    }
                    let k = 2.0 * PI / self.wl;
                    let ph = if *reverse {
                        C64::new(1.0, 0.0)
                    } else {
                        C64::new(0.0, 2.0 * k * self.n1 * c0 * d).exp()
                    };
                    let mut out = [C64::new(0.0, 0.0); 2];
                    for (pol, o) in out.iter_mut().enumerate() {
                        let s = &sp[pol];
                        let m: [C64; 4] = std::array::from_fn(|c| {
                            C64::new(s[2 * c].eval(x), s[2 * c + 1].eval(x))
                        });
                        let (m00, m10, m01, m11) = if *reverse {
                            (m[3], m[1], m[2], m[0])
                        } else {
                            (m[0], m[1], m[2], m[3])
                        };
                        let e1 = admittance(self.n1, b, pol);
                        let e2 = admittance_ev(*n2, b, pol);
                        let (bb, cc) = (m00 + m01 * e2, m10 + m11 * e2);
                        *o = (e1 * bb - cc) / (e1 * bb + cc) * ph;
                    }
                    return Ok(out);
                }
                if (*layer_phase && b > bmax + 1e-12) || x < b2.0 - 1e-12 || x > b2.1 + 1e-12 {
                    let top = b2.1.sqrt();
                    notimpl!("coating model {name}: β = {b:.4} is outside the model range (β ≤ {top:.4}, BMAX {bmax})");
                }
                if n2 * n2 <= x {
                    // 後ろの媒質で全反射（光線の追跡ではその前に止まる）
                    return Ok([C64::new(0.0, 0.0); 2]);
                }
                let n_out = if *reverse { *n2 } else { self.n1 };
                let co = (1.0 - x / (n_out * n_out)).max(0.0).sqrt();
                if co == 0.0 {
                    notimpl!("coating transmission: total reflection in the outer medium n = {n_out} (β = {b})");
                }
                let k = 2.0 * PI / self.wl;
                let ph = C64::new(0.0, k * d * n_out / co).exp();
                let mut out = [C64::new(0.0, 0.0); 2];
                // 18 本のスプラインは同じ β² の節（ψ はその前の部分）なので、区間は 1 回だけ探す
                let i = sp[0][0].locate(x);
                for (pol, o) in out.iter_mut().enumerate() {
                    let s = &sp[pol];
                    let g = if *layer_phase {
                        C64::new(0.0, k * s[8].eval_at(i, x)).exp()
                    } else {
                        C64::new(1.0, 0.0)
                    };
                    let m: [C64; 4] = std::array::from_fn(|c| {
                        g * C64::new(s[2 * c].eval_at(i, x), s[2 * c + 1].eval_at(i, x))
                    });
                    let (m00, m10, m01, m11) = if *reverse {
                        (m[3], m[1], m[2], m[0])
                    } else {
                        (m[0], m[1], m[2], m[3])
                    };
                    let e1 = admittance(self.n1, b, pol);
                    let e2 = admittance(*n2, b, pol);
                    let tp = e1 * 2.0 / (e1 * m00 + m10 + e1 * e2 * m01 + e2 * m11);
                    *o = tp * (e2 / e1).sqrt() * ph;
                }
                out
            }
            Kind::ModelMirror {
                name,
                n_ref,
                d,
                b2,
                r: rr,
                branch,
                band,
            } => {
                let b = self.n1 * s0;
                let x = b * b;
                let (lo, hi) = (b2[0], b2[b2.len() - 1]);
                if x < lo - 1e-12 || x > hi + 1e-12 {
                    let top = hi.sqrt();
                    notimpl!("coating model {name} (mirror): β = {b:.4} is outside the model range (β ≤ {top:.4})");
                }
                if let Some((a, c)) = band {
                    if *a <= x && x <= *c {
                        let (lo, hi) = (a.sqrt(), c.sqrt());
                        notimpl!("coating model {name} (mirror): β = {b:.5} is near where the outer medium turns to total reflection (β {lo:.5} to {hi:.5}); too few measured points to set the value");
                    }
                }
                let k = 2.0 * PI / self.wl;
                let ph = C64::new(0.0, 2.0 * k * self.n1 * c0 * d).exp();
                let mut out = [C64::new(0.0, 0.0); 2];
                for (pol, o) in out.iter_mut().enumerate() {
                    let r = match branch.as_deref() {
                        Some(bb) => bb.eval(x, &|j| rr[pol][j]),
                        None => rational_at(b2, 0, b2.len(), x, &|j| rr[pol][j]),
                    };
                    let one = C64::new(1.0, 0.0);
                    let y = admittance(*n_ref, b, pol) * (one - r) / (one + r);
                    let e0 = admittance(self.n1, b, pol);
                    *o = (e0 - y) / (e0 + y) * ph;
                }
                out
            }
        })
    }
}
