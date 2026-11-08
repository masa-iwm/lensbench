//! カタログの品の種類・仕様・リンクと、仕様での絞り込み。
//!
//! ユーザーカタログ（[`crate::UserCatalog`]）の品は `kind`・`url`・`specs`・`tags`・出典を持てる。
//! カタログ全体の `url_template` と `overlay` は、同じメーカーのほかのカタログ（ZMF・.zmx の玉など）の品にも
//! 仕様とリンクを足す（元のファイルは書き換えない）。
//!
//! 一覧の 1 行は JSON のオブジェクト（CLI の browse・serve の行と同じ）。[`SpecFilter`] はその行に当てる。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// 品の種類（一覧での分類。ブロックの種類とは別: ウィンドウ・偏光子も今は玉・面の並びのブロックで書ける）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Lens,
    Mirror,
    Splitter,
    Polarizer,
    Waveplate,
    Window,
    Filter,
    Prism,
    /// レンズチューブ（機構）
    Tube,
    /// ケージ板
    #[serde(rename = "cage_plate")]
    CagePlate,
    /// マウント（素子の保持）
    Mount,
    /// ケージのロッド
    Rod,
    /// アダプタ・スペーサ・止め輪など
    Adapter,
    Other,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemKind::Lens => "lens",
            ItemKind::Mirror => "mirror",
            ItemKind::Splitter => "splitter",
            ItemKind::Polarizer => "polarizer",
            ItemKind::Waveplate => "waveplate",
            ItemKind::Window => "window",
            ItemKind::Filter => "filter",
            ItemKind::Prism => "prism",
            ItemKind::Tube => "tube",
            ItemKind::CagePlate => "cage_plate",
            ItemKind::Mount => "mount",
            ItemKind::Rod => "rod",
            ItemKind::Adapter => "adapter",
            ItemKind::Other => "other",
        }
    }
    /// 機構部品か（光学の面を持たず、`mech` の寸法を持つ）
    pub fn is_mech(self) -> bool {
        matches!(
            self,
            ItemKind::Tube
                | ItemKind::CagePlate
                | ItemKind::Mount
                | ItemKind::Rod
                | ItemKind::Adapter
        )
    }
    pub const ALL: [ItemKind; 14] = [
        ItemKind::Lens,
        ItemKind::Mirror,
        ItemKind::Splitter,
        ItemKind::Polarizer,
        ItemKind::Waveplate,
        ItemKind::Window,
        ItemKind::Filter,
        ItemKind::Prism,
        ItemKind::Tube,
        ItemKind::CagePlate,
        ItemKind::Mount,
        ItemKind::Rod,
        ItemKind::Adapter,
        ItemKind::Other,
    ];
    pub fn parse(s: &str) -> Option<ItemKind> {
        ItemKind::ALL
            .into_iter()
            .find(|k| k.as_str().eq_ignore_ascii_case(s.trim()))
    }
}

/// 名前の決まった仕様: (名前, 単位, 説明)。値は数・[下限, 上限]・文字列。ほかの名前も自由に使える
pub const SPEC_NAMES: &[(&str, &str, &str)] = &[
    (
        "wavelength_nm",
        "nm",
        "設計波長域 [下限, 上限]（1 つの波長なら数）",
    ),
    ("diameter", "mm", "外径（四角い品は一辺か [幅, 高さ]）"),
    ("clear_aperture", "mm", "有効径"),
    ("thickness", "mm", "厚さ・キューブの一辺"),
    ("efl", "mm", "焦点距離"),
    ("material", "", "硝材・基板"),
    ("coating", "", "膜（AR・反射膜の名前）"),
    ("reflectance", "%", "反射率（帯域の平均など）"),
    ("transmittance", "%", "透過率"),
    ("split_ratio", "", "分岐比 R:T（例 \"50:50\"）"),
    ("extinction_ratio", "", "消光比（例 1000 は 1000:1）"),
    ("retardance", "波長", "遅延（例 0.25 は λ/4）"),
    ("order", "", "波長板の次数（zero・multi・achromatic など）"),
    ("wedge", "arcmin", "くさび角"),
    (
        "flatness",
        "λ",
        "面精度（λ@633nm の分母でなく λ の値、例 0.1 は λ/10）",
    ),
    ("surface_quality", "", "スクラッチ・ディグ（例 \"40-20\"）"),
    ("damage_threshold", "", "損傷しきい値（条件つきの文字列）"),
    ("aoi_deg", "deg", "入射角"),
    ("mount", "", "取付け（ねじ・マウントの規格）"),
    ("thread", "", "機構のねじ（例 \"SM1\"）"),
    ("length", "mm", "機構の長さ（チューブ・ロッド）"),
    ("inner_diameter", "mm", "機構の内径・保持径"),
    ("rod_spacing", "mm", "ケージのロッドの間隔"),
    ("price", "", "価格（通貨つきの文字列）"),
];

/// 名前の決まった仕様の単位（無い名前は空）
pub fn spec_unit(name: &str) -> &'static str {
    SPEC_NAMES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, u, _)| *u)
        .unwrap_or("")
}

/// ほかのカタログの品へ足す仕様・リンク（`overlay` の値）
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct Overlay {
    /// 品の説明が空のときに使う
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ItemKind>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub specs: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// メーカー名の比べ方: 大文字小文字・空白・記号を無視（"EDMUND OPTICS" と "Edmund_Optics" は同じ）
pub fn vendor_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

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
pub fn apply_overlays(row: &mut Value, cats: &[&crate::UserCatalog]) {
    let vendor = vendor_key(row["vendor"].as_str().unwrap_or(""));
    let part = row["part"].as_str().unwrap_or("").to_string();
    let Some(obj) = row.as_object_mut() else {
        return;
    };
    for cat in cats {
        if cat.vendor.trim().is_empty() || vendor_key(&cat.vendor) != vendor {
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
        if obj.get("category").map_or(true, Value::is_null) {
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

/// 行の仕様の値: `specs` の中、無ければ行の同じ名前の欄（efl・diameter・thickness など一覧の値）
pub fn spec_of<'a>(row: &'a Value, name: &str) -> Option<&'a Value> {
    row["specs"]
        .get(name)
        .or_else(|| row.get(name))
        .filter(|v| !v.is_null())
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
/// - `名前=値`: 数なら一致（相対 1e-9）、範囲 [下限, 上限] の仕様なら値を含む、文字列なら含む（大文字小文字は区別しない）
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
        let (name, op, rest) = [
            (">=", Op::Ge),
            ("<=", Op::Le),
            (">", Op::Gt),
            ("<", Op::Lt),
            ("=", Op::Eq),
        ]
        .into_iter()
        .find_map(|(t, op)| s.split_once(t).map(|(a, b)| (a, op, b)))
        .unwrap_or((s, Op::Has, ""));
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(format!("仕様の条件 {s:?}: 名前が無い"));
        }
        let text = rest.trim().to_string();
        let num = |x: &str| {
            x.trim()
                .parse::<f64>()
                .map_err(|_| format!("仕様の条件 {s:?}: {x:?} が数でない"))
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
    pub fn matches(&self, row: &Value) -> bool {
        let Some(v) = spec_of(row, &self.name) else {
            return false;
        };
        if self.op == Op::Has {
            return true;
        }
        // 値の範囲（数は [x, x]、[a, b] は範囲、文字列の数も数として）
        let range = match v {
            Value::Number(n) => n.as_f64().map(|x| (x, x)),
            Value::Array(a) if a.len() == 2 => match (a[0].as_f64(), a[1].as_f64()) {
                (Some(a), Some(b)) => Some((a.min(b), a.max(b))),
                _ => None,
            },
            Value::String(t) => t.trim().parse::<f64>().ok().map(|x| (x, x)),
            _ => None,
        };
        match self.op {
            Op::Eq if self.lo.is_nan() => {
                // 文字列で含む（配列なら要素のどれか）
                let q = self.text.to_lowercase();
                let has = |x: &Value| match x {
                    Value::String(t) => t.to_lowercase().contains(&q),
                    other => other.to_string().to_lowercase().contains(&q),
                };
                match v {
                    Value::Array(a) => a.iter().any(has),
                    other => has(other),
                }
            }
            Op::Eq => {
                let Some((a, b)) = range else {
                    return false;
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
}

impl CatalogQuery {
    pub fn matches(&self, row: &Value) -> bool {
        if !self.kinds.is_empty() {
            let cat = row_category(row);
            if !self.kinds.iter().any(|k| k.as_str() == cat) {
                return false;
            }
        }
        if !self.vendor.trim().is_empty()
            && !vendor_key(row["vendor"].as_str().unwrap_or("")).contains(&vendor_key(&self.vendor))
        {
            return false;
        }
        if !self.text.trim().is_empty() {
            let mut hay = String::new();
            for k in ["vendor", "part", "description"] {
                hay.push_str(row[k].as_str().unwrap_or(""));
                hay.push('\n');
            }
            for k in ["tags", "glasses"] {
                if let Some(a) = row[k].as_array() {
                    for t in a {
                        hay.push_str(t.as_str().unwrap_or(""));
                        hay.push('\n');
                    }
                }
            }
            if let Some(m) = row["specs"].as_object() {
                for v in m.values() {
                    match v {
                        Value::String(t) => hay.push_str(t),
                        other => hay.push_str(&other.to_string()),
                    }
                    hay.push('\n');
                }
            }
            let hay = hay.to_lowercase();
            if !self
                .text
                .split_whitespace()
                .all(|w| hay.contains(&w.to_lowercase()))
            {
                return false;
            }
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
    }

    #[test]
    fn overlay_and_template() {
        let mut cat = crate::UserCatalog {
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
