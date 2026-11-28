//! 非順次の経路の表の行の順次の系（`nsc --sequential`）: 系そのもの・ゴースト（.zmx に書く）・プロジェクトに足す光束

use std::process::Command;

fn run(args: &[&str]) -> serde_json::Value {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
        .current_dir(root)
        .args(args)
        .env_remove("LENSBENCH_GLASSCAT")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "{err}");
    serde_json::from_slice(&o.stdout).unwrap_or(serde_json::Value::Null)
}

const OPTS: [&str; 10] = [
    "--glass-dir",
    "tests/glass",
    "--grid",
    "5",
    "--pixels",
    "8",
    "--paths",
    "15",
    "--sequential",
    "--json",
];

/// 単レンズの系: 反射なしの経路は系そのもの、2 回反射の経路はゴーストで、書いた .zmx は読める
#[test]
fn ghost_rows_write_their_systems() {
    let dir = std::env::temp_dir().join(format!("lb-nsc-seq-{}", std::process::id()));
    let out = dir.display().to_string();
    let mut a = vec!["nsc", "tests/refs/A0/system.zmx"];
    a.extend(OPTS);
    a.extend(["--out", &out]);
    let v = run(&a);
    let rows = v["paths"].as_array().unwrap();
    assert_eq!(rows[0]["sequential"]["kind"], "system");
    let g = rows
        .iter()
        .find(|r| r["reflections"] == 2 && r["sequential"]["kind"] == "ghost")
        .expect("a ghost row");
    let q = &g["sequential"];
    assert!(q["b"].as_u64() < q["a"].as_u64(), "{q}");
    let f = q["file"].as_str().unwrap();
    assert!(std::path::Path::new(f).is_file(), "{f}");
    let i = run(&["info", f, "--glass-dir", "tests/glass", "--json"]);
    assert!(i["effl"].is_number(), "{i}");
    for r in rows.iter().filter(|r| r["reflections"].as_u64() > Some(2)) {
        assert_eq!(r["sequential"]["kind"], "none", "{r}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// トワイマン・グリーン: BS の経路は系そのもの、玉の面で折り返す経路は光束（BS の腕の中の玉も）。2 回以上の
/// 折り返しは turn の並び
#[test]
fn turn_rows_are_beams() {
    let mut a = vec!["nsc", "examples/projects/P16_twyman_green.json"];
    a.extend(OPTS);
    // 玉の中で 2 回折り返す経路まで
    let k = a.iter().position(|x| *x == "--paths").unwrap();
    a[k + 1] = "30";
    let v = run(&a);
    let rows = v["paths"].as_array().unwrap();
    assert!(rows
        .iter()
        .any(|r| r["sequential"]["kind"] == "system" && r["sequential"]["name"] == "path RT"));
    let beams: Vec<&serde_json::Value> = rows
        .iter()
        .map(|r| &r["sequential"])
        .filter(|q| q["kind"] == "beam")
        .collect();
    assert!(beams.len() >= 2, "{rows:?}");
    for q in &beams {
        let turns = match q["turn"].as_array() {
            Some(v) => v.clone(),
            None => vec![q["turn"].clone()],
        };
        assert!(turns.iter().all(|t| t["block"] == "TEST"), "{q}");
    }
    assert!(beams.iter().any(|q| q["path"] == "TT"), "{beams:?}");
    assert!(
        beams
            .iter()
            .any(|q| q["turn"].as_array().is_some_and(|v| v.len() == 2)),
        "{beams:?}"
    );
}
