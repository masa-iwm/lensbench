//! 光線エイミング（絞りへの狙い・実光線エイミング・主光線の継続法）

use super::*;
use crate::newton2;
use crate::tol::{AIM_FAIL, AIM_LINE_MIN, AIM_STEP, AIM_TOL};

impl System {
    /// 物体 spec から絞り面の局所座標 (x, y) を通る実光線を光線ごとに解く（[`Launch::Stop`]）。
    /// 未知数は、有限物体なら物点からの方向の傾き (l/n, m/n)（面 1 の変換の前の座標系）、無限遠物体なら
    /// 入射瞳面での位置。入射瞳が無限遠に近い系でも未知数の大きさが変わらない
    pub(super) fn aim_stop(
        &self,
        spec: ObjSpec,
        x: &[f64],
        y: &[f64],
        w: usize,
    ) -> (Vec<(V3, V3)>, Vec<bool>) {
        let s = self.stop();
        let fr = self.frames();
        let plan = batch::Plan::new(self, fr, w, s, false);
        let r0 = fr.obj.r;
        let geom = StartGeom::new(self, spec);
        let start = |v: [f64; 2]| -> (V3, V3) {
            match spec {
                ObjSpec::Dir(_) => geom.ray(v[0], v[1]),
                ObjSpec::Point(pt) => {
                    let f0 = fr.surf[0].expect("有限物体の座標系");
                    let p = add(f0.o, mv(&f0.r, [pt[0], pt[1], 0.0]));
                    (p, normalize(mv(&r0, [v[0], v[1], 1.0])))
                }
            }
        };
        let at_stop = |v: [f64; 2]| -> [f64; 2] {
            let (p, k) = start(v);
            let r = batch::trace_one_plain(&plan, p, k);
            if r.status.surf().is_some() {
                return [f64::NAN; 2];
            }
            [r.xyz[0], r.xyz[1]]
        };
        let h = match spec {
            ObjSpec::Point(_) => AIM_STEP,
            ObjSpec::Dir(_) => AIM_STEP * (self.paraxial().epd / 2.0).max(1.0),
        };
        let jac = |v: [f64; 2], q: [f64; 2]| -> [f64; 4] {
            let qx = at_stop([v[0] + h, v[1]]);
            let qy = at_stop([v[0], v[1] + h]);
            [
                (qx[0] - q[0]) / h,
                (qy[0] - q[0]) / h,
                (qx[1] - q[1]) / h,
                (qy[1] - q[1]) / h,
            ]
        };
        let nw = newton2::Newton2 {
            tol: 0.0,
            max_it: 40,
            diff: newton2::Diff::fwd(h),
            line: newton2::Line::Full,
        };
        let lim = |t: [f64; 2]| AIM_TOL * t[0].abs().max(t[1].abs()).max(1.0);
        // 絞りの中心を通る光線を解き、その点のヤコビアンで光線ごとの初期値を作る
        let v0 = match spec {
            ObjSpec::Dir(_) => [0.0, 0.0],
            ObjSpec::Point(_) => {
                let (_, k) = self.start_ray(spec, 0.0, 0.0);
                let kk = mtv(&r0, k);
                [kk[0] / kk[2], kk[1] / kk[2]]
            }
        };
        let [c] = nw.solve_lanes(
            |v: &[[f64; 2]; 1]| [at_stop(v[0])],
            [v0],
            [[0.0; 2]],
            [lim([0.0; 2])],
        );
        let vc = c.v;
        let jc = jac(vc, at_stop(vc));
        let guess = |t: [f64; 2]| -> [f64; 2] {
            let d = solve2(jc, t);
            if d[0].is_finite() && d[1].is_finite() {
                [vc[0] + d[0], vc[1] + d[1]]
            } else {
                vc
            }
        };
        let result = |t: [f64; 2], s: &newton2::Sol| -> ((V3, V3), bool) {
            let fail = !(t[0].is_finite() && t[1].is_finite())
                || !(s.err() <= AIM_FAIL * t[0].abs().max(t[1].abs()).max(1.0));
            (start(s.v), fail)
        };
        let one = |i: usize| -> ((V3, V3), bool) {
            let tgt = [x[i], y[i]];
            let [s] = nw.solve_lanes(
                |v: &[[f64; 2]; 1]| [at_stop(v[0])],
                [guess(tgt)],
                [tgt],
                [lim(tgt)],
            );
            result(tgt, &s)
        };
        let n = x.len().min(y.len());
        let par = n >= PAR_MIN_RAYS && !crate::par::sequential();
        // L 本ずつまとめて同じ反復をする（光線ごとの手順・結果は上の one と同じビット。収束・中断した光線は
        // そこで止め、残りの光線の反復だけを続ける）
        const L: usize = batch::L;
        if !batch::use_for(n) {
            let out: Vec<_> = if par {
                (0..n)
                    .into_par_iter()
                    .with_min_len(PAR_CHUNK)
                    .map(scoped(one))
                    .collect()
            } else {
                (0..n).map(one).collect()
            };
            return out.into_iter().unzip();
        }
        let at_stop_l = |v: &[[f64; 2]; L]| -> [[f64; 2]; L] {
            let st = v.map(start);
            let r =
                batch::trace_lanes::<batch::NoRec>(&plan, &st.map(|s| s.0), &st.map(|s| s.1), None);
            std::array::from_fn(|l| {
                if r[l].status.surf().is_some() {
                    [f64::NAN; 2]
                } else {
                    [r[l].xyz[0], r[l].xyz[1]]
                }
            })
        };
        let grp = |b: usize| -> [((V3, V3), bool); L] {
            // 端の組の足りない分は最後の光線を繰り返す（結果は捨てる）
            let idx: [usize; L] = std::array::from_fn(|l| (b * L + l).min(n - 1));
            let tgt: [[f64; 2]; L] = idx.map(|i| [x[i], y[i]]);
            let s = nw.solve_lanes(at_stop_l, tgt.map(guess), tgt, tgt.map(lim));
            std::array::from_fn(|l| result(tgt[l], &s[l]))
        };
        let nb = n.div_ceil(L);
        let g: Vec<_> = if par {
            (0..nb)
                .into_par_iter()
                .with_min_len(PAR_CHUNK / L)
                .map(scoped(grp))
                .collect()
        } else {
            (0..nb).map(grp).collect()
        };
        let mut out = g.into_iter().flatten().collect::<Vec<_>>();
        out.truncate(n);
        out.into_iter().unzip()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn aim(
        &self,
        spec: ObjSpec,
        px: &[f64],
        py: &[f64],
        ex: &mut [f64],
        ey: &mut [f64],
        fail: &mut [bool],
        w: usize,
    ) {
        let (rx, ry) = self.stop_radius();
        let s = self.stop();
        let fr = self.frames();
        let plan = batch::Plan::new(self, fr, w, s, false);
        let tol = AIM_TOL;
        // 絞りの半径は符号つき（像が反転する向きなら負）なので大きさで
        let rmax = rx.abs().max(ry.abs()).max(1.0);
        let fail_tol = AIM_FAIL * rmax;
        let scale_ = ex
            .iter()
            .chain(ey.iter())
            .fold(1.0f64, |m, v| m.max(v.abs()));
        let h = AIM_STEP * scale_;
        let lim = tol * rmax;
        let geom = StartGeom::new(self, spec);
        let at_stop = |v: [f64; 2]| -> [f64; 2] {
            let (p, k) = geom.ray(v[0], v[1]);
            let r = batch::trace_one_plain(&plan, p, k);
            [r.xyz[0], r.xyz[1]]
        };
        // 1 本ずつの反復
        let one = |mut a: AimLane| -> AimLane {
            a.q = at_stop(a.v);
            while let Some(j) = a.check(lim) {
                let v = a.v;
                let jac = j.then(|| (at_stop([v[0] + h, v[1]]), at_stop([v[0], v[1] + h])));
                a.update(h, jac);
                a.q = at_stop(a.v);
            }
            a
        };
        // L 本ずつまとめて同じ反復をする（光線ごとの手順・結果は上の one と同じビット。
        // 収束した光線はそこで止め、残りの光線の反復だけを続ける）
        const L: usize = batch::L;
        let at_stop_l = |v: &[[f64; 2]; L]| -> [[f64; 2]; L] {
            let st = v.map(|v| geom.ray(v[0], v[1]));
            let r =
                batch::trace_lanes::<batch::NoRec>(&plan, &st.map(|s| s.0), &st.map(|s| s.1), None);
            std::array::from_fn(|l| [r[l].xyz[0], r[l].xyz[1]])
        };
        let group = |a: &mut [AimLane; L]| {
            let q = at_stop_l(&std::array::from_fn(|l| a[l].v));
            for l in 0..L {
                a[l].q = q[l];
            }
            let mut act = [true; L];
            loop {
                let mut need = [false; L];
                for l in 0..L {
                    if act[l] {
                        match a[l].check(lim) {
                            None => act[l] = false,
                            Some(j) => need[l] = j,
                        }
                    }
                }
                if !act.iter().any(|&x| x) {
                    break;
                }
                let jac = if need.iter().any(|&x| x) {
                    let qx = at_stop_l(&std::array::from_fn(|l| [a[l].v[0] + h, a[l].v[1]]));
                    let qy = at_stop_l(&std::array::from_fn(|l| [a[l].v[0], a[l].v[1] + h]));
                    Some((qx, qy))
                } else {
                    None
                };
                for l in 0..L {
                    if act[l] {
                        a[l].update(h, jac.filter(|_| need[l]).map(|(qx, qy)| (qx[l], qy[l])));
                    }
                }
                let qn = at_stop_l(&std::array::from_fn(|l| a[l].v));
                for l in 0..L {
                    if act[l] {
                        a[l].q = qn[l];
                    }
                }
            }
        };
        // 光線は互いに独立なので、本数が多ければ並列に（組は入力の順に固定、スレッド数によらず同じビット）
        // 光線 i の初期状態 init(i) から解き、(v, 解けなかったか) を返す
        let res = |a: &AimLane| {
            (
                a.v,
                a.failed(fail_tol),
                !(a.q[0].is_finite() && a.q[1].is_finite()),
            )
        };
        let solve = |n: usize,
                     init: &(dyn Fn(usize) -> AimLane + Sync)|
         -> Vec<([f64; 2], bool, bool)> {
            let par = n >= PAR_MIN_RAYS && !crate::par::sequential();
            if plan.is_empty() || !batch::use_for(n) {
                let f = |i: usize| res(&one(init(i)));
                if par {
                    (0..n)
                        .into_par_iter()
                        .with_min_len(PAR_CHUNK)
                        .map(scoped(f))
                        .collect()
                } else {
                    (0..n).map(f).collect()
                }
            } else {
                // 端の組の足りない分は最後の光線を繰り返す（結果は捨てる）
                let nb = n.div_ceil(L);
                let grp = |b: usize| {
                    let mut a: [AimLane; L] = std::array::from_fn(|l| init((b * L + l).min(n - 1)));
                    group(&mut a);
                    std::array::from_fn::<_, L, _>(|l| res(&a[l]))
                };
                let g: Vec<_> = if par {
                    (0..nb)
                        .into_par_iter()
                        .with_min_len(PAR_CHUNK / L)
                        .map(scoped(grp))
                        .collect()
                } else {
                    (0..nb).map(grp).collect()
                };
                let mut out = g.into_iter().flatten().collect::<Vec<_>>();
                out.truncate(n);
                out
            }
        };
        let n = px.len();
        let tgt = |q: [f64; 2]| [q[0] * rx, q[1] * ry];
        // 近軸の入射瞳から始めると光線が絞りまで届かない系（超広角など）: 主光線を視野の継続法で解き、
        // 初期値をその入射瞳上の点だけずらす（届く系はずらさない。結果は今までと同じビット）
        // 90° を超える視野（物体側の後ろから来る光）は、届いても光線を逆向きに進めた別の解のことがあるので必ず継続法で
        let behind = matches!(spec, ObjSpec::Dir(d) if d[2] < 0.0);
        let shift = if !behind && at_stop([0.0, 0.0]).iter().all(|v| v.is_finite()) {
            None
        } else {
            self.aim_chief_continuation(spec, w)
        };
        if let Some(c) = shift {
            ex.iter_mut().for_each(|v| *v += c[0]);
            ey.iter_mut().for_each(|v| *v += c[1]);
        }
        let sh = shift.unwrap_or([0.0, 0.0]);
        // 本数が多いときは、格子の光線を先に解いて q → v を多項式で当てはめ、初期値と固定の逆ヤコビアンに使う
        let fit = if n >= AIM_FIT_MIN {
            AimFit::grid(px, py).and_then(|(c, hw, pts)| {
                let epd = self.paraxial().epd;
                let init = |i: usize| {
                    let p = pts[i];
                    let v = [p[0] * epd / 2.0 + sh[0], p[1] * epd / 2.0 + sh[1]];
                    AimLane::new(tgt(p), v, None, v)
                };
                let v: Vec<_> = solve(pts.len(), &init)
                    .into_iter()
                    .map(|(v, f, nan)| (!f && !nan).then_some(v))
                    .collect();
                AimFit::new(c, hw, &pts, &v)
            })
        } else {
            None
        };
        let init = |i: usize| {
            let q = [px[i], py[i]];
            let v0 = [ex[i], ey[i]];
            if let Some(f) = &fit {
                let (v, d) = f.eval(q);
                let m = [d[0] / rx, d[1] / ry, d[2] / rx, d[3] / ry];
                if v.iter().chain(&m).all(|x| x.is_finite()) {
                    return AimLane::new(tgt(q), v, Some(m), v0);
                }
            }
            AimLane::new(tgt(q), v0, None, v0)
        };
        let mut out = solve(n, &init);
        // 解けなかった光線（絞りまで届かなかったものも）は、解けた光線のうち瞳座標の最も近いものから、目標を少しずつ
        // 動かす継続法で解き直す（近軸の初期値の光線が頂点面に平行になる折り曲げの系など）。継続法は 1 本で数百〜数千回の
        // 追跡になるので、解けた領域の縁に隣り合う光線（最も近い解けた光線が、どちらかの光線の最近接の光線の間隔の 1.5 倍
        // 以内）だけを解き直し、解けた光線を足して縁を広げる。縁に届かない光線（瞳の外の届かない領域の奥）は解き直さずに
        // 失敗とする（以前は全部を解き直し、届かない光線 1 本に約 7 ms かかった。見直し KRN-07）
        let mut bad: Vec<usize> = (0..n).filter(|&i| out[i].1 || out[i].2).collect();
        let mut good: Vec<usize> = (0..n).filter(|&i| !out[i].1 && !out[i].2).collect();
        if !bad.is_empty() && !good.is_empty() {
            let pts: Vec<[f64; 2]> = (0..n).map(|i| [px[i], py[i]]).collect();
            let all = NearGrid::new(&pts, &(0..n).collect::<Vec<_>>());
            // 光線ごとの最近接の（別の）光線までの距離
            let gap: Vec<f64> = bad
                .iter()
                .map(|&i| {
                    all.nearest(&pts, pts[i], Some(i))
                        .map_or(f64::INFINITY, |(_, d)| d)
                })
                .collect();
            let mut gap_of = vec![0.0; n];
            for (&i, &g) in bad.iter().zip(&gap) {
                gap_of[i] = g;
            }
            let gap_good = |j: usize| all.nearest(&pts, pts[j], Some(j)).map_or(0.0, |x| x.1);
            loop {
                let near = NearGrid::new(&pts, &good);
                let mut front = Vec::new();
                let mut rest = Vec::new();
                for &i in &bad {
                    match near.nearest(&pts, pts[i], None) {
                        Some((j, d)) if d <= 1.5 * gap_of[i].max(gap_good(j)) => front.push((i, j)),
                        _ => rest.push(i),
                    }
                }
                if front.is_empty() {
                    break;
                }
                let redo = |&(i, j): &(usize, usize)| {
                    self.aim_follow(spec, w, out[j].0, tgt([px[j], py[j]]), tgt([px[i], py[i]]))
                };
                // 光線ごとに独立なので結果はスレッド数によらず同じビット
                let re: Vec<Option<[f64; 2]>> = if front.len() > 1 && !crate::par::sequential() {
                    front.par_iter().with_min_len(1).map(scoped(redo)).collect()
                } else {
                    front.iter().map(redo).collect()
                };
                let mut solved = 0;
                for (&(i, _), r) in front.iter().zip(re) {
                    if let Some(v) = r {
                        out[i] = (v, false, false);
                        good.push(i);
                        solved += 1;
                    }
                }
                if solved == 0 {
                    break;
                }
                // 失敗した縁の光線はもう試さない（次の縁は新しく解けた光線の隣）
                bad = rest;
            }
        }
        for (i, (v, f, _)) in out.into_iter().enumerate() {
            fail[i] = f;
            (ex[i], ey[i]) = if f { (0.0, 0.0) } else { (v[0], v[1]) };
        }
    }

    /// 主光線（絞りの中心を通る光線）の入射瞳の座標を、物体（視野の角度・物点）を軸上から少しずつ動かす継続法で解く。
    /// 近軸の入射瞳から始めると絞りに届かない超広角の系の光線エイミングの初期値に使う。解けなければ None
    pub(super) fn aim_chief_continuation(&self, spec: ObjSpec, w: usize) -> Option<[f64; 2]> {
        let s = self.stop();
        let fr = self.frames();
        let plan = batch::Plan::new(self, fr, w, s, false);
        let (o1, r0) = (fr.obj.o, fr.obj.r);
        // 無限遠物体の未知数は、面 1 の原点を通り光線に垂直な面の上の位置（入射瞳の面の座標は視野 90° で決まらなく
        // なるので、90° を超える視野へもたどれるように）。光線はその面から十分前に戻した点から出し、各面まで光線の
        // 向きに進む（逆向きに進んで交わる解を採らない）ものだけを認める
        let n_path = match spec {
            ObjSpec::Dir(d) if d[0] != 0.0 || d[1] != 0.0 => {
                let ph = d[1].atan2(d[0]);
                [-ph.sin(), ph.cos(), 0.0]
            }
            _ => [1.0, 0.0, 0.0],
        };
        let basis = |d: V3| -> (V3, V3) {
            let u1 = normalize(sub(n_path, scale(d, dot(n_path, d))));
            (u1, cross(d, u1))
        };
        let back = 1e3
            + (1..self.image())
                .map(|j| self.surfaces[j].t.abs())
                .filter(|t| t.is_finite())
                .sum::<f64>();
        let nrec = (1..=s).filter(|&j| !self.surfaces[j].ignore).count();
        let at = |sp: ObjSpec, v: [f64; 2]| -> [f64; 2] {
            let (p, k) = match sp {
                ObjSpec::Dir(d) => {
                    let (u1, u2) = basis(d);
                    let q = add(scale(u1, v[0]), scale(u2, v[1]));
                    (add(o1, mv(&r0, sub(q, scale(d, back)))), mv(&r0, d))
                }
                ObjSpec::Point(_) => self.start_ray(sp, v[0], v[1]),
            };
            let mut rec = vec![trace::RecEntry::NAN; nrec];
            let r = batch::trace_one(&plan, p, k, Some(&mut rec[..]));
            if r.status.surf().is_some() {
                return [f64::NAN; 2];
            }
            // 光路長が面ごとに減らない（光線の向きに進む）
            let mut prev: f64 = 0.0;
            for e in &rec {
                if e.opl < prev - 1e-9 * prev.abs().max(1.0) {
                    return [f64::NAN; 2];
                }
                prev = e.opl;
            }
            [r.xyz[0], r.xyz[1]]
        };
        let (rx, ry) = self.stop_radius();
        let nw = newton2::Newton2 {
            tol: AIM_TOL * rx.abs().max(ry.abs()).max(1.0),
            max_it: 60,
            diff: newton2::Diff::fwd(AIM_STEP * (self.paraxial().epd / 2.0).max(1.0)),
            // 光線が面から外れたら刻みを半分に
            line: newton2::Line::Finite { min: AIM_LINE_MIN },
        };
        let newton = |sp: ObjSpec, v: [f64; 2]| -> Option<[f64; 2]> {
            let s = nw.solve(|v| at(sp, v), v, [0.0; 2]);
            s.ok().then_some(s.v)
        };
        // 軸上（主光線は入射瞳の中心）から物体を動かす
        let step = |t: f64| -> ObjSpec {
            match spec {
                ObjSpec::Dir(d) => {
                    let th = d[2].clamp(-1.0, 1.0).acos() * t;
                    let ph = d[1].atan2(d[0]);
                    ObjSpec::Dir([th.sin() * ph.cos(), th.sin() * ph.sin(), th.cos()])
                }
                ObjSpec::Point(p) => ObjSpec::Point([p[0] * t, p[1] * t]),
            }
        };
        // 走査の範囲: 面 1 から絞りまでの面の半径の最大の 2 倍（なければ全長）
        let span = (1..=s)
            .map(|j| self.surfaces[j].semi)
            .filter(|v| v.is_finite())
            .fold(0.0f64, f64::max)
            .max(1.0)
            * 2.0;
        let scan = |sp: ObjSpec| -> Option<[f64; 2]> {
            let m = 400;
            let mut best: Option<(f64, [f64; 2])> = None;
            for i in 0..=m {
                let v = [0.0, -span + 2.0 * span * i as f64 / m as f64];
                let q = at(sp, v);
                let e = q[0].hypot(q[1]);
                if e.is_finite() && best.is_none_or(|b| e < b.0) {
                    best = Some((e, v));
                }
            }
            newton(sp, best?.1)
        };
        let mut v = [0.0, 0.0];
        let mut t: f64 = 0.0;
        let mut dt: f64 = 1.0 / 16.0;
        while t < 1.0 {
            let tn = (t + dt).min(1.0);
            match newton(step(tn), v) {
                Some(vn) => {
                    v = vn;
                    t = tn;
                    dt = (dt * 1.5).min(0.25);
                }
                None => {
                    dt *= 0.5;
                    if dt < 1e-4 {
                        // 解の枝が途切れた（視野 90° を越えると、光線が面 1 に当たる所が飛ぶ）: 少し先の視野で、
                        // 継続法の面の中の直線（v1 = 0）を走査して初期値を探し直す
                        let tn = (t + 1.0 / 64.0).min(1.0);
                        v = scan(step(tn))?;
                        t = tn;
                        dt = 1.0 / 64.0;
                    }
                }
            }
        }
        match spec {
            // 入射瞳の面（物体の座標系の z = 近軸の入射瞳の位置）での座標に直す
            ObjSpec::Dir(d) => {
                let (u1, u2) = basis(d);
                let q = add(scale(u1, v[0]), scale(u2, v[1]));
                let zp = self.paraxial().enpp.min(ENPP_MAX);
                let t = (zp - q[2]) / d[2];
                let e = [q[0] + t * d[0], q[1] + t * d[1]];
                (e[0].is_finite() && e[1].is_finite()).then_some(e)
            }
            ObjSpec::Point(_) => Some(v),
        }
    }

    /// 入射瞳の座標 v0 で絞りの点 t0 を通る光線から、目標を t1 まで少しずつ動かして（継続法、各点はニュートン法）
    /// t1 を通る光線の入射瞳の座標を解く。解けなければ None
    pub(super) fn aim_follow(
        &self,
        spec: ObjSpec,
        w: usize,
        v0: [f64; 2],
        t0: [f64; 2],
        t1: [f64; 2],
    ) -> Option<[f64; 2]> {
        let s = self.stop();
        let fr = self.frames();
        let plan = batch::Plan::new(self, fr, w, s, false);
        let at = |v: [f64; 2], tg: [f64; 2]| -> [f64; 2] {
            let (p, k) = self.start_ray(spec, v[0], v[1]);
            let r = batch::trace_one_plain(&plan, p, k);
            if r.status.surf().is_some() {
                return [f64::NAN; 2];
            }
            [r.xyz[0] - tg[0], r.xyz[1] - tg[1]]
        };
        let (rx, ry) = self.stop_radius();
        let nw = newton2::Newton2 {
            tol: AIM_TOL * rx.abs().max(ry.abs()).max(1.0),
            max_it: 60,
            diff: newton2::Diff::fwd(AIM_STEP * (self.paraxial().epd / 2.0).abs().max(1.0)),
            line: newton2::Line::Finite { min: AIM_LINE_MIN },
        };
        let newton = |v: [f64; 2], tg: [f64; 2]| -> Option<[f64; 2]> {
            let s = nw.solve(|v| at(v, tg), v, [0.0; 2]);
            s.ok().then_some(s.v)
        };
        let mut v = v0;
        let mut t: f64 = 0.0;
        let mut dt: f64 = 1.0 / 8.0;
        while t < 1.0 {
            let tn = (t + dt).min(1.0);
            let tg = [t0[0] + tn * (t1[0] - t0[0]), t0[1] + tn * (t1[1] - t0[1])];
            match newton(v, tg) {
                Some(vn) => {
                    v = vn;
                    t = tn;
                    dt = (dt * 1.5).min(0.25);
                }
                None => {
                    dt *= 0.5;
                    if dt < 1e-4 {
                        return None;
                    }
                }
            }
        }
        Some(v)
    }
}
