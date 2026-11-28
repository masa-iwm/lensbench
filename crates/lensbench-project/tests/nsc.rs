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
use lensbench_project::{expand, Env, ExpandOpts, Project};
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

/// 視野を斜めにした（光線が BS の面に斜めに入る）p
fn oblique(p: &Project) -> Project {
    let mut v: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
    v["fields"] = serde_json::json!({"type": "angle", "points": [[0, 0], [0.2, 0.3], [-0.5, 0.1]]});
    Project::from_json(&v.to_string()).unwrap()
}

/// 大域の座標に置いた BS（`at`）・合流で入る BS の面でも、斜めの光線の屈折が順次の系と同じ（面の両側の媒質を
/// 光の進む向きで決める。鏡の回数の偶奇では合わない）。媒質の食い違いの注意も出ない
#[test]
fn placed_and_joined_bs_refract_oblique_rays() {
    let f = root().join("examples/projects/P20_mach_zehnder.json");
    let env = Env::for_file(&f, &[root().join("tests/glass")], None);
    let mut cases = vec![("P20".to_string(), Project::load(&f).unwrap(), env)];
    for (s, port, r_loop) in [(1.0, "c", true), (-1.0, "d", false)] {
        cases.push((
            format!("ring {port}"),
            crate::join::ring(s, port, r_loop),
            self::env(),
        ));
    }
    for (label, p, env) in cases {
        let p = oblique(&p);
        let sc = Scene::from_project(&p, &env).unwrap();
        let media: Vec<&String> = sc.notes.iter().filter(|n| n.contains("media")).collect();
        assert!(media.is_empty(), "{label}: {media:?}");
        assert!(project_paths(&label, &p, &env, None) > 0, "{label}");
    }
}

/// 反射 2 回以上の枝が、turn を並べた光束（玉の中の 2 回反射・玉をまたぐゴースト・4 回）と一致する
#[test]
fn multiple_reflections_match_the_turn_lists() {
    for turns in [
        r#"[{"block": "L", "surface": 1}, {"block": "L", "surface": 0}]"#,
        r#"[{"block": "M", "surface": 0}, {"block": "L", "surface": 1}]"#,
        r#"[{"block": "M", "surface": 1}, {"block": "L", "surface": 0}]"#,
        r#"[{"block": "M", "surface": 0}, {"block": "L", "surface": 1},
            {"block": "M", "surface": 1}, {"block": "M", "surface": 0}]"#,
        r#"[{"block": "L", "surface": 1}, {"block": "L", "surface": 0}, {"block": "M", "surface": 0}]"#,
    ] {
        let txt = format!(
            r#"{{"wavelengths": [{{"um": 0.5875618}}], "glass_catalogs": ["SCHOTT"],
              "aperture": {{"type": "epd", "value": 4}}, "object_distance": 100,
              "fields": {{"type": "angle", "points": [[0, 0], [0, 2]]}},
              "blocks": [{{"type": "stop", "semi": 2}}, {{"type": "air", "t": 10}},
              {{"type": "lens", "id": "L", "source": {{"surfaces": [
                 {{"radius": 40, "t": 4, "glass": "N-BK7", "semi": 8}}, {{"radius": -60, "semi": 8}}]}}}},
              {{"type": "air", "t": 8}},
              {{"type": "lens", "id": "M", "source": {{"surfaces": [
                 {{"radius": 30, "t": 3, "glass": "N-BK7", "semi": 8}}, {{"radius": 80, "semi": 8}}]}}}},
              {{"type": "air", "t": 20}}],
              "beams": [{{"name": "g", "turn": {turns}}}]}}"#
        );
        let p = Project::from_json(&txt).unwrap();
        // 並べた turn の数だけ鏡で折り返す
        let x = expand(
            &p,
            &env(),
            &ExpandOpts {
                state: Some("g"),
                ..Default::default()
            },
        )
        .unwrap();
        let mirrors = x.system.surfaces.iter().filter(|d| d.is_mirror()).count();
        assert_eq!(mirrors, p.beams[0].turn.len(), "{turns}");
        assert!(project_paths(turns, &p, &env(), None) > 0, "{turns}");
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

/// 経路ごとの画素の和は検出器の画素の和に一致し、図に描く枝は経路の表と同じ出来事の列で光源の始点から始まる
#[test]
fn path_pixels_and_polylines_agree_with_the_table() {
    let s = load("A0");
    let coat = Coatings::for_system(&s).unwrap();
    let sc = scene_of(&s);
    let tr = Tracer::new(&sc, &[&s], &coat, &NscOpts::default()).unwrap();
    let src = system_source(&s, &[0, s.fields.len() - 1], 9, s.primary, 1.0, None).unwrap();
    let r = tr.run(&src).unwrap();
    let d = &r.detectors[0];
    let mut sum = vec![0.0; d.power.len()];
    for p in &r.paths {
        let inside: f64 = p.pixels.iter().map(|x| x.1).sum();
        assert!(inside <= p.power * (1.0 + 1e-12));
        assert!(p.pixels.windows(2).all(|w| w[0].0 < w[1].0));
        for &(q, w) in &p.pixels {
            sum[q] += w;
        }
    }
    for (a, b) in sum.iter().zip(&d.power) {
        assert!((a - b).abs() <= 1e-12 * b.abs().max(1e-12), "{a} vs {b}");
    }
    let ids = lensbench_project::nsc::spread(src.len(), 20);
    assert!(!ids.is_empty() && ids.len() <= 20);
    let lines = tr.polylines(&src, &ids);
    assert!(!lines.is_empty());
    for l in &lines {
        assert!(r.paths.iter().any(|p| p.events == l.events));
        assert_eq!(l.pts.len(), l.events.len() + 1);
        assert!(ids.iter().any(|&i| src[i].p == l.pts[0]));
        assert_eq!(l.events.last().unwrap().1, EventKind::Detect);
    }
    // 番号の同じ光線は run と同じ枝（2 回反射の枝も出る）
    assert!(lines
        .iter()
        .any(|l| l.events.iter().filter(|e| e.1.reflects()).count() == 2));
}

fn load_path(rel: &str) -> System {
    let opt = lensbench_formats::zmx::LoadOptions {
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    lensbench_formats::zmx::load_with(&root().join(rel), &opt).unwrap()
}

/// 自動の半直径の系（端の視野の瞳の縁の光線が面の開口の円にちょうど当たる）: 面と縁の継ぎ目で閉じ込められず、
/// 像面の半直径ちょうどの既定の検出器にも届く（反射の無い光は大きな検出器と同じ値）
#[test]
fn rays_at_the_edge_of_a_face_are_not_trapped() {
    for rel in [
        "tests/surftypes_os/O3/system.zmx",
        "tests/surftypes_os/I7/system.zmx",
        "tests/surftypes_os/I4/system.zmx",
        "tests/refs/Z1/system.zmx",
    ] {
        let s = load_path(rel);
        let coat = Coatings::for_system(&s).unwrap();
        let sc = scene_of(&s);
        let last = s.fields.len() - 1;
        let src = system_source(&s, &[last], 5, s.primary, 1.0, None).unwrap();
        let t0 = std::time::Instant::now();
        let r = Tracer::new(&sc, &[&s], &coat, &NscOpts::default())
            .unwrap()
            .run(&src)
            .unwrap();
        let dt = t0.elapsed().as_secs_f64();
        check_budget(rel, &r.budget, 1e-12);
        assert_eq!(r.budget.truncated, 0.0, "{rel}");
        let w = Tracer::new(&sc, &[&s], &coat, &wide())
            .unwrap()
            .run(&src)
            .unwrap();
        // 反射の無い経路（順次の光）は、像面の半直径ちょうどの既定の検出器でも全部数える。反射した枝は像面の外にも落ちる
        let direct = |r: &lensbench_project::nsc::NscResult| -> f64 {
            r.paths
                .iter()
                .filter(|p| p.reflections() == 0)
                .map(|p| p.power)
                .sum()
        };
        assert!(
            (direct(&r) - direct(&w)).abs() < 1e-12,
            "{rel}: direct light {} with the default detector, {} with a wide one",
            direct(&r),
            direct(&w)
        );
        eprintln!("{rel}: {dt:.3} s, detected {:.6}", r.budget.detected);
    }
}

/// 合流で同じ BS へ戻る輪（サニャック・リング共振器）の枝が、輪の経路ごとの展開と一致する
#[test]
fn ring_branches_match_the_ring_paths() {
    for (s, port, r_loop) in [(1.0, "c", true), (-1.0, "d", false)] {
        let p = crate::join::ring(s, port, r_loop);
        project_paths(&format!("ring {port}"), &p, &env(), None);
    }
}
