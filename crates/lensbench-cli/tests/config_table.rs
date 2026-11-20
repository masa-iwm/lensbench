//! 多重構成の表: serve の config_table・config_diff と edit の add_config_row・set_config_coatings

use lensbench_engine::Session;
use serde_json::{json, Value};

/// 2 つの構成: 面 1 の曲率と面 2 の厚さが構成で違う
const ZMX: &str = "VERS 260327 42
MODE SEQ
UNIT MM X W X CM MR CPMM
ENPD 10
FTYP 0 0 1 1 0 0 0
YFLN 0
FWGN 1
WAVM 1 0.55 1
PWAV 1
SURF 0
  TYPE STANDARD
  CURV 0.0
  DISZ INFINITY
SURF 1
  STOP
  TYPE STANDARD
  CURV 0.02
  DISZ 5
  GLAS N-BK7 0 0 1.5 40
  DIAM 6 1 0 0 1
SURF 2
  TYPE STANDARD
  CURV -0.01
  DISZ 90
  DIAM 6 1 0 0 1
SURF 3
  TYPE STANDARD
  CURV 0.0
  DISZ 0
MNUM 2 1
THIC 2 1 90 0 0 0 1 1 1 0 0 \"\" 0
THIC 2 2 80 0 0 0 1 1 1 0 0 \"\" 0
CRVT 1 1 0.02 0 0 0 1 1 1 0 0 \"\" 0
CRVT 1 2 0.025 0 0 0 1 1 1 0 0 \"\" 0
";

fn ok(sv: &Session, method: &str, p: Value) -> Value {
    sv.call(method, &p, Some(1))
        .unwrap_or_else(|e| panic!("{method}: {}", e.message))
}

#[test]
fn config_table_from_zmx_and_edit() {
    let sv = Session::new();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let d = std::env::temp_dir().join(format!("lb-config-table-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("two.zmx");
    std::fs::write(&f, ZMX).unwrap();
    ok(
        &sv,
        "load",
        json!({ "file": f, "glass_dirs": [format!("{root}/tests/glass")], "cfg": 2 }),
    );
    let t = ok(&sv, "config_table", json!({}));
    assert_eq!(
        (
            t["n"].as_u64(),
            t["zmx_cfgs"].as_u64(),
            t["zmx_cfg"].as_u64()
        ),
        (Some(2), Some(2), Some(2))
    );
    // .zmx の多重構成は読込で表になる（構成 2 を読んだので、今の構成は 2）
    assert_eq!(t["rows"].as_array().unwrap().len(), 2, "{t}");

    // config_diff（.zmx から構成ごとに読み直した表）を当てても同じ
    let d2 = ok(&sv, "config_diff", json!({}));
    assert_eq!(d2["rows"], 2, "{d2}");
    ok(&sv, "edit", json!({ "ops": d2["ops"] }));
    let t = ok(&sv, "config_table", json!({}));
    assert_eq!((t["n"].as_u64(), t["active"].as_u64()), (Some(2), Some(2)));
    let rows = t["rows"].as_array().unwrap();
    let row = |name: &str| {
        rows.iter()
            .find(|r| r["param"] == name)
            .unwrap_or_else(|| panic!("{name} が無い: {t}"))["values"]
            .clone()
    };
    assert_eq!(row("1.c"), json!([0.02, 0.025]));
    assert_eq!(row("2.t"), json!([90.0, 80.0]));
    let th = |j: usize| {
        ok(&sv, "get", json!({ "param": format!("{j}.t") }))["value"]
            .as_f64()
            .unwrap()
    };
    assert_eq!(th(2), 80.0);

    // 構成を切り替えると値が入る
    let u0 = ok(&sv, "history", json!({}))["undo"].as_u64().unwrap();
    ok(
        &sv,
        "edit",
        json!({ "ops": [{ "op": "set_config", "k": 1 }] }),
    );
    assert_eq!(th(2), 90.0);
    // 構成の切り替えだけが続いても元に戻す段は 1 つ（間があいても）。戻すと切り替える前の構成
    ok(
        &sv,
        "edit",
        json!({ "ops": [{ "op": "set_config", "k": 2 }] }),
    );
    std::thread::sleep(std::time::Duration::from_millis(1100));
    ok(
        &sv,
        "edit",
        json!({ "ops": [{ "op": "set_config", "k": 1 }] }),
    );
    assert_eq!(ok(&sv, "history", json!({}))["undo"].as_u64(), Some(u0 + 1));
    ok(&sv, "undo", json!({}));
    assert_eq!(th(2), 80.0);
    ok(&sv, "redo", json!({}));
    assert_eq!(th(2), 90.0);

    // 行を足す（どの構成も今の値）・値を変える・消す
    ok(
        &sv,
        "edit",
        json!({ "ops": [{ "op": "add_config_row", "param": "1.t" }] }),
    );
    let t = ok(&sv, "config_table", json!({}));
    let r = t["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["param"] == "1.t")
        .unwrap()
        .clone();
    assert_eq!(r["values"], json!([5.0, 5.0]));
    ok(
        &sv,
        "edit",
        json!({ "ops": [
            { "op": "set_config_row", "param": "1.t", "values": [5.0, 6.0] },
            { "op": "set_config", "k": 2 },
        ] }),
    );
    assert_eq!(th(1), 6.0);
    // 表の外（set・面の編集）で値を変えると、今の構成の列も変わる
    ok(&sv, "set", json!({ "values": { "1.t": 6.5 } }));
    let col = |p: &str| {
        let t = ok(&sv, "config_table", json!({}));
        t["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["param"] == p)
            .unwrap()["values"]
            .clone()
    };
    assert_eq!(col("1.t"), json!([5.0, 6.5]));
    ok(
        &sv,
        "edit",
        json!({ "ops": [{ "op": "set_surface", "surface": 2, "fields": { "t": 77.0 } }] }),
    );
    assert_eq!(col("2.t"), json!([90.0, 77.0]));
    ok(
        &sv,
        "edit",
        json!({ "ops": [{ "op": "remove_config_row", "param": "1.t" }] }),
    );
    assert_eq!(
        ok(&sv, "config_table", json!({}))["rows"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // 構成の数を 3 に（増やした構成は今の値）
    ok(
        &sv,
        "edit",
        json!({ "ops": [{ "op": "set_config_count", "n": 3 }] }),
    );
    let t = ok(&sv, "config_table", json!({}));
    assert_eq!(t["n"], 3);
    // 膜の行: 足すとどの構成も今の膜（なし）、名前で書き換える。材料の行の操作は膜の行を断る
    let row = |p: &str| {
        ok(&sv, "config_table", json!({}))["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["param"] == p)
            .cloned()
            .unwrap_or_else(|| panic!("no row {p}"))
    };
    ok(
        &sv,
        "edit",
        json!({ "ops": [{ "op": "add_config_row", "param": "1.coating" }] }),
    );
    assert_eq!(row("1.coating")["names"], json!([""]));
    ok(
        &sv,
        "edit",
        json!({ "ops": [{ "op": "set_config_coatings", "param": "1.coating", "names": ["", "AR", "AR"] }] }),
    );
    let r = row("1.coating");
    assert_eq!(r["names"], json!(["", "AR"]));
    assert_eq!(r["values"], json!([0.0, 1.0, 1.0]));
    for (op, param) in [
        ("set_config_coatings", "1.material"),
        ("set_config_materials", "1.coating"),
    ] {
        let e = sv
            .call(
                "edit",
                &json!({ "ops": [{ "op": op, "param": param, "names": ["", "", ""] }] }),
                Some(1),
            )
            .unwrap_err();
        assert!(e.message.contains("is not a"), "{op}: {}", e.message);
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// 読み込んだ後にファイルが書き換わっても、構成の数と構成ごとの系は読み込んだときの中身から引く（読み直さない）
#[test]
fn config_table_uses_the_loaded_file() {
    let sv = Session::new();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let d = std::env::temp_dir().join(format!("lb-config-loaded-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("two.zmx");
    std::fs::write(&f, ZMX).unwrap();
    ok(
        &sv,
        "load",
        json!({ "file": f, "glass_dirs": [format!("{root}/tests/glass")], "cfg": 1 }),
    );
    // 別のアプリが 1 構成のファイルに書き換えた
    let one = ZMX.split("MNUM").next().unwrap();
    std::fs::write(&f, one).unwrap();
    let t = ok(&sv, "config_table", json!({}));
    assert_eq!(t["zmx_cfgs"].as_u64(), Some(2), "{t}");
    let d2 = ok(&sv, "config_diff", json!({}));
    assert_eq!(d2["rows"], 2, "{d2}");
    let _ = std::fs::remove_dir_all(&d);
}

/// 構成で膜が変わる .zmx（COTN）から作る表は膜の行を set_config_coatings で書く
#[test]
fn config_diff_writes_coating_rows() {
    let sv = Session::new();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let d = std::env::temp_dir().join(format!("lb-config-coat-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("coat.zmx");
    let zmx = ZMX.replace(
        "MNUM 2 1\n",
        "MNUM 2 1\nCOTN 1 1 \"\" 0 0 0 1 1 0 0.0 \"\" 0\nCOTN 1 2 \"AR\" 0 0 0 1 1 0 0.0 \"\" 0\n",
    );
    std::fs::write(&f, zmx).unwrap();
    ok(
        &sv,
        "load",
        json!({ "file": f, "glass_dirs": [format!("{root}/tests/glass")], "cfg": 1 }),
    );
    let d2 = ok(&sv, "config_diff", json!({}));
    let op = d2["ops"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["op"] == "set_config_coatings")
        .unwrap_or_else(|| panic!("{d2}"))
        .clone();
    assert_eq!(op["param"], "1.coating");
    assert_eq!(op["names"], json!(["", "AR"]));
    let _ = std::fs::remove_dir_all(&d);
}
