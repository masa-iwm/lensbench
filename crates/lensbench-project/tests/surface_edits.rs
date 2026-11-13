//! ブロックの面の値の上書き（surface_edits の GUI の面の表）

use lensbench_project::{expand, expand_report, Env, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env() -> Env<'static> {
    Env {
        base: root().join("tests/refs"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

fn project(place: &str, edits: &str) -> Result<lensbench_core::System, String> {
    let txt = format!(
        r#"{{ "glass_catalogs": ["SCHOTT"], "wavelengths": [{{"um": 0.5875618}}],
        "aperture": {{"type": "epd", "value": 5}},
        "blocks": [{{"type": "stop", "semi": 2.5}}, {{"type": "air", "t": 2}},
          {{"type": "lens", "id": "L1", {place} "source": {{"surfaces": [
            {{"radius": 50, "t": 4, "glass": "N-BK7", "semi": 10}}, {{"radius": -50, "semi": 10}}]}}}},
          {{"type": "air", "t": 30}}],
        "surface_edits": {edits} }}"#
    );
    expand(&Project::from_json(&txt)?, &env())
}

#[test]
fn edits_count_surfaces_without_coordinate_breaks() {
    let edits = r#"[{"block": "L1", "surface": 1, "fields": {"r": -40, "semi": 8}},
                    {"block": "L1", "surface": 0, "fields": {"material": "F2"}}]"#;
    for place in ["", r#""decenter": [0, 0.5],"#] {
        let s = project(place, edits).unwrap();
        let lens: Vec<usize> = (0..s.surfaces.len())
            .filter(|&j| s.surfaces[j].mat == "F2")
            .collect();
        assert_eq!(lens.len(), 1, "{place}");
        let j = lens[0];
        assert!((s.surfaces[j].c - 1.0 / 50.0).abs() < 1e-15);
        assert!((s.surfaces[j + 1].c + 1.0 / 40.0).abs() < 1e-15, "{place}");
        assert_eq!(s.surfaces[j + 1].semi, 8.0);
    }
    // 無ければ上書きなしと同じ
    let a = project("", "[]").unwrap();
    assert!(a.surfaces.iter().any(|d| d.mat == "N-BK7"));
}

#[test]
fn bad_edits_are_errors() {
    let e = project("", r#"[{"block": "L1", "surface": 2, "fields": {"r": 1}}]"#).unwrap_err();
    assert!(e.contains("面 2"), "{e}");
    let e = project(
        "",
        r#"[{"block": "L1", "surface": 0, "fields": {"zz": 1}}]"#,
    )
    .unwrap_err();
    assert!(e.contains("zz"), "{e}");
}

/// 鏡の back で往復するブロック: 1 つの上書きが往路と復路の同じ面に当たる（復路は逆の順、厚さは符号が逆）
#[test]
fn edits_apply_to_both_passes() {
    let txt = |edits: &str| {
        format!(
            r#"{{ "glass_catalogs": ["SCHOTT"], "wavelengths": [{{"um": 0.5875618}}],
            "aperture": {{"type": "epd", "value": 5}},
            "blocks": [{{"type": "stop", "semi": 2.5}}, {{"type": "air", "t": 2}},
              {{"type": "lens", "id": "L1", "source": {{"surfaces": [
                {{"radius": 50, "t": 4, "glass": "N-BK7", "semi": 10}}, {{"radius": -30, "t": 2, "glass": "F2", "semi": 9}},
                {{"radius": -80, "semi": 8}}]}}}},
              {{"type": "air", "t": 10}}, {{"type": "mirror", "id": "M1", "back": true, "semi": 12}}, {{"type": "air", "t": 20}}],
            "surface_edits": {edits} }}"#
        )
    };
    let run = |edits: &str| expand(&Project::from_json(&txt(edits)).unwrap(), &env());
    let a = run("[]").unwrap();
    // 往路 2 3 4、鏡 5、復路 6 7 8（4 3 2 の順）
    assert_eq!(a.surfaces[7].c, a.surfaces[3].c);
    let s = run(
        r#"[{"block": "L1", "surface": 1, "fields": {"r": -25, "semi": 7}},
            {"block": "L1", "surface": 0, "fields": {"t": 5, "material": "F2"}}]"#,
    )
    .unwrap();
    for j in [3, 7] {
        assert!((s.surfaces[j].c + 1.0 / 25.0).abs() < 1e-15, "面 {j}");
        assert_eq!(s.surfaces[j].semi, 7.0, "面 {j}");
    }
    assert_eq!(s.surfaces[2].t, 5.0);
    assert_eq!(s.surfaces[7].t, -5.0);
    assert_eq!(s.surfaces[2].mat, "F2");
    assert_eq!(s.surfaces[7].mat, "F2");
    // 往路と復路で同じ玉: 復路の面は往路の面を逆にたどった値
    for (f, b) in [(2, 8), (3, 7), (4, 6)] {
        assert_eq!(s.surfaces[f].c, s.surfaces[b].c, "{f} {b}");
        assert_eq!(s.surfaces[f].semi, s.surfaces[b].semi, "{f} {b}");
    }
    // 最後の面の後はブロックの外
    let e = run(r#"[{"block": "L1", "surface": 2, "fields": {"t": 3}}]"#).unwrap_err();
    assert!(e.contains("後の空気"), "{e}");
    // 面の前後の傾き偏心は復路で前後・向きが変わるので断る
    let e = run(r#"[{"block": "L1", "surface": 0, "fields": {"before": {"d": [0, 0.1, 0, 0, 0], "order": 0}}}]"#)
        .unwrap_err();
    assert!(e.contains("傾き偏心"), "{e}");
    // 面の値だけなら最後の面も変えられる
    let s = run(r#"[{"block": "L1", "surface": 2, "fields": {"semi": 6}}]"#).unwrap();
    assert_eq!((s.surfaces[4].semi, s.surfaces[6].semi), (6.0, 6.0));
}

/// 面の値の名前に無い項目は面のデータ（serve の set_surface_data と同じ）として当てる
#[test]
fn data_fields_are_applied() {
    let s = project(
        "",
        r#"[{"block": "L1", "surface": 0, "fields": {"type": "EVENASPH", "asph": [0, 1e-5], "ap": {"Circular": {"min": 0, "max": 6, "dx": 0, "dy": 0}}}}]"#,
    )
    .unwrap();
    let j = s.surfaces.iter().position(|d| d.mat == "N-BK7").unwrap();
    assert_eq!(s.surfaces[j].asph, vec![0.0, 1e-5]);
    assert!(matches!(
        s.surfaces[j].ap,
        lensbench_core::surface::Aperture::Circular { max, .. } if max == 6.0
    ));
    let e = project(
        "",
        r#"[{"block": "L1", "surface": 0, "fields": {"stop": true}}]"#,
    )
    .unwrap_err();
    assert!(e.contains("stop"), "{e}");
}

/// 系に無いブロック（OFF の群の中・状態で外したもの）への上書きは、surface_ops と同じく当てずに注意を出す
#[test]
fn edits_to_missing_blocks_warn() {
    let txt = r#"{ "glass_catalogs": ["SCHOTT"], "wavelengths": [{"um": 0.5875618}],
        "aperture": {"type": "epd", "value": 5},
        "blocks": [{"type": "stop", "semi": 2.5}, {"type": "air", "t": 2},
          {"type": "lens", "id": "L1", "source": {"surfaces": [
            {"radius": 50, "t": 4, "glass": "N-BK7", "semi": 10}, {"radius": -50, "semi": 10}]}},
          {"type": "air", "t": 30}],
        "surface_edits": [{"block": "X", "surface": 0, "fields": {"r": 1}}] }"#;
    let (s, warn) = expand_report(&Project::from_json(txt).unwrap(), &env()).unwrap();
    assert!(warn.iter().any(|w| w.contains("X")), "{warn:?}");
    assert!(s.surfaces.iter().all(|d| d.c != 1.0));
}

/// OFF・空気で残す群の中の id は、後ろの別のブロックの面を指さない（範囲の記録を巻き戻す）
#[test]
fn off_air_group_does_not_leak_spans() {
    let txt = |edits: &str| {
        format!(
            r#"{{ "glass_catalogs": ["SCHOTT"], "wavelengths": [{{"um": 0.5875618}}],
            "aperture": {{"type": "epd", "value": 5}},
            "blocks": [{{"type": "stop", "semi": 2.5}}, {{"type": "air", "t": 10}},
              {{"type": "group", "id": "G", "on": false, "off": "air", "blocks": [
                {{"type": "lens", "id": "L1", "source": {{"surfaces": [
                  {{"radius": 50, "t": 4, "glass": "N-BK7", "semi": 6}}, {{"radius": -50, "semi": 6}}]}}}},
                {{"type": "air", "id": "d1", "t": 3}},
                {{"type": "lens", "id": "L1b", "source": {{"surfaces": [
                  {{"radius": 40, "t": 3, "glass": "N-BK7", "semi": 6}}, {{"radius": -40, "semi": 6}}]}}}}]}},
              {{"type": "air", "t": 5}},
              {{"type": "lens", "id": "L2", "source": {{"surfaces": [
                {{"radius": 60, "t": 4, "glass": "N-BK7", "semi": 6}}, {{"radius": -60, "semi": 6}}]}}}},
              {{"type": "air", "t": 60}}],
            "surface_edits": {edits} }}"#
        )
    };
    let run = |e: &str| expand_report(&Project::from_json(&txt(e)).unwrap(), &env()).unwrap();
    let (a, _) = run("[]");
    let (b, warn) = run(r#"[{"block": "L1", "surface": 0, "fields": {"semi": 2.5}}]"#);
    assert_eq!(a.surfaces.len(), b.surfaces.len());
    for (x, y) in a.surfaces.iter().zip(&b.surfaces) {
        assert_eq!(x.semi, y.semi);
    }
    assert!(warn.iter().any(|w| w.contains("L1")), "{warn:?}");
    // 群の長さ（4 + 3 + 3 = 10）は空気で残る: L2 の前面は 10 + 10 + 5 = 25 の位置
    let z: f64 = a.surfaces[1..a.surfaces.len() - 3]
        .iter()
        .map(|d| d.t)
        .sum();
    assert!((z - 25.0).abs() < 1e-12, "{z}");
    // 関係のない注意（L1b の範囲）を出さない
    assert!(!warn.iter().any(|w| w.contains("L1b")), "{warn:?}");
}
