//! カタログの「未完の品」（厚さ・径の足りない品）を補う。
//!
//! 未完の品（[`crate::UserItem::is_incomplete`]）は blocks を持たず、分かっている仕様・品ページの URL・
//! 足りない値の名前（`missing`）だけを持つ。ユーザーが品を選んだときに品ページを 1 件だけ開き、
//! - 仕様表の文字（`Thickness (mm): 3.00` など）を [`read_page`] で読むか、
//! - CAD 欄から落とした STEP を [`read_step`] で測り、
//!
//! ユーザーが確かめた値で [`complete`] が普通の品（blocks つき）を作る。作った品はユーザー側のカタログ
//! （補足のカタログ）に保存し、次からは普通の品として並べる（元のカタログは書き換えない）。

use crate::UserItem;
use serde_json::{json, Value};

/// 補える値の名前
pub const FILLABLE: [&str; 2] = ["thickness", "diameter"];

/// 仕様表から読んだ 1 欄
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// 値の名前（`thickness`・`diameter`、枠の `mount_thickness`・`mount_diameter`・`clear_aperture`）
    pub name: &'static str,
    /// ページの欄の名前（`Thickness (mm)` など）
    pub label: String,
    /// 欄の値の文字
    pub raw: String,
    /// 読めた値 [mm]（径は数か [幅, 高さ]）。値が 2 つ以上並ぶなど決められないときは None
    pub value: Option<Value>,
}

/// 仕様表の読み取りの結果
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageRead {
    /// 厚さ [mm]
    pub thickness: Option<f64>,
    /// 径 [mm]（数か [幅, 高さ]）
    pub diameter: Option<Value>,
    /// 枠付きの品の枠の厚さ・外径、有効径 [mm]（[`page_frame`]）
    pub mount_thickness: Option<f64>,
    pub mount_diameter: Option<f64>,
    pub clear_aperture: Option<f64>,
    /// 見つけた欄（値の読めなかった欄も。ユーザーに見せる）
    pub found: Vec<Reading>,
}

impl PageRead {
    /// 名前の値（`thickness`・`diameter`）
    pub fn get(&self, name: &str) -> Option<Value> {
        match name {
            "thickness" => self.thickness.map(|x| json!(x)),
            "diameter" => self.diameter.clone(),
            "mount_thickness" => self.mount_thickness.map(|x| json!(x)),
            "mount_diameter" => self.mount_diameter.map(|x| json!(x)),
            "clear_aperture" => self.clear_aperture.map(|x| json!(x)),
            _ => None,
        }
    }
}

/// ページの文字のうち見る長さの上限（大きなページでも止まらない）
const MAX_TEXT: usize = 4 << 20;

/// 欄の名前（小文字・末尾の `:` を除いたもの）が、どの値の欄か。優先度（小さいほど先）も返す
fn label_kind(label: &str) -> Option<(&'static str, u8)> {
    let l = label
        .trim()
        .trim_end_matches(':')
        .trim()
        .to_ascii_lowercase();
    // 欄の名前は短い（長い行は説明の文）
    if l.is_empty() || l.len() > 48 {
        return None;
    }
    // 枠（マウント・ハウジング）の厚さ・外径と有効径
    let head0 = l.split('(').next().unwrap_or("").trim();
    let frame = match head0 {
        "mount thickness" | "housing thickness" | "mounted thickness" | "mount thickness t" => {
            Some(("mount_thickness", 0))
        }
        "mount diameter"
        | "mount outer diameter"
        | "housing diameter"
        | "mounted diameter"
        | "housing outer diameter"
        | "mount od" => Some(("mount_diameter", 0)),
        "clear aperture" | "clear aperture ca" | "ca" => Some(("clear_aperture", 0)),
        _ => None,
    };
    if let Some(f) = frame {
        return (head0.len() == l.len() || l[head0.len()..].trim_start().starts_with('('))
            .then_some(f);
    }
    let bad = [
        "mount",
        "housing",
        "tolerance",
        "cell",
        "edge thickness",
        "total",
        "frame",
        "clear aperture",
        "beam",
        "minor",
        "major",
        "wedge",
        "coating",
        "ring",
        "thread",
    ];
    if bad.iter().any(|b| l.contains(b)) {
        return None;
    }
    let head = l.split('(').next().unwrap_or("").trim();
    let t = match head {
        "thickness" | "substrate thickness" | "thickness t" => Some(("thickness", 0)),
        "center thickness" | "center thickness ct" | "ct" => Some(("thickness", 1)),
        "diameter" | "outer diameter" | "diameter d" | "outside diameter" => Some(("diameter", 0)),
        "dimensions" | "size" | "dimension" => Some(("diameter", 1)),
        _ => None,
    };
    // 単位の書かれた欄か、名前だけの欄（`Thickness:`）
    t.filter(|_| head.len() == l.len() || l[head.len()..].trim_start().starts_with('('))
}

/// 欄の単位の倍率（mm へ）。欄の名前の `(mm)`・`(in)`、無ければ値の単位
fn unit_scale(label: &str, raw: &str) -> f64 {
    let l = label.to_ascii_lowercase();
    if l.contains("(mm)") || l.contains("mm)") {
        return 1.0;
    }
    if l.contains("(in") || l.contains("inch") {
        return 25.4;
    }
    let r = raw.to_ascii_lowercase();
    if r.contains("mm") {
        1.0
    } else if r.contains('"') || r.contains("inch") || inch_after_number(&r) {
        25.4
    } else {
        1.0
    }
}

/// 数の直後の `in`（`0.25 in`・`0.25in`）。`3.0 including` のような語の頭は数えない
fn inch_after_number(r: &str) -> bool {
    let b = r.as_bytes();
    numbers(r).iter().any(|&(st, _)| {
        let mut i = st;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
            i += 1;
        }
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        b[i..].starts_with(b"in") && b.get(i + 2).is_none_or(|c| !c.is_ascii_alphabetic())
    })
}

/// 文字の中の数（符号のない小数）を順に
fn numbers(s: &str) -> Vec<(usize, f64)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() || (b[i] == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit()) {
            let st = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if let Ok(x) = s[st..i].trim_end_matches('.').parse::<f64>() {
                out.push((st, x));
            }
        } else {
            i += 1;
        }
    }
    out
}

/// 欄の値の文字から値 [mm] を読む。許容差（`±0.1`・`+0.00/-0.10`）は捨て、公称値が 2 つ以上
/// （`2.00 & 3.00`）なら決めない。径の欄は `25 x 25` を [幅, 高さ] に
fn parse_value(name: &str, label: &str, raw: &str) -> Option<Value> {
    let k = unit_scale(label, raw);
    // 許容差の始まり（±・+・-）より前だけを公称値として見る。`x`・`×` で並ぶのは寸法
    let cut = raw
        .char_indices()
        .find(|&(_, c)| matches!(c, '±' | '+' | '-' | '−' | '('))
        .map_or(raw.len(), |(i, _)| i);
    let nominal = &raw[..cut];
    let nums = numbers(nominal);
    if nums.is_empty() {
        return None;
    }
    let lower = nominal.to_ascii_lowercase();
    let is_dims = lower.contains(" x ") || lower.contains('×') || lower.contains("x");
    let several = nums.len() > 1;
    if several && !(name == "diameter" && is_dims && nums.len() == 2) {
        return None;
    }
    if name == "diameter" && nums.len() == 2 {
        let (a, b) = (nums[0].1 * k, nums[1].1 * k);
        if (a - b).abs() < 1e-9 {
            return Some(json!(round6(a)));
        }
        return Some(json!([round6(a), round6(b)]));
    }
    let x = nums[0].1 * k;
    (x > 0.0 && x.is_finite()).then(|| json!(round6(x)))
}

fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// 品ページの文字（ブラウザの `document.body.innerText`、またはページを選んで写した文字）から厚さ・径を読む。
/// 欄の名前の行に値が無ければ次の空でない行を値とみる。欄が無いページでは何も返さない（止まらない）
pub fn read_page(text: &str) -> PageRead {
    let text = if text.len() > MAX_TEXT {
        let mut e = MAX_TEXT;
        while !text.is_char_boundary(e) {
            e -= 1;
        }
        &text[..e]
    } else {
        text
    };
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut out = PageRead::default();
    let mut best: [Option<(u8, Value)>; 5] = Default::default();
    for (i, line) in lines.iter().enumerate() {
        // `欄: 値`・`欄\t値`・`欄`（値は次の行）
        let (label, rest) = match line.find(':') {
            Some(c) => (&line[..c], line[c + 1..].trim()),
            None => match line.find('\t') {
                Some(c) => (&line[..c], line[c + 1..].trim()),
                None => (*line, ""),
            },
        };
        let Some((name, prio)) = label_kind(label) else {
            continue;
        };
        // 名前だけの行（`Thickness`）は、表の欄として `:` かタブがあるか次の行が値のときだけ
        let raw = if rest.is_empty() {
            match lines[i + 1..].iter().find(|x| !x.is_empty()) {
                Some(n) if label_kind(n.split(':').next().unwrap_or("")).is_none() => *n,
                _ => continue,
            }
        } else {
            rest
        };
        if raw.len() > 80 || numbers(raw).is_empty() {
            continue;
        }
        // 有効径の「90%」などは寸法でない
        let value = if raw.contains('%') {
            None
        } else {
            parse_value(name, label, raw)
        };
        let slot = match name {
            "thickness" => 0,
            "diameter" => 1,
            "mount_thickness" => 2,
            "mount_diameter" => 3,
            _ => 4,
        };
        if let Some(v) = &value {
            if best[slot].as_ref().is_none_or(|(p, _)| prio < *p) {
                best[slot] = Some((prio, v.clone()));
            }
        }
        out.found.push(Reading {
            name,
            label: label.trim().to_string(),
            raw: raw.to_string(),
            value,
        });
    }
    out.thickness = best[0].take().and_then(|(_, v)| v.as_f64());
    out.diameter = best[1].take().map(|(_, v)| v);
    out.mount_thickness = best[2].take().and_then(|(_, v)| v.as_f64());
    out.mount_diameter = best[3].take().and_then(|(_, v)| v.as_f64());
    out.clear_aperture = best[4].take().and_then(|(_, v)| v.as_f64());
    out
}

/// 仕様表の枠の厚さ・外径（無ければ径の欄）・有効径から、枠付きの品の簡単な枠（長さ全体で有効径の穴）。
/// 座の位置は仕様表から分からないので、厚さ `thickness` のガラスを枠の中央に置く。ガラスの径 `glass` が
/// 分かれば保持径にする。値が足りない・外径が有効径以下なら None
pub fn page_frame(
    r: &PageRead,
    thickness: Option<f64>,
    glass: Option<f64>,
) -> Option<crate::MechSpec> {
    let len = r.mount_thickness.filter(|x| *x > 0.0)?;
    let ca = r.clear_aperture.filter(|x| *x > 0.0)?;
    let od = r
        .mount_diameter
        .or_else(|| r.diameter.as_ref().and_then(Value::as_f64))
        .filter(|x| *x > ca)?;
    let seat = thickness
        .filter(|t| *t > 0.0 && *t <= len)
        .map_or(0.0, |t| round6((len - t) / 2.0));
    Some(crate::MechSpec {
        bores: vec![crate::BoreSpec {
            z0: 0.0,
            z1: len,
            diameter: ca,
        }],
        length: len,
        outer_diameter: od,
        holds: Some(crate::HoldSpec {
            diameter: glass.filter(|g| *g >= ca && *g <= od).unwrap_or(ca),
            seat_z: seat,
        }),
        ..Default::default()
    })
}

/// 仕様表の値（STEP で分けた寸法と突き合わせる [`spec_check`]）[mm]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpecValues {
    pub thickness: Option<f64>,
    pub diameter: Option<f64>,
    pub mount_thickness: Option<f64>,
    pub mount_diameter: Option<f64>,
    pub clear_aperture: Option<f64>,
}

impl SpecValues {
    /// 読んだ仕様表の値を先に、無い値は品の `specs` から
    pub fn of(specs: &serde_json::Map<String, Value>, page: Option<&PageRead>) -> Self {
        let get = |k: &str| {
            page.and_then(|p| p.get(k))
                .or_else(|| specs.get(k).cloned())
                .and_then(|v| v.as_f64())
                .filter(|x| *x > 0.0 && x.is_finite())
        };
        SpecValues {
            thickness: get("thickness"),
            diameter: get("diameter"),
            mount_thickness: get("mount_thickness"),
            mount_diameter: get("mount_diameter"),
            clear_aperture: get("clear_aperture"),
        }
    }

    /// serve の params（`{"thickness": …, …}`）から
    pub fn from_value(v: &Value) -> Self {
        let m = v.as_object().cloned().unwrap_or_default();
        Self::of(&m, None)
    }

    pub fn to_value(&self) -> Value {
        let mut m = serde_json::Map::new();
        for (k, v) in [
            ("thickness", self.thickness),
            ("diameter", self.diameter),
            ("mount_thickness", self.mount_thickness),
            ("mount_diameter", self.mount_diameter),
            ("clear_aperture", self.clear_aperture),
        ] {
            if let Some(v) = v {
                m.insert(k.into(), json!(v));
            }
        }
        Value::Object(m)
    }

    /// 枠付きの品で、仕様の厚さ・径が枠の値か（枠の厚さ・外径の欄が無く、枠の長さ・外径と合う）。
    /// Thorlabs のマウント付きフィルタなどは「厚さ」「径」が枠の値
    fn frame_valued(&self, frame: Option<&crate::MechSpec>) -> (bool, bool) {
        let Some(f) = frame else {
            return (false, false);
        };
        let near = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() <= SPEC_TOL);
        (
            self.mount_thickness.is_none() && near(self.thickness, f.length),
            self.mount_diameter.is_none() && near(self.diameter, f.outer_diameter),
        )
    }
}

/// STEP と仕様表の値が同じとみなす差 [mm]（図面の寸法の丸め程度）
pub const SPEC_TOL: f64 = 0.05;

/// STEP の値と仕様表の値の突き合わせ 1 項目
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpecCmp {
    /// 項目（`thickness`・`diameter` はガラス、`mount_thickness`・`mount_diameter`・`clear_aperture` は枠）
    pub key: &'static str,
    pub step: f64,
    pub spec: f64,
    /// 仕様の値の欄（枠の値を「厚さ」「径」の欄に書く品では `thickness`・`diameter`）
    pub spec_key: &'static str,
}

impl SpecCmp {
    /// 合うか。有効径は品質を保証する範囲で、枠の穴（機構の開口）より小さいのが普通なので、仕様の値が穴より
    /// 大きい時だけ食い違いとする
    pub fn ok(&self) -> bool {
        if self.key == "clear_aperture" {
            self.spec <= self.step + SPEC_TOL
        } else {
            (self.step - self.spec).abs() <= SPEC_TOL
        }
    }

    /// 項目の名前（ユーザーに見せる）
    pub fn label(&self) -> &'static str {
        match self.key {
            "thickness" => "ガラスの厚さ",
            "diameter" => "ガラスの径",
            "mount_thickness" => "枠の厚さ",
            "mount_diameter" => "枠の外径",
            _ => "有効径",
        }
    }

    /// 食い違いの説明（保存する品の notes に書く）
    pub fn note(&self) -> String {
        format!(
            "{}: STEP {} mm、仕様表 {} mm（差 {} mm）",
            self.label(),
            round6(self.step),
            round6(self.spec),
            round6(self.step - self.spec)
        )
    }
}

/// STEP で測ったガラス（厚さ・径）と枠を、仕様表の値と突き合わせる。比べられる項目だけを返す。
/// 枠付きの品で仕様の「厚さ」「径」が枠の長さ・外径と合えば、それらは枠の値として比べる。
/// ガラスと枠が 1 つの立体（`merged`）のガラスの径は見えている径なので、仕様の径とは比べない
pub fn spec_check(
    thickness: Option<f64>,
    diameter: Option<f64>,
    frame: Option<&crate::MechSpec>,
    merged: bool,
    s: &SpecValues,
) -> Vec<SpecCmp> {
    let (ft, fd) = s.frame_valued(frame);
    let mut out = Vec::new();
    let mut push = |key, step: Option<f64>, spec: Option<f64>, spec_key| {
        if let (Some(step), Some(spec)) = (step, spec) {
            out.push(SpecCmp {
                key,
                step,
                spec,
                spec_key,
            });
        }
    };
    if !ft {
        push("thickness", thickness, s.thickness, "thickness");
    }
    if !fd && !merged {
        push("diameter", diameter, s.diameter, "diameter");
    }
    if let Some(f) = frame {
        let (mt, mtk) = if ft {
            (s.thickness, "thickness")
        } else {
            (s.mount_thickness, "mount_thickness")
        };
        push("mount_thickness", Some(f.length), mt, mtk);
        let (md, mdk) = if fd {
            (s.diameter, "diameter")
        } else {
            (s.mount_diameter, "mount_diameter")
        };
        push("mount_diameter", Some(f.outer_diameter), md, mdk);
        let clear = f
            .bores
            .iter()
            .map(|b| b.diameter)
            .fold(f64::INFINITY, f64::min);
        push(
            "clear_aperture",
            clear.is_finite().then_some(clear),
            s.clear_aperture,
            "clear_aperture",
        );
    }
    out
}

/// ガラスと枠が 1 つの立体の STEP で、STEP から分からないガラスの本当の径を仕様表で補う。
/// 仕様の径が枠の値でなく、見えている径以上で枠の外径より小さければその値
pub fn spec_glass_diameter(visible: f64, frame: &crate::MechSpec, s: &SpecValues) -> Option<f64> {
    let (_, fd) = s.frame_valued(Some(frame));
    s.diameter
        .filter(|_| !fd)
        .filter(|d| *d >= visible - SPEC_TOL && *d < frame.outer_diameter)
}

/// STEP から測った大きさ
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct StepSize {
    /// 厚さ [mm]（円の軸の向きの長さ。円が無ければ箱の最も短い辺）
    pub thickness: f64,
    /// 径 [mm]（円の最大の直径。円が無ければ箱の残りの 2 辺 [幅, 高さ]）
    pub diameter: Value,
    /// 長さの単位の倍率（mm へ）
    pub unit_mm: f64,
    /// 測り方の説明
    pub note: String,
    /// 枠付きの品の枠（機構部品の寸法、z は枠の端から）。ガラスは `thickness`・`diameter`
    pub frame: Option<crate::MechSpec>,
    /// ガラスと枠が 1 つの立体で、ガラスの径は見えている径（本当の径は仕様表で補う。[`spec_glass_diameter`]）
    pub merged: bool,
    /// ガラスの無い機構部品（チューブ・ケージ板・マウント）。厚さ・径は枠の長さ・外径で、寸法は `frame`
    pub mech: bool,
    /// 機構部品の付属の止め輪（SM1RR など）。素子に合わせて動かすので `frame` に含めない。z は枠の端から
    pub retaining_rings: Vec<RetainingRing>,
}

/// 機構部品の付属の止め輪 [mm]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct RetainingRing {
    pub z0: f64,
    pub z1: f64,
    pub inner_diameter: f64,
    pub outer_diameter: f64,
}

/// 品の簡易外形（カタログの `mech`）と、取った STEP から測った寸法の食い違い（長さ・外径・外形の形）。
/// 食い違いの説明の並び（空なら合う）。差は [`SPEC_TOL`] まで許す
pub fn cad_check(mech: &crate::MechSpec, s: &StepSize) -> Vec<String> {
    let mut v = Vec::new();
    let Some(f) = s.frame.as_ref().filter(|_| s.mech) else {
        return vec!["STEP から機構部品の寸法を測れない（ガラスがある・穴が無いなど）".into()];
    };
    for (label, cat, step) in [
        ("長さ", mech.length, f.length),
        ("外径", mech.outer_diameter, f.outer_diameter),
    ] {
        if cat > 0.0 && (cat - step).abs() > SPEC_TOL {
            v.push(format!(
                "{label}: カタログ {cat} mm、STEP {step} mm（差 {} mm）",
                round6(step - cat)
            ));
        }
    }
    if mech.outline != f.outline {
        let n = |o: crate::MechOutline| if o.is_round() { "丸" } else { "角" };
        v.push(format!(
            "外形: カタログ {}、STEP {}",
            n(mech.outline),
            n(f.outline)
        ));
    }
    v
}

/// 取った品の STEP のキャッシュのフォルダ（[`set_cad_cache_dir`] で決めたもの。無ければ環境変数
/// `LENSBENCH_CAD_CACHE`、なければ OS の設定の場所: Windows は `%APPDATA%\lensbench\cad`、ほかは
/// `$XDG_CACHE_HOME/lensbench/cad` か `~/.cache/lensbench/cad`）
pub fn cad_cache_dir() -> std::path::PathBuf {
    use std::path::PathBuf;
    if let Some(d) = CAD_CACHE_DIR.get() {
        return d.clone();
    }
    // Web 版（ファイルシステムが無い）: 置き場（vfs）の /cad
    if cfg!(target_arch = "wasm32") {
        return PathBuf::from("/cad");
    }
    if let Some(d) = std::env::var_os("LENSBENCH_CAD_CACHE").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        env("APPDATA")
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache")))
    };
    base.unwrap_or_else(std::env::temp_dir)
        .join("lensbench")
        .join("cad")
}

static CAD_CACHE_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// キャッシュのフォルダを決める（GUI の起動時に設定のフォルダの `cad`。1 回だけ）
pub fn set_cad_cache_dir(d: std::path::PathBuf) {
    let _ = CAD_CACHE_DIR.set(d);
}

/// キャッシュにある品の STEP（gzip の [`cad_cache_path`]、無ければ以前の形の `.step`）。どちらも無ければ None
pub fn cad_cache_find(dir: &std::path::Path, item: &str, url: &str) -> Option<std::path::PathBuf> {
    let p = cad_cache_path(dir, item, url);
    if lensbench_core::vfs::is_file(&p) {
        return Some(p);
    }
    let old = p.with_extension("");
    lensbench_core::vfs::is_file(&old).then_some(old)
}

/// キャッシュのファイル: `<品の名前（ファイル名に使えない文字は _）>-<URL の FNV-1a 64 の 16 進>.step.gz`（gzip の STEP）。
/// 共有の置き場の鍵は `.gz` を除いた名前
pub fn cad_cache_path(dir: &std::path::Path, item: &str, url: &str) -> std::path::PathBuf {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.trim().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let name: String = item
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.+".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    dir.join(format!("{name}-{h:016x}.step.gz"))
}

/// 殻の実体の番号と、STEP の座標から部品の座標 [mm] への変換（3 行 4 列 `[R | t]`）
pub type ShellPose = (u64, [[f64; 4]; 3]);

/// STEP の立体の殻（`CLOSED_SHELL` の実体の番号）ごとの、STEP の座標から部品の座標 [mm] への変換（3 行 4 列 `[R | t]`、
/// 組立の置き方ごとに 1 つ）。部品の座標は [`read_step`] で測った枠・機構部品の寸法の座標（z は枠の端から光軸の向き、
/// x は角の部品の辺に直交）。取った STEP の形を部品の簡易外形と同じ所に描くのに使う
pub fn step_shell_poses(text: &str) -> Result<Vec<ShellPose>, String> {
    let upper = text.to_ascii_uppercase();
    crate::step_geom::shell_poses(&upper, step_unit(&upper))
}

/// STEP の長さの単位（mm へ）
fn step_unit(upper: &str) -> f64 {
    let squeezed: String = upper.chars().filter(|c| !c.is_whitespace()).collect();
    let upper = squeezed.as_str();
    if upper.contains("'INCH'") {
        25.4
    } else if upper.contains("SI_UNIT(.MILLI.,.METRE.)") {
        1.0
    } else if upper.contains("SI_UNIT(.CENTI.,.METRE.)") {
        10.0
    } else if upper.contains("SI_UNIT($,.METRE.)") {
        1000.0
    } else {
        1.0
    }
}

/// STEP（AP203/AP214/AP242 の B-rep）の文字から、品の厚さ・径を測る。
///
/// 立体（`MANIFOLD_SOLID_BREP`）ごとに組立の置き方を当てて共通の座標に置き、最大の円の軸を光軸とする。
/// 光軸と同じ軸の穴（内壁の円柱の面）を持つ立体を枠、持たない立体をガラスとみなし、両方あれば
/// 厚さ・径はガラスの値で、枠は [`StepSize::frame`]（機構部品の寸法）に入れる。ガラスが無く穴のある立体だけなら機構部品（チューブ・
/// ケージ板・マウント）として全体を枠にし（[`StepSize::mech`]）、ねじ（SM05〜SM3 の内径）・ケージのロッドの間隔も読む。枠の分からない品は、
/// 径は軸に平行な円の最大の直径、厚さは頂点と円の中心の軸の向きの広がり。円が無ければ（四角い品）頂点の箱の
/// 最も短い辺を厚さ、残りを [幅, 高さ] とする
pub fn read_step(text: &str) -> Result<StepSize, String> {
    let upper = text.to_ascii_uppercase();
    let k = step_unit(&upper);
    let m = crate::step_geom::measure(&upper, k, round6)?;
    Ok(StepSize {
        thickness: m.thickness,
        diameter: m.diameter,
        unit_mm: k,
        note: m.note,
        frame: m.frame,
        merged: m.merged,
        mech: m.mech,
        retaining_rings: m.retaining_rings,
    })
}

/// 基板の名前から硝材（名前・ガラスのカタログ）。分からない基板は N-BK7 で代用し、代用なら true
pub fn material_glass(material: &str) -> (&'static str, &'static str, bool) {
    let m = material.to_ascii_lowercase();
    let has = |k: &str| m.contains(k);
    if has("fused silica") || has("7980") || has("fused quartz") || has("uvfs") {
        ("F_SILICA", "MISC", false)
    } else if has("n-bk7") || m.trim() == "bk7" || has("bk7") {
        ("N-BK7", "SCHOTT", false)
    } else if has("b270") || has("b 270") {
        ("B270", "SCHOTT", false)
    } else if has("caf2") || has("calcium fluoride") {
        ("CAF2", "MISC", false)
    } else if has("sapphire") {
        ("SAPPHIRE", "INFRARED", false)
    } else if has("baf2") || has("barium fluoride") {
        ("BAF2", "INFRARED", false)
    } else if has("germanium") {
        ("GERMANIUM", "INFRARED", false)
    } else if has("silicon") {
        ("SILICON", "INFRARED", false)
    } else if has("mgf2") || has("magnesium fluoride") {
        ("MGF2", "INFRARED", false)
    } else if has("znse") || has("zinc selenide") {
        ("ZNSE", "INFRARED", false)
    } else {
        ("N-BK7", "SCHOTT", true)
    }
}

/// 値の決め方（補足のメモに書く）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillFrom {
    /// 品ページの仕様表
    Page,
    /// 品ページの CAD（STEP）
    Step,
    /// ユーザーの入力
    Manual,
}

impl FillFrom {
    pub fn parse(s: &str) -> FillFrom {
        match s {
            "page" => FillFrom::Page,
            "step" => FillFrom::Step,
            _ => FillFrom::Manual,
        }
    }
    fn text(self) -> &'static str {
        match self {
            FillFrom::Page => "品ページの仕様表",
            FillFrom::Step => "品ページの STEP",
            FillFrom::Manual => "ユーザーの入力",
        }
    }
}

/// 径の値（数か [幅, 高さ]）の半径（四角は長い辺の半分）
fn semi_of(d: &Value) -> Option<f64> {
    let x = match d {
        Value::Number(n) => n.as_f64()?,
        Value::Array(a) => a.iter().filter_map(Value::as_f64).fold(0.0, f64::max),
        _ => return None,
    };
    (x > 0.0 && x.is_finite()).then_some(x / 2.0)
}

/// 未完の品に足りない値を足して、普通の品（blocks つき）にする。
///
/// `thickness`・`diameter` は足りない値（`missing`）の分が要る（ほかは品の specs のまま）。
/// 鏡は平面鏡、ほかは基板（`specs.material`）の平行平板（分からない基板は N-BK7 で代用し、notes に書く）。
/// `checked` は値を確かめた日（YYYY-MM-DD）。`frame` は枠付きの品の枠（[`StepSize::frame`]・[`page_frame`]）で、
/// 品の `frame` に入れる（有効径 `clear_aperture` が無ければ枠の最小の内径を入れる）
pub fn complete(
    item: &UserItem,
    thickness: Option<f64>,
    diameter: Option<Value>,
    frame: Option<crate::MechSpec>,
    from: FillFrom,
    checked: &str,
) -> Result<UserItem, String> {
    use crate::catalog_spec::ItemKind;
    let mut it = item.clone();
    for m in &item.missing {
        let ok = match m.as_str() {
            "thickness" => thickness.is_some_and(|t| t > 0.0 && t.is_finite()),
            "diameter" => diameter.as_ref().and_then(semi_of).is_some(),
            other => return Err(format!("補えない値: {other}")),
        };
        if !ok {
            return Err(format!("{m} の値が要る"));
        }
    }
    if let Some(t) = thickness.filter(|t| *t > 0.0 && t.is_finite()) {
        it.specs.insert("thickness".into(), json!(t));
    }
    if let Some(d) = diameter.filter(|d| semi_of(d).is_some()) {
        it.specs.insert("diameter".into(), d);
    }
    let t = it
        .specs
        .get("thickness")
        .and_then(Value::as_f64)
        .filter(|t| *t > 0.0);
    let semi = it
        .specs
        .get("diameter")
        .and_then(semi_of)
        .ok_or("径（diameter）が無い")?;
    let material = it
        .specs
        .get("material")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // 四角い品（[幅, 高さ]）は矩形の開口、semi は対角の半分（描く大きさ）
    let rect = match it.specs.get("diameter") {
        Some(Value::Array(a)) if a.len() == 2 => match (a[0].as_f64(), a[1].as_f64()) {
            (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some((w / 2.0, h / 2.0)),
            _ => None,
        },
        _ => None,
    };
    let semi = rect.map_or(semi, |(hx, hy)| hx.hypot(hy));
    let ap = rect.map(|(hx, hy)| json!({ "type": "rect", "hx": hx, "hy": hy }));
    let surf = |mut v: Value| {
        if let Some(a) = &ap {
            v["aperture"] = a.clone();
        }
        v
    };
    let block = if it.kind == Some(ItemKind::Mirror) {
        surf(json!({ "type": "mirror", "semi": semi }))
    } else {
        let t = t.ok_or("厚さ（thickness）が無い")?;
        let (glass, cat, sub) = material_glass(&material);
        if sub {
            it.notes.push(format!(
                "基板「{}」の硝材が分からないので blocks は N-BK7 で代用",
                if material.is_empty() {
                    "なし"
                } else {
                    &material
                }
            ));
        }
        if !it.glass_catalogs.iter().any(|c| c == cat) {
            it.glass_catalogs.push(cat.to_string());
        }
        json!({
            "type": "lens",
            "source": { "surfaces": [
                surf(json!({ "t": t, "glass": glass, "semi": semi })),
                surf(json!({ "semi": semi })),
            ] },
        })
    };
    it.blocks = vec![serde_json::from_value(block).map_err(|e| format!("blocks: {e}"))?];
    if let Some(f) = frame {
        f.validate().map_err(|e| format!("枠: {e}"))?;
        if f.bores.is_empty() {
            return Err("枠: 穴（bores）が無い".into());
        }
        let clear = f
            .bores
            .iter()
            .map(|b| b.diameter)
            .fold(f64::INFINITY, f64::min);
        it.specs
            .entry("clear_aperture".to_string())
            .or_insert_with(|| json!(round6(clear)));
        if let (Some(h), Some(t)) = (f.holds, t) {
            let len = if f.length > 0.0 {
                f.length
            } else {
                f.bores.iter().map(|b| b.z1).fold(0.0, f64::max)
            };
            if h.seat_z < 0.0 || h.seat_z + t > len + 1e-6 {
                it.notes.push(format!(
                    "ガラス（{} 〜 {} mm）が枠の長さ {len} mm からはみ出す",
                    h.seat_z,
                    h.seat_z + t
                ));
            }
        }
        if from == FillFrom::Page {
            it.notes
                .push("枠の座の位置は仕様表から分からないので、ガラスを枠の中央に置いた".into());
        }
        it.notes.push(format!(
            "枠を{}で分けた: 長さ {} mm、外径 {} mm、最小の内径 {} mm",
            from.text(),
            f.length,
            f.outer_diameter,
            round6(clear)
        ));
        it.frame = Some(f);
    }
    let names: Vec<&str> = item
        .missing
        .iter()
        .map(|m| if m == "thickness" { "厚さ" } else { "径" })
        .collect();
    it.notes.push(format!(
        "{}を{}で補った（{checked}）",
        names.join("・"),
        from.text()
    ));
    if from != FillFrom::Manual && !it.url.is_empty() {
        it.source = it.url.clone();
    }
    if !checked.is_empty() {
        it.checked = checked.to_string();
    }
    if it.description.is_empty() {
        let d = match it.specs.get("diameter") {
            Some(Value::Array(a)) => a
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(" × "),
            Some(x) => x.to_string(),
            None => "—".into(),
        };
        it.description = format!(
            "{}{}径 {d} mm、厚さ {} mm",
            it.title,
            if it.title.is_empty() { "" } else { "、" },
            t.map_or("—".into(), |x| format!("{x}")),
        );
    }
    it.missing.clear();
    Ok(it)
}

/// UNIX 時刻（秒）の日付（YYYY-MM-DD、UTC）
pub fn date_of_unix(secs: u64) -> String {
    // Howard Hinnant の civil_from_days
    let z = (secs / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_reads_thickness_and_diameter() {
        // 欄と値が別の行（ブラウザの innerText の表）
        let t = "Physical & Mechanical Properties\nDiameter (mm):\n25.00 +0.00/-0.10\nThickness (mm):\n3.00 ±0.10\nMounted Thickness (mm):\n5.00\nClear Aperture CA (mm):\n22.00\n";
        let r = read_page(t);
        assert_eq!(r.thickness, Some(3.0));
        assert_eq!(r.diameter, Some(json!(25.0)));
        // 同じ行（`欄: 値`・タブ）とインチ
        let r = read_page("Thickness (inches):\t0.25\nDiameter (mm): 12.5");
        assert_eq!(r.thickness, Some(6.35));
        assert_eq!(r.diameter, Some(json!(12.5)));
        // 単位の無い欄: 数の直後の in だけインチ（including などの語は mm のまま）
        assert_eq!(
            read_page("Thickness: 3.0 including AR coating").thickness,
            Some(3.0)
        );
        assert_eq!(read_page("Thickness: 0.25 in").thickness, Some(6.35));
        assert_eq!(read_page("Thickness: 0.25in").thickness, Some(6.35));
        // 四角の寸法
        let r = read_page("Dimensions (mm):\n25.0 x 35.0\nThickness (mm): 1.1");
        assert_eq!(r.diameter, Some(json!([25.0, 35.0])));
        assert_eq!(r.thickness, Some(1.1));
    }

    fn frame(len: f64, od: f64, bore: f64) -> crate::MechSpec {
        crate::MechSpec {
            bores: vec![crate::BoreSpec {
                z0: 0.0,
                z1: len,
                diameter: bore,
            }],
            length: len,
            outer_diameter: od,
            holds: Some(crate::HoldSpec {
                diameter: bore,
                seat_z: 0.5,
            }),
            ..Default::default()
        }
    }

    /// STEP で分けた寸法と仕様表の値の突き合わせ（Edmund の枠付きフィルタの形）
    #[test]
    fn spec_check_compares_glass_and_frame() {
        let r = read_page("Diameter (mm):\n25.00\nSubstrate Thickness (mm):\n2.00\nMount Thickness (mm):\n3.50\nClear Aperture CA (mm):\n21.00\n");
        let s = SpecValues::of(&serde_json::Map::new(), Some(&r));
        let f = frame(3.5, 24.99, 21.209);
        let c = spec_check(Some(2.0), Some(23.9), Some(&f), false, &s);
        let keys: Vec<_> = c.iter().map(|c| (c.key, c.ok())).collect();
        // 径 25 は枠の外径 24.99 と合い、枠の外径の欄が無いので枠の値として比べる
        assert_eq!(
            keys,
            [
                ("thickness", true),
                ("mount_thickness", true),
                ("mount_diameter", true),
                ("clear_aperture", true),
            ]
        );
        assert_eq!(c[2].spec_key, "diameter");
        // 有効径が枠の穴より大きいのは食い違い
        let big = SpecValues {
            clear_aperture: Some(22.0),
            ..s
        };
        let c = spec_check(Some(2.0), Some(23.9), Some(&f), false, &big);
        assert!(!c[3].ok());
        assert!(
            c[3].note().contains("有効径: STEP 21.209 mm、仕様表 22 mm"),
            "{}",
            c[3].note()
        );
        // 品の specs の値も使う（読んだページが先）
        let mut m = serde_json::Map::new();
        m.insert("thickness".into(), json!(1.5));
        m.insert("clear_aperture".into(), json!(21.2));
        let s2 = SpecValues::of(&m, None);
        assert_eq!((s2.thickness, s2.clear_aperture), (Some(1.5), Some(21.2)));
        let c = spec_check(Some(2.0), None, Some(&f), false, &s2);
        assert_eq!(
            c.iter()
                .filter(|c| !c.ok())
                .map(|c| c.key)
                .collect::<Vec<_>>(),
            ["thickness"]
        );
        assert_eq!(SpecValues::from_value(&s.to_value()), s);
    }

    /// ガラスと枠が 1 つの立体（Thorlabs のマウント付きフィルタの形）: 仕様の厚さ・径は枠の値、
    /// ガラスの径は見えている径なので比べない
    #[test]
    fn spec_check_merged_and_glass_diameter() {
        let f = frame(6.3, 25.4, 21.0);
        let s = SpecValues {
            thickness: Some(6.3),
            diameter: Some(25.4),
            clear_aperture: Some(21.0),
            ..Default::default()
        };
        let c = spec_check(Some(5.3), Some(21.0), Some(&f), true, &s);
        assert_eq!(
            c.iter()
                .map(|c| (c.key, c.spec_key, c.ok()))
                .collect::<Vec<_>>(),
            [
                ("mount_thickness", "thickness", true),
                ("mount_diameter", "diameter", true),
                ("clear_aperture", "clear_aperture", true),
            ]
        );
        // 仕様の径が枠の値なら、ガラスの径は補えない
        assert_eq!(spec_glass_diameter(21.0, &f, &s), None);
        // ガラスの径の欄があれば補う
        let s = SpecValues {
            diameter: Some(22.0),
            mount_diameter: Some(25.4),
            ..s
        };
        assert_eq!(spec_glass_diameter(21.0, &f, &s), Some(22.0));
        assert_eq!(
            spec_glass_diameter(
                21.0,
                &f,
                &SpecValues {
                    diameter: Some(26.0),
                    ..s
                }
            ),
            None
        );
        // 枠の無い品は厚さ・径だけ
        let c = spec_check(Some(2.0), Some(25.0), None, false, &s);
        assert_eq!(
            c.iter().map(|c| c.key).collect::<Vec<_>>(),
            ["thickness", "diameter"]
        );
    }

    /// 枠付きの品: 仕様表の枠の厚さ・外径・有効径から簡単な枠（ガラスは中央）
    #[test]
    fn page_frame_from_mount_fields() {
        let t = "Diameter (mm):\n25.00\nSubstrate Thickness (mm):\n2.00\nMount Thickness (mm):\n3.50\nClear Aperture CA (mm):\n21.00\n";
        let r = read_page(t);
        assert_eq!(r.thickness, Some(2.0));
        assert_eq!(
            (r.mount_thickness, r.mount_diameter, r.clear_aperture),
            (Some(3.5), None, Some(21.0))
        );
        let f = page_frame(&r, r.thickness, Some(23.9)).unwrap();
        assert_eq!(f.length, 3.5);
        assert_eq!(f.outer_diameter, 25.0);
        assert_eq!(f.bores.len(), 1);
        assert_eq!((f.bores[0].z1, f.bores[0].diameter), (3.5, 21.0));
        let h = f.holds.unwrap();
        assert_eq!((h.seat_z, h.diameter), (0.75, 23.9));
        // 有効径の「90%」は寸法でない
        assert_eq!(read_page("Clear Aperture (%): 90%").clear_aperture, None);
        assert!(page_frame(&read_page("Mount Thickness (mm): 3.5"), None, None).is_none());
        // 品に枠を入れる
        let it = incomplete("filter", &["thickness"], json!({ "diameter": 25.0 }));
        let c = complete(&it, Some(2.0), None, Some(f), FillFrom::Page, "").unwrap();
        assert_eq!(c.frame.as_ref().unwrap().length, 3.5);
        assert_eq!(c.specs["clear_aperture"], json!(21.0));
        assert!(c.notes.iter().any(|n| n.contains("中央")), "{:?}", c.notes);
        // はみ出すガラスは注記
        let c = complete(
            &it,
            Some(4.0),
            None,
            page_frame(&r, None, None),
            FillFrom::Page,
            "",
        )
        .unwrap();
        assert!(
            c.notes.iter().any(|n| n.contains("はみ出す")),
            "{:?}",
            c.notes
        );
    }

    #[test]
    fn page_without_values_does_not_stop() {
        let r = read_page("");
        assert_eq!(r, PageRead::default());
        let r = read_page("Just a moment...\nVerify you are human\nThickness\n");
        assert!(r.thickness.is_none() && r.diameter.is_none());
        // 公称値が 2 つ並ぶ欄は決めない（見つけた欄としては返す）
        let r = read_page("Substrate Thickness (mm):\n2.00 & 3.00\n");
        assert!(r.thickness.is_none());
        assert_eq!(r.found.len(), 1);
        assert!(r.found[0].value.is_none());
        // 大きなページ（数 MB）でも返る
        let big = "lorem ipsum dolor\n".repeat(400_000) + "Thickness (mm): 2.0\n";
        let r = read_page(&big);
        assert!(r.thickness.is_none() || r.thickness == Some(2.0));
    }

    #[test]
    fn center_thickness_is_second_choice() {
        let r = read_page("Center Thickness CT (mm): 4.0\nThickness (mm): 3.0\n");
        assert_eq!(r.thickness, Some(3.0));
        let r = read_page("Center Thickness CT (mm): 4.0\n");
        assert_eq!(r.thickness, Some(4.0));
    }

    /// 厚さ t・半径 r の円板の B-rep（上下の円と側面の円柱）
    fn disc_step(t: f64, r: f64, unit: &str) -> String {
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n\
#1=CARTESIAN_POINT('',(0.,0.,0.));\n#2=DIRECTION('',(0.,0.,1.));\n#3=DIRECTION('',(1.,0.,0.));\n\
#4=AXIS2_PLACEMENT_3D('',#1,#2,#3);\n#5=CIRCLE('',#4,{r});\n#6=CARTESIAN_POINT('',(0.,0.,{t}));\n\
#7=AXIS2_PLACEMENT_3D('',#6,#2,#3);\n#8=CIRCLE('',#7,{r});\n#9=CYLINDRICAL_SURFACE('',#4,{r});\n\
#10=CARTESIAN_POINT('',({r},0.,0.));\n#11=CARTESIAN_POINT('',({r},0.,{t}));\n\
#12=VERTEX_POINT('',#10);\n#13=VERTEX_POINT('',#11);\n#14=PLANE('',#7);\n\
#20=( LENGTH_UNIT() NAMED_UNIT(*) {unit} );\nENDSEC;\nEND-ISO-10303-21;\n"
        )
    }

    #[test]
    fn step_disc_size() {
        let s = read_step(&disc_step(3.0, 12.5, "SI_UNIT(.MILLI.,.METRE.)")).unwrap();
        assert_eq!(s.thickness, 3.0);
        assert_eq!(s.diameter, json!(25.0));
        assert_eq!(s.unit_mm, 1.0);
        // インチの STEP
        let s = read_step(&disc_step(0.25, 0.5, "CONVERSION_BASED_UNIT('INCH',#21)")).unwrap();
        assert_eq!(s.thickness, 6.35);
        assert_eq!(s.diameter, json!(25.4));
        assert!(read_step("not a step").is_err());
    }

    #[test]
    fn step_box_size() {
        let mut d = String::from("ISO-10303-21;\nDATA;\n");
        let mut n = 1;
        for x in [0.0, 25.0] {
            for y in [0.0, 35.0] {
                for z in [0.0, 1.1] {
                    d += &format!(
                        "#{n}=CARTESIAN_POINT('',({x},{y},{z}));\n#{}=VERTEX_POINT('',#{n});\n",
                        n + 1
                    );
                    n += 2;
                }
            }
        }
        d += "ENDSEC;\n";
        let s = read_step(&d).unwrap();
        assert_eq!(s.thickness, 1.1);
        assert_eq!(s.diameter, json!([25.0, 35.0]));
    }

    fn incomplete(kind: &str, missing: &[&str], specs: Value) -> UserItem {
        serde_json::from_value(json!({
            "kind": kind, "url": "https://example.com/p/1", "title": "Filter 1",
            "missing": missing, "specs": specs,
        }))
        .unwrap()
    }

    #[test]
    fn complete_builds_blocks() {
        let it = incomplete("filter", &["thickness"], json!({ "diameter": 25.0 }));
        assert!(it.is_incomplete());
        assert!(complete(&it, None, None, None, FillFrom::Page, "2026-11-10").is_err());
        let c = complete(&it, Some(3.0), None, None, FillFrom::Page, "2026-11-10").unwrap();
        assert!(!c.is_incomplete() && c.missing.is_empty());
        assert_eq!(c.blocks.len(), 1);
        assert_eq!(c.specs["thickness"], json!(3.0));
        assert_eq!(c.glass_catalogs, vec!["SCHOTT"]);
        assert_eq!(c.source, "https://example.com/p/1");
        assert_eq!(c.checked, "2026-11-10");
        assert!(c.notes.iter().any(|n| n.contains("N-BK7 で代用")));
        let b = serde_json::to_value(&c.blocks[0]).unwrap();
        assert_eq!(b["type"], "lens");
        // 鏡は平面鏡、径だけ
        let m = incomplete(
            "mirror",
            &["diameter"],
            json!({ "material": "Fused Silica" }),
        );
        let c = complete(
            &m,
            None,
            Some(json!(25.4)),
            None,
            FillFrom::Step,
            "2026-11-10",
        )
        .unwrap();
        let b = serde_json::to_value(&c.blocks[0]).unwrap();
        assert_eq!(b["type"], "mirror");
        assert_eq!(b["semi"], json!(12.7));
        // 基板の分かる品
        let w = incomplete(
            "polarizer",
            &["diameter"],
            json!({ "thickness": 2, "material": "Fused Silica (Corning 7980)" }),
        );
        let c = complete(
            &w,
            None,
            Some(json!([10.0, 20.0])),
            None,
            FillFrom::Manual,
            "",
        )
        .unwrap();
        assert_eq!(c.glass_catalogs, vec!["MISC"]);
        // 四角い品は両面に矩形の開口、semi は対角の半分
        let b = serde_json::to_value(&c.blocks[0]).unwrap();
        for q in b["source"]["surfaces"].as_array().unwrap() {
            assert_eq!(q["aperture"]["type"], "rect", "{q}");
            assert_eq!(q["aperture"]["hx"], json!(5.0));
            assert_eq!(q["aperture"]["hy"], json!(10.0));
            assert!((q["semi"].as_f64().unwrap() - 125f64.sqrt()).abs() < 1e-12);
        }
        let c = complete(
            &m,
            None,
            Some(json!([10.0, 20.0])),
            None,
            FillFrom::Manual,
            "",
        )
        .unwrap();
        let b = serde_json::to_value(&c.blocks[0]).unwrap();
        assert_eq!(b["aperture"]["hx"], json!(5.0));
        assert_eq!(b["aperture"]["hy"], json!(10.0));
        assert!(c.notes.iter().all(|n| !n.contains("代用")));
    }

    #[test]
    fn dates() {
        assert_eq!(date_of_unix(0), "1970-01-01");
        assert_eq!(date_of_unix(1_794_268_800), "2026-11-10");
    }
}
