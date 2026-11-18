//! .zmx の評価関数の行と項の読み書きの試験

use super::*;
use lensbench_formats::zmx::merit::merit_function_text;

/// 面の後に評価関数を置いた .zmx（OpticStudio 26.1 の書き方、Double Gauss 28 で作った行を A0 に合わせたもの）
fn zmx(unit: &str, merit: &[&str]) -> String {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let src = lensbench_core::glass::read_text(&root.join("tests/refs/A0/system.zmx")).unwrap();
    let mut out = String::new();
    for l in src.lines() {
        // A0 の空の評価関数（BLNK 1 行）は除く
        if l.starts_with("BLNK") {
            continue;
        }
        if l.starts_with("TOL ") {
            for m in merit {
                out += m;
                out += "\n";
            }
        }
        out += &l.replace("UNIT MM", &format!("UNIT {unit}"));
        out += "\n";
    }
    out
}

#[test]
fn rows_to_terms() {
    let rows = [
        "DMFS 0 0 0 0 0 0 0 0 0 0",
        "BLNK comment",
        "EFFL 0 2 0 0 0 0 100 1 0 0",
        "PMAG 0 1 0 0 0 0 100 1 0 0",
        "EXPD 0 0 0 0 0 0 25 0 0 0",
        "RSCE 3 0 0 0.69999999999999996 0 0 0 1 0 0",
        "RSCH 2 1 0 1 0 0 0.001 2 0 0",
        "OPDC 0 2 0 1 0.29999999999999999 0.69999999999999996 0 1.2 0 0",
        "ZERN 11 2 2 2 1 0 0.01 0.80000000000000004 0 0",
        "ZERN 4 1 3 1 2 0.29999999999999999 0.20000000000000001 1.5 1 0",
        "MTFS 2 0 3 50 0 0 0.40000000000000002 2 0 0",
        "REAY 5 1 0 0.5 0 -1 -5 0.29999999999999999 0 0",
        "CENY 9 1 3 0 5 0 14 0.59999999999999998 0 0",
        "EFFL 0 2 0 0 0 0 90 0 0 0",
        "OPGT 14 0 0 0 0 0 90 2 0 0",
        "OPLT 6 0 0 0 0 0 1 1 0 0",
        "CTGT 2 2 0 0 0 0 1.5 3 0 0",
        "TOTR 0 0 0 0 0 0 80 -1 0 0",
        "CONF 2 0 0 0 0 0 0 0 0 0",
        "WFNO 0 0 0 0 0 0 4 0.69999999999999996 0 0",
        "wfno 0 0 0 0 0 0 4 1 0 0",
    ];
    let mf = merit_function_text(&zmx("MM", &rows)).unwrap();
    let z = terms_from_zmx(&mf, 2, 9, &ZmxSys::default());
    let want = vec![
        json!({"type": "paraxial", "q": "efl", "target": 100.0, "weight": 1.0}),
        json!({"type": "spot", "rings": 3, "wave": null, "field": [0.0, 0.7], "reference": "centroid", "quadrature": "opticstudio", "weight": 1.0}),
        json!({"type": "opd", "wave": 2, "field": [0.0, 1.0], "px": 0.3, "py": 0.7, "target": 0.0, "weight": 1.2}),
        json!({"type": "zernike", "term": 11, "wave": 2, "n": 64, "field": 2, "kind": "standard", "fit": 11, "target": 0.01, "weight": 0.8}),
        json!({"type": "mtf", "wave": null, "field": 3, "freq": 50.0, "dir": "sag", "target": 0.4, "weight": 2.0}),
        json!({"type": "ray", "surf": 5, "wave": 1, "field": [0.0, 0.5], "px": 0.0, "py": -1.0, "q": "y", "target": -5.0, "weight": 0.3}),
        json!({"type": "centroid", "wave": 1, "field": 3, "q": "y", "target": 14.0, "weight": 0.6}),
        json!({"type": "paraxial", "q": "efl", "min": 90.0, "weight": 2.0}),
        json!({"type": "thickness", "surf": 2, "min": 1.5, "weight": 3.0}),
        json!({"type": "paraxial", "q": "wfno", "target": 4.0, "weight": 0.7, "config": 2}),
    ];
    assert_eq!(z.terms, want);
    assert_eq!(z.configs, 2);
    let skipped: Vec<usize> = z
        .notes
        .iter()
        .map(|n| n.split_whitespace().nth(4).unwrap().parse().unwrap())
        .collect();
    // PMAG の波長 1（主波長でない）・RSCH の目標・Annular の ZERN・目標のない項への OPLT・負の重み・読めない行
    assert_eq!(skipped, [4, 7, 10, 16, 18, 21], "{:?}", z.notes);
    // 長さの目標はレンズ単位から mm に、MTF はレンズ単位が mm のときだけ
    let mf = merit_function_text(&zmx("IN", &rows)).unwrap();
    let z = terms_from_zmx(&mf, 2, 9, &ZmxSys::default());
    assert_eq!(z.terms[0]["target"], json!(2540.0));
    // 長さの項の重みは単位の 2 乗で割る（OpticStudio と同じ釣り合い）
    assert_eq!(z.terms[0]["weight"], json!(1.0 / (25.4 * 25.4)));
    let ray = z.terms.iter().find(|t| t["type"] == "ray").unwrap();
    assert_eq!(ray["target"], json!(-127.0));
    assert!(z.terms.iter().all(|t| t["type"] != "mtf"));
    let i = z.terms.iter().position(|t| t.get("min").is_some()).unwrap();
    assert_eq!(z.terms[i]["min"], json!(90.0 * 25.4));
}

#[test]
fn wizard_rows_become_terms() {
    let rows = [
        "DMFS 0 0 0 0 0 0 0 0 0 0",
        "BLNK Sequential merit function: RMS spot x+y chief X Wgt = 1.0000 Y Wgt = 1.0000 RA 2 x 2 rays",
        "BLNK Operands for field 1.",
        "TRAD 0 1 0 0 0.5 0.5 0 0.5 0 0",
        "TRAE 0 1 0 0 0.5 0.5 0 0.5 0 0",
        "TRAD 0 2 0 0 0.5 0.5 0 0.25 0 0",
        "TRAE 0 2 0 0 0.5 0.5 0 0.25 0 0",
        "BLNK Operands for field 2.",
        "TRAD 0 1 0 1 0.5 -0.5 0 0.0625 0 0",
        "TRAE 0 1 0 1 0.5 -0.5 0 0.0625 0 0",
        "TRAD 0 1 0 1 0.5 0.5 0 0.0625 0 0",
        "TRAE 0 1 0 1 0.5 0.5 0 0.0625 0 0",
        "BLNK Operands for field 3.",
        "TRAD 0 1 0 0.5 0.5 0.5 0 0.1 0 0",
        "EFFL 0 2 0 0 0 0 100 1 0 0",
        "TRAD 0 1 0 1 0.25 0.75 0 1 0 0",
        "DMFS 0 0 0 0 0 0 0 0 0 0",
        "BLNK Sequential merit function: RMS wavefront centroid GQ 1 rings 2 arms",
        "BLNK Operands for field 1.",
        "PRIM 0 1 0 0 0 0 0 0 0 0",
        "OPDX 0 1 0 0 0.70710678118654757 0 0 0.75 0 0",
        "DMFS 0 0 0 0 0 0 0 0 0 0",
        "BLNK Sequential merit function: PTV spot radius chief",
    ];
    let mf = merit_function_text(&zmx("MM", &rows)).unwrap();
    let z = terms_from_zmx(&mf, 2, 9, &ZmxSys::default());
    let spot = |f: [f64; 2], axis: &str, w: f64| json!({"type": "spot", "field": f, "wave": null, "reference": "chief", "quadrature": "grid", "n": 2, "axis": axis, "weight": w});
    let want = vec![
        spot([0.0, 0.0], "x", 0.75),
        spot([0.0, 0.0], "y", 0.75),
        spot([0.0, 1.0], "x", 0.125),
        spot([0.0, 1.0], "y", 0.125),
        json!({"type": "paraxial", "q": "efl", "target": 100.0, "weight": 1.0}),
        json!({"type": "wavefront", "field": [0.0, 0.0], "wave": 1, "reference": "centroid", "quadrature": "opticstudio", "rings": 1, "arms": 2, "weight": 0.75}),
    ];
    assert_eq!(z.terms, want);
    assert_eq!(z.sources, [4, 5, 9, 10, 15, 21]);
    // 軸上（4 分の 1）・瞳の半分のどちらでもない視野 3 の 1 行と、視野の節の外の TRAD（行 16）は 1 行ずつ読んで飛ばし、
    // RMS でない Wizard は注意にする
    assert_eq!(z.notes.len(), 4, "{:?}", z.notes);
    assert!(z.notes[0].contains("row 14:") && z.notes[0].contains("pupil sampling"));
    assert!(z.notes[1].contains("row 23") && z.notes[1].contains("PTV"));
    assert!(z.notes[2].contains("row 14 (TRAD"));
    assert!(z.notes[3].contains("row 16 (TRAD"));
}

#[test]
fn more_calc_rows_become_terms() {
    let rows = [
        "EFFL 0 0 0 0 0 0 0 0 0 0",
        "CONS 0 0 0 0 0 0 -2 0 0 0",
        "MAXX 1 2 0 0 0 0 0 1 0 0",
        "MINN 2 1 0 0 0 0 0 1 0 0",
        "QSUM 2 1 0 0 0 0 0 1 0 0",
        "SINE 2 1 0 0 0 0 0 1 0 0",
        "ATAN 2 1 0 0 0 0 0 1 0 0",
        "OPVA 1 0 0 0 0 0 90 1 0 0",
        "ABGT 2 0 0 0 0 0 1 1 0 0",
    ];
    let mf = merit_function_text(&zmx("MM", &rows)).unwrap();
    let z = terms_from_zmx(&mf, 2, 9, &ZmxSys::default());
    assert!(z.notes.is_empty(), "{:?}", z.notes);
    let efl = json!({"type": "paraxial", "q": "efl", "target": 0.0});
    let k = std::f64::consts::PI / 180.0;
    let calc = |op: &str, args: Value, t: f64| json!({"type": "calc", "op": op, "args": args, "target": t, "weight": 1.0});
    let want = vec![
        calc("max", json!([efl, -2.0]), 0.0),
        // a > b なら行 a だけ（MAXX・MINN）、和は 0（QSUM）
        calc("min", json!([-2.0]), 0.0),
        calc("qsum", json!([0.0]), 0.0),
        calc(
            "sin",
            json!([{"type": "calc", "op": "prod", "args": [-2.0, k], "target": 0.0}]),
            0.0,
        ),
        calc(
            "prod",
            json!([{"type": "calc", "op": "atan", "args": [-2.0], "target": 0.0}, 1.0 / k]),
            0.0,
        ),
        calc("sum", json!([efl]), 90.0),
        json!({"type": "calc", "op": "abs", "args": [-2.0], "min": 1.0, "weight": 1.0}),
    ];
    assert_eq!(z.terms, want);
}

#[test]
fn calc_rows_stop_growing() {
    // 前の 2 行を足す行を重ねると、入れ子の行の数は倍々に増える。上限を超えた行は注意にする
    let mut rows = vec![
        "CONS 0 0 0 0 0 0 1 0 0 0".to_string(),
        "CONS 0 0 0 0 0 0 1 0 0 0".to_string(),
    ];
    for k in 3..=30 {
        rows.push(format!("SUMM {} {} 0 0 0 0 0 1 0 0", k - 1, k - 2));
    }
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    let mf = merit_function_text(&zmx("MM", &rows)).unwrap();
    let z = terms_from_zmx(&mf, 2, 9, &ZmxSys::default());
    assert!(
        !z.notes.is_empty() && z.notes.iter().all(|n| n.contains("too deeply")),
        "{:?}",
        z.notes
    );
    assert!(z.terms.len() >= 10, "{}", z.terms.len());
    assert!(serde_json::to_string(&z.terms).unwrap().len() < 2_000_000);
}

#[test]
fn calc_rows_become_terms() {
    let rows = [
        "EFFL 0 2 0 0 0 0 100 0 0 0",
        "CONS 0 0 0 0 0 0 5 0 0 0",
        "BLNK",
        "DIFF 1 2 0 0 0 0 0.5 1 0 0",
        "OSUM 1 3 0 0 0 0 0 1 0 0",
        "CTGT 2 0 0 0 0 0 1 0 0 0",
        "ABSO 6 0 0 0 0 0 0 1 0 0",
        "ABSO 9 0 0 0 0 0 0 1 0 0",
        "ABSO 4 0 0 0 0 0 0 1 0 0",
        "OPGT 4 0 0 0 0 0 1 2 0 0",
    ];
    let mf = merit_function_text(&zmx("MM", &rows)).unwrap();
    let z = terms_from_zmx(&mf, 2, 9, &ZmxSys::default());
    let efl = json!({"type": "paraxial", "q": "efl", "target": 100.0});
    let diff = json!({"type": "calc", "op": "diff", "args": [efl, 5.0], "target": 0.5});
    let mut want = vec![
        diff.clone(),
        json!({"type": "calc", "op": "sum", "args": [0.0, efl, 5.0, 0.0], "target": 0.0}),
        // 後ろの行の値は 0
        json!({"type": "calc", "op": "abs", "args": [0.0], "target": 0.0}),
        json!({"type": "calc", "op": "abs", "args": [diff], "target": 0.0}),
        json!({"type": "calc", "op": "diff", "args": [efl, 5.0], "min": 1.0}),
    ];
    for (t, w) in want.iter_mut().zip([1.0, 1.0, 1.0, 1.0, 2.0]) {
        t["weight"] = json!(w);
    }
    assert_eq!(z.terms, want);
    // 片側の条件（CTGT）を参照する ABSO は飛ばす
    assert_eq!(z.notes.len(), 1, "{:?}", z.notes);
    assert!(z.notes[0].contains("row 7 (ABSO") && z.notes[0].contains("bound"));
    // 項として読める
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let s = lensbench_formats::zmx::load_with(
        &root.join("tests/refs/A0/system.zmx"),
        &lensbench_formats::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    let t =
        crate::merit::terms_from_json(&Value::Array(z.terms), std::slice::from_ref(&s)).unwrap();
    let mut p = lensbench_optim::Problem::single(s.clone());
    p.terms = t;
    p.check().unwrap();
    let v = p.term_values(&[]).unwrap();
    let f = s.paraxial().efl;
    assert!((v[0].value() - (f - 5.0)).abs() < 1e-12);
    assert!((v[1].value() - (f + 5.0)).abs() < 1e-12);
    assert!((v[3].value() - (f - 5.0).abs()).abs() < 1e-12);
}

#[test]
fn terms_can_be_read() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let s = lensbench_formats::zmx::load_with(
        &root.join("tests/refs/A0/system.zmx"),
        &lensbench_formats::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    let rows = [
        "EFFL 0 0 0 0 0 0 50 1 0 0",
        "RWCE 3 0 0 0.69999999999999996 0 0 0 0.5 0 0",
        "ZERN 9 0 1 1 0 0 0 1 0 0",
        "MTFT 2 0 1 30 0 0 0.6 1 0 0",
        "REAX 9 0 0 1 0 1 0 1 0 0",
        "CENX 9 0 1 0 5 0 0 1 0 0",
        "OPLT 1 0 0 0 0 0 60 1 0 0",
    ];
    let mf = merit_function_text(&zmx("MM", &rows)).unwrap();
    let z = terms_from_zmx(&mf, s.primary + 1, s.image(), &ZmxSys::of(&s));
    assert!(z.notes.is_empty(), "{:?}", z.notes);
    let t =
        crate::merit::terms_from_json(&Value::Array(z.terms), std::slice::from_ref(&s)).unwrap();
    let mut p = lensbench_optim::Problem::single(s.clone());
    p.terms = t;
    p.check().unwrap();
    let v = p.term_values(&[]).unwrap();
    assert_eq!(v.len(), 7);
    assert_eq!(v[0].value(), s.paraxial().efl);
    // Wizard の行をまとめた項（格子・腕・x と y）
    let rows = [
        "DMFS 0 0 0 0 0 0 0 0 0 0",
        "BLNK Sequential merit function: RMS spot x+y chief X Wgt = 1.0000 Y Wgt = 1.0000 RA 2 x 2 rays",
        "BLNK Operands for field 1.",
        "TRAD 0 1 0 1 0.5 0.5 0 1 0 0",
        "TRAE 0 1 0 1 0.5 0.5 0 1 0 0",
        "TRAD 0 1 0 1 0.5 -0.5 0 1 0 0",
        "TRAE 0 1 0 1 0.5 -0.5 0 1 0 0",
        "DMFS 0 0 0 0 0 0 0 0 0 0",
        "BLNK Sequential merit function: RMS wavefront centroid GQ 1 rings 2 arms",
        "BLNK Operands for field 1.",
        "OPDX 0 1 0 1 0.70710678118654757 0 0 1 0 0",
    ];
    let mf = merit_function_text(&zmx("MM", &rows)).unwrap();
    let z = terms_from_zmx(&mf, s.primary + 1, s.image(), &ZmxSys::of(&s));
    assert!(z.notes.is_empty(), "{:?}", z.notes);
    let t =
        crate::merit::terms_from_json(&Value::Array(z.terms), std::slice::from_ref(&s)).unwrap();
    p.terms = t;
    p.check().unwrap();
    let v = p.term_values(&[]).unwrap();
    assert_eq!(v.len(), 3);
    // x と y の 2 乗の和はスポット半径の 2 乗
    let r = |axis: Option<&str>| {
        let mut j = json!({"type": "spot", "field": [0.0, 1.0], "reference": "chief", "quadrature": "grid", "n": 4});
        if let Some(a) = axis {
            j["axis"] = json!(a);
        }
        let mut p = lensbench_optim::Problem::single(s.clone());
        p.terms = crate::merit::terms_from_json(&j, std::slice::from_ref(&s)).unwrap();
        p.term_values(&[]).unwrap()[0].value()
    };
    let (x, y, xy) = (r(Some("x")), r(Some("y")), r(None));
    assert!(
        (x * x + y * y - xy * xy).abs() < 1e-15 * xy * xy,
        "{x} {y} {xy}"
    );
}

/// 項 → 行 → 項で同じ項に戻る（表せない項は注意）
#[test]
fn terms_to_rows_and_back() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = lensbench_formats::zmx::LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let s =
        lensbench_formats::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap();
    let terms = vec![
        json!({"type": "paraxial", "q": "efl", "target": 50.0, "weight": 2.0}),
        json!({"type": "spot", "rings": 4, "wave": null, "field": [0.0, 0.5], "reference": "chief", "quadrature": "opticstudio", "weight": 1.0}),
        json!({"type": "opd", "wave": 1, "field": [0.0, 1.0], "px": 0.3, "py": 0.7, "target": 0.0, "weight": 1.0}),
        json!({"type": "zernike", "term": 9, "wave": 1, "n": 128, "field": 1, "kind": "fringe", "fit": 37, "target": 0.1, "weight": 1.0}),
        json!({"type": "mtf", "wave": null, "field": 1, "freq": 30.0, "dir": "tan", "target": 0.5, "weight": 1.0}),
        json!({"type": "ray", "surf": 3, "wave": 1, "field": [0.0, 1.0], "px": 0.0, "py": 1.0, "q": "m", "target": 0.0, "weight": 1.0}),
        json!({"type": "paraxial", "q": "totr", "max": 60.0, "weight": 4.0}),
        json!({"type": "paraxial", "q": "pimh", "wave": 3, "target": 20.0, "weight": 1.0}),
        json!({"type": "ray", "wave": 1, "field": [0.0, 1.0], "px": 0.5, "py": 0.5, "q": "trar", "target": 0.0, "weight": 1.0}),
        json!({"type": "ray", "surf": 4, "wave": 1, "field": [0.0, 1.0], "px": 0.0, "py": 1.0, "q": "opth", "target": 50.0, "weight": 1.0}),
        json!({"type": "ray", "surf": 5, "wave": 2, "field": [0.0, 1.0], "px": 0.3, "py": -0.7, "q": "aoi", "target": 10.0, "weight": 1.0}),
        json!({"type": "ray", "surf": 6, "wave": 1, "field": [0.0, 0.5], "px": 0.0, "py": 1.0, "q": "gz", "target": 40.0, "weight": 1.0}),
        json!({"type": "ray", "surf": 0, "to": 6, "field": [0.0, 1.0], "px": 0.0, "py": 1.0, "q": "opth", "target": 60.0, "weight": 1.0}),
        json!({"type": "ray", "surf": 1, "to": 5, "wave": 2, "field": [0.0, 0.7], "px": 0.0, "py": 1.0, "q": "aoe", "max": 20.0, "weight": 1.0}),
        json!({"type": "ray", "surf": 2, "to": 4, "wave": 1, "field": [0.0, 0.0], "px": 0.5, "py": 0.5, "q": "aoi", "min": 1.0, "weight": 1.0}),
        json!({"type": "ray", "paraxial": true, "surf": 4, "wave": 1, "field": [0.0, 1.0], "px": 0.0, "py": 1.0, "q": "nm", "target": 0.1, "weight": 1.0}),
        json!({"type": "paraxial", "q": "linv", "wave": 3, "target": 4.0, "weight": 1.0}),
        json!({"type": "paraxial", "q": "petz", "target": -600.0, "weight": 1.0}),
        json!({"type": "surface", "surf": 2, "q": "power", "wave": 1, "target": 0.01, "weight": 1.0}),
        json!({"type": "surface", "surf": 3, "q": "index", "wave": 3, "target": 1.6, "weight": 1.0}),
        json!({"type": "cardinal", "q": "efl", "from": 1, "to": 3, "wave": null, "target": 40.0, "weight": 1.0}),
        json!({"type": "cardinal", "q": "principal_image", "from": 2, "to": 5, "wave": 3, "min": -10.0, "weight": 1.0}),
        json!({"type": "fnum", "field": 2, "wave": 1, "dir": "sag", "target": 3.0, "weight": 1.0}),
        json!({"type": "fnum", "field": 1, "wave": 3, "dir": "tan", "max": 4.0, "weight": 1.0}),
        json!({"type": "thickness", "surf": 3, "min": 1.0, "weight": 1.0}),
        json!({"type": "thickness", "surf": 2, "to": 5, "target": 20.0, "weight": 0.5}),
        json!({"type": "thickness", "surf": 4, "target": 2.0, "weight": 1.0}),
        json!({"type": "thickness", "surf": 1, "edge": true, "max": 3.0, "weight": 1.0}),
        json!({"type": "thickness", "surf": 1, "to": 5, "each": "air", "edge": true, "zone": 0.5, "min": 0.5, "weight": 1.0}),
        json!({"type": "thickness", "surf": 1, "to": 5, "each": "glass", "max": 10.0, "weight": 1.0}),
        json!({"type": "thickness", "surf": 1, "to": 5, "each": "all", "edge": true, "max": 10.0, "weight": 1.0}),
        json!({"type": "surface", "surf": 2, "q": "curv", "min": 0.01, "weight": 1.0}),
        json!({"type": "surface", "surf": 2, "q": "conic", "target": -1.0, "weight": 1.0}),
        json!({"type": "surface", "surf": 1, "q": "sag", "x": 3.0, "y": 4.0, "target": 0.2, "weight": 1.0}),
        json!({"type": "surface", "surf": 1, "q": "sagy", "target": 0.0, "weight": 1.0}),
        json!({"type": "surface", "surf": 1, "q": "thick", "x": 0.0, "y": 5.0, "target": 8.0, "weight": 1.0}),
        json!({"type": "surface", "surf": 3, "q": "diam", "max": 40.0, "weight": 1.0}),
        json!({"type": "surface", "surf": 3, "q": "mech_diam", "target": 50.0, "weight": 1.0}),
        json!({"type": "surface", "surf": 4, "q": "gz", "target": 30.0, "weight": 1.0}),
        json!({"type": "thickness", "surf": 1, "to": 3, "each": "air", "edge": true, "max": 3.0, "weight": 1.0}),
        json!({"type": "centroid", "wave": null, "field": 1, "q": "x", "target": 0.0, "weight": 1.0, "config": 2}),
    ];
    let bad = [
        json!({"type": "paraxial", "q": "pwfn", "target": 1.0}),
        json!({"type": "mtf", "field": [0.0, 1.0], "freq": 30.0}),
        json!({"type": "coupling"}),
        json!({"type": "thickness", "surf": 1, "to": 3, "edge": true, "target": 3.0}),
        json!({"type": "ray", "surf": 3, "field": 1, "q": "trax"}),
        json!({"type": "ray", "surf": 1, "to": 3, "wave": 3, "field": 1, "q": "opth"}),
        json!({"type": "ray", "surf": 1, "to": 3, "field": 1, "q": "x"}),
        json!({"type": "cardinal", "q": "efl", "wave": 1, "target": 40.0}),
    ];
    let all: Vec<Value> = terms.iter().chain(&bad).cloned().collect();
    let systems = [s.clone(), s.clone()];
    let before = [MeritRow::blank("from the zmx")];
    let (rows, notes) = rows_from_terms(&all, &systems, &before);
    assert_eq!(notes.len(), 8, "{notes:?}");
    // 行を .zmx に書いて読み直す
    let mut txt = String::new();
    for r in before.iter().chain(&rows) {
        txt += &r.text();
        txt += "\n";
    }
    let z = zmx("MM", &txt.lines().collect::<Vec<_>>());
    let mf = merit_function_text(&z).unwrap();
    assert_eq!(mf.rows.len(), rows.len() + 1);
    let back = terms_from_zmx(&mf, s.primary + 1, s.image(), &ZmxSys::of(&s));
    assert!(back.notes.is_empty(), "{:?}", back.notes);
    // MTF の標本は渡さない
    let mut want = terms.clone();
    want[4].as_object_mut().unwrap().remove("n");
    assert_eq!(back.terms, want);
}

/// Standard の ZERN は隣り合う ZERN の行（Term のほかが同じ）の一番大きい Term と 11 の大きい方で当てはめる（OpticStudio 26.1）。
/// lensbench の項から書く行は、当てはめる項の数・瞳の求積が OpticStudio と違えば注意にする
#[test]
fn zern_fit_follows_opticstudio() {
    let rows = [
        "ZERN 4 2 2 3 1 0 0 1 0 0",
        "ZERN 22 2 2 3 1 0 0.5 1 0 0",
        "ZERN 5 2 2 2 1 0 0 1 0 0",
        "ZERN 4 2 2 3 0 0 0 1 0 0",
    ];
    let mf = merit_function_text(&zmx("MM", &rows)).unwrap();
    let z = terms_from_zmx(&mf, 2, 9, &ZmxSys::default());
    let fit: Vec<_> = z.terms.iter().map(|t| t["fit"].as_u64().unwrap()).collect();
    assert_eq!(fit, [22, 22, 11, 37]);
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let s = lensbench_formats::zmx::load_with(
        &root.join("tests/refs/A0/system.zmx"),
        &lensbench_formats::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    let t = |fit: Option<u64>| {
        let mut v = json!({"type": "zernike", "term": 5, "field": 1, "kind": "standard"});
        if let Some(f) = fit {
            v["fit"] = json!(f);
        }
        v
    };
    let spot = |q: &str| json!({"type": "spot", "field": [0.0, 1.0], "quadrature": q});
    let all = [t(Some(11)), t(None), spot("opticstudio"), spot("lensbench")];
    let (rows, notes) = rows_from_terms(&all, &[s], &[]);
    assert_eq!(rows.len(), 4);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].starts_with("term 2:"), "{notes:?}");
    assert!(notes[1].starts_with("term 4:"), "{notes:?}");
}

/// EFLA は空気中の単レンズ（面 Surf の前が空気、後がガラス、Surf + 1 の後が空気）だけ読む
#[test]
fn efla_single_lens_only() {
    // 面 1・2 が単レンズ、面 3〜5 が 2 枚の接合
    let sys = ZmxSys {
        glass: vec![false, true, false, true, true, false, false, false],
        symmetric: true,
    };
    let rows = [
        "EFLA 1 0 0 0 0 0 0 1 0 0",
        "EFLA 3 0 0 0 0 0 0 1 0 0",
        "EFLA 4 0 0 0 0 0 0 1 0 0",
        "EFLA 2 0 0 0 0 0 0 1 0 0",
    ];
    let mf = merit_function_text(&zmx("MM", &rows)).unwrap();
    let z = terms_from_zmx(&mf, 1, 7, &sys);
    assert_eq!(z.terms.len(), 1, "{:?}", z.notes);
    assert_eq!(z.terms[0]["from"], 1);
    assert_eq!(z.terms[0]["to"], 2);
    assert_eq!(z.notes.len(), 3, "{:?}", z.notes);
}
