//! 最適化のパネル: 変数・評価関数の項・設定を並べ、serve の `optimize` で減衰最小二乗を回す。
//!
//! 設定は serve の `optimize` の params と同じ JSON（[`OptPanel::params`]）で、そのままファイルに保存・読込できる。
//! 項の種類とキーは `lensbench_design::merit`（CLI の `--term`）と同じ。走っている間は
//! [`lensbench_engine::optimize_progress`] で評価関数の履歴を読み、止めるのは
//! [`lensbench_engine::optimize_cancel`]。

use egui::{RichText, Ui};
use egui_plot::{Line, Plot};
use serde_json::{json, Map, Value};

use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::param_pick::{self as pp, Set};
use crate::theme;
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
    "isna",
];
const RAY_Q: &[&str] = &["x", "y", "z", "l", "m", "n", "trax", "tray", "trar", "opth"];
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
];
const REFERENCE: &[&str] = &["centroid", "chief"];
/// スポット・波面の瞳の求積（arms は opticstudio、n は grid と使う）
const QUADRATURE: &[&str] = &["lensbench", "opticstudio", "grid"];

/// 項の種類
pub(crate) const TERM_TYPES: &[&str] = &[
    "spot",
    "wavefront",
    "mtf",
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
    "surface",
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
            ("dir", Choice(&["both", "tan", "sag"])),
            ("n", Int),
            ("target", Num),
        ],
        "paraxial" => &[("q", Choice(PARAXIAL_Q)), ("wave", Int), ("target", Num)],
        "ray" => &[
            ("field", Field),
            ("wave", Int),
            ("px", Num),
            ("py", Num),
            ("surf", Int),
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
            ("edge", Bool),
            ("zone", Num),
            ("each", Choice(&["all", "air", "glass"])),
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
        "surface" => &[
            ("surf", Int),
            ("q", Choice(SURF_Q)),
            ("x", Num),
            ("y", Num),
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
            ("target", Num),
        ],
        _ => &[],
    }
}

/// 種類の説明（ツールチップ）
fn type_hint(ty: &str) -> &'static str {
    match ty {
        "spot" => tr("RMS spot radius (field, wavelength; empty wavelength = polychromatic)"),
        "wavefront" => tr("RMS wavefront error [waves]"),
        "mtf" => tr("MTF − target (default target 1)"),
        "paraxial" => tr("System quantity such as a paraxial value − target (same values as System Data)"),
        "ray" => tr("Position / direction cosine of one real ray in the surface's local coordinates − target (like REAX–REAC)"),
        "centroid" => tr("x or y of the beam centroid on the image − target (same rays as the spot term; e.g. a compensator that centres the returned spot with a mirror tilt)"),
        "opd" => tr("OPD of one ray [waves] − target (like OPDC)"),
        "zernike" => tr("Zernike coefficient [waves] − target"),
        "coupling" => tr("Fiber coupling efficiency − target (default target 1, finite object)"),
        "thickness" => tr("Center thickness of surface surf (the sum up to surface to) − target (like CTVA and TTHI; min or max for CTGT and CTLT). edge: edge thickness (ETVA). each: compare each surface with min or max (MNCT, MNCA, MNEG etc.)"),
        "calc" => tr("op of args (numbers or terms with a single value, e.g. {\"type\": \"paraxial\", \"q\": \"efl\", \"target\": 0}, 5) − target (like SUMM, DIFF, PROD, DIVI, ABSO, SQRT and RECI)"),
        "surface" => tr("Curvature, conic, sag, thickness at a point, diameter or global vertex position of surface surf − target (like CVVA, COVA, SSAG, SAGX, STHI, DMVA and GLCX; min or max for CVGT, DMLT etc.). x and y are the point for sag and thick"),
        "plugin" => tr("Output output of a registered plugin − target (name is rms, zernike, …; args is the settings as JSON, e.g. {\"rings\": 4}; index picks one element)"),
        "zplm" => tr("Value of OPTRETURN data from a ZPL macro − target (like ZPLM; macro is a number nn = ZPLnn.ZPL or a name, file is a .ZPL path, macro_dirs are folders to search separated by ;)"),
        _ => "",
    }
}

/// 種類ごとの、足したときの値
fn defaults(ty: &str) -> Vec<(&'static str, &'static str)> {
    match ty {
        "spot" | "wavefront" | "opd" | "zernike" => vec![("field", "1")],
        "mtf" => vec![("field", "1"), ("freq", "30")],
        "paraxial" => vec![("q", "efl"), ("target", "")],
        "ray" => vec![("field", "1"), ("py", "1"), ("q", "y"), ("target", "0")],
        "centroid" => vec![("field", "1"), ("q", "x"), ("target", "0")],
        "thickness" => vec![("surf", "1"), ("target", "")],
        "surface" => vec![("surf", "1"), ("q", "curv"), ("target", "")],
        "zplm" => vec![("macro", "0"), ("data", "0")],
        "plugin" => vec![("name", "rms"), ("field", "1"), ("output", "wfe")],
        _ => vec![],
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
            new_term: "paraxial".into(),
            ..Default::default()
        }
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
        let kind = ks
            .iter()
            .find(|(a, _)| a == k)
            .map(|x| x.1)
            .unwrap_or(KeyKind::Num);
        let bad = || trf("Value of {}: {}", &[k, &v]);
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
                    merit_ui(ui, p, weak)
                });
                section(ui, "opt_settings_sec", tr("Settings"), |ui| {
                    settings_ui(ui, p, weak)
                });
            });
        });
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
    let multi = !p.configs.trim().is_empty();
    let mut del = None;
    egui::Grid::new("opt_vars")
        .striped(true)
        .num_columns(if multi { 6 } else { 5 })
        .show(ui, |ui| {
            ui.label(RichText::new(tr("Name")).small().color(weak));
            ui.label(RichText::new(tr("Current")).small().color(weak));
            ui.label(RichText::new(tr("Min")).small().color(weak));
            ui.label(RichText::new(tr("Max")).small().color(weak));
            if multi {
                ui.label(RichText::new(tr("Config")).small().color(weak));
            }
            ui.label("");
            ui.end_row();
            for (i, v) in p.vars.iter_mut().enumerate() {
                pp::edit(ui, &mut v.name, 110.0, "7.t", &c.sys, set);
                match var_value(c.rows, &v.name) {
                    Some(x) => ui.label(RichText::new(fmt_num(x)).monospace()),
                    None => ui.weak("—"),
                };
                small_edit(ui, &mut v.lo, 60.0, "−∞");
                small_edit(ui, &mut v.hi, 60.0, "∞");
                if multi {
                    small_edit(ui, &mut v.configs, 50.0, tr("All")).on_hover_text(tr(
                        "Configurations it applies to (comma separated, empty = all)",
                    ));
                }
                if ui.small_button("✖").on_hover_text(tr("Delete")).clicked() {
                    del = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = del {
        p.vars.remove(i);
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

pub(crate) fn merit_ui(ui: &mut Ui, p: &mut OptPanel, weak: egui::Color32) {
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
    terms_ui(ui, "opt", &mut p.terms, &mut p.new_term, weak);
}

/// 評価関数の項の欄（最適化・差し替え探索）。id は欄の id の頭（同時に出るパネルで分ける）
pub(crate) fn terms_ui(
    ui: &mut Ui,
    id: &str,
    terms: &mut Vec<TermRow>,
    new_term: &mut String,
    weak: egui::Color32,
) {
    if new_term.is_empty() {
        *new_term = "paraxial".into();
    }
    let mut del = None;
    for (i, t) in terms.iter_mut().enumerate() {
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt((id, "term_ty", i))
                        .selected_text(t.ty.clone())
                        .width(90.0)
                        .show_ui(ui, |ui| {
                            for ty in TERM_TYPES {
                                if ui
                                    .selectable_label(t.ty == *ty, *ty)
                                    .on_hover_text(type_hint(ty))
                                    .clicked()
                                    && t.ty != *ty
                                {
                                    *t = TermRow::new(ty);
                                }
                            }
                        })
                        .response
                        .on_hover_text(type_hint(&t.ty));
                    for (k, kind) in keys(&t.ty) {
                        ui.label(RichText::new(*k).small().color(weak));
                        let mut v = t.get(k).to_string();
                        let changed = match kind {
                            KeyKind::Choice(_) | KeyKind::Bool => {
                                let opts = match kind {
                                    KeyKind::Choice(o) => *o,
                                    _ => &["true", "false"],
                                };
                                let mut ch = false;
                                egui::ComboBox::from_id_salt((id, "term_k", i, *k))
                                    .selected_text(if v.is_empty() { "—" } else { v.as_str() })
                                    .width(70.0)
                                    .show_ui(ui, |ui| {
                                        for o in opts.iter() {
                                            ch |= ui
                                                .selectable_value(&mut v, o.to_string(), *o)
                                                .changed();
                                        }
                                    });
                                ch
                            }
                            KeyKind::Field | KeyKind::Pair | KeyKind::Macro => {
                                small_edit(ui, &mut v, 56.0, "").changed()
                            }
                            KeyKind::Text | KeyKind::Dirs | KeyKind::Json | KeyKind::JsonList => {
                                small_edit(ui, &mut v, 140.0, "").changed()
                            }
                            _ => small_edit(ui, &mut v, 48.0, "").changed(),
                        };
                        if changed {
                            t.set(k, &v);
                        }
                    }
                    ui.label(RichText::new(tr("Weight")).small().color(weak));
                    small_edit(ui, &mut t.weight, 40.0, "1");
                    ui.label(RichText::new(tr("Config")).small().color(weak));
                    small_edit(ui, &mut t.config, 30.0, "1");
                    if ui.small_button("✖").on_hover_text(tr("Delete")).clicked() {
                        del = Some(i);
                    }
                });
            });
    }
    if let Some(i) = del {
        terms.remove(i);
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
}

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
mod tests {
    use super::*;

    /// 評価関数の単位は項が同じ単位のときだけ（スポットは µm、波面は波、混ざると無し）
    #[test]
    fn gux20_merit_unit() {
        let mut p = OptPanel {
            merit: "spot".into(),
            ..Default::default()
        };
        assert_eq!(merit_unit(&p), (1000.0, "µm"));
        p.terms.push(TermRow::new("centroid"));
        assert_eq!(merit_unit(&p).1, "µm");
        p.terms.push(TermRow::new("wavefront"));
        assert_eq!(merit_unit(&p).1, "");
        p.merit = "none".into();
        p.terms.retain(|t| t.ty == "wavefront");
        assert_eq!(merit_unit(&p).1, tr("waves"));
        p.terms.clear();
        assert_eq!(merit_unit(&p).1, "");
    }

    /// 変数の行に面の値（t・r・c・k）の今の値を出す
    #[test]
    fn gux21_var_value_reads_surface_rows() {
        let row = |r: f64, t: f64| crate::lens_data::Row {
            r,
            t,
            k: -1.0,
            ..Default::default()
        };
        let rows = vec![row(f64::INFINITY, 0.0), row(50.0, 4.0)];
        assert_eq!(var_value(&rows, "1.t"), Some(4.0));
        assert_eq!(var_value(&rows, " 1.r"), Some(50.0));
        assert_eq!(var_value(&rows, "1.c"), Some(0.02));
        assert_eq!(var_value(&rows, "1.k"), Some(-1.0));
        assert_eq!(var_value(&rows, "0.r"), Some(f64::INFINITY));
        assert_eq!(var_value(&rows, "0.c"), Some(0.0));
        assert_eq!(var_value(&rows, "5.t"), None);
        assert_eq!(var_value(&rows, "G1.shift"), None);
    }

    #[test]
    fn params_round_trip() {
        let mut p = OptPanel::new();
        p.vars.push(VarRow {
            name: "7.t".into(),
            lo: "0.5".into(),
            hi: "".into(),
            configs: "1, 2".into(),
        });
        let mut t = TermRow::new("ray");
        t.set("field", "0, 0.5");
        t.weight = "2".into();
        p.terms.push(t);
        let mut q = TermRow::new("paraxial");
        q.set("target", "50");
        p.terms.push(q);
        let mut z = TermRow::new("zplm");
        z.set("macro", "lbEval");
        z.set("data", "3");
        z.set("macro_dirs", "C:\\a, b; /c");
        p.terms.push(z);
        let mut z = TermRow::new("zplm");
        z.set("macro", "7");
        p.terms.push(z);
        let mut g = TermRow::new("plugin");
        g.set("args", r#"{"f": 60, "passes": 1}"#);
        g.set("target", "0.001");
        p.terms.push(g);
        p.configs = "1, 2".into();
        p.merit = "none".into();
        let v = p.params().unwrap();
        assert_eq!(
            v["vars"][0],
            json!({"name": "7.t", "lo": 0.5, "configs": [1, 2]})
        );
        assert_eq!(
            v["terms"][0],
            json!({"type": "ray", "field": [0.0, 0.5], "py": 1.0, "q": "y", "target": 0.0, "weight": 2.0})
        );
        assert_eq!(
            v["terms"][1],
            json!({"type": "paraxial", "q": "efl", "target": 50.0})
        );
        assert_eq!(
            v["terms"][2],
            json!({"type": "zplm", "macro": "lbEval", "data": 3, "macro_dirs": ["C:\\a, b", "/c"]})
        );
        assert_eq!(
            v["terms"][3],
            json!({"type": "zplm", "macro": 7, "data": 0})
        );
        assert_eq!(v["configs"], json!([1, 2]));
        // 項の JSON は CLI と同じ読み方で読める
        let mut back = OptPanel::new();
        back.set_params(&v).unwrap();
        assert_eq!(back.params().unwrap(), v);
        // 間違い
        let mut bad = OptPanel::new();
        let mut t = TermRow::new("paraxial");
        t.set("target", "abc");
        bad.terms.push(t);
        assert!(bad.params().is_err());
        let mut bad = OptPanel::new();
        let mut t = TermRow::new("plugin");
        t.set("args", "[1, 2]");
        bad.terms.push(t);
        assert!(bad.params().is_err());
    }

    #[test]
    fn combo_edits() {
        assert_eq!(combo_edit("7.t", 2.0).unwrap(), (7, "t", json!(2.0)));
        assert_eq!(combo_edit("3.c", 0.02).unwrap(), (3, "r", json!(50.0)));
        assert_eq!(combo_edit("3.c", 0.0).unwrap(), (3, "r", json!(0.0)));
        assert!(combo_edit("lens1.tx", 1.0).is_err());
        assert!(combo_edit("3.dx", 1.0).is_err());
    }
}
