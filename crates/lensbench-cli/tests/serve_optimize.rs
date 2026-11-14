//! serve の optimize: CLI の optimize と同じ結果、apply で読み込んだ系に当たること、構成の並び。
//! 1 つの場でなので、別のテストの実行ファイルにしている（並列のテストと系を取り合わない）。

use lensbench_engine::Session;
use serde_json::{json, Value};
use std::path::Path;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn call(sv: &Session, m: &str, p: Value) -> Value {
    sv.call(m, &p, None)
        .unwrap_or_else(|e| panic!("{m}: {}", e.message))
}

fn load(sv: &Session, file: &str, state: Option<&str>) {
    call(
        sv,
        "load",
        json!({
            "file": Path::new(ROOT).join(file).display().to_string(),
            "glass_dirs": [Path::new(ROOT).join("tests/glass").display().to_string()],
            "state": state,
        }),
    );
}

#[test]
fn optimize_serve() {
    let sv = Session::new();
    // 1 つのテストの中で順に（1 つの場で）
    load(&sv, "tests/refs/A0/system.zmx", None);
    call(&sv, "set", json!({ "values": { "8.t": 0.3 } }));
    let t = json!({"type": "ray", "field": 1, "py": 0.7, "q": "y"});
    let p = json!({
        "vars": ["8.t", {"name": "6.c", "lo": 0.0, "hi": 0.05}],
        "merit": "spot",
        "terms": [t],
    });
    let r = call(&sv, "optimize", p.clone());
    assert!(r["merit"].as_f64().unwrap() < r["merit0"].as_f64().unwrap());
    assert_eq!(r["names"], json!(["8.t", "6.c"]));
    assert_eq!(r["applied"], json!(false));
    // 進み具合は終わると消える
    assert!(sv.optimize_progress().is_none());
    // 足した項の値（既定の項の後）
    let terms = r["terms"].as_array().unwrap();
    assert_eq!(terms.last().unwrap()["default"], json!(false));
    assert!(terms[0]["default"].as_bool().unwrap());
    // 系は変えていない
    assert_eq!(
        call(&sv, "get", json!({"param": "8.t"}))["value"],
        json!(0.3)
    );
    // apply で当てる
    let mut pa = p.clone();
    pa["apply"] = json!(true);
    let ra = call(&sv, "optimize", pa);
    assert_eq!(ra["applied"], json!(true));
    assert_eq!(ra["x"], r["x"]);
    assert_eq!(
        call(&sv, "get", json!({"param": "8.t"}))["value"],
        r["x"][0]
    );
    // 間違い
    for bad in [
        json!({"vars": ["8.t"], "terms": [{"type": "ray", "q": "y"}]}),
        json!({"vars": ["8.t"], "merit": "foo"}),
        json!({"vars": [{"name": "8.t", "configs": [3]}]}),
    ] {
        assert!(
            lensbench_engine::call("optimize", &bad, None).is_err(),
            "{bad}"
        );
    }

    // 構成の並び（プロジェクトの状態）: 2 つの状態の既定の項（視野 4 つずつ）。読み込んだ状態（far）に効かない変数は
    // 当てない
    load(
        &sv,
        "examples/projects/P6_states_focus_group.json",
        Some("far"),
    );
    let r = call(
        &sv,
        "optimize",
        json!({
            "vars": [{"name": "front.shift", "configs": [1]}, {"name": "front.shift", "configs": [2]}],
            "configs": ["far", "near"],
            "iters": 3,
            "apply": true,
        }),
    );
    assert_eq!(r["terms"].as_array().unwrap().len(), 8);
    assert_eq!(r["applicable"], json!([true, false]));
    assert_eq!(r["applied"], json!(true));
    assert_eq!(
        call(&sv, "get", json!({"param": "front.shift"}))["value"],
        r["x"][0]
    );
}

/// 最適化は始めたときの系で回し、その間も読む・変える要求を止めない。回している間に系が変われば結果を当てない（GAR-20）
#[test]
fn optimize_does_not_block_and_skips_stale_apply() {
    let sv = std::sync::Arc::new(Session::new());
    load(&sv, "tests/refs/A0/system.zmx", None);
    let p = json!({
        "vars": ["8.t", {"name": "6.c", "lo": 0.0, "hi": 0.05}],
        "merit": "spot",
        "iters": 1_000_000,
        "apply": true,
    });
    let mut skipped = false;
    for k in 0..5 {
        call(&sv, "set", json!({ "values": { "8.t": 0.3 } }));
        let (s2, p2) = (sv.clone(), p.clone());
        let h = std::thread::spawn(move || call(&s2, "optimize", p2));
        let t0 = std::time::Instant::now();
        while sv.optimize_progress().is_none() && !h.is_finished() {
            assert!(t0.elapsed().as_secs() < 30);
            std::thread::yield_now();
        }
        // 回している間も読める・変えられる
        let v = 1.0 + k as f64;
        call(&sv, "get", json!({"param": "8.t"}));
        call(&sv, "set", json!({ "values": { "8.t": v } }));
        sv.optimize_cancel();
        let r = h.join().unwrap();
        let now = call(&sv, "get", json!({"param": "8.t"}))["value"].clone();
        if r["applied"] == json!(false) {
            // 当てなかった: 後から入れた値が残り、注意が付く
            assert_eq!(now, json!(v));
            assert!(r["notes"].to_string().contains("系が変わった"), "{r}");
            skipped = true;
            break;
        }
    }
    assert!(skipped, "最適化が毎回 set より先に終わった");
}

/// スレッドのプールを使う場（threads ≥ 2）で、apply つきの最適化と別のスレッドの解析を並べても固まらない
/// （長い要求を読む要求のプールの盗みに載せない・場の鍵をプールのスレッドで待たない）
#[test]
fn optimize_with_threads_and_parallel_reads_does_not_hang() {
    let sv = std::sync::Arc::new(Session::new());
    load(&sv, "tests/refs/A0/system.zmx", None);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let readers: Vec<_> = (0..3)
        .map(|_| {
            let (sv, stop) = (sv.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let p = json!({"cmd": "mtf", "freqs": [10, 20, 30]});
                    sv.call("analyze", &p, Some(4)).unwrap();
                }
            })
        })
        .collect();
    let (tx, rx) = std::sync::mpsc::channel();
    let s2 = sv.clone();
    std::thread::spawn(move || {
        let p = json!({"vars": ["1.c", "2.c"], "merit": "spot", "iters": 2, "apply": true});
        for _ in 0..30 {
            s2.call("optimize", &p, Some(4)).unwrap();
        }
        tx.send(()).unwrap();
    });
    let r = rx.recv_timeout(std::time::Duration::from_secs(120));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(r.is_ok(), "最適化が終わらない（固まった）");
    for h in readers {
        h.join().unwrap();
    }
}
