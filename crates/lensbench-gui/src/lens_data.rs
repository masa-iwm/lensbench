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

mod cell_menu;
mod lines;
mod names;
mod strip;
mod table;
use cell_menu::{cell_menu, override_item};
pub(crate) use names::TYPE_NAMES;

use std::collections::BTreeSet;

#[cfg(test)]
use lines::{fold_lines, lines_of, run_end, visible_key, Line};
pub(crate) use table::ui;

use egui::{RichText, Ui};
use lensbench_engine::session::views;
use serde_json::{json, Map, Value};

use crate::i18n::{n_, tr, trf};
use crate::json_view::fmt_num;
use crate::theme;

/// 表の 1 行（1 つの面）
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Row {
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
    pub(crate) fn is_cb(&self) -> bool {
        self.typ == "COORDBRK"
    }
}

/// 玉・群（serve の surface の units）
pub(crate) use lensbench_engine::session::views::UnitView as Unit;

/// ソルブ（serve の solves の 1 つ）の量と説明
fn solve_text(q: &views::SolveView) -> (String, String) {
    let mut t = format!("{} {}", q.kind, fmt_num(q.value));
    if let Some(o) = q.other {
        t += &trf("(surface {})", &[&o]);
    }
    (q.what.clone(), t)
}

/// ピックアップ（serve の pickups の 1 つ）の量と説明
fn pickup_text(p: &views::PickupView) -> (String, String) {
    let t = format!(
        "{} × {} {} + {}",
        fmt_num(p.scale),
        trf("Surface {}", &[&p.src]),
        p.from.as_deref().unwrap_or(&p.what),
        fmt_num(p.offset),
    );
    (p.what.clone(), t)
}

/// serve の `surface`（全部の面）の結果を engine の型で読む（鍵が足りない・綴りが違えば誤り）
#[cfg(test)]
pub(crate) fn parse(v: &Value) -> Result<(Vec<Row>, Vec<Unit>), String> {
    let r: views::SurfacesView = views::read(v).map_err(|e| format!("surface: {e}"))?;
    Ok(parse_view(r))
}

/// 面の表の型（[`views::SurfacesView`]）を、表の行と玉・群に
pub(crate) fn parse_view(r: views::SurfacesView) -> (Vec<Row>, Vec<Unit>) {
    let rows = r
        .surfaces
        .into_iter()
        .map(|f| Row {
            solves: f.solves.iter().map(solve_text).collect(),
            pickups: f.pickups.iter().map(pickup_text).collect(),
            typ: f.typ,
            comment: f.comment,
            r: f.r,
            t: f.t,
            material: f.material,
            coating: f.coating,
            semi: f.semi,
            semi_auto: f.semi_auto,
            k: f.k,
            stop: f.stop,
            ignore: f.ignore,
            asph: f.asph,
            cx: f.cx,
            cb: f.cb,
            cb_order: f.cb_order,
            focal: f.focal,
            jones: f.jones,
            dn: f.dn,
            disp: f.disp,
        })
        .collect();
    (rows, r.units)
}

/// 表のブロック（木の節）
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Block {
    pub name: String,
    /// 面（光の通る順、重なりなし）
    pub surfs: Vec<usize>,
    /// 親のブロック（None は最上位）
    pub parent: Option<usize>,
}

/// 玉・群を面の範囲の入れ子で木にする。読込で作った玉は、名前のあるブロックの中にあれば外す。同じ面の組は最初の 1 つ
pub(crate) fn tree(units: &[Unit]) -> Vec<Block> {
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

/// 組合せの要素ごとの面の範囲（最初・最後）。要素のブロックの id で serve の `blocks` から引く（組合せの図と同じ、
/// [`crate::combo::LayoutData::spans_for`]）
pub(crate) fn elem_spans(
    ids: &[String],
    blocks: &[(String, Vec<[usize; 2]>)],
    lead_stop: bool,
    rows: &[Row],
) -> Vec<Option<[usize; 2]>> {
    // ブロックの表は写さない（画面ごとに呼ぶ）
    crate::combo::spans_in(blocks, rows.iter().position(|r| r.stop), ids, lead_stop)
}

/// 表の列（変えられる値）。Par は面タイプのパラメータ（OpticStudio の Par 1〜8 と同じ番号）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Col {
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
pub(crate) const MAX_PAR: u8 = 8;

/// 種類の列に打った文字に合う面タイプ（前方一致を先に、次に名前・説明の部分一致）
pub(crate) fn type_matches(s: &str) -> Vec<&'static str> {
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
    n_("r² coeff"),
    n_("r⁴ coeff"),
    n_("r⁶ coeff"),
    n_("r⁸ coeff"),
    n_("r¹⁰ coeff"),
    n_("r¹² coeff"),
    n_("r¹⁴ coeff"),
    n_("r¹⁶ coeff"),
];
const TOR_LABELS: [&str; 7] = [
    n_("y² coeff"),
    n_("y⁴ coeff"),
    n_("y⁶ coeff"),
    n_("y⁸ coeff"),
    n_("y¹⁰ coeff"),
    n_("y¹² coeff"),
    n_("y¹⁴ coeff"),
];
const CB_LABELS: [&str; 6] = [
    n_("Decenter X"),
    n_("Decenter Y"),
    n_("Tilt X"),
    n_("Tilt Y"),
    n_("Tilt Z"),
    n_("Sequence"),
];
const JONES_LABELS: [&str; 8] = [
    n_("A re"),
    n_("A im"),
    n_("B re"),
    n_("B im"),
    n_("C re"),
    n_("C im"),
    n_("D re"),
    n_("D im"),
];

/// レンズデータの左の帯のアイコン
const ICONS: [&str; 11] = ["🗀", "▶", "▼", "▦", "▣", "▭", "＋", "－", "⚙", "⊞", "📤"];

/// レンズデータの左の帯のアイコンの字（帯の幅があるとき）
fn icon_label(t: &str) -> Option<&'static str> {
    Some(match t {
        "🗀" => "Drill-down",
        "▶" | "▼" => "Fold",
        "▦" => "Blocks",
        "▣" | "▭" => "Select",
        "＋" => "Insert",
        "－" => "Delete",
        "⚙" => "Details",
        "⊞" => "Lenses / groups",
        "📤" => "Export",
        _ => return None,
    })
}

/// tr で引く名前（パラメータの名前・面タイプの説明。訳の表の検査用）
#[cfg(test)]
pub(crate) fn labels() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = TYPE_NAMES.iter().map(|x| x.1).collect();
    v.extend([
        "Drill-down",
        "Fold",
        "Blocks",
        "Select",
        "Insert",
        "Delete",
        "Details",
        "Lenses / groups",
        "Export",
    ]);
    v.extend(ASPH_LABELS);
    v.extend(TOR_LABELS);
    v.extend(CB_LABELS);
    v.extend(JONES_LABELS);
    v.extend(["Radius of rotation", "Focal length"]);
    v
}

/// 面タイプ typ のパラメータ i（1〜）の名前。そのタイプに無ければ None（番号は計算側の surf_parm と同じ）
pub(crate) fn par_label(typ: &str, i: u8) -> Option<&'static str> {
    let i = i as usize;
    let l = match typ {
        "EVENASPH" | "FRESNELS" | "CFRESNEL" | "BINARY_2" => ASPH_LABELS.get(i.checked_sub(1)?)?,
        "TOROIDAL" if i == 1 => "Radius of rotation",
        "TOROIDAL" => TOR_LABELS.get(i.checked_sub(2)?)?,
        "COORDBRK" => CB_LABELS.get(i.checked_sub(1)?)?,
        "PARAXIAL" if i == 1 => "Focal length",
        "JONESMAT" => JONES_LABELS.get(i.checked_sub(1)?)?,
        _ => return None,
    };
    Some(tr(l))
}

/// 面タイプのパラメータの数
pub(crate) fn par_count(typ: &str) -> u8 {
    (1..=MAX_PAR)
        .take_while(|&i| par_label(typ, i).is_some())
        .count() as u8
}

/// 面 r のパラメータ i の値
pub(crate) fn par_value(r: &Row, i: u8) -> Option<f64> {
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
pub(crate) fn par_fields(r: &Row, i: u8, v: f64) -> Result<Map<String, Value>, String> {
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
                return Err(tr("Order is 0 (decenter then tilt) or 1 (tilt then decenter)").into());
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
    pub(crate) fn all(npar: u8) -> Vec<Col> {
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
    pub(crate) fn field(self) -> &'static str {
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
            Col::Type => tr("Type").into(),
            Col::Comment => tr("Comment").into(),
            Col::R => tr("Radius").into(),
            Col::T => tr("Thickness").into(),
            Col::Material => tr("Material").into(),
            Col::Coating => tr("Coating").into(),
            Col::Semi => tr("Semi-diameter").into(),
            Col::Conic => tr("Conic").into(),
            Col::Par(i) => typ
                .and_then(|t| par_label(t, i))
                .map(String::from)
                .unwrap_or_else(|| trf("Par {}", &[&i])),
        }
    }

    /// ピックアップの量の名前（serve の add_pickup の what）。種類・コメントは None。硝材は dn（材料と一緒に追従し、
    /// disp も組で足す）、膜は coating（名前を写す）
    pub(crate) fn pick_what(self) -> Option<String> {
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
    pub(crate) fn has_extra(self) -> bool {
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
    pub(crate) fn solve_what(self) -> Option<&'static str> {
        match self {
            Col::R => Some("c"),
            Col::T => Some("t"),
            _ => None,
        }
    }

    /// 面 r のこの値のソルブ・ピックアップ（説明）
    pub(crate) fn solved(self, r: &Row) -> Option<(char, String)> {
        if let Some(w) = self.solve_what() {
            if let Some((_, t)) = r.solves.iter().find(|s| s.0 == w) {
                return Some(('S', trf("Solve: {}", &[t])));
            }
        }
        let pw = self.pick_what()?;
        r.pickups
            .iter()
            .find(|p| p.0 == pw || (self == Col::R && p.0 == "c"))
            .map(|(_, t)| ('P', trf("Pickup: {}", &[t])))
    }

    pub(crate) fn text(self, r: &Row) -> String {
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
            Col::Semi if r.semi_auto => format!("{} ({})", n(r.semi), tr("Auto")),
            Col::Semi => n(r.semi),
            Col::Conic => n(r.k),
            Col::Par(i) => par_value(r, i).map(n).unwrap_or_default(),
        }
    }

    /// 書き換えの最初の文字（丸めない値）
    pub(crate) fn edit_text(self, r: &Row) -> String {
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
    pub(crate) fn has_list(self) -> bool {
        matches!(self, Col::Type | Col::Material)
    }

    /// 入力の文字を値に（曲率半径の inf・空は平面 = 0）
    pub(crate) fn parse(self, s: &str) -> Result<Value, String> {
        let s = s.trim();
        match self {
            Col::Comment | Col::Material | Col::Coating => Ok(json!(s)),
            Col::Type => type_matches(s)
                .first()
                .map(|t| json!(t))
                .ok_or_else(|| trf("No surface type named {}", &[&s])),
            Col::Semi if s.eq_ignore_ascii_case("auto") || s == "Auto" || s == tr("Auto") => {
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
                .ok_or_else(|| trf("{} is not a number", &[&s])),
        }
    }

    fn numeric(self) -> bool {
        !matches!(
            self,
            Col::Type | Col::Comment | Col::Material | Col::Coating
        )
    }
}

/// 表から GUI に頼むこと（文書を変える操作は記録のファイルにこの形で残る）
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Act {
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
    /// 面 j のデータの項目を変える（serve の edit の set_surface_data）
    Data { j: usize, data: Map<String, Value> },
    /// そのほかの編集の操作（絞り・ソルブ・ピックアップ。開いた系だけ）
    Ops(Vec<Value>),
    /// 面 j の全部の項目を読む（serve の surface_data）
    NeedData(usize),
    /// 硝材の候補（ガラスカタログ）を読む
    NeedGlasses,
    /// 最適化の変数に足す（名前は serve の set と同じ 4.t など）
    OptVar(String),
    /// 面 j の列 col の値を多重構成の表（状態の表）の行にする
    ConfigRow { j: usize, col: Col },
    /// 面 j の列 col の面の上書きを外す（組合せの系、要素の元の値に戻す）
    DropOverride { j: usize, col: Col },
    /// カタログの行を表へ落とした: 面 `after` の後に差し込む
    DropPick {
        after: usize,
        pick: Box<crate::combo::Pick>,
    },
}

/// 候補の一覧の覚え（打った文字、絞るか、ガラスの数、行の数、候補）
type ChoiceCache = (
    String,
    bool,
    usize,
    usize,
    std::sync::Arc<Vec<(String, String)>>,
);

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
    /// 決められなかった理由（欄を閉じずに赤枠で出す。Esc で取り消し）
    error: Option<String>,
    /// 候補の一覧（打った文字・絞るか・ガラスの数・行の数が変わったときだけ作り直す。毎フレーム全ガラスを
    /// 写さない）
    choices: Option<ChoiceCache>,
    /// 選んでいる候補まで一覧を送る
    scroll: bool,
}

/// 入力できるドロップダウンの候補（値、説明）。種類は面タイプ、硝材はガラスカタログ（と鏡）。filter なら text で絞る
/// （完全一致を先に、次に前方一致、部分一致）
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
                .map(|u| (u.clone(), tr("used in this system").to_string()))
                .collect();
            let all = extra
                .into_iter()
                .chain(std::iter::once((
                    "MIRROR".to_string(),
                    tr("Mirror").to_string(),
                )))
                .chain(glasses.iter().map(|g| (g.name.clone(), g.desc.clone())));
            if !filter {
                return all.collect();
            }
            let q = text.trim().to_uppercase();
            let (mut exact, mut head, mut rest) = (Vec::new(), Vec::new(), Vec::new());
            for (n, d) in all {
                let u = n.to_uppercase();
                match crate::widgets::match_rank(&u, &d.to_uppercase(), &q) {
                    Some(true) if u == q => exact.push((n, d)),
                    Some(true) => head.push((n, d)),
                    Some(false) => rest.push((n, d)),
                    None => {}
                }
            }
            exact.extend(head);
            exact.extend(rest);
            exact
        }
        _ => Vec::new(),
    }
}

/// 硝材の候補（GUI のガラスカタログの名前と、カタログ・nd・νd の説明）
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GlassChoice {
    pub name: String,
    pub desc: String,
}

/// セルの右クリックの欄の入力
#[derive(Debug, Clone, Default)]
struct CellMenu {
    /// ソルブの種類・値・param・別の面
    kind: String,
    /// ソルブの種類の一覧を欄の中に開いているか（別の小窓にすると、選んだときに親の欄が閉じて一覧だけ残る）
    kind_open: bool,
    val: String,
    param: String,
    other: String,
    /// ピックアップの元の面・倍率・オフセット
    src: String,
    scale: String,
    offset: String,
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
pub(crate) enum RangePick {
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

impl RowMenu {
    /// 右クリックの欄（4 か所）の結果をこのフレームの操作に足す。何か選んだら true
    fn merge(&mut self, m: RowMenu) -> bool {
        let any = !m.acts.is_empty() || m.insert || m.delete || m.detail || m.paste.is_some();
        self.acts.extend(m.acts);
        self.insert |= m.insert;
        self.insert_before |= m.insert_before;
        if m.paste.is_some() {
            self.paste = m.paste;
        }
        self.delete |= m.delete;
        self.detail |= m.detail;
        any
    }
}

/// 硝材の欄の文字を入れられるか: 空・AIR・MIRROR、nd,νd[,ΔPgF] のモデルガラス、カタログのガラス、表で使っている
/// ガラス。カタログをまだ読んでいなければ serve に任せる
fn material_ok(text: &str, glasses: &[GlassChoice], rows: &[Row]) -> Result<(), String> {
    let m = text.trim();
    if m.is_empty()
        || m.eq_ignore_ascii_case("AIR")
        || m.eq_ignore_ascii_case("MIRROR")
        || glasses.is_empty()
        || glasses.iter().any(|g| g.name.eq_ignore_ascii_case(m))
        || rows.iter().any(|r| r.material.eq_ignore_ascii_case(m))
    {
        return Ok(());
    }
    let v: Vec<f64> = m
        .split(',')
        .filter_map(|x| x.trim().parse::<f64>().ok())
        .collect();
    if (2..=3).contains(&v.len()) && m.split(',').count() == v.len() && v[0] > 0.0 {
        return Ok(());
    }
    Err(trf(
        "Glass {} is not in the catalogs (write a model glass as nd,νd like 1.5,60; add catalogs in File > Catalog folders…)",
        &[&m],
    ))
}

/// 書き換えた文字を、面 j（今の値 r）の列 col の変更に（今と同じなら None）
fn commit_act(r: &Row, j: usize, col: Col, text: &str) -> Option<Act> {
    // パラメータの列を空で決めたら変えない（焦点距離・非球面係数などに 0 を入れない）
    if matches!(col, Col::Par(_)) && text.trim().is_empty() {
        return None;
    }
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
        Col::Par(i) => par_value(r, i).is_none_or(same_num),
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

/// 面の表の列 col の値の、多重構成の表の面の行の値の名前（面の上書きの名前と、並びの何番目か）。行にできない列は None
pub(crate) fn mc_field(r: &Row, col: Col) -> Option<(&'static str, Option<usize>)> {
    Some(match col {
        Col::R => ("r", None),
        Col::T => ("t", None),
        Col::Conic => ("k", None),
        Col::Semi => ("semi", None),
        Col::Material => ("material", None),
        Col::Coating => ("coating", None),
        Col::Par(i) if r.typ == "EVENASPH" && (1..=8).contains(&i) => {
            ("asph", Some(i as usize - 1))
        }
        _ => return None,
    })
}

/// 面 j の列 col の多重構成の行の名前（serve の add_config_row の param）。行にできない列は None
pub(crate) fn config_param(r: &Row, j: usize, col: Col) -> Option<String> {
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

impl LensData {
    /// 足した面が表の at と違う行に入るとき（組合せの系で前の絞りの前に足すと絞りの後に入る）、今のセルを移す先を直す
    pub(crate) fn insert_lands_at(&mut self, row: usize) {
        if let Some((at, _)) = self.ins_at.as_mut() {
            *at = row;
        }
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
        acts.push(Act::Refused(
            tr("The paste target cells cannot be changed").into(),
        ));
    }
    acts
}

/// 面・玉・群のピックアップの元の面: ブロックの名前ならその面、「a-b」なら a〜b、番号 s なら s から n 面
fn range_src(text: &str, blocks: &[Block], n: usize) -> Result<Vec<usize>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err(tr("A pickup source (surface number, range or block name) is needed").into());
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
            "Unknown pickup source {} (surface number, 3-5 or block name)",
            &[&t],
        )),
    }
}

/// 面の並び dst を面の並び src からピックアップする操作（serve の edit の add_pickup）。形（曲率・コーニック・
/// 面タイプのパラメータ）と膜は対応する面（反転・戻りは逆の順）から、面の間の硝材・厚さは対応する間から。
/// 1 面だけ（面としてのピックアップ）を同じ向きでするときは、その面の後の硝材も
pub(crate) fn range_pick_ops(
    rows: &[Row],
    dst: &[usize],
    src: &[usize],
    mode: RangePick,
) -> Result<Vec<Value>, String> {
    let n = dst.len();
    if n == 0 || n != src.len() {
        return Err(trf(
            "Surface counts differ (target {}, source {})",
            &[&n, &src.len()],
        ));
    }
    if let Some(k) = dst.iter().chain(src).find(|&&k| k >= rows.len()) {
        return Err(trf("No surface {}", &[k]));
    }
    if dst.iter().any(|d| src.contains(d)) {
        return Err(tr("Pickup target and source surfaces overlap").into());
    }
    let rev = mode != RangePick::Same;
    if rev && dst.iter().chain(src).any(|&k| rows[k].is_cb()) {
        return Err(tr("A range with coordinate breaks cannot be picked up reversed").into());
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

/// 行の右クリックの欄: 絞り・無視・面の挿入・削除・面や玉・群のピックアップ・詳細設定
#[allow(clippy::too_many_arguments)]
fn row_menu(
    ui: &mut Ui,
    cm: &mut CellMenu,
    j: usize,
    r: &Row,
    locked: Option<&str>,
    last: usize,
    span: Option<[usize; 2]>,
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
            // 外すと前の絞り・絞りの要素に戻る
            if ui.button(tr("Remove stop")).clicked() {
                m.acts
                    .push(Act::Ops(vec![json!({ "op": "unset_stop", "surface": j })]));
            }
        } else if ui.button(tr("Make this surface the stop")).clicked() {
            m.acts
                .push(Act::Ops(vec![json!({ "op": "set_stop", "surface": j })]));
        }
        let mut ig = r.ignore;
        if ui.checkbox(&mut ig, tr("Ignore this surface")).changed() {
            let mut fields = Map::new();
            fields.insert("ignore".into(), json!(ig));
            m.acts.push(Act::Fields { j, fields });
        }
    });
    ui.separator();
    m.insert_before = ui.button(tr("Insert surface before (Insert)")).clicked();
    m.insert = ui.button(tr("Insert surface after (Alt+Insert)")).clicked();
    if ui.button(tr("Paste (overwrite, Ctrl+V)")).clicked() {
        m.paste = Some(false);
    }
    m.insert |= m.insert_before;
    m.delete = ui
        .add_enabled(
            !edge && !r.stop,
            egui::Button::new(tr("Delete this surface (Delete)")),
        )
        .clicked();
    ui.separator();
    ui.label(
        RichText::new(tr(
            "Groups in a combination are set in the element settings (Group)",
        ))
        .small()
        .color(theme::weak_text(ui.visuals())),
    );
    // 面・玉・群としてのピックアップ（選んだ範囲の面を、別の面・範囲・ブロックから。反転・戻りの向きも）
    ui.separator();
    let span = span.unwrap_or([j, j]);
    let dst: Vec<usize> = (span[0]..=span[1]).collect();
    ui.label(
        RichText::new(if dst.len() == 1 {
            trf("Pick up surface {} as a surface", &[&j])
        } else {
            trf(
                "Pick up surfaces {}–{} as a lens/group",
                &[&span[0], &span[1]],
            )
        })
        .strong(),
    );
    ui.add_enabled_ui(locked.is_none(), |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut cm.rp_src)
                    .hint_text(tr("Source: surface, 3-5 or block"))
                    .desired_width(150.0),
            );
            if ui.button(tr("Pickups")).clicked() {
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
                (RangePick::Same, tr("Same"), tr("Same values as the source")),
                (
                    RangePick::Flip,
                    tr("Reverse"),
                    tr("Turned-around lens: reverse surface order, negate curvature and aspheric terms (thickness unchanged)"),
                ),
                (
                    RangePick::Return,
                    tr("Return"),
                    tr("Return pass after a mirror (double pass): reverse surface order, keep curvature, negate thickness"),
                ),
            ] {
                ui.selectable_value(&mut cm.rp_mode, v, l).on_hover_text(h);
            }
        });
    });
    ui.separator();
    m.detail = ui
        .button(tr("Details (aperture, tilt/decenter, shape data)"))
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
pub(crate) struct LensData {
    /// 右クリックの貼り付けを頼んだ（Some(true) は面を挿入して）。次の Paste で使う
    paste_insert: Option<bool>,
    /// 貼り付けを頼んだ画面の回（届かなければ数回で忘れる。次の Ctrl+V が挿入にならない）
    paste_asked: u64,
    /// 入っているブロックの名前（外から順。空は系の全部）
    pub scope: Vec<String>,
    /// 選んだブロック
    pub sel: Option<String>,
    /// 選んだ印を付ける面の範囲（図で選んだ要素のうち、表のブロックで選べないもの: BS・先頭に自動で置く絞り）
    pub hl: Vec<[usize; 2]>,
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
    /// 一度でも見たブロック（新しいブロックは開いた状態で出す。たたむのは利用者が選んだときだけ）
    seen: BTreeSet<String>,
    /// 最後に見える所へスクロールした面（選んだ面が変わったときだけスクロールし、手で動かせるように）
    scrolled: Option<usize>,
}

/// 表を描くときに渡すもの
pub(crate) struct Ctx<'a> {
    pub rows: &'a [Row],
    pub blocks: &'a [Block],
    /// ロックしたブロックの名前
    pub locked: &'a BTreeSet<String>,
    /// 面 j の列 c を表で変えられない理由（ロック以外）
    pub read_only: &'a dyn Fn(usize, Col) -> Option<String>,
    /// 面 j の列 c に組合せの要素の面の上書きがある（カタログの玉・取り込み。セルに印、右クリックで外す）
    pub overridden: &'a dyn Fn(usize, Col) -> bool,
    /// 表についての注意（左端の帯の ⓘ に出す）
    pub note: Option<&'a str>,
    /// 硝材の候補（GUI のガラスカタログ）
    pub glasses: &'a [GlassChoice],
    /// ガラスカタログを読み終えた（候補が空でも）
    pub glasses_loaded: bool,
}

impl LensData {
    /// 系が変わった（別のファイル）: 階層・選択・ロックを戻す
    pub(crate) fn reset(&mut self) {
        // 表示の切り替えは系が変わっても残す
        *self = LensData {
            tree: self.tree,
            ..LensData::default()
        };
    }

    /// ブロック name に入る（name までの道をたどる）
    pub(crate) fn enter(&mut self, blocks: &[Block], name: &str) {
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
pub(crate) fn locked_at(blocks: &[Block], locked: &BTreeSet<String>, j: usize) -> Option<String> {
    blocks
        .iter()
        .find(|b| locked.contains(&b.name) && b.surfs.contains(&j))
        .map(|b| b.name.clone())
}

/// 面 j の欄 col を書き換えられないロックのブロック。ロックは中身（面・硝材）だけを止め、
/// ブロックの最後の面の厚さ（= ブロックの後の間隔）は変えられる
pub(crate) fn locked_cell(
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
pub(crate) fn block_locked(blocks: &[Block], locked: &BTreeSet<String>, mut i: usize) -> bool {
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

#[cfg(test)]
#[path = "lens_data_tests.rs"]
mod tests;
