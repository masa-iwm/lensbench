//! 拡張の登録を 1 つの値にまとめる（XCT-07）
//!
//! 別の exe（非公開版など）が足すもの（プラグイン・カタログの形式・利用者定義の面・検出器のブロック・解析）を
//! [`Extensions`] に集め、入口で 1 回 [`Extensions::install`] する。足す側のクレートは `Extensions` を返す関数を
//! 1 つ持てばよく、入口は呼ぶ順を気にしなくてよい。GUI の見せ方（図・欄）は `lensbench_gui::ext` で別に足す。
//!
//! core の分（プラグイン・カタログの形式・面のモデル）は値の表 [`lensbench_core::ext::Ext`] になる
//! （[`Extensions::core_ext`]。計算ごとに `lensbench_core::ext::with` で渡せる）。install はそれをプロセスの既定に
//! 重ねる。検出器のブロック・解析はまだプロセスで共通の表（`detectors::register`・[`crate::register`]）

use lensbench_analysis::Analysis;
use lensbench_core::catalog::LensCatalogSource;
use lensbench_core::ext::Ext;
use lensbench_core::plugin::Registry;
use lensbench_core::usersurf::SurfaceModel;
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

    /// core の分（プラグイン・カタログの形式・面のモデル）の表（同梱の分の上に足したもの）
    pub fn core_ext(&self) -> Ext {
        let mut e = Ext::default();
        self.add_core(&mut e);
        e
    }

    fn add_core(&self, e: &mut Ext) {
        for s in &self.sources {
            e.add_catalog_source(s.clone());
        }
        for m in &self.surfaces {
            e.add_surface_model(m.clone());
        }
        for f in &self.plugins {
            f(e.plugins_mut());
        }
    }

    /// プロセスの既定に入れる（何度呼んでもよい。同じ名前は置き換える）。名前が組み込みと重なれば誤り
    pub fn install(self) -> Result<(), String> {
        lensbench_core::ext::update_default(|e| self.add_core(e));
        for k in self.detectors {
            lensbench_project::detectors::register(k)?;
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
        fn load(&self, _: &Path) -> lensbench_core::Result<Vec<CatalogLens>> {
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
            lensbench_core::ext::current()
                .catalog_sources()
                .iter()
                .any(|s| s.extensions() == [e])
        };
        assert!(has("lbext-test"));
        assert!(lensbench_project::detectors::get("ext-test-detector").is_some());
        let bad = lensbench_project::BUILTIN_BLOCKS[0];
        assert!(Extensions::new().detector(k(bad)).install().is_err());
    }
}
