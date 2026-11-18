//! カタログの品の仕様での絞り込み・検索と、ほかのカタログの品へ仕様・リンクを足す（overlay・url_template）。
//!
//! ユーザーカタログ（[`lensbench_project::UserCatalog`]）の品は `kind`・`url`・`specs`・`tags`・出典を持てる。
//! カタログ全体の `url_template` と `overlay` は、同じメーカーのほかのカタログ（ZMF・.zmx の玉など）の品にも
//! 仕様とリンクを足す（元のファイルは書き換えない）。
//!
//! 一覧の 1 行は JSON のオブジェクト（CLI の browse・serve の行と同じ）。絞り込みは行から 1 回作る [`CatalogRow`]
//! （メーカーの鍵・種類・仕様の値の型・検索の文字列）に当てる。

pub use lensbench_project::catalog_spec::{spec_unit, ItemKind, Overlay, SPEC_NAMES};
use serde_json::{json, Map, Value};

/// メーカー名の比べ方（大文字小文字・空白・記号を無視。core と同じ 1 つの型）
pub use lensbench_core::catalog::VendorKey;

/// `*`（任意の文字列）と `?`（1 文字）だけのパターン。大文字小文字は区別しない
pub fn glob_match(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = s.chars().flat_map(char::to_lowercase).collect();
    let (mut i, mut j) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while j < t.len() {
        if i < p.len() && (p[i] == '?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(si) = star {
            i = si + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

/// 一覧の行に、同じメーカーのカタログの `url_template`・`overlay` を当てる。
/// 行にもとからある値（品が自分で持つ url・specs の項目・説明）は残す。overlay はパターンの具体的な順
/// （`*` の無い型番が先、次に長いパターン）に当て、先に当てたものを優先する
pub fn apply_overlays(row: &mut Value, cats: &[&lensbench_project::UserCatalog]) {
    let vendor = VendorKey::new(row["vendor"].as_str().unwrap_or(""));
    let part = row["part"].as_str().unwrap_or("").to_string();
    let Some(obj) = row.as_object_mut() else {
        return;
    };
    for cat in cats {
        if cat.vendor.trim().is_empty() || VendorKey::new(&cat.vendor) != vendor {
            continue;
        }
        let mut pats: Vec<(&String, &Overlay)> = cat
            .overlay
            .iter()
            .filter(|(p, _)| glob_match(p, &part))
            .collect();
        pats.sort_by_key(|(p, _)| (p.contains(['*', '?']), std::cmp::Reverse(p.len())));
        for (_, o) in pats {
            merge_overlay(obj, o);
        }
        if !cat.url_template.is_empty()
            && obj
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty()
        {
            obj.insert(
                "url".into(),
                json!(cat.url_template.replace("{part}", &part)),
            );
        }
    }
}

fn merge_overlay(obj: &mut Map<String, Value>, o: &Overlay) {
    let empty = |obj: &Map<String, Value>, k: &str| {
        obj.get(k).and_then(Value::as_str).unwrap_or("").is_empty()
    };
    if !o.url.is_empty() && empty(obj, "url") {
        obj.insert("url".into(), json!(o.url));
    }
    if !o.description.is_empty() && empty(obj, "description") {
        obj.insert("description".into(), json!(o.description));
    }
    if let Some(k) = o.kind {
        if obj.get("category").is_none_or(Value::is_null) {
            obj.insert("category".into(), json!(k.as_str()));
        }
    }
    if !o.specs.is_empty() {
        let specs = obj.entry("specs").or_insert_with(|| json!({}));
        if let Some(m) = specs.as_object_mut() {
            for (k, v) in &o.specs {
                m.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    if !o.tags.is_empty() {
        let tags = obj.entry("tags").or_insert_with(|| json!([]));
        if let Some(a) = tags.as_array_mut() {
            for t in &o.tags {
                if !a.iter().any(|x| x == t) {
                    a.push(json!(t));
                }
            }
        }
    }
}

/// 仕様の値（読み込みのときに 1 回だけ型を決める）
#[derive(Debug, Clone, PartialEq)]
pub enum SpecValue {
    /// 数（数として読める文字列も）。text は含むで比べるときの書いた形（小文字）
    Num { value: f64, text: String },
    /// 範囲 [下限, 上限]（下限 ≤ 上限に並べ直す）。text は書いた形
    Range { lo: f64, hi: f64, text: String },
    /// 文字列（含むで比べる。小文字にしたもの）
    Text(String),
    /// 範囲でない配列（要素のどれかが含めばよい。要素は小文字の文字列）
    List(Vec<String>),
}

impl SpecValue {
    /// JSON の値から（null は None）
    pub fn of(v: &Value) -> Option<SpecValue> {
        let text = |x: &Value| match x {
            Value::String(t) => t.to_lowercase(),
            other => other.to_string().to_lowercase(),
        };
        Some(match v {
            Value::Null => return None,
            Value::Number(n) => SpecValue::Num {
                value: n.as_f64()?,
                text: text(v),
            },
            Value::Array(a) if a.len() == 2 && a[0].is_number() && a[1].is_number() => {
                let (x, y) = (a[0].as_f64()?, a[1].as_f64()?);
                SpecValue::Range {
                    lo: x.min(y),
                    hi: x.max(y),
                    text: text(v),
                }
            }
            Value::Array(a) => SpecValue::List(a.iter().map(text).collect()),
            Value::String(t) => match t.trim().parse::<f64>() {
                Ok(x) => SpecValue::Num {
                    value: x,
                    text: t.to_lowercase(),
                },
                Err(_) => SpecValue::Text(t.to_lowercase()),
            },
            other => SpecValue::Text(text(other)),
        })
    }
    /// 数で比べるときの範囲（数は [x, x]）
    fn range(&self) -> Option<(f64, f64)> {
        match *self {
            SpecValue::Num { value, .. } => Some((value, value)),
            SpecValue::Range { lo, hi, .. } => Some((lo, hi)),
            _ => None,
        }
    }
    /// 小文字の q を含むか（数は書いた形の文字列で、範囲は要素のどちらかで）
    fn contains(&self, q: &str) -> bool {
        match self {
            SpecValue::Num { text, .. } | SpecValue::Text(text) => text.contains(q),
            SpecValue::Range { text, .. } => serde_json::from_str::<Vec<Value>>(text)
                .map(|a| a.iter().any(|x| x.to_string().contains(q)))
                .unwrap_or(false),
            SpecValue::List(a) => a.iter().any(|t| t.contains(q)),
        }
    }
}

/// 一覧の 1 行の、絞り込みに使う値（行を読んだときに 1 回だけ作る）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogRow {
    /// メーカーの鍵
    pub vendor: VendorKey,
    /// 品の種類（[`row_category`]）
    pub category: String,
    /// 機構部品の保持径 [mm]
    pub hold: Option<f64>,
    /// 仕様: `specs` の中と、行の同じ名前の欄（efl・diameter・thickness など一覧の値。`specs` が先）
    pub specs: std::collections::BTreeMap<String, SpecValue>,
    /// 検索の文字列（メーカー・型番・説明・タグ・ガラス・仕様の文字列を改行でつないで小文字に）
    pub text: String,
}

impl CatalogRow {
    pub fn new(row: &Value) -> CatalogRow {
        let mut specs = std::collections::BTreeMap::new();
        if let Some(m) = row.as_object() {
            for (k, v) in m.iter().filter(|(k, _)| *k != "specs") {
                if let Some(x) = SpecValue::of(v) {
                    specs.insert(k.clone(), x);
                }
            }
        }
        let mut text = String::new();
        for k in ["vendor", "part", "description"] {
            text.push_str(row[k].as_str().unwrap_or(""));
            text.push('\n');
        }
        for k in ["tags", "glasses"] {
            for t in row[k].as_array().into_iter().flatten() {
                text.push_str(t.as_str().unwrap_or(""));
                text.push('\n');
            }
        }
        if let Some(m) = row["specs"].as_object() {
            for (k, v) in m {
                match v {
                    Value::String(t) => text.push_str(t),
                    other => text.push_str(&other.to_string()),
                }
                text.push('\n');
                if let Some(x) = SpecValue::of(v) {
                    specs.insert(k.clone(), x);
                }
            }
        }
        CatalogRow {
            vendor: VendorKey::new(row["vendor"].as_str().unwrap_or("")),
            category: row_category(row).to_string(),
            hold: row["mech"]["holds"]["diameter"].as_f64(),
            specs,
            text: text.to_lowercase(),
        }
    }

    /// 仕様の値（[`CatalogRow::specs`]）
    pub fn spec(&self, name: &str) -> Option<&SpecValue> {
        self.specs.get(name)
    }
}

/// 比べ方
#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Has,
    Eq,
    Ge,
    Le,
    Gt,
    Lt,
}

/// 仕様での絞り込み 1 つ。書き方:
/// - `名前`: その仕様がある
/// - `名前=値`: 数なら一致（相対 1e-9）、範囲 [下限, 上限] の仕様なら値を含む、文字列なら含む（大文字小文字は区別しない）。
///   値が数でも、仕様が数で比べられない文字列（"50:50" など）なら文字列の含むで見る
/// - 名前と値は最初に現れる演算子で割る（`damage_threshold=>10 J/cm2` は `=` で割り、値は `>10 J/cm2`）
/// - `名前=a..b`: 数なら a〜b の中、範囲の仕様なら [a, b] を全部含む
/// - `名前>=値`・`<=`・`>`・`<`: 数で比べる。範囲の仕様は重なりで（`>=v` は上限が v 以上、`<=v` は下限が v 以下）
#[derive(Debug, Clone, PartialEq)]
pub struct SpecFilter {
    pub name: String,
    op: Op,
    text: String,
    lo: f64,
    hi: f64,
}

impl SpecFilter {
    pub fn parse(s: &str) -> Result<SpecFilter, String> {
        let s = s.trim();
        // 最初に現れる演算子で割る（値に < > = を含む文字列のため）。同じ位置なら 2 文字の方
        let (name, op, rest) = [
            (">=", Op::Ge),
            ("<=", Op::Le),
            (">", Op::Gt),
            ("<", Op::Lt),
            ("=", Op::Eq),
        ]
        .into_iter()
        .filter_map(|(t, op)| s.find(t).map(|i| (i, t, op)))
        .min_by_key(|&(i, t, _)| (i, std::cmp::Reverse(t.len())))
        .map(|(i, t, op)| (&s[..i], op, &s[i + t.len()..]))
        .unwrap_or((s, Op::Has, ""));
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(format!("spec condition {s:?}: no name"));
        }
        let text = rest.trim().to_string();
        let num = |x: &str| {
            x.trim()
                .parse::<f64>()
                .map_err(|_| format!("spec condition {s:?}: {x:?} is not a number"))
        };
        let (lo, hi) = match op {
            Op::Has => (f64::NAN, f64::NAN),
            Op::Eq => match text.split_once("..") {
                Some((a, b)) => {
                    let (a, b) = (num(a)?, num(b)?);
                    (a.min(b), a.max(b))
                }
                None => match text.parse::<f64>() {
                    Ok(x) => (x, x),
                    Err(_) => (f64::NAN, f64::NAN),
                },
            },
            _ => {
                let x = num(&text)?;
                (x, x)
            }
        };
        Ok(SpecFilter {
            name,
            op,
            text,
            lo,
            hi,
        })
    }

    /// 行がこの条件に合うか
    pub fn matches(&self, row: &CatalogRow) -> bool {
        let Some(v) = row.spec(&self.name) else {
            return false;
        };
        if self.op == Op::Has {
            return true;
        }
        let range = v.range();
        let contains = || v.contains(&self.text.to_lowercase());
        match self.op {
            Op::Eq if self.lo.is_nan() => contains(),
            Op::Eq => {
                // 数で比べられない仕様（"50:50" など）は、範囲でない条件なら文字列の含むに落とす
                let Some((a, b)) = range else {
                    return self.lo == self.hi && contains();
                };
                if a == b && self.lo != self.hi {
                    // 数の仕様が条件の範囲の中
                    a >= self.lo && a <= self.hi
                } else if a == b {
                    (a - self.lo).abs() <= 1e-9 * a.abs().max(self.lo.abs()).max(1e-300)
                } else {
                    a <= self.lo && b >= self.hi
                }
            }
            Op::Ge => range.is_some_and(|(_, b)| b >= self.lo),
            Op::Gt => range.is_some_and(|(_, b)| b > self.lo),
            Op::Le => range.is_some_and(|(a, _)| a <= self.lo),
            Op::Lt => range.is_some_and(|(a, _)| a < self.lo),
            Op::Has => true,
        }
    }
}

/// 一覧の絞り込み（CLI catalog-search・serve の catalog_search）
#[derive(Debug, Clone, Default)]
pub struct CatalogQuery {
    /// 品の種類（空なら全部）
    pub kinds: Vec<ItemKind>,
    /// メーカー（含む、大文字小文字・記号を無視。空なら全部）
    pub vendor: String,
    /// 型番・説明・タグ・仕様の文字列に含む語（空白で区切った語をすべて含む）
    pub text: String,
    pub specs: Vec<SpecFilter>,
    /// 径 [mm] の素子が入る機構部品だけ（保持径が径以上で、[`FIT_TOL`] より太くない）
    pub fits: Option<f64>,
}

/// 素子が機構部品に入るとみなす保持径と径の差の上限 [mm]（これより太い保持径は、配置の確認で「細い」の注意になる）
pub const FIT_TOL: f64 = 0.25;

/// 保持径 `hold` に、径 `d` の素子が入るか（保持径が d 以上、d + FIT_TOL 以下）
pub fn fits(hold: f64, d: f64) -> bool {
    hold >= d - 1e-6 && hold - d <= FIT_TOL + 1e-9
}

/// 一覧の行（機構部品）の保持径に、径 `d` の素子が入るか
pub fn fits_row(row: &Value, d: f64) -> bool {
    row["mech"]["holds"]["diameter"]
        .as_f64()
        .is_some_and(|h| fits(h, d))
}

impl CatalogQuery {
    pub fn matches(&self, row: &CatalogRow) -> bool {
        if let Some(d) = self.fits {
            if !row.hold.is_some_and(|h| fits(h, d)) {
                return false;
            }
        }
        if !self.kinds.is_empty() && !self.kinds.iter().any(|k| k.as_str() == row.category) {
            return false;
        }
        if !self.vendor.trim().is_empty() && !row.vendor.matches(&VendorKey::new(&self.vendor)) {
            return false;
        }
        if !self
            .text
            .split_whitespace()
            .all(|w| row.text.contains(&w.to_lowercase()))
        {
            return false;
        }
        self.specs.iter().all(|f| f.matches(row))
    }
}

/// 行の種類: `category`、無ければブロックの種類（`kind`）
pub fn row_category(row: &Value) -> &str {
    row["category"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| row["kind"].as_str())
        .unwrap_or("lens")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob() {
        assert!(glob_match("LA1*", "la1050-A"));
        assert!(glob_match("*-A", "LA1050-A"));
        assert!(glob_match("BS01?", "BS013"));
        assert!(!glob_match("BS01?", "BS0134"));
        assert!(glob_match("*", ""));
        assert!(!glob_match("AC", "AC254"));
    }

    #[test]
    fn spec_filters() {
        let row = json!({
            "vendor": "LB", "part": "P1", "efl": 100.0,
            "specs": { "wavelength_nm": [400, 700], "extinction_ratio": 1000, "coating": "AR 400-700 nm", "split_ratio": "50:50" }
        });
        let row = CatalogRow::new(&row);
        let ok = |s: &str| SpecFilter::parse(s).unwrap().matches(&row);
        assert!(ok("wavelength_nm=633"));
        assert!(!ok("wavelength_nm=800"));
        assert!(ok("wavelength_nm=450..650"));
        assert!(!ok("wavelength_nm=350..650"));
        assert!(ok("wavelength_nm>=650"));
        assert!(!ok("wavelength_nm>=800"));
        assert!(ok("wavelength_nm<=400"));
        assert!(ok("extinction_ratio>=1000"));
        assert!(!ok("extinction_ratio>1000"));
        assert!(ok("extinction_ratio=1000"));
        assert!(ok("coating=ar 400"));
        assert!(ok("split_ratio=50:50"));
        assert!(ok("efl=90..110"));
        assert!(ok("efl<200"));
        assert!(ok("coating"));
        assert!(!ok("wedge"));
        assert!(SpecFilter::parse("efl>=x").is_err());
        assert!(SpecFilter::parse(">=1").is_err());
        // 数として読める値でも、数で比べられない仕様には文字列の含むで当たる
        assert!(ok("split_ratio=50"));
        assert!(!ok("split_ratio=50..60"));
    }

    #[test]
    fn spec_filter_first_operator() {
        let row =
            json!({ "specs": { "damage_threshold": ">10 J/cm2", "mount": "SM1 <x>", "n": 5 } });
        let row = CatalogRow::new(&row);
        let f = SpecFilter::parse("damage_threshold=>10 J/cm2").unwrap();
        assert_eq!(
            (f.name.as_str(), f.op, f.text.as_str()),
            ("damage_threshold", Op::Eq, ">10 J/cm2")
        );
        assert!(f.matches(&row));
        let f = SpecFilter::parse("mount=SM1 <x>").unwrap();
        assert_eq!((f.name.as_str(), f.op), ("mount", Op::Eq));
        assert!(f.matches(&row));
        // 同じ位置なら 2 文字の演算子
        let f = SpecFilter::parse("n>=5").unwrap();
        assert_eq!((f.op, f.lo), (Op::Ge, 5.0));
        assert!(f.matches(&row));
        assert_eq!(SpecFilter::parse("n<5").unwrap().op, Op::Lt);
    }

    #[test]
    fn overlay_and_template() {
        let mut cat = lensbench_project::UserCatalog {
            vendor: "Edmund Optics".into(),
            url_template: "https://example.com/p/{part}".into(),
            ..Default::default()
        };
        cat.overlay.insert(
            "AB*".into(),
            Overlay {
                specs: [("coating".to_string(), json!("MgF2"))].into(),
                kind: Some(ItemKind::Lens),
                tags: vec!["achromat".into()],
                ..Default::default()
            },
        );
        cat.overlay.insert(
            "AB12".into(),
            Overlay {
                specs: [("coating".to_string(), json!("VIS-NIR"))].into(),
                url: "https://example.com/special".into(),
                ..Default::default()
            },
        );
        let mut row = json!({ "vendor": "EDMUND_OPTICS", "part": "AB12", "kind": "lens" });
        apply_overlays(&mut row, &[&cat]);
        assert_eq!(row["specs"]["coating"], "VIS-NIR");
        assert_eq!(row["url"], "https://example.com/special");
        assert_eq!(row["tags"][0], "achromat");
        let mut row =
            json!({ "vendor": "EDMUND_OPTICS", "part": "AB13", "specs": { "coating": "own" } });
        apply_overlays(&mut row, &[&cat]);
        assert_eq!(row["specs"]["coating"], "own");
        assert_eq!(row["url"], "https://example.com/p/AB13");
        let mut row = json!({ "vendor": "OTHER", "part": "AB13" });
        apply_overlays(&mut row, &[&cat]);
        assert!(row.get("url").is_none());
    }

    #[test]
    fn query() {
        let rows = [
            json!({ "vendor": "LB", "part": "W1", "category": "window", "kind": "lens", "description": "UV fused silica", "specs": { "wavelength_nm": [200, 2000] } }),
            json!({ "vendor": "LB", "part": "M1", "kind": "mirror", "tags": ["dielectric"] }),
            json!({ "vendor": "XY", "part": "L1", "kind": "lens", "glasses": ["N-BK7"] }),
        ];
        let rows: Vec<CatalogRow> = rows.iter().map(CatalogRow::new).collect();
        let n = |q: &CatalogQuery| rows.iter().filter(|r| q.matches(r)).count();
        assert_eq!(n(&CatalogQuery::default()), 3);
        assert_eq!(
            n(&CatalogQuery {
                kinds: vec![ItemKind::Window],
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            n(&CatalogQuery {
                kinds: vec![ItemKind::Lens],
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            n(&CatalogQuery {
                vendor: "lb".into(),
                ..Default::default()
            }),
            2
        );
        assert_eq!(
            n(&CatalogQuery {
                text: "fused silica".into(),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            n(&CatalogQuery {
                text: "dielectric".into(),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            n(&CatalogQuery {
                text: "bk7".into(),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            n(&CatalogQuery {
                specs: vec![SpecFilter::parse("wavelength_nm=1550").unwrap()],
                ..Default::default()
            }),
            1
        );
    }
}
