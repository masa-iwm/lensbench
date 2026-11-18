//! 系の編集の操作の JSON の型。`op` で操作を選び、操作ごとに使えるキーだけを受ける（知らないキーは誤り）。
//! serve の `edit` の `ops`（[`EditOp`]）と、ブロックの `surface_ops`（[`SurfaceOp`]、面をブロックの面でも指せる
//! 絞り・ソルブ・ピックアップだけ）。当て方は [`crate::edit_ops`]。`docs/schema/edit-ops.schema.json` はこの型から作る。
//!
//! 面の値・面のデータ・系の設定・機構部品の書き換え（`fields`・`data`・`part`）は、系ファイルの同じ名前の項目と同じ形の
//! `{名前: 値}` で、名前と値の確かめは当てるとき（[`lensbench_core::edit::SURF_FIELDS`]・[`crate::edit_ops::SYSTEM_KEYS`]）。

use lensbench_core::mech::MechPart;
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::borrow::Cow;

/// 数。JSON に書けない無限遠などは `{"$f": "inf"}`（`"-inf"`・`"nan"`、系ファイルと同じ）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Num(pub f64);

impl Serialize for Num {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let x = self.0;
        if x.is_finite() {
            s.serialize_f64(x)
        } else {
            let t = if x.is_nan() {
                "nan"
            } else if x > 0.0 {
                "inf"
            } else {
                "-inf"
            };
            let mut m = Map::new();
            m.insert("$f".into(), Value::from(t));
            m.serialize(s)
        }
    }
}

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            N(f64),
            F {
                #[serde(rename = "$f")]
                f: String,
            },
        }
        match Raw::deserialize(d)
            .map_err(|_| serde::de::Error::custom("a number or {\"$f\": \"inf\"} is needed"))?
        {
            Raw::N(x) => Ok(Num(x)),
            Raw::F { f } => match f.as_str() {
                "inf" => Ok(Num(f64::INFINITY)),
                "-inf" => Ok(Num(f64::NEG_INFINITY)),
                "nan" => Ok(Num(f64::NAN)),
                t => Err(serde::de::Error::custom(format!(
                    "{{\"$f\": \"{t}\"}} (inf, -inf, nan)"
                ))),
            },
        }
    }
}

impl JsonSchema for Num {
    fn schema_name() -> Cow<'static, str> {
        "Num".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "数。JSON に書けない数は {\"$f\": \"inf\"}（-inf・nan も）",
            "anyOf": [
                {"type": "number"},
                {
                    "type": "object",
                    "properties": {"$f": {"enum": ["inf", "-inf", "nan"]}},
                    "required": ["$f"],
                    "additionalProperties": false
                }
            ]
        })
    }
}

/// 番号か名前。面は OpticStudio の面番号か面のコメント、玉・群と機構部品は 0 始まりの番号か名前
#[derive(Serialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum Ref {
    /// 番号
    Index(usize),
    /// 名前
    Name(String),
}

/// プロジェクトのブロックの面
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BlockFace {
    /// ブロックの id
    pub block: String,
    /// ブロックの面の番号（0 始まり、座標ブレークを除く。往復するブロックは往路の面）
    pub surface: usize,
}

/// ブロックの `surface_ops` の面: ブロックの面 `{"block": id, "surface": i}` か、展開した系の面の番号か名前
#[derive(Serialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum BlockRef {
    /// ブロックの面
    Block(BlockFace),
    /// 展開した系の面
    Sys(Ref),
}

fn ref_of<E: serde::de::Error>(v: Value, surface: bool) -> Result<Ref, E> {
    let msg = match (&v, surface) {
        (Value::Number(n), true) => format!("surface number {n} (an integer 0 or more)"),
        (Value::Number(n), false) => format!("item number {n} (an integer 0 or more)"),
        (v, true) => format!("surface number or name needed ({v})"),
        (v, false) => format!("item number or name needed ({v})"),
    };
    match v {
        Value::Number(n) => n
            .as_u64()
            .map(|i| Ref::Index(i as usize))
            .ok_or_else(|| E::custom(msg)),
        Value::String(t) => Ok(Ref::Name(t)),
        _ => Err(E::custom(msg)),
    }
}

impl<'de> Deserialize<'de> for Ref {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        ref_of(Value::deserialize(d)?, false)
    }
}

impl<'de> Deserialize<'de> for BlockRef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Value::deserialize(d)? {
            v @ Value::Object(_) => BlockFace::deserialize(v).map(BlockRef::Block).map_err(|e| {
                serde::de::Error::custom(format!("block surface (block, surface): {e}"))
            }),
            v => ref_of(v, true).map(BlockRef::Sys).map_err(|e: D::Error| {
                serde::de::Error::custom(format!(
                    "{e} (a block surface is written with block and surface)"
                ))
            }),
        }
    }
}

/// 面を絞りにする
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StopOp<S> {
    pub surface: S,
}

/// 円錐係数・半直径・面タイプのパラメータを ZPL マクロで解く（同じ面・同じ量のものは置き換える。OpticStudio の
/// KZPL・DIAM の Macro Solve・PZPL）
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, bound(deserialize = "S: Deserialize<'de>"))]
pub struct MacroSolveOp<S> {
    pub surface: S,
    /// マクロの名前か番号（系のマクロ、プロジェクトでは `macros`。番号は 0 始まり）
    #[serde(rename = "macro")]
    pub macro_ref: Ref,
    /// 解く量: `k`（円錐係数）・`semi`（半直径）・`p1` …（面タイプのパラメータ）
    pub what: String,
}

impl<S> MacroSolveOp<S> {
    /// 面を f で読み替える（None は当てない）
    pub fn map<T, E>(
        self,
        mut f: impl FnMut(S) -> Result<Option<T>, E>,
    ) -> Result<Option<MacroSolveOp<T>>, E> {
        Ok(f(self.surface)?.map(|surface| MacroSolveOp {
            surface,
            macro_ref: self.macro_ref,
            what: self.what,
        }))
    }
}

/// 厚さか曲率のソルブを置く（同じ面・同じ量のソルブは置き換える）
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, bound(deserialize = "S: Deserialize<'de>"))]
pub struct SolveOp<S> {
    pub surface: S,
    /// 種類（既定 marginal_height、[`lensbench_core::solves::SOLVE_KINDS`]）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// 目標の値（既定 0）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Num>,
    /// 別の面を参照する種類の面
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<S>,
    /// 帯・縁の高さ（既定 0）。種類 `macro_curv`・`macro_thick` ではマクロの番号（`macro` で名前でも指せる）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<Num>,
    /// 種類 `macro_curv`・`macro_thick` のマクロの名前か番号（系のマクロ、プロジェクトでは `macros`）。書けば `param` より優先
    #[serde(rename = "macro", default, skip_serializing_if = "Option::is_none")]
    pub macro_ref: Option<Ref>,
}

/// ピックアップを置く: dst の量 = src の量 × scale + offset
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PickupOp<S> {
    /// 値を写される面
    pub dst: S,
    /// 元の面
    pub src: S,
    /// 量（c r t k p1〜 dn disp semi before0〜4 after0〜4）
    pub what: String,
    /// 倍率（既定 1）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Num>,
    /// 足す値（既定 0）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Num>,
    /// 元の量（省くと what）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

impl<S> StopOp<S> {
    /// 面を f で読み替える（None は当てない）
    pub fn map<T, E>(
        self,
        mut f: impl FnMut(S) -> Result<Option<T>, E>,
    ) -> Result<Option<StopOp<T>>, E> {
        Ok(f(self.surface)?.map(|surface| StopOp { surface }))
    }
}

impl<S> SolveOp<S> {
    /// 面を f で読み替える（どれかが None なら当てない）
    pub fn map<T, E>(
        self,
        mut f: impl FnMut(S) -> Result<Option<T>, E>,
    ) -> Result<Option<SolveOp<T>>, E> {
        let Some(surface) = f(self.surface)? else {
            return Ok(None);
        };
        let other = match self.other {
            Some(o) => match f(o)? {
                Some(o) => Some(o),
                None => return Ok(None),
            },
            None => None,
        };
        Ok(Some(SolveOp {
            surface,
            kind: self.kind,
            value: self.value,
            other,
            param: self.param,
            macro_ref: self.macro_ref,
        }))
    }

    /// 曲率のソルブか（種類の名前が読めなければ false）
    pub fn is_curv(&self) -> bool {
        lensbench_core::solves::solve_kind_parse(
            self.kind.as_deref().unwrap_or("marginal_height"),
            0.0,
            0.0,
            Some(0),
        )
        .is_ok_and(|k| k.is_curv())
    }
}

impl<S> PickupOp<S> {
    /// 面を f で読み替える（どれかが None なら当てない）
    pub fn map<T, E>(
        self,
        mut f: impl FnMut(S) -> Result<Option<T>, E>,
    ) -> Result<Option<PickupOp<T>>, E> {
        let (Some(dst), Some(src)) = (f(self.dst)?, f(self.src)?) else {
            return Ok(None);
        };
        Ok(Some(PickupOp {
            dst,
            src,
            what: self.what,
            scale: self.scale,
            offset: self.offset,
            from: self.from,
        }))
    }
}

/// ソルブを外す量
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SolveWhat {
    /// 厚さ（既定）
    #[default]
    T,
    /// 曲率
    C,
}

/// 系の編集の操作（serve の `edit` の `ops` の要素）。`op` で選ぶ
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum EditOp {
    /// 面 at の前に面を入れる（surface は面の値の名前と値、[`lensbench_core::edit::SURF_FIELDS`]）
    InsertSurface {
        at: usize,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        surface: Map<String, Value>,
    },
    /// 面を消す
    DeleteSurface { surface: Ref },
    /// 面の値を変える（名前と値、[`lensbench_core::edit::SURF_FIELDS`]。aperture は {"type": "circular", "max": 3, …} か null）
    SetSurface {
        surface: Ref,
        fields: Map<String, Value>,
    },
    /// 面タイプを変える
    SetSurfaceType {
        surface: Ref,
        #[serde(rename = "type")]
        typ: String,
    },
    /// 材料を変える
    SetMaterial { surface: Ref, material: String },
    /// 面を絞りにする
    SetStop(StopOp<Ref>),
    /// ピックアップを置く
    AddPickup(PickupOp<Ref>),
    /// ピックアップを外す
    RemovePickup { dst: Ref, what: String },
    /// ソルブを置く
    AddSolve(SolveOp<Ref>),
    /// 円錐係数・半直径・面タイプのパラメータのマクロのソルブを置く
    AddMacroSolve(MacroSolveOp<Ref>),
    /// ソルブを外す
    RemoveSolve {
        surface: Ref,
        #[serde(default)]
        what: SolveWhat,
    },
    /// 面のデータを全部の項目で変える（serve の `surface_data` の data と同じ形、書いた項目だけ）。屈折率 n・絞り stop・
    /// 鏡 mirror は変えない（材料・set_stop で）。面タイプの形のデータ zern・grid・bin2・cylf・grin、前後の傾き偏心
    /// before・after もここで
    SetSurfaceData {
        surface: Ref,
        data: Map<String, Value>,
    },
    /// パラメータの値を変える（名前は set と同じ）
    SetParam { param: String, value: Num },
    /// 系の設定の項目を変える（[`crate::edit_ops::SYSTEM_KEYS`] の書いた分だけ。形は serve の `system_data` と同じ）
    SetSystemData { data: Map<String, Value> },
    /// 玉・群を作る（spans は面の範囲 [[最初, 最後], …]、pivot は既定 [0, 0, 0]）
    DefineUnit {
        name: String,
        spans: Vec<[usize; 2]>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pivot: Option<[Num; 3]>,
    },
    /// 玉・群を変える（書いた分だけ。範囲を変えるのは動き・移動が 0 のときだけ）
    SetUnit {
        unit: Ref,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spans: Option<Vec<[usize; 2]>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pivot: Option<[Num; 3]>,
    },
    /// 玉・群を外す（動き・移動が 0 のときだけ）
    RemoveUnit { unit: Ref },
    /// 機構部品を置く（系ファイルの mechanics の要素と同じ形）
    AddMech { part: MechPart },
    /// 機構部品を変える（part は書いた項目だけ）
    SetMech { mech: Ref, part: Map<String, Value> },
    /// 機構部品を外す
    RemoveMech { mech: Ref },
    /// 多重構成の数
    SetConfigCount { n: usize },
    /// 多重構成の表の行（パラメータの構成ごとの値）
    SetConfigRow { param: String, values: Vec<Num> },
    /// 多重構成の表に行を足す（どの構成も今の値）
    AddConfigRow { param: String },
    /// 多重構成の表の行を外す
    RemoveConfigRow { param: String },
    /// 構成 k（1 始まり）を系に入れる
    SetConfig { k: usize },
    /// 構成を at（1 始まり）に入れる。from の構成の値で（既定は今の構成、pickup なら 1）、pickup なら from を写す
    InsertConfig {
        at: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<usize>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pickup: bool,
    },
    /// 構成 k を外す
    RemoveConfig { k: usize },
    /// 表の行 param の構成 k を構成 from から写す（× scale + offset）
    SetConfigPickup {
        param: String,
        k: usize,
        from: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scale: Option<Num>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<Num>,
    },
    /// 表の行 param の構成 k の写しをやめる
    RemoveConfigPickup { param: String, k: usize },
}

impl EditOp {
    /// 多重構成の表の操作か（そうでなければ、変えた値を表の今の構成の列にも写す）
    pub fn is_config(&self) -> bool {
        use EditOp::*;
        matches!(
            self,
            SetConfigCount { .. }
                | SetConfigRow { .. }
                | AddConfigRow { .. }
                | RemoveConfigRow { .. }
                | SetConfig { .. }
                | InsertConfig { .. }
                | RemoveConfig { .. }
                | SetConfigPickup { .. }
                | RemoveConfigPickup { .. }
        )
    }

    /// 操作の名前（`op`）
    pub fn name(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v["op"].as_str().map(str::to_string))
            .unwrap_or_default()
    }
}

/// ブロックの `surface_ops` の操作（[`crate::FaceMods`]）: serve の `edit` の `set_stop`・`add_solve`・`add_pickup`・`remove_solve`・
/// `remove_pickup` と同じ形で、面（`surface`・`dst`・`src`・`other`）はブロックの面 `{"block": id, "surface": i}` でも指せる
/// （当てる面 `surface`・`dst` の番号はそのブロックの面）。
/// `remove_*` はブロックの元（.zmx など）にあるソルブ・ピックアップを外す（無ければ当てずに注意）
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SurfaceOp {
    /// 面を絞りにする
    SetStop(StopOp<BlockRef>),
    /// ソルブを置く
    AddSolve(SolveOp<BlockRef>),
    /// 円錐係数・半直径・面タイプのパラメータのマクロのソルブを置く
    AddMacroSolve(MacroSolveOp<BlockRef>),
    /// ピックアップを置く
    AddPickup(PickupOp<BlockRef>),
    /// ソルブを外す
    RemoveSolve {
        surface: BlockRef,
        #[serde(default)]
        what: SolveWhat,
    },
    /// ピックアップを外す
    RemovePickup { dst: BlockRef, what: String },
}

impl SurfaceOp {
    /// 面を f で系の面に読み替えて編集の操作にする（f が None を返した面があれば当てない）
    pub fn to_edit<E>(
        &self,
        mut f: impl FnMut(BlockRef) -> Result<Option<Ref>, E>,
    ) -> Result<Option<EditOp>, E> {
        Ok(match self.clone() {
            SurfaceOp::SetStop(o) => o.map(f)?.map(EditOp::SetStop),
            SurfaceOp::AddSolve(o) => o.map(f)?.map(EditOp::AddSolve),
            SurfaceOp::AddMacroSolve(o) => o.map(f)?.map(EditOp::AddMacroSolve),
            SurfaceOp::AddPickup(o) => o.map(f)?.map(EditOp::AddPickup),
            SurfaceOp::RemoveSolve { surface, what } => {
                f(surface)?.map(|surface| EditOp::RemoveSolve { surface, what })
            }
            SurfaceOp::RemovePickup { dst, what } => {
                f(dst)?.map(|dst| EditOp::RemovePickup { dst, what })
            }
        })
    }

    /// 操作を当てる面（`surface`・`dst`。`src`・`other` は含めない）
    pub fn target_mut(&mut self) -> &mut BlockRef {
        match self {
            SurfaceOp::SetStop(x) => &mut x.surface,
            SurfaceOp::AddSolve(x) => &mut x.surface,
            SurfaceOp::AddMacroSolve(x) => &mut x.surface,
            SurfaceOp::AddPickup(x) => &mut x.dst,
            SurfaceOp::RemoveSolve { surface, .. } => surface,
            SurfaceOp::RemovePickup { dst, .. } => dst,
        }
    }

    /// 番号で書いた面の `set_stop` の面の番号
    pub fn stop_index(&self) -> Option<usize> {
        match self {
            SurfaceOp::SetStop(StopOp {
                surface: BlockRef::Sys(Ref::Index(i)),
            }) => Some(*i),
            _ => None,
        }
    }

    /// 操作の名前（`op`）
    pub fn name(&self) -> &'static str {
        match self {
            SurfaceOp::SetStop(_) => "set_stop",
            SurfaceOp::AddSolve(_) => "add_solve",
            SurfaceOp::AddMacroSolve(_) => "add_macro_solve",
            SurfaceOp::AddPickup(_) => "add_pickup",
            SurfaceOp::RemoveSolve { .. } => "remove_solve",
            SurfaceOp::RemovePickup { .. } => "remove_pickup",
        }
    }
}
