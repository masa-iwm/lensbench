//! 足したものの試験: 足し忘れの検査と、CLI・serve と同じ呼び出しで解析とプラグインが動くこと、形の約束

use lensbench_extension_example::extensions;

#[test]
fn nothing_is_missing() {
    assert_eq!(lensbench_sdk::check(&extensions()), Vec::<String>::new());
    // 組み込みと名前が重ならない（Web 版の入口はここが通る前提で expect する）
    extensions().install().unwrap();
}

#[test]
fn the_plugin_runs() {
    use lensbench_sdk::plugin::{Array, Arrays, Config, Context, Value};
    let table = extensions().ext().unwrap();
    let p = table.plugins().get("example_gain").unwrap();
    let cfg = Config::from([("gain".to_string(), Value::Num(3.0))]);
    let out = p
        .prepare(&cfg)
        .unwrap()
        .run(
            &Context::default(),
            &Arrays::from([(
                "x".to_string(),
                Array::f64(vec![2], vec![1.0, 2.0]).unwrap(),
            )]),
        )
        .unwrap();
    assert_eq!(out["y"].as_f64().unwrap(), [3.0, 6.0]);
}

#[test]
fn the_coatings_are_found() {
    use lensbench_sdk::coating::check_coatings;
    let table = extensions().ext().unwrap();
    let cat = table.find_coating("example_ar").unwrap();
    assert_eq!(cat.name, "EXAMPLE_COATINGS");
    check_coatings(cat.set()).unwrap();
    let dir = std::env::temp_dir().join(format!("example-layers-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("MY_AR.layers");
    std::fs::write(&p, "# MgF2 on top of ZrO2\n1.38 0.0996\n2.05 0.0671\n").unwrap();
    let set = table
        .coating_source_for("layers")
        .unwrap()
        .load(&p)
        .unwrap();
    check_coatings(&set).unwrap();
    assert_eq!(set.coat["MY_AR"].len(), 2);
}

#[test]
fn the_shape_keeps_the_contract() {
    use lensbench_sdk::plugin::{Config, Value};
    use lensbench_sdk::shape::check_shape;
    let cfg = Config::from([("a".to_string(), Value::Num(2e-4))]);
    check_shape(&lensbench_extension_example::Saddle, &cfg, 10.0).unwrap();
}
