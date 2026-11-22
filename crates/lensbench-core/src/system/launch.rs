//! 光線の生成と追跡の入口（瞳座標・物体・記録・Fresnel の座標系）

use super::*;

/// 光線ごとの開始位置・方向（大域）。瞳座標の光線は入射瞳の上の点だけを持ち、使うときに
/// [`System::start_ray`] で求める（大きな配列を作らない。値は先に求めるのと同じビット）
pub(super) enum Starts {
    Done(Vec<(V3, V3)>),
    Pupil {
        geom: StartGeom,
        ex: Vec<f64>,
        ey: Vec<f64>,
    },
}

/// [`System::start_ray`] のうち瞳の点によらない量（物体の座標系、入射瞳の位置、平行光の方向・物点）。
/// 光線ごとに座標系・近軸量を引き直さないよう、1 回の追跡で 1 度だけ求める。値は同じ式・同じ順なので同じビット
#[derive(Clone, Copy)]
pub(super) struct StartGeom {
    pub(super) o1: V3,
    pub(super) r0: M3,
    pub(super) z: f64,
    pub(super) kind: StartKind,
}

#[derive(Clone, Copy)]
pub(super) enum StartKind {
    /// 大域の方向
    Dir(V3),
    /// 大域の物点
    Point(V3),
}

impl StartGeom {
    pub(super) fn new(s: &System, spec: ObjSpec) -> StartGeom {
        let fr = s.frames();
        let (o1, r0) = (fr.obj.o, fr.obj.r);
        let z = s.paraxial().enpp.min(ENPP_MAX);
        let kind = match spec {
            ObjSpec::Dir(d) => StartKind::Dir(mv(&r0, d)),
            ObjSpec::Point(pt) => {
                let f0 = fr.surf[0].expect("有限物体の座標系");
                // 物体面が平面でない形（Tilted など）なら物点はその面の上（OpticStudio と同じ）
                let sh = s.surfaces[0].shape();
                let z0 = if sh.is_freeform() {
                    sh.sag(pt[0], pt[1]).0
                } else {
                    0.0
                };
                StartKind::Point(add(f0.o, mv(&f0.r, [pt[0], pt[1], z0])))
            }
        };
        StartGeom { o1, r0, z, kind }
    }

    #[inline]
    pub(super) fn ray(&self, ex: f64, ey: f64) -> (V3, V3) {
        let (o1, r0) = (self.o1, &self.r0);
        let e = add(o1, mv(r0, [ex, ey, self.z]));
        match self.kind {
            StartKind::Dir(d) => {
                let p = sub(e, scale(d, dot(sub(e, o1), d)));
                (p, d)
            }
            StartKind::Point(p) => {
                let mut k = sub(e, p);
                if dot(k, col(r0, 2)) < 0.0 {
                    k = scale(k, -1.0);
                }
                (p, normalize(k))
            }
        }
    }
}

impl Starts {
    pub(super) fn len(&self) -> usize {
        match self {
            Starts::Done(v) => v.len(),
            Starts::Pupil { ex, .. } => ex.len(),
        }
    }

    #[inline]
    pub(super) fn get(&self, i: usize) -> (V3, V3) {
        match self {
            Starts::Done(v) => v[i],
            Starts::Pupil { geom, ex, ey } => geom.ray(ex[i], ey[i]),
        }
    }
}

impl System {
    /// 物体の指定と瞳座標の光線を波長 w で追跡する（ケラレ係数・止める面・記録・視野は [`TraceOpts`]）
    pub fn trace_spec(
        &self,
        spec: ObjSpec,
        px: &[f64],
        py: &[f64],
        w: usize,
        o: TraceOpts,
    ) -> Trace {
        let o = (o.vig, o.upto, o.record, o.hxy);
        self.trace_spec_inner(spec, px, py, w, o.0, o.1, o.2, o.3, None)
            .0
    }

    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub(super) fn trace_spec_inner(
        &self,
        spec: ObjSpec,
        px: &[f64],
        py: &[f64],
        w: usize,
        vig: Option<[f64; 5]>,
        last: Option<usize>,
        record: bool,
        hxy: Option<(f64, f64)>,
        pol: Option<(&[[C64; 2]], &[PolSurf], bool)>,
    ) -> (Trace, PolField, Option<Error>) {
        let l = Launch::Pupil {
            spec,
            signed: false,
            px,
            py,
            vig,
            hxy,
        };
        self.trace_launch_inner(&l, w, last, RecSel::from(record), pol)
    }

    /// 光線の始め方 l から、光線ごとの開始位置・方向（大域）と、解けなかった光線の印を求める
    pub(super) fn launch_rays(&self, l: &Launch, w: usize) -> (Vec<(V3, V3)>, Vec<bool>) {
        let (st, fail) = self.launch_starts(l, w);
        let st = match st {
            Starts::Done(v) => v,
            Starts::Pupil { .. } => {
                let start = |i: usize| st.get(i);
                let n = st.len();
                if n >= PAR_MIN_RAYS && !crate::par::sequential() {
                    (0..n)
                        .into_par_iter()
                        .with_min_len(PAR_CHUNK)
                        .map(scoped(start))
                        .collect()
                } else {
                    (0..n).map(start).collect()
                }
            }
        };
        (st, fail)
    }

    /// 物体 spec・入射瞳の座標 v（光線エイミングの未知数と同じ: 近軸の入射瞳の面の上の点 [mm]、物体の座標系）の光線の
    /// 開始位置・方向（大域。[`Launch::Rays`] で追う）。[`Launch::Pupil`] の光線は、瞳座標を [`Self::aim_pupil`] で
    /// v に直したもの
    pub fn pupil_ray(&self, spec: ObjSpec, v: [f64; 2]) -> (V3, V3) {
        StartGeom::new(self, spec).ray(v[0], v[1])
    }

    /// 物体 spec・正規化瞳座標 p（けられ係数・符号の反転なし）の光線の入射瞳の座標（[`Self::pupil_ray`]。エイミングは
    /// 系の設定どおりで、[`Launch::Pupil`] と同じ）。エイミングで解けなければ None
    pub fn aim_pupil(&self, spec: ObjSpec, p: [f64; 2], w: usize) -> Option<[f64; 2]> {
        let l = Launch::Pupil {
            spec,
            signed: false,
            px: &[p[0]],
            py: &[p[1]],
            vig: Some([0.0; 5]),
            hxy: None,
        };
        let (st, fail) = self.launch_starts(&l, w);
        match st {
            Starts::Pupil { ex, ey, .. } if !fail[0] => Some([ex[0], ey[0]]),
            _ => None,
        }
    }

    /// [`Self::aim_pupil`] で、近くの光線（物体 from・入射瞳の座標 v0。隣の視野・瞳の点）から解く（近くの光線を 1 本ずつ
    /// 続けて追う解析で、光線ごとに主光線の継続法から解き直さないため）。解けなければ None（そのときは
    /// [`Self::aim_pupil`] で解き直す）。エイミングの無い系は近軸の入射瞳の点
    pub fn aim_pupil_near(
        &self,
        from: ObjSpec,
        v0: [f64; 2],
        spec: ObjSpec,
        p: [f64; 2],
        w: usize,
    ) -> Option<[f64; 2]> {
        if self.aiming == Aiming::Off {
            let epd = self.paraxial().epd;
            return Some([p[0] * epd / 2.0, p[1] * epd / 2.0]);
        }
        let (rx, ry) = self.stop_radius();
        self.aim_near(from, v0, spec, [p[0] * rx, p[1] * ry], w)
    }

    /// [`Self::launch_rays`] で、瞳座標の光線は開始位置・方向をまだ求めない（追跡の組の中で求める）
    pub(super) fn launch_starts(&self, l: &Launch, w: usize) -> (Starts, Vec<bool>) {
        match *l {
            Launch::Pupil {
                spec,
                signed,
                px,
                py,
                vig,
                hxy,
            } => {
                let n = px.len();
                // 瞳座標（符号の反転・けられ係数をかけたもの）。作業の配列はこの 2 本だけ
                let sg = if signed && self.pupil_reversed() {
                    -1.0
                } else {
                    1.0
                };
                let vig = match vig {
                    Some(v) => Ok(v),
                    None => {
                        let (hx, hy) = hxy.unwrap_or((0.0, 0.0));
                        self.vig_factors(hx, hy)
                    }
                };
                // ケラレ係数が決められない点（視野が有限でない）は、係数 0 で黙って追わず全光線を失敗にする
                // （try_trace は同じ条件で誤りを返す）
                let Ok([vdx, vdy, vcx, vcy, van]) = vig else {
                    return (
                        Starts::Pupil {
                            geom: StartGeom::new(self, spec),
                            ex: vec![f64::NAN; n],
                            ey: vec![f64::NAN; n],
                        },
                        vec![true; n],
                    );
                };
                let (qx, qy): (Vec<f64>, Vec<f64>) =
                    if vdx != 0.0 || vdy != 0.0 || vcx != 0.0 || vcy != 0.0 || van != 0.0 {
                        let (s, c) = van.to_radians().sin_cos();
                        (0..n)
                            .map(|i| {
                                let (px, py) = (sg * px[i], sg * py[i]);
                                let (a, b) = (vdx + px * (1.0 - vcx), vdy + py * (1.0 - vcy));
                                (a * c - b * s, a * s + b * c)
                            })
                            .unzip()
                    } else if sg < 0.0 {
                        (
                            px.iter().map(|v| -v).collect(),
                            py.iter().map(|v| -v).collect(),
                        )
                    } else {
                        (px.to_vec(), py.to_vec())
                    };
                let epd = self.paraxial().epd;
                let mut ex: Vec<f64> = qx.iter().map(|v| v * epd / 2.0).collect();
                let mut ey: Vec<f64> = qy.iter().map(|v| v * epd / 2.0).collect();
                let mut fail = vec![false; n];
                if self.aiming != Aiming::Off {
                    self.aim(spec, &qx, &qy, &mut ex, &mut ey, &mut fail, w);
                }
                (
                    Starts::Pupil {
                        geom: StartGeom::new(self, spec),
                        ex,
                        ey,
                    },
                    fail,
                )
            }
            Launch::Stop { spec, x, y } => {
                let (st, fail) = self.aim_stop(spec, x, y, w);
                (Starts::Done(st), fail)
            }
            Launch::Rays { p, k } => {
                let st = p
                    .iter()
                    .zip(k)
                    .map(|(&p, &k)| {
                        let nk = norm(k);
                        (
                            p,
                            if nk > 0.0 {
                                scale(k, 1.0 / nk)
                            } else {
                                [f64::NAN; 3]
                            },
                        )
                    })
                    .collect();
                (Starts::Done(st), vec![false; p.len()])
            }
        }
    }

    /// 有限物体で入射瞳が物体より前にあるか（ENPP + 物体距離 < 0）。このとき OpticStudio の瞳座標の光線は
    /// 入射瞳の点から遠ざかる向きになり、px = 1 は絞りの反対側を通る（[`Launch::Pupil`] の signed）
    pub fn pupil_reversed(&self) -> bool {
        !self.infinite() && self.paraxial().enpp + self.surfaces[0].t < 0.0
    }

    /// 物体の座標系（有限物体は物体面、無限遠物体は面 1 の変換の前の座標系）の原点と回転（大域）。
    /// [`Launch::Rays`] に物体の座標系で与えた光線を大域に直すのに使う
    pub fn object_frame(&self) -> Frame {
        let fr = self.frames();
        fr.surf[0].unwrap_or(fr.obj)
    }

    /// 光線の始め方 l で追跡する（波長 w、面 last まで）。rec は記録する面（[`RecSel`]、bool なら全部か無し）。
    /// 面の多い系で一部の面だけ記録すると速い
    pub fn trace_launch<'r>(
        &self,
        l: &Launch,
        w: usize,
        last: Option<usize>,
        rec: impl Into<RecSel<'r>>,
    ) -> Trace {
        self.trace_launch_inner(l, w, last, rec.into(), None).0
    }

    /// 光線の始め方 l での偏光追跡（[`Self::trace_pol_spec`] と同じ約束）。rec で面の交点も記録する
    pub fn trace_pol_launch<'r>(
        &self,
        l: &Launch,
        w: usize,
        pol: PolIn,
        rec: RecSel<'r>,
    ) -> Result<PolTrace> {
        let ps = self.pol_surfaces(w, pol.coat)?;
        let (t, e, err) =
            self.trace_launch_inner(l, w, None, rec, Some((pol.pols, &ps, pol.phase)));
        if let Some(e) = err {
            return Err(e);
        }
        Ok(PolTrace { trace: t, e })
    }

    /// Fresnel・Cylinder Fresnel の面（面 last まで）があれば、同じ物体・視野・波長の主光線（瞳座標 0、絞りの中心）の
    /// 交点を、その面の光路長の基準点にする（面の番号ごとの局所座標 (x, y)）。OpticStudio と同じ約束。
    /// 光線を直接与えたとき（主光線がない）と主光線が通らないときは頂点
    pub(super) fn fresnel_refs(&self, l: &Launch, w: usize, last: usize) -> Option<Vec<[f64; 2]>> {
        let is_f = |j: usize| {
            let d = &self.surfaces[j];
            !d.ignore && matches!(d.action(), crate::action::Action::Fresnel(_))
        };
        if !(1..=last).any(is_f) {
            return None;
        }
        let zero = [0.0];
        let chief = match *l {
            Launch::Pupil {
                spec,
                signed,
                vig,
                hxy,
                ..
            } => Launch::Pupil {
                spec,
                signed,
                px: &zero,
                py: &zero,
                vig,
                hxy,
            },
            Launch::Stop { spec, .. } => Launch::Stop {
                spec,
                x: &zero,
                y: &zero,
            },
            Launch::Rays { .. } => return None,
        };
        let (st, fail) = self.launch_rays(&chief, w);
        let fr = self.frames();
        let mut refs = vec![[0.0; 2]; self.surfaces.len()];
        if fail[0] {
            return Some(refs);
        }
        let nrec = (1..=last).filter(|&j| !self.surfaces[j].ignore).count();
        let mut rec = vec![trace::RecEntry::NAN; nrec];
        let (p, k) = st[0];
        let plan = batch::Plan::new(self, fr, w, last, self.floating_clip);
        let r = batch::trace_one(&plan, p, k, Some(&mut rec[..]));
        if r.status.is_ok() {
            for e in &rec {
                if is_f(e.surf) {
                    refs[e.surf] = [e.xyz[0], e.xyz[1]];
                }
            }
        }
        Some(refs)
    }

    #[allow(clippy::type_complexity)]
    pub(super) fn trace_launch_inner(
        &self,
        l: &Launch,
        w: usize,
        last: Option<usize>,
        rec: RecSel<'_>,
        pol: Option<(&[[C64; 2]], &[PolSurf], bool)>,
    ) -> (Trace, PolField, Option<Error>) {
        let mut dst = Dst::trace();
        let (d, es, err) = self.trace_launch_core(l, w, last, rec, pol, &mut dst);
        let Dst::Trace {
            rays,
            p0,
            k0,
            opl_os,
        } = dst
        else {
            unreachable!()
        };
        let t = Trace {
            rays,
            parity: d.parity,
            p0,
            k0,
            opl_os,
            rec: d.rec,
            last: d.last,
            hx_hy: d.hx_hy,
        };
        (t, es, err)
    }

    /// 光線の始め方 l で追跡し、結果を buf の列（`buf.cols()` の列と状態・けられ）に書く。
    /// 値は [`Self::trace_launch`] と同じビット。buf は呼び出しをまたいで使い回せる
    pub fn trace_launch_into(&self, l: &Launch, w: usize, last: Option<usize>, buf: &mut RayBuf) {
        let mut dst = Dst::Buf(buf);
        let (d, _, _) = self.trace_launch_core(l, w, last, RecSel::Off, None, &mut dst);
        buf.parity = d.parity;
        buf.last = d.last;
        buf.hx_hy = d.hx_hy;
    }

    /// 視野の光線を瞳座標で追跡し、結果を buf の列に書く（[`Self::try_trace`] の列版、記録なし）
    pub fn try_trace_into(
        &self,
        field: FieldSel,
        px: &[f64],
        py: &[f64],
        w: Option<usize>,
        last: Option<usize>,
        buf: &mut RayBuf,
    ) -> Result<()> {
        self.check_crystals()?;
        let w = w.unwrap_or(self.primary);
        let (hx, hy) = self.field_sel_norm(field, w)?;
        if let (Some(_), None) = (&self.cache.focus, last) {
            // 焦点をずらした像面の追跡は Trace の経路で（列に写す）
            let t = self.try_trace(field, px, py, Some(w), None, false)?;
            buf.fill_from(&t);
            return Ok(());
        }
        let spec = self.object_spec(hx, hy)?;
        let vig = self.vig_factors(hx, hy)?;
        let l = Launch::Pupil {
            spec,
            signed: false,
            px,
            py,
            vig: Some(vig),
            hxy: Some((hx, hy)),
        };
        self.trace_launch_into(&l, w, last, buf);
        buf.hx_hy = (hx, hy);
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    fn trace_launch_core(
        &self,
        l: &Launch,
        w: usize,
        last: Option<usize>,
        rec: RecSel<'_>,
        pol: Option<(&[[C64; 2]], &[PolSurf], bool)>,
        dst: &mut Dst,
    ) -> (Done, PolField, Option<Error>) {
        let (starts, fail) = self.launch_starts(l, w);
        let n = starts.len();
        // エイミングに失敗した光線は NaN から出す（位置・方向・光路長・記録・電場がすべて NaN になる）
        let start_of = |i: usize| {
            if fail[i] {
                ([f64::NAN; 3], [f64::NAN; 3])
            } else {
                starts.get(i)
            }
        };
        let (spec, hxy) = match *l {
            Launch::Pupil { spec, hxy, .. } => (Some(spec), hxy),
            Launch::Stop { spec, .. } => (Some(spec), None),
            Launch::Rays { .. } => (None, None),
        };
        let last = last.unwrap_or(self.image());
        let fres_ref = self.fresnel_refs(l, w, last);
        let fr = self.frames();
        let clip = self.floating_clip;
        let st = self.stop();
        let kw = self.wave(w).k_mm();
        // 記録する面（無視しない面）の配列を先に用意し、光線ごとにその位置へ直接書く
        let mut rec_out: Vec<RecSurf> = Vec::new();
        let mut slot = Vec::new();
        let record = !matches!(rec, RecSel::Off);
        if record {
            let mut heads = Vec::new();
            let mut parity = 1.0;
            for j in 1..=last {
                let d = &self.surfaces[j];
                if d.ignore {
                    continue;
                }
                if d.is_mirror() {
                    parity = -parity;
                }
                let want = match rec {
                    RecSel::Surfs(v) => v.contains(&j),
                    _ => true,
                };
                if want {
                    slot.push(heads.len());
                    heads.push((j, parity));
                } else {
                    slot.push(usize::MAX);
                }
            }
            // 配列の確保と NaN の書き込み（新しいページの割り当て）が追跡より重くなるので、面ごとに並列に
            let mk = |&(surf, parity): &(usize, f64)| RecSurf {
                surf,
                parity,
                xyz: vec![[f64::NAN; 3]; n],
                lmn: vec![[f64::NAN; 3]; n],
                opl: vec![f64::NAN; n],
                p: vec![[f64::NAN; 3]; n],
            };
            rec_out = if n * heads.len() >= 1 << 16 && !crate::par::sequential() {
                heads.par_iter().map(scoped(mk)).collect()
            } else {
                heads.iter().map(mk).collect()
            };
        }
        let ptrs = SoaPtrs::new(&mut rec_out, slot, n);
        // 偏光: 物体の座標系の (Ex, Ey, 0) を光線に垂直な面へ射影する（規格化しない、OpticStudio と同じ）
        let e_start = |inp: &[[C64; 2]], k: V3, out: &mut Vec<CV3>| {
            out.extend(inp.iter().map(|a| {
                let j: CV3 = [0, 1, 2].map(|c| a[0] * fr.obj.r[c][0] + a[1] * fr.obj.r[c][1]);
                let jk = j[0] * k[0] + j[1] * k[1] + j[2] * k[2];
                [0, 1, 2].map(|c| j[c] - jk * k[c])
            }))
        };
        let phase = pol.is_some_and(|p| p.2);
        // 最後の面の局所座標の電場に伝搬の位相 exp(−i k OPL) をかける（phase のとき）
        let put_phase = |e: &mut [CV3], opl: f64| {
            if phase {
                let ph = C64::new(0.0, -kw * opl).exp();
                e.iter_mut().for_each(|x| *x = x.map(|c| c * ph));
            }
        };
        // 面ごとの準備（光線によらない量）は 1 回だけ
        let mut plan = batch::Plan::new(self, fr, w, last, clip);
        if let Some(r) = fres_ref {
            plan = plan.with_fresnel_ref(r);
        }
        // 結果の書き先（無限遠物体の opl_os は面 1 の原点を通り軸に垂直な面から測る。面 1 が無視（IGNR）なら、
        // その次の有効な面（大域座標の基準と同じ））
        let os = match spec {
            Some(ObjSpec::Dir(_)) => {
                let j1 = (1..fr.surf.len())
                    .find(|&j| fr.surf[j].is_some())
                    .unwrap_or(1);
                let ob = fr.surf[j1].expect("frame of the object surface");
                Some((ob.o, col(&ob.r, 2), self.index_before(j1, w)))
            }
            _ => None,
        };
        let out = dst.prepare(n, os);
        let one = |i: usize| {
            let (p, k) = start_of(i);
            let mut row = SoaRow { out: &ptrs, i };
            let rec = if record { Some(&mut row) } else { None };
            let mut pr = pol.map(|(inp, surf, _)| {
                let mut e = Vec::with_capacity(inp.len());
                e_start(inp, k, &mut e);
                batch::PolLanes::<1>::new(e, inp.len(), surf)
            });
            let mut r = match pr.as_mut() {
                Some(q) => batch::trace_one_pol(&plan, p, k, rec, q),
                None => batch::trace_one(&plan, p, k, rec),
            };
            if fail[i] {
                r.status = RayStatus::AimFailed { stop: st };
            }
            out.put(i, &r, p, k);
            // 偏光なしの追跡で結果を大きくしないよう箱に入れる
            pr.map(|mut pr| {
                put_phase(&mut pr.e, r.opl);
                let [perr] = pr.err;
                Box::new((r.status.is_ok() && r.vig == 0, pr.e, perr))
            })
        };
        // 光線は互いに独立なので、本数が多いときは並列に追う（結果の順序は入力のまま、
        // スレッド数によらず同じビット）。with_threads(Some(1), ..) の中では逐次
        let par = n >= PAR_MIN_RAYS && !crate::par::sequential();
        // L 本ずつまとめて追う（1 本ずつと同じビット。組は入力の順に固定）。偏光ありは別の組で（下）
        let wide = batch::use_for(n).then_some(&plan);
        let (plan, pol_plan) = match pol {
            None => (wide, None),
            Some(_) => (None, wide),
        };
        let mut es = Vec::new();
        let mut first_err = None;
        // 束を止めるのは、最後まで通った（けられず、エイミングにも失敗していない）光線のエラーだけ。
        // ほかの光線は電場が NaN になるだけ
        let mut take_err = |good: bool, perr: Option<Error>| {
            if first_err.is_none() && good {
                first_err = perr;
            }
        };
        const L: usize = batch::L;
        let ng = n.div_ceil(L);
        if let Some(plan) = plan {
            // 結果の列へ組ごとに（並列に）直接書く（中間の配列を作らない）
            let group = |b: usize| {
                // 端の組の足りない分は最後の光線を繰り返す（結果は捨てる）
                let idx: [usize; L] = std::array::from_fn(|l| (b * L + l).min(n - 1));
                let start = idx.map(start_of);
                let (p, k) = (start.map(|s| s.0), start.map(|s| s.1));
                let mut sink = SoaLanes {
                    out: &ptrs,
                    base: b * L,
                };
                let r = batch::trace_lanes(plan, &p, &k, record.then_some(&mut sink));
                for l in 0..(n - b * L).min(L) {
                    let mut x = r[l];
                    if fail[idx[l]] {
                        x.status = RayStatus::AimFailed { stop: st };
                    }
                    out.put(b * L + l, &x, p[l], k[l]);
                }
            };
            if par {
                (0..ng)
                    .into_par_iter()
                    .with_min_len(PAR_CHUNK / L)
                    .for_each(scoped(group));
            } else {
                (0..ng).for_each(group);
            }
        } else if let (Some(plan), Some((inp, surf, _))) = (&pol_plan, pol) {
            // 偏光あり: L 本ずつまとめて追う（電場も 1 本ずつと同じ演算）
            let np = inp.len();
            let group = |b: usize| {
                let idx: [usize; L] = std::array::from_fn(|l| (b * L + l).min(n - 1));
                let start = idx.map(start_of);
                let (p, k) = (start.map(|s| s.0), start.map(|s| s.1));
                let mut e = Vec::with_capacity(L * np);
                for kk in &k {
                    e_start(inp, *kk, &mut e);
                }
                let mut pl = batch::PolLanes::<L>::new(e, np, surf);
                let mut sink = SoaLanes {
                    out: &ptrs,
                    base: b * L,
                };
                let mut r =
                    batch::trace_lanes_pol(plan, &p, &k, record.then_some(&mut sink), &mut pl);
                let m = (n - b * L).min(L);
                let mut good = [false; L];
                for l in 0..m {
                    if fail[idx[l]] {
                        r[l].status = RayStatus::AimFailed { stop: st };
                    }
                    put_phase(&mut pl.e[l * np..(l + 1) * np], r[l].opl);
                    out.put(b * L + l, &r[l], p[l], k[l]);
                    good[l] = r[l].status.is_ok() && r[l].vig == 0;
                }
                (good, pl.e, pl.err, m)
            };
            let groups: Vec<_> = if par {
                (0..ng)
                    .into_par_iter()
                    .with_min_len(PAR_CHUNK / L)
                    .map(scoped(group))
                    .collect()
            } else {
                (0..ng).map(group).collect()
            };
            es.reserve(n * np);
            for (good, e, mut perr, m) in groups {
                for l in 0..m {
                    take_err(good[l], perr[l].take());
                }
                es.extend_from_slice(&e[..m * np]);
            }
        } else {
            let pes: Vec<_> = if par {
                (0..n)
                    .into_par_iter()
                    .with_min_len(PAR_CHUNK)
                    .map(scoped(one))
                    .collect()
            } else {
                (0..n).map(one).collect()
            };
            es.reserve(n * pol.map_or(0, |p| p.0.len()));
            for b in pes.into_iter().flatten() {
                let (good, e, perr) = *b;
                es.extend_from_slice(&e);
                take_err(good, perr);
            }
        }
        drop(ptrs);
        // 安全: 上のどの経路でも 0..n の全部の光線を out.put で書いた
        unsafe { dst.finish(n) };
        let mut parity = 1.0;
        for j in 1..=last {
            let d = &self.surfaces[j];
            if !d.ignore && d.is_mirror() {
                parity = -parity;
            }
        }
        (
            Done {
                parity,
                rec: rec_out,
                last,
                hx_hy: hxy.unwrap_or((0.0, 0.0)),
            },
            PolField::from_flat(pol.map_or(0, |p| p.0.len()), es),
            first_err,
        )
    }

    /// 視野の光線を瞳座標 (px, py) で波長 w（既定は主波長）、面 last（既定は像面）まで追跡する。
    /// rec は記録する面（[`RecSel`]、bool なら全部か無し）
    pub fn try_trace<'r>(
        &self,
        field: FieldSel,
        px: &[f64],
        py: &[f64],
        w: Option<usize>,
        last: Option<usize>,
        rec: impl Into<RecSel<'r>>,
    ) -> Result<Trace> {
        self.check_crystals()?;
        let w = w.unwrap_or(self.primary);
        let (hx, hy) = self.field_sel_norm(field, w)?;
        let rec = rec.into();
        if let (Some(f), None, RecSel::Off) = (&self.cache.focus, last, &rec) {
            return f.trace(self, field, (hx, hy), px, py, w);
        }
        let spec = self.object_spec(hx, hy)?;
        let vig = self.vig_factors(hx, hy)?;
        let l = Launch::Pupil {
            spec,
            signed: false,
            px,
            py,
            vig: Some(vig),
            hxy: Some((hx, hy)),
        };
        let mut t = self.trace_launch(&l, w, last, rec);
        t.hx_hy = (hx, hy);
        Ok(t)
    }
}
