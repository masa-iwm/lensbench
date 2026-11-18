//! 拡張の登録を 1 つの値にまとめる
//!
//! 別の exe（非公開版など）が足すもの（プラグイン・カタログの形式・利用者定義の面・検出器のブロック・解析・誤りの
//! 文の訳）を [`Extensions`] に集め、入口で 1 回 [`Extensions::install`] する。足す側のクレートは `Extensions` を
//! 返す関数を 1 つ持てばよく、入口は呼ぶ順を気にしなくてよい。GUI の見せ方（図・欄）・Python の関数は、それぞれの
//! クレートの拡張トレイト（`lensbench_gui::ext::GuiExtensions`・`lensbench::ext::PyExtensions`）で同じ値に足す
//! （表の欄に入れる関数 [`Extensions::with_ext`] を使う。このクレートは egui・pyo3 を知らない）。
//!
//! 足したものはすべて値の表 [`lensbench_core::ext::Ext`] 1 つになる（[`Extensions::ext`]。検出器の種類・解析は
//! 表の欄。計算ごとに `lensbench_core::ext::with` で渡せる）。install はそれをプロセスの既定に重ねる。
//!
//! 拡張には名前（[`Extensions::named`]）を付けられる。名前は 2 つの拡張が同じ名前のものを足したときの誤りの文に
//! だけ使い、利用者に見える名前（サブコマンド・検出器の type・プラグインの名前）には入らない

use lensbench_analysis::{Analyses, Analysis, DynAnalysis};
use lensbench_core::catalog::LensCatalogSource;
use lensbench_core::ext::Ext;
use lensbench_core::plugin::Registry;
use lensbench_core::usersurf::SurfaceModel;
use lensbench_project::detectors::DetectorKind;
use std::collections::BTreeSet;
use std::sync::Arc;

/// 表に足す関数（[`Extensions::with_ext`]）
type Hook = Arc<dyn Fn(&mut Ext) + Send + Sync>;

/// 足す拡張の集まり（[`Extensions::install`] でプロセスに入れる）
#[derive(Default, Clone)]
pub struct Extensions {
    name: Option<&'static str>,
    replacing: bool,
    plugins: Vec<fn(&mut Registry)>,
    sources: Vec<Arc<dyn LensCatalogSource + Send + Sync>>,
    surfaces: Vec<Arc<dyn SurfaceModel>>,
    detectors: Vec<DetectorKind>,
    analyses: Vec<Arc<dyn DynAnalysis>>,
    hooks: Vec<Hook>,
    messages: Vec<&'static str>,
}

impl Extensions {
    pub fn new() -> Self {
        Self::default()
    }

    /// 名前の付いた拡張（[`Extensions::merge`] の誤りの文に出る）
    pub fn named(name: &'static str) -> Self {
        Self {
            name: Some(name),
            ..Self::default()
        }
    }

    /// 拡張の名前
    pub fn name(&self) -> Option<&'static str> {
        self.name
    }

    /// [`Extensions::merge`] で、前の拡張と同じ名前のものを置き換えてよい（無ければ誤り）
    pub fn replacing(mut self) -> Self {
        self.replacing = true;
        self
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
    pub fn analysis<A: Analysis>(mut self, a: A) -> Self {
        let name = a.meta().name;
        let mut t = Analyses::default();
        t.add(a);
        self.analyses.extend(t.get(name).cloned());
        self
    }

    /// 表に足す関数（GUI の見せ方・Python の関数など、このクレートが型を知らない欄を足す拡張トレイトが使う）。
    /// install・ext のときに、ほかのものを足した後で足した順に呼ぶ
    pub fn with_ext(mut self, f: impl Fn(&mut Ext) + Send + Sync + 'static) -> Self {
        self.hooks.push(Arc::new(f));
        self
    }

    /// 誤りの文の訳の表（`lensbench_core::i18n::add_table` と同じ形の TSV）。install のときに足す
    pub fn messages(mut self, table: &'static str) -> Self {
        self.messages.push(table);
        self
    }

    /// 足した誤りの文の訳の表
    pub fn message_tables(&self) -> &[&'static str] {
        &self.messages
    }

    /// 足した解析の名前（足した順）
    pub fn analysis_names(&self) -> Vec<&'static str> {
        self.analyses.iter().map(|a| a.meta().name).collect()
    }

    /// 足した検出器の種類
    pub fn detector_kinds(&self) -> &[DetectorKind] {
        &self.detectors
    }

    /// 足した利用者定義の面のモデル
    pub fn surface_models(&self) -> &[Arc<dyn SurfaceModel>] {
        &self.surfaces
    }

    /// other の分を後ろに足す。2 つが同じ名前のもの（解析・検出器・プラグイン・面のモデル・カタログの拡張子）を
    /// 足していれば誤り（other が [`Extensions::replacing`] なら other の方が勝つ）
    pub fn merge(mut self, other: Extensions) -> lensbench_core::Result<Self> {
        if !other.replacing {
            let both: Vec<String> = self.names().intersection(&other.names()).cloned().collect();
            if !both.is_empty() {
                let who = |e: &Extensions| e.name.unwrap_or("(unnamed)");
                lensbench_core::invalid!(
                    "extensions {} and {} both add {}",
                    who(&self),
                    who(&other),
                    both.join(", ")
                );
            }
        }
        self.plugins.extend(other.plugins);
        self.sources.extend(other.sources);
        self.surfaces.extend(other.surfaces);
        self.detectors.extend(other.detectors);
        self.analyses.extend(other.analyses);
        self.hooks.extend(other.hooks);
        self.messages.extend(other.messages);
        Ok(self)
    }

    /// 足すものの名前（種類を頭に付ける）
    fn names(&self) -> BTreeSet<String> {
        let mut reg = Registry::default();
        for f in &self.plugins {
            f(&mut reg);
        }
        let mut n: BTreeSet<String> = reg
            .list()
            .into_iter()
            .map(|(k, _)| format!("plugin {k}"))
            .collect();
        n.extend(
            self.analysis_names()
                .iter()
                .map(|a| format!("analysis {a}")),
        );
        n.extend(
            self.detectors
                .iter()
                .map(|d| format!("detector {}", d.name)),
        );
        n.extend(
            self.surfaces
                .iter()
                .map(|m| format!("surface model {}", m.name())),
        );
        for s in &self.sources {
            n.extend(
                s.extensions()
                    .iter()
                    .map(|e| format!("catalog format .{e}")),
            );
        }
        n
    }

    /// 足したものの表（同梱の分の上に足したもの）。名前が組み込みと重なれば誤り
    pub fn ext(&self) -> lensbench_core::Result<Ext> {
        let mut e = Ext::default();
        self.add_to(&mut e)?;
        Ok(e)
    }

    /// 表 e に足す（同じ名前は置き換える）。名前が組み込みと重なれば誤り
    pub fn add_to(&self, e: &mut Ext) -> lensbench_core::Result<()> {
        for s in &self.sources {
            e.add_catalog_source(s.clone());
        }
        for m in &self.surfaces {
            e.add_surface_model(m.clone());
        }
        for f in &self.plugins {
            f(e.plugins_mut());
        }
        for k in &self.detectors {
            lensbench_project::detectors::add(e, *k)?;
        }
        for a in &self.analyses {
            crate::registry::add_dyn(e, a.clone())?;
        }
        for h in &self.hooks {
            h(e);
        }
        Ok(())
    }

    /// プロセスの既定に入れる（何度呼んでもよい。同じ名前は置き換える）。名前が組み込みと重なれば誤り（何も入れない）。
    /// 誤りの文の訳も足す（同じ表は 1 回だけ）
    pub fn install(self) -> lensbench_core::Result<()> {
        lensbench_core::ext::update_default(|e| {
            let mut n = e.clone();
            self.add_to(&mut n)?;
            *e = n;
            Ok::<(), lensbench_core::Error>(())
        })?;
        add_messages(&self.messages);
        Ok(())
    }
}

/// 誤りの文の訳の表を足す（同じ表は 1 回だけ）
fn add_messages(tables: &[&'static str]) {
    use lensbench_core::LockExt;
    static ADDED: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
    let mut added = ADDED.lock_ok();
    for t in tables {
        let key = t.as_ptr() as usize;
        if !added.contains(&key) {
            added.push(key);
            lensbench_core::i18n::add_table(t);
        }
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
        a.merge(b).unwrap().install().unwrap();
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
        // 表を値で渡せば、その計算だけに効く（プロセスの既定に漏れない）
        let e = Extensions::new()
            .detector(k("ext-test-scoped"))
            .ext()
            .unwrap();
        lensbench_core::ext::with(Arc::new(e), || {
            assert!(lensbench_project::detectors::get("ext-test-scoped").is_some());
        });
        assert!(lensbench_project::detectors::get("ext-test-scoped").is_none());
    }

    /// 2 つの拡張が同じ名前のものを足すと merge が誤り（replacing なら後の方が勝つ）。表に足す関数も merge で残る
    #[test]
    fn merge_rejects_the_same_name_from_two_extensions() {
        let k = DetectorKind {
            name: "ext-test-twice",
            doc: "",
            check: |_| Ok(()),
            schema: None,
        };
        let a = Extensions::named("a").detector(k);
        let b = Extensions::named("b").detector(k);
        let e = a
            .clone()
            .merge(b)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            e.contains("extensions a and b both add detector ext-test-twice"),
            "{e}"
        );
        let b = Extensions::named("b").detector(k).replacing();
        assert!(a.clone().merge(b).is_ok());

        #[derive(Clone, Default)]
        struct Mark(u32);
        let c = Extensions::named("c").with_ext(|e| e.slot_mut::<Mark>().0 += 1);
        let t = a.merge(c).unwrap().ext().unwrap();
        assert_eq!(t.slot::<Mark>().map(|m| m.0), Some(1));
    }
}
