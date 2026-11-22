//! 最適化（DLS）とズーム軌跡の設定の型（CLI の旗・serve の params・Python の引数の元）
//!
//! 既定値は [`OptimizeParams`]・[`ZoomParams`] の `Default` だけに書く。CLI の `default_value_t`、Python の引数の既定は
//! ここから取り（Python は試験で突き合わせる）、serve は JSON をこの型に読む。

use lensbench_analysis::sweep::RmsRef;
use lensbench_core::param::Param;
use lensbench_core::{Error, Result, System};
use lensbench_optim::zoom::{TargetMethod, ZoomSpec};
use lensbench_optim::{DefaultMerit, DlsOpts, JacMode, ParaxQ, Problem, Var};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::fmt;
use std::str::FromStr;

/// 変数: "7.t"、範囲つき "7.t=0.5:3"（片側は "7.t=0.5:"・"7.t=:3"）、または {"name", "lo", "hi", "configs"}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum VarSpec {
    /// 名前（範囲は "=lo:hi" で書ける）
    Name(String),
    /// 名前と範囲・効く構成
    Full {
        name: String,
        /// 下限（null・省略で下限なし）
        #[serde(default)]
        lo: Option<f64>,
        /// 上限
        #[serde(default)]
        hi: Option<f64>,
        /// 効く構成の番号（1 始まり。省略ですべての構成）。最適化だけ
        #[serde(default)]
        configs: Option<Vec<usize>>,
    },
}

/// 変数を読んだもの
#[derive(Debug, Clone, PartialEq)]
pub struct VarDef {
    pub name: String,
    pub lo: f64,
    pub hi: f64,
    /// 効く構成（0 始まり。None ですべて）
    pub configs: Option<Vec<usize>>,
}

impl From<&str> for VarSpec {
    fn from(s: &str) -> Self {
        VarSpec::Name(s.to_string())
    }
}

impl VarSpec {
    /// 文字の書き方（"7.t"・"7.t=lo:hi"）の並びを読み、範囲の書き方を確かめる
    pub fn parse_all(v: &[String]) -> Result<Vec<VarSpec>> {
        let v: Vec<VarSpec> = v.iter().map(|x| VarSpec::from(x.as_str())).collect();
        v.iter().try_for_each(|x| x.resolve(1).map(drop))?;
        Ok(v)
    }

    /// 名前と範囲を読む（構成の番号は 1〜n_configs を確かめて 0 始まりに）
    pub fn resolve(&self, n_configs: usize) -> Result<VarDef> {
        match self {
            VarSpec::Name(v) => {
                let (name, range) = match v.split_once('=') {
                    Some((a, b)) => (a.trim(), Some(b)),
                    None => (v.trim(), None),
                };
                let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
                if let Some(r) = range {
                    let (a, b) = r.split_once(':').ok_or_else(|| {
                        Error::Invalid(format!("the range of variable {v} is min:max"))
                    })?;
                    let num = |t: &str, d: f64| -> Result<f64> {
                        let t = t.trim();
                        if t.is_empty() {
                            Ok(d)
                        } else {
                            t.parse().map_err(|_| {
                                Error::Invalid(format!("range {t} of variable {v} can't be read"))
                            })
                        }
                    };
                    lo = num(a, lo)?;
                    hi = num(b, hi)?;
                }
                Ok(VarDef {
                    name: name.to_string(),
                    lo,
                    hi,
                    configs: None,
                })
            }
            VarSpec::Full {
                name,
                lo,
                hi,
                configs,
            } => {
                let configs = configs
                    .as_ref()
                    .map(|c| {
                        c.iter()
                            .map(|&k| {
                                if k >= 1 && k <= n_configs {
                                    Ok(k - 1)
                                } else {
                                    Err(Error::Invalid(format!(
                                        "configs of variable {name}: {k} (1 to {n_configs})"
                                    )))
                                }
                            })
                            .collect::<Result<Vec<_>>>()
                    })
                    .transpose()?;
                Ok(VarDef {
                    name: name.clone(),
                    lo: lo.unwrap_or(f64::NEG_INFINITY),
                    hi: hi.unwrap_or(f64::INFINITY),
                    configs,
                })
            }
        }
    }
}

macro_rules! str_enum {
    ($(#[$m:meta])* $name:ident ($label:literal) { $($(#[$vm:meta])* $v:ident = $s:literal $(| $alias:literal)*),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, JsonSchema)]
        #[serde(rename_all = "lowercase")]
        pub enum $name {
            $($(#[$vm])* $v),+
        }
        impl $name {
            /// 選べる名前
            pub const NAMES: &'static [&'static str] = &[$($s),+];
            pub fn as_str(self) -> &'static str {
                match self { $($name::$v => $s),+ }
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
        impl FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> std::result::Result<Self, String> {
                match s.to_ascii_lowercase().as_str() {
                    $($s $(| $alias)* => Ok($name::$v),)+
                    _ => Err(format!("{s} (one of {})", Self::NAMES.join(", "))),
                }
            }
        }
        /// 名前（大文字・別名も）を読み、誤りには欄の名前を付ける（serve の誤りでどの欄か分かるように）
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                s.parse()
                    .map_err(|e| serde::de::Error::custom(format!("{}: {e}", $label)))
            }
        }
    };
}

str_enum!(
    /// 既定の評価関数（全視野・全波長の RMS）
    Merit ("merit") {
        /// RMS スポット
        #[default]
        Spot = "spot",
        /// RMS 波面
        Wavefront = "wavefront",
        /// なし（足した項だけ）
        None = "none",
    }
);

str_enum!(
    /// 像の評価関数（ズーム軌跡。なしは選べない）
    ImageMerit ("merit") {
        #[default]
        Spot = "spot",
        Wavefront = "wavefront",
    }
);

str_enum!(
    /// RMS の基準
    Reference ("reference") {
        /// 重心
        #[default]
        Centroid = "centroid",
        /// 主光線
        Chief = "chief",
    }
);

str_enum!(
    /// ヤコビアンの求め方
    Jac ("jac") {
        /// 解析的な列が 16 以上の項は解析的、少ない項は中心差分
        #[default]
        Auto = "auto",
        /// すべて中心差分
        Fd = "fd",
        /// すべて前進差分
        Forward = "forward",
    }
);

str_enum!(
    /// ズーム軌跡の近軸量の目標の満たし方
    ZoomMethod ("method") {
        /// 罰金: 目標の項の重みを上げる
        Penalty = "penalty",
        /// 拡大ラグランジュ法: 項の目標をずらす
        #[default]
        Lagrange = "lagrange",
    }
);

impl From<ZoomMethod> for TargetMethod {
    fn from(m: ZoomMethod) -> TargetMethod {
        match m {
            ZoomMethod::Penalty => TargetMethod::Penalty,
            ZoomMethod::Lagrange => TargetMethod::Lagrange,
        }
    }
}

/// 選べる DLS の設定（[`DlsOpts`] の qr・gtol・active_set。optimize・zoom）
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DlsChoice {
    /// 一歩を QR で解く
    pub qr: bool,
    /// 勾配の判定（0 は判定しない）
    pub gtol: f64,
    /// 境界の有効制約法
    pub active_set: bool,
}

impl DlsChoice {
    /// 反復の上限 iters の DLS の設定（ほかは既定）
    pub fn opts(self, iters: usize) -> DlsOpts {
        DlsOpts {
            max_iter: iters,
            qr: self.qr,
            gtol: self.gtol,
            active_set: self.active_set,
            ..Default::default()
        }
    }
}

impl From<Reference> for RmsRef {
    fn from(r: Reference) -> RmsRef {
        match r {
            Reference::Centroid => RmsRef::Centroid,
            Reference::Chief => RmsRef::Chief,
        }
    }
}

impl From<Jac> for JacMode {
    fn from(j: Jac) -> JacMode {
        match j {
            Jac::Auto => JacMode::Auto,
            Jac::Fd => JacMode::FiniteDiff,
            Jac::Forward => JacMode::Forward,
        }
    }
}

impl From<ImageMerit> for DefaultMerit {
    fn from(m: ImageMerit) -> DefaultMerit {
        match m {
            ImageMerit::Spot => DefaultMerit::Spot,
            ImageMerit::Wavefront => DefaultMerit::Wavefront,
        }
    }
}

impl Merit {
    pub fn default_merit(self) -> Option<DefaultMerit> {
        match self {
            Merit::Spot => Some(DefaultMerit::Spot),
            Merit::Wavefront => Some(DefaultMerit::Wavefront),
            Merit::None => None,
        }
    }
}

/// 0 以上の数（gtol）。負の数と NaN は誤り（CLI の旗・serve の params で使い方の誤りにする）
pub fn non_negative(v: f64) -> std::result::Result<f64, String> {
    if v.is_nan() || v < 0.0 {
        Err(format!("must be 0 or more ({v})"))
    } else {
        Ok(v)
    }
}

/// CLI の旗の値（0 以上の数）。数でなければ数の読みの誤りのまま
pub fn parse_non_negative(
    s: &str,
) -> std::result::Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    Ok(non_negative(s.trim().parse()?)?)
}

fn non_negative_de<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<f64, D::Error> {
    non_negative(f64::deserialize(d)?).map_err(serde::de::Error::custom)
}

/// null は none（なし）として読む
fn merit_or_null<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Merit, D::Error> {
    Ok(Option::<Merit>::deserialize(d)?.unwrap_or(Merit::None))
}

/// 並び（null は空、並びでない値 1 つは 1 つの並び。serve の以前の書き方も通す）
fn seq<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Vec<T>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany<T> {
        Many(Vec<T>),
        One(T),
    }
    Ok(match Option::<OneOrMany<T>>::deserialize(d)? {
        None => Vec::new(),
        Some(OneOrMany::Many(v)) => v,
        Some(OneOrMany::One(x)) => vec![x],
    })
}

/// JSON の設定を読む。誤りにはどの鍵の値かを付ける（serde の型の誤りは鍵の名前を言わないので、鍵ごとに読み直して探す）
pub fn read<T: serde::de::DeserializeOwned>(v: &Value) -> lensbench_core::Result<T> {
    let e = match serde_json::from_value(v.clone()) {
        Ok(t) => return Ok(t),
        Err(e) => e.to_string(),
    };
    let bad = v.as_object().and_then(|o| {
        o.iter()
            .find(|(k, x)| {
                let one = Value::Object([((*k).clone(), (*x).clone())].into_iter().collect());
                serde_json::from_value::<T>(one).is_err()
            })
            .map(|(k, _)| k.clone())
    });
    Err(match bad {
        Some(k) if !e.starts_with(&format!("{k}:")) && !e.contains(&format!("`{k}`")) => {
            format!("{k}: {e}").into()
        }
        _ => e.into(),
    })
}

/// 最適化（DLS）の設定。CLI の `optimize`・serve の `optimize`・Python の `System.optimize`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct OptimizeParams {
    /// 変数（"7.t"・"7.t=0.5:3"・{"name", "lo", "hi", "configs"}）
    #[serde(deserialize_with = "seq")]
    pub vars: Vec<VarSpec>,
    /// 既定の評価関数: spot・wavefront・none（null も none）
    #[serde(deserialize_with = "merit_or_null")]
    pub merit: Merit,
    /// 足す項（評価関数の項の JSON の並び）
    #[serde(deserialize_with = "seq")]
    pub terms: Vec<Value>,
    /// 基準
    pub reference: Reference,
    /// 瞳のガウス求積のリング数
    #[schemars(extend("x-min" = 1))]
    pub rings: usize,
    /// 反復の上限
    pub iters: usize,
    /// ヤコビアンの求め方
    pub jac: Jac,
    /// 構成（.zmx の構成の番号かプロジェクトの状態の名前）の並び。省けば読み込んだ系だけ（serve）
    #[serde(deserialize_with = "seq")]
    pub configs: Vec<Value>,
    /// 最適化した値を読み込んだ系に当てる（serve）
    pub apply: bool,
    /// 一歩を正規方程式でなく QR で解く
    pub qr: bool,
    /// 残差と変数の列の余弦の最大がこれより小さければ止める（0 は判定しない）
    #[schemars(extend("x-min" = 0))]
    #[serde(deserialize_with = "non_negative_de")]
    pub gtol: f64,
    /// 境界: 範囲を出る変数を境界に止めて残りを解き直す（成分ごとに切らずに）
    pub active_set: bool,
    /// 止まった光線（全反射・面に届かない）をけられた光線と同じく除いて評価する（既定は評価できない誤り）
    pub stopped_as_vignetted: bool,
    /// 大域の探索の始点の数（今の値の始点を含む。0・1 は局所の最適化だけ）
    pub starts: usize,
    /// 大域の探索の乱数の種
    pub seed: u64,
    /// 大域の探索で残す解の数
    #[schemars(extend("x-min" = 1))]
    pub keep: usize,
    /// 大域の探索で範囲の無い変数を振る幅（目安の大きさ max(|値|, 種類ごとの目安) に対する比）
    pub spread: f64,
    /// ガラスの置き換え（省けばしない）。最適化（大域の探索）の後に、面の硝材をカタログの硝材から選ぶ
    pub glass: Option<GlassParams>,
}

/// ガラスの置き換えの設定（[`OptimizeParams::glass`]、[`crate::glass_sub`]）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GlassParams {
    /// 置き換える面の番号
    pub surfaces: Vec<usize>,
    /// 候補のカタログ（"SCHOTT" など。省けば系の読み込んだガラス）
    #[serde(deserialize_with = "seq")]
    pub catalogs: Vec<String>,
    /// 面ごとに比べる候補の数（今の実効の nd・vd に近い順）
    #[schemars(extend("x-min" = 1))]
    pub candidates: usize,
    /// 面を一巡する回数の上限
    #[schemars(extend("x-min" = 1))]
    pub passes: usize,
}

impl Default for GlassParams {
    fn default() -> Self {
        GlassParams {
            surfaces: Vec::new(),
            catalogs: Vec::new(),
            candidates: 8,
            passes: 2,
        }
    }
}

/// [`OptimizeParams::run`] の結果
#[derive(Debug, Clone)]
pub struct OptimizeRun {
    /// 最後の問題（ガラスの置き換えで dn・disp の変数を外したもの。構成は最適化した系）
    pub problem: Problem,
    /// 最良の局所の最適化の結果（x0・merit0 は今の値のもので元の問題の変数の並び）。x は最後の問題の変数の並び
    /// （元の問題の変数の並びの値は [`OptimizeRun::x`]）
    pub result: lensbench_optim::DlsResult,
    /// 元の問題の変数ごとの最後の値（ガラスの置き換えで外した dn・disp は 0）
    pub x: Vec<f64>,
    /// 大域の探索の結果（回したとき）
    pub global: Option<lensbench_optim::GlobalResult>,
    /// ガラスの置き換えの記録（回したとき）
    pub glass: Option<GlassRun>,
}

/// ガラスの置き換えの記録
#[derive(Debug, Clone)]
pub struct GlassRun {
    pub changes: Vec<crate::glass_sub::GlassChange>,
    /// 評価した候補の数
    pub tried: usize,
    /// 面ごとの最後の材料の名前（置き換える面の順）
    pub materials: Vec<(usize, String)>,
    /// 置き換える前の評価関数の値
    pub merit_before: f64,
}

impl Default for OptimizeParams {
    fn default() -> Self {
        OptimizeParams {
            vars: Vec::new(),
            merit: Merit::Spot,
            terms: Vec::new(),
            reference: Reference::Centroid,
            rings: 3,
            iters: 50,
            jac: Jac::Auto,
            configs: Vec::new(),
            apply: false,
            qr: false,
            gtol: 0.0,
            active_set: false,
            stopped_as_vignetted: false,
            starts: 0,
            seed: 1,
            keep: 5,
            spread: 0.5,
            glass: None,
        }
    }
}

impl OptimizeParams {
    /// 大域の探索の設定（starts が 2 以上のとき）
    pub fn global_opts(&self) -> Option<lensbench_optim::GlobalOpts> {
        (self.starts >= 2).then(|| lensbench_optim::GlobalOpts {
            starts: self.starts,
            keep: self.keep,
            seed: self.seed,
            spread: self.spread,
            local: self.dls_opts(),
        })
    }

    /// 最適化を回す: 局所の最適化か大域の探索（[`OptimizeParams::starts`]）、続けてガラスの置き換え（[`OptimizeParams::glass`]）。
    /// 進み具合は、局所の最適化なら反復ごと、大域の探索なら始点ごとに知らせる（ガラスの置き換えは知らせない）
    pub fn run(&self, p: &Problem) -> Result<OptimizeRun> {
        self.run_in(p, &[])
    }

    /// [`OptimizeParams::run`]。ガラスの置き換えの候補のカタログを、フォルダ glass_dirs（の後に既定のフォルダ）から探す
    pub fn run_in(&self, p: &Problem, glass_dirs: &[std::path::PathBuf]) -> Result<OptimizeRun> {
        let o = self.dls_opts();
        let (mut result, global) = match self.global_opts() {
            Some(g) => {
                let r = lensbench_optim::global_search(p, &g)?;
                (r.best.clone(), Some(r))
            }
            None => (lensbench_optim::dls(p, &o)?, None),
        };
        let mut problem = p.clone();
        let mut glass = None;
        let cancelled = result.stop == lensbench_optim::Stop::Cancelled;
        if let (Some(gp), false) = (&self.glass, cancelled) {
            let pool = crate::glass_sub::candidate_pool(&p.configs[0], &gp.catalogs, glass_dirs)?;
            let go = crate::glass_sub::GlassSubOpts {
                surfaces: gp.surfaces.clone(),
                glasses: pool,
                candidates: gp.candidates,
                passes: gp.passes,
                local: o,
            };
            let merit_before = result.merit;
            let g = match lensbench_core::par::with_progress(None, || {
                crate::glass_sub::substitute(p, &result.x, &go)
            }) {
                Ok(g) => g,
                // 止めたら置き換える前の結果を返す
                Err(Error::Cancelled) => {
                    result.stop = lensbench_optim::Stop::Cancelled;
                    let x = result.x.clone();
                    return Ok(OptimizeRun {
                        problem,
                        result,
                        x,
                        global,
                        glass: None,
                    });
                }
                Err(e) => return Err(e),
            };
            let mut r = g.result;
            r.x0 = result.x0.clone();
            r.merit0 = result.merit0;
            let materials = gp
                .surfaces
                .iter()
                .map(|&j| (j, g.problem.configs[0].surfaces[j].mat.name()))
                .collect();
            glass = Some(GlassRun {
                changes: g.changes,
                tried: g.tried,
                materials,
                merit_before,
            });
            if !glass.as_ref().is_some_and(|x| x.changes.is_empty()) {
                result = r;
                problem = g.problem;
            }
        }
        let x = p
            .vars
            .iter()
            .map(|v| {
                problem
                    .vars
                    .iter()
                    .position(|u| u == v)
                    .map_or(0.0, |i| result.x[i])
            })
            .collect();
        Ok(OptimizeRun {
            problem,
            result,
            x,
            global,
            glass,
        })
    }

    /// 問題を作る（変数・既定の評価関数・terms の項・ヤコビアン）。返す数は既定の評価関数の項の数
    /// （terms の項はその後ろ）。呼び手は返した後に項を足してから [`Problem::check`] する
    pub fn problem(&self, systems: Vec<System>) -> Result<(Problem, usize)> {
        let mut p = Problem::new(systems);
        for v in &self.vars {
            let d = v.resolve(p.configs.len())?;
            let first = d
                .configs
                .as_ref()
                .and_then(|c| c.first().copied())
                .unwrap_or(0);
            let param = p.configs[first].param(&d.name)?;
            let mut var = Var::new(param).bounds(d.lo, d.hi);
            if let Some(c) = d.configs {
                var = var.in_configs(c);
            }
            p.vars.push(var);
        }
        if let Some(k) = self.merit.default_merit() {
            p.add_default(k, self.rings, self.reference.into());
        }
        let n_default = p.terms.len();
        for (i, t) in self.terms.iter().enumerate() {
            if crate::merit::is_comment(t) {
                continue;
            }
            let t = crate::merit::term_from_json(t, &p.configs)
                .map_err(|e| Error::Invalid(format!("term {}: {e}", i + 1)))?;
            p.terms.push(t);
        }
        p.jac = self.jac.into();
        p.stopped_as_vignetted = self.stopped_as_vignetted;
        Ok((p, n_default))
    }

    /// DLS の設定
    pub fn dls_opts(&self) -> DlsOpts {
        self.dls().opts(self.iters)
    }

    /// 選んだ DLS の設定
    pub fn dls(&self) -> DlsChoice {
        DlsChoice {
            qr: self.qr,
            gtol: self.gtol,
            active_set: self.active_set,
        }
    }
}

/// 状態ごとの近軸量の目標: 数の並び（null は目標なし）、または CLI の書き方 "50,-,80"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Targets {
    List(Vec<Option<f64>>),
    Text(String),
}

impl Targets {
    pub fn values(&self) -> Result<Vec<Option<f64>>> {
        match self {
            Targets::List(v) => Ok(v.clone()),
            Targets::Text(t) => t
                .split(',')
                .map(|x| {
                    let x = x.trim();
                    if x == "-" || x.is_empty() {
                        Ok(None)
                    } else {
                        x.parse()
                            .map(Some)
                            .map_err(|_| Error::Invalid(format!("target {x} can't be read")))
                    }
                })
                .collect(),
        }
    }
}

/// ズーム軌跡の設定。CLI の `zoom`・serve の `zoom`・Python の `zoom`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ZoomParams {
    /// 状態ごとに解くパラメータ（"G2.shift"・"7.t"、範囲は "G2.shift=-5:20" か {"name", "lo", "hi"}）
    #[serde(deserialize_with = "seq")]
    pub vars: Vec<VarSpec>,
    /// 状態ごとの焦点距離の目標（状態の数だけ。null・"-" は目標なし）
    pub efl: Option<Targets>,
    /// 状態ごとの近軸倍率の目標（有限物体）
    pub pmag: Option<Targets>,
    /// 隣り合う状態の間にとる点の数
    pub steps: usize,
    /// 像の評価関数
    pub merit: ImageMerit,
    /// 基準
    pub reference: Reference,
    /// 瞳のガウス求積のリング数
    #[schemars(extend("x-min" = 1))]
    pub rings: usize,
    /// 1 点の反復の上限
    pub iters: usize,
    /// 目標との差の許し（目標に対する比）
    pub tol: f64,
    /// 状態（プロジェクトの状態の名前か .zmx の構成の番号）の並び。省けばすべて（serve）
    #[serde(deserialize_with = "seq")]
    pub configs: Vec<Value>,
    /// 目標の満たし方
    pub method: ZoomMethod,
    /// 一歩を正規方程式でなく QR で解く
    pub qr: bool,
    /// 残差と変数の列の余弦の最大がこれより小さければ止める（0 は判定しない）
    #[schemars(extend("x-min" = 0))]
    #[serde(deserialize_with = "non_negative_de")]
    pub gtol: f64,
    /// 境界: 範囲を出る変数を境界に止めて残りを解き直す（成分ごとに切らずに）
    pub active_set: bool,
    /// 止まった光線（全反射・面に届かない）をけられた光線と同じく除いて評価する（既定は評価できない誤り）
    pub stopped_as_vignetted: bool,
}

impl Default for ZoomParams {
    fn default() -> Self {
        ZoomParams {
            vars: Vec::new(),
            efl: None,
            pmag: None,
            steps: 0,
            merit: ImageMerit::Spot,
            reference: Reference::Centroid,
            rings: 3,
            iters: 50,
            tol: 1e-8,
            configs: Vec::new(),
            method: ZoomMethod::Lagrange,
            qr: false,
            gtol: 0.0,
            active_set: false,
            stopped_as_vignetted: false,
        }
    }
}

impl ZoomParams {
    /// 状態の系 cs（names は誤りの文に出す名前）で解く形にする
    pub fn spec(&self, cs: &[System], names: &[String]) -> Result<ZoomSpec> {
        let n = cs.len();
        if n == 0 {
            return Err(Error::Invalid("no states".into()));
        }
        if self.vars.is_empty() {
            return Err(Error::Invalid(
                "no vars (the list of parameters to solve)".into(),
            ));
        }
        let (q, t) = match (&self.efl, &self.pmag) {
            (Some(_), Some(_)) => return Err(Error::Invalid("only one of efl and pmag".into())),
            (Some(t), None) => (ParaxQ::Efl, Some(t)),
            (None, Some(t)) => (ParaxQ::Pmag, Some(t)),
            (None, None) => (ParaxQ::Efl, None),
        };
        let targets = match t {
            None => vec![None; n],
            Some(t) => t.values()?,
        };
        if targets.len() != n {
            return Err(Error::Invalid(format!(
                "{} targets ({n} states: {})",
                targets.len(),
                names.join(", ")
            )));
        }
        let mut moving: Vec<Param> = Vec::new();
        let mut bounds = Vec::new();
        for v in &self.vars {
            let d = v.resolve(1)?;
            moving.push(cs[0].param(&d.name)?);
            bounds.push((d.lo, d.hi));
        }
        let mut sp = ZoomSpec::new(moving, targets);
        sp.bounds = bounds;
        sp.q = q;
        sp.steps = self.steps;
        sp.rings = self.rings;
        sp.target_tol = self.tol;
        sp.merit = self.merit.into();
        sp.reference = self.reference.into();
        sp.method = self.method.into();
        sp.stopped_as_vignetted = self.stopped_as_vignetted;
        sp.opts = DlsChoice {
            qr: self.qr,
            gtol: self.gtol,
            active_set: self.active_set,
        }
        .opts(self.iters);
        Ok(sp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn var_forms() {
        let v: VarSpec = serde_json::from_value(json!("7.t=0.5:")).unwrap();
        let d = v.resolve(1).unwrap();
        assert_eq!((d.name.as_str(), d.lo, d.hi), ("7.t", 0.5, f64::INFINITY));
        let v: VarSpec =
            serde_json::from_value(json!({"name": "7.t", "hi": 3.0, "configs": [2]})).unwrap();
        let d = v.resolve(2).unwrap();
        assert_eq!(
            (d.lo, d.hi, d.configs),
            (f64::NEG_INFINITY, 3.0, Some(vec![1]))
        );
        assert!(v.resolve(1).is_err());
        assert!(VarSpec::from("7.t=1").resolve(1).is_err());
    }

    #[test]
    fn optimize_params_read() {
        let p: OptimizeParams =
            serde_json::from_value(json!({"vars": ["3.t"], "merit": null})).unwrap();
        assert_eq!(p.merit, Merit::None);
        assert_eq!(p.rings, OptimizeParams::default().rings);
        let p: OptimizeParams = serde_json::from_value(json!({})).unwrap();
        assert_eq!(p.merit, Merit::Spot);
        assert!(serde_json::from_value::<OptimizeParams>(json!({"ring": 3})).is_err());
        assert!(serde_json::from_value::<OptimizeParams>(json!({"jac": "x"})).is_err());
        assert_eq!("Wavefront".parse::<Merit>().unwrap(), Merit::Wavefront);
        // 別名 wfe は受けない
        assert!("wfe".parse::<Merit>().is_err());
        assert!("x"
            .parse::<Jac>()
            .unwrap_err()
            .contains("auto, fd, forward"));
    }

    /// 止まった光線の扱いは JSON（serve）から問題とズームの設定まで届く
    #[test]
    fn stopped_as_vignetted_reaches_the_problem() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let s = lensbench_formats::zmx::load_with(
            &root.join("tests/refs/A0/system.zmx"),
            &lensbench_formats::zmx::LoadOptions {
                cfg: 1,
                catalog_dirs: vec![root.join("tests/glass")],
                ..Default::default()
            },
        )
        .unwrap();
        let p: OptimizeParams = read(&json!({"merit": "none"})).unwrap();
        assert!(!p.problem(vec![s.clone()]).unwrap().0.stopped_as_vignetted);
        let p: OptimizeParams =
            read(&json!({"merit": "none", "stopped_as_vignetted": true})).unwrap();
        assert!(p.problem(vec![s]).unwrap().0.stopped_as_vignetted);
        let z: ZoomParams = read(&json!({"stopped_as_vignetted": true})).unwrap();
        assert!(z.stopped_as_vignetted && !ZoomParams::default().stopped_as_vignetted);
    }

    #[test]
    fn targets_text_and_list() {
        assert_eq!(
            Targets::Text("50, -,80".into()).values().unwrap(),
            vec![Some(50.0), None, Some(80.0)]
        );
        let z: ZoomParams =
            serde_json::from_value(json!({"vars": ["G2.shift"], "efl": [50.0, null]})).unwrap();
        assert_eq!(z.efl.unwrap().values().unwrap(), vec![Some(50.0), None]);
        // null は空、項 1 つのオブジェクトは 1 つの並び
        let o: OptimizeParams = serde_json::from_value(
            json!({"vars": null, "configs": null, "terms": {"op": "efl", "target": 50.0}}),
        )
        .unwrap();
        assert!(o.vars.is_empty() && o.configs.is_empty() && o.terms.len() == 1);
        // 誤りにはどの鍵かを付ける
        let e = read::<ZoomParams>(&json!({"vars": ["7.t"], "qr": 1})).unwrap_err();
        assert!(e.message().unwrap_or_default().starts_with("qr: "), "{e}");
        // gtol の負の数は読みの誤り（serve の要求の誤り）
        let e = read::<OptimizeParams>(&json!({"gtol": -1.0})).unwrap_err();
        assert!(
            e.message()
                .unwrap_or_default()
                .starts_with("gtol: must be 0 or more"),
            "{e}"
        );
        assert!(read::<ZoomParams>(&json!({"gtol": -0.5})).is_err());
        assert!(parse_non_negative("-1").is_err() && parse_non_negative("x").is_err());
        assert_eq!(parse_non_negative("1e-6").unwrap(), 1e-6);
        let e = read::<ZoomParams>(&json!({"method": "exact"})).unwrap_err();
        assert!(
            e.message()
                .unwrap_or_default()
                .starts_with("method: exact (one of penalty, lagrange)"),
            "{e}"
        );
    }
}
