//! JSON スキーマ: プロジェクト・系ファイル・ユーザーカタログの形を（差し替え探索の設定は lensbench-design）、
//! serde の型から schemars で作る。リポジトリの `docs/schema/<名前>.schema.json` はこれで作り、テスト
//! （`tests/schema.rs`）で型とずれていないかを確かめる（作り直すには `LENSBENCH_UPDATE_SCHEMA=1 cargo test
//! -p lensbench-project --test it schema::`）。
//!
//! - 説明は型・項目のドキュメントコメント。既定値は serde の default から。
//! - どの JSON にも先頭に `"$schema": "…"` を書ける（エディタの補完・検査用。読むときは無視する）。
//! - 検出器のブロック（[`crate::DetectorBlock`]）は、組み込みでない `"type"` の何でもよいブロックとして書き、登録した
//!   種類（[`crate::detectors`]）にスキーマがあればそれを足す（リポジトリのスキーマは登録なしで作る）。
//! - 系ファイルの数は JSON に書けない値（inf・NaN）を `{"$f": "inf"}` などで書くので、系の中の数はそれも許す。

use crate::{detectors, Project, UserCatalog, BUILTIN_BLOCKS};
use lensbench_core::System;
use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema};
use serde_json::{json, Map, Value};

/// スキーマの名前（`docs/schema/<名前>.schema.json`）
pub const NAMES: [&str; 3] = ["project", "system", "user-catalog"];

/// スキーマの `$id` の元（公開リポジトリの main の docs/schema）
pub const BASE_URL: &str = "https://raw.githubusercontent.com/masa-iwm/lensbench/main/docs/schema/";

/// 系ファイル（[`crate::sysfile`]）の形。スキーマを作るためだけの型（読み書きは sysfile が Value で行う）
#[allow(dead_code)]
#[derive(JsonSchema)]
#[schemars(rename = "SystemFile")]
struct SystemFile {
    /// 系ファイルの印（"lensbench-system"）
    #[schemars(extend("const" = "lensbench-system"))]
    format: String,
    /// 形式の版（読めるのはこの lensbench が書く版以下）
    format_version: u64,
    /// 書いた lensbench の版
    #[serde(default)]
    lensbench: String,
    /// `system` の部分（キーを名前の順にした空白なしの JSON）の SHA-256（"sha256:…"）。合わなければ読むときに注意を出す
    #[serde(default)]
    hash: String,
    /// 膜の実測表・モデルのファイル（相対パスは系ファイルのフォルダから）
    #[serde(default)]
    coatings: Vec<String>,
    /// 順次の面の列の系
    system: System,
}

/// 型のスキーマ（draft 2020-12、読む側の形）。仕上げは [`finish`]
pub fn gen<T: JsonSchema>() -> Value {
    let g = SchemaSettings::draft2020_12()
        .for_deserialize()
        .into_generator();
    let s: Schema = g.into_root_schema_for::<T>();
    s.to_value()
}

/// 名前のスキーマ（JSON の値）。登録した検出器の種類も入る。知らない名前なら None
pub fn schema(name: &str) -> Option<Value> {
    schema_with(name, &detectors::list())
}

/// 名前のスキーマ。検出器は `kinds` だけ（空なら組み込みのブロックと、何でもよい検出器のブロックだけ）
pub fn schema_with(name: &str, kinds: &[detectors::DetectorKind]) -> Option<Value> {
    let (mut v, title) = match name {
        "project" => (gen::<Project>(), "lensbench プロジェクトファイル"),
        "system" => {
            let mut v = gen::<SystemFile>();
            allow_nonfinite(&mut v);
            v["$defs"]["NonFinite"] = json!({
                "description": "JSON に書けない数（無限遠の inf、NaN）",
                "type": "object",
                "properties": {"$f": {"enum": ["inf", "-inf", "nan"]}},
                "required": ["$f"],
                "additionalProperties": false
            });
            (v, "lensbench 系ファイル")
        }
        "user-catalog" => (gen::<UserCatalog>(), "lensbench ユーザーカタログ"),
        _ => return None,
    };
    add_detectors(&mut v, kinds);
    finish(&mut v, name, title);
    Some(v)
}

/// `$id`・`title`・どの JSON にも書ける `$schema` のキーを足す（他のクレートの設定のスキーマも同じ形にする）
pub fn finish(v: &mut Value, name: &str, title: &str) {
    let o = v.as_object_mut().expect("スキーマはオブジェクト");
    o.insert(
        "$id".into(),
        Value::from(format!("{BASE_URL}{name}.schema.json")),
    );
    o.insert("title".into(), Value::from(title));
    if let Some(Value::Object(p)) = o.get_mut("properties") {
        p.insert(
            "$schema".into(),
            json!({
                "description": "このファイルのスキーマ（エディタの補完・検査用。読むときは無視する）",
                "type": "string"
            }),
        );
    }
}

/// すべてのスキーマ（名前と値）。登録した検出器も入る
pub fn all() -> Vec<(&'static str, Value)> {
    all_with(&detectors::list())
}

/// すべてのスキーマで、検出器は `kinds` だけ。docs/schema は `all_with(&[])`（同じプロセスの試験・プラグインが
/// 登録した種類に左右されない、XCT-32）
pub fn all_with(kinds: &[detectors::DetectorKind]) -> Vec<(&'static str, Value)> {
    NAMES
        .iter()
        .map(|n| (*n, schema_with(n, kinds).expect("NAMES の名前")))
        .collect()
}

/// スキーマを書くときの文字列（キーを名前の順に並べて整形、末尾に改行）。serde_json の preserve_order が
/// 他のクレートで有効になっても同じ文字列になるよう、並びを決めて書く
pub fn to_text(v: &Value) -> String {
    serde_json::to_string_pretty(&sorted(v)).expect("JSON") + "\n"
}

fn sorted(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|k| (k.clone(), sorted(&o[k])))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        x => x.clone(),
    }
}

/// ブロックの選択肢（`$defs/Block/oneOf`）に検出器のブロックを足す: 登録した種類ごとのもの（スキーマがあれば）と、
/// それ以外の名前の何でもよいブロック
fn add_detectors(v: &mut Value, kinds: &[detectors::DetectorKind]) {
    let Some(Value::Array(a)) = v.pointer_mut("/$defs/Block/oneOf") else {
        return;
    };
    let id = json!({"type": "string", "default": ""});
    let mut taken: Vec<&str> = BUILTIN_BLOCKS.to_vec();
    for k in kinds {
        let Some(f) = k.schema else { continue };
        let mut sc = f();
        if let Some(o) = sc.as_object_mut() {
            o.insert("description".into(), json!(k.doc));
            let props = o
                .entry("properties")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("properties はオブジェクト");
            props.insert("type".into(), json!({"type": "string", "const": k.name}));
            props.insert("id".into(), id.clone());
            let req = o.entry("required").or_insert_with(|| json!([]));
            if let Value::Array(r) = req {
                r.push(json!("type"));
            }
        }
        a.push(sc);
        taken.push(k.name);
    }
    a.push(json!({
        "description": "検出器（登録した種類。最後のブロックだけに置け、面は足さない）。項目は種類ごと",
        "type": "object",
        "properties": {
            "type": {"type": "string", "not": {"enum": taken}},
            "id": id,
        },
        "required": ["type"],
    }));
}

fn is_number_type(t: &Value) -> bool {
    match t {
        Value::String(s) => s == "number",
        Value::Array(a) => a.iter().any(|x| x == "number"),
        _ => false,
    }
}

/// 数の型（`"type": "number"`）を、数か NonFinite の `{"$f": …}` にする
fn allow_nonfinite(v: &mut Value) {
    match v {
        Value::Object(o) => {
            for x in o.values_mut() {
                allow_nonfinite(x);
            }
            if o.get("type").is_some_and(is_number_type) {
                let mut outer = Map::new();
                for k in ["description", "default", "title"] {
                    if let Some(x) = o.remove(k) {
                        outer.insert(k.into(), x);
                    }
                }
                let inner = Value::Object(std::mem::take(o));
                outer.insert(
                    "anyOf".into(),
                    json!([inner, {"$ref": "#/$defs/NonFinite"}]),
                );
                *o = outer;
            }
        }
        Value::Array(a) => {
            for x in a {
                allow_nonfinite(x);
            }
        }
        _ => {}
    }
}

/// JSON の値から `$schema` を除く（deny_unknown_fields の型を読む前に）
pub fn strip_schema_key(txt: &str) -> Option<Value> {
    if !txt.contains("$schema") {
        return None;
    }
    match serde_json::from_str::<Value>(txt) {
        Ok(Value::Object(mut o)) => o.remove("$schema").map(|_| Value::Object(o)),
        _ => None,
    }
}
