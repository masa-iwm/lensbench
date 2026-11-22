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
            "ray-fan" | "opd-fan" | "pupil-fan" => back::<FanOut>(&t, &v),
            "field-curvature" => back::<FieldCurvOut>(&t, &v),
            "longitudinal" => back::<LongOut>(&t, &v),
            "lateral-color" => back::<LatOut>(&t, &v),
            "focal-shift" => back::<ShiftOut>(&t, &v),
            "grid-distortion" => back::<GridOut>(&t, &v),
            "seidel" => back::<SeidelOut>(&t, &v),
            "footprint" => back::<FootprintOut>(&t, &v),
            "relative-illumination" => back::<RelIllumOut>(&t, &v),
            "vignetting" => back::<VignettingOut>(&t, &v),
            "gaussian-beam" => back::<GaussOut>(&t, &v),
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

/// 膜を使う解析（polarization の psf・mtf・huygens-mtf・相対照度、ghost）は、系の膜の注意（登録した一覧の膜が膜ファイルの
/// 同じ名前の膜より先に使われた、など）を結果の warnings に載せる。polarization なしでは載せない
#[test]
fn coating_analyses_carry_the_coating_notes() {
    use lensbench_core::usercoating::CoatingSet;
    let mut s = a0();
    let d = std::env::temp_dir().join(format!("lensbench-coatnotes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("COATING.DAT"),
        "MATE MGF2X\n0.4 1.38 0\n0.8 1.38 0\nCOAT EXT_AR\nMGF2X 0.1\n",
    )
    .unwrap();
    s.edit_raw(|p| {
        p.surfaces[1].coat = "EXT_AR".into();
        p.coating_file = Some(d.join("COATING.DAT"));
        Ok(())
    })
    .unwrap();
    let mut e = lensbench_core::ext::Ext::default();
    e.add_coating_catalog(
        "EXAMPLE",
        CoatingSet::parse_dat("MATE MGF2X\n0.4 1.38 0\n0.8 1.38 0\nCOAT EXT_AR\nMGF2X 0.25\n")
            .unwrap(),
    );
    lensbench_core::ext::with(std::sync::Arc::new(e), || {
        let cx = crate::registry::Cx::new(&s);
        let a = Analyses::builtin();
        let noted = |v: &serde_json::Value| {
            v["warnings"].as_array().is_some_and(|w| {
                w.iter().any(|x| {
                    x.as_str()
                        .is_some_and(|x| x.starts_with("coating EXT_AR:") && x.contains("EXAMPLE"))
                })
            })
        };
        for (name, p) in [
            ("psf", json!({ "field": 1, "image": 32 })),
            ("mtf", json!({ "field": 1, "freqs": [20.0] })),
            (
                "huygens-mtf",
                json!({ "field": 1, "image": 32, "freqs": [20.0] }),
            ),
            ("relative-illumination", json!({ "points": 3 })),
        ] {
            let off = a.get(name).unwrap().run_json(&cx, &p).unwrap();
            assert!(!noted(&off), "{name}: {off}");
            let mut q = p.clone();
            q["polarization"] = json!(true);
            let on = a.get(name).unwrap().run_json(&cx, &q).unwrap();
            assert!(noted(&on), "{name}: {}", on["warnings"]);
        }
        let g = a.get("ghost").unwrap().run_json(&cx, &json!({})).unwrap();
        assert!(noted(&g), "{}", g["warnings"]);
    });
}

/// 系の入射偏光（input_pol）は polarization の解析に入る: x と y の直線偏光で値が違い、偏光なしは既定と同じ。
/// 相対照度の透過率も同じ
#[test]
fn system_input_polarization_is_used() {
    let base = a0();
    let with = |unpolarized: bool, jx: f64, jy: f64| {
        let mut s = base.clone();
        s.edit(|p| {
            p.input_pol = lensbench_core::system::InputPolarization {
                unpolarized,
                jx,
                jy,
                ..Default::default()
            };
            Ok(())
        })
        .unwrap();
        s
    };
    let a = Analyses::builtin();
    let run = |s: &lensbench_core::System, name: &str, p: serde_json::Value| {
        let cx = crate::registry::Cx::new(s);
        a.get(name).unwrap().run_json(&cx, &p).unwrap()
    };
    for (name, p) in [
        (
            "mtf",
            json!({ "field": 2, "freqs": [20.0, 40.0], "polarization": true }),
        ),
        (
            "relative-illumination",
            json!({ "points": 3, "polarization": true }),
        ),
    ] {
        let def = run(&base, name, p.clone());
        // 偏光なしの印があれば振幅は使わない
        assert_eq!(run(&with(true, 1.0, 0.0), name, p.clone()), def, "{name}");
        let x = run(&with(false, 1.0, 0.0), name, p.clone());
        let y = run(&with(false, 0.0, 2.0), name, p.clone());
        assert_ne!(x, y, "{name}");
        assert_ne!(x, def, "{name}");
    }
    // 振幅が両方 0 は誤り
    let e = a
        .get("mtf")
        .unwrap()
        .run_json(
            &crate::registry::Cx::new(&with(false, 0.0, 0.0)),
            &json!({ "field": 2, "polarization": true }),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("must not be zero"), "{e}");
}
