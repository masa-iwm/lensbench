//! 面の表（OpticStudio の Lens Data のような表）と、ブロック（玉・群）の階層・ロック。
//!
//! 表は serve の `surface`（全部の面と、玉・群の面の範囲 `units`）から作る。ブロックは玉・群の面の範囲の入れ子で
//! 木にする（読込で作った玉 lens1 … は、名前のあるブロックの中にあれば出さない）。最初は系の全部の面と最上位の
//! ブロックを見せ、ブロックをダブルクリックするとそのブロックに入って中のブロックと面だけを見せる。
//!
//! 折りたたみ表示（[`LensData::tree`]）では、ブロックに入らずにその場で開いて（▶/▼）中の面を表で直接変えられる。
//!
//! 値の変更は [`Act::Edit`] で GUI に返す（開いた系は serve の `edit`、組合せの系は組合せの要素の値と面の上書き）。
//! ロックしたブロック（と、その中のブロック）の面は変えられない。

use std::collections::BTreeSet;

use egui::{Key, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use serde_json::{json, Map, Value};

use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::theme;

/// 表の 1 行（1 つの面）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub typ: String,
    pub comment: String,
    /// 曲率半径（平面は inf）
    pub r: f64,
    pub t: f64,
    pub material: String,
    pub coating: String,
    pub semi: f64,
    /// 半径を光線から求める（自動）
    pub semi_auto: bool,
    pub k: f64,
    pub stop: bool,
    /// 光線が通らない面（無視）
    pub ignore: bool,
    /// 非球面の係数 r², r⁴, …（面タイプのパラメータ）
    pub asph: Vec<f64>,
    /// x の曲率（Toroidal）
    pub cx: f64,
    /// 座標ブレークの偏心 x y・傾き x y z と順序
    pub cb: Vec<f64>,
    pub cb_order: f64,
    /// 焦点距離（Paraxial）
    pub focal: f64,
    /// Jones 行列（実・虚 ×4）
    pub jones: Vec<f64>,
    /// 媒質の屈折率のずれ dn・分散の相対変化 disp（硝材の追加設定）
    pub dn: f64,
    pub disp: f64,
    /// ソルブ（量 c・t、説明）
    pub solves: Vec<(String, String)>,
    /// ピックアップ（量 c r t k pN semi …、説明）
    pub pickups: Vec<(String, String)>,
}

impl Row {
    pub fn is_cb(&self) -> bool {
        self.typ == "COORDBRK"
    }
}

/// 玉・群（serve の units）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Unit {
    pub name: String,
    pub spans: Vec<[usize; 2]>,
    /// 読込で作った玉（lens1 …）
    pub auto: bool,
    /// 剛体の動き dx dy tx ty tz [mm, deg]
    pub motion: [f64; 5],
    /// 群の移動 [mm]
    pub shift: f64,
    /// 範囲の中の媒質の dn・disp（媒質が無いか面ごとに違えば None）
    pub dn: Option<f64>,
    pub disp: Option<f64>,
}

/// JSON の数（inf は {"$f": "inf"}）
fn num(v: &Value) -> f64 {
    if let Some(x) = v.as_f64() {
        return x;
    }
    match v.get("$f").and_then(Value::as_str) {
        Some("inf") => f64::INFINITY,
        Some("-inf") => f64::NEG_INFINITY,
        _ => f64::NAN,
    }
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .map(|a| a.iter().map(num).collect())
        .unwrap_or_default()
}

/// ソルブ（serve の solves の 1 つ）の量と説明
fn solve_text(q: &Value) -> (String, String) {
    let what = q["what"].as_str().unwrap_or("t").to_string();
    let mut t = format!(
        "{} {}",
        q["kind"].as_str().unwrap_or_default(),
        fmt_num(num(&q["value"]))
    );
    if let Some(o) = q["other"].as_u64() {
        t += &trf("（面 {}）", &[&o]);
    }
    (what, t)
}

/// ピックアップ（serve の pickups の 1 つ）の量と説明
fn pickup_text(p: &Value) -> (String, String) {
    let what = p["what"].as_str().unwrap_or_default().to_string();
    let t = format!(
        "{} × {} {} + {}",
        fmt_num(num(&p["scale"])),
        trf("面 {}", &[&p["src"]]),
        p["from"].as_str().unwrap_or(&what),
        fmt_num(num(&p["offset"])),
    );
    (what, t)
}

/// serve の `surface`（全部の面）の結果
pub fn parse(v: &Value) -> (Vec<Row>, Vec<Unit>) {
    let s = |x: &Value| x.as_str().unwrap_or_default().to_string();
    let rows = v["surfaces"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|f| Row {
                    typ: s(&f["type"]),
                    comment: s(&f["comment"]),
                    r: num(&f["r"]),
                    t: num(&f["t"]),
                    material: s(&f["material"]),
                    coating: s(&f["coating"]),
                    semi: num(&f["semi"]),
                    semi_auto: f["semi_auto"] == true,
                    k: num(&f["k"]),
                    stop: f["stop"] == true,
                    ignore: f["ignore"] == true,
                    asph: nums(&f["asph"]),
                    cx: num(&f["cx"]),
                    cb: nums(&f["cb"]),
                    cb_order: num(&f["cb_order"]),
                    focal: num(&f["focal"]),
                    jones: nums(&f["jones"]),
                    dn: f["dn"].as_f64().unwrap_or(0.0),
                    disp: f["disp"].as_f64().unwrap_or(0.0),
                    solves: f["solves"]
                        .as_array()
                        .map(|a| a.iter().map(solve_text).collect())
                        .unwrap_or_default(),
                    pickups: f["pickups"]
                        .as_array()
                        .map(|a| a.iter().map(pickup_text).collect())
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    let units = v["units"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|u| Unit {
                    name: s(&u["name"]),
                    spans: u["spans"]
                        .as_array()
                        .map(|x| {
                            x.iter()
                                .filter_map(|p| {
                                    Some([p[0].as_u64()? as usize, p[1].as_u64()? as usize])
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    auto: u["auto"] == true,
                    motion: std::array::from_fn(|i| u["motion"][i].as_f64().unwrap_or(0.0)),
                    shift: u["shift"].as_f64().unwrap_or(0.0),
                    dn: u["dn"].as_f64(),
                    disp: u["disp"].as_f64(),
                })
                .collect()
        })
        .unwrap_or_default();
    (rows, units)
}

/// 表のブロック（木の節）
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub name: String,
    /// 面（光の通る順、重なりなし）
    pub surfs: Vec<usize>,
    /// 親のブロック（None は最上位）
    pub parent: Option<usize>,
}

/// 玉・群を面の範囲の入れ子で木にする。読込で作った玉は、名前のあるブロックの中にあれば外す。同じ面の組は最初の 1 つ
pub fn tree(units: &[Unit]) -> Vec<Block> {
    let sets: Vec<BTreeSet<usize>> = units
        .iter()
        .map(|u| u.spans.iter().flat_map(|s| s[0]..=s[1]).collect())
        .collect();
    let mut keep: Vec<usize> = Vec::new();
    for (i, u) in units.iter().enumerate() {
        if sets[i].is_empty() || keep.iter().any(|&j| sets[j] == sets[i]) {
            continue;
        }
        if u.auto
            && units
                .iter()
                .enumerate()
                .any(|(j, w)| !w.auto && sets[i].is_subset(&sets[j]))
        {
            continue;
        }
        keep.push(i);
    }
    keep.sort_by_key(|&i| (sets[i].first().copied(), std::cmp::Reverse(sets[i].len())));
    let parent = |i: usize| {
        keep.iter()
            .enumerate()
            .filter(|(_, &j)| {
                j != i && sets[j].len() > sets[i].len() && sets[i].is_subset(&sets[j])
            })
            .min_by_key(|(_, &j)| sets[j].len())
            .map(|(b, _)| b)
    };
    keep.iter()
        .map(|&i| Block {
            name: units[i].name.clone(),
            surfs: sets[i].iter().copied().collect(),
            parent: parent(i),
        })
        .collect()
}

/// 組合せの要素ごとの面の範囲（最初・最後）。決め方は組合せの図と同じ（[`crate::combo::LayoutData::spans_for`]）
pub fn elem_spans(
    ids: &[String],
    units: &[Unit],
    lead_stop: bool,
    rows: &[Row],
) -> Vec<Option<[usize; 2]>> {
    let d = crate::combo::LayoutData {
        vertices: vec![None; rows.len()],
        plates: vec![("image".into(), rows.len().saturating_sub(1), Vec::new())],
        stop: rows.iter().position(|r| r.stop),
        units: units
            .iter()
            .map(|u| (u.name.clone(), u.spans.clone()))
            .collect(),
        ..Default::default()
    };
    d.spans_for(ids, lead_stop)
}

/// 表の列（変えられる値）。Par は面タイプのパラメータ（OpticStudio の Par 1〜8 と同じ番号）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Col {
    Type,
    Comment,
    R,
    T,
    Material,
    Coating,
    Semi,
    Conic,
    Par(u8),
}

/// パラメータの列の数の上限
pub const MAX_PAR: u8 = 8;

/// 面タイプの名前と説明（種類の列の候補）
pub const TYPE_NAMES: &[(&str, &str)] = &[
    ("STANDARD", "標準"),
    ("EVENASPH", "偶数次非球面"),
    ("COORDBRK", "座標ブレーク"),
    ("PARAXIAL", "近軸レンズ"),
    ("TOROIDAL", "トロイダル"),
    ("JONESMAT", "Jones 行列"),
    ("SZERNSAG", "Zernike（Standard）"),
    ("FZERNSAG", "Zernike（Fringe）"),
    ("GRID_SAG", "Grid Sag"),
    ("FRESNELS", "フレネル"),
    ("CFRESNEL", "円柱フレネル"),
    ("BINARY_2", "Binary 2"),
    ("GRINSUR8", "GRIN（Gradium）"),
    ("GRINSUR9", "GRIN（Gradient 9）"),
    ("USER", "利用者定義"),
];

/// 種類の列に打った文字に合う面タイプ（前方一致を先に、次に名前・説明の部分一致）
pub fn type_matches(s: &str) -> Vec<&'static str> {
    let q = s.trim().to_uppercase();
    let mut out: Vec<&'static str> = TYPE_NAMES
        .iter()
        .filter(|(n, _)| n.starts_with(&q))
        .map(|x| x.0)
        .collect();
    for (n, d) in TYPE_NAMES {
        if !out.contains(n)
            && (n.contains(&q)
                || d.to_uppercase().contains(&q)
                || tr(d).to_uppercase().contains(&q))
        {
            out.push(n);
        }
    }
    out
}

/// 非球面の係数の名前（r² … r¹⁶）
const ASPH_LABELS: [&str; 8] = [
    "r² の係数",
    "r⁴ の係数",
    "r⁶ の係数",
    "r⁸ の係数",
    "r¹⁰ の係数",
    "r¹² の係数",
    "r¹⁴ の係数",
    "r¹⁶ の係数",
];
const TOR_LABELS: [&str; 7] = [
    "y² の係数",
    "y⁴ の係数",
    "y⁶ の係数",
    "y⁸ の係数",
    "y¹⁰ の係数",
    "y¹² の係数",
    "y¹⁴ の係数",
];
const CB_LABELS: [&str; 6] = ["偏心 X", "偏心 Y", "傾き X", "傾き Y", "傾き Z", "順序"];
const JONES_LABELS: [&str; 8] = [
    "A 実", "A 虚", "B 実", "B 虚", "C 実", "C 虚", "D 実", "D 虚",
];

/// tr で引く名前（パラメータの名前・面タイプの説明。訳の表の検査用）
#[cfg(test)]
pub fn labels() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = TYPE_NAMES.iter().map(|x| x.1).collect();
    v.extend(ASPH_LABELS);
    v.extend(TOR_LABELS);
    v.extend(CB_LABELS);
    v.extend(JONES_LABELS);
    v.extend(["回転半径", "焦点距離"]);
    v
}

/// 面タイプ typ のパラメータ i（1〜）の名前。そのタイプに無ければ None（番号は計算側の surf_parm と同じ）
pub fn par_label(typ: &str, i: u8) -> Option<&'static str> {
    let i = i as usize;
    let l = match typ {
        "EVENASPH" | "FRESNELS" | "CFRESNEL" | "BINARY_2" => ASPH_LABELS.get(i.checked_sub(1)?)?,
        "TOROIDAL" if i == 1 => "回転半径",
        "TOROIDAL" => TOR_LABELS.get(i.checked_sub(2)?)?,
        "COORDBRK" => CB_LABELS.get(i.checked_sub(1)?)?,
        "PARAXIAL" if i == 1 => "焦点距離",
        "JONESMAT" => JONES_LABELS.get(i.checked_sub(1)?)?,
        _ => return None,
    };
    Some(tr(l))
}

/// 面タイプのパラメータの数
pub fn par_count(typ: &str) -> u8 {
    (1..=MAX_PAR)
        .take_while(|&i| par_label(typ, i).is_some())
        .count() as u8
}

/// 面 r のパラメータ i の値
pub fn par_value(r: &Row, i: u8) -> Option<f64> {
    par_label(&r.typ, i)?;
    let i = i as usize;
    let at = |v: &[f64], j: usize| v.get(j).copied().unwrap_or(0.0);
    Some(match r.typ.as_str() {
        "TOROIDAL" if i == 1 => {
            if r.cx == 0.0 {
                f64::INFINITY
            } else {
                1.0 / r.cx
            }
        }
        "TOROIDAL" => at(&r.asph, i - 2),
        "COORDBRK" if i == 6 => r.cb_order,
        "COORDBRK" => at(&r.cb, i - 1),
        "PARAXIAL" => r.focal,
        "JONESMAT" => at(&r.jones, i - 1),
        _ => at(&r.asph, i - 1),
    })
}

/// 面 r のパラメータ i を v にする面の値（serve の edit の set_surface の fields）
pub fn par_fields(r: &Row, i: u8, v: f64) -> Result<Map<String, Value>, String> {
    let mut f = Map::new();
    let i = i as usize;
    let put = |a: &[f64], j: usize, n: usize| -> Value {
        let mut a = a.to_vec();
        if a.len() < n.max(j + 1) {
            a.resize(n.max(j + 1), 0.0);
        }
        a[j] = v;
        json!(a)
    };
    match r.typ.as_str() {
        "TOROIDAL" if i == 1 => {
            let cx = if v == 0.0 || v.is_infinite() {
                0.0
            } else {
                1.0 / v
            };
            f.insert("cx".into(), json!(cx));
        }
        "TOROIDAL" => {
            f.insert("asph".into(), put(&r.asph, i - 2, 0));
        }
        "COORDBRK" if i == 6 => {
            if v != 0.0 && v != 1.0 {
                return Err(tr("順序は 0（偏心してから傾ける）か 1（傾けてから偏心）").into());
            }
            f.insert("cb_order".into(), json!(v));
        }
        "COORDBRK" => {
            f.insert("cb".into(), put(&r.cb, i - 1, 5));
        }
        "PARAXIAL" => {
            f.insert("focal".into(), json!(v));
        }
        "JONESMAT" => {
            f.insert("jones".into(), put(&r.jones, i - 1, 8));
        }
        _ => {
            f.insert("asph".into(), put(&r.asph, i - 1, 0));
        }
    }
    Ok(f)
}

impl Col {
    /// 表の列（パラメータは npar 列）
    pub fn all(npar: u8) -> Vec<Col> {
        let mut v = vec![
            Col::Type,
            Col::Comment,
            Col::R,
            Col::T,
            Col::Material,
            Col::Coating,
            Col::Semi,
            Col::Conic,
        ];
        v.extend((1..=npar.min(MAX_PAR)).map(Col::Par));
        v
    }

    /// 面の値の名前（serve の edit の set_surface。パラメータは面タイプで変わるので [`par_fields`]）
    pub fn field(self) -> &'static str {
        match self {
            Col::Type => "type",
            Col::Comment => "comment",
            Col::R => "r",
            Col::T => "t",
            Col::Material => "material",
            Col::Coating => "coating",
            Col::Semi => "semi",
            Col::Conic => "k",
            Col::Par(_) => "par",
        }
    }

    /// 見出し（パラメータは今の面のタイプの名前、無ければ番号）
    fn header(self, typ: Option<&str>) -> String {
        match self {
            Col::Type => tr("種類").into(),
            Col::Comment => tr("コメント").into(),
            Col::R => tr("曲率半径").into(),
            Col::T => tr("厚さ").into(),
            Col::Material => tr("硝材").into(),
            Col::Coating => tr("膜").into(),
            Col::Semi => tr("半径").into(),
            Col::Conic => tr("コーニック").into(),
            Col::Par(i) => typ
                .and_then(|t| par_label(t, i))
                .map(String::from)
                .unwrap_or_else(|| trf("パラメータ {}", &[&i])),
        }
    }

    /// ピックアップの量の名前（serve の add_pickup の what）。種類・コメントは None。硝材は dn（材料と一緒に追従し、
    /// disp も組で足す）、膜は coating（名前を写す）
    pub fn pick_what(self) -> Option<String> {
        Some(match self {
            Col::R => "r".into(),
            Col::T => "t".into(),
            Col::Semi => "semi".into(),
            Col::Conic => "k".into(),
            Col::Par(i) => format!("p{i}"),
            Col::Material => "dn".into(),
            Col::Coating => "coating".into(),
            _ => return None,
        })
    }

    /// セルの右端の [...]（ソルブ・ピックアップ・硝材の追加設定）があるか
    pub fn has_extra(self) -> bool {
        self.solve_what().is_some() || self.pick_what().is_some()
    }

    /// ピックアップの量のうち、この列で組にして足す・外すもの（硝材は dn と disp）
    fn pick_set(self) -> Vec<String> {
        match self {
            Col::Material => vec!["dn".into(), "disp".into()],
            c => c.pick_what().into_iter().collect(),
        }
    }

    /// ソルブの量（c・t）
    pub fn solve_what(self) -> Option<&'static str> {
        match self {
            Col::R => Some("c"),
            Col::T => Some("t"),
            _ => None,
        }
    }

    /// 面 r のこの値のソルブ・ピックアップ（説明）
    pub fn solved(self, r: &Row) -> Option<(char, String)> {
        if let Some(w) = self.solve_what() {
            if let Some((_, t)) = r.solves.iter().find(|s| s.0 == w) {
                return Some(('S', trf("ソルブ: {}", &[t])));
            }
        }
        let pw = self.pick_what()?;
        r.pickups
            .iter()
            .find(|p| p.0 == pw || (self == Col::R && p.0 == "c"))
            .map(|(_, t)| ('P', trf("ピックアップ: {}", &[t])))
    }

    pub fn text(self, r: &Row) -> String {
        let n = |x: f64| {
            if x.is_infinite() {
                if x > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
            } else if x.is_nan() {
                "—".into()
            } else {
                fmt_num((x * 1e6).round() / 1e6)
            }
        };
        match self {
            Col::Type => r.typ.clone(),
            Col::Comment => r.comment.clone(),
            Col::R => n(r.r),
            Col::T => n(r.t),
            Col::Material => r.material.clone(),
            Col::Coating => r.coating.clone(),
            // 自動は値の後ろに印（入力欄に auto と書くと自動、数を書くと固定）
            Col::Semi if r.semi_auto => format!("{} ({})", n(r.semi), tr("自動")),
            Col::Semi => n(r.semi),
            Col::Conic => n(r.k),
            Col::Par(i) => par_value(r, i).map(n).unwrap_or_default(),
        }
    }

    /// 書き換えの最初の文字（丸めない値）
    pub fn edit_text(self, r: &Row) -> String {
        let full = crate::surf_edit::full;
        match self {
            Col::R if r.r.is_infinite() => String::new(),
            Col::R => full(r.r),
            Col::T => full(r.t),
            Col::Semi if r.semi_auto => "auto".into(),
            Col::Semi => full(r.semi),
            Col::Conic => full(r.k),
            Col::Par(i) => match par_value(r, i) {
                Some(x) if x.is_infinite() => String::new(),
                Some(x) => full(x),
                None => String::new(),
            },
            _ => self.text(r),
        }
    }

    /// 入力できるドロップダウンの列（種類・硝材）
    pub fn has_list(self) -> bool {
        matches!(self, Col::Type | Col::Material)
    }

    /// 入力の文字を値に（曲率半径の inf・空は平面 = 0）
    pub fn parse(self, s: &str) -> Result<Value, String> {
        let s = s.trim();
        match self {
            Col::Comment | Col::Material | Col::Coating => Ok(json!(s)),
            Col::Type => type_matches(s)
                .first()
                .map(|t| json!(t))
                .ok_or_else(|| trf("{} という面タイプは無い", &[&s])),
            Col::Semi if s.eq_ignore_ascii_case("auto") || s == "自動" || s == tr("自動") => {
                Ok(json!("auto"))
            }
            Col::R | Col::Par(_)
                if s.is_empty()
                    || s.eq_ignore_ascii_case("inf")
                    || s.eq_ignore_ascii_case("infinity") =>
            {
                Ok(json!(0.0))
            }
            _ => s
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .map(|x| json!(x))
                .ok_or_else(|| trf("{} は数ではない", &[&s])),
        }
    }

    fn numeric(self) -> bool {
        !matches!(
            self,
            Col::Type | Col::Comment | Col::Material | Col::Coating
        )
    }
}

/// 表から GUI に頼むこと
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// 面 j の値を変える
    Edit { j: usize, col: Col, value: Value },
    /// ブロックを選ぶ（None は選ばない）
    Select(Option<String>),
    /// ブロックのロックを切り替える
    Lock(String, bool),
    /// 面の範囲 [最初, 最後] だけを系ファイルに書き出す（serve の save_range）
    ExportRange([usize; 2]),
    /// 表に出ない理由（ロック・組合せの要素の設定で変える値など）
    Refused(String),
    /// 面 j の値を名前で変える（serve の edit の set_surface の fields）
    Fields {
        j: usize,
        fields: Map<String, Value>,
    },
    /// 玉・群・機構部品の欄を開く・閉じる（開いた系）
    UnitMech,
    /// 面 j のデータの項目を変える（serve の edit の set_surface_data）
    Data { j: usize, data: Map<String, Value> },
    /// そのほかの編集の操作（絞り・ソルブ・ピックアップ。開いた系だけ）
    Ops(Vec<Value>),
    /// 面 j の全部の項目を読む（serve の surface_data）
    NeedData(usize),
    /// 硝材の候補（ガラスカタログ）を読む
    NeedGlasses,
}

/// 書き換え中のセル
#[derive(Debug, Clone)]
struct Editing {
    j: usize,
    col: Col,
    text: String,
    /// 入力欄にフォーカスを渡したか
    focused: bool,
    /// 最初に全部を選ぶ（Enter・F2・ダブルクリック）。打った文字から始めたときは後ろにカーソル
    select_all: bool,
    /// 種類の候補の選んでいる番号
    pick: usize,
    /// 種類の候補の上にポインタがあった（クリックで入力欄のフォーカスが外れても決めない）
    list_hover: bool,
    /// 候補を打った文字で絞る（打ち始めたら。開いたときは全部の候補から今の値を選ぶ）
    filtered: bool,
    /// 候補を ↑↓ で選んだ（硝材は選ばなければ打った文字のまま決める）
    moved: bool,
    /// 選んでいる候補まで一覧を送る
    scroll: bool,
}

/// 入力できるドロップダウンの候補（値、説明）。種類は面タイプ、硝材はガラスカタログ（と鏡）。filter なら text で絞る
/// （前方一致を先に、次に部分一致）
fn list_choices(
    col: Col,
    glasses: &[GlassChoice],
    used: &[String],
    text: &str,
    filter: bool,
) -> Vec<(String, String)> {
    match col {
        Col::Type => {
            let names: Vec<&str> = if filter {
                type_matches(text)
            } else {
                TYPE_NAMES.iter().map(|x| x.0).collect()
            };
            names
                .into_iter()
                .map(|t| {
                    let d = TYPE_NAMES.iter().find(|x| x.0 == t).map_or("", |x| tr(x.1));
                    (t.to_string(), d.to_string())
                })
                .collect()
        }
        Col::Material => {
            // この系で使っている硝材（カタログに無いものも）を先に、次に鏡、カタログ
            let extra: Vec<(String, String)> = used
                .iter()
                .filter(|u| !u.eq_ignore_ascii_case("MIRROR"))
                .filter(|u| !glasses.iter().any(|g| g.name.eq_ignore_ascii_case(u)))
                .map(|u| (u.clone(), tr("この系で使用").to_string()))
                .collect();
            let all = extra
                .into_iter()
                .chain(std::iter::once((
                    "MIRROR".to_string(),
                    tr("鏡").to_string(),
                )))
                .chain(glasses.iter().map(|g| (g.name.clone(), g.desc.clone())));
            if !filter {
                return all.collect();
            }
            let q = text.trim().to_uppercase();
            let (mut head, mut rest) = (Vec::new(), Vec::new());
            for (n, d) in all {
                let u = n.to_uppercase();
                if u.starts_with(&q) {
                    head.push((n, d));
                } else if u.contains(&q) || d.to_uppercase().contains(&q) {
                    rest.push((n, d));
                }
            }
            head.extend(rest);
            head
        }
        _ => Vec::new(),
    }
}

/// 硝材の候補（GUI のガラスカタログの名前と、カタログ・nd・νd の説明）
#[derive(Debug, Clone, PartialEq)]
pub struct GlassChoice {
    pub name: String,
    pub desc: String,
}

/// セルの右クリックの欄の入力
#[derive(Debug, Clone, Default)]
struct CellMenu {
    /// ソルブの種類・値・param・別の面
    kind: String,
    val: String,
    param: String,
    other: String,
    /// ピックアップの元の面・倍率・オフセット
    src: String,
    scale: String,
    offset: String,
    /// 選んだ範囲で作る玉・群の名前
    unit_name: String,
    /// 硝材の追加設定の dn・disp の入力（cell の面・列が変わったら今の値から）
    dn: String,
    disp: String,
    cell: Option<(usize, Col)>,
    /// 面・玉・群のピックアップの元（面の番号・「a-b」・ブロックの名前）と向き
    rp_src: String,
    rp_mode: RangePick,
}

/// 面・玉・群のピックアップの向き
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RangePick {
    /// 同じ向き（元と同じ値）
    #[default]
    Same,
    /// 向きを変えた玉（面の順を逆にし、曲率・非球面の符号を変える。厚さはそのまま）
    Flip,
    /// 鏡の後の戻り（面の順を逆にし、曲率はそのまま、厚さの符号を変える。ダブルパス）
    Return,
}

/// 行の右クリックの欄で選んだこと
#[derive(Default)]
struct RowMenu {
    acts: Vec<Act>,
    insert: bool,
    /// 前に挿入（Insert）
    insert_before: bool,
    delete: bool,
    detail: bool,
    /// 貼り付け（false は上書き、true は面を挿入して）
    paste: Option<bool>,
}

/// 書き換えた文字を、面 j（今の値 r）の列 col の変更に（今と同じなら None）
fn commit_act(r: &Row, j: usize, col: Col, text: &str) -> Option<Act> {
    let v = match col.parse(text) {
        Ok(v) => v,
        Err(e) => return Some(Act::Refused(e)),
    };
    let same_num = |cur: f64| {
        let x = v.as_f64();
        if cur.is_infinite() {
            x == Some(0.0)
        } else {
            x == Some(cur)
        }
    };
    let same = match col {
        Col::Semi if v == json!("auto") => r.semi_auto,
        // 自動の面に数を書けば固定にする（値が同じでも）
        Col::Semi if r.semi_auto => false,
        Col::Semi => same_num(r.semi),
        Col::Type => v.as_str() == Some(r.typ.as_str()),
        Col::R => same_num(r.r),
        Col::T => same_num(r.t),
        Col::Conic => same_num(r.k),
        Col::Par(i) => par_value(r, i).map_or(true, same_num),
        _ => text.trim() == col.text(r),
    };
    if same {
        return None;
    }
    match col {
        Col::Par(i) => match par_fields(r, i, v.as_f64().unwrap_or(0.0)) {
            Ok(fields) => Some(Act::Fields { j, fields }),
            Err(e) => Some(Act::Refused(e)),
        },
        _ => Some(Act::Edit { j, col, value: v }),
    }
}

/// ソルブの種類が曲率のものか
fn curv_kind(k: &str) -> bool {
    lensbench_core::edit::solve_kind_parse(k, 0.0, Some(0)).is_ok_and(|x| x.is_curv())
}

/// セルの右クリック・[...] の欄: その値のソルブ・ピックアップ（OpticStudio のソルブの小欄のように）。硝材は
/// 屈折率の追加設定（dn・disp・複屈折）も。bire は面の複屈折の値（読んでいなければ None）、combo は組合せの系
#[allow(clippy::too_many_arguments)]
fn cell_menu(
    ui: &mut Ui,
    m: &mut CellMenu,
    j: usize,
    col: Col,
    r: &Row,
    bire: Option<&Value>,
    combo: bool,
    cfg: (usize, &[String]),
) -> Vec<Act> {
    let mut ops: Vec<Value> = Vec::new();
    let mut acts = Vec::new();
    let weak = theme::weak_text(ui.visuals());
    if m.cell != Some((j, col)) {
        m.cell = Some((j, col));
        m.dn = crate::surf_edit::full(r.dn);
        m.disp = crate::surf_edit::full(r.disp);
    }
    let field = |ui: &mut Ui, label: &str, s: &mut String, hint: &str, w: f32| {
        ui.label(RichText::new(label).color(weak));
        ui.add(
            egui::TextEdit::singleline(s)
                .hint_text(hint)
                .desired_width(w),
        )
    };
    if let Some(w) = col.solve_what() {
        ui.label(RichText::new(tr("ソルブ")).strong());
        if let Some((_, t)) = r.solves.iter().find(|s| s.0 == w) {
            ui.horizontal(|ui| {
                ui.label(t);
                if ui.small_button(tr("外す")).clicked() {
                    ops.push(json!({ "op": "remove_solve", "surface": j, "what": w }));
                }
            });
        }
        let kinds: Vec<&str> = lensbench_core::edit::SOLVE_KINDS
            .iter()
            .copied()
            .filter(|k| curv_kind(k) == (w == "c"))
            .collect();
        if !kinds.contains(&m.kind.as_str()) {
            m.kind = kinds.first().copied().unwrap_or_default().into();
        }
        ui.menu_button(trf("種類: {}", &[&m.kind]), |ui| {
            for k in &kinds {
                if ui.selectable_label(m.kind == *k, *k).clicked() {
                    m.kind = k.to_string();
                }
            }
        });
        ui.horizontal(|ui| {
            field(ui, tr("値"), &mut m.val, "0", 60.0);
            field(ui, "param", &mut m.param, "", 50.0);
            field(ui, tr("別の面"), &mut m.other, "", 40.0);
        });
        if ui.button(tr("＋ ソルブにする")).clicked() {
            let mut op = json!({ "op": "add_solve", "surface": j, "kind": m.kind });
            let f = |s: &str| s.trim().parse::<f64>().ok();
            if let Some(x) = f(&m.val) {
                op["value"] = json!(x);
            }
            if let Some(x) = f(&m.param) {
                op["param"] = json!(x);
            }
            if let Ok(o) = m.other.trim().parse::<usize>() {
                op["other"] = json!(o);
            }
            ops.push(op);
        }
        ui.separator();
    }
    let picked_mat = r.pickups.iter().any(|p| p.0 == "dn");
    if let Some(w) = col.pick_what() {
        let set = col.pick_set();
        ui.label(RichText::new(tr("ピックアップ")).strong());
        let now: Vec<&(String, String)> = r
            .pickups
            .iter()
            .filter(|p| p.0 == w || (col == Col::R && p.0 == "c"))
            .collect();
        for (_, t) in &now {
            ui.horizontal(|ui| {
                ui.label(t);
                if ui.small_button(tr("外す")).clicked() {
                    // 硝材は dn・disp の組を外す
                    for p in r
                        .pickups
                        .iter()
                        .filter(|p| set.contains(&p.0) || (col == Col::R && p.0 == "c"))
                    {
                        ops.push(json!({ "op": "remove_pickup", "dst": j, "what": p.0 }));
                    }
                }
            });
        }
        // 硝材・膜は名前を写すだけ（倍率・オフセットは使わない）
        let named = matches!(col, Col::Material | Col::Coating);
        ui.horizontal(|ui| {
            field(ui, tr("元の面"), &mut m.src, "", 40.0);
            if !named {
                field(ui, tr("倍率"), &mut m.scale, "1", 50.0);
                field(ui, tr("オフセット"), &mut m.offset, "0", 50.0);
            }
        });
        if col == Col::Material {
            ui.label(
                RichText::new(tr("元の面の硝材（dn・disp も）に追従する"))
                    .small()
                    .color(weak),
            );
        }
        if ui.button(tr("＋ ピックアップにする")).clicked() {
            match m.src.trim().parse::<usize>() {
                Ok(src) => {
                    let f = |x: &str, d: f64| x.trim().parse::<f64>().unwrap_or(d);
                    let (sc, off) = if named {
                        (1.0, 0.0)
                    } else {
                        (f(&m.scale, 1.0), f(&m.offset, 0.0))
                    };
                    for w in &set {
                        ops.push(json!({
                            "op": "add_pickup", "dst": j, "src": src, "what": w,
                            "scale": sc, "offset": off,
                        }));
                    }
                }
                Err(_) => acts.push(Act::Refused(
                    tr("ピックアップには元の面の番号が要る").into(),
                )),
            }
        }
        ui.separator();
    }
    // 硝材の追加設定: 屈折率のずれ・分散の変化・複屈折（鏡・座標ブレークには無い）
    if col == Col::Material && !r.is_cb() && !r.material.eq_ignore_ascii_case("MIRROR") {
        ui.label(RichText::new(tr("屈折率")).strong());
        let mut fields = Map::new();
        ui.add_enabled_ui(!picked_mat, |ui| {
            egui::Grid::new(("ld_index", j))
                .num_columns(2)
                .show(ui, |ui| {
                    for (key, name, cur, buf) in [
                        ("dn", tr("屈折率のずれ dn"), r.dn, &mut m.dn),
                        ("disp", tr("分散の変化 disp"), r.disp, &mut m.disp),
                    ] {
                        ui.label(RichText::new(name).color(weak));
                        let resp = ui.add(egui::TextEdit::singleline(buf).desired_width(110.0));
                        if resp.lost_focus() {
                            match buf.trim().parse::<f64>().ok().filter(|v| v.is_finite()) {
                                Some(v) if v != cur => {
                                    fields.insert(key.into(), json!(v));
                                }
                                Some(_) => {}
                                None => {
                                    acts.push(Act::Refused(trf("{} は数ではない", &[&buf.trim()])))
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
        });
        if picked_mat {
            ui.label(
                RichText::new(tr("硝材のピックアップ中（dn・disp も元の面から）"))
                    .small()
                    .color(weak),
            );
        }
        if !fields.is_empty() {
            acts.push(Act::Fields { j, fields });
        }
        // 複屈折材の板の入口・出口（開いた系の標準面。組合せは要素の波長板で）
        if (!combo && r.typ == "STANDARD") || bire.is_some_and(|b| !b.is_null()) {
            ui.label(RichText::new(tr("複屈折")).color(weak));
            match bire {
                Some(b) => {
                    if let Some(nb) = crate::surf_edit::bire_ui(ui, j, b) {
                        let mut data = Map::new();
                        data.insert("bire".into(), nb);
                        acts.push(Act::Data { j, data });
                    }
                }
                None => {
                    ui.weak(tr("面の値を読んでいます…"));
                }
            }
        }
        ui.separator();
    }
    // 多重構成の行にする（開いた系）
    if let (false, Some(name)) = (combo, config_param(r, j, col)) {
        let have = cfg.1.iter().any(|p| p.eq_ignore_ascii_case(&name));
        let resp = ui.add_enabled(
            !have,
            egui::Button::new(if have {
                trf("多重構成の行 {} あり", &[&name])
            } else {
                trf("多重構成の行にする（{}）", &[&name])
            }),
        );
        if resp.clicked() {
            if cfg.0 < 2 {
                ops.push(json!({ "op": "set_config_count", "n": 2 }));
            }
            ops.push(json!({ "op": "add_config_row", "param": name }));
        }
        ui.separator();
    }
    if !ops.is_empty() {
        acts.push(Act::Ops(ops));
    }
    if !acts.is_empty() {
        ui.close_menu();
    }
    acts
}

/// 面 j の列 col の多重構成の行の名前（serve の add_config_row の param）。行にできない列は None
pub fn config_param(r: &Row, j: usize, col: Col) -> Option<String> {
    let k = match col {
        Col::R => "r".to_string(),
        Col::T => "t".into(),
        Col::Conic => "k".into(),
        Col::Material => "dn".into(),
        Col::Par(i) => match r.typ.as_str() {
            "EVENASPH" if (1..=8).contains(&i) => format!("a{}", 2 * i),
            "COORDBRK" if (1..=5).contains(&i) => {
                ["dx", "dy", "tx", "ty", "tz"][i as usize - 1].into()
            }
            _ => return None,
        },
        _ => return None,
    };
    Some(format!("{j}.{k}"))
}

/// 今のセルの行 cur と起点 anchor の間の行（表の並び rows の順）
fn selection(rows: &[usize], anchor: Option<usize>, cur: Option<usize>) -> Vec<usize> {
    let Some(j) = cur else {
        return Vec::new();
    };
    let pos = |k: usize| rows.iter().position(|&x| x == k);
    match (anchor.and_then(pos), pos(j)) {
        (Some(a), Some(b)) => rows[a.min(b)..=a.max(b)].to_vec(),
        _ => vec![j],
    }
}

/// セルを選ぶ（Shift は範囲を広げる、keep は範囲を残す＝範囲の中の右クリック）
fn click_to(st: &mut LensData, j: usize, col: Col, shift: bool, keep: bool) {
    st.ins_at = None;
    if shift {
        if st.anchor.is_none() {
            st.anchor = Some(st.cur.map_or(j, |x| x.0));
        }
    } else if !keep {
        st.anchor = None;
    }
    if !keep {
        st.cur = Some((j, col));
    }
}

/// 面 at に面を足したとき（after なら面 at−1 の後に、そうでなければ面 at の前に）、ブロック scope（と外のブロック）が
/// at から始まる・at−1 で終わるなら、足した面も含める範囲にする（serve の edit の set_unit。insert_surface の後に当てる）
fn grow_blocks(blocks: &[Block], scope: Option<usize>, at: usize, after: bool) -> Vec<Value> {
    let mut ops = Vec::new();
    let mut b = scope;
    while let Some(i) = b {
        let blk = &blocks[i];
        let has = |x: usize| blk.surfs.contains(&x);
        let edge = if after {
            at > 0 && has(at - 1) && !has(at)
        } else {
            has(at) && (at == 0 || !has(at - 1))
        };
        if edge {
            // 続く面の組 [最初, 最後] を、足した後の番号に（at から始まる組は前に 1 つ伸ばす）
            let mut spans: Vec<[usize; 2]> = Vec::new();
            for &x in &blk.surfs {
                match spans.last_mut() {
                    Some(s) if s[1] + 1 == x => s[1] = x,
                    _ => spans.push([x, x]),
                }
            }
            for s in &mut spans {
                if s[0] > at {
                    s[0] += 1;
                }
                if s[1] >= at {
                    s[1] += 1;
                } else if after && s[1] + 1 == at {
                    s[1] = at;
                }
            }
            ops.push(json!({ "op": "set_unit", "unit": blk.name, "spans": spans }));
        }
        b = blk.parent;
    }
    ops
}

/// 面 j を含む一番内側のブロック
fn inner_block(blocks: &[Block], j: usize) -> Option<String> {
    blocks
        .iter()
        .filter(|b| b.surfs.contains(&j))
        .min_by_key(|b| b.surfs.len())
        .map(|b| b.name.clone())
}

/// 選んだ行を表の文字に（タブ区切り、1 行に 1 面。Excel に貼れる。たたんだブロックの行は除く）
fn copy_text(rows: &[Row], sel: &[usize], cols: &[Col]) -> String {
    sel.iter()
        .filter_map(|&j| rows.get(j))
        .map(|r| {
            cols.iter()
                .map(|c| c.edit_text(r).replace(['\t', '\n', '\r'], " "))
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 貼り付け: タブ区切りの文字を、今のセル（行 at、列 ci）から右下へ当てる。1 つの値で範囲を選んでいれば、範囲の行の今の列に。
/// 変えられないセル（ro が理由を返す）・同じ値は飛ばす。種類は先に変える（パラメータの意味が種類で変わる）
/// 面を挿入して貼り付け: 写した行の数だけ、面 at の前に空の面を挿入し（ブロックの中なら gros_blocks でブロックを伸ばす）、
/// 写した行の値を最初の列から書く（OpticStudio の Paste Surfaces）
fn insert_paste_acts(
    rows: &[Row],
    blocks: &[Block],
    scope: Option<usize>,
    at: usize,
    cols: &[Col],
    text: &str,
) -> Vec<Act> {
    let n = text
        .trim_end_matches(['\r', '\n'])
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .count();
    if n == 0 {
        return Vec::new();
    }
    let mut ops = Vec::new();
    for k in 0..n {
        ops.push(json!({ "op": "insert_surface", "at": at + k, "surface": {} }));
        if k == 0 {
            ops.extend(grow_blocks(blocks, scope, at, false));
        }
    }
    let blank = Row {
        typ: "STANDARD".into(),
        r: f64::INFINITY,
        // insert_surface の {} の面は半径 0（固定）
        ..Default::default()
    };
    let mut after: Vec<Row> = rows.to_vec();
    for _ in 0..n {
        after.insert(at, blank.clone());
    }
    let targets: Vec<usize> = (at..at + n).collect();
    let mut acts = vec![Act::Ops(ops)];
    acts.extend(paste_acts(&after, &targets, cols, 0, text, &|_, _| None));
    acts
}

fn paste_acts(
    rows: &[Row],
    targets: &[usize],
    cols: &[Col],
    ci: usize,
    text: &str,
    ro: &dyn Fn(usize, Col) -> Option<String>,
) -> Vec<Act> {
    let lines: Vec<Vec<&str>> = text
        .trim_end_matches(['\r', '\n'])
        .split('\n')
        .map(|l| l.trim_end_matches('\r').split('\t').collect())
        .collect();
    let single = lines.len() == 1 && lines[0].len() == 1;
    let mut acts = Vec::new();
    let mut skipped = 0;
    let mut put = |j: usize, cells: &[&str]| {
        let Some(r0) = rows.get(j) else { return };
        let mut r = r0.clone();
        let mut fields = Map::new();
        // 種類の列を先に
        let order: Vec<(usize, &&str)> = {
            let mut v: Vec<(usize, &&str)> = cells.iter().enumerate().collect();
            v.sort_by_key(|(k, _)| cols.get(ci + k) != Some(&Col::Type));
            v
        };
        for (k, t) in order {
            let Some(&col) = cols.get(ci + k) else {
                continue;
            };
            if ro(j, col).is_some() {
                skipped += 1;
                continue;
            }
            match commit_act(&r, j, col, t) {
                Some(Act::Edit {
                    col: Col::Type,
                    value,
                    ..
                }) => {
                    r.typ = value.as_str().unwrap_or_default().to_string();
                    acts.push(Act::Edit {
                        j,
                        col: Col::Type,
                        value,
                    });
                }
                Some(Act::Edit { col, value, .. }) => {
                    fields.insert(col.field().into(), value);
                }
                Some(Act::Fields { fields: f, .. }) => {
                    // 同じ並び（asph など）を続けて変えるので、今の値にも入れておく
                    for (key, v) in f {
                        let nums = || -> Vec<f64> {
                            v.as_array()
                                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                                .unwrap_or_default()
                        };
                        let x = v.as_f64().unwrap_or(0.0);
                        match key.as_str() {
                            "asph" => r.asph = nums(),
                            "cb" => r.cb = nums(),
                            "jones" => r.jones = nums(),
                            "cx" => r.cx = x,
                            "focal" => r.focal = x,
                            "cb_order" => r.cb_order = x,
                            _ => {}
                        }
                        fields.insert(key, v);
                    }
                }
                Some(a @ Act::Refused(_)) => acts.push(a),
                _ => {}
            }
        }
        if !fields.is_empty() {
            acts.push(Act::Fields { j, fields });
        }
    };
    if single && targets.len() > 1 {
        for &j in targets {
            put(j, &lines[0]);
        }
    } else {
        for (n, cells) in lines.iter().enumerate() {
            if let Some(&j) = targets.get(n) {
                put(j, cells);
            }
        }
    }
    if skipped > 0 && acts.is_empty() {
        acts.push(Act::Refused(tr("貼り付け先は変えられないセル").into()));
    }
    acts
}

/// 面・玉・群のピックアップの元の面: ブロックの名前ならその面、「a-b」なら a〜b、番号 s なら s から n 面
fn range_src(text: &str, blocks: &[Block], n: usize) -> Result<Vec<usize>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err(tr("ピックアップの元（面の番号・範囲・ブロック名）が要る").into());
    }
    if let Some(b) = blocks.iter().find(|b| b.name == t) {
        return Ok(b.surfs.clone());
    }
    let num = |x: &str| x.trim().parse::<usize>().ok();
    if let Some((a, b)) = t.split_once(['-', '~', '〜']) {
        if let (Some(a), Some(b)) = (num(a), num(b)) {
            if a <= b {
                return Ok((a..=b).collect());
            }
        }
    }
    match num(t) {
        Some(s) => Ok((s..s + n).collect()),
        None => Err(trf(
            "ピックアップの元 {} がわからない（面の番号・3-5・ブロック名）",
            &[&t],
        )),
    }
}

/// 面の並び dst を面の並び src からピックアップする操作（serve の edit の add_pickup）。形（曲率・コーニック・
/// 面タイプのパラメータ）と膜は対応する面（反転・戻りは逆の順）から、面の間の硝材・厚さは対応する間から。
/// 1 面だけ（面としてのピックアップ）を同じ向きでするときは、その面の後の硝材も
pub fn range_pick_ops(
    rows: &[Row],
    dst: &[usize],
    src: &[usize],
    mode: RangePick,
) -> Result<Vec<Value>, String> {
    let n = dst.len();
    if n == 0 || n != src.len() {
        return Err(trf("面の数が違う（先 {}、元 {}）", &[&n, &src.len()]));
    }
    if let Some(k) = dst.iter().chain(src).find(|&&k| k >= rows.len()) {
        return Err(trf("面 {} は無い", &[k]));
    }
    if dst.iter().any(|d| src.contains(d)) {
        return Err(tr("ピックアップの先と元の面が重なっている").into());
    }
    let rev = mode != RangePick::Same;
    if rev && dst.iter().chain(src).any(|&k| rows[k].is_cb()) {
        return Err(tr("座標ブレークを含む範囲は逆向きにピックアップできない").into());
    }
    let pick = |w: &str, d: usize, s: usize, sc: f64| json!({ "op": "add_pickup", "dst": d, "src": s, "what": w, "scale": sc, "offset": 0.0 });
    let flip = if mode == RangePick::Flip { -1.0 } else { 1.0 };
    let mut ops = Vec::new();
    for (i, &d) in dst.iter().enumerate() {
        let s = if rev { src[n - 1 - i] } else { src[i] };
        let (rd, rs) = (&rows[d], &rows[s]);
        if !rd.is_cb() && !rs.is_cb() {
            ops.push(pick("c", d, s, flip));
            ops.push(pick("k", d, s, 1.0));
        }
        if rd.typ == rs.typ && rd.typ != "JONESMAT" {
            // 反転すると面のサグの符号が変わる（非球面の係数も）。焦点距離（近軸面）はそのまま
            let sc = if rd.typ == "PARAXIAL" { 1.0 } else { flip };
            for p in 1..=par_count(&rd.typ) {
                ops.push(pick(&format!("p{p}"), d, s, sc));
            }
        }
        ops.push(pick("coating", d, s, 1.0));
    }
    let mirror = |k: usize| rows[k].material.eq_ignore_ascii_case("MIRROR");
    let mut gap = |d: usize, s: usize, t: Option<f64>| {
        if !mirror(d) && !mirror(s) && !rows[d].is_cb() && !rows[s].is_cb() {
            ops.push(pick("dn", d, s, 1.0));
            ops.push(pick("disp", d, s, 1.0));
        }
        if let Some(sc) = t {
            ops.push(pick("t", d, s, sc));
        }
    };
    let tsc = if mode == RangePick::Return { -1.0 } else { 1.0 };
    for i in 0..n - 1 {
        let s = if rev { src[n - 2 - i] } else { src[i] };
        gap(dst[i], s, Some(tsc));
    }
    if n == 1 && !rev {
        gap(dst[0], src[0], None);
    }
    Ok(ops)
}

/// 行の右クリックの欄: 絞り・無視・面の挿入・削除・玉や群を作る・外す・面や玉・群のピックアップ・詳細設定
#[allow(clippy::too_many_arguments)]
fn row_menu(
    ui: &mut Ui,
    cm: &mut CellMenu,
    j: usize,
    r: &Row,
    locked: Option<&str>,
    combo: bool,
    last: usize,
    span: Option<[usize; 2]>,
    block: Option<String>,
    rows: &[Row],
    blocks: &[Block],
) -> RowMenu {
    let mut m = RowMenu::default();
    let edge = j == 0 || j == last;
    if let Some(why) = locked {
        ui.label(RichText::new(why).color(ui.visuals().warn_fg_color));
    }
    ui.add_enabled_ui(!edge && locked.is_none(), |ui| {
        if r.stop {
            // 組合せでは外せる（前の絞り・絞りの要素に戻す）。開いた系は別の面を絞りにすると外れる
            if combo {
                if ui.button(tr("絞りを外す")).clicked() {
                    m.acts
                        .push(Act::Ops(vec![json!({ "op": "unset_stop", "surface": j })]));
                }
            } else {
                ui.add_enabled(
                    false,
                    egui::Button::new(tr("絞り（別の面を絞りにすると外れる）")),
                );
            }
        } else if ui.button(tr("この面を絞りにする")).clicked() {
            m.acts
                .push(Act::Ops(vec![json!({ "op": "set_stop", "surface": j })]));
        }
        let mut ig = r.ignore;
        if ui.checkbox(&mut ig, tr("この面を無視")).changed() {
            let mut fields = Map::new();
            fields.insert("ignore".into(), json!(ig));
            m.acts.push(Act::Fields { j, fields });
        }
    });
    if !combo {
        ui.separator();
        m.insert_before = ui.button(tr("前に面を挿入（Insert）")).clicked();
        m.insert = ui.button(tr("後に面を挿入（Alt+Insert）")).clicked();
        if ui.button(tr("貼り付け（上書き、Ctrl+V）")).clicked() {
            m.paste = Some(false);
        }
        if ui
            .button(tr("前に面を挿入して貼り付け（Ctrl+Shift+V）"))
            .on_hover_text(tr(
                "写した行の数だけ面を挿入してから値を書く（OpticStudio の Paste Surfaces）",
            ))
            .clicked()
        {
            m.paste = Some(true);
        }
        m.insert |= m.insert_before;
        m.delete = ui
            .add_enabled(
                !edge && !r.stop,
                egui::Button::new(tr("この面を削除（Delete）")),
            )
            .clicked();
    }
    // 玉・群（開いた系）: 選んだ行の面の範囲で作る、この面のブロックを外す
    if !combo {
        ui.separator();
        let span = span.unwrap_or([j, j]);
        ui.label(RichText::new(trf("面 {}〜{} で玉・群を作る", &[&span[0], &span[1]])).strong());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut cm.unit_name)
                    .hint_text(tr("名前（G1 など）"))
                    .desired_width(110.0),
            );
            if ui
                .add_enabled(
                    !cm.unit_name.trim().is_empty(),
                    egui::Button::new(tr("作る")),
                )
                .clicked()
            {
                m.acts.push(Act::Ops(vec![json!({
                    "op": "define_unit", "name": cm.unit_name.trim(), "spans": [span],
                })]));
                cm.unit_name.clear();
            }
        });
        if let Some(b) = block {
            if ui
                .button(trf("ブロック {} を外す（面は残す）", &[&b]))
                .clicked()
            {
                m.acts
                    .push(Act::Ops(vec![json!({ "op": "remove_unit", "unit": b })]));
            }
        }
    } else {
        ui.separator();
        ui.label(
            RichText::new(tr("組合せの群は要素の設定の「群」で"))
                .small()
                .color(theme::weak_text(ui.visuals())),
        );
    }
    // 面・玉・群としてのピックアップ（選んだ範囲の面を、別の面・範囲・ブロックから。反転・戻りの向きも）
    ui.separator();
    let span = span.unwrap_or([j, j]);
    let dst: Vec<usize> = (span[0]..=span[1]).collect();
    ui.label(
        RichText::new(if dst.len() == 1 {
            trf("面 {} を面としてピックアップ", &[&j])
        } else {
            trf(
                "面 {}〜{} を玉・群としてピックアップ",
                &[&span[0], &span[1]],
            )
        })
        .strong(),
    );
    ui.add_enabled_ui(locked.is_none(), |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut cm.rp_src)
                    .hint_text(tr("元: 面の番号・3-5・ブロック名"))
                    .desired_width(150.0),
            );
            if ui.button(tr("ピックアップ")).clicked() {
                let r = range_src(&cm.rp_src, blocks, dst.len())
                    .and_then(|src| range_pick_ops(rows, &dst, &src, cm.rp_mode));
                m.acts.push(match r {
                    Ok(ops) => Act::Ops(ops),
                    Err(e) => Act::Refused(e),
                });
            }
        });
        ui.horizontal(|ui| {
            for (v, l, h) in [
                (RangePick::Same, tr("同じ向き"), tr("元と同じ値")),
                (
                    RangePick::Flip,
                    tr("反転"),
                    tr("向きを変えた玉: 面の順を逆に、曲率・非球面の符号を変える（厚さはそのまま）"),
                ),
                (
                    RangePick::Return,
                    tr("戻り"),
                    tr("鏡の後の戻り（ダブルパス）: 面の順を逆に、曲率はそのまま、厚さの符号を変える"),
                ),
            ] {
                ui.selectable_value(&mut cm.rp_mode, v, l).on_hover_text(h);
            }
        });
    });
    ui.separator();
    m.detail = ui
        .button(tr("詳細設定（開口・傾き偏心・形のデータ）"))
        .clicked();
    if !m.acts.is_empty() || m.insert || m.delete || m.detail {
        ui.close_menu();
    }
    m
}

/// 左端の帯のアイコンの下に出す欄（押すたびに開く・閉じる、外のクリックで閉じる）
fn popup(ui: &mut Ui, r: &egui::Response, id: &str, add: impl FnOnce(&mut Ui)) {
    let id = ui.make_persistent_id(id);
    if r.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    egui::popup::popup_below_widget(
        ui,
        id,
        r,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_min_width(220.0);
            add(ui);
        },
    );
}

/// 表の状態
#[derive(Debug, Clone, Default)]
pub struct LensData {
    /// 右クリックの貼り付けを頼んだ（Some(true) は面を挿入して）。次の Paste で使う
    paste_insert: Option<bool>,
    /// 入っているブロックの名前（外から順。空は系の全部）
    pub scope: Vec<String>,
    /// 選んだブロック
    pub sel: Option<String>,
    /// 今のセル（面、列）
    pub cur: Option<(usize, Col)>,
    /// 書き換え中のセル
    edit: Option<Editing>,
    /// セルの右クリックの欄（ソルブ・ピックアップの入力）
    menu: CellMenu,
    /// 面単位の選択（ブロックの中の面を選んでもブロックを選ばない・ブロック全体をハイライトしない）
    pub by_surface: bool,
    /// 足した面（面の数が変わったら今のセルをそこへ。面の数）
    ins_at: Option<(usize, usize)>,
    /// 今のセルを横にも見える所へスクロールする（キーで列を動かしたとき）
    hscroll: bool,
    /// 範囲選択の起点の行（None は今のセルの行だけ）
    anchor: Option<usize>,
    /// キーボード（Shift+F10）・セルの [...] で開いたセルの欄（面、列、出す所、セルの欄だけか）
    kbd_menu: Option<(usize, Col, egui::Pos2, bool)>,
    /// 欄を開いたフレーム（開いたクリックで「外のクリック」として閉じない）
    kbd_fresh: bool,
    /// 表の下に面の詳細設定（開口・傾き偏心・形のデータ・全項目）とブロックの設定を出す
    pub detail: bool,
    /// 開いた系（組合せでない系）でロックしたブロック
    pub locked: BTreeSet<String>,
    /// キーを表で使う（表をクリックした後）
    active: bool,
    /// 選んだ面の設定の欄
    pub insp: crate::surf_edit::Inspector,
    /// 設定の欄に出すブロック（たたんだブロックの行を選んだ・ブロックだけを選んだ）。欄はパネルの側で描く
    pub block_view: Option<String>,
    /// 折りたたみ表示（ブロックをその場で開いて中の面を見せる）。false はブロックに入って中を見る表示
    pub tree: bool,
    /// 折りたたみ表示で開いたブロック
    pub open: BTreeSet<String>,
    /// 最後に見える所へスクロールした面（選んだ面が変わったときだけスクロールし、手で動かせるように）
    scrolled: Option<usize>,
}

/// 表を描くときに渡すもの
pub struct Ctx<'a> {
    pub rows: &'a [Row],
    pub blocks: &'a [Block],
    /// ロックしたブロックの名前
    pub locked: &'a BTreeSet<String>,
    /// 面 j の列 c を表で変えられない理由（ロック以外）
    pub read_only: &'a dyn Fn(usize, Col) -> Option<String>,
    /// 組合せの系（絞り・ソルブ・ピックアップは開いた系だけ）
    pub combo: bool,
    /// 玉・群・機構部品の欄（開いた系だけ。Some は今開いているか）
    pub unit_mech: Option<bool>,
    /// 表についての注意（左端の帯の ⓘ に出す）
    pub note: Option<&'a str>,
    /// 多重構成の構成の数と行の名前（セルのメニューの「多重構成の行にする」）
    pub config_n: usize,
    pub config_rows: &'a [String],
    /// 硝材の候補（GUI のガラスカタログ）
    pub glasses: &'a [GlassChoice],
    /// ガラスカタログを読み終えた（候補が空でも）
    pub glasses_loaded: bool,
}

impl LensData {
    /// 系が変わった（別のファイル）: 階層・選択・ロックを戻す
    pub fn reset(&mut self) {
        // 表示の切り替えは系が変わっても残す
        *self = LensData {
            tree: self.tree,
            ..LensData::default()
        };
    }

    /// ブロック name に入る（name までの道をたどる）
    pub fn enter(&mut self, blocks: &[Block], name: &str) {
        if let Some(path) = path_to(blocks, name) {
            self.scope = path;
            self.sel = None;
            self.cur = None;
            self.edit = None;
        }
    }

    fn scope_index(&self, blocks: &[Block]) -> Option<usize> {
        let last = self.scope.last()?;
        blocks.iter().position(|b| &b.name == last)
    }
}

/// 最上位から name までのブロックの名前
fn path_to(blocks: &[Block], name: &str) -> Option<Vec<String>> {
    let mut i = blocks.iter().position(|b| b.name == name)?;
    let mut v = vec![blocks[i].name.clone()];
    while let Some(p) = blocks[i].parent {
        v.push(blocks[p].name.clone());
        i = p;
    }
    v.reverse();
    Some(v)
}

/// 面 j がロックしたブロックの中か
pub fn locked_at(blocks: &[Block], locked: &BTreeSet<String>, j: usize) -> Option<String> {
    blocks
        .iter()
        .find(|b| locked.contains(&b.name) && b.surfs.contains(&j))
        .map(|b| b.name.clone())
}

/// 面 j の欄 col を書き換えられないロックのブロック。ロックは中身（面・硝材）だけを止め、
/// ブロックの最後の面の厚さ（= ブロックの後の間隔）は変えられる
pub fn locked_cell(
    blocks: &[Block],
    locked: &BTreeSet<String>,
    j: usize,
    col: Col,
) -> Option<String> {
    blocks
        .iter()
        .find(|b| {
            locked.contains(&b.name)
                && b.surfs.contains(&j)
                && !(col == Col::T && b.surfs.iter().max() == Some(&j))
        })
        .map(|b| b.name.clone())
}

/// ブロック i がロックされているか（親がロックされていても）
pub fn block_locked(blocks: &[Block], locked: &BTreeSet<String>, mut i: usize) -> bool {
    loop {
        if locked.contains(&blocks[i].name) {
            return true;
        }
        match blocks[i].parent {
            Some(p) => i = p,
            None => return false,
        }
    }
}

/// 表の行（鍵の面、たたんだブロックなら (ブロック, 最初の面)）。ブロックの行の鍵は最後の面（その厚さ = ブロックの後の間隔）。
/// 往復するブロックは往路・復路でそれぞれ 1 行
fn fold_lines(
    surfs: &[usize],
    child_of: impl Fn(usize) -> Option<usize>,
) -> Vec<(usize, Option<(usize, usize)>)> {
    let mut v: Vec<(usize, Option<(usize, usize)>)> = Vec::new();
    for &j in surfs {
        match child_of(j) {
            Some(i) => match v.last_mut() {
                Some((key, Some((b, _)))) if *b == i && *key + 1 == j => *key = j,
                _ => v.push((j, Some((i, j)))),
            },
            None => v.push((j, None)),
        }
    }
    v
}

/// 表の 1 行
#[derive(Debug, Clone, PartialEq)]
struct Line {
    /// 鍵の面（たたんだブロックはその最後の面）
    key: usize,
    /// たたんだブロック（ブロック, 最初の面）
    fold: Option<(usize, usize)>,
    /// 開いたブロックの深さ（字下げ）
    depth: usize,
    /// この行から始まる開いたブロック（外から順）
    heads: Vec<usize>,
    /// この行の属するいちばん内側のブロック（たたんだブロックはそのもの）
    blk: Option<usize>,
}

/// 階層 scope の面 surfs を表の行にする。open(i) のブロックはたたまずに中の面（と中のブロック）を字下げして並べる
fn lines_of(
    blocks: &[Block],
    scope: Option<usize>,
    surfs: &[usize],
    open: &dyn Fn(usize) -> bool,
    depth: usize,
) -> Vec<Line> {
    let child_of = |j: usize| {
        (0..blocks.len()).find(|&i| blocks[i].parent == scope && blocks[i].surfs.contains(&j))
    };
    let mut v = Vec::new();
    for (key, f) in fold_lines(surfs, child_of) {
        match f {
            Some((i, a)) if open(i) => {
                let inner: Vec<usize> = blocks[i]
                    .surfs
                    .iter()
                    .copied()
                    .filter(|&x| a <= x && x <= key)
                    .collect();
                let mut sub = lines_of(blocks, Some(i), &inner, open, depth + 1);
                if let Some(first) = sub.first_mut() {
                    first.heads.insert(0, i);
                }
                v.extend(sub);
            }
            Some((i, a)) => v.push(Line {
                key,
                fold: Some((i, a)),
                depth,
                heads: Vec::new(),
                blk: Some(i),
            }),
            None => v.push(Line {
                key,
                fold: None,
                depth,
                heads: Vec::new(),
                blk: scope,
            }),
        }
    }
    v
}

/// 面 j を表で見せる行の鍵（その行か、j を含むたたんだ行）。表に無ければ None
fn visible_key(lines: &[Line], blocks: &[Block], j: usize) -> Option<usize> {
    lines
        .iter()
        .find(|l| match l.fold {
            None => l.key == j,
            Some((i, a)) => a <= j && j <= l.key && blocks[i].surfs.contains(&j),
        })
        .map(|l| l.key)
}

/// ブロック i の中で、面 j から続く面の最後（たたんだときの行の鍵）
fn run_end(b: &Block, j: usize) -> usize {
    let mut k = j;
    while b.surfs.contains(&(k + 1)) {
        k += 1;
    }
    k
}

/// 返す Ui は設定の欄（ブロックの設定はパネルの側でここに描く）
pub fn ui(ui: &mut Ui, st: &mut LensData, c: &Ctx) -> (Vec<Act>, Ui) {
    let mut acts = Vec::new();
    let weak = theme::weak_text(ui.visuals());
    let accent = ui.visuals().selection.stroke.color;
    // 消えたブロックに入っていたら外へ
    while !st.scope.is_empty() && st.scope_index(c.blocks).is_none() {
        st.scope.pop();
    }
    if st
        .sel
        .as_ref()
        .is_some_and(|s| !c.blocks.iter().any(|b| &b.name == s))
    {
        st.sel = None;
    }
    // マウスの戻るボタン（パネルの上）で親のブロックへ
    let mouse_back = ui.rect_contains_pointer(ui.max_rect())
        && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1));
    if mouse_back && !st.scope.is_empty() {
        st.sel = st.scope.pop();
        st.cur = None;
        st.edit = None;
        st.active = true;
    }
    let scope = st.scope_index(c.blocks);
    let surfs: Vec<usize> = match scope {
        Some(i) => c.blocks[i].surfs.clone(),
        None => (0..c.rows.len()).collect(),
    };
    let children: Vec<usize> = (0..c.blocks.len())
        .filter(|&i| c.blocks[i].parent == scope)
        .collect();
    // 消えたブロックは開いた組から外す
    st.open.retain(|n| c.blocks.iter().any(|b| &b.name == n));
    // 表の行: 今の階層の面と、直下のブロックを 1 行にたたんだもの（OpticStudio の Black Box のように。中は入って見る）。
    // 折りたたみ表示では開いたブロックの中の面もその場に並べる
    let tree = st.tree;
    let open_set = st.open.clone();
    let is_open = |i: usize| tree && open_set.contains(&c.blocks[i].name);
    let lines = lines_of(c.blocks, scope, &surfs, &is_open, 0);
    let rows: Vec<usize> = lines.iter().map(|l| l.key).collect();
    let line = |j: usize| lines.iter().find(|l| l.key == j);
    let folded = |j: usize| line(j).and_then(|l| l.fold);
    // 面 j の行の属するブロック（選ぶ・入る・開くブロック）
    let child_of = |j: usize| line(j).and_then(|l| l.blk);
    // 面を足した後は、新しい面の表が届いたら今のセルを足した面へ（届くまでは今のセルを動かさない）
    if let Some((at, n)) = st.ins_at {
        if c.rows.len() != n {
            st.ins_at = None;
            st.cur = Some((at, st.cur.map_or(Col::R, |x| x.1)));
            st.anchor = None;
        }
    }
    // 今のセルが見えない面（たたんだブロックの中・表示の切り替え・全部たたむの後）なら、それを含むたたんだ行へ
    if let (Some((j, col)), None) = (st.cur, st.ins_at) {
        let vis = visible_key(&lines, c.blocks, j);
        if vis != Some(j) {
            st.cur = vis.map(|k| (k, col));
            st.edit = None;
        }
    }
    // 開く・たたむ（折りたたみ表示）
    let mut toggle: Option<String> = None;
    // 今の階層の中のブロック（全部開く・たたむ）
    let inside: Vec<String> = c
        .blocks
        .iter()
        .enumerate()
        .filter(|&(i, b)| match scope {
            Some(s) => i != s && b.surfs.iter().all(|x| c.blocks[s].surfs.contains(x)),
            None => true,
        })
        .map(|(_, b)| b.name.clone())
        .collect();

    // 左端の縦の帯: 階層・ブロック・面の挿入削除・詳細設定・玉群・保存のアイコン。押したときだけ中身（一覧・ドロップダウン）を出す
    let mut go: Option<usize> = None;
    let mut enter: Option<String> = None;
    let (mut ins_btn, mut del_btn) = (false, false);
    let mut export_btn = false;
    // 挿入は今の面の前（Insert・＋）。後は Alt+Insert・右クリックの「後に」
    let mut ins_before = false;
    // 右クリックの「貼り付け」（Some(true) は面を挿入して）。クリップボードは次のフレームの Paste で届く
    let mut menu_paste: Option<bool> = None;
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let strip_w = 30.0;
    let strip_r = egui::Rect::from_min_size(full.min, egui::vec2(strip_w, full.height()));
    let body_r = egui::Rect::from_min_max(
        egui::pos2(full.left() + strip_w + 5.0, full.top()),
        full.max,
    );
    ui.painter().vline(
        strip_r.right() + 2.0,
        strip_r.y_range(),
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
    let mut strip = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("ld_strip")
            .max_rect(strip_r)
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    let mut body = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("ld_body")
            .max_rect(body_r)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    {
        let ui = &mut strip;
        ui.spacing_mut().item_spacing.y = 4.0;
        let icon = |ui: &mut Ui, t: &str, on: bool| -> egui::Response {
            ui.add(
                egui::Button::new(RichText::new(t).size(15.0))
                    .selected(on)
                    .min_size(egui::vec2(26.0, 26.0)),
            )
        };
        // 階層: 系と全部のブロックの入れ子の一覧（クリックでそのブロックに入る）と、表示の切り替え
        let here = st.scope.last().cloned();
        let hint = match &here {
            Some(n) => trf(
                "階層: {}（クリックで一覧・表示の切り替え。Backspace で 1 つ上へ）",
                &[n],
            ),
            None => tr("階層: 系の全部の面（クリックで一覧・表示の切り替え）").to_string(),
        };
        let r = icon(ui, "🗀", here.is_some()).on_hover_text(hint);
        popup(ui, &r, "ld_scope", |ui| {
            ui.label(RichText::new(tr("階層")).strong());
            if ui
                .selectable_label(st.scope.is_empty(), tr("系（全部の面）"))
                .clicked()
            {
                go = Some(0);
                ui.memory_mut(|m| m.close_popup());
            }
            for (i, b) in c.blocks.iter().enumerate() {
                let mut depth = 0;
                let mut p = b.parent;
                while let Some(q) = p {
                    depth += 1;
                    p = c.blocks[q].parent;
                }
                let lk = block_locked(c.blocks, c.locked, i);
                ui.horizontal(|ui| {
                    ui.add_space(12.0 * (depth + 1) as f32);
                    let text = if lk {
                        format!("■ {}", b.name)
                    } else {
                        b.name.clone()
                    };
                    let in_path = st.scope.contains(&b.name);
                    let mut rt = RichText::new(text);
                    if in_path {
                        rt = rt.strong();
                    }
                    if ui
                        .selectable_label(here.as_deref() == Some(b.name.as_str()), rt)
                        .on_hover_text(trf(
                            "面 {}〜{}（クリックで中へ）",
                            &[&b.surfs[0], &b.surfs[b.surfs.len() - 1]],
                        ))
                        .clicked()
                    {
                        enter = Some(b.name.clone());
                        ui.memory_mut(|m| m.close_popup());
                    }
                });
            }
            ui.separator();
            ui.label(RichText::new(tr("表示")).strong());
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(!st.tree, tr("階層"))
                    .on_hover_text(tr(
                        "ブロックを 1 行にたたみ、ダブルクリックで中に入って面を変える",
                    ))
                    .clicked()
                {
                    st.tree = false;
                }
                if ui
                    .selectable_label(st.tree, tr("折りたたみ"))
                    .on_hover_text(tr(
                        "ブロックをその場で開いて（▶/▼）、中に入らずに面を変える",
                    ))
                    .clicked()
                {
                    st.tree = true;
                }
            });
            if st.tree && !inside.is_empty() {
                ui.horizontal(|ui| {
                    if ui.small_button(tr("全部開く")).clicked() {
                        st.open.extend(inside.iter().cloned());
                    }
                    if ui.small_button(tr("全部たたむ")).clicked() {
                        for n in &inside {
                            st.open.remove(n);
                        }
                    }
                });
            }
        });
        // ブロック: 今の階層の直下のブロックのドロップダウンと、選んだブロックのロック・開く・中へ
        if !children.is_empty() || st.sel.is_some() {
            let hint = match &st.sel {
                Some(n) => trf("ブロック: {}（クリックで選ぶ・ロック・中へ）", &[n]),
                None => tr("ブロック（クリックで選ぶ・ロック・中へ）").to_string(),
            };
            let r = icon(ui, "▦", st.sel.is_some()).on_hover_text(hint);
            popup(ui, &r, "ld_block", |ui| {
                ui.label(RichText::new(tr("ブロック")).strong());
                for &i in &children {
                    let b = &c.blocks[i];
                    let lk = block_locked(c.blocks, c.locked, i);
                    let text = if lk {
                        format!("■ {}", b.name)
                    } else {
                        b.name.clone()
                    };
                    let r = ui
                        .selectable_label(st.sel.as_deref() == Some(b.name.as_str()), text)
                        .on_hover_text(trf(
                            "面 {}〜{}",
                            &[&b.surfs[0], &b.surfs[b.surfs.len() - 1]],
                        ));
                    if r.clicked() {
                        st.sel = Some(b.name.clone());
                        acts.push(Act::Select(st.sel.clone()));
                    }
                }
                if let Some(name) = st.sel.clone() {
                    ui.separator();
                    let mut lk = c.locked.contains(&name);
                    if ui
                        .checkbox(&mut lk, trf("{} をロック", &[&name]))
                        .on_hover_text(tr(
                            "ロックしたブロック（と中のブロック）の面は変えられない（Ctrl+L でも切り替え）",
                        ))
                        .changed()
                    {
                        acts.push(Act::Lock(name.clone(), lk));
                    }
                    ui.horizontal(|ui| {
                        if tree {
                            let label = if st.open.contains(&name) {
                                tr("たたむ")
                            } else {
                                tr("開く")
                            };
                            if ui
                                .button(label)
                                .on_hover_text(tr("ダブルクリック・Enter でも"))
                                .clicked()
                            {
                                toggle = Some(name.clone());
                                ui.memory_mut(|m| m.close_popup());
                            }
                        }
                        if ui
                            .button(tr("中へ"))
                            .on_hover_text(if tree {
                                tr("ブロックの中だけを見る")
                            } else {
                                tr("ダブルクリック・Ctrl+Enter でも")
                            })
                            .clicked()
                        {
                            enter = Some(name.clone());
                            ui.memory_mut(|m| m.close_popup());
                        }
                    });
                }
            });
        }
        // 選び方: ブロック単位（面を選ぶとそのブロックも選んでハイライト）・面単位（選んだ面だけ）
        let (t, hint) = if st.by_surface {
            (
                "▭",
                tr("選び方: 面単位（選んだ面だけをハイライト）。クリックでブロック単位に"),
            )
        } else {
            (
                "▣",
                tr("選び方: ブロック単位（面を選ぶとそのブロック全体を選ぶ）。クリックで面単位に"),
            )
        };
        if icon(ui, t, st.by_surface).on_hover_text(hint).clicked() {
            st.by_surface = !st.by_surface;
            if st.by_surface {
                st.sel = None;
                acts.push(Act::Select(None));
            }
        }
        ui.separator();
        // 面の挿入・削除（開いた系だけ。Insert・Delete キー・右クリックでも）
        if !c.combo {
            if icon(ui, "＋", false)
                .on_hover_text(tr("選んだ行の前に面を挿入（Insert。後は Alt+Insert）"))
                .clicked()
            {
                ins_btn = true;
                ins_before = true;
            }
            del_btn |= icon(ui, "－", false)
                .on_hover_text(tr("選んだ面を消す（Delete）"))
                .clicked();
        }
        // 表の下の詳細設定（標準的な設定は表で。開口・傾き偏心・形のデータ・ブロックの設定は詳細で）
        if icon(ui, "⚙", st.detail)
            .on_hover_text(tr(
                "表の下に面の詳細設定（開口・前後の傾き偏心・形のデータ・全項目 JSON）とブロックの設定を出す。面の番号のダブルクリックでも",
            ))
            .clicked()
        {
            st.detail = !st.detail;
        }
        if let Some(on) = c.unit_mech {
            if icon(ui, "⊞", on)
                .on_hover_text(tr(
                    "面の範囲で玉・群（ズーム群・フォーカス群など）を作る・機構部品を置く。範囲は選んでいる面から始まる",
                ))
                .clicked()
            {
                acts.push(Act::UnitMech);
            }
        }
        // 選んだ面の範囲（玉・群）だけを系ファイルに書き出す（系全体の保存はファイルのメニュー・Ctrl+S）
        export_btn |= icon(ui, "📤", false)
            .on_hover_text(tr(
                "選んだ面の範囲（玉・群）だけを系ファイルに書き出す…（Shift で範囲を選ぶ。系全体の保存はファイル → 保存）",
            ))
            .clicked();
        if let Some(note) = c.note {
            ui.label(RichText::new("ℹ").size(15.0).color(weak))
                .on_hover_text(note);
        }
    }
    let ui = &mut body;
    if let Some(n) = go {
        st.scope.truncate(n);
        st.cur = None;
        st.edit = None;
    }

    let mut menu_acts: Vec<Act> = Vec::new();
    // 表
    // 書き換えを始めるセル（面、列、打った文字。None は今の値を全部選んで）
    let mut start: Option<(usize, Col, Option<String>)> = None;
    // 決めた書き換え（面、列、文字）と、その後に今のセルを動かす向き（行、列）
    let mut commit: Option<(usize, Col, String)> = None;
    let mut after: Option<(i32, i32)> = None;
    let mut cancel = false;
    let mut clicked_any = false;
    let mut open_detail = false;
    let mut cur_pos: Option<egui::Pos2> = None;
    // 横に見える所へスクロールする今のセル（表の中の縦のスクロールは横の行き先を捨てるので、外の横のスクロールで）
    let mut hscroll_to: Option<egui::Rect> = None;
    let sel_block = st.sel.clone();
    // 選んだ行（Shift+クリック・Shift+↑↓ で範囲。コピー・貼り付け・削除・玉や群を作る）
    let shift_down = ui.input(|i| i.modifiers.shift);
    // 面の複屈折の値（硝材の追加設定。serve の surface_data で読む）
    let bire_now = st
        .insp
        .data
        .as_ref()
        .map(|(dj, v)| (*dj, v["data"]["bire"].clone()));
    let bire_of = |j: usize| bire_now.as_ref().filter(|b| b.0 == j).map(|b| &b.1);
    if let Some((j, Col::Material)) = st.cur {
        if j < c.rows.len() && bire_of(j).is_none() && st.insp.asked != Some(j) {
            st.insp.asked = Some(j);
            acts.push(Act::NeedData(j));
        }
    }
    let selected = selection(&rows, st.anchor, st.cur.map(|x| x.0));
    // 選んだ行の面の範囲（たたんだ行はブロックの最初の面から）
    let sel_span: Option<[usize; 2]> = {
        let lo = selected
            .iter()
            .map(|&j| folded(j).map_or(j, |(_, a)| a))
            .min();
        let hi = selected.iter().copied().max();
        lo.zip(hi).map(|(a, b)| [a, b])
    };
    let row_h = 20.0;
    // パラメータの列は系の面タイプの最大の数（OpticStudio の Par 1〜）
    let npar = c.rows.iter().map(|r| par_count(&r.typ)).max().unwrap_or(0);
    let cols = Col::all(npar);
    let cur_typ = st
        .cur
        .and_then(|(j, _)| c.rows.get(j))
        .map(|r| r.typ.clone());
    let last = c.rows.len().saturating_sub(1);
    // 面 j の列 col を変えられない理由（ロック・組合せ・その面タイプに無いパラメータ）
    let cell_ro = |j: usize, col: Col| -> Option<String> {
        let r = c.rows.get(j)?;
        if let Col::Par(i) = col {
            if par_label(&r.typ, i).is_none() {
                return Some(trf("{} にパラメータ {} は無い", &[&r.typ, &i]));
            }
        }
        if col == Col::Type && (j == 0 || j == last) {
            return Some(tr("物体面・像面の種類は変えられない").into());
        }
        locked_cell(c.blocks, c.locked, j, col)
            .map(|n| trf("{} はロック中", &[&n]))
            .or_else(|| (c.read_only)(j, col))
    };
    // 詳細設定を出すときは、表と設定に残りを分ける（パネルが低くても設定の欄が押し出されないように。
    // 横に広く低いパネルでは左右に並べる。境はドラッグで動かせる）。出さないときは表が残りを全部使う
    let (mut top, bottom) = if st.detail {
        crate::split::two(ui, "lens_data", (0.5, 0.55), 90.0)
    } else {
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        let child = |ui: &mut Ui, r: egui::Rect, salt: &str| {
            ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("lens_data_full", salt))
                    .max_rect(r)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            )
        };
        // 下に詳細設定を折りたたんだ帯（クリックで開く）
        let bar_h = 20.0;
        let (table_r, bar_r) = if rect.height() > 3.0 * bar_h {
            let (a, b) = rect.split_top_bottom_at_y(rect.bottom() - bar_h);
            (a, b)
        } else {
            (
                rect,
                egui::Rect::from_min_size(rect.left_bottom(), egui::Vec2::ZERO),
            )
        };
        (child(ui, table_r, "a"), child(ui, bar_r, "b"))
    };
    // 幅が足りなければ横に流す（見出しと横のスクロールバーの分を引く）
    let table_h = (top.available_height() - 22.0 - 14.0).max(20.0);
    egui::ScrollArea::horizontal()
        .id_salt("lens_data_h")
        .show(&mut top, |ui| {
            // 文字を選べるラベルはクリックを取ってしまう（セルの空き・行のクリックが行に届かない）
            ui.style_mut().interaction.selectable_labels = false;
            let mut table = TableBuilder::new(ui)
                .id_salt(("lens_data", npar))
                .max_scroll_height(table_h)
                .auto_shrink([false, true])
                .striped(true)
                .resizable(true)
                .sense(Sense::click())
                .column(Column::initial(40.0).at_least(30.0))
                .column(Column::initial(70.0).at_least(30.0));
            for (n, &col) in cols.iter().enumerate() {
                table = table.column(if n + 1 == cols.len() {
                    Column::remainder().at_least(50.0)
                } else {
                    match col {
                        Col::Type => Column::initial(80.0).at_least(40.0),
                        Col::Par(_) => Column::initial(70.0).at_least(40.0),
                        c if c.numeric() => Column::initial(80.0).at_least(40.0),
                        _ => Column::initial(90.0).at_least(40.0),
                    }
                });
            }
            let cur = st.cur.map(|x| x.0);
            if cur != st.scrolled {
                if let Some(pos) = cur.and_then(|j| rows.iter().position(|&x| x == j)) {
                    table = table.scroll_to_row(pos, None);
                }
                st.scrolled = cur;
            }
            table
                .header(22.0, |mut h| {
                    h.col(|ui| {
                        ui.strong(tr("面"));
                    });
                    h.col(|ui| {
                        ui.strong(tr("ブロック"));
                    });
                    for &col in &cols {
                        h.col(|ui| {
                            let t = col.header(cur_typ.as_deref());
                            let r = ui.add(egui::Label::new(RichText::new(&t).strong()).truncate());
                            if let Col::Par(i) = col {
                                r.on_hover_text(trf("パラメータ {}: {}", &[&i, &t]));
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_h, rows.len(), |mut row| {
                        let j = rows[row.index()];
                        let r = &c.rows[j];
                        let blk = child_of(j).filter(|&i| Some(i) != scope);
                        let fold = folded(j);
                        let lock = locked_at(c.blocks, c.locked, j);
                        let in_sel =
                            blk.is_some_and(|i| Some(&c.blocks[i].name) == sel_block.as_ref());
                        row.set_selected(in_sel || selected.contains(&j));
                        row.col(|ui| {
                            if let Some((i, a)) = fold {
                                let resp = ui.interact(ui.max_rect(), ui.id().with("ld_num"), Sense::click());
                                ui.label(RichText::new(format!("{a}–{j}")).monospace().color(weak));
                                if resp.clicked() || resp.secondary_clicked() {
                                    clicked_any = true;
                                    let col = st.cur.map_or(Col::R, |x| x.1);
                                    click_to(st, j, col, shift_down, false);
                                    if st.sel.as_deref() != Some(c.blocks[i].name.as_str()) {
                                        st.sel = Some(c.blocks[i].name.clone());
                                        acts.push(Act::Select(st.sel.clone()));
                                    }
                                }
                                if resp.double_clicked() {
                                    if tree {
                                        toggle = Some(c.blocks[i].name.clone());
                                    } else {
                                        enter = Some(c.blocks[i].name.clone());
                                    }
                                }
                                return;
                            }
                            let t = if j == 0 {
                                "OBJ".to_string()
                            } else if j == last {
                                "IMA".to_string()
                            } else if r.stop {
                                format!("STO {j}")
                            } else {
                                j.to_string()
                            };
                            let mut rt = RichText::new(t).monospace();
                            if r.ignore {
                                rt = rt.strikethrough().color(weak);
                            }
                            // セル全体で受ける（文字の外の空いたところでも右クリック・クリックが効く）
                            let resp = ui.interact(ui.max_rect(), ui.id().with("ld_num"), Sense::click());
                            ui.label(rt);
                            let resp = if r.ignore {
                                resp.on_hover_text(tr("この面は無視（光線が通らない）"))
                            } else {
                                resp.on_hover_text(tr("右クリック・Shift+F10 で絞り・無視・挿入・削除、ダブルクリックで詳細設定"))
                            };
                            if resp.clicked() || resp.secondary_clicked() {
                                clicked_any = true;
                                let col = st.cur.map_or(Col::R, |x| x.1);
                                let keep = resp.secondary_clicked() && selected.contains(&j);
                                click_to(st, j, col, shift_down, keep);
                            }
                            if resp.double_clicked() {
                                open_detail = true;
                            }
                            let row_ro = lock.as_ref().map(|n| trf("{} はロック中", &[n]));
                            resp.context_menu(|ui| {
                                let m = row_menu(
                                    ui,
                                    &mut st.menu,
                                    j,
                                    r,
                                    row_ro.as_deref(),
                                    c.combo,
                                    last,
                                    sel_span,
                                    inner_block(c.blocks, j), c.rows, c.blocks,
                                );
                                menu_acts.extend(m.acts);
                                ins_btn |= m.insert;
                                    ins_before |= m.insert_before;
if let Some(p) = m.paste { menu_paste = Some(p); }
                                del_btn |= m.delete;
                                open_detail |= m.detail;
                            });
                        });
                        row.col(|ui| {
                            // セル全体（ブロック名の外）: 行を選ぶ・右クリックで行の欄（面の番号の列と同じ）。
                            // ブロック名は後に足すので、名前の上はブロックの操作が勝つ
                            let cell = ui.interact(ui.max_rect(), ui.id().with("ld_blk_cell"), Sense::click());
                            if cell.clicked() || cell.secondary_clicked() {
                                clicked_any = true;
                                let col = st.cur.map_or(Col::R, |x| x.1);
                                let keep = cell.secondary_clicked() && selected.contains(&j);
                                click_to(st, j, col, shift_down, keep);
                            }
                            if fold.is_none() {
                                let row_ro = lock.as_ref().map(|n| trf("{} はロック中", &[n]));
                                cell.context_menu(|ui| {
                                    let m = row_menu(
                                        ui,
                                        &mut st.menu,
                                        j,
                                        r,
                                        row_ro.as_deref(),
                                        c.combo,
                                        last,
                                        sel_span,
                                        inner_block(c.blocks, j),
                                        c.rows,
                                        c.blocks,
                                    );
                                    menu_acts.extend(m.acts);
                                    ins_btn |= m.insert;
                                    ins_before |= m.insert_before;
if let Some(p) = m.paste { menu_paste = Some(p); }
                                    del_btn |= m.delete;
                                    open_detail |= m.detail;
                                });
                            }
                            let (depth, heads) =
                                line(j).map_or((0, Vec::new()), |l| (l.depth, l.heads.clone()));
                            ui.spacing_mut().item_spacing.x = 2.0;
                            // 字下げ（開いたブロックの中）
                            for _ in 0..depth.saturating_sub(heads.len()) {
                                ui.label(RichText::new("│").color(weak));
                            }
                            // この行から始まる開いたブロック（▼）と、たたんだブロック（▶）
                            let mut marks: Vec<(usize, bool)> =
                                heads.iter().map(|&i| (i, true)).collect();
                            if let (Some(_), Some(i)) = (fold, blk) {
                                marks.push((i, false));
                            }
                            for (i, opened) in marks {
                                let b = &c.blocks[i];
                                let lk = block_locked(c.blocks, c.locked, i);
                                let text = match (tree, opened, lk) {
                                    (false, _, true) => format!("■ {}", b.name),
                                    (false, _, false) => format!("▶ {}", b.name),
                                    (true, o, l) => format!(
                                        "{} {}{}",
                                        if o { "▼" } else { "▶" },
                                        b.name,
                                        if l { " ■" } else { "" }
                                    ),
                                };
                                let hint = match (tree, opened) {
                                    (false, _) => tr("ダブルクリックで中へ"),
                                    (true, true) => tr("クリックでたたむ"),
                                    (true, false) => tr("クリックで開く"),
                                };
                                let r = ui
                                    .add(
                                        egui::Label::new(
                                            RichText::new(text).color(accent).strong(),
                                        )
                                        .sense(Sense::click()),
                                    )
                                    .on_hover_text(hint);
                                if tree && r.clicked() {
                                    toggle = Some(b.name.clone());
                                }
                                if !tree && r.double_clicked() {
                                    enter = Some(b.name.clone());
                                } else if r.clicked() {
                                    st.sel = Some(b.name.clone());
                                    acts.push(Act::Select(st.sel.clone()));
                                }
                            }
                        });
                        for &col in &cols {
                            row.col(|ui| {
                                // たたんだブロックの行: 厚さ（ブロックの後の間隔）だけ見せて変えられる。ほかはダブルクリックで中へ
                                if let (Some((i, a)), false) = (fold, col == Col::T) {
                                    let text = match col {
                                        Col::Type => tr("ブロック").to_string(),
                                        Col::Comment => trf(
                                            "{} 面",
                                            &[&c.blocks[i]
                                                .surfs
                                                .iter()
                                                .filter(|&&x| a <= x && x <= j)
                                                .count()],
                                        ),
                                        _ => String::new(),
                                    };
                                    let resp = ui.interact(ui.max_rect(), ui.id().with("ld_fold_cell"), Sense::click());
                                    ui.add(egui::Label::new(RichText::new(text).color(weak)).truncate());
                                    if st.cur == Some((j, col)) {
                                        ui.painter().rect_stroke(
                                            ui.max_rect(),
                                            2.0,
                                            egui::Stroke::new(1.5_f32, accent),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    if resp.clicked() || resp.double_clicked() {
                                        clicked_any = true;
                                        click_to(st, j, col, shift_down, false);
                                        if st.sel.as_deref() != Some(c.blocks[i].name.as_str()) {
                                            st.sel = Some(c.blocks[i].name.clone());
                                            acts.push(Act::Select(st.sel.clone()));
                                        }
                                    }
                                    if resp.double_clicked() {
                                        if tree {
                                            toggle = Some(c.blocks[i].name.clone());
                                        } else {
                                            enter = Some(c.blocks[i].name.clone());
                                        }
                                    }
                                    return;
                                }
                                if let Some(e) = st.edit.as_mut().filter(|e| e.j == j && e.col == col) {
                                    let (tab, shift, up, down, enter_k, esc) = ui.input(|i| {
                                        (
                                            i.key_pressed(Key::Tab),
                                            i.modifiers.shift,
                                            i.key_pressed(Key::ArrowUp),
                                            i.key_pressed(Key::ArrowDown),
                                            i.key_pressed(Key::Enter),
                                            i.key_pressed(Key::Escape),
                                        )
                                    });
                                    // Tab で欄の外へフォーカスを移さない（Tab・↑↓ は決めて隣のセルへ）
                                    let te = egui::TextEdit::singleline(&mut e.text)
                                        .desired_width(f32::INFINITY)
                                        .lock_focus(true);
                                    let te = if col.numeric() {
                                        te.horizontal_align(egui::Align::RIGHT)
                                    } else {
                                        te
                                    };
                                    let resp = ui.add(te);
                                    if !e.focused {
                                        resp.request_focus();
                                        if let Some(mut s) =
                                            egui::TextEdit::load_state(ui.ctx(), resp.id)
                                        {
                                            let n = e.text.chars().count();
                                            let from = if e.select_all { 0 } else { n };
                                            s.cursor.set_char_range(Some(
                                                egui::text::CCursorRange::two(
                                                    egui::text::CCursor::new(from),
                                                    egui::text::CCursor::new(n),
                                                ),
                                            ));
                                            s.store(ui.ctx(), resp.id);
                                        }
                                        e.focused = true;
                                    }
                                    let mv = if tab {
                                        Some((0, if shift { -1 } else { 1 }))
                                    } else if up && !col.has_list() {
                                        Some((-1, 0))
                                    } else if down && !col.has_list() {
                                        Some((1, 0))
                                    } else {
                                        None
                                    };
                                    if col.has_list() {
                                        // 種類・硝材: 入力できるドロップダウン。開いたときは全部の候補から今の値を選び、
                                        // 打ち始めたら絞る（↑↓ で選び Enter、クリックでも）
                                        if resp.changed() {
                                            e.filtered = true;
                                            e.moved = false;
                                            e.pick = 0;
                                            e.scroll = true;
                                        }
                                        let mut used: Vec<String> = Vec::new();
                                        if col == Col::Material {
                                            for x in c.rows {
                                                let n = x.material.trim();
                                                if !n.is_empty() && !used.iter().any(|u| u.eq_ignore_ascii_case(n)) {
                                                    used.push(n.to_string());
                                                }
                                            }
                                        }
                                        let m = list_choices(col, c.glasses, &used, &e.text, e.filtered);
                                        if !e.filtered && !e.moved && e.scroll {
                                            let now = col.text(r);
                                            e.pick = m
                                                .iter()
                                                .position(|x| x.0.eq_ignore_ascii_case(&now))
                                                .unwrap_or(0);
                                        }
                                        if e.pick >= m.len() {
                                            e.pick = 0;
                                        }
                                        if up && e.pick > 0 {
                                            e.pick -= 1;
                                            e.moved = true;
                                            e.scroll = true;
                                        }
                                        if down && e.pick + 1 < m.len() {
                                            e.pick += 1;
                                            e.moved = true;
                                            e.scroll = true;
                                        }
                                        let mut chosen: Option<String> = None;
                                        let row_hh = 18.0;
                                        let scroll_to = std::mem::take(&mut e.scroll).then_some(e.pick);
                                        let area = egui::Area::new(resp.id.with("choices"))
                                            .order(egui::Order::Foreground)
                                            .fixed_pos(resp.rect.left_bottom())
                                            .show(ui.ctx(), |ui| {
                                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                                    ui.set_min_width(if col == Col::Material { 300.0 } else { 220.0 });
                                                    if m.is_empty() {
                                                        ui.weak(if col == Col::Type {
                                                            tr("合う面タイプが無い")
                                                        } else if c.glasses.is_empty() && !c.glasses_loaded {
                                                            tr("ガラスカタログを読んでいます…（名前・1.5,60 のようなモデルガラスは打って Enter）")
                                                        } else {
                                                            tr("合うガラスが無い（打った名前・モデルガラスのまま Enter で決める）")
                                                        });
                                                        return;
                                                    }
                                                    let mut sa = egui::ScrollArea::vertical()
                                                        .id_salt("choices_scroll")
                                                        .max_height(280.0)
                                                        .auto_shrink([false, true]);
                                                    if let Some(k) = scroll_to {
                                                        sa = sa.vertical_scroll_offset(
                                                            (k as f32 - 5.0).max(0.0) * row_hh,
                                                        );
                                                    }
                                                    ui.spacing_mut().item_spacing.y = 0.0;
                                                    sa.show_rows(ui, row_hh, m.len(), |ui, range| {
                                                        for k in range {
                                                            let (t, d) = &m[k];
                                                            let txt = if d.is_empty() {
                                                                t.clone()
                                                            } else {
                                                                format!("{t}  {d}")
                                                            };
                                                            if ui.selectable_label(k == e.pick, txt).clicked()
                                                            {
                                                                chosen = Some(t.clone());
                                                            }
                                                        }
                                                    });
                                                    if col == Col::Material {
                                                        ui.label(
                                                            RichText::new(trf("{} 件。打って絞る（名前・カタログ・nd）", &[&m.len()]))
                                                                .small()
                                                                .color(weak),
                                                        );
                                                    }
                                                });
                                            });
                                        let hover = area.response.contains_pointer();
                                        // 種類は選んでいる候補、硝材は ↑↓ で選んだか打った文字で始まる候補（ほかは打った文字のまま）
                                        let pick = m.get(e.pick).map(|x| x.0.clone()).filter(|x| {
                                            e.moved
                                                || (!e.filtered && x.eq_ignore_ascii_case(&e.text))
                                                || (col == Col::Type && e.filtered)
                                                || (!e.text.trim().is_empty()
                                                    && x.to_uppercase().starts_with(&e.text.trim().to_uppercase()))
                                        });
                                        if esc {
                                            cancel = true;
                                        } else if let Some(t) = chosen {
                                            commit = Some((j, col, t));
                                        } else if enter_k || mv.is_some() {
                                            commit = Some((j, col, pick.unwrap_or(e.text.clone())));
                                            after = mv;
                                        } else if resp.lost_focus() && !hover && !e.list_hover {
                                            commit = Some((j, col, if e.filtered || e.moved { pick.unwrap_or(e.text.clone()) } else { e.text.clone() }));
                                        }
                                        e.list_hover = hover;
                                    } else if esc {
                                        cancel = true;
                                    } else if mv.is_some() || resp.lost_focus() {
                                        commit = Some((j, col, e.text.clone()));
                                        after = mv;
                                    }
                                    if commit.is_some() && after.is_some() {
                                        resp.surrender_focus();
                                    }
                                    return;
                                }
                                let text = col.text(r);
                                let is_cur = st.cur == Some((j, col));
                                let ro = cell_ro(j, col);
                                let mut rt = RichText::new(&text);
                                if col.numeric() {
                                    rt = rt.monospace();
                                }
                                if col == Col::Type {
                                    rt = rt.small();
                                }
                                if ro.is_some() || r.ignore {
                                    rt = rt.color(weak);
                                }
                                let layout = if col.numeric() {
                                    egui::Layout::right_to_left(egui::Align::Center)
                                } else {
                                    egui::Layout::left_to_right(egui::Align::Center)
                                };
                                let solved = col.solved(r);
                                // セル全体でクリックを受ける（文字の外の空いたところでも、このセルを選ぶ）。文字のラベルは
                                // クリックを取らない（取ると、文字の上と外で選ばれるセルが変わる）
                                let cell_rect = ui.max_rect();
                                let resp = ui.interact(cell_rect, ui.id().with("ld_cell"), Sense::click());
                                let hover = ui.rect_contains_pointer(cell_rect);
                                let mut extra_at: Option<egui::Pos2> = None;
                                let mut drop_open = false;
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    // 種類・硝材のドロップダウンの ▾（全部の候補から今の値を選んで開く）
                                    if col.has_list() && ro.is_none() && (is_cur || hover) {
                                        let (rect, b) = ui.allocate_exact_size(
                                            egui::vec2(12.0, row_h - 4.0),
                                            Sense::click(),
                                        );
                                        if b.hovered() {
                                            ui.painter().rect_filled(
                                                rect,
                                                2.0,
                                                ui.visuals().widgets.hovered.bg_fill,
                                            );
                                        }
                                        ui.painter().text(
                                            rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "▼",
                                            egui::FontId::proportional(8.0),
                                            weak,
                                        );
                                        if b.on_hover_text(tr("候補から選ぶ（打つと絞り込む）")).clicked() {
                                            drop_open = true;
                                        }
                                    }
                                    // セルの右端の [...]: ソルブ・ピックアップ（と硝材の追加設定）の欄を開く。
                                    // ソルブ・ピックアップがあれば S・P（OpticStudio のセルの右の小さな欄のように）
                                    if col.has_extra() {
                                        let show = solved.is_some() || is_cur || hover;
                                        let can = ro.is_none() && show;
                                        let (rect, b) = ui.allocate_exact_size(
                                            egui::vec2(14.0, row_h - 4.0),
                                            if can { Sense::click() } else { Sense::hover() },
                                        );
                                        if show {
                                            let (glyph, color) = match &solved {
                                                Some((ch, _)) => (ch.to_string(), accent),
                                                None => ("…".to_string(), weak),
                                            };
                                            if can && b.hovered() {
                                                ui.painter().rect_filled(
                                                    rect,
                                                    2.0,
                                                    ui.visuals().widgets.hovered.bg_fill,
                                                );
                                            }
                                            ui.painter().text(
                                                rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                glyph,
                                                egui::FontId::proportional(11.0),
                                                color,
                                            );
                                        }
                                        let b = match (&solved, can) {
                                            (Some((_, why)), _) => b.on_hover_text(why.as_str()),
                                            (None, true) if col == Col::Material => b.on_hover_text(tr(
                                                "ピックアップ・屈折率のずれ・分散・複屈折",
                                            )),
                                            (None, true) => b.on_hover_text(tr("ソルブ・ピックアップ")),
                                            _ => b,
                                        };
                                        if b.clicked() {
                                            extra_at = Some(rect.left_bottom());
                                        }
                                    }
                                    ui.with_layout(layout, |ui| {
                                        ui.add(egui::Label::new(rt).truncate());
                                    });
                                });
                                if drop_open {
                                    clicked_any = true;
                                    click_to(st, j, col, false, false);
                                    start = Some((j, col, None));
                                }
                                if let Some(pos) = extra_at {
                                    clicked_any = true;
                                    click_to(st, j, col, false, false);
                                    st.kbd_menu = Some((j, col, pos, true));
                                    st.kbd_fresh = true;
                                }
                                if is_cur {
                                    cur_pos = Some(ui.max_rect().left_bottom());
                                    if st.hscroll {
                                        hscroll_to = Some(ui.max_rect());
                                        st.hscroll = false;
                                    }
                                    ui.painter().rect_stroke(
                                        ui.max_rect(),
                                        2.0,
                                        egui::Stroke::new(1.5_f32, accent),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                let resp = match (&ro, &solved) {
                                    (Some(why), _) => resp.on_hover_text(why),
                                    (None, Some((_, why))) => resp.on_hover_text(why),
                                    _ => resp,
                                };
                                if resp.clicked() || resp.double_clicked() || resp.secondary_clicked() {
                                    clicked_any = true;
                                    // 範囲を選んでいる行の右クリックは範囲を残す（今のセルも動かさない）
                                    let keep = resp.secondary_clicked()
                                        && selected.len() > 1
                                        && selected.contains(&j);
                                    click_to(st, j, col, shift_down, keep);
                                    if let Some(i) = blk.filter(|_| fold.is_some() || !st.by_surface) {
                                        if st.sel.as_deref() != Some(c.blocks[i].name.as_str()) {
                                            st.sel = Some(c.blocks[i].name.clone());
                                            acts.push(Act::Select(st.sel.clone()));
                                        }
                                    }
                                }
                                if resp.double_clicked() {
                                    match &ro {
                                        Some(why) => acts.push(Act::Refused(why.clone())),
                                        None => start = Some((j, col, None)),
                                    }
                                }
                                let row_ro = lock.as_ref().map(|n| trf("{} はロック中", &[n]));
                                resp.context_menu(|ui| {
                                    if ro.is_none() {
                                        menu_acts.extend(cell_menu(
                                            ui,
                                            &mut st.menu,
                                            j,
                                            col,
                                            r,
                                            bire_of(j),
                                            c.combo,
                                            (c.config_n, c.config_rows),
                                        ));
                                    }
                                    let m = row_menu(
                                    ui,
                                    &mut st.menu,
                                    j,
                                    r,
                                    row_ro.as_deref(),
                                    c.combo,
                                    last,
                                    sel_span,
                                    inner_block(c.blocks, j), c.rows, c.blocks,
                                );
                                    menu_acts.extend(m.acts);
                                    ins_btn |= m.insert;
                                    ins_before |= m.insert_before;
if let Some(p) = m.paste { menu_paste = Some(p); }
                                    del_btn |= m.delete;
                                    open_detail |= m.detail;
                                });
                            });
                        }
                        // セルの文字の外（空いたところ・面・ブロックの列）のクリックでも行を選ぶ
                        let rr = row.response();
                        if rr.clicked() || rr.double_clicked() {
                            clicked_any = true;
                            let col = st.cur.map_or(Col::R, |x| x.1);
                            click_to(st, j, col, shift_down, false);
                            st.edit = None;
                            if let Some(i) = blk.filter(|_| fold.is_some() || !st.by_surface) {
                                if st.sel.as_deref() != Some(c.blocks[i].name.as_str()) {
                                    st.sel = Some(c.blocks[i].name.clone());
                                    acts.push(Act::Select(st.sel.clone()));
                                }
                            }
                            if let (true, Some((i, _))) = (rr.double_clicked(), fold) {
                                if tree {
                                    toggle = Some(c.blocks[i].name.clone());
                                } else {
                                    enter = Some(c.blocks[i].name.clone());
                                }
                            }
                        }
                    });
                });
            if let Some(r) = hscroll_to {
                // 縦は表が scroll_to_row で合わせるので、横だけ（縦の範囲は今の見える所）
                let v = ui.clip_rect();
                ui.scroll_to_rect(egui::Rect::from_x_y_ranges(r.x_range(), v.y_range()), None);
            }
        });
    if cancel {
        st.edit = None;
        st.active = true;
    }
    if !menu_acts.is_empty() {
        st.active = true;
    }
    acts.append(&mut menu_acts);
    if open_detail {
        st.detail = true;
    }
    if clicked_any {
        st.active = true;
    } else if ui.input(|i| i.pointer.any_click()) && !ui.ui_contains_pointer() {
        st.active = false;
    }

    // キー（表をクリックした後。書き換え中は入力欄が使う）
    if st.active
        && st.edit.is_none()
        && st.kbd_menu.is_none()
        && ui.memory(|m| m.focused().is_none())
    {
        // Shift+F10: 今のセルの右クリックの欄をキーボードで
        if ui.input(|i| i.modifiers.shift && i.key_pressed(Key::F10)) {
            if let Some((j, col)) = st.cur {
                let pos = cur_pos.unwrap_or_else(|| ui.min_rect().left_top());
                st.kbd_menu = Some((j, col, pos, false));
            }
        }
        // Insert は前に、Alt+Insert は後に。修飾キーは押したときので見る（同じフレームで離しても）。
        // Shift+Insert は Windows の貼り付け（egui も貼り付けにする）なので挿入に使わない
        let (ins_k, ins_shift, del_k) = ui.input(|i| {
            let (mut k, mut sh) = (false, false);
            for e in &i.events {
                match e {
                    egui::Event::Key {
                        key: Key::Insert,
                        pressed: true,
                        modifiers,
                        ..
                    } if !modifiers.shift => {
                        k = true;
                        sh |= !modifiers.alt;
                    }
                    _ => {}
                }
            }
            (k, sh, i.key_pressed(Key::Delete))
        });
        ins_btn |= ins_k;
        ins_before |= ins_shift;
        del_btn |= del_k;
        let (up, down, left, right, enter_k, f2, back, esc, ctrl, lkey) = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowLeft) || (i.key_pressed(Key::Tab) && i.modifiers.shift),
                i.key_pressed(Key::ArrowRight) || (i.key_pressed(Key::Tab) && !i.modifiers.shift),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::F2),
                i.key_pressed(Key::Backspace),
                i.key_pressed(Key::Escape),
                i.modifiers.command,
                // 文字を打つと書き換えを始めるので、ロックは Ctrl+L
                i.modifiers.command && i.key_pressed(Key::L),
            )
        });
        // コピー（Ctrl+C）・切り取りはコピーだけ・貼り付け（Ctrl+V）。タブ区切りで Excel と行き来できる
        let (copy, paste) = ui.input(|i| {
            let copy = i
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut));
            let paste = i.events.iter().find_map(|e| match e {
                egui::Event::Paste(t) => Some(t.clone()),
                _ => None,
            });
            (copy, paste)
        });
        if copy {
            let sel: Vec<usize> = selected
                .iter()
                .copied()
                .filter(|&j| folded(j).is_none())
                .collect();
            if !sel.is_empty() {
                ui.ctx().copy_text(copy_text(c.rows, &sel, &cols));
            }
        }
        // Ctrl+Shift+V・右クリックの「挿入して貼り付け」は面を挿入して（右クリックは次のフレームの Paste まで覚える）
        let insert_paste = ui.input(|i| i.modifiers.shift) || st.paste_insert == Some(true);
        if paste.is_some() {
            st.paste_insert = None;
        }
        if let (Some(t), Some((j, _)), true) = (&paste, st.cur, insert_paste) {
            let first = folded(j).map_or(j, |(_, a)| a);
            if !c.combo {
                if let Some(n) = locked_at(c.blocks, c.locked, first) {
                    acts.push(Act::Refused(trf("{} はロック中", &[&n])));
                } else {
                    acts.extend(insert_paste_acts(
                        c.rows,
                        c.blocks,
                        scope,
                        first.max(1),
                        &cols,
                        t,
                    ));
                }
            }
        } else if let (Some(t), Some((j, col))) = (paste, st.cur) {
            // 貼り付け先: 範囲を選んでいれば範囲の行、なければ今の行から下へ（たたんだ行は飛ばす）
            let start = if selected.len() > 1 { selected[0] } else { j };
            let p0 = rows.iter().position(|&x| x == start).unwrap_or(0);
            let targets: Vec<usize> = if selected.len() > 1 {
                selected.clone()
            } else {
                rows[p0..].to_vec()
            }
            .into_iter()
            .filter(|&k| folded(k).is_none())
            .collect();
            let ci = cols.iter().position(|&x| x == col).unwrap_or(0);
            acts.extend(paste_acts(c.rows, &targets, &cols, ci, &t, &cell_ro));
        }
        // 打った文字（Ctrl・Alt なし）。セルの書き換えをその文字から始める
        let typed: Option<String> = ui.input(|i| {
            if i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt {
                return None;
            }
            i.events.iter().find_map(|e| match e {
                egui::Event::Text(t) if !t.is_empty() => Some(t.clone()),
                _ => None,
            })
        });
        if !rows.is_empty() {
            let (j, col) = st.cur.unwrap_or((rows[0], Col::R));
            let pos = rows.iter().position(|&x| x == j).unwrap_or(0);
            let ci = cols.iter().position(|&x| x == col).unwrap_or(2);
            let mut moved = None;
            if up && pos > 0 {
                moved = Some((rows[pos - 1], col));
            } else if down && pos + 1 < rows.len() {
                moved = Some((rows[pos + 1], col));
            } else if left && ci > 0 {
                moved = Some((j, cols[ci - 1]));
            } else if right && ci + 1 < cols.len() {
                moved = Some((j, cols[ci + 1]));
            } else if (up || down || left || right) && st.cur.is_none() {
                moved = Some((j, col));
            }
            if let Some(m) = moved {
                // Shift+↑↓ は範囲を広げる
                let shift = ui.input(|i| i.modifiers.shift);
                if shift && (up || down) {
                    if st.anchor.is_none() {
                        st.anchor = Some(j);
                    }
                } else if up || down {
                    st.anchor = None;
                }
                st.cur = Some(m);
                st.hscroll = left || right;
                if let Some(i) = child_of(m.0)
                    .filter(|&i| Some(i) != scope && (folded(m.0).is_some() || !st.by_surface))
                {
                    if st.sel.as_deref() != Some(c.blocks[i].name.as_str()) {
                        st.sel = Some(c.blocks[i].name.clone());
                        acts.push(Act::Select(st.sel.clone()));
                    }
                }
            }
            if let Some((j, col)) = st.cur {
                let on_block = folded(j).filter(|_| col != Col::T);
                if tree && ((ctrl && enter_k) || (enter_k && on_block.is_some())) {
                    // 折りたたみ表示: たたんだ行は Enter で開く、開いたブロックの中の面は Ctrl+Enter でそのブロックをたたむ
                    if let Some(i) = child_of(j) {
                        toggle = Some(c.blocks[i].name.clone());
                    }
                } else if (ctrl && enter_k) || (enter_k && on_block.is_some()) {
                    if let Some(i) = child_of(j) {
                        enter = Some(c.blocks[i].name.clone());
                    }
                } else if on_block.is_some() {
                    // たたんだブロックの行のほかの列は書き換えない
                } else if enter_k || f2 || typed.is_some() {
                    match cell_ro(j, col) {
                        Some(why) => acts.push(Act::Refused(why)),
                        None => start = Some((j, col, typed.clone())),
                    }
                }
            }
        }
        if lkey {
            if let Some(s) = st.sel.clone() {
                let now = c.locked.contains(&s);
                acts.push(Act::Lock(s, !now));
            }
        }
        if back && !st.scope.is_empty() {
            let out = st.scope.pop();
            st.sel = out;
            st.cur = None;
        }
        if esc {
            st.cur = None;
            st.anchor = None;
        }
    }
    // キーボードで開いたセルの欄（右クリックと同じ中身。Esc・外のクリックで閉じる）
    if let Some((j, col, pos, cell_only)) = st.kbd_menu {
        if j >= c.rows.len() || folded(j).is_some() {
            st.kbd_menu = None;
        } else {
            let r = &c.rows[j];
            let ro = cell_ro(j, col);
            let row_ro = locked_at(c.blocks, c.locked, j).map(|n| trf("{} はロック中", &[&n]));
            let mut done = false;
            let menu = &mut st.menu;
            let area = egui::Area::new(ui.id().with("ld_kbd_menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos)
                .show(ui.ctx(), |ui| {
                    egui::Frame::menu(ui.style()).show(ui, |ui| {
                        ui.set_max_width(340.0);
                        if ro.is_none() {
                            let a = cell_menu(
                                ui,
                                menu,
                                j,
                                col,
                                r,
                                bire_of(j),
                                c.combo,
                                (c.config_n, c.config_rows),
                            );
                            done |= !a.is_empty();
                            acts.extend(a);
                        }
                        if cell_only {
                            return;
                        }
                        let m = row_menu(
                            ui,
                            menu,
                            j,
                            r,
                            row_ro.as_deref(),
                            c.combo,
                            last,
                            sel_span,
                            inner_block(c.blocks, j),
                            c.rows,
                            c.blocks,
                        );
                        done |= !m.acts.is_empty()
                            || m.insert
                            || m.delete
                            || m.detail
                            || m.paste.is_some();
                        acts.extend(m.acts);
                        ins_btn |= m.insert;
                        ins_before |= m.insert_before;
                        if let Some(p) = m.paste {
                            menu_paste = Some(p);
                        }
                        del_btn |= m.delete;
                        open_detail |= m.detail;
                    });
                });
            let fresh = std::mem::take(&mut st.kbd_fresh);
            if done
                || ui.input(|i| i.key_pressed(Key::Escape))
                || (!fresh && area.response.clicked_elsewhere())
            {
                st.kbd_menu = None;
                st.active = true;
            }
            if open_detail {
                st.detail = true;
            }
        }
    }
    // 選んだ範囲の書き出し（物体面・像面は除く）
    if export_btn {
        match sel_span.map(|[a, b]| [a.max(1), b.min(last.saturating_sub(1))]) {
            Some([a, b]) if a <= b => acts.push(Act::ExportRange([a, b])),
            _ => acts.push(Act::Refused(
                tr("書き出す面を選んでください（物体面・像面は除く）").into(),
            )),
        }
    }
    // 面の挿入・削除（serve の edit の insert_surface・delete_surface）
    if ins_btn || del_btn {
        let cur = st.cur.map(|x| x.0).or(rows.first().copied());
        match cur {
            _ if c.combo => acts.push(Act::Refused(
                tr("組合せの系では面を足し引きできない（要素で）").into(),
            )),
            None => {}
            Some(j) => {
                // たたんだブロックの行は、ブロックの最初の面
                let first = folded(j).map_or(j, |(_, a)| a);
                let lock = locked_at(c.blocks, c.locked, first);
                if let Some(n) = lock {
                    acts.push(Act::Refused(trf("{} はロック中", &[&n])));
                } else if ins_btn {
                    // 後に: たたんだブロックの行はブロックの最後の面の後。像面の後には足せないので像面の前
                    let at = if ins_before {
                        first.max(1)
                    } else {
                        (j + 1).min(last).max(1)
                    };
                    let after = !ins_before && at == j + 1;
                    let mut ops = vec![
                        serde_json::json!({ "op": "insert_surface", "at": at, "surface": {} }),
                    ];
                    // ブロックの中を見ているとき、そのブロック（と外のブロック）の最初の面の前に足した面もブロックに入れる
                    // （入れないとブロックの外になって表から消える）
                    ops.extend(grow_blocks(c.blocks, scope, at, after));
                    acts.push(Act::Ops(ops));
                    st.ins_at = Some((at, c.rows.len()));
                } else if selected.len() > 1 {
                    // 選んだ範囲の面をまとめて消す（後ろから。OBJ・IMA・ブロックの行・絞り・ロックした面は残す）
                    let mut del: Vec<usize> = selected
                        .iter()
                        .copied()
                        .filter(|&k| {
                            folded(k).is_none()
                                && k != 0
                                && k + 1 != c.rows.len()
                                && !c.rows[k].stop
                                && locked_at(c.blocks, c.locked, k).is_none()
                        })
                        .collect();
                    del.sort_unstable_by(|a, b| b.cmp(a));
                    if del.is_empty() {
                        acts.push(Act::Refused(tr("選んだ範囲に消せる面が無い").into()));
                    } else {
                        acts.push(Act::Ops(
                            del.iter()
                                .map(|&k| serde_json::json!({ "op": "delete_surface", "surface": k }))
                                .collect(),
                        ));
                        st.anchor = None;
                        st.cur = Some((
                            del[del.len() - 1].min(c.rows.len() - 2),
                            st.cur.map_or(Col::R, |x| x.1),
                        ));
                    }
                } else if folded(j).is_some() {
                    acts.push(Act::Refused(
                        tr("ブロックの行は消せない（中に入って面を消す）").into(),
                    ));
                } else if j == 0 || j + 1 == c.rows.len() {
                    acts.push(Act::Refused(tr("物体面・像面は消せない").into()));
                } else {
                    acts.push(Act::Ops(vec![
                        serde_json::json!({ "op": "delete_surface", "surface": j }),
                    ]));
                }
            }
        }
    }
    if let Some((j, col, typed)) = start {
        st.cur = Some((j, col));
        let r = &c.rows[j];
        // 打った文字から（その文字だけ）、Enter・F2・ダブルクリックは今の値（丸めない値）を全部選んで
        let (text, select_all) = match typed {
            Some(t) => (t, false),
            None => (col.edit_text(r), true),
        };
        st.edit = Some(Editing {
            j,
            col,
            text,
            focused: false,
            select_all,
            pick: 0,
            list_hover: false,
            filtered: !select_all,
            moved: false,
            scroll: true,
        });
        if col == Col::Material && c.glasses.is_empty() {
            acts.push(Act::NeedGlasses);
        }
    }
    if let Some((j, col, text)) = commit {
        st.edit = None;
        st.active = true;
        if let Some(a) = commit_act(&c.rows[j], j, col, &text) {
            acts.push(a);
        }
        // Tab・↑↓ で決めたら隣のセルへ
        if let Some((dr, dc)) = after {
            let pos = rows.iter().position(|&x| x == j).unwrap_or(0) as i32;
            let ci = cols.iter().position(|&x| x == col).unwrap_or(0) as i32;
            let p = (pos + dr).clamp(0, rows.len() as i32 - 1) as usize;
            let k = (ci + dc).clamp(0, cols.len() as i32 - 1) as usize;
            st.cur = Some((rows[p], cols[k]));
            st.hscroll = dc != 0;
        }
    }
    if let Some(name) = toggle {
        if st.open.remove(&name) {
            // たたんだブロックの中の面を選んでいたら、たたんだ行（続く面の最後）へ
            if let (Some((j, col)), Some(b)) = (st.cur, c.blocks.iter().find(|b| b.name == name)) {
                if b.surfs.contains(&j) {
                    st.cur = Some((run_end(b, j), col));
                    st.edit = None;
                }
            }
        } else if let Some(path) = path_to(c.blocks, &name) {
            // 外のブロックも開く（たたんだ外のブロックの中では見えないので）
            st.open.extend(path);
        }
        st.active = true;
    }
    if let Some(name) = enter {
        st.enter(c.blocks, &name);
        st.active = true;
    }
    // 右クリックの貼り付け: クリップボードを頼み、次のフレームの Paste で貼る
    if let Some(p) = menu_paste {
        st.paste_insert = Some(p);
        st.active = true;
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
    }
    // 選んだ面の設定（たたんだブロックの行・ブロックだけを選んだときは、ブロックの設定をパネルの側で）
    let mut bottom = bottom;
    st.block_view = None;
    // 詳細設定の帯: 閉じているときは下に折りたたんだ帯、開いているときは欄の上の見出し（どちらもクリックで開閉）
    let what = match st.cur.map(|x| x.0).filter(|&j| j < c.rows.len()) {
        Some(j) => match folded(j) {
            Some((i, _)) => trf("ブロック {}", &[&c.blocks[i].name]),
            None => trf("面 {}", &[&j]),
        },
        None => st
            .sel
            .clone()
            .map_or(String::new(), |b| trf("ブロック {}", &[&b])),
    };
    if bottom.max_rect().height() > 1.0 || st.detail {
        let label = format!(
            "{} {}{}",
            if st.detail { "▼" } else { "▶" },
            tr("詳細設定"),
            if what.is_empty() {
                String::new()
            } else {
                format!(" — {what}")
            }
        );
        // 表の下の 1 行: 詳細設定の開閉と、表の使い方の案内（長ければ切る。ポインタを当てると全部）
        bottom.horizontal(|ui| {
            let r = ui
                .add(
                    egui::Label::new(RichText::new(label).small().color(weak))
                        .sense(Sense::click()),
                )
                .on_hover_text(tr(
                    "開口・前後の傾き偏心・形のデータ・全項目 JSON とブロックの設定（クリックで開く・閉じる）",
                ));
            if r.clicked() {
                st.detail = !st.detail;
            }
            let hint = tr(
                "ソルブ・ピックアップはセル右端の […]、dn・分散・複屈折は硝材の […]、絞り・無視・挿入・貼り付けは右クリック（Shift+F10）。キーの一覧は F1",
            );
            ui.add(egui::Label::new(RichText::new(hint).small().color(weak)).truncate())
                .on_hover_text(hint);
        });
    }
    if !st.detail {
        return (acts, bottom);
    }
    match st.cur.map(|x| x.0).filter(|&j| j < c.rows.len()) {
        Some(j) if folded(j).is_none() => {
            if st.insp.data.as_ref().map_or(true, |d| d.0 != j) && st.insp.asked != Some(j) {
                st.insp.asked = Some(j);
                acts.push(Act::NeedData(j));
            }
            let ro = locked_at(c.blocks, c.locked, j)
                .map(|n| trf("{} はロック中", &[&n]))
                .or_else(|| (c.read_only)(j, Col::R));
            egui::ScrollArea::vertical()
                .id_salt("surf_edit")
                .auto_shrink([false, false])
                .show(&mut bottom, |ui| {
                    acts.extend(crate::surf_edit::ui(ui, &mut st.insp, j, ro));
                });
        }
        Some(j) => {
            st.block_view = folded(j).map(|(i, _)| c.blocks[i].name.clone());
        }
        None => match st.sel.clone() {
            Some(b) => st.block_view = Some(b),
            None => {
                bottom.weak(tr(
                    "面かブロックを選ぶと、その設定をここで変えられます（ブロックはダブルクリック・Enter で中へ、マウスの戻るボタン・Backspace で外へ）。",
                ));
            }
        },
    }
    (acts, bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(name: &str, spans: &[[usize; 2]], auto: bool) -> Unit {
        Unit {
            name: name.into(),
            spans: spans.to_vec(),
            auto,
            ..Default::default()
        }
    }

    #[test]
    fn tree_nests_by_surfaces_and_hides_auto_lenses_inside_blocks() {
        let units = vec![
            unit("lens1", &[[3, 4]], true),
            unit("lens2", &[[9, 10]], true),
            unit("lens3", &[[12, 13]], true),
            unit("L1", &[[2, 5]], false),
            unit("G", &[[6, 10]], false),
            unit("M1", &[[6, 8]], false),
            unit("L2", &[[9, 10]], false),
        ];
        let t = tree(&units);
        let names: Vec<&str> = t.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["L1", "G", "M1", "L2", "lens3"]);
        let parent = |n: &str| {
            let b = t.iter().find(|b| b.name == n).unwrap();
            b.parent.map(|p| t[p].name.as_str())
        };
        assert_eq!(parent("L1"), None);
        assert_eq!(parent("M1"), Some("G"));
        assert_eq!(parent("L2"), Some("G"));
        assert_eq!(parent("lens3"), None);
        assert_eq!(path_to(&t, "L2").unwrap(), ["G", "L2"]);
        let locked: BTreeSet<String> = ["G".to_string()].into();
        assert_eq!(locked_at(&t, &locked, 9).as_deref(), Some("G"));
        assert_eq!(locked_at(&t, &locked, 3), None);
        assert!(block_locked(&t, &locked, 3));
    }

    #[test]
    fn blocks_fold_to_one_line() {
        // 面 0..12、L1 = 2..5、G = 6..10（往路）と 12..13（復路のつもり）
        let blk = |j: usize| match j {
            2..=5 => Some(0),
            6..=10 | 12..=13 => Some(1),
            _ => None,
        };
        let surfs: Vec<usize> = (0..15).collect();
        let v = fold_lines(&surfs, blk);
        assert_eq!(
            v,
            vec![
                (0, None),
                (1, None),
                (5, Some((0, 2))),
                (10, Some((1, 6))),
                (11, None),
                (13, Some((1, 12))),
                (14, None),
            ]
        );
    }

    #[test]
    fn open_blocks_show_their_surfaces_in_place() {
        // 面 0..12、L1 = 2..5（中に lens = 3..4）、G = 6..10
        let blocks = vec![
            Block {
                name: "L1".into(),
                surfs: (2..=5).collect(),
                parent: None,
            },
            Block {
                name: "lens".into(),
                surfs: vec![3, 4],
                parent: Some(0),
            },
            Block {
                name: "G".into(),
                surfs: (6..=10).collect(),
                parent: None,
            },
        ];
        let surfs: Vec<usize> = (0..12).collect();
        let keys = |v: &[Line]| {
            v.iter()
                .map(|l| (l.key, l.fold.map(|f| f.0)))
                .collect::<Vec<_>>()
        };
        // たたんだまま（階層の表示と同じ）
        let v = lines_of(&blocks, None, &surfs, &|_| false, 0);
        assert_eq!(
            keys(&v),
            [
                (0, None),
                (1, None),
                (5, Some(0)),
                (10, Some(2)),
                (11, None)
            ]
        );
        // L1 を開く: 中の lens はたたんだ 1 行
        let v = lines_of(&blocks, None, &surfs, &|i| i == 0, 0);
        assert_eq!(
            keys(&v),
            [
                (0, None),
                (1, None),
                (2, None),
                (4, Some(1)),
                (5, None),
                (10, Some(2)),
                (11, None)
            ]
        );
        assert_eq!(v[2].heads, [0]);
        assert_eq!((v[2].depth, v[2].blk), (1, Some(0)));
        assert_eq!((v[3].depth, v[3].blk), (1, Some(1)));
        // L1 と lens を開く: 面 3 から lens も始まる
        let v = lines_of(&blocks, None, &surfs, &|i| i <= 1, 0);
        let k: Vec<usize> = v.iter().map(|l| l.key).collect();
        assert_eq!(k, [0, 1, 2, 3, 4, 5, 10, 11]);
        assert_eq!(v[3].heads, [1]);
        assert_eq!((v[3].depth, v[3].blk), (2, Some(1)));
        assert_eq!((v[5].depth, v[5].blk), (1, Some(0)));
        // 先頭の面からいくつものブロックが始まる
        let b2 = vec![
            blocks[0].clone(),
            Block {
                name: "in".into(),
                surfs: vec![2, 3],
                parent: Some(0),
            },
        ];
        let v = lines_of(&b2, None, &surfs, &|_| true, 0);
        assert_eq!(v[2].heads, [0, 1]);
        assert_eq!(run_end(&blocks[2], 7), 10);
        // たたんだ中の面は、たたんだ行へ
        let v = lines_of(&blocks, None, &surfs, &|_| false, 0);
        assert_eq!(visible_key(&v, &blocks, 3), Some(5));
        assert_eq!(visible_key(&v, &blocks, 7), Some(10));
        assert_eq!(visible_key(&v, &blocks, 1), Some(1));
        assert_eq!(visible_key(&v[..2], &blocks, 7), None);
    }

    #[test]
    fn parse_surface_reply() {
        let v = json!({
            "surfaces": [
                {"type": "STANDARD", "comment": "", "r": {"$f": "inf"}, "t": {"$f": "inf"}, "material": "", "coating": "", "semi": 0.0, "k": 0.0, "stop": false},
                {"type": "STANDARD", "comment": "front", "r": 50.0, "t": 4.0, "material": "N-BK7", "coating": "AR", "semi": 10.0, "k": -1.0, "stop": true},
            ],
            "units": [{"name": "lens1", "spans": [[1, 2]], "auto": true}],
        });
        let (rows, units) = parse(&v);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].r.is_infinite());
        assert_eq!(Col::R.text(&rows[0]), "Infinity");
        assert_eq!(rows[1].comment, "front");
        assert!(rows[1].stop);
        assert_eq!(Col::Conic.text(&rows[1]), "-1");
        assert_eq!(units, vec![unit("lens1", &[[1, 2]], true)]);
    }

    #[test]
    fn parse_input() {
        assert_eq!(Col::R.parse("inf").unwrap(), json!(0.0));
        assert_eq!(Col::R.parse("").unwrap(), json!(0.0));
        assert_eq!(Col::R.parse(" -40.5 ").unwrap(), json!(-40.5));
        assert!(Col::T.parse("abc").is_err());
        assert!(Col::T.parse("").is_err());
        assert_eq!(Col::Material.parse(" F2 ").unwrap(), json!("F2"));
    }

    #[test]
    fn element_spans_fill_unnamed_elements() {
        let rows: Vec<Row> = (0..12)
            .map(|j| Row {
                stop: j == 1,
                ..Default::default()
            })
            .collect();
        let units = vec![unit("L1", &[[2, 5]], false), unit("L2", &[[9, 10]], false)];
        let ids: Vec<String> = ["L1", "BS1", "L2"].iter().map(|s| s.to_string()).collect();
        let v = elem_spans(&ids, &units, true, &rows);
        assert_eq!(v, vec![Some([2, 5]), Some([6, 8]), Some([9, 10])]);
    }

    #[test]
    fn type_column_matches_by_prefix_then_description() {
        assert_eq!(type_matches("ev")[0], "EVENASPH");
        assert_eq!(type_matches("co")[0], "COORDBRK");
        assert!(type_matches("非球面").contains(&"EVENASPH"));
        assert_eq!(type_matches("").len(), TYPE_NAMES.len());
        assert!(type_matches("xyz").is_empty());
        assert_eq!(Col::Type.parse("st"), Ok(json!("STANDARD")));
        assert!(Col::Type.parse("xyz").is_err());
    }

    /// セルの「多重構成の行にする」の名前は core の多重構成の名前として読める
    #[test]
    fn config_param_names() {
        let cb = Row {
            typ: "COORDBRK".into(),
            ..Default::default()
        };
        let asph = Row {
            typ: "EVENASPH".into(),
            ..Default::default()
        };
        let std = Row {
            typ: "STANDARD".into(),
            ..Default::default()
        };
        let cases = [
            (config_param(&std, 2, Col::R), Some("2.r")),
            (config_param(&std, 2, Col::T), Some("2.t")),
            (config_param(&std, 2, Col::Conic), Some("2.k")),
            (config_param(&std, 2, Col::Material), Some("2.dn")),
            (config_param(&std, 2, Col::Semi), None),
            (config_param(&asph, 3, Col::Par(1)), Some("3.a2")),
            (config_param(&asph, 3, Col::Par(4)), Some("3.a8")),
            (config_param(&cb, 4, Col::Par(3)), Some("4.tx")),
            (config_param(&cb, 4, Col::Par(6)), None),
        ];
        for (got, want) in cases {
            assert_eq!(got.as_deref(), want);
            if let Some(n) = got {
                let k = n.split_once('.').unwrap().1;
                lensbench_core::param::Kind::parse(k).unwrap();
            }
        }
    }

    #[test]
    fn parameters_follow_opticstudio_numbers() {
        let cb = Row {
            typ: "COORDBRK".into(),
            cb: vec![1.0, 2.0, 3.0, 4.0, 5.0],
            cb_order: 1.0,
            ..Default::default()
        };
        assert_eq!(par_count("COORDBRK"), 6);
        assert_eq!(par_value(&cb, 3), Some(3.0));
        assert_eq!(par_value(&cb, 6), Some(1.0));
        assert_eq!(par_value(&cb, 7), None);
        let f = par_fields(&cb, 2, 9.0).unwrap();
        assert_eq!(f["cb"], json!([1.0, 9.0, 3.0, 4.0, 5.0]));
        assert!(par_fields(&cb, 6, 2.0).is_err());
        let asph = Row {
            typ: "EVENASPH".into(),
            asph: vec![0.0, 1e-5],
            ..Default::default()
        };
        assert_eq!(par_count("EVENASPH"), 8);
        assert_eq!(par_value(&asph, 2), Some(1e-5));
        assert_eq!(par_value(&asph, 5), Some(0.0));
        assert_eq!(
            par_fields(&asph, 4, 2e-9).unwrap()["asph"],
            json!([0.0, 1e-5, 0.0, 2e-9])
        );
        let tor = Row {
            typ: "TOROIDAL".into(),
            cx: 0.02,
            ..Default::default()
        };
        assert_eq!(par_value(&tor, 1), Some(50.0));
        assert_eq!(par_fields(&tor, 1, 25.0).unwrap()["cx"], json!(0.04));
        assert_eq!(par_count("STANDARD"), 0);
        assert_eq!(par_count("PARAXIAL"), 1);
        assert_eq!(par_count("JONESMAT"), 8);
    }

    #[test]
    fn commit_only_changed_values() {
        let r = Row {
            typ: "STANDARD".into(),
            r: f64::INFINITY,
            t: 5.0,
            ..Default::default()
        };
        assert_eq!(commit_act(&r, 1, Col::T, "5"), None);
        assert_eq!(commit_act(&r, 1, Col::R, ""), None);
        assert_eq!(
            commit_act(&r, 1, Col::R, "-20"),
            Some(Act::Edit {
                j: 1,
                col: Col::R,
                value: json!(-20.0)
            })
        );
        assert_eq!(
            commit_act(&r, 1, Col::Type, "ev"),
            Some(Act::Edit {
                j: 1,
                col: Col::Type,
                value: json!("EVENASPH")
            })
        );
        assert!(matches!(
            commit_act(&r, 1, Col::T, "x"),
            Some(Act::Refused(_))
        ));
        let cb = Row {
            typ: "COORDBRK".into(),
            cb: vec![0.0; 5],
            ..Default::default()
        };
        assert!(matches!(
            commit_act(&cb, 2, Col::Par(4), "10"),
            Some(Act::Fields { j: 2, .. })
        ));
    }

    #[test]
    fn copy_and_paste_rows_as_tab_separated_text() {
        let rows = vec![
            Row {
                typ: "STANDARD".into(),
                r: f64::INFINITY,
                t: 10.0,
                ..Default::default()
            },
            Row {
                typ: "STANDARD".into(),
                r: 50.0,
                t: 5.0,
                material: "N-BK7".into(),
                semi: 5.0,
                ..Default::default()
            },
            Row {
                typ: "STANDARD".into(),
                r: -50.0,
                t: 40.0,
                semi: 5.0,
                ..Default::default()
            },
            Row {
                typ: "STANDARD".into(),
                r: f64::INFINITY,
                ..Default::default()
            },
        ];
        let cols = Col::all(8);
        let t = copy_text(&rows, &[1, 2], &cols);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].starts_with("STANDARD\t\t50\t5\tN-BK7\t\t5\t0"),
            "{}",
            lines[0]
        );
        let ro = |_: usize, _: Col| None;
        // 1 行を別の行へ（Excel の \r\n も）: 違う値だけ変える
        let acts = paste_acts(&rows, &[2], &cols, 0, &format!("{}\r\n", lines[0]), &ro);
        assert_eq!(acts.len(), 1);
        let Act::Fields { j: 2, fields } = &acts[0] else {
            panic!("{acts:?}")
        };
        assert_eq!(fields.len(), 3, "{fields:?}"); // r・t・material
        assert_eq!(fields["material"], json!("N-BK7"));
        // 面を挿入して貼り付け: 2 行なら面 1 の前に 2 面を挿入し、挿入した面 1・2 に値を書く
        let acts = insert_paste_acts(&rows, &[], None, 1, &cols, &t);
        let Act::Ops(ops) = &acts[0] else {
            panic!("{acts:?}")
        };
        assert_eq!(
            ops,
            &vec![
                json!({ "op": "insert_surface", "at": 1, "surface": {} }),
                json!({ "op": "insert_surface", "at": 2, "surface": {} }),
            ]
        );
        let js: Vec<usize> = acts[1..]
            .iter()
            .map(|a| match a {
                Act::Fields { j, fields } => {
                    assert!(fields.contains_key("t"), "{fields:?}");
                    *j
                }
                _ => panic!("{a:?}"),
            })
            .collect();
        assert_eq!(js, vec![1, 2]);
        // 1 つの値を選んだ範囲の行の今の列に
        let acts = paste_acts(&rows, &[1, 2], &cols, 3, "7", &ro);
        assert_eq!(acts.len(), 2);
        // 種類を変えてからパラメータ（EVENASPH の r⁴）
        let acts = paste_acts(
            &rows,
            &[1],
            &[Col::Type, Col::Par(1), Col::Par(2)],
            0,
            "EVENASPH\t0\t1e-5",
            &ro,
        );
        assert_eq!(
            acts[0],
            Act::Edit {
                j: 1,
                col: Col::Type,
                value: json!("EVENASPH")
            }
        );
        let Act::Fields { fields, .. } = &acts[1] else {
            panic!("{acts:?}")
        };
        assert_eq!(fields["asph"], json!([0.0, 1e-5]));
        // 変えられないセルは飛ばす
        let no = |_: usize, _: Col| Some(String::from("x"));
        assert!(matches!(
            paste_acts(&rows, &[1], &cols, 3, "7", &no)[0],
            Act::Refused(_)
        ));
        // 選択の範囲
        assert_eq!(selection(&[0, 1, 2, 3], Some(3), Some(1)), vec![1, 2, 3]);
        assert_eq!(selection(&[0, 1, 2, 3], None, Some(2)), vec![2]);
    }

    #[test]
    fn inserting_at_the_start_of_the_entered_block_keeps_the_new_surface_inside() {
        let units = vec![
            unit("G12", &[[3, 6], [8, 11]], false),
            unit("G2", &[[3, 4], [10, 11]], false),
        ];
        let b = tree(&units);
        let g12 = b.iter().position(|x| x.name == "G12");
        let ops = grow_blocks(&b, g12, 3, false);
        assert_eq!(
            ops,
            vec![json!({ "op": "set_unit", "unit": "G12", "spans": [[3, 7], [9, 12]] })]
        );
        // 途中の面の前なら範囲はもともと伸びる
        assert!(grow_blocks(&b, g12, 5, false).is_empty());
        // 中のブロック G2 に入っていれば G2 と、同じ所から始まる G12 も
        let g2 = b.iter().position(|x| x.name == "G2");
        assert_eq!(grow_blocks(&b, g2, 3, false).len(), 2);
        assert!(grow_blocks(&b, None, 3, false).is_empty());
        // 最後の面の後に足す（Insert）: 面 6 の後（at 7）なら最初の組が伸びる
        assert_eq!(
            grow_blocks(&b, g12, 7, true),
            vec![json!({ "op": "set_unit", "unit": "G12", "spans": [[3, 7], [9, 12]] })]
        );
        // 途中の面の後なら範囲はもともと伸びる
        assert!(grow_blocks(&b, g12, 5, true).is_empty());
    }

    /// 鏡の後の戻り（ダブルパス）: 面 4・5 を面 1・2 から逆の順に。曲率はそのまま、間の硝材は元の間、厚さは符号を変える
    #[test]
    fn range_pickup_return_and_flip() {
        use lensbench_core::edit::pick_kind_parse;
        use lensbench_core::param::Pickup;
        let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS ___BLANK 1 0 1.62 36.4 0 0 0 0 0\n  CONI -0.5\n\
SURF 2\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 10\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ -10\n  GLAS MIRROR 0 0 1.5 40 0 0 0 0 0 0\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ -1\n\
SURF 5\n  TYPE STANDARD\n  CURV 0.0\n  DISZ -20\n\
SURF 6\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
        let raw = lensbench_core::zmx::parse_text(txt).unwrap();
        let base = lensbench_core::zmx::build(&raw, &Default::default(), &[], None).unwrap();
        let rows: Vec<Row> = base
            .surfaces
            .iter()
            .map(|d| Row {
                typ: "STANDARD".into(),
                material: d.mat.clone(),
                ..Default::default()
            })
            .collect();
        let apply = |mode: RangePick, dst: &[usize]| {
            let mut s = base.clone();
            for op in range_pick_ops(&rows, dst, &[1, 2], mode).unwrap() {
                let w = pick_kind_parse(op["what"].as_str().unwrap()).unwrap();
                let p = Pickup::new(
                    op["dst"].as_u64().unwrap() as usize,
                    op["src"].as_u64().unwrap() as usize,
                    w,
                    op["scale"].as_f64().unwrap(),
                    0.0,
                );
                s.add_pickup(p).unwrap();
            }
            s
        };
        let s = apply(RangePick::Return, &[4, 5]);
        let d = &s.surfaces;
        assert_eq!((d[4].c, d[5].c), (-0.01, 0.02));
        assert_eq!((d[4].k, d[5].k), (0.0, -0.5));
        assert_eq!(d[4].mat, d[1].mat);
        assert_eq!(d[4].n, d[1].n);
        assert_eq!(d[4].t, -4.0);
        // 戻りの最後の面の後（玉の外）は変えない
        assert_eq!(d[5].t, -20.0);
        // 元の値を変えると追従する
        let mut s2 = s.clone();
        s2.set_material(1, "1.7,30").unwrap();
        assert_eq!(s2.surfaces[4].mat, "1.7,30");
        // 反転: 曲率の符号が変わり、厚さはそのまま
        let s = apply(RangePick::Flip, &[4, 5]);
        assert_eq!((s.surfaces[4].c, s.surfaces[5].c), (0.01, -0.02));
        assert_eq!(s.surfaces[4].t, 4.0);
        // 数が合わない・重なりはエラー
        assert!(range_pick_ops(&rows, &[4], &[1, 2], RangePick::Same).is_err());
        assert!(range_pick_ops(&rows, &[2, 3], &[1, 2], RangePick::Same).is_err());
        let b = vec![Block {
            name: "L1".into(),
            surfs: vec![1, 2],
            parent: None,
        }];
        assert_eq!(range_src("L1", &b, 9).unwrap(), vec![1, 2]);
        assert_eq!(range_src("3-5", &b, 9).unwrap(), vec![3, 4, 5]);
        assert_eq!(range_src("3", &b, 2).unwrap(), vec![3, 4]);
        assert!(range_src("x", &b, 2).is_err());
    }
}
