//! 条件で定める視野: 実像高との一致、法線入射で戻る主光線の視野、パラメータ変更での解き直し、プロジェクトファイル。

use lensbench_core::analysis;
use lensbench_core::catalog::{LensCatalog, ZmxFileSource};
use lensbench_core::solve::{RayProblem, Residual, Unknown};
use lensbench_core::system::{FieldCond, FieldSel, FieldType, ObjSpec};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn zmx_sys(name: &str) -> System {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root().join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap()
}

const P14: &str = "examples/projects/P14_tilted_spherical_double_pass.json";

fn p14_json() -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(root().join(P14)).unwrap()).unwrap()
}

/// P14（傾けた球面鏡のダブルパス）をプロジェクトの JSON v から読む
fn load_json(v: &serde_json::Value, tag: &str) -> Result<System, String> {
    let dir = std::env::temp_dir().join(format!("lb_field_solve_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("p.json");
    std::fs::write(&p, serde_json::to_string(v).unwrap()).unwrap();
    let cat = LensCatalog::load(&[root().join("examples/lenses/LB")], &[&ZmxFileSource]).unwrap();
    let look = |a: &str, b: &str| cat.zmx(a, b);
    let r = lensbench_project::load(&p, &[root().join("tests/glass")], Some(&look));
    let _ = std::fs::remove_dir_all(&dir);
    r
}

fn bits(s: &System) -> Vec<u64> {
    let (px, py) = analysis::pupil_grid(8);
    let mut v = Vec::new();
    for f in 0..s.fields.len() {
        for w in 0..s.wavelengths.len() {
            let t = s
                .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                .unwrap();
            for r in &t.rays {
                v.extend(r.xyz.iter().chain(r.lmn.iter()).map(|x| x.to_bits()));
                v.push(r.opl.to_bits());
            }
        }
    }
    v
}

/// 像面の位置の条件は実像高と同じビット
#[test]
fn image_position_is_real_image_height() {
    for name in ["A0", "C0", "B0"] {
        let mut a = zmx_sys(name);
        // 視野の値を像高にする（角度の視野の像面での位置）
        let fs: Vec<_> = (0..a.fields.len())
            .map(|i| {
                let t = a.trace(FieldSel::Index(i), &[0.0], &[0.0], None, None, false);
                (t.rays[0].xyz[0], t.rays[0].xyz[1])
            })
            .collect();
        for (f, &(x, y)) in a.fields.iter_mut().zip(&fs) {
            f.x = x;
            f.y = y;
        }
        a.field_type = FieldType::RealH;
        a.changed().unwrap();
        let mut b = zmx_sys(name);
        for (f, &(x, y)) in b.fields.iter_mut().zip(&fs) {
            f.x = x;
            f.y = y;
        }
        b.field_type = FieldType::Solve(FieldCond::Position { surf: b.image() });
        b.changed().unwrap();
        assert_eq!(bits(&a), bits(&b), "{name}");
    }
}

/// 法線入射で戻る主光線の視野: solve_ray と同じ物点、スポット・MTF が視野番号で動く、パラメータ変更で解き直す
#[test]
fn chief_ray_returning_on_itself_as_a_field() {
    let mut v = p14_json();
    v["fields"] = serde_json::json!({
        "type": "solve",
        "solve": { "cond": "dir_sum", "s1": "1", "s2": "6" },
        "points": [[0.0, 0.0], [0.002, -0.001]]
    });
    let s = load_json(&v, "a").unwrap();
    assert_eq!(
        s.field_type,
        FieldType::Solve(FieldCond::DirSum { s1: 1, s2: 6 })
    );
    let w = s.primary;
    for (i, tgt) in [[0.0, 0.0], [0.002, -0.001]].into_iter().enumerate() {
        let (hx, hy) = s.hxhy(i);
        let spec = s.object_spec(hx, hy).unwrap();
        let ObjSpec::Point(o) = spec else { panic!() };
        // 主光線の方向余弦の和が目標値
        let t = s.trace_spec(spec, &[0.0], &[0.0], w, Some([0.0; 5]), None, true, None);
        let (a, b) = (t.rec_at(1).unwrap().lmn[0], t.rec_at(6).unwrap().lmn[0]);
        let r = [a[0] + b[0] - tgt[0], a[1] + b[1] - tgt[1]];
        assert!(r[0].abs().max(r[1].abs()) < 1e-12, "視野 {i}: {r:?}");
        if i == 0 {
            let sol = s
                .solve_ray(&RayProblem::new(
                    Unknown::Object,
                    Residual::DirSum { s1: 1, s2: 6 },
                    ObjSpec::Point([0.0, 0.0]),
                    w,
                ))
                .unwrap();
            let ObjSpec::Point(q) = sol.object else {
                panic!()
            };
            assert!(
                (q[0] - o[0]).abs().max((q[1] - o[1]).abs()) < 1e-9,
                "{q:?} {o:?}"
            );
        }
        analysis::spot(&s, FieldSel::Index(i), None, 4).unwrap();
        analysis::mtf(&s, FieldSel::Index(i), Some(w), Some(16), &[0.0, 10.0]).unwrap();
    }
    let o0 = s.object_spec(0.0, 0.0).unwrap();
    let s2 = s.with_params(&[(s.param("3.dx").unwrap(), 0.5)]).unwrap();
    let o1 = s2.object_spec(0.0, 0.0).unwrap();
    assert_ne!(o0, o1, "パラメータを変えたら解き直す");
}

#[test]
fn project_errors() {
    let mut v = p14_json();
    v["fields"] = serde_json::json!({ "type": "solve", "points": [[0.0, 0.0]] });
    assert!(load_json(&v, "b").unwrap_err().contains("solve"));
    v["fields"] = serde_json::json!({
        "type": "solve", "solve": { "cond": "position", "surface": "nope" }, "points": [[0.0, 0.0]]
    });
    assert!(load_json(&v, "c").is_err());
    v["fields"] = serde_json::json!({
        "type": "angle", "solve": { "cond": "position", "surface": "1" }, "points": [[0.0, 0.0]]
    });
    assert!(load_json(&v, "d").is_err());
}

/// .zmx には書けない
#[test]
fn zmx_write_refuses() {
    let mut s = zmx_sys("A0");
    s.field_type = FieldType::Solve(FieldCond::DirSum { s1: 1, s2: 2 });
    assert!(lensbench_core::zmx_write::write(&s).is_err());
}
