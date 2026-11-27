//! 非順次の追跡: 決まった経路の枝が順次の展開（ゴースト・光束の turn・BS の経路）と光線 1 本ずつ一致する、
//! エネルギーの収支、モンテカルロと決まった分岐、スレッド数で同じ値

use lensbench_analysis::ghost::{ghost_system, pairs};
use lensbench_core::coating::{Coatings, C64};
use lensbench_core::math::V3;
use lensbench_core::system::{FieldSel, Launch, PolIn, RecSel};
use lensbench_core::System;
use lensbench_project::nsc::{
    system_source, Budget, DetectorSpec, End, NscOpts, Split, StartRay, Tracer,
};
use lensbench_project::raypath::{EventKind, RayPath};
use lensbench_project::scene::{expansions, ObjectKind, Scene};
use lensbench_project::{Env, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(name: &str) -> System {
    let opt = lensbench_formats::zmx::LoadOptions {
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    lensbench_formats::zmx::load_with(&root().join(format!("tests/refs/{name}/system.zmx")), &opt)
        .unwrap()
}

/// 系 1 つの場面
fn scene_of(s: &System) -> Scene {
    let mut sc = Scene::new();
    sc.add_system("system", s, None).unwrap();
    sc.finish().unwrap();
    sc
}

/// 大きな検出器（ゴーストの枝が像面の半直径の外に落ちても数える）
fn wide() -> NscOpts {
    NscOpts {
        detector: DetectorSpec {
            half: Some([1e3, 1e3]),
            pixels: [8, 8],
        },
        ..Default::default()
    }
}

/// 順次の系の経路を場面の面の出来事の列に直す。to_face は系の面 → 場面の面。場面に面の無い面（空気中のダミー）・
/// 絞り・働きの無い面（同じ媒質の間で膜の無い面）は非順次で出来事にならないので除く
fn expected(
    sc: &Scene,
    path: &RayPath,
    to_face: impl Fn(usize) -> Option<usize>,
) -> Vec<(usize, EventKind)> {
    path.0
        .iter()
        .filter_map(|e| {
            let f = to_face(e.surf)?;
            let face = &sc.faces[f];
            if sc.objects[face.object].kind == ObjectKind::Stop {
                return None;
            }
            let kind = sc.objects[face.object].kind;
            if kind == ObjectKind::Lens
                && face.minus.label() == face.plus.label()
                && face.coat.is_empty()
            {
                return None;
            }
            Some((f, e.kind))
        })
        .collect()
}

/// 物体の座標系の (Ex, Ey) を方向 k に垂直な面へ射影した電場（順次の偏光追跡の入力と同じ）の |E|²
fn e0_power(s: &System, k: V3, j: [C64; 2]) -> f64 {
    let r = s.frames().obj.r;
    let v: [C64; 3] = [0, 1, 2].map(|c| j[0] * r[c][0] + j[1] * r[c][1]);
    let kk = lensbench_core::math::normalize(k);
    let jk = v[0] * kk[0] + v[1] * kk[1] + v[2] * kk[2];
    (0..3).map(|c| (v[c] - jk * kk[c]).norm_sqr()).sum()
}

/// 光源の光線（視野ごとの瞳の点）
fn rays_of(s: &System) -> Vec<(V3, V3)> {
    let w = s.primary;
    let pts = [(0.0, 0.0), (0.7, 0.0), (0.0, -0.6), (-0.4, 0.5), (0.3, 0.3)];
    let mut v = Vec::new();
    for f in 0..s.fields.len() {
        let (hx, hy) = s.field_sel_norm(FieldSel::Index(f), w).unwrap();
        let spec = s.object_spec(hx, hy).unwrap();
        for &(x, y) in &pts {
            if let Some(q) = s.aim_pupil(spec, [x, y], w) {
                let (p, k) = s.pupil_ray(spec, q);
                v.push((lensbench_project::nsc::back_off(s, spec, p, k), k));
            }
        }
    }
    v
}

/// 順次の系 seq（経路 want）と非順次の枝を、光線ごと・偏光 x・y ごとに比べる。比べた数を返す
#[allow(clippy::too_many_arguments)]
fn compare(
    label: &str,
    tr: &Tracer,
    seq: &System,
    coat: &Coatings,
    want: &[(usize, EventKind)],
    rays: &[(V3, V3)],
    pos_tol: f64,
    pow_tol: f64,
) -> usize {
    let w = seq.primary;
    let mut n = 0;
    let r = seq.frames().obj.r;
    for (i, &(p, k)) in rays.iter().enumerate() {
        for j in [
            [C64::new(1.0, 0.0), C64::new(0.0, 0.0)],
            [C64::new(0.0, 0.0), C64::new(1.0, 0.0)],
        ] {
            let t = seq
                .trace_pol_launch(
                    &Launch::Rays { p: &[p], k: &[k] },
                    w,
                    PolIn::new(&[j], coat).no_phase(),
                    RecSel::Off,
                )
                .unwrap();
            let ro = &t.trace.rays[0];
            if !ro.status.is_ok() || ro.vig != 0 {
                continue;
            }
            let p_seq: f64 =
                t.e[0][0].iter().map(|z| z.norm_sqr()).sum::<f64>() / e0_power(seq, k, j);
            let st = StartRay::polarized(p, k, j, &r, 1.0);
            let mut out = Vec::new();
            tr.trace(&st, i as u64, false, &mut out);
            let br: Vec<_> = out
                .iter()
                .filter(|b| {
                    let ev: Vec<(usize, EventKind)> =
                        b.hits.iter().map(|h| (h.face, h.kind)).collect();
                    ev == want
                })
                .collect();
            // 縁・機構部品などで順次と違う所に当たった枝は無いことがある（順次の追跡は縁を見ない）
            let Some(b) = br.first() else {
                continue;
            };
            assert_eq!(
                br.len(),
                1,
                "{label} ray {i}: {} branches with the same path",
                br.len()
            );
            let End::Detected { x, y, .. } = b.end else {
                panic!("{label}: not detected")
            };
            let (dx, dy) = (x - ro.xyz[0], y - ro.xyz[1]);
            assert!(
                dx.abs() < pos_tol && dy.abs() < pos_tol,
                "{label} ray {i} {j:?}: position off by ({dx:e}, {dy:e}) mm"
            );
            let rel = (b.power - p_seq).abs() / p_seq.max(1e-300);
            assert!(
                rel < pow_tol,
                "{label} ray {i} {j:?}: power {} vs {} (rel {rel:e})",
                b.power,
                p_seq
            );
            n += 1;
        }
    }
    n
}

/// 2 回反射の枝がゴーストの展開（ghost_system）と位置 1e-9 mm・パワー 1e-9 で一致する（A0・F0 の全組）
#[test]
fn double_bounces_match_the_ghost_systems() {
    for name in ["A0", "F0"] {
        let s = load(name);
        let coat = Coatings::for_system(&s).unwrap();
        let sc = scene_of(&s);
        let tr = Tracer::new(&sc, &[&s], &coat, &wide()).unwrap();
        let rays = rays_of(&s);
        let mut total = 0;
        let ps = pairs(&s, 1, s.image());
        assert!(!ps.is_empty());
        for (a, b) in ps {
            let g = ghost_system(&s, a, b).unwrap();
            let path = RayPath::sequential(&g.sys, None).unwrap();
            let want = expected(&sc, &path, |j| g.origin[j].and_then(|o| sc.face_of(0, o)));
            assert_eq!(
                want.iter().filter(|e| e.1.reflects()).count(),
                2,
                "{name} {a} {b}"
            );
            let gc = Coatings::for_system(&g.sys).unwrap();
            total += compare(
                &format!("{name} ghost {a} {b}"),
                &tr,
                &g.sys,
                &gc,
                &want,
                &rays,
                1e-9,
                1e-9,
            );
        }
        // ほとんどの組・光線で比べている
        eprintln!("{name}: {total} branches compared");
        assert!(total > 100, "{name}: only {total} branches compared");
        // 反射の無い枝は元の系そのもの
        let path = RayPath::sequential(&s, None).unwrap();
        let want = expected(&sc, &path, |j| sc.face_of(0, j));
        let n = compare(
            &format!("{name} direct"),
            &tr,
            &s,
            &coat,
            &want,
            &rays,
            1e-9,
            1e-9,
        );
        assert!(n > 0);
    }
}

fn env() -> Env<'static> {
    Env {
        step_files: None,
        base: root().join("tests/refs"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

/// プロジェクトの場面の、展開ごとの経路の枝が展開した系と一致する。比べた数を返す
fn project_paths(label: &str, p: &Project, env: &Env, coat: Option<&Coatings>) -> usize {
    let xs = expansions(p, env).unwrap();
    let sc = Scene::from_project(p, env).unwrap();
    let systems: Vec<&System> = xs.iter().map(|x| &x.1.system).collect();
    let own = Coatings::for_system(systems[0]).unwrap();
    let coat = coat.unwrap_or(&own);
    let tr = Tracer::new(&sc, &systems, coat, &wide()).unwrap();
    let mut n = 0;
    for (i, (name, x)) in xs.iter().enumerate() {
        let s = &x.system;
        let path = RayPath::sequential(s, None).unwrap();
        let want = expected(&sc, &path, |j| sc.face_of(i, j));
        let rays = rays_of(s);
        let m = compare(
            &format!("{label} {name}"),
            &tr,
            s,
            coat,
            &want,
            &rays,
            1e-9,
            1e-9,
        );
        assert!(m > 0, "{label} {name}: nothing compared");
        n += m;
    }
    n
}

/// 反射 1 回の枝が光束の turn（玉の面での部分反射で戻る）と一致する
#[test]
fn single_reflections_match_the_turned_beams() {
    for surf in [0, 1] {
        let txt = format!(
            r#"{{"wavelengths": [{{"um": 0.5875618}}], "glass_catalogs": ["SCHOTT"],
              "aperture": {{"type": "epd", "value": 4}}, "object_distance": 100,
              "fields": {{"type": "angle", "points": [[0, 0], [0, 2]]}},
              "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}},
              {{"type": "lens", "id": "L", "source": {{"surfaces": [
                 {{"radius": 40, "t": 4, "glass": "N-BK7", "semi": 8}}, {{"radius": -60, "semi": 8}}]}}}},
              {{"type": "air", "t": 20}}],
              "beams": [{{"name": "g", "turn": {{"block": "L", "surface": {surf}}}}}]}}"#
        );
        let p = Project::from_json(&txt).unwrap();
        project_paths(&format!("turn {surf}"), &p, &env(), None);
    }
}

/// BS の経路の枝が tests/splitter の系（BS の経路ごとの展開）と一致する
#[test]
fn splitter_branches_match_the_bs_paths() {
    let mut n = 0;
    for e in std::fs::read_dir(root().join("tests/splitter"))
        .unwrap()
        .flatten()
    {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "json") && !p.ends_with("bs_refs.json") {
            let pr = Project::load(&p).unwrap();
            let env = Env::for_file(&p, &[root().join("tests/glass")], None);
            let c = Coatings {
                file: lensbench_core::coating::CoatingFile::load(
                    &root().join("tests/splitter/LBBS.DAT"),
                )
                .unwrap(),
                ..Default::default()
            };
            n += project_paths(&p.display().to_string(), &pr, &env, Some(&c));
        }
    }
    assert!(n > 0);
}

fn check_budget(label: &str, b: &Budget, tol: f64) {
    let r = b.residual() / b.source;
    assert!(
        r.abs() < tol,
        "{label}: residual {r:e} of the source ({b:?})"
    );
}

/// エネルギーの収支（検出 + 吸収 + 外へ + 閾値で捨てた + 上限で止めた = 光源）が 1e-12
#[test]
fn the_energy_budget_closes() {
    for name in ["A0", "F0", "C0"] {
        let s = load(name);
        let coat = Coatings::for_system(&s).unwrap();
        let sc = scene_of(&s);
        let tr = Tracer::new(&sc, &[&s], &coat, &NscOpts::default()).unwrap();
        let src = system_source(&s, &[0, s.fields.len() - 1], 9, s.primary, 1.0, None).unwrap();
        let r = tr.run(&src).unwrap();
        check_budget(name, &r.budget, 1e-12);
        assert!((r.budget.source - 1.0).abs() < 1e-12);
        let det: f64 = r.detectors.iter().map(|d| d.total()).sum();
        // 検出器の矩形の外（像面の半直径の外）の枝は数えないので、画素の和は検出の和以下
        assert!(det <= r.budget.detected * (1.0 + 1e-12));
        let paths: f64 = r.paths.iter().map(|p| p.power).sum();
        assert!(
            (paths - r.budget.detected).abs() < 1e-12,
            "{name}: {paths} vs {}",
            r.budget.detected
        );
        eprintln!(
            "{name}: {} paths, detected {:.6}, absorbed {:.3e}, escaped {:.3e}, discarded {:.3e}",
            r.paths.len(),
            r.budget.detected,
            r.budget.absorbed,
            r.budget.escaped,
            r.budget.discarded
        );
    }
}

/// モンテカルロは決まった分岐と画素ごとに 3σ 以内、スレッド数 1・4 で同じ値
#[test]
fn monte_carlo_agrees_with_the_deterministic_split() {
    let s = load("A0");
    let coat = Coatings::for_system(&s).unwrap();
    let sc = scene_of(&s);
    let det = NscOpts {
        detector: DetectorSpec {
            half: None,
            pixels: [8, 8],
        },
        ..Default::default()
    };
    let mc = NscOpts {
        split: Split::MonteCarlo { seed: 11 },
        ..det.clone()
    };
    let src = system_source(&s, &[0], 41, s.primary, 1.0, None).unwrap();
    let a = Tracer::new(&sc, &[&s], &coat, &det)
        .unwrap()
        .run(&src)
        .unwrap();
    // 光線を増やしてばらつきを小さく（同じ光線を何度も撃つ: 乱数は光線の番号ごと）
    let many: Vec<StartRay> = (0..20).flat_map(|_| src.iter().copied()).collect();
    let tm = Tracer::new(&sc, &[&s], &coat, &mc).unwrap();
    let m1 = lensbench_core::par::with_threads(Some(1), || tm.run(&many).unwrap());
    let m4 = lensbench_core::par::with_threads(Some(4), || tm.run(&many).unwrap());
    assert_eq!(m1, m4);
    let d4 = lensbench_core::par::with_threads(Some(4), || {
        Tracer::new(&sc, &[&s], &coat, &det)
            .unwrap()
            .run(&src)
            .unwrap()
    });
    assert_eq!(a, d4);
    let k = 20.0;
    let (pa, pm) = (&a.detectors[0], &m1.detectors[0]);
    let mut checked = 0;
    for q in 0..pa.power.len() {
        let want = pa.power[q];
        let got = pm.power[q] / k;
        // モンテカルロの枝 1 本のパワーは光源の光線 1 本のパワー（unit）の程度で、画素に入る本数はポアソン:
        // 期待値 want·k / unit 本、σ(got) = √(want · unit / k)
        let unit = 1.0 / src.len() as f64;
        let sigma = (want * unit / k).sqrt() + 1e-12;
        assert!(
            (got - want).abs() < 3.0 * sigma.max(want * 1e-6),
            "pixel {q}: mc {got} vs {want} (σ {sigma})"
        );
        if want > 0.0 {
            checked += 1;
        }
    }
    assert!(checked > 4);
}

/// 空気中の絞りは開口の外を遮る有限の板: 最初に当たるのが板の中（開口の外）なら吸収、開口の中と板の外径（システムの面の
/// 半直径の最大）の外は素通り
#[test]
fn the_stop_is_a_finite_plate() {
    let s = load("A0");
    let coat = Coatings::for_system(&s).unwrap();
    let sc = scene_of(&s);
    let tr = Tracer::new(&sc, &[&s], &coat, &wide()).unwrap();
    let fi = sc
        .faces
        .iter()
        .position(|f| sc.objects[f.object].kind == ObjectKind::Stop)
        .expect("a stop");
    let f = &sc.faces[fi];
    let semi = f.refs[0].surf;
    let semi = s.surfaces[semi].semi;
    let big = s
        .surfaces
        .iter()
        .filter(|d| !d.ignore)
        .map(|d| d.semi.max(d.mech_semi.unwrap_or(0.0)))
        .fold(0.0, f64::max);
    let col = |c: usize| -> V3 { [f.frame.r[0][c], f.frame.r[1][c], f.frame.r[2][c]] };
    let (ex, ez) = (col(0), col(2));
    let start = |rad: f64| -> V3 { [0, 1, 2].map(|a| f.frame.o[a] + ex[a] * rad - ez[a] * 1e-3) };
    let hit_stop = |rad: f64| {
        let st = StartRay::polarized(
            start(rad),
            ez,
            [C64::new(1.0, 0.0), C64::new(0.0, 0.0)],
            &f.frame.r,
            1.0,
        );
        let mut out = Vec::new();
        tr.trace(&st, 0, true, &mut out);
        // 光源の光線が最初に当たる所（反射して戻った枝は見ない）
        out.iter().any(|b| {
            b.hits
                .first()
                .is_some_and(|h| h.face == fi && h.kind == EventKind::Absorb)
        })
    };
    assert!(big > semi * 1.01, "A0: plate {big} vs aperture {semi}");
    assert!(
        hit_stop(0.5 * (semi + big)),
        "inside the plate, outside the aperture"
    );
    assert!(!hit_stop(0.5 * semi), "inside the aperture");
    assert!(!hit_stop(big * 1.5), "outside the plate");
}
