use lensbench_core::Waves;
use serde_json::json;

use crate::registry::{Analyses, ParamTy};

#[test]
fn params_from_schema() {
    let a = Analyses::builtin();
    let mtf = a.get("mtf").unwrap();
    let ps = mtf.params();
    let get = |n: &str| ps.iter().find(|p| p.name == n).unwrap().clone();
    assert_eq!(get("field").ty, ParamTy::Count);
    assert!(get("field").optional);
    assert_eq!(get("freqs").ty, ParamTy::Nums);
    assert_eq!(get("max_freq").ty, ParamTy::Num);
    assert_eq!(get("freq_steps").default, json!(20));
    assert_eq!(get("freq_steps").min, Some(1.0));
    assert_eq!(get("method").ty, ParamTy::Choice);
    assert_eq!(get("method").choices, ["auto", "interp", "direct", "fft"]);
    assert_eq!(get("each_wave").ty, ParamTy::Flag);
    assert!(get("max_freq").doc.contains("上限"));
    let rms = a.get("rms").unwrap().params();
    assert!(rms
        .iter()
        .any(|p| p.name == "reference" && p.choices == ["centroid", "chief"]));
    let g = a.get("ghost").unwrap().params();
    assert_eq!(
        g.iter().find(|p| p.name == "coatings").unwrap().ty,
        ParamTy::Paths
    );
    assert_eq!(
        g.iter().find(|p| p.name == "out_dir").unwrap().ty,
        ParamTy::Path
    );
}

fn a0() -> lensbench_core::System {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = lensbench_core::zmx::LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    lensbench_core::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap()
}

/// 同梱の解析はどれも既定の設定（周波数の要るものは周波数だけ足す）で動き、結果は core の関数と同じ値
#[test]
fn every_builtin_runs_with_defaults() {
    let s = a0();
    let cx = crate::registry::Cx::new(&s);
    for a in Analyses::builtin().iter() {
        let name = a.meta().name;
        let p = match name {
            "mtf-focus" => json!({ "freqs": [10.0, 20.0], "points": 3 }),
            "mtf-field" => json!({ "freqs": [10.0, 20.0], "points": 3 }),
            "psf" => json!({ "field": 1, "image": 32 }),
            "huygens-mtf" => json!({ "field": 1, "image": 32, "freq_steps": 2 }),
            _ => json!({}),
        };
        let r = a.run_json(&cx, &p);
        let v = r.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(v.is_object(), "{name}");
    }
    let v = Analyses::builtin()
        .get("spot")
        .unwrap()
        .run_json(&cx, &json!({ "field": 2, "wave": 1 }))
        .unwrap();
    let sp =
        lensbench_core::analysis::spot(&s, lensbench_core::FieldSel::Index(1), Waves::One(0), 6)
            .unwrap();
    assert_eq!(
        v["spot"][0]["rms_centroid"].as_f64().unwrap().to_bits(),
        sp.rms_centroid.to_bits()
    );
    assert_eq!(v["spot"][0]["field"], json!(2));
}

#[test]
fn unknown_keys_and_types_are_errors() {
    let s = a0();
    let cx = crate::registry::Cx::new(&s);
    let a = Analyses::builtin().get("spot").unwrap();
    let e = a
        .run_json(&cx, &json!({ "ringz": 3 }))
        .unwrap_err()
        .to_string();
    assert!(e.contains("ringz"), "{e}");
    let e = a
        .run_json(&cx, &json!({ "rings": "x" }))
        .unwrap_err()
        .to_string();
    assert!(e.contains("spot の設定"), "{e}");
}
