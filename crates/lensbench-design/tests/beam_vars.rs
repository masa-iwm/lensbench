//! 光束をまたぐ最適化: 変数 `ブロック:面.種類` を光束ごとの系・往復の面に当て、構成の差の項（diff）で光路長の差を合わせる

use lensbench_design::dls_spec::{OptimizeParams, VarSpec};
use lensbench_project::{expand, Env, ExpandOpts, Project};
use serde_json::json;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env() -> Env<'static> {
    Env {
        step_files: None,
        base: root().join("examples/projects"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

/// P16（被検の玉 TEST を往復する腕と参照の腕）に光束 test・ref と、TEST の面 1 の部分反射で戻る光束 ghost を足し、
/// TEST の面 0 の半径を r0 にしたもの
fn project(r0: f64) -> Project {
    let txt =
        std::fs::read_to_string(root().join("examples/projects/P16_twyman_green.json")).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    v["blocks"][2]["t"][1]["source"]["surfaces"][0]["radius"] = json!(r0);
    v["beams"] = json!([
        {"name": "test", "path": "TR"},
        {"name": "refb", "path": "RT"},
        {"name": "ghost", "path": "TR", "turn": {"block": "TEST", "surface": 1}}
    ]);
    Project::from_json(&v.to_string()).unwrap()
}

fn beams(
    p: &Project,
) -> (
    Vec<lensbench_core::System>,
    Vec<Vec<lensbench_project::BlockSpans>>,
) {
    ["test", "refb", "ghost"]
        .iter()
        .map(|b| {
            let x = expand(
                p,
                &env(),
                &ExpandOpts {
                    state: Some(b),
                    ..Default::default()
                },
            )
            .unwrap();
            let sp = x.block_spans();
            (x.system, sp)
        })
        .unzip()
}

/// 共通の玉の面を変えると、光束ごとの系（往復の 2 面・部分反射の枝）がそれぞれ単独で変えたものと一致する
#[test]
fn a_block_face_variable_moves_every_pass_of_every_beam() {
    let (sys, spans) = beams(&project(60.0));
    let op = OptimizeParams {
        vars: vec![VarSpec::Name("TEST:0.r".into())],
        terms: vec![json!({"type": "paraxial", "q": "efl", "target": 0, "weight": 0})],
        merit: lensbench_design::dls_spec::Merit::None,
        ..OptimizeParams::default()
    };
    let (prob, _) = op.problem_with(sys, &spans).unwrap();
    prob.check().unwrap();
    let v = &prob.vars[0];
    // test: 往路と復路の 2 面、refb: なし、ghost: 往路の 1 面と戻りの 1 面
    let n = |c: usize| {
        usize::from(v.configs.as_ref().unwrap().contains(&c))
            + v.links.iter().filter(|l| l.0 == c).count()
    };
    assert_eq!((n(0), n(1), n(2)), (2, 0, 2));
    assert_eq!(prob.x0().unwrap(), vec![60.0]);
    let got = prob.systems(&[55.0]).unwrap();
    let (want, _) = beams(&project(55.0));
    for (g, w) in got.iter().zip(&want) {
        assert_eq!(g.surfaces.len(), w.surfaces.len());
        for (a, b) in g.surfaces.iter().zip(&w.surfaces) {
            assert!((a.c - b.c).abs() < 1e-15, "{} {}", a.c, b.c);
        }
        let ta = g
            .try_trace(
                lensbench_core::FieldSel::Index(1),
                &[0.3],
                &[0.7],
                None,
                None,
                false,
            )
            .unwrap();
        let tb = w
            .try_trace(
                lensbench_core::FieldSel::Index(1),
                &[0.3],
                &[0.7],
                None,
                None,
                false,
            )
            .unwrap();
        assert!((ta.rays[0].opl - tb.rays[0].opl).abs() < 1e-9);
    }
}

/// diff の項で 2 つの光束の主光線の光路長の差を合わせる（参照のミラーの位置を変数に）
#[test]
fn a_difference_term_matches_the_path_lengths_of_two_beams() {
    let (sys, spans) = beams(&project(60.0));
    let opth = json!({"type": "ray", "q": "opth", "field": 1, "px": 0, "py": 0});
    let op = OptimizeParams {
        vars: vec![VarSpec::Full {
            name: "M_REF.shift".into(),
            lo: None,
            hi: None,
            configs: Some(vec![2]),
        }],
        terms: vec![json!({"type": "diff", "a": 1, "b": 2, "term": opth, "target": 0.0})],
        merit: lensbench_design::dls_spec::Merit::None,
        ..OptimizeParams::default()
    };
    let (prob, _) = op.problem_with(sys.clone(), &spans).unwrap();
    prob.check().unwrap();
    let x0 = prob.x0().unwrap();
    let d0 = prob.term_values(&x0).unwrap()[0].value();
    let opl = |s: &lensbench_core::System| {
        s.try_trace(
            lensbench_core::FieldSel::Index(0),
            &[0.0],
            &[0.0],
            None,
            None,
            false,
        )
        .unwrap()
        .rays[0]
            .opl
    };
    assert!((d0 - (opl(&sys[0]) - opl(&sys[1]))).abs() < 1e-9, "{d0}");
    assert!(d0.abs() > 1.0, "{d0}");
    let run = op.run_in(&prob, &[root().join("tests/glass")]).unwrap();
    let d = run.problem.term_values(&run.result.x).unwrap()[0].value();
    assert!(d.abs() < 1e-6, "{d}");
    // 往復なので、ミラーを光路長の差の半分だけ動かす（反射の後の腕なので、厚さの符号は逆）
    assert!(
        ((run.result.x[0] - x0[0]).abs() - d0.abs() / 2.0).abs() < 1e-6,
        "{:?} {x0:?} {d0}",
        run.result.x
    );
}
