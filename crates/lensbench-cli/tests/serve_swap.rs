//! serve の swap（GUI）: CLI の swap と同じ結果、count_only、project で渡した組合せ。
//! 系はプロセスに 1 つなので、別のテストの実行ファイルにしている。

use lensbench_engine::Session;
use serde_json::{json, Value};
use std::path::Path;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn call(sv: &Session, m: &str, p: Value) -> Value {
    sv.call(m, &p, None)
        .unwrap_or_else(|e| panic!("{m}: {}", e.message))
}

fn path(f: &str) -> String {
    Path::new(ROOT).join(f).display().to_string()
}

#[test]
fn swap_serve() {
    let sv = Session::new();
    call(
        &sv,
        "load",
        json!({
            "file": path("examples/projects/P18_swap_two.json"),
            "glass_dirs": [path("tests/glass")],
            "lens_catalogs": [path("examples/lenses/LB")],
        }),
    );
    let spec = json!({
        "slots": [{"block": "A", "filter": {"efl": [50, 150]}, "flip": true}],
        "vars": ["gap.t=0.5:20", "bfl.t=1:200"],
        "iters": 20,
    });
    let mut c = spec.clone();
    c["count_only"] = json!(true);
    let n = call(&sv, "swap", c);
    let k = n["slots"][0].as_array().unwrap().len();
    assert!(k >= 2 && k.is_multiple_of(2), "{n}");
    assert_eq!(n["count"], json!(k));
    assert!(n.get("results").is_none());

    let r = call(&sv, "swap", spec.clone());
    assert_eq!(r["count"], json!(k));
    assert_eq!(r["done"], json!(k));
    assert_eq!(r["cancelled"], json!(false));
    let rows = r["results"].as_array().unwrap();
    let m: Vec<f64> = rows
        .iter()
        .map(|x| x["merit"].as_f64().unwrap_or(f64::INFINITY))
        .collect();
    assert!(m.windows(2).all(|w| w[0] <= w[1]), "{m:?}");
    assert!(sv.swap_progress().is_none());

    // CLI（lensbench_design::swap）と同じ
    let p = lensbench_project::Project::from_json(
        &std::fs::read_to_string(path("examples/projects/P18_swap_two.json")).unwrap(),
    )
    .unwrap();
    let cat =
        lensbench_core::catalog::LensCatalog::load_registered(&[path("examples/lenses/LB").into()])
            .unwrap();
    let env = lensbench_project::Env {
        base: Path::new(ROOT).join("examples/projects"),
        glass_dirs: vec![path("tests/glass").into()],
        catalog: None,
    };
    let s = lensbench_design::swap::SwapSearch::new(
        &p,
        &env,
        &cat,
        serde_json::from_value(spec.clone()).unwrap(),
    )
    .unwrap();
    let want = s.run(None);
    assert_eq!(rows[0]["i"], want[0]["i"]);
    assert_eq!(rows[0]["merit"], want[0]["merit"]);

    // project で渡す（GUI の組合せ）、top
    let mut pv = serde_json::to_value(&p).unwrap();
    pv["blocks"][3]["t"] = json!(10.0);
    let mut q = spec.clone();
    q["project"] = pv;
    q["top"] = json!(1);
    let r2 = call(&sv, "swap", q);
    assert_eq!(r2["results"].as_array().unwrap().len(), 1);
    assert_eq!(r2["done"], json!(k));

    // 間違い
    let bad = json!({"slots": [{"block": "nope"}]});
    assert!(sv.call("swap", &bad, None).is_err());
    let bad = json!({"slots": [{"block": "A"}], "extra": 1});
    assert!(sv.call("swap", &bad, None).is_err());
}
