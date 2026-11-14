//! 引数・設定の一覧（名前・型・既定・説明）から作る入力欄。
//!
//! 一覧は serve の `analysis_args`（CLI の解析の引数、clap の定義から）か、プラグインの設定の項目
//! （`plugins` の `params`、[`lensbench_core::plugin::ParamSpec`]）と同じ形の JSON。欄の値は文字列で持ち
//! （並びの保存に入る）、空なら省く（解析・プラグインの既定）。計算のときに型に合わせて JSON の値にする。

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

use egui::{RichText, Ui};
use serde_json::{json, Map, Value};

use crate::i18n::{tr, trf};
use crate::theme;

/// 1 つの引数・設定の項目
#[derive(Debug, Clone, PartialEq)]
pub struct ArgSpec {
    pub name: String,
    /// num・count・int・flag・str・choice・nums・path
    pub ty: String,
    pub default: Value,
    pub unit: String,
    pub doc: String,
    pub choices: Vec<String>,
    /// nums の個数の上限（2 なら 1 個で x・y 共通）
    pub len: Option<usize>,
    /// 旗を繰り返して複数渡す（値はカンマ区切り）
    pub repeat: bool,
    /// 省けない引数
    pub required: bool,
    /// 一緒に使えない引数の名前
    pub conflicts: Vec<String>,
    /// 欄の短い名前（日本語の鍵、表示は tr で）。無ければ説明の頭か名前
    pub label: Option<&'static str>,
    /// 数の下限（リング数 0 などを欄で止める）
    pub min: Option<f64>,
}

impl ArgSpec {
    pub fn parse(v: &Value) -> Option<ArgSpec> {
        Some(ArgSpec {
            name: v["name"].as_str()?.to_string(),
            ty: v["type"].as_str().unwrap_or("str").to_string(),
            default: v["default"].clone(),
            unit: v["unit"].as_str().unwrap_or_default().to_string(),
            doc: v["doc"].as_str().unwrap_or_default().to_string(),
            choices: v["choices"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            len: v["len"].as_u64().map(|n| n as usize),
            repeat: v["repeat"].as_bool().unwrap_or(false),
            required: v["required"].as_bool().unwrap_or(false),
            conflicts: v["conflicts"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            label: None,
            min: v["min"].as_f64(),
        })
    }

    /// 欄に出す名前: 決めた短い名前、日本語なら説明の頭（短いとき）、ほかは引数の名前
    pub fn title(&self) -> String {
        if let Some(l) = self.label {
            return tr(l).to_string();
        }
        if crate::i18n::is_ja() {
            if let Some(h) = doc_head(&self.doc) {
                return h;
            }
        }
        // 設定のオブジェクトの項目は組の中で最後の部分だけ
        self.name
            .rsplit('.')
            .next()
            .unwrap_or(&self.name)
            .to_string()
    }

    /// 単位: 項目の単位か、説明の最初の [ ] の中（「、」の前）。英語で日本語の単位は出さない
    pub fn unit_text(&self) -> String {
        let u = if !self.unit.is_empty() {
            self.unit.clone()
        } else {
            doc_unit(&self.doc).unwrap_or_default()
        };
        match u.as_str() {
            "レンズ単位" => tr("レンズ単位").to_string(),
            _ if !u.is_ascii() && !u.contains('µ') && !crate::i18n::is_ja() => String::new(),
            _ => u,
        }
    }

    pub fn parse_list(v: &Value) -> Vec<ArgSpec> {
        v.as_array()
            .map(|a| a.iter().filter_map(ArgSpec::parse).collect())
            .unwrap_or_default()
    }

    /// 既定値を欄に書く形で（無ければ空）
    pub fn default_text(&self) -> String {
        value_text(&self.default)
    }

    /// 欄の文字列を JSON の値に（空なら None）
    pub fn value(&self, t: &str) -> Result<Option<Value>, String> {
        let t = t.trim();
        if t.is_empty() {
            return Ok(None);
        }
        let bad = || trf("{}: 値 {} が読めない", &[&self.name, &t]);
        let num = |s: &str| -> Result<f64, String> {
            s.trim()
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .ok_or_else(bad)
        };
        let v = self.value_raw(t, bad, num)?;
        if v.is_null() {
            return Ok(None);
        }
        if let (Some(m), Some(x)) = (self.min, v.as_f64()) {
            if x < m {
                return Err(trf("{} は {} 以上", &[&self.name, &m]));
            }
        }
        Ok(Some(v))
    }

    fn value_raw(
        &self,
        t: &str,
        bad: impl Fn() -> String,
        num: impl Fn(&str) -> Result<f64, String>,
    ) -> Result<Value, String> {
        Ok(match self.ty.as_str() {
            "flag" => match t {
                "true" => json!(true),
                "false" => json!(false),
                // プラグインの設定の旗は 0 か 1
                _ => match t.parse::<f64>() {
                    Ok(1.0) => json!(true),
                    Ok(0.0) => json!(false),
                    _ => return Err(bad()),
                },
            },
            "num" => json!(num(t)?),
            "count" => {
                let x = num(t)?;
                if x < 0.0 || x.fract() != 0.0 {
                    return Err(trf("{} は 0 以上の整数", &[&self.name]));
                }
                json!(x as u64)
            }
            "int" => {
                let x = num(t)?;
                if x.fract() != 0.0 {
                    return Err(trf("{} は整数", &[&self.name]));
                }
                json!(x as i64)
            }
            "nums" => {
                let v: Vec<f64> = split(t).map(num).collect::<Result<_, _>>()?;
                if v.is_empty() {
                    return Ok(Value::Null);
                }
                match self.len {
                    Some(n) if v.len() > n => {
                        return Err(trf("{} は {} 個まで", &[&self.name, &n]))
                    }
                    // x・y 共通（設定の JSON は 2 個が要る）
                    Some(2) if v.len() == 1 => Value::Array(vec![num_json(v[0]); 2]),
                    _ => Value::Array(v.into_iter().map(num_json).collect()),
                }
            }
            _ if self.repeat => json!(split(t).map(str::to_string).collect::<Vec<_>>()),
            _ => json!(t),
        })
    }
}

/// 説明の頭（最初の「（」「[」「。」「、」「:」の前）。短い（16 文字以下）ときだけ
fn doc_head(doc: &str) -> Option<String> {
    let end = doc
        .find(['（', '[', '。', '、', ':', '：', '('])
        .unwrap_or(doc.len());
    let h = doc[..end].trim();
    (!h.is_empty() && h.chars().count() <= 16).then(|| h.to_string())
}

/// 説明の最初の [ ] の中（「、」の前）
fn doc_unit(doc: &str) -> Option<String> {
    let a = doc.find('[')?;
    let rest = &doc[a + 1..];
    let b = rest.find(']')?;
    let u = rest[..b].split('、').next()?.trim();
    (!u.is_empty() && u.chars().count() <= 12).then(|| u.to_string())
}

/// 解析の引数の短い名前（日本語の鍵）。`cmd` ごとに違うものを先に
fn analysis_label(cmd: &str, name: &str) -> Option<&'static str> {
    Some(match (cmd, name) {
        ("mtf" | "mtf-focus" | "mtf-field" | "geo-mtf", "n") => "求積点の数",
        ("wavefront" | "zernike", "n") => "格子の一辺",
        ("fringe", "n") => "瞳の格子の一辺",
        ("pop", "n") => "入力の標本の数",
        ("mtf-field", "points") => "視野の点の数",
        ("rms", "points") => "横軸の点の数",
        ("fringe", "pupil") => "瞳の光線の格子",
        ("rays", "pupil") => "瞳座標",
        (_, "all_waves") => "全波長",
        ("pop", "rays") => "瞳の光線の数",
        ("rays", "rays") => "光線の本数",
        ("fringe", "center") => "範囲の中心",
        ("pop", "center") => "出力の中心",
        ("fringe", "width") => "範囲の一辺",
        ("mech-check", "grid") => "瞳の格子の一辺",
        ("elements", "out") => "組み直した系の書き出し",
        (_, "field") => "視野",
        (_, "wave") => "波長",
        (_, "freqs") => "周波数",
        (_, "max_freq") => "周波数の上限",
        (_, "freq_steps") => "周波数の分割数",
        (_, "rings") => "リング数",
        (_, "map") => "波面の格子も出す",
        (_, "kind") => "種類",
        (_, "terms") => "項数",
        (_, "direct") => "直接追跡",
        (_, "fft") => "FFT",
        (_, "interp") => "補間",
        (_, "each_wave") => "波長ごとも出す",
        (_, "range") => "ずらしの範囲 ±",
        (_, "steps") => "ずらしの点の数",
        (_, "axis") => "視野を振る向き",
        (_, "vs") => "横軸",
        (_, "reference") => "基準",
        (_, "field_wave") => "視野を解析の波長で解く",
        (_, "huygens") => "Huygens PSF",
        (_, "pupil") => "瞳の格子の一辺",
        (_, "image") => "像の格子の一辺",
        (_, "delta") => "像の格子の間隔",
        (_, "diff_limit") => "回折限界をかける",
        (_, "beam1") => "光束 1（試料）",
        (_, "beam2") => "光束 2（参照）",
        (_, "mode") => "重ねる所",
        (_, "scale") => "OPD の差の倍率",
        (_, "xtilt") => "x の傾きの縞",
        (_, "ytilt") => "y の傾きの縞",
        (_, "npx") => "画素の数",
        (_, "sample_power") => "試料の強度の倍率",
        (_, "reference_power") => "参照の強度の倍率",
        (_, "delay") => "参照の光路長",
        (_, "polarized") => "偏光追跡で重ねる",
        (_, "sample_input") => "試料の入射偏光",
        (_, "reference_input") => "参照の入射偏光",
        (_, "quantity") => "量",
        (_, "out") => "書き出すファイル",
        (_, "float") => "tif を float で",
        (_, "grid") => "格子も出す",
        (_, "surf") => "場を作る面",
        (_, "dist") => "伝える距離",
        (_, "to") => "伝える先の面",
        (_, "method") => "方法",
        (_, "half") => "入力の半幅",
        (_, "no_aperture") => "開口を掛けない",
        (_, "out_n") => "出力の標本の数",
        (_, "out_half") => "出力の半幅",
        (_, "fan") => "光線の並び",
        (_, "surface") => "書く面",
        (_, "first") => "最初の面",
        (_, "last") => "最後の面",
        (_, "coatings") => "膜のファイル",
        (_, "out_dir") => "系を書くフォルダ",
        (_, "reverse") => "反転する玉",
        (_, "table") => "膜の実測表",
        _ => return None,
    })
}

/// 一緒に使えない旗の組（2 つ以上。並びは一覧の順）。組は 1 つの選択で見せる
pub fn exclusive_groups(specs: &[&ArgSpec]) -> Vec<Vec<usize>> {
    let flag = |i: usize| specs[i].ty == "flag";
    let clash = |i: usize, j: usize| {
        specs[i].conflicts.contains(&specs[j].name) || specs[j].conflicts.contains(&specs[i].name)
    };
    let mut seen = vec![false; specs.len()];
    let mut out = Vec::new();
    for i in 0..specs.len() {
        if seen[i] || !flag(i) {
            continue;
        }
        // どの 2 つも一緒に使えない旗だけを集める
        let mut g = vec![i];
        let rest: Vec<usize> = (i + 1..specs.len())
            .filter(|&j| !seen[j] && flag(j))
            .collect();
        for j in rest {
            if g.iter().all(|&k| clash(k, j)) {
                g.push(j);
            }
        }
        if g.len() >= 2 {
            for &k in &g {
                seen[k] = true;
            }
            out.push(g);
        }
    }
    out
}

/// 解析 `cmd` の引数の一覧（CLI の定義から。プロセスの中で 1 度だけ作る）
pub fn analysis_specs(cmd: &str) -> Vec<ArgSpec> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<ArgSpec>>>> = OnceLock::new();
    let mut c = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    c.entry(cmd.to_string())
        .or_insert_with(|| {
            let mut v = lensbench_engine::params::analysis_args(cmd)
                .map(|v| ArgSpec::parse_list(&Value::Array(v)))
                .unwrap_or_default();
            for s in &mut v {
                s.label = analysis_label(cmd, &s.name);
            }
            v
        })
        .clone()
}

/// 設定のオブジェクトの欄の項目（拡張の解析の [`crate::ext::ObjectArg`]）
pub fn object_specs(o: &crate::ext::ObjectArg) -> Vec<ArgSpec> {
    ArgSpec::parse_list(&Value::Array((o.specs)()))
}

/// 数を JSON に。整数の値は整数で（設定の JSON の整数の項目、画素の数など、が読めるように）
fn num_json(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 9.0e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

/// カンマ・空白で区切る
fn split(t: &str) -> impl Iterator<Item = &str> {
    t.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
}

/// JSON の値を欄の文字列に
pub fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(value_text).collect::<Vec<_>>().join(", "),
        Value::Bool(b) => b.to_string(),
        x => x.to_string(),
    }
}

/// 欄の値を名前 → JSON の値に（空の欄は省く）
pub fn to_map(
    specs: &[ArgSpec],
    vals: &BTreeMap<String, String>,
) -> Result<Map<String, Value>, String> {
    let mut m = Map::new();
    for (k, t) in vals {
        // 一覧に無い名前（解析の種類を変えた前の欄など）は渡さない
        let Some(s) = specs.iter().find(|s| &s.name == k) else {
            continue;
        };
        if let Some(v) = s.value(t)? {
            // 既定が false の旗の false は省くのと同じ（CLI の旗は false を受けない）
            let def_true = s.default == json!(true) || s.default.as_f64() == Some(1.0);
            if v != json!(false) || s.ty != "flag" || def_true {
                m.insert(k.clone(), v);
            }
        }
    }
    Ok(m)
}

/// 「.」でつないだ名前を入れ子のオブジェクトに（`mla.pitch` → {"mla": {"pitch": …}}）
pub fn nest(m: &Map<String, Value>) -> Value {
    let mut root = Map::new();
    for (k, v) in m {
        let parts: Vec<&str> = k.split('.').collect();
        let mut cur = &mut root;
        for p in &parts[..parts.len() - 1] {
            cur = cur
                .entry(p.to_string())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("入れ子はオブジェクト");
        }
        cur.insert(parts[parts.len() - 1].to_string(), v.clone());
    }
    Value::Object(root)
}

/// 入れ子のオブジェクトを「.」でつないだ名前 → 欄の文字列に（[`nest`] の逆）
pub fn flatten(v: &Value) -> BTreeMap<String, String> {
    fn walk(prefix: &str, v: &Value, out: &mut BTreeMap<String, String>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    let name = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    walk(&name, x, out);
                }
            }
            Value::Null => {}
            x => {
                out.insert(prefix.to_string(), value_text(x));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk("", v, &mut out);
    out
}

/// 数の欄を ↑↓ で 1 刻み動かす（整数は 1、実数は書いた最後の桁の 1）。空なら既定から。動かせなければ None
pub fn step_text(t: &str, default: &Value, ty: &str, up: bool) -> Option<String> {
    let base = if t.trim().is_empty() {
        value_text(default)
    } else {
        t.trim().to_string()
    };
    let x: f64 = base.parse().ok().filter(|x: &f64| x.is_finite())?;
    let sign = if up { 1.0 } else { -1.0 };
    match ty {
        "count" | "int" => {
            let y = x.round() + sign;
            (ty == "int" || y >= 0.0).then(|| format!("{}", y as i64))
        }
        "num" => {
            // 指数の書き方は桁を数えにくいので 1 刻みでなく 10% 動かす
            if base.contains(['e', 'E']) {
                return Some(format!("{}", x * (1.0 + 0.1 * sign)));
            }
            let dec = base.split('.').nth(1).map_or(0, str::len).min(12);
            let d = 10f64.powi(-(dec as i32));
            Some(format!("{:.*}", dec, x + sign * d))
        }
        _ => None,
    }
}

/// 欄を並べる（名前・入力・単位・既定に戻す）。一緒に使えない旗は 1 つの選択に。値を変えたら true
/// （文字の欄は入力を終えたとき・Enter・数の欄の ↑↓）
pub fn form(
    ui: &mut Ui,
    salt: impl std::hash::Hash + Copy,
    specs: &[&ArgSpec],
    vals: &mut BTreeMap<String, String>,
) -> bool {
    let weak = theme::weak_text(ui.visuals());
    let mut changed = false;
    let groups = exclusive_groups(specs);
    let in_group = |i: usize| groups.iter().find(|g| g.contains(&i));
    egui::Grid::new(("argform", salt))
        .num_columns(3)
        .spacing([8.0, 3.0])
        .striped(true)
        .show(ui, |ui| {
            for (i, s) in specs.iter().enumerate() {
                if let Some(g) = in_group(i) {
                    if g[0] == i {
                        changed |= choice_group(ui, salt, specs, g, vals);
                    }
                    continue;
                }
                let set = vals.get(&s.name).is_some_and(|t| !t.trim().is_empty());
                let unit = s.unit_text();
                let mut label = RichText::new(s.title());
                if set {
                    label = label.strong();
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let r = ui.label(label);
                    if s.required {
                        ui.label(RichText::new("*").color(ui.visuals().warn_fg_color))
                            .on_hover_text(tr("省けない値"));
                    }
                    r.on_hover_ui(|ui| tip(ui, s));
                });
                ui.horizontal(|ui| {
                    let t = vals.entry(s.name.clone()).or_default();
                    match s.ty.as_str() {
                        "flag" => {
                            let def = s.default == json!(true) || s.default.as_f64() == Some(1.0);
                            let mut b = match s.value(t) {
                                Ok(Some(v)) => v == json!(true),
                                _ => def,
                            };
                            if ui.checkbox(&mut b, "").changed() {
                                *t = if b == def {
                                    String::new()
                                } else {
                                    b.to_string()
                                };
                                changed = true;
                            }
                        }
                        "choice" => {
                            let def = s.default_text();
                            let shown = if t.trim().is_empty() {
                                if def.is_empty() {
                                    tr("（既定）").to_string()
                                } else {
                                    trf("{}（既定）", &[&def])
                                }
                            } else {
                                t.clone()
                            };
                            egui::ComboBox::from_id_salt(("argchoice", salt, &s.name))
                                .selected_text(shown)
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(t.is_empty(), tr("（既定）")).clicked()
                                    {
                                        t.clear();
                                        changed = true;
                                    }
                                    for c in &s.choices {
                                        if ui.selectable_label(t == c, c).clicked() {
                                            *t = if *c == def { String::new() } else { c.clone() };
                                            changed = true;
                                        }
                                    }
                                });
                        }
                        _ => {
                            let hint = match (s.default_text(), s.ty.as_str()) {
                                (d, _) if !d.is_empty() => d,
                                (_, "nums") if s.required => tr("a, b, …（必須）").to_string(),
                                (_, "nums") => tr("自動（a, b, …）").to_string(),
                                _ if s.required => tr("必須").to_string(),
                                _ => tr("自動").to_string(),
                            };
                            let w = if s.ty == "path" || s.ty == "str" || s.ty == "nums" {
                                180.0
                            } else {
                                90.0
                            };
                            let r = ui.add(
                                egui::TextEdit::singleline(t)
                                    .hint_text(hint)
                                    .desired_width(w),
                            );
                            let num = matches!(s.ty.as_str(), "num" | "count" | "int");
                            if num && r.has_focus() {
                                let (up, down) = ui.input_mut(|i| {
                                    (
                                        i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                                        i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                                    )
                                });
                                if up || down {
                                    if let Some(n) = step_text(t, &s.default, &s.ty, up)
                                        .filter(|n| s.value(n).is_ok())
                                    {
                                        *t = n;
                                        changed = true;
                                    }
                                }
                            }
                            let r = if num {
                                r.on_hover_text(tr(
                                    "↑↓ で増減（整数は 1、実数は書いた最後の桁の 1）",
                                ))
                            } else {
                                r
                            };
                            let bad = s.value(t).is_err();
                            if bad {
                                ui.colored_label(ui.visuals().error_fg_color, "⚠")
                                    .on_hover_text(s.value(t).err().unwrap_or_default());
                            }
                            if r.lost_focus() && !bad {
                                changed = true;
                            }
                            if s.ty == "path" {
                                let start = ui
                                    .small_button("…")
                                    .on_hover_text(tr("ファイルを選ぶ"))
                                    .clicked();
                                // 欄ごとに区別する（ui.id() は兄弟の Ui で同じになる）
                                let key = format!(
                                    "arg-path-{:?}",
                                    egui::Id::new(("argpath", salt, &s.name))
                                );
                                if let Some(p) =
                                    crate::files::open(&key, start, crate::files::FileDialog::new)
                                {
                                    *t = p.display().to_string();
                                    changed = true;
                                }
                            }
                        }
                    }
                    if !unit.is_empty() {
                        ui.label(RichText::new(&unit).small().color(weak));
                    }
                });
                if reset_button(ui, set) {
                    vals.remove(&s.name);
                    changed = true;
                }
                ui.end_row();
            }
        });
    // 空の欄は持たない（並びの保存を小さく）
    vals.retain(|_, t| !t.trim().is_empty());
    changed
}

/// 欄の名前に乗せたときの説明（引数の名前・説明・単位・既定）
fn tip(ui: &mut Ui, s: &ArgSpec) {
    ui.label(RichText::new(&s.name).monospace().strong());
    if !s.doc.is_empty() {
        ui.label(&s.doc);
    }
    if !s.unit.is_empty() {
        ui.label(format!("[{}]", s.unit));
    }
    let d = s.default_text();
    if !d.is_empty() {
        ui.label(trf("既定: {}", &[&d]));
    }
}

fn reset_button(ui: &mut Ui, set: bool) -> bool {
    ui.add_enabled(
        set,
        egui::Button::new(RichText::new("↺").small()).frame(false),
    )
    .on_hover_text(tr("既定に戻す"))
    .clicked()
}

/// 一緒に使えない旗の組を 1 つの選択（既定・旗のどれか）で
fn choice_group(
    ui: &mut Ui,
    salt: impl std::hash::Hash + Copy,
    specs: &[&ArgSpec],
    g: &[usize],
    vals: &mut BTreeMap<String, String>,
) -> bool {
    let mut changed = false;
    let on = g.iter().copied().find(|&k| {
        vals.get(&specs[k].name)
            .is_some_and(|t| t == "true" || t == "1")
    });
    let r = ui.label(if on.is_some() {
        RichText::new(tr("方式")).strong()
    } else {
        RichText::new(tr("方式"))
    });
    r.on_hover_ui(|ui| {
        ui.label(tr("どれか 1 つ（一緒には使えない）"));
        for &k in g {
            ui.separator();
            tip(ui, specs[k]);
        }
    });
    let shown = on.map_or_else(|| tr("自動（既定）").to_string(), |k| specs[k].title());
    egui::ComboBox::from_id_salt(("argexcl", salt, &specs[g[0]].name))
        .selected_text(shown)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(on.is_none(), tr("自動（既定）"))
                .clicked()
            {
                for &k in g {
                    vals.remove(&specs[k].name);
                }
                changed = true;
            }
            for &k in g {
                let r = ui.selectable_label(on == Some(k), specs[k].title());
                let r = r.on_hover_text(&specs[k].doc);
                if r.clicked() {
                    for &j in g {
                        vals.remove(&specs[j].name);
                    }
                    vals.insert(specs[k].name.clone(), "true".into());
                    changed = true;
                }
            }
        });
    if reset_button(ui, on.is_some()) {
        for &k in g {
            vals.remove(&specs[k].name);
        }
        changed = true;
    }
    ui.end_row();
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, ty: &str, len: Option<usize>) -> ArgSpec {
        ArgSpec {
            name: name.into(),
            ty: ty.into(),
            default: Value::Null,
            unit: String::new(),
            doc: String::new(),
            choices: Vec::new(),
            len,
            repeat: false,
            required: false,
            conflicts: Vec::new(),
            label: None,
            min: None,
        }
    }

    /// 下限のある数（リング数 0 など）は欄で止める（GUX-08）
    #[test]
    fn min_refuses_below() {
        let mut s = spec("rings", "count", None);
        s.min = Some(1.0);
        assert!(s.value("0").is_err());
        assert_eq!(s.value("1").unwrap(), Some(json!(1)));
        assert_eq!(s.value("").unwrap(), None);
    }

    #[test]
    fn step_numbers() {
        assert_eq!(
            step_text("3", &Value::Null, "count", true).as_deref(),
            Some("4")
        );
        assert_eq!(step_text("0", &Value::Null, "count", false), None);
        assert_eq!(
            step_text("", &json!(64), "count", false).as_deref(),
            Some("63")
        );
        assert_eq!(
            step_text("0.25", &Value::Null, "num", true).as_deref(),
            Some("0.26")
        );
        assert_eq!(
            step_text("2", &Value::Null, "num", false).as_deref(),
            Some("1")
        );
        assert_eq!(
            step_text("-1", &Value::Null, "int", false).as_deref(),
            Some("-2")
        );
        assert_eq!(step_text("", &Value::Null, "num", true), None);
        assert_eq!(step_text("a", &Value::Null, "nums", true), None);
    }

    #[test]
    fn exclusive_flags_group() {
        let mut d = spec("direct", "flag", None);
        d.conflicts = vec!["fft".into(), "interp".into()];
        let mut f = spec("fft", "flag", None);
        f.conflicts = vec!["interp".into()];
        let i = spec("interp", "flag", None);
        let e = spec("each_wave", "flag", None);
        let n = spec("n", "count", None);
        let v = [&n, &d, &f, &i, &e];
        assert_eq!(exclusive_groups(&v), vec![vec![1, 2, 3]]);
    }

    #[test]
    fn heads_and_units() {
        assert_eq!(
            doc_head("ガウス求積のリング数").as_deref(),
            Some("ガウス求積のリング数")
        );
        assert_eq!(
            doc_head("周波数 [cycles/mm、アフォーカル像空間では…]").as_deref(),
            Some("周波数")
        );
        assert_eq!(
            doc_head("瞳の重なりの求積の点の数の目安を長く書いた説明の文"),
            None
        );
        assert_eq!(
            doc_unit("周波数 [cycles/mm、アフォーカル像空間では…]").as_deref(),
            Some("cycles/mm")
        );
        assert_eq!(
            doc_unit("範囲 ±range [レンズ単位]").as_deref(),
            Some("レンズ単位")
        );
        assert_eq!(doc_unit("説明"), None);
    }

    /// 同梱の解析の引数はどれも短い名前があるか、説明の頭が短い
    #[test]
    fn analysis_args_have_titles() {
        for (cmd, _, _) in crate::layout::ANALYSES {
            for s in analysis_specs(cmd) {
                if s.name == "save" {
                    continue;
                }
                assert!(
                    s.label.is_some() || doc_head(&s.doc).is_some(),
                    "{cmd} {}: {}",
                    s.name,
                    s.doc
                );
            }
        }
    }

    #[test]
    fn values_by_type() {
        assert_eq!(
            spec("a", "num", None).value(" 2.5 ").unwrap(),
            Some(json!(2.5))
        );
        assert_eq!(spec("a", "count", None).value("3").unwrap(), Some(json!(3)));
        assert!(spec("a", "count", None).value("-1").is_err());
        assert!(spec("a", "num", None).value("x").is_err());
        assert_eq!(spec("a", "num", None).value("").unwrap(), None);
        assert_eq!(
            spec("a", "nums", None).value("10, 20.5 40").unwrap(),
            Some(json!([10, 20.5, 40]))
        );
        // 2 個の並びは 1 個で x・y 共通
        assert_eq!(
            spec("a", "nums", Some(2)).value("0.2").unwrap(),
            Some(json!([0.2, 0.2]))
        );
        assert!(spec("a", "nums", Some(2)).value("1,2,3").is_err());
        assert_eq!(
            spec("a", "flag", None).value("true").unwrap(),
            Some(json!(true))
        );
        assert_eq!(spec("a", "str", None).value("x").unwrap(), Some(json!("x")));
    }

    #[test]
    fn map_skips_unknown_and_false() {
        let specs = vec![spec("n", "count", None), spec("f", "flag", None)];
        let vals: BTreeMap<String, String> = [("n", "4"), ("f", "false"), ("old", "1")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let m = to_map(&specs, &vals).unwrap();
        assert_eq!(Value::Object(m), json!({ "n": 4 }));
    }

    #[test]
    fn nest_and_flatten_round_trip() {
        let mut m = Map::new();
        m.insert("mla.pitch".into(), json!([0.2, 0.2]));
        m.insert("detector.noise.shot".into(), json!(true));
        m.insert("reference".into(), json!("lenslet"));
        let v = nest(&m);
        assert_eq!(
            v,
            json!({ "mla": { "pitch": [0.2, 0.2] }, "detector": { "noise": { "shot": true } }, "reference": "lenslet" })
        );
        let f = flatten(&v);
        assert_eq!(f["mla.pitch"], "0.2, 0.2");
        assert_eq!(f["detector.noise.shot"], "true");
        assert_eq!(f["reference"], "lenslet");
    }
}
