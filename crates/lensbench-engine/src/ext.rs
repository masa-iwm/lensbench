//! 拡張の登録を 1 つの値にまとめる（XCT-07）
//!
//! 別の exe（非公開版など）が足すもの（プラグイン・カタログの形式・利用者定義の面・検出器のブロック・解析）を
//! [`Extensions`] に集め、入口で 1 回 [`Extensions::install`] する。足す側のクレートは `Extensions` を返す関数を
//! 1 つ持てばよく、入口は呼ぶ順を気にしなくてよい。GUI の見せ方（図・欄）は `lensbench_gui::ext` で別に足す。
//!
//! 今はプロセスで共通の表（`plugin::global`・`catalog::register_source`・`usersurf::register`・
//! `detectors::register`・[`crate::register`]）へ入れる。表を値として計算に渡す形は後で（XCT-07 の残り）

use lensbench_analysis::Analysis;
use lensbench_core::catalog::LensCatalogSource;
use lensbench_core::plugin::Registry;
use lensbench_core::usersurf::SurfaceModel;
use lensbench_core::RwLockExt;
use lensbench_project::detectors::DetectorKind;
use std::sync::Arc;

type AddAnalysis = Box<dyn FnOnce() -> Result<(), String> + Send>;

/// 足す拡張の集まり（[`Extensions::install`] でプロセスに入れる）
#[derive(Default)]
pub struct Extensions {
    plugins: Vec<fn(&mut Registry)>,
    sources: Vec<Arc<dyn LensCatalogSource + Send + Sync>>,
    surfaces: Vec<Arc<dyn SurfaceModel>>,
    detectors: Vec<DetectorKind>,
    analyses: Vec<AddAnalysis>,
}

impl Extensions {
    pub fn new() -> Self {
        Self::default()
    }

    /// プラグインを足す関数（同梱と同じ名前なら、こちらが勝つ）
    pub fn plugins(mut self, f: fn(&mut Registry)) -> Self {
        self.plugins.push(f);
        self
    }

    /// レンズカタログの形式（同じ拡張子なら後から足したもの）
    pub fn catalog_source(mut self, s: Arc<dyn LensCatalogSource + Send + Sync>) -> Self {
        self.sources.push(s);
        self
    }

    /// 利用者定義の面のモデル
    pub fn surface_model(mut self, m: Arc<dyn SurfaceModel>) -> Self {
        self.surfaces.push(m);
        self
    }

    /// プロジェクトの検出器のブロック
    pub fn detector(mut self, k: DetectorKind) -> Self {
        self.detectors.push(k);
        self
    }

    /// 解析（CLI のサブコマンド・serve の analyze・GUI の解析）
    pub fn analysis<A: Analysis + Send + 'static>(mut self, a: A) -> Self {
        self.analyses.push(Box::new(move || crate::register(a)));
        self
    }

    /// other の分を後ろに足す
    pub fn merge(mut self, other: Extensions) -> Self {
        self.plugins.extend(other.plugins);
        self.sources.extend(other.sources);
        self.surfaces.extend(other.surfaces);
        self.detectors.extend(other.detectors);
        self.analyses.extend(other.analyses);
        self
    }

    /// プロセスで共通の表に入れる（何度呼んでもよい。同じ名前は置き換える）。名前が組み込みと重なれば誤り
    pub fn install(self) -> Result<(), String> {
        for s in self.sources {
            lensbench_core::catalog::register_source(s);
        }
        for m in self.surfaces {
            lensbench_core::usersurf::register(m);
        }
        for k in self.detectors {
            lensbench_project::detectors::register(k)?;
        }
        {
            let mut g = lensbench_core::plugin::global().write_ok();
            for f in &self.plugins {
                f(&mut g);
            }
        }
        for a in self.analyses {
            a()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lensbench_core::catalog::CatalogLens;
    use std::path::Path;

    struct Dummy;
    impl LensCatalogSource for Dummy {
        fn extensions(&self) -> &[&str] {
            &["lbext-test"]
        }
        fn load(&self, _: &Path) -> Result<Vec<CatalogLens>, String> {
            Ok(Vec::new())
        }
    }

    /// merge した 2 つの分がどちらも入り、組み込みと同じ名前の検出器は誤りになる
    #[test]
    fn install_merged_and_rejects_builtin_names() {
        let k = |name: &'static str| DetectorKind {
            name,
            doc: "",
            check: |_| Ok(()),
            schema: None,
        };
        let a = Extensions::new().catalog_source(Arc::new(Dummy));
        let b = Extensions::new().detector(k("ext-test-detector"));
        a.merge(b).install().unwrap();
        let has = |e: &str| {
            lensbench_core::catalog::sources()
                .iter()
                .any(|s| s.extensions() == [e])
        };
        assert!(has("lbext-test"));
        assert!(lensbench_project::detectors::get("ext-test-detector").is_some());
        let bad = lensbench_project::BUILTIN_BLOCKS[0];
        assert!(Extensions::new().detector(k(bad)).install().is_err());
    }
}
