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
    assert!(get("max_freq").doc.starts_with("Upper limit"));
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
        g.iter().find(|p| p.name == "out").unwrap().ty,
        ParamTy::Path
    );
}

fn a0() -> lensbench_core::System {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = lensbench_formats::zmx::LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    lensbench_formats::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap()
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
        // 型のままの結果は公開した結果の型で、JSON は同じ。JSON から型へ読み直せる（Web 版の画面）
        let t = a.run_typed(&cx, &p).unwrap();
        assert_eq!(t.to_json().unwrap(), v, "{name}");
        fn back<T: serde::de::DeserializeOwned + serde::Serialize + 'static>(
            t: &crate::Typed,
            v: &serde_json::Value,
        ) {
            assert!(t.get::<T>().is_some(), "{}", t.name());
            let x: T =
                serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{}: {e}", t.name()));
            assert_eq!(&serde_json::to_value(&x).unwrap(), v, "{}", t.name());
        }
        use super::*;
        match name {
            "spot" => back::<SpotOut>(&t, &v),
            "wavefront" => back::<WavefrontOut>(&t, &v),
            "zernike" => back::<ZernikeOut>(&t, &v),
            "ghost" => back::<GhostOut>(&t, &v),
            "rms" => back::<RmsOut>(&t, &v),
            "psf" => back::<PsfOut>(&t, &v),
            "mtf" | "huygens-mtf" => back::<MtfOut>(&t, &v),
            "mtf-focus" => back::<MtfFocusOut>(&t, &v),
            "mtf-field" => back::<MtfFieldOut>(&t, &v),
            "geo-mtf" => back::<GeoMtfOut>(&t, &v),
            "ray-fan" | "opd-fan" => back::<FanOut>(&t, &v),
            "field-curvature" => back::<FieldCurvOut>(&t, &v),
            "longitudinal" => back::<LongOut>(&t, &v),
            "lateral-color" => back::<LatOut>(&t, &v),
            "focal-shift" => back::<ShiftOut>(&t, &v),
            "grid-distortion" => back::<GridOut>(&t, &v),
            "seidel" => back::<SeidelOut>(&t, &v),
            "footprint" => back::<FootprintOut>(&t, &v),
            "relative-illumination" => back::<RelIllumOut>(&t, &v),
            "vignetting" => back::<VignettingOut>(&t, &v),
            n => panic!("no result type listed for {n}"),
        }
    }
    let v = Analyses::builtin()
        .get("spot")
        .unwrap()
        .run_json(&cx, &json!({ "field": 2, "wave": 1 }))
        .unwrap();
    let sp =
        crate::analysis::spot(&s, lensbench_core::FieldSel::Index(1), Waves::One(0), 6).unwrap();
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
    assert!(e.contains("settings of spot"), "{e}");
}

/// 設定 polarization（膜・偏光の振幅）は psf・mtf・huygens-mtf で使え、膜のないガラスの系では値が少し変わる
#[test]
fn polarization_option_changes_the_pupil_amplitude() {
    let s = a0();
    let cx = crate::registry::Cx::new(&s);
    let a = Analyses::builtin();
    for (name, p) in [
        ("psf", json!({ "field": 1, "image": 32 })),
        ("mtf", json!({ "field": 1, "freqs": [20.0, 40.0] })),
        (
            "huygens-mtf",
            json!({ "field": 1, "image": 32, "freqs": [20.0] }),
        ),
    ] {
        let off = a.get(name).unwrap().run_json(&cx, &p).unwrap();
        let mut q = p.clone();
        q["polarization"] = json!(true);
        let on = a.get(name).unwrap().run_json(&cx, &q).unwrap();
        assert_ne!(off, on, "{name}");
        let pick = |v: &serde_json::Value| -> f64 {
            match name {
                "psf" => v["psf"][16 * 32 + 16]["i"].as_f64().unwrap(),
                _ => v["mtf"][0]["tangential"][0].as_f64().unwrap(),
            }
        };
        let (x, y) = (pick(&off), pick(&on));
        assert!(
            y.is_finite() && (x - y).abs() < 0.05 * x.abs().max(1e-3),
            "{name} {x} {y}"
        );
    }
}

/// 視野が軸上だけの系: 相対照度・ケラレ図・グリッド歪曲は「軸外の視野が要る」と返す（横軸の視野の値に意味が無い）。
/// 長方形の正規化で振る向きに視野が無いときも同じ
#[test]
fn field_sweeps_need_a_field_off_the_axis() {
    let mut s = a0();
    s.edit(|p| {
        for f in p.fields.iter_mut() {
            f.x = 0.0;
            f.y = 0.0;
        }
        Ok(())
    })
    .unwrap();
    let cx = crate::registry::Cx::new(&s);
    for name in ["relative-illumination", "vignetting", "grid-distortion"] {
        let a = Analyses::builtin().get(name).unwrap();
        let e = a.run_json(&cx, &json!({})).unwrap_err().to_string();
        assert!(e.contains("needs a field off the axis"), "{name}: {e}");
    }
    // 視野が y だけ: 長方形の正規化で x に振ると視野が無い。半径の正規化なら x にも振れる
    let mut s = a0();
    s.edit(|p| {
        p.field_norm = lensbench_core::system::FieldNorm::Rect;
        for f in p.fields.iter_mut() {
            f.x = 0.0;
        }
        Ok(())
    })
    .unwrap();
    let cx = crate::registry::Cx::new(&s);
    let a = Analyses::builtin().get("relative-illumination").unwrap();
    assert!(a
        .run_json(&cx, &json!({ "axis": "x", "points": 3 }))
        .is_err());
    assert!(a
        .run_json(&cx, &json!({ "axis": "y", "points": 3 }))
        .is_ok());
    s.edit(|p| {
        p.field_norm = lensbench_core::system::FieldNorm::Radial;
        Ok(())
    })
    .unwrap();
    let cx = crate::registry::Cx::new(&s);
    assert!(a
        .run_json(&cx, &json!({ "axis": "x", "points": 3 }))
        .is_ok());
}
