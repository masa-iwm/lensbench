//! 書き出しの形（光線の表・画像）や系の読み方の要る解析の定義。解析の登録簿（[`lensbench_analysis::Analyses`]）に
//! 同梱の解析と並べて足す（lensbench-analysis は書き出しの lensbench-export より下の層なので、ここに置く。段階 2-3 の engine 層へ移す）

mod fringe;
mod trace;

/// 登録簿に足す
pub fn register(a: &mut lensbench_analysis::Analyses) {
    a.add(fringe::Info);
    a.add(trace::Rays);
    a.add(trace::Pop);
    a.add(trace::MechCheck);
    a.add(trace::Elements);
    a.add(fringe::Fringe);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// ここの解析はどれも既定の設定で動く（機構部品の無い系の mech-check、系の読み方の無い fringe は誤り）。
    /// info は系の読み方が無くても系の値を出す
    #[test]
    fn each_runs_with_defaults() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let opt = lensbench_core::zmx::LoadOptions {
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s =
            lensbench_core::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap();
        let cx = lensbench_analysis::Cx::new(&s);
        let mut a = lensbench_analysis::Analyses::default();
        super::register(&mut a);
        for d in a.iter() {
            let name = d.meta().name;
            let p = match name {
                "pop" => json!({ "n": 32, "rays": 11 }),
                _ => json!({}),
            };
            let r = d.run_json(&cx, &p);
            match name {
                "mech-check" => assert!(r.unwrap_err().to_string().contains("機構部品がない")),
                "fringe" => assert!(r.unwrap_err().to_string().contains("読み方")),
                _ => {
                    let v = r.unwrap_or_else(|e| panic!("{name}: {e}"));
                    assert!(v.is_object(), "{name}");
                }
            }
        }
    }
}
