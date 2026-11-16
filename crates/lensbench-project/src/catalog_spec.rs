//! カタログの品の種類・名前の決まった仕様・ほかのカタログへ足す中身（ユーザーカタログの形の一部）。
//!
//! ユーザーカタログ（[`crate::UserCatalog`]）の品は `kind`・`url`・`specs`・`tags`・出典を持てる。
//! カタログ全体の `url_template` と `overlay` は、同じメーカーのほかのカタログ（ZMF・.zmx の玉など）の品にも
//! 仕様とリンクを足す（元のファイルは書き換えない）。足し方・仕様での絞り込み・検索は lensbench-catalog（PRJ-05）。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// 品の種類（一覧での分類。ブロックの種類とは別: ウィンドウ・偏光子も今は玉・面の並びのブロックで書ける）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Lens,
    Mirror,
    Splitter,
    Polarizer,
    Waveplate,
    Window,
    Filter,
    Prism,
    /// レンズチューブ（機構）
    Tube,
    /// ケージ板
    #[serde(rename = "cage_plate")]
    CagePlate,
    /// マウント（素子の保持）
    Mount,
    /// ケージのロッド
    Rod,
    /// アダプタ・スペーサ・止め輪など
    Adapter,
    Other,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemKind::Lens => "lens",
            ItemKind::Mirror => "mirror",
            ItemKind::Splitter => "splitter",
            ItemKind::Polarizer => "polarizer",
            ItemKind::Waveplate => "waveplate",
            ItemKind::Window => "window",
            ItemKind::Filter => "filter",
            ItemKind::Prism => "prism",
            ItemKind::Tube => "tube",
            ItemKind::CagePlate => "cage_plate",
            ItemKind::Mount => "mount",
            ItemKind::Rod => "rod",
            ItemKind::Adapter => "adapter",
            ItemKind::Other => "other",
        }
    }
    /// 機構部品か（光学の面を持たず、`mech` の寸法を持つ）
    pub fn is_mech(self) -> bool {
        matches!(
            self,
            ItemKind::Tube
                | ItemKind::CagePlate
                | ItemKind::Mount
                | ItemKind::Rod
                | ItemKind::Adapter
        )
    }
    pub const ALL: [ItemKind; 14] = [
        ItemKind::Lens,
        ItemKind::Mirror,
        ItemKind::Splitter,
        ItemKind::Polarizer,
        ItemKind::Waveplate,
        ItemKind::Window,
        ItemKind::Filter,
        ItemKind::Prism,
        ItemKind::Tube,
        ItemKind::CagePlate,
        ItemKind::Mount,
        ItemKind::Rod,
        ItemKind::Adapter,
        ItemKind::Other,
    ];
    pub fn parse(s: &str) -> Option<ItemKind> {
        ItemKind::ALL
            .into_iter()
            .find(|k| k.as_str().eq_ignore_ascii_case(s.trim()))
    }
}

/// 名前の決まった仕様: (名前, 単位, 説明)。値は数・[下限, 上限]・文字列。ほかの名前も自由に使える
pub const SPEC_NAMES: &[(&str, &str, &str)] = &[
    (
        "wavelength_nm",
        "nm",
        "設計波長域 [下限, 上限]（1 つの波長なら数）",
    ),
    ("diameter", "mm", "外径（四角い品は一辺か [幅, 高さ]）"),
    ("clear_aperture", "mm", "有効径"),
    ("thickness", "mm", "厚さ・キューブの一辺"),
    ("efl", "mm", "焦点距離"),
    ("material", "", "硝材・基板"),
    ("coating", "", "膜（AR・反射膜の名前）"),
    ("reflectance", "%", "反射率（帯域の平均など）"),
    ("transmittance", "%", "透過率"),
    ("split_ratio", "", "分岐比 R:T（例 \"50:50\"）"),
    ("extinction_ratio", "", "消光比（例 1000 は 1000:1）"),
    ("retardance", "波長", "遅延（例 0.25 は λ/4）"),
    ("order", "", "波長板の次数（zero・multi・achromatic など）"),
    ("wedge", "arcmin", "くさび角"),
    (
        "flatness",
        "λ",
        "面精度（λ@633nm の分母でなく λ の値、例 0.1 は λ/10）",
    ),
    ("surface_quality", "", "スクラッチ・ディグ（例 \"40-20\"）"),
    ("damage_threshold", "", "損傷しきい値（条件つきの文字列）"),
    ("aoi_deg", "deg", "入射角"),
    ("mount", "", "取付け（ねじ・マウントの規格）"),
    ("thread", "", "機構のねじ（例 \"SM1\"）"),
    ("length", "mm", "機構の長さ（チューブ・ロッド）"),
    ("inner_diameter", "mm", "機構の内径・保持径"),
    ("rod_spacing", "mm", "ケージのロッドの間隔"),
    ("price", "", "価格（通貨つきの文字列）"),
];

/// 名前の決まった仕様の単位（無い名前は空）
pub fn spec_unit(name: &str) -> &'static str {
    SPEC_NAMES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, u, _)| *u)
        .unwrap_or("")
}

/// ほかのカタログの品へ足す仕様・リンク（`overlay` の値）
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct Overlay {
    /// 品の説明が空のときに使う
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ItemKind>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub specs: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}
