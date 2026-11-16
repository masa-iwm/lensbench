//! 近軸追跡・近軸量・射出瞳・対称性

use super::*;
use crate::action::Action;
use crate::shape::Shape;

impl System {
    /// 面 1 の頂点面で高さ y1、物体空間の傾き u0 の近軸光線 → 各面の頂点面での高さ y と面の後の傾き u（展開した向き）
    pub fn ptrace(&self, y1: f64, u0: f64, w: usize, upto: Option<usize>) -> (Vec<f64>, Vec<f64>) {
        let sf = &self.surfaces;
        let upto = upto.unwrap_or(sf.len() - 1);
        let mut y = vec![f64::NAN; sf.len()];
        let mut u = vec![f64::NAN; sf.len()];
        let mut n = self.n(0)[w];
        let (mut uu, mut yy, mut sgn) = (u0, y1, 1.0);
        let mut gmed: Option<(&crate::grin::Grin, f64)> = None;
        y[0] = yy - uu * if sf[0].t.is_finite() { sf[0].t } else { 0.0 };
        u[0] = uu;
        for j in 1..=upto {
            let d = &sf[j];
            if d.ignore {
                // 無視する面は位置も厚さも持たない。直前の高さ・傾きを写して、全番号で「面 j の後」が定義されるようにする
                // （像面の直前を無視すると u[img - 1] が NaN になり、近軸量・射出瞳が壊れていた）
                y[j] = yy;
                u[j] = uu;
                continue;
            }
            y[j] = yy;
            let action = d.action();
            match action {
                _ if d.is_coord_break() => {}
                Action::Paraxial { f, .. } => uu -= yy / f,
                Action::User(u) => {
                    if let Some(u) = u {
                        uu -= yy * u.instance().paraxial_power();
                    }
                }
                Action::Jones(_) => n = self.n(j)[w],
                Action::Refract
                | Action::Reflect { .. }
                | Action::Binary2(_)
                | Action::Fresnel(_)
                | Action::Birefringent(_) => {
                    // 形の近軸の曲率（トロイダル面は YZ 面の近軸。OpticStudio と同じ）。Fresnel の屈折は
                    // 屈折の面（YZ 面）で決まる
                    let bend = match action {
                        Action::Fresnel(b) => b.paraxial_curv(),
                        _ => None,
                    };
                    let c = sgn * bend.unwrap_or_else(|| d.shape().paraxial_curv());
                    // Binary 2 の位相の 2 次の項: n′u′ に (λ/2π) ∂Φ/∂y を足す（λ はレンズの単位 mm）
                    let dphi = match action {
                        Action::Binary2(Some(b)) => {
                            let a = b.coef.first().copied().unwrap_or(0.0);
                            self.wavelengths[w] * 1e-3 / (2.0 * std::f64::consts::PI)
                                * b.order
                                * 2.0
                                * a
                                / (b.norm * b.norm)
                                * yy
                        }
                        _ => 0.0,
                    };
                    if d.mirror {
                        uu += 2.0 * c * yy - sgn * dphi / n;
                        sgn = -sgn;
                    } else {
                        // GRIN の媒質へは頂点の屈折率で
                        let n2 = d
                            .grin
                            .as_deref()
                            .and_then(|g| g.vertex_index(self.wavelengths[w]))
                            .unwrap_or(self.n(j)[w]);
                        uu = (n * uu - yy * c * (n2 - n) + sgn * dphi) / n2;
                        n = n2;
                    }
                }
            }
            u[j] = uu;
            // GRIN の媒質（座標ブレークの後ろも続く）と、その面の局所 z の今の位置
            gmed = match d.grin.as_deref().filter(|_| !d.mirror) {
                Some(g) => Some((g, 0.0)),
                None if d.is_coord_break() => gmed,
                None => None,
            };
            if j < upto {
                match gmed {
                    // GRIN の媒質の中は分布の近軸の受け渡し（出口の屈折率が次の面の前の屈折率）
                    Some((g, z0)) => {
                        match g.paraxial(self.wavelengths[w], yy, uu, z0, d.t, sgn) {
                            Some((y2, u2, n2)) => {
                                yy = y2;
                                uu = u2;
                                n = n2;
                            }
                            None => {
                                yy = f64::NAN;
                                uu = f64::NAN;
                            }
                        }
                        gmed = Some((g, z0 + d.t));
                    }
                    None => yy += sgn * d.t * uu,
                }
            }
        }
        (y, u)
    }

    pub(super) fn stop_lin(&self) -> (f64, f64) {
        let s = self.stop();
        let a = self.ptrace(1.0, 0.0, self.primary, Some(s)).0[s];
        let b = self.ptrace(0.0, 1.0, self.primary, Some(s)).0[s];
        (a, b)
    }

    /// 主波長の近軸量
    pub fn paraxial(&self) -> &Paraxial {
        self.cache.par.get_or_init(|| self.compute_paraxial())
    }

    pub(super) fn compute_paraxial(&self) -> Paraxial {
        let sf = &self.surfaces;
        let s = self.stop();
        let t0 = sf[0].t;
        let pw = self.primary;
        let (a, b) = self.stop_lin();
        let enpp = if self.telecentric || a.abs() < 1e-300 {
            ENPP_MAX
        } else {
            let e = b / a;
            if e.abs() >= ENPP_MAX {
                ENPP_MAX
            } else {
                e
            }
        };
        let img = self.image();
        let (_, uk) = self.ptrace(1.0, 0.0, pw, None);
        let n_i = self.image_index(pw);
        // 空気中の値（像空間の屈折率をかけた角度で割る）。像面がガラスの中でも OpticStudio の EFFL は
        // R/(n−1) の形（2026 R1.01 で実測）
        let efl = if uk[img - 1] != 0.0 {
            -1.0 / (n_i * uk[img - 1])
        } else {
            f64::INFINITY
        };
        let (kind, val) = self.aperture;
        let mut u_obj: Option<f64> = None;
        let epd = match kind {
            ApertureKind::Epd => val,
            ApertureKind::FNum => efl.abs() / val,
            ApertureKind::Float => {
                let semi = sf[s].semi;
                if self.infinite() {
                    2.0 * semi / a.abs()
                } else {
                    let u = semi / (b + a * t0).abs();
                    u_obj = Some(u);
                    2.0 * u * (enpp + t0).abs()
                }
            }
            ApertureKind::Na | ApertureKind::Cone => {
                let n0 = self.n(0)[pw];
                let th = if kind == ApertureKind::Na {
                    (val / n0).asin()
                } else {
                    val.to_radians()
                };
                let u = th.tan();
                u_obj = Some(u);
                2.0 * u * (enpp + t0).abs()
            }
            ApertureKind::Pwfn => {
                if self.infinite() {
                    let (_, um) = self.ptrace(1.0, 0.0, pw, None);
                    2.0 * 1.0 / (2.0 * val * um[img - 1].abs() * n_i)
                } else {
                    let (_, um) = self.ptrace(t0, 1.0, pw, None);
                    let u = 1.0 / (2.0 * val * um[img - 1].abs() * n_i);
                    u_obj = Some(u);
                    2.0 * u * (enpp + t0).abs()
                }
            }
        };
        let (ym, um) = if self.infinite() {
            self.ptrace(epd / 2.0, 0.0, pw, None)
        } else {
            let u = *u_obj.get_or_insert(epd / 2.0 / (enpp + t0));
            self.ptrace(u * t0, u, pw, None)
        };
        let (yc, uc) = if self.infinite() {
            self.ptrace(-enpp, 1.0, pw, None)
        } else if enpp >= ENPP_MAX {
            self.ptrace(1.0, 0.0, pw, None)
        } else {
            let u0 = -1.0 / (enpp + t0);
            self.ptrace(1.0 + u0 * t0, u0, pw, None)
        };
        let (yci, uci) = (yc[img], uc[img - 1]);
        let (ymi, umi) = (ym[img], um[img - 1]);
        // 像面から近軸像面までずらした主光線の高さ（像面が近軸像面にあれば yci と同じ）
        let ycp = if umi != 0.0 {
            yci - uci * ymi / umi
        } else {
            yci
        };
        let mut expp = if uci != 0.0 {
            -yci / uci
        } else {
            f64::INFINITY
        };
        let nmir = sf[1..img].iter().filter(|d| d.mirror && !d.ignore).count();
        if nmir % 2 == 1 {
            expp = -expp;
        }
        let expd = if expp.is_finite() {
            2.0 * (ymi + umi * expp).abs()
        } else {
            f64::INFINITY
        };
        Paraxial {
            enpp,
            epd,
            efl,
            expp,
            expd,
            stop: s,
            stop_radius: ym[s].abs(),
            u_obj,
            chief_image_per_unit: yci,
            chief_parimage_per_unit: ycp,
            wfno: if umi != 0.0 {
                1.0 / (2.0 * n_i * umi.abs())
            } else {
                f64::INFINITY
            },
            isfn: efl.abs() / epd,
            pmag: if !self.infinite() && umi != 0.0 {
                self.n(0)[pw] * um[0] / (n_i * umi)
            } else {
                0.0
            },
            marginal: (ym, um),
            chief: (yc, uc),
        }
    }

    /// 座標ブレークや面の傾き偏心が面を軸から動かしていなければ true
    pub fn symmetric(&self) -> bool {
        self.symmetric_opts(true)
    }

    /// [`System::symmetric`]。freeform が false なら Zernike・Grid Sag の面を見ない（OpticStudio の自動の
    /// 半直径の光線の取り方は、これらの面があっても回転対称の系と同じ）
    pub(crate) fn symmetric_opts(&self, freeform: bool) -> bool {
        for d in &self.surfaces {
            if d.ignore {
                continue;
            }
            if d.is_coord_break() && d.cb.iter().any(|&v| v != 0.0) {
                return false;
            }
            if d.before.is_some() || matches!(d.after, After::Tilt(_)) {
                return false;
            }
            // Zernike・Grid Sag・Odd Asphere・Tilted・Irregular の面は回転対称とみない（瞳は実光線で求める。OpticStudio と
            // 同じ。自作の系で、近軸の瞳にすると EXPD・OPDC の OpticStudio との差が大きくなった）
            if freeform && d.shape().is_freeform() {
                return false;
            }
            // GRIN の公差の項・Gradient 9 の形の傾き
            if d.grin.as_deref().is_some_and(|g| !g.symmetric()) {
                return false;
            }
        }
        !self.units.iter().any(|u| u.moved())
    }

    /// 射出瞳の位置（像面からその局所 z に沿って。OPD の基準）。回転対称なら近軸、そうでなければ
    /// OpticStudio と同じく小さい y 視野の実主光線が軸上主光線と交わる位置（2 視野で外挿）
    pub fn exit_pupil(&self) -> f64 {
        *self.cache.expp.get_or_init(|| {
            let mut e = self.paraxial().expp;
            if !self.symmetric() {
                // 補助の光線の物体指定が解けなければ射出瞳は求まらない（NaN。OPD は誤りにする）
                let Some(r0) = self.trace_novig(0.0, 0.0, &[0.0], &[0.0]) else {
                    return f64::NAN;
                };
                let (p0, k0) = (r0.rays[0].xyz, r0.rays[0].lmn);
                // GRIN の分布・形を傾けた面も同じ（G8B で OpticStudio と OPDC 5e-7 波）
                let free = self.surfaces.iter().any(|d| {
                    !d.ignore
                        && (d.shape().is_freeform()
                            || d.grin.as_deref().is_some_and(|g| !g.symmetric()))
                });
                let cross_z = |p1: V3, k1: V3| -> f64 {
                    if free {
                        // Zernike・Grid Sag・傾けた GRIN の面を含む系は y-z 面で交わる点（3 次元の最近点だと、
                        // 面の傾きの項で主光線が x にも振れる系でずれる。OpticStudio 26.1 の EXPP と、鏡 1 枚の
                        // Z2 で 3e-12、G3 で 8e-7 一致。OpticStudio との照合）
                        let det = k1[1] * k0[2] - k0[1] * k1[2];
                        let (ry, rz) = (p1[1] - p0[1], p1[2] - p0[2]);
                        let t = (rz * k1[1] - ry * k1[2]) / det;
                        p0[2] + k0[2] * t
                    } else {
                        // 傾き・偏心の系は 3 次元の最近点（C0・C1 で OpticStudio と一致）
                        let w0 = sub(p0, p1);
                        let (a, b, c) = (dot(k0, k0), dot(k0, k1), dot(k1, k1));
                        let (d, f) = (dot(k0, w0), dot(k1, w0));
                        add(p0, scale(k0, (b * f - c * d) / (a * c - b * b)))[2]
                    }
                };
                if self.cusp_pupil() {
                    // 頂点の尖った面がある系: OpticStudio と同じく視野 0.01° の実主光線 1 本（外挿しない。外挿すると
                    // 尖りの傾きの分が消える）。その光線の高さが絞りの後のどこかの面で近軸の主光線と逆の符号になれば
                    // （尖りで曲がって軸を横切る）、視野を 1/100 にした光線で求め直す
                    let yc = &self.paraxial().chief.0;
                    let (s, img) = (self.stop(), self.image());
                    let chief = |deg: f64| {
                        self.trace_spec(
                            ObjSpec::Dir(normalize([0.0, deg.to_radians().tan(), 1.0])),
                            &[0.0],
                            &[0.0],
                            self.primary,
                            TraceOpts::new().vig([0.0; 5]).record(),
                        )
                    };
                    let mut r1 = chief(CUSP_FIELD_DEG);
                    let flipped = r1
                        .rec
                        .iter()
                        .any(|q| q.surf > s && q.surf < img && q.xyz[0][1] * yc[q.surf] < 0.0);
                    if flipped {
                        r1 = chief(CUSP_FIELD_DEG / 100.0);
                    }
                    e = cross_z(r1.rays[0].xyz, r1.rays[0].lmn);
                } else {
                    let mut z = [0.0; 2];
                    for (i, h) in [2e-4, 1e-4].iter().enumerate() {
                        let r1 = if self.on_axis_only() {
                            // 視野が軸上だけの系は正規化視野が軸上に潰れる（object_spec）ので、最大視野 10（度・mm）の
                            // 系と同じ小さい値（視野の単位で 2e-3・1e-3）の主光線にする。以前は 2 本とも軸上の主光線に
                            // なり、交点が 0/0 で NaN、OPD の基準が黙って平面に落ちていた。C0 で最大視野を 0.5〜50 に
                            // した外挿は 47.4273〜47.4292（小さいほど丸めで揺れ、5 以上で 47.42734〜47.42754）
                            match self.object_spec_abs(0.0, *h * ON_AXIS_PUPIL_FIELD) {
                                Ok(spec) => self.trace_spec(
                                    spec,
                                    &[0.0],
                                    &[0.0],
                                    self.primary,
                                    TraceOpts::new().vig([0.0; 5]),
                                ),
                                Err(_) => {
                                    z[i] = f64::NAN;
                                    continue;
                                }
                            }
                        } else {
                            match self.trace_novig(0.0, *h, &[0.0], &[0.0]) {
                                Some(t) => t,
                                None => {
                                    z[i] = f64::NAN;
                                    continue;
                                }
                            }
                        };
                        z[i] = cross_z(r1.rays[0].xyz, r1.rays[0].lmn);
                    }
                    e = (4.0 * z[1] - z[0]) / 3.0;
                }
            }
            e
        })
    }

    /// r の 1 次の項（β1、頂点が尖る）がある Odd Asphere の面を含む、無限遠物体・角度の視野の系なら true。
    /// OpticStudio 2026 R1.01 はこの系の瞳を、視野 0.01° の実主光線 1 本と、軸上の視野の +y 側だけの瞳座標 1e-3 の
    /// 光線 1 本で求める（尖りの傾き β1 がそのまま瞳の位置・径に効く。β1 = ±1e-3・1e-5、入射瞳径 10・20、
    /// 最大視野 2・5・10° の自作の系で EXPP・EXPD が 1e-5 mm 一致。OpticStudio との照合）。
    /// 外挿や ±両側の光線で求める極限の値とは違う
    pub(crate) fn cusp_pupil(&self) -> bool {
        self.infinite()
            && self.field_type == FieldType::Angle
            && self.surfaces.iter().any(|d| {
                !d.ignore
                    && matches!(d.shape(), Shape::OddAsphere { b, .. } if b.first().is_some_and(|&b1| b1 != 0.0))
            })
    }

    /// 瞳の算出用: ケラレ係数を掛けずに主波長で追う（瞳は系の性質で、視野のケラレ係数とは無関係）
    /// 物体指定が解けない（実像高の視野で補助の光線が求まらないなど）なら None
    pub(super) fn trace_novig(&self, hx: f64, hy: f64, px: &[f64], py: &[f64]) -> Option<Trace> {
        let spec = self.object_spec(hx, hy).ok()?;
        let mut t = self.trace_spec(
            spec,
            px,
            py,
            self.primary,
            TraceOpts::new().vig([0.0; 5]).hxy((hx, hy)),
        );
        t.hx_hy = (hx, hy);
        Some(t)
    }

    /// 射出瞳径（OpticStudio の EXPD）
    pub fn exit_pupil_diameter(&self) -> f64 {
        *self.cache.expd.get_or_init(|| {
            let e = self.exit_pupil();
            if e.is_nan() {
                return f64::NAN;
            }
            if (self.aiming == Aiming::Off && self.symmetric()) || !e.is_finite() {
                self.paraxial().expd
            } else {
                // 頂点の尖った面がある系は OpticStudio と同じく +y 側の瞳座標 1e-3 の光線だけ（[`System::cusp_pupil`]）
                let cusp = self.cusp_pupil();
                let dl = if cusp { 1e-3 } else { 1e-4 };
                let Some(r) = self.trace_novig(0.0, 0.0, &[0.0, 0.0, 0.0], &[0.0, dl, -dl]) else {
                    return f64::NAN;
                };
                let p: Vec<V3> = r.rays.iter().map(|x| x.xyz).collect();
                let k: Vec<V3> = r.rays.iter().map(|x| x.lmn).collect();
                let c = k[0];
                let cc = add(p[0], scale(c, (e - p[0][2]) / c[2]));
                let q: Vec<V3> = (0..3)
                    .map(|i| add(p[i], scale(k[i], dot(sub(cc, p[i]), c) / dot(k[i], c))))
                    .collect();
                if cusp {
                    2.0 * norm(sub(q[1], q[0])) / dl
                } else {
                    norm(sub(q[1], q[2])) / dl
                }
            }
        })
    }

    /// 理想レンズ j の近軸の物体距離（局所 z、進行方向に。無限大は平行光）
    pub fn lens_conjugate(&self, j: usize) -> f64 {
        self.cache.conj.get_or_init(|| {
            let (ym, um) = &self.paraxial().marginal;
            (0..self.surfaces.len())
                .map(|j| {
                    let prev = (0..j)
                        .rev()
                        .find(|&i| !self.surfaces[i].ignore)
                        .unwrap_or(0);
                    let u_in = um[prev];
                    if u_in.abs() > 1e-15 * ym[j].abs().max(1.0) {
                        -ym[j] / u_in
                    } else {
                        f64::INFINITY
                    }
                })
                .collect()
        })[j]
    }

    // ---------------------------------------------------------------------------------- 視野
}
