//! 利用者が定義する材料の口: 屈折率を返すモデルを、名前つきの「カタログ」の単位で拡張の表に足す
//! （[`crate::ext::Ext::add_material_catalog`]）。
//!
//! 足したカタログは AGF のカタログと同じに名前で引ける（.zmx の GCAT・プロジェクトの `glass_catalogs`・
//! `名前@カタログ`）。フォルダに同じ名前の AGF があっても登録したカタログが先（系ファイルを読むときは注意を返す）。同梱のカタログ
//! （SCHOTT・OHARA）と同じ名前は `Extensions` を入れる時（`install`・`ext`）の誤り。中のモデルは系のガラスの表（[`Glass`]）に分散式の枝の 1 つとして入るので、屈折率を求める所・
//! dn・disp・分散の範囲の注意・内部透過率は AGF のガラスと同じ経路を通る。カタログを足していない exe・プロセスでは、
//! AGF のカタログが無いときと同じ誤りになる。
//!
//! 約束:
//! - [`MaterialModel::index`] は系の空気に対する相対屈折率を、系の温度 [°C]・気圧 [atm] で返す（空気・温度の
//!   換算はモデルが受け持つ。AGF の熱の係数は使わない）。システムの「屈折率を環境に合わせる」がオフのときは、
//!   [`MaterialModel::tref`] と 1 atm で呼ぶ。状態を持たず、どのスレッドから呼んでもよい
//! - [`MaterialModel::range`] の外の波長は AGF の LD と同じく注意になる（値はモデルが返したまま）
//! - 屈折率の微分（変数）は、どの媒質にも効く `dn`・`disp` で受ける（モデル固有の設定は持たない）
//! - 屈折率は求める時の拡張の表で決まる（系はモデルを持たない。面の形と違う）。表の外で系の温度などを変えると誤り

use std::collections::HashMap;
use std::sync::Arc;

use crate::glass::Glass;

/// 分散式の番号のうち、利用者の材料のモデルを表すもの（AGF の 1〜13 と重ならない）
pub const USER_FORMULA: i32 = 0;

/// 利用者が定義する材料（名前で引く。設定は持たない）
pub trait MaterialModel: Send + Sync {
    /// 材料の名前（カタログの中で一意）
    fn name(&self) -> &str;
    /// 1 行の説明
    fn doc(&self) -> String {
        String::new()
    }
    /// 系の空気に対する相対屈折率（波長 wl [µm]、系の温度 t [°C]、気圧 p [atm]）
    fn index(&self, wl: f64, t: f64, p: f64) -> f64;
    /// 基準の温度 [°C]（屈折率を環境に合わせないときに使う。既定 20）
    fn tref(&self) -> f64 {
        20.0
    }
    /// 有効な波長の範囲 [µm]（外は注意。既定は範囲なし）
    fn range(&self) -> Option<[f64; 2]> {
        None
    }
    /// 内部透過率の表: (波長 [µm], 内部透過率, 厚さ [mm])。AGF の IT 行と同じ扱い（既定は無し = 吸収なし）
    fn transmittance(&self) -> Vec<[f64; 3]> {
        Vec::new()
    }
}

/// 名前つきの材料の集まり（カタログの単位）
#[derive(Clone)]
pub struct MaterialCatalog {
    pub name: String,
    /// 材料（読むのは [`MaterialCatalog::models`]。後から書き換えると名前の索引とずれるので外には出さない）
    models: Vec<Arc<dyn MaterialModel>>,
    /// 名前 → models の番号（屈折率を求めるたびに引くので）
    index: HashMap<String, usize>,
}

impl std::fmt::Debug for MaterialCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaterialCatalog")
            .field("name", &self.name)
            .field(
                "models",
                &self.models.iter().map(|m| m.name()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl MaterialCatalog {
    /// 名前 name のカタログ（同じ名前のモデルは後のものを引く）
    pub fn new(name: &str, models: Vec<Arc<dyn MaterialModel>>) -> Self {
        let index = models
            .iter()
            .enumerate()
            .map(|(i, m)| (m.name().to_string(), i))
            .collect();
        MaterialCatalog {
            name: name.to_string(),
            models,
            index,
        }
    }

    /// 系のガラスの表に入れる形（名前 → ガラス）
    pub fn glasses(&self) -> HashMap<String, Glass> {
        self.models
            .iter()
            .map(|m| {
                let g = Glass {
                    name: m.name().to_string(),
                    catalog: self.name.clone(),
                    formula: USER_FORMULA,
                    c: Vec::new(),
                    td: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, m.tref()],
                    ld: m.range(),
                    it: m.transmittance(),
                };
                (g.name.clone(), g)
            })
            .collect()
    }

    /// 材料（並びは登録の順）
    pub fn models(&self) -> &[Arc<dyn MaterialModel>] {
        &self.models
    }

    /// 名前の材料
    pub fn model(&self, name: &str) -> Option<&Arc<dyn MaterialModel>> {
        self.index.get(name).map(|&i| &self.models[i])
    }
}

/// 今の拡張の表（[`crate::ext::current`]）のカタログ name（大文字小文字は問わない）の、系のガラスの表に入れる形
pub fn catalog(name: &str) -> Option<HashMap<String, Glass>> {
    crate::ext::current()
        .material_catalog(name)
        .map(|c| c.glasses())
}

/// 系のガラス g（分散式 [`USER_FORMULA`]）の屈折率: 今の拡張の表の、g のカタログ・名前のモデルで（組み立ての時で
/// なく、求める時の表で決まる）。モデルが無ければ NaN（系の組み立て・屈折率の計算は [`missing`] の誤りで先に止める）
pub(crate) fn index(g: &Glass, wl: f64, t: f64, p: f64) -> f64 {
    match crate::ext::current().material_catalog(&g.catalog) {
        Some(c) => c.model(&g.name).map_or(f64::NAN, |m| m.index(wl, t, p)),
        None => f64::NAN,
    }
}

/// 系のガラス g が利用者の材料で、今の拡張の表にそのモデルが無いとき、その誤りの文（あれば Some）
pub fn missing(g: &Glass) -> Option<String> {
    if g.formula != USER_FORMULA {
        return None;
    }
    let e = crate::ext::current();
    match e.material_catalog(&g.catalog) {
        Some(c) if c.model(&g.name).is_some() => None,
        Some(_) => Some(format!(
            "glass {} is a user-defined material, but the registered material catalog {} has no {}",
            g.name, g.catalog, g.name
        )),
        None => Some(format!(
            "glass {} is a user-defined material of catalog {}, but no extension registers that catalog (install the extension that adds it)",
            g.name, g.catalog
        )),
    }
}

/// モデルが約束を守るかを確かめる（拡張の試験から呼ぶ）: 有効な範囲（無ければ 0.4〜0.8 µm）の 9 波長で、
/// 基準の温度・1 atm と 0 °C・40 °C・0.5 atm の屈折率が有限で正、内部透過率の表が 0〜1・厚さが正
pub fn check_material(m: &dyn MaterialModel) -> crate::Result<()> {
    let name = m.name();
    let [lo, hi] = m.range().unwrap_or([0.4, 0.8]);
    if !(lo > 0.0 && lo < hi && hi.is_finite()) {
        crate::invalid!("material model {name}: the range [{lo}, {hi}] µm is not a valid interval");
    }
    let tref = m.tref();
    for i in 0..9 {
        let wl = lo + (hi - lo) * i as f64 / 8.0;
        for (t, p) in [(tref, 1.0), (0.0, 1.0), (40.0, 1.0), (tref, 0.5)] {
            let n = m.index(wl, t, p);
            if !(n.is_finite() && n > 0.0) {
                crate::invalid!("material model {name}: the index at {wl} µm, {t} °C, {p} atm is {n} (not a positive number)");
            }
        }
    }
    for &[wl, tr, th] in &m.transmittance() {
        if !(wl > 0.0 && (0.0..=1.0).contains(&tr) && th > 0.0) {
            crate::invalid!("material model {name}: the transmittance row ({wl} µm, {tr}, {th} mm) is out of range");
        }
    }
    Ok(())
}
