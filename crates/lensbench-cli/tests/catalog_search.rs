//! カタログの品を探す: CLI catalog-search と serve の catalog_search

use serde_json::{json, Value};
use std::process::Command;

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn cli(args: &[&str]) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
        .args(args)
        .current_dir(root())
        .output()
        .unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(if o.status.success() {
            &o.stdout
        } else {
            &o.stderr
        })
        .into_owned(),
    )
}

fn parts(v: &Value) -> Vec<&str> {
    v["lenses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["part"].as_str().unwrap())
        .collect()
}

#[test]
fn cli_search() {
    let base = [
        "catalog-search",
        "examples/lenses/LB",
        "examples/lenses/LBP",
        "--glass-dir",
        "tests/glass",
    ];
    let run = |extra: &[&str]| -> Value {
        let a: Vec<&str> = base.iter().chain(extra).copied().collect();
        let (ok, out) = cli(&a);
        assert!(ok, "{out}");
        serde_json::from_str(&out).unwrap()
    };
    let v = run(&[
        "--json",
        "--kind",
        "mirror",
        "--kind",
        "window",
        "--spec",
        "wavelength_nm=633",
    ]);
    assert_eq!(parts(&v), ["CM-254-100", "PM-254", "WIN-254-3"]);
    let v = run(&[
        "--json",
        "--vendor",
        "lb",
        "--spec",
        "split_ratio=50:50",
        "--spec",
        "aoi_deg=45",
    ]);
    assert_eq!(parts(&v), ["BSP-254"]);
    let v = run(&["--json", "--text", "achromat"]);
    assert_eq!(v["lenses"][0]["url"], "https://example.com/lb/ACH-45");
    // 表
    let (ok, out) = cli(&[&base[..], &["--kind", "polarizer"]].concat());
    assert!(ok, "{out}");
    assert!(out.starts_with("1 品"), "{out}");
    assert!(
        out.contains("POL-254") && out.contains("https://example.com/lb/POL-254"),
        "{out}"
    );
    // 誤り
    let (ok, out) = cli(&[&base[..], &["--kind", "gizmo"]].concat());
    assert!(!ok && out.contains("gizmo"), "{out}");
    let (ok, out) = cli(&[&base[..], &["--spec", "efl>=abc"]].concat());
    assert!(!ok && out.contains("abc"), "{out}");
    // 仕様の名前の一覧
    let (ok, out) = cli(&["catalog-search", "x", "--list-specs", "--json"]);
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert!(v["specs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["name"] == "extinction_ratio"));
}

#[test]
fn serve_search() {
    let p = json!({
        "lens_catalog": [root().join("examples/lenses/LBP")],
        "glass_dir": [root().join("tests/glass")],
        "kinds": ["splitter"],
        "specs": ["wavelength_nm=420..680"],
    });
    let v = lensbench_engine::browse("catalog_search", &p).unwrap();
    assert_eq!(parts(&v), ["BS-127", "BSP-254", "PBS-127"]);
    let v = lensbench_engine::call("catalog_search", &p, Some(1)).unwrap();
    assert_eq!(parts(&v).len(), 3);
    let bad = json!({ "lens_catalog": [], "kinds": ["gizmo"] });
    assert!(lensbench_engine::browse("catalog_search", &bad).is_err());
}

/// 機構部品の検索と、機構のケラレ（mech-check）
#[test]
fn mechanical_parts_and_mech_check() {
    let (ok, out) = cli(&[
        "catalog-search",
        "examples/lenses/LBP",
        "--glass-dir",
        "tests/glass",
        "--kind",
        "tube",
        "--kind",
        "cage_plate",
        "--json",
    ]);
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(parts(&v), ["CP-30", "TUBE-25-30"]);
    let tube = v["lenses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["part"] == "TUBE-25-30")
        .unwrap();
    assert_eq!(tube["inner_diameter"], json!(23.0));
    assert_eq!(tube["mech"]["thread"], "M25");

    let (ok, out) = cli(&[
        "mech-check",
        "examples/projects/P19_tube_cage.json",
        "--glass-dir",
        "tests/glass",
        "--json",
    ]);
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let rs = v["results"].as_array().unwrap();
    assert_eq!(rs.len(), 3);
    // 軸上は切れない。20° は全部どこかで切れる（チューブで先に切れた分と、残りをケージ板が切る）
    assert_eq!(rs[0]["passed_pct"], json!(100.0));
    let first: f64 = rs[2]["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["first_pct"].as_f64().unwrap())
        .sum();
    assert!(
        (first + rs[2]["passed_pct"].as_f64().unwrap() + rs[2]["optical_pct"].as_f64().unwrap()
            - 100.0)
            .abs()
            < 1e-9
    );
    assert!(rs[2]["parts"][0]["first_pct"].as_f64().unwrap() > 0.0);
    // 配置の確認: チューブはレンズを保持径に収め、はみ出しも重なりもない
    assert_eq!(v["fit"], json!([]));
    // 機構の無い系は誤り
    let (ok, _) = cli(&[
        "mech-check",
        "examples/projects/P18_swap_two.json",
        "--glass-dir",
        "tests/glass",
        "--lens-catalog",
        "examples/lenses",
    ]);
    assert!(!ok);
}

/// 機構部品の描画: layout の書き出しに部品が入る
#[test]
fn layout_draws_mechanics() {
    let dir = std::env::temp_dir().join(format!("lb372-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let svg = dir.join("p19.svg");
    let stp = dir.join("p19.step");
    let (ok, out) = cli(&[
        "layout",
        "examples/projects/P19_tube_cage.json",
        "--glass-dir",
        "tests/glass",
        "--out",
        svg.to_str().unwrap(),
        "--out",
        stp.to_str().unwrap(),
        "--json",
    ]);
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["mechanics"], json!(["tube", "cage"]));
    assert!(std::fs::read_to_string(&svg).unwrap().contains("tube"));
    assert!(std::fs::read_to_string(&stp)
        .unwrap()
        .contains("MANIFOLD_SOLID_BREP('cage 1'"));
    std::fs::remove_dir_all(&dir).ok();
}

/// gzip で圧縮したカタログ（.json.gz）をそのまま読む: 一覧・検索と、プロジェクトの catalog（.json を書いて .json.gz だけある）
#[test]
fn gzipped_catalogs() {
    use std::io::Write;
    let dir = std::env::temp_dir().join(format!("lbgz-{}", std::process::id()));
    let cat = dir.join("LBP");
    std::fs::create_dir_all(&cat).unwrap();
    let src = std::fs::read(root().join("examples/lenses/LBP/parts.json")).unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gz.write_all(&src).unwrap();
    std::fs::write(cat.join("parts.json.gz"), gz.finish().unwrap()).unwrap();
    let plain = |d: &std::path::Path| -> Value {
        let (ok, out) = cli(&[
            "catalog-search",
            d.to_str().unwrap(),
            "--glass-dir",
            "tests/glass",
            "--kind",
            "tube",
            "--kind",
            "cage_plate",
            "--json",
        ]);
        assert!(ok, "{out}");
        serde_json::from_str(&out).unwrap()
    };
    let v = plain(&cat);
    assert_eq!(parts(&v), ["CP-30", "TUBE-25-30"]);
    // 同じ名前の .json と .json.gz があれば .json だけ（品が 2 重にならない）
    std::fs::write(cat.join("parts.json"), &src).unwrap();
    assert_eq!(parts(&plain(&cat)), ["CP-30", "TUBE-25-30"]);
    std::fs::remove_file(cat.join("parts.json")).unwrap();
    // プロジェクトは parts.json を指し、あるのは parts.json.gz だけ
    let p = std::fs::read_to_string(root().join("examples/projects/P19_tube_cage.json"))
        .unwrap()
        .replace("../lenses/LBP/parts.json", "LBP/parts.json");
    let pj = dir.join("p19.json");
    std::fs::write(&pj, p).unwrap();
    let (ok, out) = cli(&[
        "mech-check",
        pj.to_str().unwrap(),
        "--glass-dir",
        "tests/glass",
        "--json",
    ]);
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["parts"][0]["name"], "tube");
    std::fs::remove_dir_all(&dir).ok();
}

/// メーカーのフォルダの親を渡しても、1 段下のカタログを読む（0 品のまま進まない）
#[test]
fn parent_of_vendor_folder() {
    let (ok, out) = cli(&["catalog-search", "examples/lenses", "--json"]);
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert!(parts(&v).contains(&"PCX-100"), "{out}");
}
