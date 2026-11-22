//! 別の exe に組み込むときの差し込み口: 拡張の表（`lensbench_core::ext::Ext`）に足したカタログの形式・プラグインが、
//! CLI（と、同じ API を呼ぶ GUI）に効くこと。表は計算ごとに渡すので、ほかの試験には漏れない

use std::path::Path;
use std::sync::Arc;

use lensbench_core::catalog::{CatalogLens, LensCatalogSource};
use lensbench_core::ext::{self, Ext};
use lensbench_core::plugin::{Arrays, Config, Context, Instance, Kind, Plugin};

/// 中身が .zmx の本文で、拡張子だけ違う形式（メーカーは "EXT" 固定）
struct ExtSource;

impl LensCatalogSource for ExtSource {
    fn extensions(&self) -> &[&str] {
        &["lbext"]
    }
    fn load(&self, path: &Path) -> lensbench_core::Result<Vec<CatalogLens>> {
        let zmx = lensbench_core::glass::read_text(path)
            .map_err(|e| lensbench_core::Error::Io(e.to_string()))?;
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
        lensbench_engine::call(
            m,
            &serde_json::json!({ "lens_catalogs": [dir], "glass_dirs": [root.join("tests/glass")] }),
            Some(1),
        )
    };

    // 表に無ければ .lbext を飛ばす（フォルダの中の関係のないファイル）
    let before = call("lenses").unwrap().to_string();
    assert!(!before.contains("EXT"), "{before}");

    let mut e = Ext::default();
    e.add_catalog_source(Arc::new(ExtSource));
    e.plugins_mut().register(Arc::new(Echo));
    ext::with(Arc::new(e), || {
        let after = call("lenses").unwrap().to_string();
        assert!(
            after.contains("\"EXT\"") && after.contains("BCX-50"),
            "{after}"
        );
        let reg = lensbench_engine::builtin_plugins();
        assert!(reg.get("test_ext_echo").is_ok());
        // 同梱の分も残る
        assert!(reg.get("geo_mtf").is_ok());
    });
    // 表の外には漏れない
    let again = call("lenses").unwrap().to_string();
    assert!(!again.contains("EXT"), "{again}");
    assert!(lensbench_engine::builtin_plugins()
        .get("test_ext_echo")
        .is_err());

    std::fs::remove_dir_all(&dir).ok();
}

/// 拡張の表に足した膜の一覧が、系の膜の一覧（CLI の coatings・serve の coatings と同じ関数）に効き、由来が出る
#[test]
fn registered_coating_catalog() {
    use lensbench_core::usercoating::CoatingSet;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = lensbench_formats::zmx::LoadOptions {
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let mut s =
        lensbench_formats::zmx::load_with(&root.join("examples/lenses/LB/BCX-50.zmx"), &opt)
            .unwrap();
    s.edit_raw(|p| {
        p.surfaces[1].coat = "TEST_EXT_AR".into();
        Ok(())
    })
    .unwrap();
    let kinds = |c: &lensbench_engine::load::CoatInterfaces| {
        c.list
            .iter()
            .filter(|v| v["key"].as_str().unwrap().starts_with("TEST_EXT_AR|"))
            .map(|v| v["kind"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let c = lensbench_engine::load::coat_interfaces(&s, &[]).unwrap();
    assert_eq!(kinds(&c), ["missing"]);
    let mut e = Ext::default();
    e.add_coating_catalog(
        "TEST",
        CoatingSet::parse_dat("MATE M\n0.5 1.38 0\nCOAT TEST_EXT_AR\nM 0.25\n").unwrap(),
    );
    ext::with(Arc::new(e), || {
        let c = lensbench_engine::load::coat_interfaces(&s, &[]).unwrap();
        assert_eq!(kinds(&c), ["stack"]);
        assert_eq!(
            c.registered,
            [("TEST_EXT_AR".to_string(), "TEST".to_string())]
        );
        assert!(c.needs.is_empty());
    });
}

/// 系の膜の注意（登録した一覧の膜が膜ファイルの同じ名前の膜より先に使われた、など）が、interference の polarized の結果と
/// serve の plugin（系を渡すもの）の返りの warnings にも出る。polarized なしの interference には出ない
#[test]
fn coating_notes_in_interference_and_plugin() {
    use lensbench_core::usercoating::CoatingSet;
    use serde_json::json;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("lb_extend_notes_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("COATING.DAT"),
        "MATE M\n0.4 1.38 0\n0.8 1.38 0\nCOAT TEST_NOTE_AR\nM 0.1\n",
    )
    .unwrap();
    let opt = lensbench_formats::zmx::LoadOptions {
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let mut s =
        lensbench_formats::zmx::load_with(&root.join("examples/lenses/LB/BCX-50.zmx"), &opt)
            .unwrap();
    s.edit_raw(|p| {
        p.surfaces[1].coat = "TEST_NOTE_AR".into();
        p.coating_file = Some(dir.join("COATING.DAT"));
        Ok(())
    })
    .unwrap();
    let zmx = dir.join("sys.zmx");
    lensbench_formats::zmx_write::save(&zmx, &s).unwrap();

    let mut e = Ext::default();
    e.add_coating_catalog(
        "TEST_NOTES",
        CoatingSet::parse_dat("MATE M\n0.4 1.38 0\n0.8 1.38 0\nCOAT TEST_NOTE_AR\nM 0.25\n")
            .unwrap(),
    );
    let noted = |v: &serde_json::Value| {
        v["warnings"].as_array().is_some_and(|w| {
            w.iter().any(|x| {
                x.as_str().is_some_and(|x| {
                    x.starts_with("coating TEST_NOTE_AR:") && x.contains("TEST_NOTES")
                })
            })
        })
    };
    ext::with(Arc::new(e), || {
        let sess = lensbench_engine::Session::new();
        sess.call(
            "load",
            &json!({ "file": zmx, "glass_dirs": [root.join("tests/glass")] }),
            Some(1),
        )
        .unwrap();
        let p = json!({ "cmd": "interference", "beam1": "1", "beam2": "1", "mode": "detector",
                        "field": 1, "pupil": 8, "npx": 8 });
        let off = sess.call("analyze", &p, Some(1)).unwrap();
        assert!(!noted(&off), "{off}");
        let mut q = p.clone();
        q["polarized"] = json!(true);
        let on = sess.call("analyze", &q, Some(1)).unwrap();
        assert!(noted(&on), "{on}");
        let g = sess
            .call("plugin", &json!({ "name": "ghost" }), Some(1))
            .unwrap();
        assert!(noted(&g), "{g}");
    });
    std::fs::remove_dir_all(&dir).ok();
}

/// 係数を計算で返す膜（利用者の膜）は、系の膜の一覧で種類 user・注記がモデルの doc になる
#[test]
fn a_user_coating_is_listed_as_user() {
    use lensbench_core::coating::C64;
    use lensbench_core::usercoating::{CoatCoefs, CoatInput, CoatingModel, CoatingSet};
    #[derive(Debug)]
    struct Half;
    impl CoatingModel for Half {
        fn coefs(&self, _: &CoatInput) -> lensbench_core::Result<CoatCoefs> {
            let t = C64::new(0.5f64.sqrt(), 0.0);
            Ok(CoatCoefs {
                ts: t,
                tp: t,
                ..Default::default()
            })
        }
        fn doc(&self) -> String {
            "half".into()
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = lensbench_formats::zmx::LoadOptions {
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let mut s =
        lensbench_formats::zmx::load_with(&root.join("examples/lenses/LB/BCX-50.zmx"), &opt)
            .unwrap();
    s.edit_raw(|p| {
        p.surfaces[1].coat = "TEST_USER_HALF".into();
        Ok(())
    })
    .unwrap();
    let mut set = CoatingSet::new();
    set.add_user("TEST_USER_HALF", Arc::new(Half));
    let mut e = Ext::default();
    e.add_coating_catalog("TEST_USER", set);
    ext::with(Arc::new(e), || {
        let c = lensbench_engine::load::coat_interfaces(&s, &[]).unwrap();
        let it = c
            .list
            .iter()
            .find(|v| v["key"].as_str().unwrap().starts_with("TEST_USER_HALF|"))
            .unwrap();
        assert_eq!(it["kind"], "user", "{it}");
        assert_eq!(it["note"], "half", "{it}");
    });
}
