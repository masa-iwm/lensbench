//! 足したものの試験: 足し忘れの検査と、CLI・serve と同じ呼び出しで解析とプラグインが動くこと

use lensbench_extension_example::extensions;

#[test]
fn nothing_is_missing() {
    assert_eq!(lensbench_sdk::check(&extensions()), Vec::<String>::new());
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
