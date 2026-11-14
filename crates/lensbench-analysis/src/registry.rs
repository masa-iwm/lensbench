//! 解析の約束: 解析ごとに「設定の型（serde ＋ スキーマ）→ 結果の型（serde）」を 1 回だけ書き、登録簿に並べる。
//!
//! CLI の引数・serve の `analyze`・GUI の設定欄・ドキュメントは、どれもここ（[`DynAnalysis::params`] と
//! [`DynAnalysis::run_json`]）から作る。既定値は設定の型の `Default` だけに書く（見直し ANA-03・XCT-02）。
//!
//! 名前は CLI のサブコマンドと同じ（`mtf-focus` のようにハイフン）。設定の鍵は下線（`max_freq`）で、CLI の旗は
//! 下線をハイフンにしたもの（`--max-freq`）。視野・波長の番号は 1 始まり。

use std::marker::PhantomData;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use lensbench_core::{Error, Result, System};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

/// 解析の名前と説明
#[derive(Debug, Clone, Copy)]
pub struct Meta {
    /// 名前（CLI のサブコマンド・serve の `cmd`）
    pub name: &'static str,
    /// 1 行の説明（`--help` の一覧）
    pub about: &'static str,
    /// 長い説明（`<名前> --help`。空なら about だけ）
    pub help: &'static str,
}

/// 解析に渡す入力（系など）
pub struct Cx<'a> {
    /// 解析する系（状態・構成・経路を当てたもの）
    pub system: &'a System,
    /// 系の読み方（別の構成・状態の系、状態・経路の一覧）。系だけを渡すとき（Python・試験）は None
    pub source: Option<&'a dyn Source>,
}

impl<'a> Cx<'a> {
    /// 系だけの入力
    pub fn new(system: &'a System) -> Cx<'a> {
        Cx {
            system,
            source: None,
        }
    }

    /// 系の読み方（無ければ誤り）
    pub fn source(&self) -> Result<&'a dyn Source> {
        self.source
            .ok_or_else(|| Error::Invalid("この解析は系のファイル（読み方）が要る".into()))
    }
}

/// 別に読む系の選び方（[`Source::load`]）
#[derive(Debug, Clone, Default)]
pub struct Variant {
    /// 光束: .zmx は構成の番号（1 始まり）、プロジェクトは状態の名前。None で読み込んだ系と同じ構成・状態
    pub beam: Option<String>,
    /// 追跡する波長 [µm] に置き換えて読む（主波長は先頭。.zmx だけ）
    pub wavelengths: Option<Vec<f64>>,
}

/// 系の読み方（CLI はコマンドラインの引数、serve は load で選んだもの）。干渉の 2 つの光束・系の一覧の解析が使う
pub trait Source {
    /// プロジェクトファイル（光束・状態を名前で選ぶ）か。.zmx・系ファイルなら false
    fn is_project(&self) -> bool;
    /// プロジェクトファイルの状態の名前（.zmx・系ファイルなら空）
    fn states(&self) -> Result<Vec<String>>;
    /// 選んだ状態
    fn state(&self) -> Option<String>;
    /// ビームスプリッターの経路の一覧（BS なし・.zmx なら空）
    fn paths(&self) -> Result<Vec<String>>;
    /// 選んだ経路
    fn path(&self) -> Option<String>;
    /// 別の読み方（光束・追跡する波長）で読んだ系。serve では読み込んだ後の変更（set・edit）も当て、当てなかった変更は notes に書く
    fn load(&self, v: &Variant, notes: &mut Vec<String>) -> Result<System>;
    /// 別の光束（.zmx は構成の番号、プロジェクトは状態の名前）で読んだ系（[`Source::load`]）
    fn variant(&self, beam: &str, notes: &mut Vec<String>) -> Result<System> {
        self.load(
            &Variant {
                beam: Some(beam.to_string()),
                ..Default::default()
            },
            notes,
        )
    }
    /// プロジェクトファイルの JSON の文字列（状態は当てない。.zmx・系ファイルなら None）
    fn project_text(&self) -> Result<Option<String>>;
    /// 系の hash（系ファイルの形の。結果・書き出しの provenance に刻む）
    fn system_hash(&self, s: &System) -> Result<String>;
}

/// 解析の定義
pub trait Analysis: Send + Sync + 'static {
    /// 設定。既定値は `Default` に書く（ほかには書かない）
    type Params: DeserializeOwned + Serialize + JsonSchema + Default;
    /// 結果
    type Output: Serialize + JsonSchema;
    fn meta(&self) -> Meta;
    fn run(&self, cx: &Cx<'_>, p: &Self::Params) -> Result<Self::Output>;
    /// 結果を書き出す（拡張子で形式を自分で決める解析だけ。None なら呼び手が結果の JSON か表で書く）
    fn save(&self, _out: &Self::Output, _path: &Path) -> Option<Result<Saved>> {
        None
    }
}

/// 解析が自分で決めた書き出し（[`Analysis::save`]）
pub enum Saved {
    /// 呼び手がこの値を JSON で書く（結果は書いた値に written を足したもの）
    Json(Value),
    /// 解析が書いた。この値を結果にする
    Written(Value),
}

/// 設定の項目の型（GUI の欄・CLI の旗の受け方）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamTy {
    /// 0 以上の整数
    Count,
    /// 整数
    Int,
    /// 実数
    Num,
    /// 旗（真偽）
    Flag,
    /// 文字列
    Str,
    /// 決まった文字列のどれか（[`Param::choices`]）
    Choice,
    /// 数の並び（CLI はカンマ区切り）
    Nums,
    /// ファイルのパス
    Path,
    /// ファイルのパスの並び（CLI は旗を繰り返す）
    Paths,
    /// JSON のオブジェクト（センサの設定など。スキーマの `x-kind = "json"`。CLI は JSON の文字列、GUI は項目の欄）
    Json,
}

impl ParamTy {
    /// GUI・ドキュメントの型の名前（プラグインの設定の項目と同じ書き方）
    pub fn as_str(self) -> &'static str {
        match self {
            ParamTy::Count => "count",
            ParamTy::Int => "int",
            ParamTy::Num => "num",
            ParamTy::Flag => "flag",
            ParamTy::Str => "str",
            ParamTy::Choice => "choice",
            ParamTy::Nums => "nums",
            ParamTy::Path => "path",
            ParamTy::Paths => "path",
            ParamTy::Json => "json",
        }
    }
}

/// 設定の項目の旧名（新しい名前, 旧名）。旧名は受けず、誤りの文で新しい名前を示す（CLI-17）
pub const OLD_PARAM_NAMES: &[(&str, &str)] = &[("reference", "ref"), ("points", "steps")];

/// 旧名なら新しい名前
pub fn renamed_param(k: &str) -> Option<&'static str> {
    OLD_PARAM_NAMES
        .iter()
        .find(|(_, o)| *o == k)
        .map(|(n, _)| *n)
}

/// 設定の 1 項目（スキーマから作る）
#[derive(Debug, Clone)]
pub struct Param {
    /// 鍵（下線。CLI の旗は下線をハイフンに）
    pub name: String,
    pub ty: ParamTy,
    /// 既定値（`Default` の値。省けるものは null）
    pub default: Value,
    /// 省いてよい（値が null になる）
    pub optional: bool,
    /// 説明（スキーマの description）
    pub doc: String,
    /// 単位（スキーマの `x-unit`）
    pub unit: String,
    /// 選べる値（[`ParamTy::Choice`]）
    pub choices: Vec<String>,
    /// 数の下限
    pub min: Option<f64>,
    /// 並びの長さが決まっていれば（中心の "x,y" など）
    pub len: Option<usize>,
    /// 書き出すファイル・フォルダのパス（スキーマの `x-write`。serve は書いてよい場所かを確かめる）
    pub write: bool,
}

impl Param {
    /// 省けない（数の並びで、既定が空で null にもできないもの。スルーフォーカスの周波数など）
    pub fn required(&self) -> bool {
        self.ty == ParamTy::Nums && !self.optional && self.default == Value::Array(Vec::new())
    }

    /// GUI の設定欄・serve の `analysis_args` の形（`{"name","type","default","unit","doc",...}`）
    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("name".into(), Value::from(self.name.clone()));
        o.insert("type".into(), Value::from(self.ty.as_str()));
        o.insert("default".into(), self.default.clone());
        o.insert("unit".into(), Value::from(self.unit.clone()));
        o.insert("doc".into(), Value::from(self.doc.clone()));
        if !self.choices.is_empty() {
            o.insert("choices".into(), Value::from(self.choices.clone()));
        }
        if let Some(n) = self.len {
            o.insert("len".into(), Value::from(n));
        }
        if let Some(m) = self.min {
            o.insert("min".into(), Value::from(m));
        }
        if self.ty == ParamTy::Paths {
            o.insert("repeat".into(), Value::Bool(true));
        }
        if self.required() {
            o.insert("required".into(), Value::Bool(true));
        }
        Value::Object(o)
    }
}

/// 型を消した解析（登録簿に並べる形）
pub trait DynAnalysis: Send + Sync {
    fn meta(&self) -> Meta;
    /// 設定の JSON スキーマ
    fn params_schema(&self) -> Value;
    /// 結果の JSON スキーマ
    fn output_schema(&self) -> Value;
    /// 設定の項目（スキーマと既定値から）
    fn params(&self) -> Vec<Param>;
    /// 設定を読めるか確かめる（知らない鍵・型違いは誤り）。呼び手が要求の誤りと解析の誤りを分けるのに使う
    fn check_params(&self, params: &Value) -> Result<()>;
    /// 設定（JSON のオブジェクト。省いた鍵は既定値、知らない鍵は誤り）で解析し、結果を JSON で返す
    fn run_json(&self, cx: &Cx<'_>, params: &Value) -> Result<Value> {
        Ok(self.run_save(cx, params, None)?.0)
    }
    /// [`DynAnalysis::run_json`] と同じに解析し、save があれば解析が決めた形で書き出す。
    /// 返す組の 2 つ目は呼び手が書くもの（None なら書き終えた、または save なし）
    fn run_save(
        &self,
        cx: &Cx<'_>,
        params: &Value,
        save: Option<&Path>,
    ) -> Result<(Value, Option<Value>)>;
}

struct Erased<A: Analysis>(A, PhantomData<fn() -> A>);

impl<A: Analysis> Erased<A> {
    fn parse(&self, params: &Value) -> Result<A::Params> {
        match params {
            Value::Null => Ok(A::Params::default()),
            v => serde_json::from_value(v.clone())
                .map_err(|e| Error::Invalid(format!("{} の設定: {e}", self.0.meta().name))),
        }
    }
}

fn schema_of<T: JsonSchema>() -> Value {
    let g = schemars::generate::SchemaSettings::draft2020_12()
        .with(|s| s.inline_subschemas = true)
        .into_generator();
    g.into_root_schema_for::<T>().to_value()
}

impl<A: Analysis> DynAnalysis for Erased<A> {
    fn meta(&self) -> Meta {
        self.0.meta()
    }
    fn params_schema(&self) -> Value {
        schema_of::<A::Params>()
    }
    fn output_schema(&self) -> Value {
        schema_of::<A::Output>()
    }
    fn params(&self) -> Vec<Param> {
        let defaults = serde_json::to_value(A::Params::default()).unwrap_or(Value::Null);
        params_of(&self.params_schema(), &defaults)
    }
    fn check_params(&self, params: &Value) -> Result<()> {
        self.parse(params).map(|_| ())
    }
    fn run_save(
        &self,
        cx: &Cx<'_>,
        params: &Value,
        save: Option<&Path>,
    ) -> Result<(Value, Option<Value>)> {
        let p = self.parse(params)?;
        let out = self.0.run(cx, &p)?;
        let json = |o: &A::Output| {
            serde_json::to_value(o)
                .map_err(|e| Error::Invalid(format!("{} の結果: {e}", self.0.meta().name)))
        };
        match save {
            None => Ok((json(&out)?, None)),
            Some(path) => match self.0.save(&out, path).transpose()? {
                Some(Saved::Written(v)) => Ok((v, None)),
                Some(Saved::Json(v)) => {
                    let mut r = v.clone();
                    r["written"] = Value::from(path.display().to_string());
                    Ok((r, Some(v)))
                }
                None => {
                    let v = json(&out)?;
                    Ok((v.clone(), Some(v)))
                }
            },
        }
    }
}

/// 型の名前（null を除いたもの）
fn types(s: &Value) -> Vec<&str> {
    match s.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .filter(|t| *t != "null")
            .collect(),
        _ => Vec::new(),
    }
}

fn nullable(s: &Value) -> bool {
    let in_type = match s.get("type") {
        Some(Value::Array(a)) => a.iter().any(|t| t == "null"),
        _ => false,
    };
    let null = Value::from("null");
    in_type
        || s.get("anyOf")
            .and_then(Value::as_array)
            .is_some_and(|a| a.iter().any(|x| x.get("type") == Some(&null)))
}

/// 選べる文字列（`enum`、または説明つきの `oneOf` の `const`）
fn choices(s: &Value) -> Vec<String> {
    if let Some(Value::Array(e)) = s.get("enum") {
        return e
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect();
    }
    for k in ["oneOf", "anyOf"] {
        if let Some(Value::Array(a)) = s.get(k) {
            let c: Vec<String> = a
                .iter()
                .flat_map(|x| {
                    x.get("const")
                        .and_then(Value::as_str)
                        .map(|c| vec![c.to_string()])
                        .unwrap_or_else(|| choices(x))
                })
                .collect();
            if !c.is_empty() {
                return c;
            }
        }
    }
    Vec::new()
}

/// 設定のスキーマを項目の並びにする（`inline_subschemas` で作ったスキーマ）
pub fn params_of(schema: &Value, defaults: &Value) -> Vec<Param> {
    let Some(Value::Object(props)) = schema.get("properties") else {
        return Vec::new();
    };
    props
        .iter()
        .map(|(name, s)| {
            let t = types(s);
            let ch = choices(s);
            let ext = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let item_ty = s.get("items").map(types).unwrap_or_default();
            let ty = if ext("x-kind") == "json" {
                ParamTy::Json
            } else if ext("x-kind") == "path" {
                if t.contains(&"array") {
                    ParamTy::Paths
                } else {
                    ParamTy::Path
                }
            } else if !ch.is_empty() {
                ParamTy::Choice
            } else if t.contains(&"boolean") {
                ParamTy::Flag
            } else if t.contains(&"integer") {
                if s.get("minimum")
                    .and_then(Value::as_f64)
                    .is_some_and(|m| m >= 0.0)
                {
                    ParamTy::Count
                } else {
                    ParamTy::Int
                }
            } else if t.contains(&"number") {
                ParamTy::Num
            } else if t.contains(&"array")
                && item_ty.iter().any(|x| *x == "number" || *x == "integer")
            {
                ParamTy::Nums
            } else {
                ParamTy::Str
            };
            let len = match (s.get("minItems"), s.get("maxItems")) {
                (Some(a), Some(b)) if a == b => a.as_u64().map(|n| n as usize),
                _ => None,
            };
            // 0 で結果が空になる数（リング数・格子の一辺など）は x-min で下限を書く
            let min = s.get("x-min").and_then(Value::as_f64);
            Param {
                name: name.clone(),
                ty,
                default: defaults.get(name).cloned().unwrap_or(Value::Null),
                optional: nullable(s),
                doc: ext("description"),
                unit: ext("x-unit"),
                choices: ch,
                min,
                len,
                write: s.get("x-write").and_then(Value::as_bool).unwrap_or(false),
            }
        })
        .collect()
}

/// 解析の登録簿
#[derive(Clone, Default)]
pub struct Analyses {
    list: Vec<Arc<dyn DynAnalysis>>,
}

impl Analyses {
    /// 同梱の解析（[`crate::defs`]）
    pub fn builtin() -> &'static Analyses {
        static B: OnceLock<Analyses> = OnceLock::new();
        B.get_or_init(|| {
            let mut a = Analyses::default();
            crate::defs::register(&mut a);
            a
        })
    }

    /// 解析を足す（同じ名前があれば置き換える）
    pub fn add<A: Analysis>(&mut self, a: A) {
        let name = a.meta().name;
        self.list.retain(|x| x.meta().name != name);
        self.list.push(Arc::new(Erased(a, PhantomData)));
    }

    /// 型を消した解析を足す（同じ名前があれば置き換える）
    pub fn add_dyn(&mut self, a: Arc<dyn DynAnalysis>) {
        let name = a.meta().name;
        self.list.retain(|x| x.meta().name != name);
        self.list.push(a);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn DynAnalysis>> {
        self.list.iter().find(|a| a.meta().name == name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn DynAnalysis>> {
        self.list.iter()
    }
}
