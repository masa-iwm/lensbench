//! 非順次の追跡の面の散乱と仕上げ（段 3）: 系の仕上げ（[`lensbench_core::scatter::Finish`]）を場面の面に当てる、
//! 光を通さない面の反射、散乱で枝を分ける

use super::*;

/// 面に付けた仕上げ（[`lensbench_core::scatter::Finish`]）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FaceFinish {
    /// 光を通さない面の反射率（書いてあれば）
    pub reflect: Option<f64>,
    pub scatter: Option<Scatter>,
    /// 縁に付けたものか
    pub edge: bool,
}

/// 場面の面ごとの仕上げ: 系（[`Scene::systems`] の順）の [`System::finishes`] を場面の面に当てる。同じ面に違う仕上げが
/// あれば誤り
pub fn face_finishes(sc: &Scene, systems: &[&System]) -> Result<Vec<Option<FaceFinish>>> {
    let mut out: Vec<Option<FaceFinish>> = vec![None; sc.faces.len()];
    for (si, sy) in systems.iter().enumerate() {
        let label = sc.systems.get(si).map_or("system", String::as_str);
        for f in &sy.finishes {
            f.check()?;
            let (faces, edge): (Vec<usize>, bool) = match f.at()? {
                FinishAt::Surface(j) => {
                    let Some(k) = sc.face_of(si, j) else {
                        invalid!("finish: surface {j} of {label} isn't an object of the non-sequential scene (a dummy surface, a coordinate break or out of range)");
                    };
                    (vec![k], false)
                }
                FinishAt::Edge(j) => {
                    let Some(k) = sc.face_of(si, j) else {
                        invalid!("finish: edge of surface {j} of {label}: the surface isn't an object of the non-sequential scene");
                    };
                    let o = &sc.objects[sc.faces[k].object];
                    let es: Vec<usize> = o
                        .faces
                        .iter()
                        .copied()
                        .filter(|&i| !matches!(sc.faces[i].shape, Shape::Optical { .. }))
                        .collect();
                    if o.kind != ObjectKind::Lens || es.is_empty() {
                        invalid!("finish: surface {j} of {label} isn't on a lens with an edge");
                    }
                    (es, true)
                }
                FinishAt::Mech(m) => {
                    let fs: Vec<usize> = sc
                        .objects
                        .iter()
                        .filter(|o| o.kind == ObjectKind::Mech && o.name == m)
                        .flat_map(|o| o.faces.iter().copied())
                        .collect();
                    if fs.is_empty() {
                        invalid!("finish: no mechanical part {m} in {label}");
                    }
                    (fs, false)
                }
            };
            let v = FaceFinish {
                reflect: f.reflect,
                scatter: f.scatter.clone(),
                edge,
            };
            for k in faces {
                match &out[k] {
                    Some(o) if *o != v => {
                        invalid!("finish: two different finishes on the same face ({label})")
                    }
                    _ => out[k] = Some(v.clone()),
                }
            }
        }
    }
    Ok(out)
}

/// 散乱の乱数と閾値（光源の光線 1 本ごと）
pub(super) struct ScatterCx {
    pub(super) rng: CounterRng,
    pub(super) id: u64,
    pub(super) keep: f64,
}

/// 枝 key の n 番目の子の枝の印
pub(super) fn branch_key(key: u64, n: u64) -> u64 {
    CounterRng::child(key, n)
}

/// 方向 m に垂直な 2 つの単位ベクトル
pub(super) fn basis(m: V3) -> (V3, V3) {
    let a = if m[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let u = normalize(cross(a, m));
    (u, cross(m, u))
}

impl Tracer<'_> {
    /// 光を通さない面（壁・絞りの板・塗った縁）: 反射率の分を鏡面の方向へ（散乱があれば散乱させて）反射し、残りは吸収
    #[allow(clippy::too_many_arguments)]
    pub(super) fn wall(
        &self,
        mut s: Live,
        fi: usize,
        ng: V3,
        pw: f64,
        sx: &mut ScatterCx,
        stack: &mut Vec<Live>,
        sink: &mut dyn FnMut(Branch, &mut Budget),
        bud: &mut Budget,
    ) {
        let r = self.faces[fi].wall;
        if r <= 0.0 {
            s.hits.push(Hit {
                face: fi,
                kind: EventKind::Absorb,
                p: s.p,
                power: 0.0,
            });
            finish(s, End::Absorbed, pw, sink, bud);
            return;
        }
        // 完全導体の反射の向きの電場（接する成分が逆向き）に √R をかける
        let en = s.e[0] * ng[0] + s.e[1] * ng[1] + s.e[2] * ng[2];
        let a = r.sqrt();
        let e: CV3 = [0, 1, 2].map(|i| (en * (2.0 * ng[i]) - s.e[i]) * a);
        let pr = power_of(&e);
        bud.absorbed += pw - pr;
        s.e = e;
        s.k = reflect(s.k, ng);
        let h = Hit {
            face: fi,
            kind: EventKind::Reflect,
            p: s.p,
            power: pr,
        };
        self.go_scatter(s, h, true, ng, sx, stack, sink, bud);
    }

    /// 鏡面の枝 s（方向・電場は鏡面の方向のもの、出来事 h はまだ足していない）を、面の散乱で分けて積む。reflect は反射
    /// した側の枝か。決まった分岐では鏡面の枝に (1 − TIS)、散乱の光線 [`NscOpts::scatter_rays`] 本に TIS を等しく分け、
    /// モンテカルロでは TIS の確率で散乱させる（パワーはそのまま）。散乱した光線の偏光は、偏光を失う仕上げでは方向に
    /// 垂直な面の中で向きが一様な直線偏光、保つ仕上げでは鏡面の方向の電場を散乱した方向に垂直な面へ射影して大きさを直したもの
    #[allow(clippy::too_many_arguments)]
    pub(super) fn go_scatter(
        &self,
        mut s: Live,
        h: Hit,
        reflect: bool,
        ng: V3,
        sx: &mut ScatterCx,
        stack: &mut Vec<Live>,
        sink: &mut dyn FnMut(Branch, &mut Budget),
        bud: &mut Budget,
    ) {
        let keep = sx.keep;
        let (bsdf, depol) = match &self.faces[h.face].scatter {
            Some((b, on, dp)) if scatter_side(*on, reflect) && h.power > 0.0 => (b, *dp),
            _ => return self.go_on(s, h, h.power, keep, stack, sink, bud),
        };
        // 出ていく側の法線と、接平面の基底
        let m = if dot(s.k, ng) >= 0.0 {
            ng
        } else {
            scale(ng, -1.0)
        };
        let (u, v) = basis(m);
        let b0 = [dot(s.k, u), dot(s.k, v)];
        let tis = bsdf.tis(b0[0].hypot(b0[1])).clamp(0.0, 1.0);
        if tis <= 0.0 {
            return self.go_on(s, h, h.power, keep, stack, sink, bud);
        }
        let kind = if reflect {
            EventKind::ScatterReflect
        } else {
            EventKind::Scatter
        };
        let depth = s.hits.len() as u32;
        let (rng, id) = (sx.rng, sx.id);
        // 散乱の方向と電場（パワー power）
        let draw = |key: u64, power: f64| -> (V3, CV3) {
            let mut st = rng.stream(id, key, depth);
            let mut uni = || st.next_uniform();
            let d = match bsdf.sample(b0, &mut uni) {
                Some(b) => {
                    let z = (1.0 - b[0] * b[0] - b[1] * b[1]).max(0.0).sqrt();
                    normalize([0, 1, 2].map(|i| b[0] * u[i] + b[1] * v[i] + z * m[i]))
                }
                None => s.k,
            };
            let a = std::f64::consts::TAU * uni();
            (d, scattered_field(&s.e, d, power, a, depol))
        };
        match self.opts.split {
            Split::MonteCarlo { .. } => {
                if rng.uniform_in(id, s.key, depth, 2) >= tis {
                    return self.go_on(s, h, h.power, keep, stack, sink, bud);
                }
                let p = power_of(&s.e);
                let (d, e) = draw(s.key, p);
                s.k = d;
                s.e = e;
                s.hits.push(Hit { kind, ..h });
                stack.push(s);
            }
            Split::Deterministic => {
                let n = self.opts.scatter_rays.max(1);
                let each = h.power * tis / n as f64;
                let mut given = 0.0;
                for i in 0..n {
                    let key = branch_key(s.key, 2 + i as u64);
                    let (d, e) = draw(key, each);
                    let c = Live {
                        p: s.p,
                        k: d,
                        e,
                        hits: s.hits.clone(),
                        skip: s.skip.clone(),
                        passes: s.passes,
                        key,
                    };
                    let pc = power_of(&c.e);
                    given += pc;
                    self.go_on(
                        c,
                        Hit {
                            kind,
                            power: pc,
                            ..h
                        },
                        pc,
                        keep,
                        stack,
                        sink,
                        bud,
                    );
                }
                if tis < 1.0 {
                    let a = (1.0 - tis).sqrt();
                    s.e = s.e.map(|z| z * a);
                    let ps = power_of(&s.e);
                    // 分けた後の丸めの差は吸収に入れる（収支を閉じる）
                    bud.absorbed += h.power - ps - given;
                    self.go_on(s, Hit { power: ps, ..h }, ps, keep, stack, sink, bud);
                } else {
                    bud.absorbed += h.power - given;
                }
            }
        }
    }
}

/// 方向 d へ散乱した光線の電場（パワー power）。偏光を保つとき（depol が false）は鏡面の方向の電場 e から d の成分を除いて
/// 大きさを直す（OpticStudio と同じ）。偏光を失うとき、または e が d とほぼ平行のときは、d に垂直な面の中の角度 a の直線偏光
pub(super) fn scattered_field(e: &CV3, d: V3, power: f64, a: f64, depol: bool) -> CV3 {
    let amp = power.sqrt();
    if !depol {
        let ed = e[0] * d[0] + e[1] * d[1] + e[2] * d[2];
        let q: CV3 = [0, 1, 2].map(|i| e[i] - ed * d[i]);
        let nq = power_of(&q).sqrt();
        if nq > 1e-9 * power_of(e).sqrt() {
            return q.map(|z| z * (amp / nq));
        }
    }
    let (p1, p2) = basis(d);
    let (sa, ca) = a.sin_cos();
    [0, 1, 2].map(|i| C64::new((ca * p1[i] + sa * p2[i]) * amp, 0.0))
}

fn scatter_side(on: ScatterSide, reflect: bool) -> bool {
    match on {
        ScatterSide::Both => true,
        ScatterSide::Reflect => reflect,
        ScatterSide::Transmit => !reflect,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dotc(e: &CV3, d: V3) -> C64 {
        e[0] * d[0] + e[1] * d[1] + e[2] * d[2]
    }

    #[test]
    fn kept_polarization_follows_the_specular_field() {
        // 鏡面の方向 +z、電場は x の直線偏光と、x・y の円偏光
        let lin: CV3 = [C64::new(1.0, 0.0), C64::new(0.0, 0.0), C64::new(0.0, 0.0)];
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let circ: CV3 = [C64::new(h, 0.0), C64::new(0.0, h), C64::new(0.0, 0.0)];
        let d = normalize([0.3, -0.2, 1.0]);
        for e in [lin, circ] {
            let q = scattered_field(&e, d, 0.25, 1.0, false);
            assert!((power_of(&q) - 0.25).abs() < 1e-15);
            assert!(dotc(&q, d).norm() < 1e-15);
            // 散乱した方向に垂直な面への射影と平行（偏光の形を保つ）
            let ed = dotc(&e, d);
            let p: CV3 = [0, 1, 2].map(|i| e[i] - ed * d[i]);
            let k = 0.5 / power_of(&p).sqrt();
            for i in 0..3 {
                assert!((q[i] - p[i] * k).norm() < 1e-15, "{q:?} {p:?}");
            }
        }
        // 散乱しない向き（d = +z）では電場はそのまま
        let q = scattered_field(&lin, [0.0, 0.0, 1.0], 1.0, 2.0, false);
        assert!((q[0] - lin[0]).norm() < 1e-15 && q[1].norm() < 1e-15);
        // 電場と平行な向きへの散乱は、向きが角度 a の直線偏光に
        let q = scattered_field(&lin, [1.0, 0.0, 0.0], 1.0, 0.0, false);
        assert!(q[0].norm() < 1e-15 && (power_of(&q) - 1.0).abs() < 1e-15);
    }

    #[test]
    fn lost_polarization_is_linear_at_the_angle() {
        let lin: CV3 = [C64::new(1.0, 0.0), C64::new(0.0, 0.0), C64::new(0.0, 0.0)];
        let d = normalize([0.3, -0.2, 1.0]);
        let (p1, p2) = basis(d);
        let a = 0.7f64;
        let q = scattered_field(&lin, d, 2.0, a, true);
        assert!((power_of(&q) - 2.0).abs() < 1e-14);
        for i in 0..3 {
            let want = (a.cos() * p1[i] + a.sin() * p2[i]) * 2f64.sqrt();
            assert!((q[i] - C64::new(want, 0.0)).norm() < 1e-14);
        }
    }
}
