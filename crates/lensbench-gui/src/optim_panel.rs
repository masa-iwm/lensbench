//! 最適化のパネル: 変数・評価関数の項・設定を並べ、serve の `optimize` で減衰最小二乗を回す。
//!
//! 設定は serve の `optimize` の params と同じ JSON（[`OptPanel::params`]）で、そのままファイルに保存・読込できる。
//! 項の種類とキーは `lensbench_design::merit`（CLI の `--term`）と同じ。走っている間は
//! [`lensbench_engine::optimize_progress`] で評価関数の履歴を読み、止めるのは
//! [`lensbench_engine::optimize_cancel`]。

use egui::{RichText, Ui};
use egui_plot::{Line, Plot};
use serde_json::{json, Map, Value};

use crate::i18n::{n_, tr, trf};
use crate::json_view::fmt_num;
use crate::param_pick::{self as pp, Set};
use crate::theme;

mod kinds;
mod sheets;
use kinds::{defaults, type_hint};
use lensbench_engine::session::views::{OptimizeView, TermValue};

/// 項のキーの値の書き方
#[derive(Clone, Copy, PartialEq)]
enum KeyKind {
    Num,
    Int,
    /// 視野: 番号か「hx, hy」
    Field,
    /// 「x, y」
    Pair,
    Choice(&'static [&'static str]),
    /// true・false
    Bool,
    /// JSON の値の並び（`[` `]` を除いて、カンマで区切る。calc の引数）
    JsonList,
    /// ZPL マクロ: 番号（ZPLnn.ZPL）か名前
    Macro,
    /// 文字列（ファイルのパス）
    Text,
    /// フォルダの並び（`;` で区切る）
    Dirs,
    /// JSON のオブジェクト（プラグインの設定・入力）
    Json,
}

const PARAXIAL_Q: &[&str] = &[
    "efl", "pmag", "enpp", "epd", "expp", "expd", "wfno", "pwfn", "isfn", "pimh", "totr", "obsn",
    "isna", "amag", "linv", "petc", "petz", "wlen",
];
const RAY_Q: &[&str] = &[
    "x", "y", "z", "l", "m", "n", "trax", "tray", "trar", "anax", "anay", "anar", "dxdx", "dxdy",
    "dydx", "dydy", "fcgt", "fcgs", "opth", "gx", "gy", "gz", "gl", "gm", "gn", "r", "ang", "tanx",
    "tany", "nl", "nm", "nn", "aoi", "aoe", "cosi", "cose",
];
const SURF_Q: &[&str] = &[
    "curv",
    "conic",
    "sag",
    "sagx",
    "sagy",
    "thick",
    "diam",
    "mech_diam",
    "gx",
    "gy",
    "gz",
    "power",
    "ynip",
    "index",
    "ga",
    "gb",
    "gc",
    "nd",
    "vd",
    "dpgf",
    "cost",
    "tce",
    "total",
    "total_clear",
    "surf_tce",
    "param",
    "semi",
];
const CARD_Q: &[&str] = &[
    "efl",
    "f_object",
    "f_image",
    "focal_object",
    "focal_image",
    "principal_object",
    "principal_image",
    "anti_principal_object",
    "anti_principal_image",
    "nodal_object",
    "nodal_image",
    "anti_nodal_object",
    "anti_nodal_image",
];
const BFS_Q: &[&str] = &[
    "curv",
    "radius",
    "vertex",
    "depth",
    "volume",
    "slope",
    "depth_rms",
    "slope_rms",
];
const REFERENCE: &[&str] = &["centroid", "chief"];
/// スポット・波面の瞳の求積（arms は opticstudio、n は grid と使う）
const QUADRATURE: &[&str] = &["lensbench", "opticstudio", "grid"];

/// 項の種類
pub(crate) const TERM_TYPES: &[&str] = &[
    "spot",
    "wavefront",
    "mtf",
    "energy",
    "strehl",
    "illum",
    "param",
    "paraxial",
    "ray",
    "centroid",
    "opd",
    "zernike",
    "coupling",
    "thickness",
    "zplm",
    "plugin",
    "calc",
    "spread",
    "tsag",
    "bfs",
    "surface",
    "cardinal",
    "fnum",
    "seidel",
    "distortion",
    "longitudinal",
    "lateral_color",
    "semi",
    "volume",
    "comment",
];

/// 種類ごとのキー（weight・config は共通）
fn keys(ty: &str) -> &'static [(&'static str, KeyKind)] {
    use KeyKind::*;
    match ty {
        "spot" => &[
            ("field", Field),
            ("wave", Int),
            ("rings", Int),
            ("reference", Choice(REFERENCE)),
            ("quadrature", Choice(QUADRATURE)),
            ("arms", Int),
            ("n", Int),
            ("axis", Choice(&["xy", "x", "y"])),
        ],
        "wavefront" => &[
            ("field", Field),
            ("wave", Int),
            ("rings", Int),
            ("reference", Choice(REFERENCE)),
            ("quadrature", Choice(QUADRATURE)),
            ("arms", Int),
            ("n", Int),
        ],
        "mtf" => &[
            ("field", Field),
            ("wave", Int),
            ("freq", Num),
            ("dir", Choice(&["both", "tan", "sag", "avg"])),
            (
                "kind",
                Choice(&["diffraction", "square", "geometric", "huygens"]),
            ),
            ("diff_limit", Bool),
            ("n", Int),
            ("target", Num),
        ],
        "energy" => &[
            ("field", Field),
            ("wave", Int),
            ("method", Choice(&["geometric", "fft", "huygens"])),
            ("airy", Bool),
            ("shape", Choice(&["circle", "x", "y", "square"])),
            ("center", Choice(&["chief", "centroid", "vertex", "middle"])),
            ("size_at", Num),
            ("fraction_at", Num),
            ("n", Int),
            ("target", Num),
            ("min", Num),
            ("max", Num),
        ],
        "strehl" => &[
            ("field", Field),
            ("wave", Int),
            ("n", Int),
            ("target", Num),
            ("min", Num),
            ("max", Num),
        ],
        "param" => &[("name", Text), ("target", Num), ("min", Num), ("max", Num)],
        "illum" => &[
            ("field", Field),
            ("wave", Int),
            ("q", Choice(&["relative", "fno"])),
            ("n", Int),
            ("target", Num),
            ("min", Num),
            ("max", Num),
        ],
        "paraxial" => &[("q", Choice(PARAXIAL_Q)), ("wave", Int), ("target", Num)],
        "ray" => &[
            ("field", Field),
            ("wave", Int),
            ("px", Num),
            ("py", Num),
            ("surf", Int),
            ("to", Int),
            ("paraxial", Bool),
            ("q", Choice(RAY_Q)),
            ("target", Num),
        ],
        "centroid" => &[
            ("field", Field),
            ("wave", Int),
            ("rings", Int),
            ("q", Choice(&["x", "y"])),
            ("target", Num),
        ],
        "opd" => &[
            ("field", Field),
            ("wave", Int),
            ("px", Num),
            ("py", Num),
            ("target", Num),
        ],
        "zernike" => &[
            ("field", Field),
            ("wave", Int),
            ("term", Int),
            ("kind", Choice(&["fringe", "standard"])),
            ("fit", Int),
            ("n", Int),
            ("target", Num),
        ],
        "thickness" => &[
            ("surf", Int),
            ("to", Int),
            ("target", Num),
            ("min", Num),
            ("max", Num),
            ("edge", Bool),
            ("zone", Num),
            ("each", Choice(&["all", "air", "glass"])),
            ("axis", Choice(&["+y", "+x", "-y", "-x"])),
            ("clear", Bool),
        ],
        "zplm" => &[
            ("macro", Macro),
            ("data", Int),
            ("hx", Num),
            ("hy", Num),
            ("px", Num),
            ("py", Num),
            ("target", Num),
            ("file", Text),
            ("macro_dirs", Dirs),
            ("default_dirs", Bool),
        ],
        "calc" => &[
            (
                "op",
                Choice(&[
                    "sum", "diff", "prod", "div", "abs", "sqrt", "reci", "max", "min", "qsum",
                    "sin", "cos", "tan", "asin", "acos", "atan",
                ]),
            ),
            ("args", JsonList),
            ("target", Num),
        ],
        "spread" => &[("term", Json), ("target", Num), ("min", Num), ("max", Num)],
        "tsag" => &[
            ("surf", Int),
            ("point", JsonList),
            ("tilt", JsonList),
            ("flat", Bool),
            ("target", Num),
            ("min", Num),
            ("max", Num),
        ],
        "bfs" => &[
            ("surf", Int),
            ("q", Choice(BFS_Q)),
            ("rmin", Num),
            ("rmax", Num),
            ("target", Num),
            ("min", Num),
            ("max", Num),
        ],
        "surface" => &[
            ("surf", Int),
            ("q", Choice(SURF_Q)),
            ("to", Int),
            ("param", Int),
            ("x", Num),
            ("y", Num),
            ("wave", Int),
            ("target", Num),
            ("min", Num),
            ("max", Num),
        ],
        "volume" => &[
            ("surf", Int),
            ("to", Int),
            ("mass", Bool),
            ("clear", Bool),
            ("cylinder", Bool),
            ("target", Num),
            ("min", Num),
            ("max", Num),
        ],
        "seidel" => &[
            ("q", Choice(&["spha", "coma", "asti", "fcur", "dist"])),
            ("surf", Int),
            ("wave", Int),
            ("target", Num),
        ],
        "distortion" => &[
            ("field", Field),
            ("reference", Field),
            ("wave", Int),
            ("absolute", Bool),
            ("peak", Bool),
            ("ftheta", Bool),
            ("coef", Choice(&["a", "b", "c", "d"])),
            ("abcd", JsonList),
            ("component", Choice(&["radial", "x", "y"])),
            ("target", Num),
        ],
        "longitudinal" => &[
            ("wave", Int),
            ("wave2", Int),
            ("zone", Num),
            ("target", Num),
        ],
        "lateral_color" => &[("wave", Int), ("wave2", Int), ("target", Num)],
        "comment" => &[("text", Text)],
        "semi" => &[("surf", Int), ("to", Int), ("min", Num), ("max", Num)],
        "fnum" => &[
            ("field", Field),
            ("wave", Int),
            ("dir", Choice(&["tan", "sag"])),
            ("target", Num),
        ],
        "cardinal" => &[
            ("q", Choice(CARD_Q)),
            ("from", Int),
            ("to", Int),
            ("wave", Int),
            ("target", Num),
        ],
        "plugin" => &[
            ("name", Text),
            ("field", Field),
            ("output", Text),
            ("index", Int),
            ("target", Num),
            ("args", Json),
            ("inputs", Json),
        ],
        "coupling" => &[
            ("wave", Int),
            ("na", Num),
            ("source", Pair),
            ("receiver", Pair),
            ("grid", Int),
            ("na_source", Num),
            ("na_receiver", Num),
            ("na_max", Num),
            ("clip_stop", Bool),
            ("target", Num),
        ],
        _ => &[],
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct VarRow {
    pub name: String,
    pub lo: String,
    pub hi: String,
    /// 効く構成（「1, 2」。空はすべて）
    pub configs: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TermRow {
    pub ty: String,
    /// キーの値（空は省く）
    pub vals: Vec<(String, String)>,
    pub weight: String,
    pub config: String,
}

impl TermRow {
    pub(crate) fn new(ty: &str) -> TermRow {
        let mut t = TermRow {
            ty: ty.into(),
            ..Default::default()
        };
        for (k, v) in defaults(ty) {
            t.set(k, v);
        }
        t
    }
    fn get(&self, k: &str) -> &str {
        self.vals
            .iter()
            .find(|(a, _)| a == k)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }
    fn set(&mut self, k: &str, v: &str) {
        match self.vals.iter_mut().find(|(a, _)| a == k) {
            Some(x) => x.1 = v.into(),
            None => self.vals.push((k.into(), v.into())),
        }
    }
}

/// パネルの操作（app が serve に送る）
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Run(Value),
    Stop,
    /// 結果の値を系（組合せ）に当てる
    Apply,
    /// 当てる前の値に戻す
    Undo,
    Save(Value),
    Load,
}

#[derive(Default)]
pub(crate) struct OptPanel {
    pub vars: Vec<VarRow>,
    pub terms: Vec<TermRow>,
    /// 既定の評価関数: spot・wavefront・none
    pub merit: String,
    /// 走らせたときの評価関数の値の倍率と単位（[`merit_unit`]）
    pub run_unit: (f64, &'static str),
    pub rings: usize,
    pub reference: String,
    pub iters: usize,
    pub jac: String,
    /// DLS の一歩を QR で解く・勾配で止める値（0 は判定しない）・境界を外れる変数を止めて解き直す・止まった光線を
    /// けられた光線と同じく除く（CLI の --qr・--gtol・--active-set・--stopped-as-vignetted）
    pub qr: bool,
    pub gtol: f64,
    pub active_set: bool,
    pub stopped_as_vignetted: bool,
    /// 構成（.zmx の構成番号か状態の名前、カンマ区切り。空は開いている系だけ）
    pub configs: String,
    pub new_var: String,
    pub new_term: String,
    pub running: bool,
    /// serve の optimize の結果
    pub result: Option<std::sync::Arc<OptimizeView>>,
    pub error: Option<String>,
    /// 系に当てる前の値（元に戻す）
    pub undo: Option<Vec<(String, f64)>>,
    /// 最適化を始めたときの系の世代（[`crate::app`] の sys_gen。変わっていれば結果を当てない）
    pub gen: u64,
    /// 変数・項の表の選択と書き換え中の文字
    pub vars_sheet: crate::widgets::sheet::SheetState,
    pub terms_sheet: TermsState,
    /// 走らせたときの項（結果の値をどの行に出すか。行が変わったら出さない）
    pub run_terms: Vec<TermRow>,
    /// 「評価」を押した（変数なしで回す。ui の最後で Run にする）
    pub eval_asked: bool,
}

impl OptPanel {
    pub(crate) fn new() -> OptPanel {
        // 既定値は設定の型（CLI・serve・Python と同じ）
        let d = lensbench_design::dls_spec::OptimizeParams::default();
        OptPanel {
            merit: d.merit.to_string(),
            rings: d.rings,
            reference: d.reference.to_string(),
            iters: d.iters,
            jac: d.jac.to_string(),
            qr: d.qr,
            gtol: d.gtol,
            active_set: d.active_set,
            stopped_as_vignetted: d.stopped_as_vignetted,
            new_term: "paraxial".into(),
            ..Default::default()
        }
    }

    /// 項の表の値の列: 走らせたときと同じ行なら結果の（後の）値と寄与
    pub(crate) fn term_vals(&self) -> Vec<Option<(f64, f64)>> {
        let Some(r) = &self.result else {
            return vec![None; self.terms.len()];
        };
        let extra: Vec<&TermValue> = r.terms.iter().filter(|t| !t.default).collect();
        // 結果の項はコメントの行を飛ばした順
        let mut k = 0;
        self.terms
            .iter()
            .enumerate()
            .map(|(i, t)| {
                if t.ty == "comment" {
                    return None;
                }
                k += 1;
                (self.run_terms.get(i) == Some(t))
                    .then(|| extra.get(k - 1).map(|v| (v.value, v.contrib)))
                    .flatten()
            })
            .collect()
    }

    /// serve の optimize の params（保存するファイルと同じ）
    pub(crate) fn params(&self) -> Result<Value, String> {
        let num = |s: &str, what: &str| -> Result<Option<f64>, String> {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            t.parse::<f64>()
                .map(Some)
                .map_err(|_| trf("Value of {} is not a number: {}", &[&what, &t]))
        };
        let mut vars = Vec::new();
        for v in &self.vars {
            let name = v.name.trim();
            if name.is_empty() {
                continue;
            }
            let mut o = Map::new();
            o.insert("name".into(), json!(name));
            if let Some(x) = num(&v.lo, name)? {
                o.insert("lo".into(), json!(x));
            }
            if let Some(x) = num(&v.hi, name)? {
                o.insert("hi".into(), json!(x));
            }
            let cs = int_list(&v.configs).map_err(|e| format!("{name}: {e}"))?;
            if !cs.is_empty() {
                o.insert("configs".into(), json!(cs));
            }
            vars.push(Value::Object(o));
        }
        let mut terms = Vec::new();
        for (i, t) in self.terms.iter().enumerate() {
            terms.push(term_json(t).map_err(|e| trf("Term {}: {}", &[&(i + 1), &e]))?);
        }
        let mut p = json!({
            "vars": vars,
            "merit": self.merit,
            "rings": self.rings,
            "reference": self.reference,
            "terms": terms,
            "iters": self.iters,
            "jac": self.jac,
        });
        let cfgs: Vec<Value> = self
            .configs
            .split(',')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(|x| match x.parse::<u64>() {
                Ok(n) => json!(n),
                Err(_) => json!(x),
            })
            .collect();
        if !cfgs.is_empty() {
            p["configs"] = json!(cfgs);
        }
        // DLS の設定は既定と違うときだけ書く（保存するファイルを短く）
        let d = lensbench_design::dls_spec::OptimizeParams::default();
        for (k, v, dv) in [
            ("qr", self.qr, d.qr),
            ("active_set", self.active_set, d.active_set),
            (
                "stopped_as_vignetted",
                self.stopped_as_vignetted,
                d.stopped_as_vignetted,
            ),
        ] {
            if v != dv {
                p[k] = json!(v);
            }
        }
        if self.gtol != d.gtol {
            p["gtol"] = json!(self.gtol);
        }
        Ok(p)
    }

    /// 保存したファイル（params と同じ形）から
    pub(crate) fn set_params(&mut self, p: &Value) -> Result<(), String> {
        let mut n = OptPanel::new();
        if let Some(m) = p["merit"].as_str() {
            n.merit = m.into();
        } else if p.get("merit").is_some_and(Value::is_null) {
            n.merit = "none".into();
        }
        if let Some(x) = p["rings"].as_u64() {
            n.rings = x as usize;
        }
        if let Some(x) = p["reference"].as_str() {
            n.reference = x.into();
        }
        if let Some(x) = p["iters"].as_u64() {
            n.iters = x as usize;
        }
        if let Some(x) = p["jac"].as_str() {
            n.jac = x.into();
        }
        for (k, v) in [
            ("qr", &mut n.qr),
            ("active_set", &mut n.active_set),
            ("stopped_as_vignetted", &mut n.stopped_as_vignetted),
        ] {
            if let Some(x) = p[k].as_bool() {
                *v = x;
            }
        }
        if let Some(x) = p["gtol"].as_f64() {
            n.gtol = x;
        }
        if let Some(a) = p["configs"].as_array() {
            n.configs = a.iter().map(text_of).collect::<Vec<_>>().join(", ");
        }
        for v in p["vars"].as_array().into_iter().flatten() {
            n.vars.push(match v {
                Value::String(s) => VarRow {
                    name: s.clone(),
                    ..Default::default()
                },
                _ => VarRow {
                    name: v["name"].as_str().unwrap_or_default().into(),
                    lo: v.get("lo").map(text_of).unwrap_or_default(),
                    hi: v.get("hi").map(text_of).unwrap_or_default(),
                    configs: v["configs"]
                        .as_array()
                        .map(|a| a.iter().map(text_of).collect::<Vec<_>>().join(", "))
                        .unwrap_or_default(),
                },
            });
        }
        n.terms = term_rows(&p["terms"])?;
        n.new_var = std::mem::take(&mut self.new_var);
        *self = n;
        Ok(())
    }
}

/// 項の JSON の並びから欄の行へ
pub(crate) fn term_rows(v: &Value) -> Result<Vec<TermRow>, String> {
    let mut out = Vec::new();
    {
        for (i, t) in v.as_array().into_iter().flatten().enumerate() {
            let o = t
                .as_object()
                .ok_or_else(|| trf("Term {} is not an object", &[&(i + 1)]))?;
            let ty = o.get("type").and_then(Value::as_str).unwrap_or_default();
            if !TERM_TYPES.contains(&ty) {
                return Err(trf("Term {} has type {}", &[&(i + 1), &ty]));
            }
            let mut r = TermRow {
                ty: ty.into(),
                ..Default::default()
            };
            for (k, v) in o {
                match k.as_str() {
                    "type" => {}
                    "weight" => r.weight = text_of(v),
                    "config" => r.config = text_of(v),
                    // フォルダの並びは ; で区切る（パスに , がありうる）
                    "macro_dirs" => r.vals.push((
                        k.clone(),
                        v.as_array()
                            .map(|a| a.iter().map(text_of).collect::<Vec<_>>().join("; "))
                            .unwrap_or_else(|| text_of(v)),
                    )),
                    // 欄の無い鍵は、保存で読んだ形に戻る文字に（数に見える文字列は引用符つき）
                    _ if !keys(ty).iter().any(|(a, _)| a == k) => {
                        r.vals.push((k.clone(), loose_text(v)))
                    }
                    _ => r.vals.push((k.clone(), text_of(v))),
                }
            }
            out.push(r);
        }
    }
    Ok(out)
}

/// 欄の行から項の JSON の並びへ
pub(crate) fn terms_json(terms: &[TermRow]) -> Result<Vec<Value>, String> {
    terms
        .iter()
        .enumerate()
        .map(|(i, t)| term_json(t).map_err(|e| trf("Term {}: {}", &[&(i + 1), &e])))
        .collect()
}

/// JSON の値を欄の文字列に
pub(crate) fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(x) => x.to_string(),
        Value::Array(a) => a.iter().map(text_of).collect::<Vec<_>>().join(", "),
        Value::Null => String::new(),
        x => x.to_string(),
    }
}

fn int_list(s: &str) -> Result<Vec<u64>, String> {
    s.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(|x| {
            x.parse::<u64>()
                .map_err(|_| trf("Configuration number {} is not an integer", &[&x]))
        })
        .collect()
}

fn pair(t: &str) -> Option<Value> {
    let v: Vec<f64> = t
        .split(',')
        .map(|x| x.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .ok()?;
    (v.len() == 2).then(|| json!(v))
}

/// 欄の無い鍵の文字列を JSON の値に（[`loose_text`] の逆）。引用符で囲んだ文字は文字列
fn loose(v: &str) -> Value {
    if let Ok(x) = serde_json::from_str::<Value>(v) {
        return x;
    }
    match serde_json::from_str::<Value>(&format!("[{v}]")) {
        Ok(x) if x.as_array().is_some_and(|a| a.len() > 1) => x,
        _ => json!(v),
    }
}

/// 欄の無い鍵の JSON の値を欄の文字に（[`loose`] で同じ値に戻る）。数・true などに見える文字列は引用符で囲む
fn loose_text(v: &Value) -> String {
    match v {
        Value::String(s) if loose(s) != *v => Value::String(s.clone()).to_string(),
        _ => text_of(v),
    }
}

/// 項の行を JSON に（`lensbench_design::merit` の形）
fn term_json(t: &TermRow) -> Result<Value, String> {
    let mut o = Map::new();
    o.insert("type".into(), json!(t.ty));
    let ks = keys(&t.ty);
    for (k, v) in &t.vals {
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        let bad = || trf("Value of {}: {}", &[k, &v]);
        // 欄の無い鍵は、読んだときの JSON の形に戻す（数・true/false・並び・オブジェクト、ほかは文字列）
        let Some(kind) = ks.iter().find(|(a, _)| a == k).map(|x| x.1) else {
            o.insert(k.clone(), loose(v));
            continue;
        };
        let x = match kind {
            KeyKind::Num => json!(v.parse::<f64>().map_err(|_| bad())?),
            KeyKind::Int => json!(v.parse::<u64>().map_err(|_| bad())?),
            KeyKind::Field => match v.parse::<u64>() {
                Ok(i) => json!(i),
                Err(_) => pair(v).ok_or_else(bad)?,
            },
            KeyKind::Pair => pair(v).ok_or_else(bad)?,
            KeyKind::Choice(_) | KeyKind::Text => json!(v),
            KeyKind::Bool => json!(v == "true"),
            KeyKind::JsonList => {
                serde_json::from_str::<Value>(&format!("[{v}]")).map_err(|_| bad())?
            }
            KeyKind::Json => {
                let x: Value = serde_json::from_str(v).map_err(|_| bad())?;
                if !x.is_object() {
                    return Err(bad());
                }
                x
            }
            KeyKind::Macro => match v.parse::<u64>() {
                Ok(i) => json!(i),
                Err(_) => json!(v),
            },
            KeyKind::Dirs => json!(v
                .split(';')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()),
        };
        o.insert(k.clone(), x);
    }
    if !t.weight.trim().is_empty() {
        let w = t.weight.trim();
        o.insert(
            "weight".into(),
            json!(w
                .parse::<f64>()
                .map_err(|_| trf("Weight {} is not a number", &[&w]))?),
        );
    }
    if !t.config.trim().is_empty() {
        let c = t.config.trim();
        o.insert(
            "config".into(),
            json!(c
                .parse::<u64>()
                .map_err(|_| trf("Configuration number {} is not an integer", &[&c]))?),
        );
    }
    Ok(Value::Object(o))
}

/// 変数の候補（面の表の行・玉と群）
pub(crate) struct Cands<'a> {
    /// 玉・群の名前
    pub units: Vec<String>,
    /// 組合せの系（面の値は t・r・k だけ組合せに当てられる）
    pub combo: bool,
    /// 名前の欄の補完の候補
    pub sys: std::rc::Rc<crate::param_pick::Sys>,
    /// 面の表の行（変数の今の値を出す）
    pub rows: &'a [crate::lens_data::Row],
}

/// 変数の今の値（面の値 `<面>.<t|r|c|k>` だけ。玉・群・ほかは None）
pub(crate) fn var_value(rows: &[crate::lens_data::Row], name: &str) -> Option<f64> {
    let (j, f) = name.trim().split_once('.')?;
    let r = rows.get(j.parse::<usize>().ok()?)?;
    Some(match f {
        "t" => r.t,
        "r" => r.r,
        // 平面は r = ∞（0 で持つ行も平面）
        "c" if r.r == 0.0 => 0.0,
        "c" => 1.0 / r.r,
        "k" => r.k,
        _ => return None,
    })
    .filter(|x| x.is_finite() || f == "r")
}

pub(crate) fn small_edit(ui: &mut Ui, s: &mut String, w: f32, hint: &str) -> egui::Response {
    let r = ui.add(
        egui::TextEdit::singleline(s)
            .desired_width(w)
            .hint_text(hint),
    );
    crate::widgets::name_field(&r, hint, s);
    r
}

/// パネルを描く。`progress` は走っている間の評価関数の履歴
pub(crate) fn ui(
    ui: &mut Ui,
    p: &mut OptPanel,
    c: &Cands,
    progress: Option<&[f64]>,
) -> Vec<Action> {
    let mut acts = Vec::new();
    let weak = theme::weak_text(ui.visuals());
    egui::ScrollArea::vertical()
        .id_salt("optim_scroll")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if p.running {
                    if ui.button(tr("Stop")).clicked() {
                        acts.push(Action::Stop);
                    }
                    ui.spinner();
                } else {
                    let no_var = p.vars.iter().all(|v| v.name.trim().is_empty());
                    let check = if no_var { Err(String::new()) } else { p.params().map(|_| ()) };
                    let run = ui
                        .add_enabled(
                            check.is_ok(),
                            egui::Button::new(RichText::new(tr("Optimize")).strong()),
                        )
                        .on_disabled_hover_text(if no_var {
                            tr("Add at least one variable (a name like 4.t in the Variables table below, or pick with […])").to_string()
                        } else {
                            check.clone().err().unwrap_or_default()
                        });
                    // 押せない理由を横にも出す
                    if no_var {
                        ui.label(
                            RichText::new(tr("Add at least one variable to enable"))
                                .small()
                                .color(ui.visuals().warn_fg_color),
                        );
                    } else if let Err(e) = &check {
                        ui.label(
                            RichText::new(format!("⚠ {e}"))
                                .small()
                                .color(ui.visuals().error_fg_color),
                        );
                    }
                    if run.clicked() {
                        match p.params() {
                            Ok(v) => {
                                p.error = None;
                                p.run_unit = merit_unit(p);
                                p.run_terms = p.terms.clone();
                                acts.push(Action::Run(v));
                            }
                            Err(e) => p.error = Some(e),
                        }
                    }
                }
                ui.add_enabled_ui(!p.running, |ui| {
                    if ui.button(tr("Save settings…")).clicked() {
                        match p.params() {
                            Ok(v) => acts.push(Action::Save(v)),
                            Err(e) => p.error = Some(e),
                        }
                    }
                    if ui.button(tr("Load settings…")).clicked() {
                        acts.push(Action::Load);
                    }
                });
            });
            if let Some(e) = &p.error {
                ui.colored_label(ui.visuals().error_fg_color, lensbench_core::i18n::tr(e));
            }
            if let Some(h) = progress {
                ui.label(trf(
                    "Iteration {}: merit {}",
                    &[
                        &(h.len().saturating_sub(1)),
                        &fmt_g(*h.last().unwrap_or(&f64::NAN)),
                    ],
                ));
                history_plot(ui, h);
            } else if let Some(r) = p.result.clone() {
                egui::CollapsingHeader::new(RichText::new(tr("Result")).strong())
                    .id_salt("opt_result")
                    .default_open(true)
                    .show(ui, |ui| result_ui(ui, p, &r, &mut acts, weak));
            }
            ui.separator();
            ui.add_enabled_ui(!p.running, |ui| {
                section(ui, "opt_vars_sec", tr("Variables"), |ui| {
                    vars_ui(ui, p, c, weak)
                });
                section(ui, "opt_merit_sec", tr("Merit function"), |ui| {
                    merit_ui(ui, p, true, weak)
                });
                section(ui, "opt_settings_sec", tr("Settings"), |ui| {
                    settings_ui(ui, p, weak)
                });
            });
        });
    // 「評価」: 変数なしで回す（系は変えず、評価関数と項の値だけ求める）
    if std::mem::take(&mut p.eval_asked) && !p.running {
        match p.params() {
            Ok(mut v) => {
                v["vars"] = json!([]);
                p.error = None;
                p.run_unit = merit_unit(p);
                p.run_terms = p.terms.clone();
                acts.push(Action::Run(v));
            }
            Err(e) => p.error = Some(e),
        }
    }
    acts
}

pub(crate) fn section(ui: &mut Ui, id: &str, title: &str, f: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .id_salt(id)
        .default_open(true)
        .show(ui, f);
}

fn fmt_g(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return format!("{x}");
    }
    format!("{x:.6e}")
}

pub(crate) fn vars_ui(ui: &mut Ui, p: &mut OptPanel, c: &Cands, weak: egui::Color32) {
    let set = Set::Var { combo: c.combo };
    if !p.vars.is_empty() {
        let mut vs = sheets::VarSheet {
            vars: &mut p.vars,
            rows: c.rows,
            names: pp::names(&c.sys, set),
            multi: !p.configs.trim().is_empty(),
        };
        crate::widgets::sheet::show(ui, "opt_vars", &mut p.vars_sheet, &mut vs);
    }
    ui.horizontal_wrapped(|ui| {
        let (_, mut enter) = pp::edit(
            ui,
            &mut p.new_var,
            110.0,
            tr("7.t, lens2.tx, …"),
            &c.sys,
            set,
        );
        if let Some(n) = pp::menu_button(
            ui,
            &c.sys,
            set,
            tr("Pick from surfaces, lenses and groups and add"),
        ) {
            p.new_var = n;
            enter = true;
        }
        let name = p.new_var.trim().to_string();
        if (ui.button(tr("Add")).clicked() || enter) && !name.is_empty() {
            if !p.vars.iter().any(|v| v.name == name) {
                p.vars.push(VarRow {
                    name,
                    ..Default::default()
                });
            }
            p.new_var.clear();
        }
    });
    if c.combo {
        ui.label(
            RichText::new(tr(
                "Combination: surface thickness (gaps), radius, conic and shape coefficients (aspheric terms etc.) can be applied back to the combination.",
            ))
            .small()
            .color(weak),
        );
    }
}

/// 評価関数の欄。vals は項の表に値・寄与の列と「評価」を出す（最適化のパネル）
pub(crate) fn merit_ui(ui: &mut Ui, p: &mut OptPanel, vals: bool, weak: egui::Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Default"));
        for (k, l) in [
            ("spot", tr("RMS spot")),
            ("wavefront", tr("RMS wavefront")),
            ("none", tr("Off")),
        ] {
            ui.selectable_value(&mut p.merit, k.to_string(), l);
        }
    });
    if p.merit != "none" {
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Rings"));
            ui.add(egui::DragValue::new(&mut p.rings).range(1..=20));
            ui.label(tr("Reference"));
            for (k, l) in [("centroid", tr("Centroid")), ("chief", tr("Chief ray"))] {
                ui.selectable_value(&mut p.reference, k.to_string(), l);
            }
        });
        ui.label(
            RichText::new(tr(
                "Adds the polychromatic RMS of every field (field weights) as terms.",
            ))
            .small()
            .color(weak),
        );
    }
    if !vals {
        terms_ui(
            ui,
            "opt",
            &mut p.terms,
            &mut p.new_term,
            &mut p.terms_sheet,
            None,
            weak,
        );
        return;
    }
    let vals = p.term_vals();
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(tr("Evaluate"))
            .on_hover_text(tr(
                "Compute the merit function and the value of each term for the current system without changing it",
            ))
            .clicked()
        {
            p.eval_asked = true;
        }
    });
    terms_ui(
        ui,
        "opt",
        &mut p.terms,
        &mut p.new_term,
        &mut p.terms_sheet,
        Some(&vals),
        weak,
    );
}

/// 評価関数の項の表の選択と、パラメータの列を畳んだか
#[derive(Debug, Clone, Default)]
pub(crate) struct TermsState {
    pub sheet: crate::widgets::sheet::SheetState,
    /// P1..Pn の列を畳んで要約の 1 列にする
    pub fold: bool,
}

/// 評価関数の項の欄（最適化・差し替え探索）。id は欄の id の頭（同時に出るパネルで分ける）
pub(crate) fn terms_ui(
    ui: &mut Ui,
    id: &str,
    terms: &mut Vec<TermRow>,
    new_term: &mut String,
    st: &mut TermsState,
    vals: Option<&[Option<(f64, f64)>]>,
    weak: egui::Color32,
) {
    if new_term.is_empty() {
        *new_term = "paraxial".into();
    }
    if !terms.is_empty() {
        let label = if st.fold {
            tr("▶ Show the parameter columns")
        } else {
            tr("◀ Fold the parameter columns")
        };
        if ui
            .small_button(label)
            .on_hover_text(tr(
                "Folded, the parameters of each term are summarized in one column",
            ))
            .clicked()
        {
            st.fold = !st.fold;
        }
        let mut ts = sheets::TermSheet {
            terms,
            new_ty: new_term.as_str(),
            npar: sheets::par_cols(),
            vals,
            fold: st.fold,
        };
        // 畳んだ表と開いた表で列の幅を別に覚える
        crate::widgets::sheet::show(ui, (id, "terms", st.fold), &mut st.sheet, &mut ts);
    }
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt((id, "new_term"))
            .selected_text(new_term.clone())
            .width(90.0)
            .show_ui(ui, |ui| {
                for ty in TERM_TYPES {
                    ui.selectable_value(new_term, ty.to_string(), *ty)
                        .on_hover_text(type_hint(ty));
                }
            });
        if ui.button(tr("Add term")).clicked() {
            terms.push(TermRow::new(new_term));
        }
    });
    ui.label(
        RichText::new(tr(
            "Fields, wavelengths and configurations start at 1 (a field can also be \"hx, hy\"); surfaces are surface numbers. Empty boxes use the default.",
        ))
        .small()
        .color(weak),
    );
    ui.label(RichText::new(tr(SHEET_KEYS)).small().color(weak));
}

/// 表の使い方（表の下の一言）
pub(crate) const SHEET_KEYS: &str = n_(
    "Click a cell and type, or press Enter/F2 or double-click to edit; arrows and Tab move. Dragging, Shift+click or Shift+arrows select a range; the row numbers, column headings and # select whole rows, columns and the table. Alt+drag, the grip at the right of a number or Alt+↑↓ changes a number. Insert adds a row, Ctrl+D duplicates, Ctrl+Delete deletes; Ctrl+C/V copy and paste.",
);

fn settings_ui(ui: &mut Ui, p: &mut OptPanel, weak: egui::Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Max iterations"));
        ui.add(egui::DragValue::new(&mut p.iters).range(1..=10000));
        ui.label(tr("Derivatives"));
        for (k, l) in [
            ("auto", tr("Analytic (where possible)")),
            ("fd", tr("All finite differences")),
            ("forward", tr("All forward differences")),
        ] {
            ui.selectable_value(&mut p.jac, k.to_string(), l);
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut p.qr, tr("Solve steps by QR"))
            .on_hover_text(tr("Solve each step by QR instead of the normal equations (no squaring of the condition number; the path can differ by rounding on ill-conditioned problems)"));
        ui.checkbox(&mut p.active_set, tr("Hold variables at their bounds"))
            .on_hover_text(tr("Bounds: fix variables that a step would take out of range at the bound and solve again for the rest (instead of clipping each component)"));
        ui.checkbox(&mut p.stopped_as_vignetted, tr("Drop stopped rays"))
            .on_hover_text(tr("Drop stopped rays (total internal reflection, missing a surface) like vignetted rays instead of failing the evaluation"));
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Gradient stop"))
            .on_hover_text(tr("Stop when the largest cosine between the residuals and a variable's column is below this (0: no gradient test)"));
        ui.add(
            egui::DragValue::new(&mut p.gtol)
                .range(0.0..=1.0)
                .speed(1e-4)
                .max_decimals(8),
        );
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Config"));
        small_edit(ui, &mut p.configs, 160.0, tr("Open system only"));
    });
    ui.label(
        RichText::new(tr(
            "To optimize several configurations together, list .zmx configuration numbers or state names separated by commas (configuration numbers of terms and variables follow this order).",
        ))
        .small()
        .color(weak),
    );
}

fn history_plot(ui: &mut Ui, h: &[f64]) {
    let pts: Vec<[f64; 2]> = h
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > 0.0)
        .map(|(i, v)| [i as f64, v.log10()])
        .collect();
    // 縦軸の名前は図の上に書く（縦書きの名前は図の高さより長く、上の行に重なる）
    ui.label(RichText::new(tr("Merit (log10)")).small().weak());
    Plot::new("opt_history")
        .height(140.0)
        .x_axis_label(tr("Iteration"))
        .allow_scroll(false)
        .show(ui, |pu| pu.line(Line::new(pts)));
}

/// 評価関数の値の倍率と単位。項がスポット（と重心）だけなら µm、波面・OPD・Zernike だけなら波、ほかは単位なし
fn merit_unit(p: &OptPanel) -> (f64, &'static str) {
    let mut kinds: Vec<&str> = p.terms.iter().map(|t| t.ty.as_str()).collect();
    if p.merit != "none" {
        kinds.push(p.merit.as_str());
    }
    let all = |set: &[&str]| !kinds.is_empty() && kinds.iter().all(|k| set.contains(k));
    if all(&["spot", "centroid"]) {
        (1000.0, "µm")
    } else if all(&["wavefront", "opd", "zernike"]) {
        (1.0, tr("waves"))
    } else {
        (1.0, "")
    }
}

fn stop_label(s: &str) -> &'static str {
    match s {
        "converged" => tr("Converged"),
        "small_step" => tr("Variables barely move"),
        "no_progress" => tr("No further decrease (local minimum)"),
        "max_iter" => tr("Max iterations"),
        "cancelled" => tr("Stopped"),
        "no_vars" => tr("No variables"),
        _ => "",
    }
}

fn result_ui(
    ui: &mut Ui,
    p: &mut OptPanel,
    r: &OptimizeView,
    acts: &mut Vec<Action>,
    weak: egui::Color32,
) {
    // 評価関数の単位（走らせたときの項が同じ単位のときだけ。混ざると単位なし）
    let (scale, unit) = p.run_unit;
    let v = |x: f64| {
        if unit.is_empty() {
            fmt_g(x)
        } else {
            format!("{} {unit}", fmt_g(x * scale))
        }
    };
    let res = ui.label(trf(
        "Merit {} → {} ({} iterations, {})",
        &[
            &v(r.merit0),
            &v(r.merit),
            &r.iterations,
            &stop_label(&r.stop),
        ],
    ));
    if unit.is_empty() {
        res.on_hover_text(tr(
            "The merit is √(Σ weight × residual² / Σ weight). Its terms have different units, so it has none",
        ));
    }
    if !r.history.is_empty() {
        let h: Vec<f64> = r
            .history
            .iter()
            .copied()
            .filter(|x| x.is_finite())
            .collect();
        history_plot(ui, &h);
    }
    let at = |v: &[f64], i: usize| v.get(i).copied().unwrap_or(f64::NAN);
    egui::Grid::new("opt_res_vars")
        .striped(true)
        .show(ui, |ui| {
            ui.label(RichText::new(tr("Variables")).small().color(weak));
            ui.label(RichText::new(tr("Before")).small().color(weak));
            ui.label(RichText::new(tr("After")).small().color(weak));
            ui.end_row();
            for (i, n) in r.names.iter().enumerate() {
                ui.label(n);
                ui.monospace(fmt_num(at(&r.x0, i)));
                ui.monospace(fmt_num(at(&r.x, i)));
                if r.applicable.get(i) == Some(&false) {
                    ui.label(
                        RichText::new(tr("Does not apply to the open system's configuration"))
                            .small()
                            .color(weak),
                    );
                }
                ui.end_row();
            }
        });
    let extra: Vec<&TermValue> = r.terms.iter().filter(|t| !t.default).collect();
    if !extra.is_empty() {
        egui::Grid::new("opt_res_terms")
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new(tr("Term")).small().color(weak));
                ui.label(RichText::new(tr("Before")).small().color(weak));
                ui.label(RichText::new(tr("After")).small().color(weak));
                ui.end_row();
                for (i, t) in extra.iter().enumerate() {
                    let ty = p.terms.get(i).map(|t| t.ty.as_str()).unwrap_or("");
                    ui.label(format!("{} {ty}", i + 1));
                    ui.monospace(fmt_g(t.value0));
                    ui.monospace(fmt_g(t.value));
                    ui.end_row();
                }
            });
    }
    for n in &r.notes {
        ui.label(
            RichText::new(lensbench_core::i18n::tr(n))
                .small()
                .color(weak),
        );
    }
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(tr("Apply to system"))
            .on_hover_text(tr(
                "Apply the optimized values to the open system (combination)",
            ))
            .clicked()
        {
            acts.push(Action::Apply);
        }
        if ui
            .add_enabled(p.undo.is_some(), egui::Button::new(tr("Undo apply")))
            .clicked()
        {
            acts.push(Action::Undo);
        }
    });
}

/// 結果の (名前, 値) のうち開いている系に当てられるもの。before なら最適化の前の値
pub(crate) fn result_values(r: &OptimizeView, before: bool) -> Vec<(String, f64)> {
    let x = if before { &r.x0 } else { &r.x };
    r.names
        .iter()
        .enumerate()
        .filter(|(i, _)| r.applicable.get(*i) != Some(&false))
        .filter_map(|(i, n)| Some((n.clone(), *x.get(i).filter(|v| v.is_finite())?)))
        .collect()
}

/// 組合せに当てる面の値: "7.t" → (7, "t", 値)。曲率 c は曲率半径 r に。当てられない変数は Err（名前）
pub(crate) fn combo_edit(name: &str, x: f64) -> Result<(usize, &'static str, Value), String> {
    let bad = || name.to_string();
    let (s, k) = name.rsplit_once('.').ok_or_else(bad)?;
    let j: usize = s.trim().parse().map_err(|_| bad())?;
    match k.trim().to_ascii_lowercase().as_str() {
        "t" => Ok((j, "t", json!(x))),
        "r" => Ok((j, "r", json!(x))),
        // 曲率 0 は平面（半径 0 の約束）
        "c" => Ok((j, "r", json!(if x == 0.0 { 0.0 } else { 1.0 / x }))),
        "k" => Ok((j, "k", json!(x))),
        _ => Err(bad()),
    }
}

#[cfg(test)]
mod tests;
