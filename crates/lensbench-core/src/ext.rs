//! 拡張の表（レンズカタログの形式・利用者定義の面のモデル・プラグイン）を 1 つの値 [`Ext`] にまとめる
//! 。
//!
//! 計算は今の文脈（[`crate::par::Ctx`]）の表を使う。文脈に無ければプロセスの既定（[`set_default`]。入口の exe・
//! Python が起動時に 1 回決める）を使う。[`with`] で表を計算ごとに渡せるので、同じプロセスで 2 つの構成を同時に
//! 持てて、試験どうしで登録が漏れない。並列の作業のスレッドへは文脈と一緒に引き継ぐ（[`crate::par`]）。
//!
//! 表は読むだけの値（`Arc<Ext>`）で、足すときは写して足したものを置き直す（[`update_default`]）。
//!
//! 上の層の拡張（プロジェクトの検出器の種類・拡張の解析・GUI の見せ方・Python の模擬の組など）も、型ごとの欄
//! （[`Ext::slot`]・[`Ext::slot_mut`]）として同じ表に載せる。拡張の登録はこの表 1 つ。

use crate::catalog::{LbcatSource, LensCatalogSource, ZmxFileSource};
use crate::error::Result;
use crate::plugin::Registry;
use crate::usersurf::{IdealMapping, SurfaceModel};
use crate::{invalid, RwLockExt};
use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// レンズカタログの形式
pub type CatalogSource = Arc<dyn LensCatalogSource + Send + Sync>;

/// 拡張の表。`Ext::default()` は同梱の分だけ（カタログは .lbcat・.zmx、面のモデルは `ideal_mapping`、
/// プラグインは無し。解析の同梱のプラグインは上の層が重ねる）
#[derive(Clone)]
pub struct Ext {
    /// 足した形式（後から足したものが先に引かれる）。同梱の形式は [`Ext::catalog_sources`] が最後に付ける
    sources: Vec<CatalogSource>,
    surfaces: BTreeMap<String, Arc<dyn SurfaceModel>>,
    plugins: Registry,
    /// 上の層の表（型ごとに 1 つ）
    slots: BTreeMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl Default for Ext {
    fn default() -> Self {
        let mut surfaces: BTreeMap<String, Arc<dyn SurfaceModel>> = BTreeMap::new();
        surfaces.insert("ideal_mapping".into(), Arc::new(IdealMapping));
        Ext {
            sources: Vec::new(),
            surfaces,
            plugins: Registry::new(),
            slots: BTreeMap::new(),
        }
    }
}

impl std::fmt::Debug for Ext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ext")
            .field("sources", &self.sources.len())
            .field("surfaces", &self.surfaces.keys().collect::<Vec<_>>())
            .field("plugins", &self.plugins.list())
            .field("slots", &self.slots.len())
            .finish()
    }
}

impl Ext {
    pub fn new() -> Self {
        Self::default()
    }

    /// レンズカタログの形式を足す（同じ拡張子なら後から足したものを使う）
    pub fn add_catalog_source(&mut self, s: CatalogSource) -> &mut Self {
        self.sources.push(s);
        self
    }

    /// 利用者定義の面のモデルを足す（同じ名前は差し替える）
    pub fn add_surface_model(&mut self, m: Arc<dyn SurfaceModel>) -> &mut Self {
        self.surfaces.insert(m.name().to_string(), m);
        self
    }

    /// 面のモデルを外す（あれば true）
    pub fn remove_surface_model(&mut self, name: &str) -> bool {
        self.surfaces.remove(name).is_some()
    }

    /// プラグインの表
    pub fn plugins(&self) -> &Registry {
        &self.plugins
    }

    /// プラグインの表（足す・外す）
    pub fn plugins_mut(&mut self) -> &mut Registry {
        &mut self.plugins
    }

    /// 上の層の表 T（無ければ None）。検出器の種類・拡張の解析など、core の知らない型の表を載せる欄
    pub fn slot<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.slots
            .get(&TypeId::of::<T>())
            .and_then(|a| a.downcast_ref::<T>())
    }

    /// 上の層の表 T を変える（無ければ既定値で作る。ほかの表と共有している中身は写してから変える）
    pub fn slot_mut<T: Any + Send + Sync + Clone + Default>(&mut self) -> &mut T {
        let id = TypeId::of::<T>();
        let a = self
            .slots
            .entry(id)
            .or_insert_with(|| Arc::new(T::default()));
        if Arc::get_mut(a).is_none() {
            let copy = a.downcast_ref::<T>().cloned().unwrap_or_default();
            *a = Arc::new(copy);
        }
        Arc::get_mut(a)
            .and_then(|x| x.downcast_mut::<T>())
            .expect("the slot was just made unique and of type T")
    }

    /// other の分を足す（形式は後ろに、同じ名前の面のモデル・プラグインは other が勝つ。上の層の表は、other に
    /// ある型の表を other のもので置き換える）
    pub fn merge(&mut self, other: &Ext) -> &mut Self {
        self.sources.extend(other.sources.iter().cloned());
        for (k, m) in &other.surfaces {
            self.surfaces.insert(k.clone(), m.clone());
        }
        for p in other.plugins.plugins() {
            self.plugins.register(p.clone());
        }
        for (k, v) in &other.slots {
            self.slots.insert(*k, v.clone());
        }
        self
    }

    /// カタログを読む形式の一覧（足したものが新しい順に先、最後に同梱の [`LbcatSource`]・[`ZmxFileSource`]）
    pub fn catalog_sources(&self) -> Vec<CatalogSource> {
        let mut v: Vec<CatalogSource> = self.sources.iter().rev().cloned().collect();
        v.push(Arc::new(LbcatSource));
        v.push(Arc::new(ZmxFileSource));
        v
    }

    /// 名前の面のモデル
    pub fn surface_model(&self, name: &str) -> Result<Arc<dyn SurfaceModel>> {
        match self.surfaces.get(name) {
            Some(m) => Ok(m.clone()),
            None => invalid!(
                "no user-defined surface model {name} (registered: {})",
                self.surface_models().join(", ")
            ),
        }
    }

    /// 面のモデルの名前（名前の順）
    pub fn surface_models(&self) -> Vec<String> {
        self.surfaces.keys().cloned().collect()
    }
}

/// プロセスの既定の表（None は `Ext::default()`）
static DEFAULT: RwLock<Option<Arc<Ext>>> = RwLock::new(None);
/// `update_default` どうしを順に並べる（DEFAULT の書き込みロックは f の間持たない）
static UPDATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// プロセスの既定の表を決める（入口の exe・Python が起動時に）
pub fn set_default(e: Arc<Ext>) {
    let _u = UPDATE.lock().unwrap_or_else(|p| p.into_inner());
    *DEFAULT.write_ok() = Some(e);
}

/// プロセスの既定の表を写して f で変え、置き直す（Python から足すプラグインなど）。
/// 置き直す前に始まった計算は前の表のまま。f の中で `current()`・`default_ext()` は呼べる（前の表が返る）が、
/// `update_default`・`set_default` を呼ぶと止まる
pub fn update_default<R>(f: impl FnOnce(&mut Ext) -> R) -> R {
    let _u = UPDATE.lock().unwrap_or_else(|p| p.into_inner());
    let mut e = DEFAULT.read_ok().as_deref().cloned().unwrap_or_default();
    let r = f(&mut e);
    *DEFAULT.write_ok() = Some(Arc::new(e));
    r
}

/// プロセスの既定の表
pub fn default_ext() -> Arc<Ext> {
    DEFAULT
        .read_ok()
        .clone()
        .unwrap_or_else(|| Arc::new(Ext::default()))
}

/// 今の計算の表（文脈で渡したもの、無ければプロセスの既定）
pub fn current() -> Arc<Ext> {
    crate::par::current_ext().unwrap_or_else(default_ext)
}

/// f を表 e の下で回す（文脈のほかの項目は引き継ぐ）。表が引き継がれるのは `par` の並列（`par::scoped` など）の
/// 作業のスレッドまでで、f の中で自分で作ったスレッド（`std::thread::spawn`・rayon を直に使う所）ではプロセスの既定に戻る
pub fn with<R>(e: Arc<Ext>, f: impl FnOnce() -> R) -> R {
    crate::par::with_ext(e, f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CatalogLens;
    use std::path::Path;

    struct Src(&'static str);
    impl LensCatalogSource for Src {
        fn extensions(&self) -> &[&str] {
            std::slice::from_ref(&self.0)
        }
        fn load(&self, _: &Path) -> Result<Vec<CatalogLens>> {
            Ok(Vec::new())
        }
    }

    fn exts(e: &Ext) -> Vec<String> {
        e.catalog_sources()
            .iter()
            .map(|s| s.extensions()[0].to_string())
            .collect()
    }

    /// 文脈で渡した表は、その計算と作業のスレッドだけに効き、外（既定）には漏れない
    #[test]
    fn scoped_table_does_not_leak() {
        let mut a = Ext::new();
        a.add_catalog_source(Arc::new(Src("lbext-a")));
        let a = Arc::new(a);
        with(a.clone(), || {
            assert_eq!(exts(&current())[0], "lbext-a");
            let v = crate::with_threads(Some(3), || {
                crate::par::map(&[1, 2, 3, 4], |_| exts(&current())[0].clone())
            });
            assert!(v.iter().all(|x| x == "lbext-a"), "{v:?}");
        });
        assert!(!exts(&current()).contains(&"lbext-a".to_string()));
        // 同梱の形式は最後、足したものは新しい順
        let mut b = Ext::new();
        b.add_catalog_source(Arc::new(Src("x")))
            .add_catalog_source(Arc::new(Src("y")));
        assert_eq!(exts(&b), ["y", "x", "lbcat", "zmx"]);
    }

    /// 同梱の面のモデルがあり、無い名前は誤り。merge は other が勝つ
    #[test]
    fn surface_models_and_merge() {
        let e = Ext::default();
        assert!(e.surface_model("ideal_mapping").is_ok());
        assert!(e.surface_model("nope").is_err());
        let mut a = Ext::new();
        a.remove_surface_model("ideal_mapping");
        assert!(a.surface_models().is_empty());
        a.merge(&e);
        assert_eq!(a.surface_models(), ["ideal_mapping"]);
    }

    /// 上の層の表は型ごと。写した表を変えても元は変わらない
    #[test]
    fn slots_are_per_type_and_copy_on_write() {
        #[derive(Clone, Default, Debug, PartialEq)]
        struct Names(Vec<&'static str>);
        let mut a = Ext::new();
        assert!(a.slot::<Names>().is_none());
        a.slot_mut::<Names>().0.push("x");
        let mut b = a.clone();
        b.slot_mut::<Names>().0.push("y");
        assert_eq!(a.slot::<Names>().unwrap().0, ["x"]);
        assert_eq!(b.slot::<Names>().unwrap().0, ["x", "y"]);
        assert!(a.slot::<u32>().is_none());
        with(Arc::new(b), || {
            assert_eq!(current().slot::<Names>().unwrap().0.len(), 2);
        });
    }

    #[test]
    fn update_default_can_read_the_table() {
        // f の中で今の表を読んでも止まらない（表は変えない: ほかの試験と共有のプロセスの既定）
        let n = update_default(|_| exts(&current()).len());
        assert_eq!(n, exts(&default_ext()).len());
    }
}
