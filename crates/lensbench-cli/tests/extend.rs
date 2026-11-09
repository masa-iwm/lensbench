//! 別の exe に組み込むときの差し込み口: プロセスで共通に登録したカタログの形式・プラグインが、
//! CLI（と、同じ API を呼ぶ GUI）に効くこと

use std::path::Path;
use std::sync::Arc;

use lensbench_core::catalog::{self, CatalogLens, LensCatalogSource};
use lensbench_core::plugin::{self, Arrays, Config, Context, Instance, Kind, Plugin};

/// 中身が .zmx の本文で、拡張子だけ違う形式（メーカーは "EXT" 固定）
struct ExtSource;

impl LensCatalogSource for ExtSource {
    fn extensions(&self) -> &[&str] {
        &["lbext"]
    }
    fn load(&self, path: &Path) -> Result<Vec<CatalogLens>, String> {
        let zmx = lensbench_core::glass::read_text(path).map_err(|e| e.to_string())?;
        Ok(vec![CatalogLens {
            vendor: "EXT".into(),
            part: path.file_stem().unwrap().to_string_lossy().into_owned(),
            efl: f64::NAN,
            zmx,
        }])
    }
}

struct Echo;
struct EchoInst;

impl Plugin for Echo {
    fn name(&self) -> &str {
        "test_ext_echo"
    }
    fn kind(&self) -> Kind {
        Kind::Analysis
    }
    fn prepare(&self, _config: &Config) -> lensbench_core::Result<Box<dyn Instance>> {
        Ok(Box::new(EchoInst))
    }
}

impl Instance for EchoInst {
    fn run(&self, _ctx: &Context<'_>, inputs: &Arrays) -> lensbench_core::Result<Arrays> {
        Ok(inputs.clone())
    }
}

#[test]
fn registered_source_and_plugin() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("lb_extend_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        root.join("examples/lenses/LB/BCX-50.zmx"),
        dir.join("BCX-50.lbext"),
    )
    .unwrap();
    let call = |m: &str| {
        lensbench_cli::serve::call(
            m,
            &serde_json::json!({ "lens_catalog": [dir], "glass_dir": [root.join("tests/glass")] }),
            Some(1),
        )
    };

    // 登録前は .lbext を飛ばす（フォルダの中の関係のないファイル）
    let before = call("lenses").unwrap().to_string();
    assert!(!before.contains("EXT"), "{before}");

    catalog::register_source(Arc::new(ExtSource));
    let after = call("lenses").unwrap().to_string();
    assert!(
        after.contains("\"EXT\"") && after.contains("BCX-50"),
        "{after}"
    );

    plugin::global().write().unwrap().register(Arc::new(Echo));
    let reg = lensbench_cli::builtin_plugins();
    assert!(reg.get("test_ext_echo").is_ok());
    // 同梱の分も残る
    assert!(reg.get("geo_mtf").is_ok());

    std::fs::remove_dir_all(&dir).ok();
}
