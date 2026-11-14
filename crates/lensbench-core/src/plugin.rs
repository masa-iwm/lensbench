//! 検出器・解析のプラグインの約束。
//!
//! プラグインは [`Plugin`] を実装し、名前で [`Registry`] に登録する。呼ぶ側は名前と設定（[`Config`]: 文字列・数・
//! 数の並びの小さな辞書）から [`Instance`] を作り、名前つきの配列（[`Arrays`]）を渡して名前つきの配列を受け取る。
//!
//! 約束:
//! - [`Plugin::prepare`] で設定を確かめ、設定から決まる前処理（係数・作用素など）をここで済ませる。
//! - [`Instance::run`] は 1 回の呼び出しの中では、原則として単一スレッドで計算し（系を追跡するものは
//!   [`crate::with_threads`] の指定に従ってよい）、呼び出しの間で状態を持たない（同じ入力なら
//!   同じビットを返す）。`&self` で呼ぶので、同じ `Instance` を複数のスレッドから同時に呼んでよい
//!   （[`run_many`]）。入力の形によるキャッシュ（最小二乗の作用素など）は、結果を変えない限り中で持ってよい。
//! - 入力の名前・形・型が合わなければ [`crate::Error::Invalid`] を返す（panic しない）。
//!
//! 組み込みのプラグインはワークスペース内の別 crate に置き、その crate の `register(&mut Registry)` で登録する
//! （例: `lensbench-analysis`）。Python からのコールバックも同じ [`Plugin`] として登録できる。

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

use crate::coating::{Coatings, C64};
use crate::{invalid, Result, System};

/// プラグインの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// 検出器: 検出面の場・光線から画素の値を作る
    Detector,
    /// 解析: 結果の配列から値・配列を作る
    Analysis,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Detector => "detector",
            Kind::Analysis => "analysis",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "detector" => Ok(Kind::Detector),
            "analysis" => Ok(Kind::Analysis),
            _ => invalid!("プラグインの種類は detector か analysis（{s}）"),
        }
    }
}

/// 設定の値（系ファイルの JSON では数・文字列・数の並びをそのまま書く）
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(untagged)
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Value {
    Num(f64),
    Str(String),
    /// 数の並び（角度 4 個など）
    Nums(Vec<f64>),
}

/// 設定: 名前 → 値
pub type Config = BTreeMap<String, Value>;

/// 設定を読む補助。知らない名前を残さないよう、読んだ名前を覚えて [`ConfigReader::finish`] で確かめる
pub struct ConfigReader<'a> {
    cfg: &'a Config,
    used: Vec<&'a str>,
}

impl<'a> ConfigReader<'a> {
    pub fn new(cfg: &'a Config) -> Self {
        Self {
            cfg,
            used: Vec::new(),
        }
    }

    fn get(&mut self, key: &str) -> Option<&'a Value> {
        let (k, v) = self.cfg.get_key_value(key)?;
        self.used.push(k);
        Some(v)
    }

    /// 数（省けば `default`）
    pub fn num(&mut self, key: &str, default: f64) -> Result<f64> {
        match self.get(key) {
            None => Ok(default),
            Some(Value::Num(x)) => Ok(*x),
            Some(Value::Nums(v)) if v.len() == 1 => Ok(v[0]),
            Some(v) => invalid!("設定 {key} は数に（{v:?}）"),
        }
    }

    /// 文字列（省けば `default`）
    pub fn str(&mut self, key: &str, default: &str) -> Result<String> {
        match self.get(key) {
            None => Ok(default.to_string()),
            Some(Value::Str(s)) => Ok(s.clone()),
            Some(v) => invalid!("設定 {key} は文字列に（{v:?}）"),
        }
    }

    /// 数の並び（省けば None）。数 1 つも長さ 1 の並びとして受ける
    pub fn nums(&mut self, key: &str) -> Result<Option<Vec<f64>>> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Num(x)) => Ok(Some(vec![*x])),
            Some(Value::Nums(v)) => Ok(Some(v.clone())),
            Some(v) => invalid!("設定 {key} は数の並びに（{v:?}）"),
        }
    }

    /// 0 以上の整数（省けば `default`）
    pub fn count(&mut self, key: &str, default: usize) -> Result<usize> {
        Ok(self.opt_count(key)?.unwrap_or(default))
    }

    /// 0 以上の整数（省けば None）
    pub fn opt_count(&mut self, key: &str) -> Result<Option<usize>> {
        let x = self.num(key, f64::NAN)?;
        if x.is_nan() {
            return Ok(None);
        }
        if !(x >= 0.0) || x.fract() != 0.0 || x > u32::MAX as f64 {
            invalid!("設定 {key} は 0 以上の整数に（{x}）");
        }
        Ok(Some(x as usize))
    }

    /// 0 か 1（省けば `default`）
    pub fn flag(&mut self, key: &str, default: bool) -> Result<bool> {
        match self.num(key, if default { 1.0 } else { 0.0 })? {
            0.0 => Ok(false),
            1.0 => Ok(true),
            x => invalid!("設定 {key} は 0 か 1 に（{x}）"),
        }
    }

    /// 波長の番号（1 始まり、省けば None）→ 0 始まり
    pub fn wave(&mut self, key: &str) -> Result<Option<usize>> {
        match self.opt_count(key)? {
            Some(0) => invalid!("設定 {key} は 1 始まり"),
            w => Ok(w.map(|w| w - 1)),
        }
    }

    /// 読まなかった名前があれば誤り（綴りの誤りを黙って無視しない）
    pub fn finish(self) -> Result<()> {
        let unknown: Vec<&str> = self
            .cfg
            .keys()
            .map(String::as_str)
            .filter(|k| !self.used.contains(k))
            .collect();
        if !unknown.is_empty() {
            invalid!("知らない設定: {}", unknown.join(", "));
        }
        Ok(())
    }
}

/// 配列の値（行優先の平らな並び）
#[derive(Debug, Clone, PartialEq)]
pub enum Data {
    F64(Vec<f64>),
    C64(Vec<C64>),
    Bool(Vec<bool>),
}

impl Data {
    pub fn len(&self) -> usize {
        match self {
            Data::F64(v) => v.len(),
            Data::C64(v) => v.len(),
            Data::Bool(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn dtype(&self) -> &'static str {
        match self {
            Data::F64(_) => "float64",
            Data::C64(_) => "complex128",
            Data::Bool(_) => "bool",
        }
    }
}

/// 形つきの配列
#[derive(Debug, Clone, PartialEq)]
pub struct Array {
    pub shape: Vec<usize>,
    pub data: Data,
}

impl Array {
    /// 形と値の数が合うか確かめて作る
    pub fn new(shape: Vec<usize>, data: Data) -> Result<Self> {
        let n: usize = shape.iter().product();
        if n != data.len() {
            invalid!("配列の形 {shape:?} と値の数 {} が合わない", data.len());
        }
        Ok(Self { shape, data })
    }

    pub fn f64(shape: Vec<usize>, v: Vec<f64>) -> Result<Self> {
        Self::new(shape, Data::F64(v))
    }

    pub fn c64(shape: Vec<usize>, v: Vec<C64>) -> Result<Self> {
        Self::new(shape, Data::C64(v))
    }

    pub fn bool(shape: Vec<usize>, v: Vec<bool>) -> Result<Self> {
        Self::new(shape, Data::Bool(v))
    }

    /// 数 1 つ（形 []）
    pub fn scalar(x: f64) -> Self {
        Self {
            shape: Vec::new(),
            data: Data::F64(vec![x]),
        }
    }

    pub fn as_f64(&self) -> Option<&[f64]> {
        match &self.data {
            Data::F64(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_c64(&self) -> Option<&[C64]> {
        match &self.data {
            Data::C64(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<&[bool]> {
        match &self.data {
            Data::Bool(v) => Some(v),
            _ => None,
        }
    }
}

/// 名前つきの配列（入力・出力）
pub type Arrays = BTreeMap<String, Array>;

/// 入力を読む補助（名前・型の確かめと、知らない名前の検出）
pub struct ArrayReader<'a> {
    arrs: &'a Arrays,
    used: Vec<&'a str>,
}

impl<'a> ArrayReader<'a> {
    pub fn new(arrs: &'a Arrays) -> Self {
        Self {
            arrs,
            used: Vec::new(),
        }
    }

    /// 名前の配列（省いてよい）
    pub fn opt(&mut self, key: &str) -> Option<&'a Array> {
        let (k, v) = self.arrs.get_key_value(key)?;
        self.used.push(k);
        Some(v)
    }

    /// 名前の配列（必須）
    pub fn req(&mut self, key: &str) -> Result<&'a Array> {
        match self.opt(key) {
            Some(a) => Ok(a),
            None => invalid!("入力 {key} がない"),
        }
    }

    /// float64 の配列（必須）→ (形, 値)
    pub fn f64(&mut self, key: &str) -> Result<(&'a [usize], &'a [f64])> {
        let a = self.req(key)?;
        match a.as_f64() {
            Some(v) => Ok((&a.shape, v)),
            None => invalid!("入力 {key} は float64 に（{}）", a.data.dtype()),
        }
    }

    /// complex128 の配列（必須）→ (形, 値)
    pub fn c64(&mut self, key: &str) -> Result<(&'a [usize], &'a [C64])> {
        let a = self.req(key)?;
        match a.as_c64() {
            Some(v) => Ok((&a.shape, v)),
            None => invalid!("入力 {key} は complex128 に（{}）", a.data.dtype()),
        }
    }

    /// 読まなかった名前があれば誤り
    pub fn finish(self) -> Result<()> {
        let unknown: Vec<&str> = self
            .arrs
            .keys()
            .map(String::as_str)
            .filter(|k| !self.used.contains(k))
            .collect();
        if !unknown.is_empty() {
            invalid!("知らない入力: {}", unknown.join(", "));
        }
        Ok(())
    }
}

/// 視野の入力: 数 1 つなら番号（1 始まり）、2 個なら正規化視野 (hx, hy)
pub fn field_sel(s: &System, a: &Array) -> Result<crate::FieldSel> {
    let Some(v) = a.as_f64() else {
        invalid!("視野は数に");
    };
    match v {
        [i] => {
            let n = s.fields.len();
            if i.fract() != 0.0 || *i < 1.0 || *i > n as f64 {
                invalid!("視野は 1〜{n}（{i}）");
            }
            Ok(crate::FieldSel::Index(*i as usize - 1))
        }
        [hx, hy] => Ok(crate::FieldSel::Norm(*hx, *hy)),
        _ => invalid!("視野は数 1 つか 2 個に（{} 個）", v.len()),
    }
}

/// 波長の番号（0 始まり）が系の範囲にあるか
pub fn check_wave(s: &System, w: Option<usize>) -> Result<()> {
    if let Some(w) = w {
        if w >= s.wavelengths.len() {
            invalid!("波長は 1〜{}（{}）", s.wavelengths.len(), w + 1);
        }
    }
    Ok(())
}

/// 入出力の名前・形の説明（一覧・ヘルプ用。呼び出しの確かめは [`Instance::run`] が行う）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spec {
    /// 1 行の説明
    pub doc: String,
    /// 設定: (名前, 説明)
    pub config: Vec<(String, String)>,
    /// 入力: (名前, 説明)
    pub inputs: Vec<(String, String)>,
    /// 出力: (名前, 説明)
    pub outputs: Vec<(String, String)>,
    /// 系（[`Context::system`]）が要るか
    pub needs_system: bool,
}

/// 設定の項目の型（GUI の欄を作る）
#[derive(Debug, Clone, PartialEq)]
pub enum ParamType {
    /// 実数
    Num,
    /// 0 以上の整数
    Count,
    /// 0 か 1（[`ConfigReader::flag`]）
    Flag,
    /// 文字列
    Str,
    /// 選択肢の文字列
    Choice(Vec<String>),
    /// 数の並び（`len` は個数の上限。1 個なら全部に同じ値などの決まりは説明に書く）
    Nums { len: Option<usize> },
}

impl ParamType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ParamType::Num => "num",
            ParamType::Count => "count",
            ParamType::Flag => "flag",
            ParamType::Str => "str",
            ParamType::Choice(_) => "choice",
            ParamType::Nums { .. } => "nums",
        }
    }
}

/// 設定の 1 項目の説明（名前・型・既定・単位・説明）。[`Plugin::params`] で返す
#[derive(Debug, Clone, PartialEq)]
pub struct ParamSpec {
    /// 設定の名前（入れ子は「.」でつなぐ。例 `mla.pitch`）
    pub name: String,
    pub ty: ParamType,
    /// 既定値（None は省けば自動で決まる・使わない）
    pub default: Option<Value>,
    /// 単位（mm・µm など。無ければ空）
    pub unit: String,
    /// 1 行の説明
    pub doc: String,
}

impl ParamSpec {
    pub fn new(name: &str, ty: ParamType, default: Option<Value>, unit: &str, doc: &str) -> Self {
        ParamSpec {
            name: name.into(),
            ty,
            default,
            unit: unit.into(),
            doc: doc.into(),
        }
    }
}

/// 呼び出しの文脈: 配列のほかに渡すもの（系・膜）。配列だけのプラグインは見ない
#[derive(Clone, Copy, Default)]
pub struct Context<'a> {
    /// 系（追跡するプラグイン用）
    pub system: Option<&'a Arc<System>>,
    /// 系の膜（偏光追跡で使う。None なら膜なし）
    pub coatings: Option<&'a Coatings>,
}

impl<'a> Context<'a> {
    /// 系だけの文脈
    pub fn with_system(system: &'a Arc<System>) -> Self {
        Self {
            system: Some(system),
            coatings: None,
        }
    }

    /// 系（なければ誤り）
    pub fn system(&self) -> Result<&'a System> {
        match self.system {
            Some(s) => Ok(s),
            None => invalid!("このプラグインには系が要る"),
        }
    }
}

/// プラグイン（設定から [`Instance`] を作る）
pub trait Plugin: Send + Sync {
    /// 登録の名前
    fn name(&self) -> &str;
    fn kind(&self) -> Kind;
    /// 入出力の説明
    fn spec(&self) -> Spec {
        Spec::default()
    }
    /// 設定の項目の型・既定値（GUI の欄を作る。空なら [`Spec::config`] の名前と説明だけ）
    fn params(&self) -> Vec<ParamSpec> {
        Vec::new()
    }
    /// 設定を確かめて呼び出しの準備をする
    fn prepare(&self, config: &Config) -> Result<Box<dyn Instance>>;
}

/// 設定の決まったプラグイン（呼び出しの単位）
pub trait Instance: Send + Sync {
    /// 入力から出力を作る（状態なし）
    fn run(&self, ctx: &Context<'_>, inputs: &Arrays) -> Result<Arrays>;

    /// 線形化: [`Instance::run`] と同じ出力と、入力の微分 1 方向から出力の微分を作る [`Linear`]。多くの方向の微分を
    /// 流すとき、解析を 1 回で済ませる口。既定は None（この口が無い。呼ぶ側は微分を run の入力に渡す）
    fn linearize(
        &self,
        _ctx: &Context<'_>,
        _inputs: &Arrays,
    ) -> Result<Option<(Arrays, Box<dyn Linear + '_>)>> {
        Ok(None)
    }
}

/// [`Instance::linearize`] の線形の写像
pub trait Linear: Send + Sync {
    /// 入力の微分 1 方向（名前・形はプラグインの決めたもの）から出力の微分
    fn apply(&self, d: &Arrays) -> Result<Arrays>;
}

/// 名前 → プラグイン
#[derive(Clone, Default)]
pub struct Registry {
    map: BTreeMap<String, Arc<dyn Plugin>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登録する（同じ名前は差し替える）
    pub fn register(&mut self, p: Arc<dyn Plugin>) {
        self.map.insert(p.name().to_string(), p);
    }

    /// 外す（あれば true）
    pub fn unregister(&mut self, name: &str) -> bool {
        self.map.remove(name).is_some()
    }

    pub fn get(&self, name: &str) -> Result<Arc<dyn Plugin>> {
        match self.map.get(name) {
            Some(p) => Ok(p.clone()),
            None => invalid!(
                "プラグイン {name} はない（登録: {}）",
                self.map.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        }
    }

    /// 登録のプラグイン（名前の順）
    pub fn plugins(&self) -> impl Iterator<Item = &Arc<dyn Plugin>> {
        self.map.values()
    }

    /// 登録の (名前, 種類) の一覧（名前の順）
    pub fn list(&self) -> Vec<(String, Kind)> {
        self.map
            .iter()
            .map(|(k, p)| (k.clone(), p.kind()))
            .collect()
    }

    /// 名前と設定から呼び出しの準備
    pub fn prepare(&self, name: &str, config: &Config) -> Result<Box<dyn Instance>> {
        self.get(name)?.prepare(config)
    }
}

/// プロセスで共通の登録（Python から使う。Rust からは [`Registry`] を自分で持ってもよい）。
/// CLI・GUI も同梱の分にこの登録を足して使うので、別の exe に組み込むときは起動の最初にここへ登録する
pub fn global() -> &'static RwLock<Registry> {
    static G: OnceLock<RwLock<Registry>> = OnceLock::new();
    G.get_or_init(Default::default)
}

/// 同じ準備で多数の入力を呼ぶ（入力ごとに独立、スレッド数は [`crate::with_threads`] に従う）。
/// 結果は入力の順で、スレッド数によらず同じビット
pub fn run_many(inst: &dyn Instance, ctx: &Context<'_>, inputs: &[Arrays]) -> Vec<Result<Arrays>> {
    crate::par::map(inputs, |a| inst.run(ctx, a))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 入力 x を k 倍して和も返す、試験用のプラグイン
    struct Scale;
    struct ScaleInst(f64);

    impl Plugin for Scale {
        fn name(&self) -> &str {
            "scale"
        }
        fn kind(&self) -> Kind {
            Kind::Analysis
        }
        fn prepare(&self, config: &Config) -> Result<Box<dyn Instance>> {
            let mut r = ConfigReader::new(config);
            let k = r.num("k", 1.0)?;
            r.finish()?;
            Ok(Box::new(ScaleInst(k)))
        }
    }

    impl Instance for ScaleInst {
        fn run(&self, _ctx: &Context<'_>, inputs: &Arrays) -> Result<Arrays> {
            let mut r = ArrayReader::new(inputs);
            let (s, x) = r.f64("x")?;
            r.finish()?;
            let y: Vec<f64> = x.iter().map(|v| v * self.0).collect();
            let sum = y.iter().sum();
            Ok(Arrays::from([
                ("y".into(), Array::f64(s.to_vec(), y)?),
                ("sum".into(), Array::scalar(sum)),
            ]))
        }
    }

    fn input(x: Vec<f64>) -> Arrays {
        Arrays::from([("x".into(), Array::f64(vec![x.len()], x).unwrap())])
    }

    #[test]
    fn registry_and_run() {
        let mut reg = Registry::new();
        reg.register(Arc::new(Scale));
        assert_eq!(reg.list(), vec![("scale".to_string(), Kind::Analysis)]);
        let inst = reg
            .prepare("scale", &Config::from([("k".into(), Value::Num(2.0))]))
            .unwrap();
        let out = inst
            .run(&Context::default(), &input(vec![1.0, 2.5]))
            .unwrap();
        assert_eq!(out["y"].as_f64().unwrap(), &[2.0, 5.0]);
        assert_eq!(out["sum"].shape, Vec::<usize>::new());
        assert!(reg.get("none").is_err());
        assert!(reg.unregister("scale") && reg.list().is_empty());
    }

    #[test]
    fn rejects_unknown_and_wrong_types() {
        let p = Scale;
        assert!(p
            .prepare(&Config::from([("kk".into(), Value::Num(2.0))]))
            .is_err());
        assert!(p
            .prepare(&Config::from([("k".into(), Value::Str("a".into()))]))
            .is_err());
        let inst = p.prepare(&Config::new()).unwrap();
        let mut a = input(vec![1.0]);
        a.insert("z".into(), Array::scalar(0.0));
        assert!(inst.run(&Context::default(), &a).is_err());
        let a = Arrays::from([(
            "x".into(),
            Array::c64(vec![1], vec![C64::new(1.0, 0.0)]).unwrap(),
        )]);
        assert!(inst.run(&Context::default(), &a).is_err());
        assert!(Array::f64(vec![2, 2], vec![0.0; 3]).is_err());
        assert!(Kind::parse("x").is_err() && Kind::parse("detector").unwrap() == Kind::Detector);
    }

    #[test]
    fn run_many_is_ordered_and_thread_independent() {
        let inst = Scale.prepare(&Config::new()).unwrap();
        let inputs: Vec<Arrays> = (0..40).map(|i| input(vec![i as f64; 3])).collect();
        let a = crate::with_threads(Some(1), || {
            run_many(inst.as_ref(), &Context::default(), &inputs)
        });
        let b = crate::with_threads(Some(4), || {
            run_many(inst.as_ref(), &Context::default(), &inputs)
        });
        assert_eq!(a, b);
        assert_eq!(a[7].as_ref().unwrap()["sum"].as_f64().unwrap(), &[21.0]);
    }

    /// 系を受け取って中で追跡するプラグイン（スポットの RMS）
    struct SpotRms;
    struct SpotRmsInst(usize);

    impl Plugin for SpotRms {
        fn name(&self) -> &str {
            "spot_rms"
        }
        fn kind(&self) -> Kind {
            Kind::Analysis
        }
        fn prepare(&self, config: &Config) -> Result<Box<dyn Instance>> {
            let mut r = ConfigReader::new(config);
            let rings = r.num("rings", 6.0)? as usize;
            r.finish()?;
            Ok(Box::new(SpotRmsInst(rings)))
        }
    }

    impl Instance for SpotRmsInst {
        fn run(&self, ctx: &Context<'_>, inputs: &Arrays) -> Result<Arrays> {
            let s = ctx.system()?;
            let mut r = ArrayReader::new(inputs);
            let (_, f) = r.f64("field")?;
            r.finish()?;
            let sp = crate::analysis::spot(s, crate::FieldSel::Index(f[0] as usize), None, self.0)?;
            Ok(Arrays::from([(
                "rms".into(),
                Array::scalar(sp.rms_centroid),
            )]))
        }
    }

    #[test]
    fn plugin_with_system() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let opt = crate::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let sys = crate::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap();
        let sys = Arc::new(sys);
        let inst = SpotRms.prepare(&Config::new()).unwrap();
        let inp: Vec<Arrays> = (0..2)
            .map(|i| Arrays::from([("field".into(), Array::scalar(i as f64))]))
            .collect();
        assert!(inst.run(&Context::default(), &inp[0]).is_err());
        let ctx = Context::with_system(&sys);
        let out = run_many(inst.as_ref(), &ctx, &inp);
        for (i, o) in out.iter().enumerate() {
            let want = crate::analysis::spot(&sys, crate::FieldSel::Index(i), None, 6).unwrap();
            assert_eq!(
                o.as_ref().unwrap()["rms"].as_f64().unwrap(),
                &[want.rms_centroid]
            );
        }
    }
}
