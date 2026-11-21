use std::process::Command;

fn run(args: &[&str]) -> (bool, String, String) {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
        .current_dir(root)
        .args(args)
        .env_remove("LENSBENCH_GLASSCAT")
        .output()
        .unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

const ZMX: &str = "tests/refs/A0/system.zmx";

#[test]
fn info_json() {
    let (ok, out, _) = run(&["info", ZMX, "--glass-dir", "tests/glass", "--json"]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let effl = v["effl"].as_f64().unwrap();
    // OpticStudio の EFFL（基準の ops.json）と一致する
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let ops: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{root}/tests/refs/A0/ops.json")).unwrap(),
    )
    .unwrap();
    let os = ops["ops"]["EFFL"].as_f64().unwrap();
    assert!((effl - os).abs() < 1e-9, "{effl} {os}");
}

#[test]
fn spot_and_zernike() {
    let (ok, out, _) = run(&[
        "spot",
        ZMX,
        "--glass-dir",
        "tests/glass",
        "--field",
        "2",
        "--json",
    ]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["spot"].as_array().unwrap().len(), 1);
    let (ok, out, _) = run(&[
        "zernike",
        ZMX,
        "--glass-dir",
        "tests/glass",
        "--terms",
        "16",
        "--json",
    ]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["zernike"].as_array().unwrap().len(), 4);
    assert_eq!(v["zernike"][0]["coef"].as_array().unwrap().len(), 16);
}

#[test]
fn errors() {
    // ガラスのフォルダが無くても同梱の SCHOTT で開き、同梱から取ったガラスを知らせる
    let (ok, _, err) = run(&["spot", ZMX]);
    assert!(
        ok && err.contains("N-SK16") && err.contains("built-in catalog"),
        "{err}"
    );
    let (ok, _, err) = run(&["spot", ZMX, "--glass-dir", "tests/glass", "--field", "9"]);
    assert!(!ok && err.contains("field 9"), "{err}");
}

#[test]
fn cfg_out_of_range() {
    let (ok, _, err) = run(&["info", ZMX, "--glass-dir", "tests/glass", "--cfg", "2"]);
    assert!(!ok && err.contains("configuration 2"), "{err}");
}

#[test]
fn small_grid_is_an_error() {
    for cmd in ["zernike", "wavefront"] {
        let (ok, _, err) = run(&[cmd, ZMX, "--glass-dir", "tests/glass", "--n", "3"]);
        assert!(!ok && err.contains("even number 4 or more"), "{cmd}: {err}");
    }
}

/// 非対称系（座標ブレーク・傾き）の視野にケラレ係数があっても、射出瞳の算出で落ちない
#[test]
fn vignetting_factors_in_a_tilted_system() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let raw = std::fs::read(format!("{root}/tests/refs/C0/system.zmx")).unwrap();
    let u16s: Vec<u16> = raw[2..]
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let txt = String::from_utf16(&u16s).unwrap();
    assert!(txt.contains("VDYN 0 0 0 0"));
    let txt = txt.replacen("VDYN 0 0 0 0", "VDYN 0 0.1 0 0", 1);
    let dir = std::env::temp_dir().join(format!("lensbench-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("c0_vig.zmx");
    std::fs::write(&p, txt).unwrap();
    let glass = format!("{root}/tests/glass");
    let (ok, out, err) = run(&[
        "wavefront",
        p.to_str().unwrap(),
        "--glass-dir",
        &glass,
        "--json",
    ]);
    std::fs::remove_dir_all(&dir).ok();
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["wavefront"][0]["rms"].as_f64().unwrap().is_finite());
}

/// 無限遠物体で面 1 が無視でも追跡できる（光路長の基準は次の有効な面）。
/// 無視の書き方は OpticStudio と同じ HIDE の 6 番目の値（OpticStudio 2026 R1 で確認）
#[test]
fn first_surface_ignored() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let raw = std::fs::read(format!("{root}/tests/refs/A0/system.zmx")).unwrap();
    let u16s: Vec<u16> = raw[2..]
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let txt = String::from_utf16(&u16s).unwrap();
    assert!(txt.contains("SURF 1\r\n"));
    let txt = txt.replacen(
        "SURF 1\r\n",
        "SURF 1\r\n  HIDE 0 0 0 0 0 1 0 0 0 0 0 0 0\r\n",
        1,
    );
    let dir = std::env::temp_dir().join(format!("lensbench-cli-ignr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("a0_ignr1.zmx");
    std::fs::write(&p, txt).unwrap();
    let glass = format!("{root}/tests/glass");
    let (ok, out, err) = run(&["spot", p.to_str().unwrap(), "--glass-dir", &glass, "--json"]);
    let (ok2, _, err2) = run(&["wavefront", p.to_str().unwrap(), "--glass-dir", &glass]);
    std::fs::remove_dir_all(&dir).ok();
    assert!(ok, "{err}");
    assert!(ok2, "{err2}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["spot"][0]["rms_centroid"].as_f64().unwrap().is_finite());
}

/// 面の無視は OpticStudio と同じく HIDE の 6 番目の値で読み書きする（OpticStudio は "IGNR" の面の行を読まない）
#[test]
fn ignored_surface_is_written_as_hide() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let raw = std::fs::read(format!("{root}/tests/refs/A0/system.zmx")).unwrap();
    let u16s: Vec<u16> = raw[2..]
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let txt = String::from_utf16(&u16s).unwrap();
    // OpticStudio 2026 R1 が面 1 を無視にして保存したときの HIDE（ほかは同じ）
    let i = txt.find("SURF 1\r\n").unwrap();
    let h = i + txt[i..].find("  HIDE ").unwrap();
    let e = h + txt[h..].find("\r\n").unwrap();
    let txt = format!("{}  HIDE 0 0 0 0 0 1 0 0 0 0 0 0 0{}", &txt[..h], &txt[e..]);
    let dir = std::env::temp_dir().join(format!("lensbench-cli-hide-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (p, q) = (dir.join("a0_hide.zmx"), dir.join("out.zmx"));
    std::fs::write(&p, txt).unwrap();
    let glass = format!("{root}/tests/glass");
    let info = |f: &std::path::Path| {
        let (ok, out, err) = run(&["info", f.to_str().unwrap(), "--glass-dir", &glass, "--json"]);
        assert!(ok, "{err}");
        serde_json::from_str::<serde_json::Value>(&out).unwrap()
    };
    let (a0, ign) = (
        info(std::path::Path::new(&format!("{root}/{ZMX}"))),
        info(&p),
    );
    // 面 1 を無視すると焦点距離が変わる
    assert_ne!(a0["effl"], ign["effl"]);
    let (ok, _, err) = run(&[
        "elements",
        p.to_str().unwrap(),
        "--glass-dir",
        &glass,
        "--out",
        q.to_str().unwrap(),
    ]);
    assert!(ok, "{err}");
    let b = std::fs::read(&q).unwrap();
    let w: Vec<u16> = b[2..]
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let out = String::from_utf16(&w).unwrap();
    let again = info(&q);
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.contains("HIDE 0 0 0 0 0 1 "), "{out}");
    assert!(!out.contains("IGNR"));
    assert_eq!(again["effl"], ign["effl"]);
}

#[test]
fn coatings_lists_interfaces_and_missing_tables() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let glass = format!("{root}/tests/glass");
    let (ok, out, err) = run(&["coatings", ZMX, "--glass-dir", &glass, "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let ifs = v["configurations"][0]["interfaces"].as_array().unwrap();
    assert!(ifs.len() >= 4);
    assert!(ifs.iter().all(|x| x["kind"] == "bare"));
    assert_eq!(v["needs_table"].as_array().unwrap().len(), 0);

    // 面 1 に膜ファイルにない膜を付けると、実測表が要る
    let raw = std::fs::read(format!("{root}/{ZMX}")).unwrap();
    let u16s: Vec<u16> = raw[2..]
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let txt = String::from_utf16(&u16s).unwrap();
    let txt = txt.replacen("SURF 1\r\n", "SURF 1\r\n  COAT ZEC_X\r\n", 1);
    let dir = std::env::temp_dir().join(format!("lensbench-cli-coat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("a0_coat.zmx");
    std::fs::write(&p, txt).unwrap();
    let (ok, out, err) = run(&[
        "coatings",
        p.to_str().unwrap(),
        "--glass-dir",
        &glass,
        "--json",
    ]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let need = v["needs_table"].as_array().unwrap();
    assert_eq!(need.len(), 1);
    let key = need[0]["key"].as_str().unwrap().to_string();
    assert!(key.starts_with("ZEC_X|AIR|"), "{key}");
    assert_eq!(need[0]["wavelengths_um"].as_array().unwrap().len(), 3);

    // 実測表を渡すと measured になる
    let wls: Vec<f64> = need[0]["wavelengths_um"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    let mut t = format!("LBCT 1\nKEY {key}\n");
    for w in &wls {
        t += &format!("WAVE {w:?}\n");
        for a in 0..=60 {
            t += &format!("{a}.0 0.98 0 0.98 0\n");
        }
    }
    let tp = dir.join("t.ctab");
    std::fs::write(&tp, t).unwrap();
    let (ok, out, err) = run(&[
        "coatings",
        p.to_str().unwrap(),
        "--glass-dir",
        &glass,
        "--coatings",
        tp.to_str().unwrap(),
        "--json",
    ]);
    std::fs::remove_dir_all(&dir).ok();
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["needs_table"].as_array().unwrap().len(), 0);
    assert_eq!(v["configurations"][0]["interfaces"][0]["kind"], "measured");
}

#[test]
fn layout_writes_every_format() {
    let dir = std::env::temp_dir().join(format!("lensbench-layout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exts = ["svg", "png", "dxf", "gltf", "glb", "obj", "stl", "step"];
    let mut args = vec![
        "layout".to_string(),
        ZMX.into(),
        "--glass-dir".into(),
        "tests/glass".into(),
        "--json".into(),
    ];
    for e in exts {
        args.push("--out".into());
        args.push(dir.join(format!("a.{e}")).display().to_string());
    }
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (ok, out, err) = run(&a);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["written"].as_array().unwrap().len(), exts.len());
    assert_eq!(v["solids"].as_array().unwrap().len(), 3);
    // 既定の光線: 2D は y 扇 7 本、3D は主光線と円周 12 本（4 視野）
    assert_eq!(v["rays_2d"], 4 * 7);
    assert_eq!(v["rays_3d"], 4 * 13);
    for e in exts {
        assert!(
            std::fs::metadata(dir.join(format!("a.{e}"))).unwrap().len() > 0,
            "{e}"
        );
    }
    // 知らない拡張子
    let bad = dir.join("a.xyz").display().to_string();
    let (ok, _, err) = run(&["layout", ZMX, "--glass-dir", "tests/glass", "--out", &bad]);
    assert!(!ok && err.contains("extension"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rays_and_analysis_are_saved() {
    let dir = std::env::temp_dir().join(format!("lensbench-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("r.csv").display().to_string();
    let (ok, _, err) = run(&[
        "rays",
        ZMX,
        "--glass-dir",
        "tests/glass",
        "--fields",
        "2",
        "--pupil",
        "0,0,0,1",
        "--surface",
        "9",
        "--save",
        &csv,
    ]);
    assert!(ok, "{err}");
    let t = std::fs::read_to_string(&csv).unwrap();
    let lines: Vec<&str> = t.lines().collect();
    assert!(lines[0].starts_with("field,wave,ray,px,py,surface,x,y,z"));
    assert_eq!(lines.len(), 3);
    // 解析の結果: csv は表、json はそのまま
    let zc = dir.join("z.csv").display().to_string();
    let (ok, _, _) = run(&[
        "zernike",
        ZMX,
        "--glass-dir",
        "tests/glass",
        "--terms",
        "4",
        "--save",
        &zc,
    ]);
    assert!(ok);
    let t = std::fs::read_to_string(&zc).unwrap();
    assert!(t.starts_with("field,coef_1,coef_2,coef_3,coef_4,"));
    assert_eq!(t.lines().count(), 5);
    let zj = dir.join("s.json").display().to_string();
    let (ok, _, _) = run(&["spot", ZMX, "--glass-dir", "tests/glass", "--save", &zj]);
    assert!(ok);
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&zj).unwrap()).unwrap();
    assert_eq!(v["spot"].as_array().unwrap().len(), 4);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn project_with_catalog_lenses() {
    // カタログレンズのブロックは --lens-catalog（.zmx のフォルダ）から引く。無ければエラー
    let p = "examples/projects/P8_catalog_two.json";
    let (ok, out, err) = run(&[
        "info",
        p,
        "--glass-dir",
        "tests/glass",
        "--lens-catalog",
        "examples/lenses/LB",
        "--json",
    ]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["effl"].as_f64().unwrap().is_finite());
    let (ok, _, err) = run(&["info", p, "--glass-dir", "tests/glass"]);
    assert!(!ok);
    assert!(err.contains("PCX-100"), "{err}");
}

#[test]
fn project_states() {
    let p = "examples/projects/P6_states_focus_group.json";
    let info = |st: Option<&str>| {
        let mut a = vec!["info", p, "--glass-dir", "tests/glass", "--json"];
        if let Some(s) = st {
            a.extend(["--state", s]);
        }
        let (ok, out, err) = run(&a);
        assert!(ok, "{err}");
        serde_json::from_str::<serde_json::Value>(&out).unwrap()
    };
    let v0 = info(None);
    assert_eq!(v0["states"], serde_json::json!(["far", "near", "no L3"]));
    // near: 前の群を物体側へ 0.4 → 面 1 から像面までが 0.4 延びる
    let v1 = info(Some("near"));
    assert_eq!(v1["state"], "near");
    let d = v1["total_track"].as_f64().unwrap() - v0["total_track"].as_f64().unwrap();
    assert!((d - 0.4).abs() < 1e-12, "{d}");
    assert_ne!(v1["effl"], v0["effl"]);
    // 状態がない・.zmx に --state・.json に --cfg はエラー
    let (ok, _, err) = run(&["info", p, "--glass-dir", "tests/glass", "--state", "x"]);
    assert!(!ok && err.contains("no state x"), "{err}");
    let (ok, _, err) = run(&["info", ZMX, "--glass-dir", "tests/glass", "--state", "near"]);
    assert!(!ok && err.contains("--state"), "{err}");
    let (ok, _, err) = run(&["info", p, "--glass-dir", "tests/glass", "--cfg", "2"]);
    assert!(!ok && err.contains("--state"), "{err}");
}

#[test]
fn layout_and_rays_follow_the_state() {
    // 状態で群を動かすと、レイアウトの大きさと光線の交点が変わる
    let p = "examples/projects/P6_states_focus_group.json";
    let dir = std::env::temp_dir().join(format!("lensbench-state-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let layout = |st: Option<&str>| {
        let out = dir.join("a.svg").display().to_string();
        let mut a = vec![
            "layout",
            p,
            "--glass-dir",
            "tests/glass",
            "--out",
            &out,
            "--json",
        ];
        if let Some(s) = st {
            a.extend(["--state", s]);
        }
        let (ok, o, err) = run(&a);
        assert!(ok, "{err}");
        serde_json::from_str::<serde_json::Value>(&o).unwrap()
    };
    let b0 = layout(None)["bounds_mm"].clone();
    let b1 = layout(Some("near"))["bounds_mm"].clone();
    assert_ne!(b0, b1);
    let rays = |st: &str| {
        let (ok, o, err) = run(&[
            "rays",
            p,
            "--glass-dir",
            "tests/glass",
            "--pupil",
            "0,1",
            "--fields",
            "1",
            "--state",
            st,
        ]);
        assert!(ok, "{err}");
        o
    };
    assert_ne!(rays("far"), rays("near"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn layout_uses_catalog_lenses() {
    // カタログレンズの玉も --lens-catalog から引いて立体になる（P8: 単レンズと貼り合わせで 3 つ）
    let dir = std::env::temp_dir().join(format!("lensbench-cat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("a.step").display().to_string();
    let (ok, o, err) = run(&[
        "layout",
        "examples/projects/P8_catalog_two.json",
        "--glass-dir",
        "tests/glass",
        "--lens-catalog",
        "examples/lenses/LB",
        "--out",
        &out,
        "--json",
    ]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(v["solids"].as_array().unwrap().len(), 3, "{v}");
    let (ok, _, err) = run(&[
        "rays",
        "examples/projects/P8_catalog_two.json",
        "--glass-dir",
        "tests/glass",
        "--lens-catalog",
        "examples/lenses/LB",
        "--pupil",
        "0,1",
    ]);
    assert!(ok, "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn user_catalog_add_list_and_use() {
    let dir = std::env::temp_dir().join(format!("lb-cli-cat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cat = dir.join("my.json");
    let c = cat.to_str().unwrap();
    let add = |extra: &[&str]| {
        let mut a = vec!["catalog-add", c, "--glass-dir", "tests/glass", "--json"];
        a.extend(extra);
        run(&a)
    };
    let (ok, _, err) = add(&["--name", "L2", "--from", ZMX, "--lens", "2"]);
    assert!(ok, "{err}");
    let p6 = "examples/projects/P6_states_focus_group.json";
    let (ok, _, err) = add(&["--name", "front", "--from", p6, "--block", "front"]);
    assert!(ok, "{err}");
    // 同じ名前は --replace なしではエラー、--lens と --block は 1 つだけ
    let (ok, _, err) = add(&["--name", "L2", "--from", ZMX, "--lens", "1"]);
    assert!(!ok && err.contains("--replace"), "{err}");
    let (ok, _, _) = add(&["--name", "x", "--from", ZMX]);
    assert!(!ok);
    let (ok, out, err) = run(&["catalog-list", c, "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let names: Vec<&str> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["L2", "front"]);
    // 保存した玉を別のプロジェクトで使う
    let proj = dir.join("p.json");
    std::fs::write(
        &proj,
        r#"{"glass_catalogs": [], "wavelengths": [{"um": 0.5875618}], "aperture": {"type": "epd", "value": 4},
            "blocks": [{"type": "stop"}, {"type": "air", "t": 2}, {"type": "user", "catalog": "my.json", "item": "L2"}, {"type": "air", "t": 30}]}"#,
    )
    .unwrap();
    let (ok, out, err) = run(&[
        "info",
        proj.to_str().unwrap(),
        "--glass-dir",
        "tests/glass",
        "--json",
    ]);
    std::fs::remove_dir_all(&dir).ok();
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["effl"].as_f64().unwrap().is_finite());
}

#[test]
fn thread_count_does_not_change_the_results() {
    let out = |th: &str| {
        let (ok, out, err) = run(&[
            "wavefront",
            ZMX,
            "--glass-dir",
            "tests/glass",
            "--json",
            "--threads",
            th,
        ]);
        assert!(ok, "{err}");
        out
    };
    let one = out("1");
    assert_eq!(one, out("4"));
    assert_eq!(one, out("0"));
}

/// 系ファイル: convert で書き出し、読み直した系の解析が同じ、--save の JSON に版と系の hash
#[test]
fn convert_to_system_file() {
    let dir = std::env::temp_dir().join(format!("lb_cli_sysfile_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("a0.json");
    let fs = f.to_str().unwrap();
    let (ok, out, err) = run(&[
        "convert",
        ZMX,
        "--glass-dir",
        "tests/glass",
        "-o",
        fs,
        "--json",
    ]);
    assert!(ok, "{err}");
    let c: serde_json::Value = serde_json::from_str(&out).unwrap();
    let h = c["system_hash"].as_str().unwrap().to_string();
    assert!(h.starts_with("sha256:"));
    let spot = |file: &str, extra: &[&str]| {
        let mut a = vec!["spot", file, "--glass-dir", "tests/glass", "--json"];
        a.extend_from_slice(extra);
        let (ok, out, err) = run(&a);
        assert!(ok, "{err}");
        serde_json::from_str::<serde_json::Value>(&out).unwrap()
    };
    assert_eq!(spot(ZMX, &[])["spot"], spot(fs, &[])["spot"]);
    // 系ファイルはガラスを名前と hash で持つ。フォルダに無ければ同梱のカタログで開き、注意（渡し方の案内・hash の違い）を出す
    let (ok, _, err) = run(&["info", fs, "--json"]);
    assert!(
        ok && err.contains("built-in catalog")
            && err.contains("--glass-dir")
            && err.contains("differs from when the file was saved"),
        "{err}"
    );
    let (ok, out, err) = run(&["info", fs, "--glass-dir", "tests/glass", "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["system_hash"].as_str().unwrap(), h);
    let saved = dir.join("spot.json");
    spot(fs, &["--save", saved.to_str().unwrap()]);
    let s: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    assert_eq!(s["provenance"]["system_hash"].as_str().unwrap(), h);
    // --state は系ファイルでは使えない
    let (ok, _, _) = run(&["info", fs, "--glass-dir", "tests/glass", "--state", "x"]);
    assert!(!ok);
    // フォルダの無いファイル名（`info a0.json`）でも、同じフォルダのカタログを見つける
    for c in v_catalogs(fs) {
        std::fs::copy(
            format!(
                "{root}/tests/glass/{c}.AGF",
                root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..")
            ),
            dir.join(format!("{c}.AGF")),
        )
        .unwrap();
    }
    let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
        .current_dir(&dir)
        .args(["info", "a0.json", "--json"])
        .env_remove("LENSBENCH_GLASSCAT")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["system_hash"].as_str().unwrap(), h);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 系ファイルの `catalogs` のカタログの名前
fn v_catalogs(file: &str) -> Vec<String> {
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    v["system"]["catalogs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn tolerance_monte_carlo() {
    // モンテカルロ公差: 種が同じなら同じ結果、最も悪い試行の系を書き出せる
    const C0: &str = "tests/refs/C0/system.zmx";
    let dir = std::env::temp_dir().join(format!("lb-tol-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let worst = dir.join("worst.json");
    let csv = dir.join("comp.csv");
    let args = |seed: &str| -> Vec<String> {
        [
            "tolerance",
            C0,
            "--glass-dir",
            "tests/glass",
            "--json",
            "--trials",
            "5",
            "--seed",
            seed,
            "--set",
            r#"{"preset":"precision","compensators":["6.t"]}"#,
            "--out",
            worst.to_str().unwrap(),
            "--comp-csv",
            csv.to_str().unwrap(),
        ]
        .iter()
        .map(|x| x.to_string())
        .collect()
    };
    let go = |seed: &str| -> serde_json::Value {
        let a = args(seed);
        let a: Vec<&str> = a.iter().map(|x| x.as_str()).collect();
        let (ok, out, err) = run(&a);
        assert!(ok, "{err}");
        serde_json::from_str(&out).unwrap()
    };
    let a = go("9");
    let b = go("9");
    assert_eq!(a["cdf"], b["cdf"]);
    assert_eq!(a["trials"], 5);
    assert!(a["sensitivity"].as_array().unwrap().len() > 10);
    assert_eq!(a["compensators"][0]["name"], "6.t");
    assert!(worst.exists());
    // 試行ごとの補償子の CSV（見出し＋試行の数の行、値は結果の values と同じ）
    let t = std::fs::read_to_string(&csv).unwrap();
    let lines: Vec<&str> = t.lines().collect();
    assert_eq!(lines[0], "trial,merit,6.t");
    assert_eq!(lines.len(), 6);
    let x: f64 = lines[1].split(',').nth(2).unwrap().parse().unwrap();
    assert_eq!(Some(x), a["compensators"][0]["values"][0].as_f64());
    // 書き出した系を読み直して評価できる
    let (ok, _, err) = run(&[
        "info",
        worst.to_str().unwrap(),
        "--glass-dir",
        "tests/glass",
    ]);
    assert!(ok, "{err}");
    // --results: 評価を JSONL に書き、もう一度回すとファイルから読んで同じ結果（試行を足しても続きから）
    let res = dir.join("r.jsonl");
    let with_results = |trials: &str| -> serde_json::Value {
        let mut a = args("9");
        a[6] = trials.to_string();
        a.extend(["--results".to_string(), res.to_str().unwrap().to_string()]);
        let a: Vec<&str> = a.iter().map(|x| x.as_str()).collect();
        let (ok, out, err) = run(&a);
        assert!(ok, "{err}");
        serde_json::from_str(&out).unwrap()
    };
    assert_eq!(with_results("5")["cdf"], a["cdf"]);
    let n5 = std::fs::read_to_string(&res).unwrap().lines().count();
    assert_eq!(with_results("5")["cdf"], a["cdf"]);
    assert_eq!(std::fs::read_to_string(&res).unwrap().lines().count(), n5);
    assert_eq!(with_results("7")["trials"], 7);
    assert_eq!(
        std::fs::read_to_string(&res).unwrap().lines().count(),
        n5 + 2
    );
    // 設定の誤り
    let (ok, _, err) = run(&[
        "tolerance",
        C0,
        "--glass-dir",
        "tests/glass",
        "--set",
        r#"{"preset":"x"}"#,
    ]);
    assert!(!ok && err.contains("tolerance set"), "{err}");
}

/// --save の中身は --json の有無によらず同じ全体（画面用の間引きは表示だけ）
#[test]
fn tolerance_save_is_full_without_json() {
    const C0: &str = "tests/refs/C0/system.zmx";
    let dir = std::env::temp_dir().join(format!("lb-tol-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (dir.join("a.json"), dir.join("b.json"));
    for (p, json) in [(&a, false), (&b, true)] {
        let mut args = vec![
            "tolerance",
            C0,
            "--glass-dir",
            "tests/glass",
            "--trials",
            "4",
            "--seed",
            "3",
            "--save",
            p.to_str().unwrap(),
        ];
        if json {
            args.push("--json");
        }
        let (ok, _, err) = run(&args);
        assert!(ok, "{err}");
    }
    let rd = |p: &std::path::Path| -> serde_json::Value {
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("provenance");
        v
    };
    let (va, vb) = (rd(&a), rd(&b));
    assert!(va.get("cdf").is_some() && va.get("merit_values").is_some());
    assert!(va["sensitivity"].as_array().unwrap().len() > 10);
    assert_eq!(va, vb);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// アフォーカル像空間の系では mtf-focus・mtf-field・rms の単位の表示も角度の単位
#[test]
fn afocal_units() {
    let dir = std::env::temp_dir().join(format!("lb-afocal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("tel.zmx");
    std::fs::write(
        &f,
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 2 1 0 0 1\n\
XFLN 0 0\nYFLN 0 0.5\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 50\n  DISZ 75\nSURF 2\n  TYPE PARAXIAL\n  PARM 1 25\n  DISZ 20\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
    )
    .unwrap();
    let f = f.to_str().unwrap();
    let unit = |args: &[&str]| -> String {
        let mut a = args.to_vec();
        a.insert(1, f);
        a.push("--json");
        let (ok, out, err) = run(&a);
        assert!(ok, "{args:?}: {err}");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        v["unit"].as_str().unwrap().to_string()
    };
    let mtf = unit(&["mtf", "--freqs", "1"]);
    assert_ne!(mtf, "cycles/mm");
    assert_eq!(unit(&["mtf-focus", "--freqs", "1"]), mtf);
    assert_eq!(unit(&["mtf-field", "--freqs", "1"]), mtf);
    assert_ne!(unit(&["rms"]), "lens");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 負の数を値として受ける（--dist -5、--center -1,2）
#[test]
fn negative_numbers_are_values() {
    let (ok, _, err) = run(&[
        "pop",
        ZMX,
        "--glass-dir",
        "tests/glass",
        "--dist",
        "-5",
        "--center",
        "-0.1,0",
        "--json",
    ]);
    assert!(ok, "{err}");
}

#[test]
fn coat_design_fits_a_reflectance_curve_and_writes_a_stack() {
    // 1/4 波長の MgF2 1 層（n 1.38、基板 1.52）の垂直入射の反射率を、材料 L から当てはめる
    let dir = std::env::temp_dir().join(format!("lensbench-coat-design-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("m.dat"), "MATE L\n0.3 1.38 0\n2.0 1.38 0\n").unwrap();
    let (n0, n1, ns, d) = (1.0f64, 1.38f64, 1.52f64, 0.55 / (4.0 * 1.38));
    let mut csv = String::from("wl_nm,value_pct\n");
    for i in 0..=40 {
        let wl = 0.45 + 0.2 * i as f64 / 40.0;
        let ph = 2.0 * std::f64::consts::PI * n1 * d / wl;
        let (r1, r2) = ((n0 - n1) / (n0 + n1), (n1 - ns) / (n1 + ns));
        let num = r1 * r1 + r2 * r2 + 2.0 * r1 * r2 * (2.0 * ph).cos();
        let den = 1.0 + r1 * r1 * r2 * r2 + 2.0 * r1 * r2 * (2.0 * ph).cos();
        csv += &format!("{},{}\n", wl * 1e3, 100.0 * num / den);
    }
    std::fs::write(dir.join("c.csv"), csv).unwrap();
    let out = dir.join("X.lbcs");
    let (ok, o, err) = run(&[
        "coat-design",
        dir.join("c.csv").to_str().unwrap(),
        "--name",
        "X",
        "--materials",
        "L",
        "--substrate",
        "1.52",
        "--coating-file",
        dir.join("m.dat").to_str().unwrap(),
        "--layers",
        "1..2",
        "--starts",
        "4",
        "--out",
        out.to_str().unwrap(),
        "--json",
    ]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert!(v["max_err"].as_f64().unwrap() < 1e-7, "{v}");
    assert_eq!(v["layers"].as_array().unwrap().len(), 1, "{v}");
    assert!(
        (v["layers"][0]["d_um"].as_f64().unwrap() - d).abs() < 1e-5,
        "{v}"
    );
    let txt = std::fs::read_to_string(&out).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(txt.contains("COAT X\nL "), "{txt}");
}

/// 周波数を省いたときの上限と分割数（--max-freq・--freq-steps）。--freqs とは一緒に使えない
#[test]
fn mtf_max_freq_steps() {
    let g = ["--glass-dir", "tests/glass", "--field", "1", "--json"];
    for cmd in ["mtf", "geo-mtf"] {
        let mut a = vec![cmd, ZMX, "--max-freq", "40", "--freq-steps", "4"];
        a.extend(g);
        let (ok, out, err) = run(&a);
        assert!(ok, "{cmd}: {err}");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let key = if cmd == "mtf" { "mtf" } else { "geometric_mtf" };
        assert_eq!(
            v[key][0]["freqs"],
            serde_json::json!([0.0, 10.0, 20.0, 30.0, 40.0]),
            "{cmd}"
        );
    }
    let mut a = vec!["mtf", ZMX, "--freqs", "10", "--max-freq", "40"];
    a.extend(g);
    assert!(!run(&a).0);
}

/// 終了コードは誤りの種類で分かれ、案内が付く
#[test]
fn exit_codes_by_kind() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let code_in = |lang: &str, args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
            .current_dir(root)
            .args(args)
            .env_remove("LENSBENCH_GLASSCAT")
            .env("LENSBENCH_LANG", lang)
            .output()
            .unwrap();
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let code = |args: &[&str]| code_in("en", args);
    // 引数の誤り（clap・値の検査）は 2。clap の誤りは英語、日本語の表示では訳す
    let (c, err) = code(&["spot", ZMX, "--bogus"]);
    assert_eq!(c, Some(2), "{err}");
    assert!(err.contains("unexpected argument '--bogus'"), "{err}");
    let (c, err) = code_in("ja", &["spot", ZMX, "--bogus"]);
    assert_eq!(c, Some(2), "{err}");
    assert!(
        err.contains("知らない引数 --bogus") && err.contains("案内:"),
        "{err}"
    );
    let (c, err) = code(&[
        "optimize",
        ZMX,
        "--glass-dir",
        "tests/glass",
        "--var",
        "8.t",
        "--merit",
        "x",
    ]);
    // 選べる値は clap が確かめる（設定の型の綴り）
    assert_eq!(c, Some(2), "{err}");
    assert!(
        err.contains("spot, wavefront, none") && err.contains("--help"),
        "{err}"
    );
    // zoom の --method・DLS の --gtol の誤りも使い方の誤り（選択肢は clap が出す）
    let zoom = |extra: &[&str]| {
        let mut a = vec![
            "zoom",
            "examples/projects/P17_zoom_three_groups.json",
            "--glass-dir",
            "tests/glass",
            "--var",
            "G2.shift",
        ];
        a.extend_from_slice(extra);
        a.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    };
    let z = zoom(&["--method", "foo"]);
    let (c, err) = code(&z.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(c, Some(2), "{err}");
    assert!(err.contains("penalty") && err.contains("lagrange"), "{err}");
    let z = zoom(&["--gtol=-1"]);
    let (c, err) = code(&z.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(c, Some(2), "{err}");
    assert!(err.contains("must be 0 or more"), "{err}");
    let (c, err) = code_in("ja", &z.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(c, Some(2), "{err}");
    assert!(err.contains("0 以上に"), "{err}");
    let (c, err) = code(&[
        "optimize",
        ZMX,
        "--glass-dir",
        "tests/glass",
        "--var",
        "8.t",
        "--gtol",
        "x",
    ]);
    assert_eq!(c, Some(2), "{err}");
    // ファイルが無いのは 3
    let (c, err) = code(&["info", "no/such.zmx"]);
    assert_eq!(c, Some(3), "{err}");
    assert!(err.contains("hint:"), "{err}");
    // 未対応は 4
    let dir = std::env::temp_dir().join(format!("lb-exit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ni = dir.join("unit.zmx");
    std::fs::write(
        &ni,
        "VERS 190513 80 123457\nMODE SEQ\nUNIT FURLONG X W X CM MR CPMM\nSURF 0\n  DISZ INFINITY\nSURF 1\n  STOP\n  DISZ 10\nSURF 2\n",
    )
    .unwrap();
    let (c, err) = code(&["info", ni.to_str().unwrap()]);
    assert_eq!(c, Some(4), "{err}");
    std::fs::remove_dir_all(&dir).ok();
    // 系の不正は 1（ガラスが無いときは渡し方の案内が文に入る。同梱にも無いカタログ）
    let dir = std::env::temp_dir().join(format!("lb-exit-glass-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ng = dir.join("glass.zmx");
    std::fs::write(
        &ng,
        "VERS 190513 80 123457\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nWAVM 1 0.5876 1\nGCAT NOSUCHCAT\nSURF 0\n  DISZ INFINITY\nSURF 1\n  STOP\n  DISZ 5\n  GLAS N-BK7 0 0 1.5 40 0 0 0 0 0 0\nSURF 2\n  DISZ 10\nSURF 3\n",
    )
    .unwrap();
    let (c, err) = code(&["spot", ng.to_str().unwrap()]);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(c, Some(1), "{err}");
    assert!(err.contains("--glass-dir"), "{err}");
}

/// `help --json` は clap の定義（登録簿の解析も）と終了コードの表を出す（マニュアルの CLI のページの元）
#[test]
fn help_json_lists_commands() {
    let (ok, out, err) = run(&["help", "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let subs = v["subcommands"].as_array().unwrap();
    assert!(subs.iter().any(|s| s["name"] == "mtf-focus"));
    assert!(v["exit_codes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["code"] == 4));
    // ふつうの help はそのまま
    let (ok, out, _) = run(&["help", "spot"]);
    // exe の名前は OS で違う（Windows は lensbench.exe）
    assert!(
        ok && out.contains("Usage: lensbench") && out.contains(" spot [OPTIONS]"),
        "{out}"
    );
    // help --json の使い方の行は exe の名前によらず lensbench（マニュアルに lensbench.exe を出さない）
    let spot = subs.iter().find(|s| s["name"] == "spot").unwrap();
    assert!(
        spot["usage"]
            .as_str()
            .unwrap()
            .starts_with("lensbench spot "),
        "{}",
        spot["usage"]
    );
}

/// 解析の設定を JSON のファイルで渡せる。コマンド行で明示した旗がファイルより優先し、旗の既定値は
/// ファイルを上書きしない
#[test]
fn params_file_and_flags() {
    let dir = std::env::temp_dir().join(format!("lb-params-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("zernike.json");
    std::fs::write(&f, r#"{"terms": 16, "field": 2}"#).unwrap();
    let f = f.to_str().unwrap();
    let base = ["zernike", ZMX, "--glass-dir", "tests/glass", "--json"];
    let terms = |args: &[&str]| -> serde_json::Value {
        let (ok, out, err) = run(args);
        assert!(ok, "{err}");
        serde_json::from_str(&out).unwrap()
    };
    let by_flags = terms(&[&base[..], &["--terms", "16", "--field", "2"]].concat());
    let by_file = terms(&[&base[..], &["--params-file", f]].concat());
    assert_eq!(by_flags, by_file);
    let over = terms(
        &[
            &base[..],
            &["--params-file", f, "--terms", "9", "--field", "2"],
        ]
        .concat(),
    );
    let nine = terms(&[&base[..], &["--terms", "9", "--field", "2"]].concat());
    assert_eq!(over, nine);
    assert_ne!(over, by_file);
    // 知らない鍵・オブジェクトでない JSON は誤り
    std::fs::write(dir.join("bad.json"), r#"{"no_such_key": 1}"#).unwrap();
    let (ok, _, _) = run(&[
        &base[..],
        &["--params-file", dir.join("bad.json").to_str().unwrap()],
    ]
    .concat());
    assert!(!ok);
    std::fs::write(dir.join("list.json"), "[1]").unwrap();
    let (ok, _, err) = run(&[
        &base[..],
        &["--params-file", dir.join("list.json").to_str().unwrap()],
    ]
    .concat());
    assert!(!ok && err.contains("JSON object"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

/// 既定が真の旗は `=false` で切れ、設定のファイルの false と同じになる（値を取らない旗では既定の真を切れなかった）
#[test]
fn flags_on_by_default_turn_off_with_false() {
    let dir = std::env::temp_dir().join(format!("lb-flag-false-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("keep.json");
    std::fs::write(&f, r#"{"remove_vignetting_factors": false, "points": 5}"#).unwrap();
    let base = [
        "relative-illumination",
        "tests/refs/A6/system.zmx",
        "--glass-dir",
        "tests/glass",
        "--json",
    ];
    let ri = |args: &[&str]| -> serde_json::Value {
        let (ok, out, err) = run(&[&base[..], args].concat());
        assert!(ok, "{err}");
        serde_json::from_str(&out).unwrap()
    };
    let replaced = ri(&["--points", "5"]);
    assert_eq!(
        ri(&["--points", "5", "--remove-vignetting-factors"]),
        replaced
    );
    assert_eq!(
        ri(&["--points", "5", "--remove-vignetting-factors=true"]),
        replaced
    );
    let kept = ri(&["--points", "5", "--remove-vignetting-factors=false"]);
    assert_ne!(kept, replaced);
    assert_eq!(ri(&["--params-file", f.to_str().unwrap()]), kept);
    // 値は = で付ける（後ろの位置引数を値と取り違えない）
    let (ok, _, _) = run(&[&base[..], &["--remove-vignetting-factors", "false"]].concat());
    assert!(!ok);
    std::fs::remove_dir_all(&dir).ok();
}

/// 既定が真の旗の --help: 使い方は `[=<true|false>]` で示し、説明は日本語の表示でも訳す（英語の説明に文を足さない）
#[test]
fn flags_on_by_default_help_is_translated() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    for name in ["relative-illumination", "vignetting"] {
        let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
            .current_dir(root)
            .args([name, "--help"])
            .env("LENSBENCH_LANG", "ja")
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&o.stdout);
        let line = out
            .lines()
            .skip_while(|l| !l.contains("--remove-vignetting-factors"))
            .take(2)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(line.contains("[=<true|false>]"), "{name}: {line}");
        assert!(line.contains("ケラレ係数"), "{name}: {line}");
        assert!(!line.contains("Replace the vignetting"), "{name}: {line}");
    }
}

/// exe の隣に catalogs/glass がある配置なら、--glass-dir なしでもそこのカタログを使う（同梱の屈折率だけのカタログの注意が出ない）
#[test]
fn glass_catalogs_next_to_the_exe_are_used() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_lensbench"));
    let dir = std::env::temp_dir().join(format!("lb_exe_glass_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join(exe.file_name().unwrap());
    std::fs::copy(exe, &copy).unwrap();
    let info = || {
        let o = Command::new(&copy)
            .current_dir(root)
            .args(["info", ZMX])
            .env_remove("LENSBENCH_GLASSCAT")
            .output()
            .unwrap();
        assert!(o.status.success());
        String::from_utf8_lossy(&o.stderr).into_owned()
    };
    // catalogs/glass が無ければ同梱のカタログを使ったと知らせる
    let err = info();
    assert!(err.contains("built-in catalog"), "{err}");
    let g = dir.join("catalogs").join("glass");
    std::fs::create_dir_all(&g).unwrap();
    for f in ["SCHOTT.AGF", "OHARA.AGF"] {
        std::fs::copy(format!("{root}/tests/glass/{f}"), g.join(f)).unwrap();
    }
    let err = info();
    std::fs::remove_dir_all(&dir).ok();
    assert!(!err.contains("built-in catalog"), "{err}");
}
