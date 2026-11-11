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
    let (ok, _, err) = run(&["spot", ZMX]);
    assert!(!ok && err.contains("N-SK16"), "{err}");
    let (ok, _, err) = run(&["spot", ZMX, "--glass-dir", "tests/glass", "--field", "9"]);
    assert!(!ok && err.contains("視野 9"), "{err}");
}

#[test]
fn cfg_out_of_range() {
    let (ok, _, err) = run(&["info", ZMX, "--glass-dir", "tests/glass", "--cfg", "2"]);
    assert!(!ok && err.contains("構成 2"), "{err}");
}

#[test]
fn small_grid_is_an_error() {
    for cmd in ["zernike", "wavefront"] {
        let (ok, _, err) = run(&[cmd, ZMX, "--glass-dir", "tests/glass", "--n", "3"]);
        assert!(!ok && err.contains("4 以上の偶数"), "{cmd}: {err}");
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
        "--table",
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
    assert!(!ok && err.contains("拡張子"));
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
        "--field",
        "2",
        "--pupil",
        "0,0;0,1",
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
    assert!(!ok && err.contains("状態 x"), "{err}");
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
            "--field",
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
    let (ok, out, _) = run(&["info", fs, "--json"]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["system_hash"].as_str().unwrap(), h);
    let saved = dir.join("spot.json");
    spot(fs, &["--save", saved.to_str().unwrap()]);
    let s: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    assert_eq!(s["provenance"]["system_hash"].as_str().unwrap(), h);
    // --state は系ファイルでは使えない
    let (ok, _, _) = run(&["info", fs, "--state", "x"]);
    assert!(!ok);
    std::fs::remove_dir_all(&dir).unwrap();
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
            "--save-worst",
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
    // 設定の誤り
    let (ok, _, err) = run(&[
        "tolerance",
        C0,
        "--glass-dir",
        "tests/glass",
        "--set",
        r#"{"preset":"x"}"#,
    ]);
    assert!(!ok && err.contains("公差の組"), "{err}");
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
