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
    // DLS の設定: 余弦は 1 を超えないので gtol 1.1 は初めの点で止まる。qr・active_set でも同じ所に着く
    let mut pg = p.clone();
    pg["gtol"] = json!(1.1);
    let rg = call(&sv, "optimize", pg);
    assert_eq!(rg["stop"], json!("small_gradient"));
    assert_eq!(rg["x"], rg["x0"]);
    let mut pq = p.clone();
    pq["qr"] = json!(true);
    pq["active_set"] = json!(true);
    let rq = call(&sv, "optimize", pq);
    let m = r["merit"].as_f64().unwrap();
    assert!((rq["merit"].as_f64().unwrap() - m).abs() < 1e-6 * m, "{rq}");
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
        json!({"vars": ["8.t"], "qr": "yes"}),
        json!({"vars": ["8.t"], "gtol": -1.0}),
    ] {
        assert!(
            lensbench_engine::call("optimize", &bad, None).is_err(),
            "{bad}"
        );
    }
    // DLS の設定の誤りはほかの params の誤りと同じ要求の誤り（−32602）
    for bad in [
        json!({"vars": ["8.t"], "qr": "yes"}),
        json!({"vars": ["8.t"], "gtol": -1.0}),
        json!({"vars": ["8.t"], "gtol": "x"}),
    ] {
        let e = sv.call("optimize", &bad, None).unwrap_err();
        assert_eq!(e.code, -32602, "{bad}: {}", e.message);
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

/// 最適化は始めたときの系で回し、その間も読む・変える要求を止めない。回している間に系が変われば結果を当てない
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
            assert!(r["notes"].to_string().contains("system changed"), "{r}");
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

/// 同じプロセスの呼び手には、結果の型が決まっている method を型のまま返す（JSON を通さない）。JSON に書けば
/// serve の結果と同じ
#[test]
fn typed_answers_match_the_json_results() {
    use lensbench_engine::Answer;
    let sv = Session::new();
    load(&sv, "tests/refs/A0/system.zmx", None);
    let (a, _) = sv.call_answer("surface", &json!({}), None).unwrap();
    let Answer::Surfaces(t) = &a else {
        panic!("{a:?}");
    };
    assert!(!t.surfaces.is_empty());
    assert_eq!(a.into_json().unwrap(), call(&sv, "surface", json!({})));
    // 面を 1 つ指定したときは型のない JSON の答え
    let (a, _) = sv
        .call_answer("surface", &json!({ "surface": 1 }), None)
        .unwrap();
    assert!(a.as_json().is_some(), "{a:?}");
    let p = json!({ "vars": ["8.t"], "merit": "spot" });
    let (a, _) = sv.call_answer("optimize", &p, None).unwrap();
    let Answer::Optimize(t) = &a else {
        panic!("{a:?}");
    };
    assert_eq!(t.names, vec!["8.t".to_string()]);
    assert_eq!(a.into_json().unwrap(), call(&sv, "optimize", p));
    // 解析は結果の型のまま（書き出すときは JSON）
    let p = json!({ "cmd": "spot", "field": 1 });
    let (a, _) = sv.call_answer("analyze", &p, None).unwrap();
    let Answer::Analysis(t) = &a else {
        panic!("{a:?}");
    };
    assert_eq!(t.name(), "spot");
    assert!(
        t.get::<serde_json::Value>().is_none(),
        "spot has its own result type"
    );
    assert_eq!(a.into_json().unwrap(), call(&sv, "analyze", p));
    // 型の無い method は JSON
    let (a, _) = sv.call_answer("ping", &json!({}), None).unwrap();
    assert!(a.as_json().is_some_and(|v| v["version"].is_string()));
}

/// 大域の探索とガラスの置き換え: 結果の global_search・glass、apply で置き換えた材料と dn が読み込んだ系に当たる
#[test]
fn optimize_global_and_glass_apply() {
    let sv = Session::new();
    load(&sv, "tests/refs/A0/system.zmx", None);
    let vars: Vec<Value> = [1, 2, 3, 4, 6, 7]
        .iter()
        .map(|j| json!({"name": format!("{j}.c"), "lo": -0.08, "hi": 0.08}))
        .collect();
    let mut vs = vars.clone();
    vs.push(json!({"name": "3.dn", "lo": -0.05, "hi": 0.05}));
    let p = json!({
        "vars": vs,
        "starts": 4,
        "seed": 3,
        "stopped_as_vignetted": true,
        // カタログを省くと読み込んだ系のガラス（試験の SCHOTT の 4 種）から
        "glass": {"surfaces": [3], "candidates": 3},
        "apply": true,
    });
    let r = call(&sv, "optimize", p);
    let g = &r["global_search"];
    assert_eq!(g["ran"], json!(4));
    assert!(!g["solutions"].as_array().unwrap().is_empty());
    let gl = &r["glass"];
    assert_eq!(gl["changes"][0]["surface"], json!(3));
    let m = gl["materials"][0][1].as_str().unwrap().to_string();
    assert!(
        ["N-SK16", "F2", "N-BK7", "N-LAK9"].contains(&m.as_str()),
        "{m}"
    );
    assert_eq!(r["x"][6], json!(0.0));
    assert_eq!(r["applied"], json!(true), "{r}");
    // 当てた系: 材料が置き換わり、dn は 0、曲率は結果の値
    assert_eq!(
        call(&sv, "get", json!({"param": "3.dn"}))["value"],
        json!(0.0)
    );
    assert_eq!(
        call(&sv, "get", json!({"param": "1.c"}))["value"],
        r["x"][0]
    );
    let sd = call(&sv, "surface_data", json!({"surface": 3}));
    assert_eq!(sd["data"]["mat"].as_str(), Some(m.as_str()), "{sd}");
    // dn を変数にしない面の dn も、置き換えれば 0 にして当てる
    call(&sv, "set", json!({ "values": { "6.dn": 0.01 } }));
    let p6 = json!({"vars": vars, "glass": {"surfaces": [6], "candidates": 2}, "apply": true});
    let r6 = call(&sv, "optimize", p6);
    assert_eq!(r6["applied"], json!(true), "{r6}");
    assert_eq!(
        call(&sv, "get", json!({"param": "6.dn"}))["value"],
        json!(0.0)
    );
    // 間違い: 置き換える面が空気
    let bad = json!({"vars": vars, "glass": {"surfaces": [2]}});
    assert!(sv.call("optimize", &bad, None).is_err());
}

/// 多重構成の表に面の材料の行があるとき: 材料が構成で違えば組ごとに置き換え、apply は行の置き換えた構成の欄を書き換える
#[test]
fn optimize_glass_config_material_row() {
    let sv = Session::new();
    load(&sv, "tests/refs/A0/system.zmx", None);
    call(
        &sv,
        "edit",
        json!({ "ops": [
            { "op": "set_config_count", "n": 3 },
            { "op": "set_config_materials", "param": "3.material", "names": ["F2", "1.7,30", "F2"] },
        ] }),
    );
    let vars: Vec<Value> = [1, 2, 6, 7]
        .iter()
        .map(|j| json!({"name": format!("{j}.c"), "lo": -0.08, "hi": 0.08}))
        .collect();
    let p = json!({
        "vars": vars,
        "configs": [1, 2],
        "glass": {"surfaces": [3], "candidates": 2, "passes": 1, "exclude": ["N-SK16"]},
        "apply": true,
    });
    let r = call(&sv, "optimize", p);
    let gl = &r["glass"];
    let ch = gl["changes"].as_array().unwrap();
    // 構成 2 のモデルガラスは必ず置き換える（構成 2 だけの組）
    let c2 = ch
        .iter()
        .find(|c| c["configs"] == json!([2]))
        .expect("config 2");
    assert_eq!(c2["from"], json!("1.7,30"));
    let to2 = c2["to"].as_str().unwrap().to_string();
    assert_ne!(to2, "N-SK16");
    // 構成 1 の材料は F2 のままか、構成 1 だけの置き換え
    let to1 = ch
        .iter()
        .find(|c| c["configs"] == json!([1]))
        .map_or("F2".to_string(), |c| c["to"].as_str().unwrap().to_string());
    // 構成ごとの最後の材料は、違うときだけ
    if to1 == to2 {
        assert!(gl.get("config_materials").is_none(), "{gl}");
    } else {
        assert_eq!(gl["config_materials"], json!([[3, [to1, to2]]]), "{gl}");
    }
    assert_eq!(r["applied"], json!(true), "{r}");
    // 行の構成 1・2 の欄が置き換わり、問題に入れなかった構成 3 は F2 のまま
    let t = call(&sv, "config_table", json!({}));
    let row = t["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["param"] == "3.material")
        .unwrap()
        .clone();
    let names: Vec<String> = row["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            row["names"][v.as_f64().unwrap() as usize]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(names, [to1, to2, "F2".to_string()], "{row}");
}
