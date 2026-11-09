//! `lensbench zoom`・`lensbench swap`

use std::process::Command;

fn run(args: &[&str]) -> (bool, serde_json::Value, String) {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
        .current_dir(root)
        .args(args)
        .env_remove("LENSBENCH_GLASSCAT")
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    (
        o.status.success(),
        serde_json::from_str(&out).unwrap_or(serde_json::Value::Null),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

const ZOOM: &str = "examples/projects/P17_zoom_three_groups.json";
const SWAP: &str = "examples/projects/P18_swap_two.json";

#[test]
fn zoom_project_states() {
    let (ok, v, err) = run(&[
        "zoom",
        ZOOM,
        "--glass-dir",
        "tests/glass",
        "--var",
        "G2.shift",
        "--var",
        "G3.shift=-10:30",
        "--efl",
        "95,110,130",
        "--steps",
        "1",
        "--json",
    ]);
    assert!(ok, "{err}");
    let p = v["path"].as_array().unwrap();
    assert_eq!(p.len(), 5);
    assert_eq!(p[0]["state"], "wide");
    assert_eq!(p[1]["state"], serde_json::Value::Null);
    assert_eq!(p[4]["state"], "tele");
    for (i, t) in [(0, 95.0), (2, 110.0), (4, 130.0)] {
        let e = p[i]["efl"].as_f64().unwrap();
        assert!((e - t).abs() < 1e-8 * t, "{e} {t}");
    }
    assert!(p[4]["G3.shift"].as_f64().unwrap() > p[0]["G3.shift"].as_f64().unwrap());
    assert_eq!(v["warnings"].as_array().unwrap().len(), 0);
    // 状態の選び方と目標の数の誤り
    let (ok, v, _) = run(&[
        "zoom",
        ZOOM,
        "--glass-dir",
        "tests/glass",
        "--states",
        "wide,tele",
        "--var",
        "G2.shift",
        "--var",
        "G3.shift",
        "--efl",
        "95,130",
        "--json",
    ]);
    assert!(ok);
    assert_eq!(v["path"].as_array().unwrap().len(), 2);
    let (ok, _, err) = run(&[
        "zoom",
        ZOOM,
        "--glass-dir",
        "tests/glass",
        "--var",
        "G2.shift",
        "--efl",
        "95,130",
    ]);
    assert!(!ok && err.contains("目標が 2 個"), "{err}");
}

#[test]
fn zoom_zmx_config() {
    // .zmx の構成（1 つ）: 像面の手前の間隔で焦点距離を合わせる（A0 の面 3 の厚さ）
    let (ok, v, err) = run(&[
        "zoom",
        "tests/refs/A0/system.zmx",
        "--glass-dir",
        "tests/glass",
        "--states",
        "1",
        "--var",
        "3.t",
        "--efl",
        "53",
        "--json",
    ]);
    assert!(ok, "{err}");
    let p = &v["path"][0];
    assert_eq!(p["state"], "1");
    assert!((p["efl"].as_f64().unwrap() - 53.0).abs() < 1e-6);
}

#[test]
fn swap_search() {
    let dir = std::env::temp_dir().join(format!("lb-swap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let spec = dir.join("spec.json");
    std::fs::write(
        &spec,
        r#"{"slots": [{"block": "A", "filter": {"efl": [40, null]}},
                      {"block": "B", "parts": [{"vendor": "LB", "part": "ACH-45"}]}],
            "vars": ["gap.t=0.5:20", "bfl.t=1:200"], "iters": 15}"#,
    )
    .unwrap();
    let spec = spec.to_string_lossy().into_owned();
    let (ok, v, err) = run(&[
        "swap",
        SWAP,
        "--glass-dir",
        "tests/glass",
        "--lens-catalog",
        "examples/lenses/LB",
        "--spec",
        &spec,
        "--top",
        "2",
        "--json",
    ]);
    assert!(ok, "{err}");
    let n = v["count"].as_u64().unwrap();
    assert!(n >= 3, "{v}");
    assert_eq!(v["slots"][1], 1);
    let r = v["results"].as_array().unwrap();
    assert_eq!(r.len(), 2);
    assert!(r[0]["merit"].as_f64().unwrap() <= r[1]["merit"].as_f64().unwrap());
    assert_eq!(r[0]["lenses"][1]["part"], "ACH-45");
    // --out で JSONL に書き出し、もう一度回すと続き（全部書いてあるので回さない）で同じ結果
    let out = dir.join("r.jsonl");
    let _ = std::fs::remove_file(&out);
    let out_s = out.to_string_lossy().into_owned();
    let args = [
        "swap",
        SWAP,
        "--glass-dir",
        "tests/glass",
        "--lens-catalog",
        "examples/lenses/LB",
        "--spec",
        &spec,
        "--top",
        "2",
        "--json",
        "--out",
        &out_s,
    ];
    let (ok, v1, err) = run(&args);
    assert!(ok, "{err}");
    assert_eq!(v1["results"], v["results"]);
    let n1 = std::fs::read_to_string(&out).unwrap().lines().count();
    assert_eq!(n1 as u64, 1 + n);
    let (ok, v2, _) = run(&args);
    assert!(ok);
    assert_eq!(v2["results"], v["results"]);
    assert_eq!(std::fs::read_to_string(&out).unwrap().lines().count(), n1);
    // カタログが無ければ誤り
    let (ok, _, _) = run(&["swap", SWAP, "--glass-dir", "tests/glass", "--spec", &spec]);
    assert!(!ok);
}
