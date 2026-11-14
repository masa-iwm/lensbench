//! モンテカルロ公差の設定の JSON の型（PRJ-11）。キーの意味は [`crate::tolerance`] の表。知らないキーは誤りで、
//! 誤りの文には欄の道（`tolerance.lenses.L1: …`）が付く。`docs/schema/tolerance.schema.json` はこの型から作る。

use crate::montecarlo::{GroupTol, LensTol};
use crate::term_spec::RefName;
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::de::{Deserializer, Error as _};
use serde::Deserialize;
use serde_json::Value;

/// 値をいったん JSON の値で受けてから形で分ける（untagged より誤りの文が分かりやすい）
macro_rules! via_value {
    ($t:ty, $f:expr) => {
        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let v = Value::deserialize(d)?;
                let f: fn(Value) -> Result<$t, String> = $f;
                f(v).map_err(D::Error::custom)
            }
        }
    };
}

fn from<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}

/// 設定（すべて省略可）
#[derive(Deserialize, JsonSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct TolSpec {
    /// このファイルのスキーマ（読むときは無視する）
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    /// 試行の数（100）。0 なら感度だけ
    pub trials: Option<usize>,
    /// 乱数の種（1）
    pub seed: Option<u64>,
    /// 既定の分布（"normal"）: "uniform"・"normal"・"normal:3"・"end"
    pub dist: Option<String>,
    /// 偏心・傾き・くさびの x・y を組にして円の中で引く: true・false・"uniform"・"normal"・"normal:σの数"
    pub radial: Option<Radial>,
    /// 玉の既定の公差の組: "commercial"・"precision"・"high"・"none"
    pub preset: Option<String>,
    /// くさびを入れる面（"back"）
    pub wedge_face: Option<Face>,
    /// カタログの玉はカタログの玉の公差を既定にする（true）
    pub catalog: Option<bool>,
    /// メーカーごとの公差 {"メーカー": 玉の公差と rules}（"*" はほか）
    pub catalog_tols: Option<IndexMap<String, Option<CatEntry>>>,
    /// 玉ごとの上書き {"名前": 玉の公差 か 組の名前 か null（公差なし）}
    pub lenses: Option<IndexMap<String, Option<LensEntry>>>,
    /// 群・ブロックの公差 {"名前": {"decenter", "tilt", "shift"} か null}
    pub groups: Option<IndexMap<String, Option<GroupFields>>>,
    /// 個別の公差の並び
    pub tols: Option<Vec<TolItem>>,
    /// 補償子（1 つか並び）
    pub compensators: Option<Compensators>,
    /// 補償子の DLS の反復の上限（10）
    pub comp_iters: Option<usize>,
    /// 評価関数（"wavefront"）
    pub merit: Option<MeritName>,
    /// 既定の評価関数の瞳のリング数（3）
    pub rings: Option<usize>,
    pub reference: Option<RefName>,
    /// 足す評価関数の項（terms.schema.json の形）
    pub terms: Option<Value>,
    /// 公差ごとの感度も求める（true）
    pub sensitivity: Option<bool>,
    /// 出す百分位（[50, 80, 90, 95, 98]）
    pub percentiles: Option<Vec<f64>>,
    /// 試行ごとのずれと値も返す（false）
    pub samples: Option<bool>,
    /// スレッド数
    pub threads: Option<usize>,
}

/// くさびを入れる面
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Face {
    /// 玉の後ろの面
    Back,
    /// 前の面
    Front,
}

/// 既定の評価関数
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MeritName {
    #[serde(alias = "wfe")]
    Wavefront,
    Spot,
    /// terms だけ
    None,
}

/// radial の値
#[derive(JsonSchema, Debug, Clone)]
#[schemars(untagged)]
pub enum Radial {
    On(bool),
    Dist(String),
}
via_value!(Radial, |v| match v {
    Value::Bool(b) => Ok(Radial::On(b)),
    Value::String(s) => Ok(Radial::Dist(s)),
    x => Err(format!(
        "radial は true・false・uniform・normal・normal:σの数（{x}）"
    )),
});

macro_rules! lens_fields {
    ($(#[$m:meta])* $name:ident { $($(#[$em:meta])* $ek:ident: $et:ty,)* }) => {
        $(#[$m])*
        #[derive(Deserialize, JsonSchema, Debug, Clone, Default)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            /// 曲率半径の比
            pub radius_rel: Option<f64>,
            /// 焦点距離の比
            pub focal_rel: Option<f64>,
            /// 中心厚 [mm]
            pub thickness: Option<f64>,
            /// くさび [deg]
            pub wedge: Option<f64>,
            pub wedge_face: Option<Face>,
            /// 屈折率のずれ
            pub dn: Option<f64>,
            /// 分散の相対変化
            pub disp: Option<f64>,
            /// 偏心 [mm]
            pub decenter: Option<f64>,
            /// 傾き [deg]
            pub tilt: Option<f64>,
            /// 後の空気の間隔 [mm]
            pub space: Option<f64>,
            $($(#[$em])* pub $ek: $et,)*
        }

        impl $name {
            /// 書いた項目だけ base を上書きする
            pub fn apply(&self, base: LensTol) -> LensTol {
                let o = |x: Option<f64>, d: f64| x.unwrap_or(d);
                LensTol {
                    radius_rel: o(self.radius_rel, base.radius_rel),
                    focal_rel: o(self.focal_rel, base.focal_rel),
                    thickness: o(self.thickness, base.thickness),
                    wedge: o(self.wedge, base.wedge),
                    dn: o(self.dn, base.dn),
                    disp: o(self.disp, base.disp),
                    decenter: o(self.decenter, base.decenter),
                    tilt: o(self.tilt, base.tilt),
                    space: o(self.space, base.space),
                    wedge_first: self.wedge_face.map_or(base.wedge_first, |f| f == Face::Front),
                }
            }
        }
    };
}

lens_fields!(
    /// 玉の公差（書いた項目だけ既定の組を上書き）
    LensFields {}
);

lens_fields!(
    /// メーカーの玉の公差と、外径・焦点距離の条件つきの上書き
    CatFields {
        /// 前から順に、条件が合うものを当てる
        rules: Option<Vec<Rule>>,
    }
);

lens_fields!(
    /// 条件つきの上書き（条件がすべて合うとき。外径・焦点距離が分からないと合わない）
    Rule {
        /// 外径 [mm] < 値
        dia_lt: Option<f64>,
        dia_le: Option<f64>,
        dia_gt: Option<f64>,
        dia_ge: Option<f64>,
        /// |焦点距離| [mm] < 値
        efl_lt: Option<f64>,
        efl_le: Option<f64>,
        efl_gt: Option<f64>,
        efl_ge: Option<f64>,
        /// true は貼り合わせ（ガラス 2 枚以上）、false は単レンズ
        cemented: Option<bool>,
    }
);

impl Rule {
    /// 外径 dia・焦点距離 efl・ガラスの枚数の玉に合うか
    pub fn matches(&self, dia: f64, efl: f64, glasses: usize) -> bool {
        let e = efl.abs();
        let c = |x: Option<f64>, f: &dyn Fn(f64) -> bool| x.is_none_or(f);
        c(self.dia_lt, &|l| dia < l)
            && c(self.dia_le, &|l| dia <= l)
            && c(self.dia_gt, &|l| dia > l)
            && c(self.dia_ge, &|l| dia >= l)
            && c(self.efl_lt, &|l| e < l)
            && c(self.efl_le, &|l| e <= l)
            && c(self.efl_gt, &|l| e > l)
            && c(self.efl_ge, &|l| e >= l)
            && self.cemented.is_none_or(|k| k == (glasses >= 2))
    }

    /// 上書きの項目（条件を除いたもの）
    pub fn fields(&self) -> LensFields {
        LensFields {
            radius_rel: self.radius_rel,
            focal_rel: self.focal_rel,
            thickness: self.thickness,
            wedge: self.wedge,
            wedge_face: self.wedge_face,
            dn: self.dn,
            disp: self.disp,
            decenter: self.decenter,
            tilt: self.tilt,
            space: self.space,
        }
    }
}

/// 玉の公差: 項目のオブジェクトか組の名前
#[derive(JsonSchema, Debug, Clone)]
#[schemars(untagged)]
pub enum LensEntry {
    Preset(String),
    Fields(LensFields),
}
via_value!(LensEntry, |v| match v {
    Value::String(s) => Ok(LensEntry::Preset(s)),
    v @ Value::Object(_) => from(v).map(LensEntry::Fields),
    x => Err(format!("オブジェクト・組の名前・null（{x}）")),
});

/// メーカーの公差: 項目のオブジェクト（rules つき）か組の名前
#[derive(JsonSchema, Debug, Clone)]
#[schemars(untagged)]
pub enum CatEntry {
    Preset(String),
    Fields(CatFields),
}
via_value!(CatEntry, |v| match v {
    Value::String(s) => Ok(CatEntry::Preset(s)),
    v @ Value::Object(_) => from(v).map(CatEntry::Fields),
    x => Err(format!("オブジェクト・組の名前・null（{x}）")),
});

/// 組の名前の公差（くさびの面は base のまま）
pub fn preset(name: &str, base: LensTol) -> Result<LensTol, String> {
    LensTol::preset(name)
        .map(|t| LensTol {
            wedge_first: base.wedge_first,
            ..t
        })
        .map_err(|e| e.to_string())
}

impl LensEntry {
    pub fn apply(&self, base: LensTol) -> Result<LensTol, String> {
        match self {
            LensEntry::Preset(p) => preset(p, base),
            LensEntry::Fields(f) => Ok(f.apply(base)),
        }
    }
}

/// 群の公差
#[derive(Deserialize, JsonSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct GroupFields {
    /// 偏心 [mm]
    pub decenter: Option<f64>,
    /// 傾き [deg]
    pub tilt: Option<f64>,
    /// 光軸方向のずれ [mm]
    pub shift: Option<f64>,
}

impl GroupFields {
    pub fn tol(&self) -> GroupTol {
        let d = GroupTol::default();
        GroupTol {
            decenter: self.decenter.unwrap_or(d.decenter),
            tilt: self.tilt.unwrap_or(d.tilt),
            shift: self.shift.unwrap_or(d.shift),
        }
    }
}

/// 個別の公差
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct TolItem {
    /// パラメータ（"3.t"・"wave" など。params とどちらか一方）
    pub param: Option<String>,
    /// パラメータの並び（同じずれを与える）
    pub params: Option<Vec<String>>,
    /// ±（lo・hi の代わり）
    pub pm: Option<f64>,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    /// 比（false）
    pub relative: Option<bool>,
    pub dist: Option<String>,
    pub label: Option<String>,
    /// パラメータごとにずれに掛ける数の並び
    pub scale: Option<Vec<f64>>,
}

/// 補償子: "9.t"・"G1.shift=-1:1" か {"param", "lo", "hi", "configs"}
#[derive(JsonSchema, Debug, Clone)]
#[schemars(untagged)]
pub enum VarSpec {
    Text(String),
    Obj(VarObj),
}
via_value!(VarSpec, |v| match v {
    Value::String(s) => Ok(VarSpec::Text(s)),
    v @ Value::Object(_) => from(v).map(VarSpec::Obj),
    x => Err(format!("補償子は文字列かオブジェクト（{x}）")),
});

/// 補償子のオブジェクト
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct VarObj {
    pub param: String,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    /// 効く構成の番号（1 から）の並び。省くと全構成で同じ値
    pub configs: Option<Vec<usize>>,
}

/// 補償子の 1 つか並び
#[derive(JsonSchema, Debug, Clone)]
#[schemars(untagged)]
pub enum Compensators {
    One(VarSpec),
    Many(Vec<VarSpec>),
}
via_value!(Compensators, |v| match v {
    Value::Array(a) => a
        .into_iter()
        .map(from)
        .collect::<Result<_, _>>()
        .map(Compensators::Many),
    v => from(v).map(Compensators::One),
});

impl Compensators {
    pub fn list(&self) -> Vec<&VarSpec> {
        match self {
            Compensators::One(v) => vec![v],
            Compensators::Many(a) => a.iter().collect(),
        }
    }
}
