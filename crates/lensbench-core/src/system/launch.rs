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
                let d0 = &s.surfaces[0];
                let z0 = if crate::freeform::is_freeform(d0.typ) {
                    crate::freeform::sag_xy(d0, pt[0], pt[1]).0
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
    ) -> (Trace, Vec<Vec<CV3>>, Option<Error>) {
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
        pols: &[[C64; 2]],
        coat: &Coatings,
        phase: bool,
        rec: RecSel<'r>,
    ) -> Result<PolTrace> {
        let ps = self.pol_surfaces(w, coat)?;
        let (t, e, err) = self.trace_launch_inner(l, w, None, rec, Some((pols, &ps, phase)));
        if let Some(e) = err {
            return Err(e);
        }
        Ok(PolTrace { trace: t, e })
    }

    /// Fresnel・Cylinder Fresnel の面（面 last まで）があれば、同じ物体・視野・波長の主光線（瞳座標 0、絞りの中心）の
    /// 交点をその面の光路長の基準点にした座標系（[`Frames::fres_ref`]）。OpticStudio と同じ約束。
    /// 光線を直接与えたとき（主光線がない）と主光線が通らないときは頂点
    pub(super) fn fresnel_frames(&self, l: &Launch, w: usize, last: usize) -> Option<Frames> {
        let is_f = |j: usize| {
            let d = &self.surfaces[j];
            !d.ignore && matches!(d.typ, SurfType::Fresnel | SurfType::CylFresnel)
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
        let mut f = fr.clone();
        f.fres_ref = vec![[0.0; 2]; self.surfaces.len()];
        if fail[0] {
            return Some(f);
        }
        let nrec = (1..=last).filter(|&j| !self.surfaces[j].ignore).count();
        let mut rec = vec![trace::RecEntry::NAN; nrec];
        let (p, k) = st[0];
        let r = trace::trace_ray(
            self,
            fr,
            w,
            p,
            k,
            0.0,
            1,
            last,
            self.floating_clip,
            Some(&mut rec),
        );
        if r.err == 0 {
            for e in &rec {
                if is_f(e.surf) {
                    f.fres_ref[e.surf] = [e.xyz[0], e.xyz[1]];
                }
            }
        }
        Some(f)
    }

    #[allow(clippy::type_complexity)]
    pub(super) fn trace_launch_inner(
        &self,
        l: &Launch,
        w: usize,
        last: Option<usize>,
        rec: RecSel<'_>,
        pol: Option<(&[[C64; 2]], &[PolSurf], bool)>,
    ) -> (Trace, Vec<Vec<CV3>>, Option<Error>) {
        let (starts, fail) = self.launch_starts(l, w);
        let n = starts.len();
        let (spec, hxy) = match *l {
            Launch::Pupil { spec, hxy, .. } => (Some(spec), hxy),
            Launch::Stop { spec, .. } => (Some(spec), None),
            Launch::Rays { .. } => (None, None),
        };
        let last = last.unwrap_or(self.image());
        let fres_fr = self.fresnel_frames(l, w, last);
        let fr = fres_fr.as_ref().unwrap_or(self.frames());
        let clip = self.floating_clip;
        let st = self.stop();
        let kw = 2.0 * std::f64::consts::PI / (self.wavelengths[w] * 1e-3);
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
                if d.mirror {
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
        let one = |i: usize| {
            let (p, k) = starts.get(i);
            let mut row = SoaRow { out: &ptrs, i };
            let rec = if record { Some(&mut row) } else { None };
            let mut pr = pol.map(|(inp, surf, _)| {
                let mut e = Vec::with_capacity(inp.len());
                e_start(inp, k, &mut e);
                PolRay {
                    e,
                    surf,
                    err: None,
                    cry: None,
                }
            });
            let mut r = match pr.as_mut() {
                Some(q) => trace::trace_core(self, fr, w, p, k, 0.0, 1, last, clip, rec, Some(q)),
                None => trace::trace_core(self, fr, w, p, k, 0.0, 1, last, clip, rec, None),
            };
            if fail[i] {
                r.err = st;
            }
            // 偏光なしの追跡で結果を大きくしないよう箱に入れる
            let pe = pr.map(|mut pr| {
                put_phase(&mut pr.e, r.opl);
                Box::new((pr.e, pr.err))
            });
            (r, p, k, pe)
        };
        // 光線は互いに独立なので、本数が多いときは並列に追う（結果の順序は入力のまま、
        // スレッド数によらず同じビット）。with_threads(Some(1), ..) の中では逐次
        let par = n >= PAR_MIN_RAYS && !crate::par::sequential();
        // L 本ずつまとめて追う（1 本ずつと同じビット。組は入力の順に固定）。偏光ありは別の組で（下）
        let plan = if batch::use_for(n) {
            Some(batch::Plan::new(self, fr, w, 1, last, clip)).filter(|p| match pol {
                None => !p.is_empty(),
                Some((_, surf, _)) => p.supports_pol(surf),
            })
        } else {
            None
        };
        let (plan, pol_plan) = match pol {
            None => (plan, None),
            Some(_) => (None, plan),
        };
        let mut rays: Vec<RayOut>;
        let mut p0: Vec<V3>;
        let mut k0: Vec<V3>;
        let mut es = Vec::new();
        let mut first_err = None;
        if let Some(plan) = &plan {
            // 結果の配列へ組ごとに直接書く（中間の配列を作らない）
            // 結果の配列は初期化せずに確保し、組ごとに（並列に）全要素を書いてから長さを決める
            // （大きな配列の初期化を 1 スレッドでしないため）
            rays = Vec::with_capacity(n);
            p0 = Vec::with_capacity(n);
            k0 = Vec::with_capacity(n);
            const L: usize = batch::L;
            type Out<'a, T> = &'a mut [MaybeUninit<T>];
            let group = |(b, ((ro, po), ko)): (usize, ((Out<RayOut>, Out<V3>), Out<V3>))| {
                // 端の組の足りない分は最後の光線を繰り返す（結果は捨てる）
                let idx: [usize; L] = std::array::from_fn(|l| (b * L + l).min(n - 1));
                let start = idx.map(|i| starts.get(i));
                let (p, k) = (start.map(|s| s.0), start.map(|s| s.1));
                let mut sink = SoaLanes {
                    out: &ptrs,
                    base: b * L,
                };
                let r = batch::trace_lanes(plan, &p, &k, record.then_some(&mut sink));
                for l in 0..ro.len() {
                    let mut x = r[l];
                    if fail[idx[l]] {
                        x.err = st;
                    }
                    ro[l].write(x);
                    po[l].write(p[l]);
                    ko[l].write(k[l]);
                }
            };
            let (ru, pu, ku) = (
                &mut rays.spare_capacity_mut()[..n],
                &mut p0.spare_capacity_mut()[..n],
                &mut k0.spare_capacity_mut()[..n],
            );
            if par {
                ru.par_chunks_mut(L)
                    .zip(pu.par_chunks_mut(L))
                    .zip(ku.par_chunks_mut(L))
                    .enumerate()
                    .with_min_len(PAR_CHUNK / L)
                    .for_each(scoped(group));
            } else {
                ru.chunks_mut(L)
                    .zip(pu.chunks_mut(L))
                    .zip(ku.chunks_mut(L))
                    .enumerate()
                    .for_each(group);
            }
            // 安全: 上で 0..n の全要素を書いた（組 b は b·L から L 本、端の組は残りの本数を書く）
            unsafe {
                rays.set_len(n);
                p0.set_len(n);
                k0.set_len(n);
            }
            drop(ptrs);
        } else {
            rays = Vec::with_capacity(n);
            p0 = Vec::with_capacity(n);
            k0 = Vec::with_capacity(n);
            // 束を止めるのは、最後まで通った（けられず、エイミングにも失敗していない）光線のエラーだけ。
            // ほかの光線は電場が NaN になるだけ
            let mut take_err = |r: &RayOut, perr: Option<Error>| {
                if first_err.is_none() && r.err == 0 && r.vig == 0 {
                    first_err = perr;
                }
            };
            if let (Some(plan), Some((inp, surf, _))) = (&pol_plan, pol) {
                // 偏光あり: L 本ずつまとめて追う（電場も 1 本ずつと同じ演算）
                const L: usize = batch::L;
                let np = inp.len();
                let group = |b: usize| {
                    let idx: [usize; L] = std::array::from_fn(|l| (b * L + l).min(n - 1));
                    let start = idx.map(|i| starts.get(i));
                    let (p, k) = (start.map(|s| s.0), start.map(|s| s.1));
                    let mut e = Vec::with_capacity(L * np);
                    for kk in &k {
                        e_start(inp, *kk, &mut e);
                    }
                    let mut pl = batch::PolLanes {
                        e,
                        np,
                        surf,
                        err: std::array::from_fn(|_| None),
                    };
                    let mut sink = SoaLanes {
                        out: &ptrs,
                        base: b * L,
                    };
                    let mut r =
                        batch::trace_lanes_pol(plan, &p, &k, record.then_some(&mut sink), &mut pl);
                    let m = (n - b * L).min(L);
                    for l in 0..m {
                        if fail[idx[l]] {
                            r[l].err = st;
                        }
                        put_phase(&mut pl.e[l * np..(l + 1) * np], r[l].opl);
                    }
                    (r, p, k, pl.e, pl.err, m)
                };
                let ng = n.div_ceil(L);
                let groups: Vec<_> = if par {
                    (0..ng)
                        .into_par_iter()
                        .with_min_len(PAR_CHUNK / L)
                        .map(scoped(group))
                        .collect()
                } else {
                    (0..ng).map(group).collect()
                };
                drop(ptrs);
                es.reserve(n);
                for (r, p, k, e, mut perr, m) in groups {
                    for l in 0..m {
                        take_err(&r[l], perr[l].take());
                        rays.push(r[l]);
                        p0.push(p[l]);
                        k0.push(k[l]);
                        es.push(e[l * np..(l + 1) * np].to_vec());
                    }
                }
            } else {
                let out: Vec<_> = if par {
                    (0..n)
                        .into_par_iter()
                        .with_min_len(PAR_CHUNK)
                        .map(scoped(one))
                        .collect()
                } else {
                    (0..n).map(one).collect()
                };
                drop(ptrs);
                for (r, p, k, pe) in out {
                    if let Some(b) = pe {
                        let (e, perr) = *b;
                        es.push(e);
                        take_err(&r, perr);
                    }
                    rays.push(r);
                    p0.push(p);
                    k0.push(k);
                }
            }
        }
        let mut parity = 1.0;
        for j in 1..=last {
            let d = &self.surfaces[j];
            if !d.ignore && d.mirror {
                parity = -parity;
            }
        }
        let opl_os = match spec {
            Some(ObjSpec::Dir(_)) => {
                // 面 1 が無視（IGNR）なら、その次の有効な面（大域座標の基準と同じ）
                let j1 = (1..fr.surf.len())
                    .find(|&j| fr.surf[j].is_some())
                    .unwrap_or(1);
                let ob = fr.surf[j1].unwrap();
                let ax = col(&ob.r, 2);
                let nb = self.index_before(j1, w);
                let f = |i: usize| rays[i].opl - dot(sub(ob.o, p0[i]), ax) / dot(k0[i], ax) * nb;
                if par {
                    (0..n)
                        .into_par_iter()
                        .with_min_len(PAR_CHUNK)
                        .map(scoped(f))
                        .collect()
                } else {
                    (0..n).map(f).collect()
                }
            }
            _ => rays.iter().map(|r| r.opl).collect(),
        };
        (
            Trace {
                rays,
                parity,
                p0,
                k0,
                opl_os,
                rec: rec_out,
                last,
                hx_hy: hxy.unwrap_or((0.0, 0.0)),
            },
            es,
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
