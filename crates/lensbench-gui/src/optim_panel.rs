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
    "efl", "pmag", "enpp", "epd", "expp", "expd", "wfno", "pwfn", "isfn", "pimh", "totr", "obna",
    "isna",
];
const RAY_Q: &[&str] = &["x", "y", "z", "l", "m", "n"];
const REFERENCE: &[&str] = &["centroid", "chief"];

/// 項の種類
pub const TERM_TYPES: &[&str] = &[
    "spot",
    "wavefront",
    "mtf",
    "paraxial",
    "ray",
    "centroid",
    "opd",
    "zernike",
    "coupling",
    "zplm",
    "plugin",
];

/// 種類ごとのキー（weight・config は共通）
fn keys(ty: &str) -> &'static [(&'static str, KeyKind)] {
    use KeyKind::*;
    match ty {
        "spot" | "wavefront" => &[
            ("field", Field),
            ("wave", Int),
            ("rings", Int),
            ("reference", Choice(REFERENCE)),
        ],
        "mtf" => &[
            ("field", Field),
            ("wave", Int),
            ("freq", Num),
            ("dir", Choice(&["both", "tan", "sag"])),
            ("n", Int),
            ("target", Num),
        ],
        "paraxial" => &[("q", Choice(PARAXIAL_Q)), ("target", Num)],
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
            ("n", Int),
            ("target", Num),
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
        "spot" => tr("RMS スポット半径（視野・波長、空の波長は多色）"),
        "wavefront" => tr("RMS 波面収差 [波]"),
        "mtf" => tr("MTF − 目標（既定の目標 1）"),
        "paraxial" => tr("近軸量など系全体の量 − 目標（System Data と同じ値）"),
        "ray" => tr("1 本の実光線の面の局所座標での位置・方向余弦 − 目標（REAX〜REAC 相当）"),
        "centroid" => tr("像面の光束の重心の x か y − 目標（スポットの項と同じ光線。鏡の傾きで像点を合わせる補償など）"),
        "opd" => tr("1 本の光線の OPD [波] − 目標（OPDC 相当）"),
        "zernike" => tr("Zernike 係数 [波] − 目標"),
        "coupling" => tr("ファイバ結合効率 − 目標（既定の目標 1、有限物体）"),
        "plugin" => tr("登録したプラグインの出力 output − 目標（name は rms・zernike など、args は設定の JSON 例 {\"rings\": 4}、index は出力の要素）"),
        "zplm" => tr("ZPL マクロの OPTRETURN data の値 − 目標（ZPLM 相当。macro は番号 nn = ZPLnn.ZPL か名前、file は .ZPL のパス、macro_dirs は探すフォルダを ; で区切る）"),
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
        "zplm" => vec![("macro", "0"), ("data", "0")],
        "plugin" => vec![("name", "rms"), ("field", "1"), ("output", "wfe")],
        _ => vec![],
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VarRow {
    pub name: String,
    pub lo: String,
    pub hi: String,
    /// 効く構成（「1, 2」。空はすべて）
    pub configs: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TermRow {
    pub ty: String,
    /// キーの値（空は省く）
    pub vals: Vec<(String, String)>,
    pub weight: String,
    pub config: String,
}

impl TermRow {
    pub fn new(ty: &str) -> TermRow {
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
pub enum Action {
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
pub struct OptPanel {
    pub vars: Vec<VarRow>,
    pub terms: Vec<TermRow>,
    /// 既定の評価関数: spot・wavefront・none
    pub merit: String,
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
    pub result: Option<Value>,
    pub error: Option<String>,
    /// 系に当てる前の値（元に戻す）
    pub undo: Option<Vec<(String, f64)>>,
    /// 最適化を始めたときの系の世代（[`crate::app`] の sys_gen。変わっていれば結果を当てない）
    pub gen: u64,
}

impl OptPanel {
    pub fn new() -> OptPanel {
        OptPanel {
            merit: "spot".into(),
            rings: 3,
            reference: "centroid".into(),
            iters: 50,
            jac: "auto".into(),
            new_term: "paraxial".into(),
            ..Default::default()
        }
    }

    /// serve の optimize の params（保存するファイルと同じ）
    pub fn params(&self) -> Result<Value, String> {
        let num = |s: &str, what: &str| -> Result<Option<f64>, String> {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            t.parse::<f64>()
                .map(Some)
                .map_err(|_| trf("{} の値 {} が数でない", &[&what, &t]))
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
            terms.push(term_json(t).map_err(|e| trf("項 {}: {}", &[&(i + 1), &e]))?);
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
    pub fn set_params(&mut self, p: &Value) -> Result<(), String> {
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
                .ok_or_else(|| trf("項 {} がオブジェクトでない", &[&(i + 1)]))?;
            let ty = o.get("type").and_then(Value::as_str).unwrap_or_default();
            if !TERM_TYPES.contains(&ty) {
                return Err(trf("項 {} の種類 {}", &[&(i + 1), &ty]));
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
        .map(|(i, t)| term_json(t).map_err(|e| trf("項 {}: {}", &[&(i + 1), &e])))
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
                .map_err(|_| trf("構成の番号 {} が整数でない", &[&x]))
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
        let bad = || trf("{} の値 {}", &[k, &v]);
        let x = match kind {
            KeyKind::Num => json!(v.parse::<f64>().map_err(|_| bad())?),
            KeyKind::Int => json!(v.parse::<u64>().map_err(|_| bad())?),
            KeyKind::Field => match v.parse::<u64>() {
                Ok(i) => json!(i),
                Err(_) => pair(v).ok_or_else(bad)?,
            },
            KeyKind::Pair => pair(v).ok_or_else(bad)?,
            KeyKind::Choice(_) | KeyKind::Text => json!(v),
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
                .map_err(|_| trf("重み {} が数でない", &[&w]))?),
        );
    }
    if !t.config.trim().is_empty() {
        let c = t.config.trim();
        o.insert(
            "config".into(),
            json!(c
                .parse::<u64>()
                .map_err(|_| trf("構成の番号 {} が整数でない", &[&c]))?),
        );
    }
    Ok(Value::Object(o))
}

/// 変数の候補（面の表の行・玉と群）
pub struct Cands<'a> {
    /// 玉・群の名前
    pub units: Vec<String>,
    /// 組合せの系（面の値は t・r・k だけ組合せに当てられる）
    pub combo: bool,
    /// 名前の欄の補完の候補
    pub sys: crate::param_pick::Sys,
    /// 面の表の行（変数の今の値を出す、GUX-21）
    pub rows: &'a [crate::lens_data::Row],
}

/// 変数の今の値（面の値 `<面>.<t|r|c|k>` だけ。玉・群・ほかは None、GUX-21）
pub fn var_value(rows: &[crate::lens_data::Row], name: &str) -> Option<f64> {
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
    ui.add(
        egui::TextEdit::singleline(s)
            .desired_width(w)
            .hint_text(hint),
    )
}

/// パネルを描く。`progress` は走っている間の評価関数の履歴
pub fn ui(ui: &mut Ui, p: &mut OptPanel, c: &Cands, progress: Option<&[f64]>) -> Vec<Action> {
    let mut acts = Vec::new();
    let weak = theme::weak_text(ui.visuals());
    egui::ScrollArea::vertical()
        .id_salt("optim_scroll")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if p.running {
                    if ui.button(tr("止める")).clicked() {
                        acts.push(Action::Stop);
                    }
                    ui.spinner();
                } else {
                    let no_var = p.vars.iter().all(|v| v.name.trim().is_empty());
                    let check = if no_var { Err(String::new()) } else { p.params().map(|_| ()) };
                    let run = ui
                        .add_enabled(
                            check.is_ok(),
                            egui::Button::new(RichText::new(tr("最適化する")).strong()),
                        )
                        .on_disabled_hover_text(if no_var {
                            tr("変数を 1 つ以上足す（下の「変数」の表に 4.t のような名前、または […] から選ぶ）").to_string()
                        } else {
                            check.clone().err().unwrap_or_default()
                        });
                    // 押せない理由を横にも出す（GUX-21）
                    if no_var {
                        ui.label(
                            RichText::new(tr("変数を 1 つ以上足すと押せる"))
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
                                acts.push(Action::Run(v));
                            }
                            Err(e) => p.error = Some(e),
                        }
                    }
                }
                ui.add_enabled_ui(!p.running, |ui| {
                    if ui.button(tr("設定を保存…")).clicked() {
                        match p.params() {
                            Ok(v) => acts.push(Action::Save(v)),
                            Err(e) => p.error = Some(e),
                        }
                    }
                    if ui.button(tr("設定を読む…")).clicked() {
                        acts.push(Action::Load);
                    }
                });
            });
            if let Some(e) = &p.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            if let Some(h) = progress {
                ui.label(trf(
                    "反復 {}: 評価関数 {}",
                    &[
                        &(h.len().saturating_sub(1)),
                        &fmt_g(*h.last().unwrap_or(&f64::NAN)),
                    ],
                ));
                history_plot(ui, h);
            } else if let Some(r) = p.result.clone() {
                egui::CollapsingHeader::new(RichText::new(tr("結果")).strong())
                    .id_salt("opt_result")
                    .default_open(true)
                    .show(ui, |ui| result_ui(ui, p, &r, &mut acts, weak));
            }
            ui.separator();
            ui.add_enabled_ui(!p.running, |ui| {
                section(ui, "opt_vars_sec", tr("変数"), |ui| {
                    vars_ui(ui, p, c, weak)
                });
                section(ui, "opt_merit_sec", tr("評価関数"), |ui| {
                    merit_ui(ui, p, weak)
                });
                section(ui, "opt_settings_sec", tr("設定"), |ui| {
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
            ui.label(RichText::new(tr("名前")).small().color(weak));
            ui.label(RichText::new(tr("今の値")).small().color(weak));
            ui.label(RichText::new(tr("下限")).small().color(weak));
            ui.label(RichText::new(tr("上限")).small().color(weak));
            if multi {
                ui.label(RichText::new(tr("構成")).small().color(weak));
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
                    small_edit(ui, &mut v.configs, 50.0, tr("すべて"))
                        .on_hover_text(tr("効く構成の番号（カンマ区切り、空はすべて）"));
                }
                if ui.small_button("✖").on_hover_text(tr("消す")).clicked() {
                    del = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = del {
        p.vars.remove(i);
    }
    ui.horizontal(|ui| {
        let (_, mut enter) = pp::edit(
            ui,
            &mut p.new_var,
            110.0,
            tr("7.t・lens2.tx など"),
            &c.sys,
            set,
        );
        if let Some(n) = pp::menu_button(ui, &c.sys, set, tr("面・玉・群から選んで足す"))
        {
            p.new_var = n;
            enter = true;
        }
        let name = p.new_var.trim().to_string();
        if (ui.button(tr("足す")).clicked() || enter) && !name.is_empty() {
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
                "組合せの系: 結果を組合せに当てられるのは面の厚さ（間隔）・曲率半径・コーニックだけです。",
            ))
            .small()
            .color(weak),
        );
    }
}

pub(crate) fn merit_ui(ui: &mut Ui, p: &mut OptPanel, weak: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(tr("既定"));
        for (k, l) in [
            ("spot", tr("RMS スポット")),
            ("wavefront", tr("RMS 波面")),
            ("none", tr("なし")),
        ] {
            ui.selectable_value(&mut p.merit, k.to_string(), l);
        }
    });
    if p.merit != "none" {
        ui.horizontal(|ui| {
            ui.label(tr("リング数"));
            ui.add(egui::DragValue::new(&mut p.rings).range(1..=20));
            ui.label(tr("基準"));
            for (k, l) in [("centroid", tr("重心")), ("chief", tr("主光線"))] {
                ui.selectable_value(&mut p.reference, k.to_string(), l);
            }
        });
        ui.label(
            RichText::new(tr("全視野（視野の重み）・多色の RMS を項にします。"))
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
                            KeyKind::Choice(opts) => {
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
                            KeyKind::Text | KeyKind::Dirs | KeyKind::Json => {
                                small_edit(ui, &mut v, 140.0, "").changed()
                            }
                            _ => small_edit(ui, &mut v, 48.0, "").changed(),
                        };
                        if changed {
                            t.set(k, &v);
                        }
                    }
                    ui.label(RichText::new(tr("重み")).small().color(weak));
                    small_edit(ui, &mut t.weight, 40.0, "1");
                    ui.label(RichText::new(tr("構成")).small().color(weak));
                    small_edit(ui, &mut t.config, 30.0, "1");
                    if ui.small_button("✖").on_hover_text(tr("消す")).clicked() {
                        del = Some(i);
                    }
                });
            });
    }
    if let Some(i) = del {
        terms.remove(i);
    }
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt((id, "new_term"))
            .selected_text(new_term.clone())
            .width(90.0)
            .show_ui(ui, |ui| {
                for ty in TERM_TYPES {
                    ui.selectable_value(new_term, ty.to_string(), *ty)
                        .on_hover_text(type_hint(ty));
                }
            });
        if ui.button(tr("項を足す")).clicked() {
            terms.push(TermRow::new(new_term));
        }
    });
    ui.label(
        RichText::new(tr(
            "視野・波長・構成は 1 始まり（視野は「hx, hy」も可）、面は面の番号。空の欄は既定の値。",
        ))
        .small()
        .color(weak),
    );
}

fn settings_ui(ui: &mut Ui, p: &mut OptPanel, weak: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(tr("反復の上限"));
        ui.add(egui::DragValue::new(&mut p.iters).range(1..=10000));
        ui.label(tr("微分"));
        for (k, l) in [
            ("auto", tr("解析的（できる所）")),
            ("fd", tr("すべて差分")),
            ("forward", tr("すべて前進差分")),
        ] {
            ui.selectable_value(&mut p.jac, k.to_string(), l);
        }
    });
    ui.horizontal(|ui| {
        ui.label(tr("構成"));
        small_edit(ui, &mut p.configs, 160.0, tr("開いている系だけ"));
    });
    ui.label(
        RichText::new(tr(
            "構成を同時に扱うときは .zmx の構成番号か状態の名前をカンマで並べます（項・変数の構成の番号はこの並びの順）。",
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
    Plot::new("opt_history")
        .height(140.0)
        .x_axis_label(tr("反復"))
        .y_axis_label(tr("評価関数（log10）"))
        .allow_scroll(false)
        .show(ui, |pu| pu.line(Line::new(pts)));
}

fn stop_label(s: &str) -> &'static str {
    match s {
        "converged" => tr("収束した"),
        "small_step" => tr("変数がほとんど動かない"),
        "no_progress" => tr("これ以上減らない（局所解）"),
        "max_iter" => tr("反復の上限"),
        "cancelled" => tr("止めた"),
        "no_vars" => tr("変数がない"),
        _ => "",
    }
}

fn result_ui(
    ui: &mut Ui,
    p: &mut OptPanel,
    r: &Value,
    acts: &mut Vec<Action>,
    weak: egui::Color32,
) {
    let f = |k: &str| r[k].as_f64().unwrap_or(f64::NAN);
    ui.label(trf(
        "評価関数 {} → {}（反復 {}、{}）",
        &[
            &fmt_g(f("merit0")),
            &fmt_g(f("merit")),
            &r["iterations"],
            &stop_label(r["stop"].as_str().unwrap_or_default()),
        ],
    ));
    if let Some(h) = r["history"].as_array() {
        let h: Vec<f64> = h.iter().filter_map(Value::as_f64).collect();
        history_plot(ui, &h);
    }
    let names: Vec<&str> = r["names"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    egui::Grid::new("opt_res_vars")
        .striped(true)
        .show(ui, |ui| {
            ui.label(RichText::new(tr("変数")).small().color(weak));
            ui.label(RichText::new(tr("前")).small().color(weak));
            ui.label(RichText::new(tr("後")).small().color(weak));
            ui.end_row();
            for (i, n) in names.iter().enumerate() {
                ui.label(*n);
                ui.monospace(fmt_num(r["x0"][i].as_f64().unwrap_or(f64::NAN)));
                ui.monospace(fmt_num(r["x"][i].as_f64().unwrap_or(f64::NAN)));
                if r["applicable"][i] == json!(false) {
                    ui.label(
                        RichText::new(tr("開いている系の構成には効かない"))
                            .small()
                            .color(weak),
                    );
                }
                ui.end_row();
            }
        });
    let extra: Vec<&Value> = r["terms"]
        .as_array()
        .map(|a| a.iter().filter(|t| t["default"] == json!(false)).collect())
        .unwrap_or_default();
    if !extra.is_empty() {
        egui::Grid::new("opt_res_terms")
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new(tr("項")).small().color(weak));
                ui.label(RichText::new(tr("前")).small().color(weak));
                ui.label(RichText::new(tr("後")).small().color(weak));
                ui.end_row();
                for (i, t) in extra.iter().enumerate() {
                    let ty = p.terms.get(i).map(|t| t.ty.as_str()).unwrap_or("");
                    ui.label(format!("{} {ty}", i + 1));
                    ui.monospace(fmt_g(t["value0"].as_f64().unwrap_or(f64::NAN)));
                    ui.monospace(fmt_g(t["value"].as_f64().unwrap_or(f64::NAN)));
                    ui.end_row();
                }
            });
    }
    for n in r["notes"].as_array().into_iter().flatten() {
        ui.label(
            RichText::new(n.as_str().unwrap_or_default())
                .small()
                .color(weak),
        );
    }
    ui.horizontal(|ui| {
        if ui
            .button(tr("系に反映"))
            .on_hover_text(tr("最適化した値を開いている系（組合せ）に当てる"))
            .clicked()
        {
            acts.push(Action::Apply);
        }
        if ui
            .add_enabled(p.undo.is_some(), egui::Button::new(tr("反映を元に戻す")))
            .clicked()
        {
            acts.push(Action::Undo);
        }
    });
}

/// 結果の (名前, 値) のうち開いている系に当てられるもの。before なら最適化の前の値
pub fn result_values(r: &Value, before: bool) -> Vec<(String, f64)> {
    let key = if before { "x0" } else { "x" };
    r["names"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(i, _)| r["applicable"][*i] != json!(false))
        .filter_map(|(i, n)| Some((n.as_str()?.to_string(), r[key][i].as_f64()?)))
        .collect()
}

/// 組合せに当てる面の値: "7.t" → (7, "t", 値)。曲率 c は曲率半径 r に。当てられない変数は Err（名前）
pub fn combo_edit(name: &str, x: f64) -> Result<(usize, &'static str, Value), String> {
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

    /// GUX-21: 変数の行に面の値（t・r・c・k）の今の値を出す
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
