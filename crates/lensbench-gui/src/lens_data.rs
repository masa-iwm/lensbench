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

/// 表の列（変えられる値）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Col {
    Comment,
    R,
    T,
    Material,
    Coating,
    Semi,
    Conic,
}

impl Col {
    pub const ALL: [Col; 7] = [
        Col::Comment,
        Col::R,
        Col::T,
        Col::Material,
        Col::Coating,
        Col::Semi,
        Col::Conic,
    ];

    /// 面の値の名前（serve の edit の set_surface）
    pub fn field(self) -> &'static str {
        match self {
            Col::Comment => "comment",
            Col::R => "r",
            Col::T => "t",
            Col::Material => "material",
            Col::Coating => "coating",
            Col::Semi => "semi",
            Col::Conic => "k",
        }
    }

    fn header(self) -> &'static str {
        match self {
            Col::Comment => tr("コメント"),
            Col::R => tr("曲率半径"),
            Col::T => tr("厚さ"),
            Col::Material => tr("硝材"),
            Col::Coating => tr("膜"),
            Col::Semi => tr("半径"),
            Col::Conic => tr("コーニック"),
        }
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
            Col::Comment => r.comment.clone(),
            Col::R => n(r.r),
            Col::T => n(r.t),
            Col::Material => r.material.clone(),
            Col::Coating => r.coating.clone(),
            // 自動は値の後ろに印（入力欄に auto と書くと自動、数を書くと固定）
            Col::Semi if r.semi_auto => format!("{} ({})", n(r.semi), tr("自動")),
            Col::Semi => n(r.semi),
            Col::Conic => n(r.k),
        }
    }

    /// 入力の文字を値に（曲率半径の inf・空は平面 = 0）
    pub fn parse(self, s: &str) -> Result<Value, String> {
        let s = s.trim();
        match self {
            Col::Comment | Col::Material | Col::Coating => Ok(json!(s)),
            Col::Semi if s.eq_ignore_ascii_case("auto") || s == "自動" || s == tr("自動") => {
                Ok(json!("auto"))
            }
            Col::R
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
        !matches!(self, Col::Comment | Col::Material | Col::Coating)
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
    /// 編集した系を系ファイルに保存する
    Save,
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
}

/// 表の状態
#[derive(Debug, Clone, Default)]
pub struct LensData {
    /// 入っているブロックの名前（外から順。空は系の全部）
    pub scope: Vec<String>,
    /// 選んだブロック
    pub sel: Option<String>,
    /// 今のセル（面、列）
    pub cur: Option<(usize, Col)>,
    /// 書き換え中のセル（面、列、文字、入力欄にフォーカスを渡したか）
    edit: Option<(usize, Col, String, bool)>,
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
    /// 開いた系を編集して、まだ保存していない
    pub dirty: bool,
    /// 組合せの系（絞り・ソルブ・ピックアップは開いた系だけ）
    pub combo: bool,
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

pub fn ui(ui: &mut Ui, st: &mut LensData, c: &Ctx) -> Vec<Act> {
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
    // 今のセルが見えない面（たたんだブロックの中・表示の切り替え・全部たたむの後）なら、それを含むたたんだ行へ
    if let Some((j, col)) = st.cur {
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

    // 階層（パンくず）
    let mut go: Option<usize> = None;
    let (mut ins_btn, mut del_btn) = (false, false);
    ui.horizontal_wrapped(|ui| {
        if ui
            .selectable_label(st.scope.is_empty(), tr("系"))
            .on_hover_text(tr("系の全部の面（Backspace で 1 つ上へ）"))
            .clicked()
        {
            go = Some(0);
        }
        for (n, name) in st.scope.iter().enumerate() {
            ui.label(RichText::new("›").color(weak));
            if ui
                .selectable_label(n + 1 == st.scope.len(), RichText::new(name).strong())
                .clicked()
            {
                go = Some(n + 1);
            }
        }
        // 表示の切り替え（ブロックに入って見る・その場で折りたたむ）
        ui.separator();
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
        if st.tree && !inside.is_empty() {
            if ui.small_button(tr("全部開く")).clicked() {
                st.open.extend(inside.iter().cloned());
            }
            if ui.small_button(tr("全部たたむ")).clicked() {
                for n in &inside {
                    st.open.remove(n);
                }
            }
        }
        // 面の挿入・削除（開いた系だけ。Insert・Delete キーでも）
        if !c.combo {
            ui.separator();
            if ui
                .small_button(tr("＋ 面"))
                .on_hover_text(tr("選んだ行の前に面を挿入（Insert）"))
                .clicked()
            {
                ins_btn = true;
            }
            if ui
                .small_button(tr("− 面"))
                .on_hover_text(tr("選んだ面を消す（Delete）"))
                .clicked()
            {
                del_btn = true;
            }
        }
        if c.dirty {
            ui.separator();
            ui.label(RichText::new(tr("変更あり")).color(ui.visuals().warn_fg_color));
            if ui.button(tr("系ファイルに保存…")).clicked() {
                acts.push(Act::Save);
            }
        }
    });
    if let Some(n) = go {
        st.scope.truncate(n);
        st.cur = None;
        st.edit = None;
    }

    // 直下のブロック: クリックで選ぶ、ダブルクリックで入る
    let mut enter: Option<String> = None;
    if !children.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("ブロック")).small().color(weak));
            for &i in &children {
                let b = &c.blocks[i];
                let lk = block_locked(c.blocks, c.locked, i);
                let text = if lk {
                    format!("■ {}", b.name)
                } else {
                    b.name.clone()
                };
                let hint = if tree {
                    "面 {}〜{}（ダブルクリックで開く・たたむ）"
                } else {
                    "面 {}〜{}（ダブルクリックで中へ）"
                };
                let r = ui
                    .selectable_label(st.sel.as_deref() == Some(b.name.as_str()), text)
                    .on_hover_text(trf(hint, &[&b.surfs[0], &b.surfs[b.surfs.len() - 1]]));
                if r.double_clicked() {
                    if tree {
                        toggle = Some(b.name.clone());
                    } else {
                        enter = Some(b.name.clone());
                    }
                } else if r.clicked() {
                    st.sel = Some(b.name.clone());
                    acts.push(Act::Select(st.sel.clone()));
                }
            }
        });
    }
    // 選んだブロックのロック
    if let Some(i) = st
        .sel
        .as_ref()
        .and_then(|s| c.blocks.iter().position(|b| &b.name == s))
    {
        let name = c.blocks[i].name.clone();
        ui.horizontal(|ui| {
            let mut lk = c.locked.contains(&name);
            if ui
                .checkbox(&mut lk, trf("{} をロック", &[&name]))
                .on_hover_text(tr(
                    "ロックしたブロック（と中のブロック）の面は変えられない（L キーでも切り替え）",
                ))
                .changed()
            {
                acts.push(Act::Lock(name.clone(), lk));
            }
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
            }
        });
    }

    // 表
    let mut start: Option<(usize, Col)> = None;
    let mut commit: Option<(usize, Col, String)> = None;
    let mut clicked_any = false;
    let sel_block = st.sel.clone();
    let row_h = 20.0;
    // 幅が足りなければ横に流す
    // 表は上の半分ほど、下に選んだ面の設定
    let table_h = (ui.available_height() * 0.5).max(120.0);
    egui::ScrollArea::horizontal()
        .id_salt("lens_data_h")
        .show(ui, |ui| {
            // 文字を選べるラベルはクリックを取ってしまう（セルの空き・行のクリックが行に届かない）
            ui.style_mut().interaction.selectable_labels = false;
            let mut table = TableBuilder::new(ui)
                .id_salt("lens_data")
                .max_scroll_height(table_h)
                .auto_shrink([false, true])
                .striped(true)
                .resizable(true)
                .sense(Sense::click())
                .column(Column::initial(40.0).at_least(30.0))
                .column(Column::initial(70.0).at_least(30.0))
                .column(Column::initial(80.0).at_least(40.0));
            for col in Col::ALL {
                table = table.column(if col == Col::Conic {
                    Column::remainder().at_least(50.0)
                } else {
                    Column::initial(if col.numeric() { 80.0 } else { 90.0 }).at_least(40.0)
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
                    h.col(|ui| {
                        ui.strong(tr("種類"));
                    });
                    for col in Col::ALL {
                        h.col(|ui| {
                            ui.strong(col.header());
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_h, rows.len(), |mut row| {
                        let j = rows[row.index()];
                        let r = &c.rows[j];
                        let blk = child_of(j);
                        let fold = folded(j);
                        let lock = locked_at(c.blocks, c.locked, j);
                        let in_sel =
                            blk.is_some_and(|i| Some(&c.blocks[i].name) == sel_block.as_ref());
                        row.set_selected(in_sel || st.cur.is_some_and(|x| x.0 == j));
                        row.col(|ui| {
                            if let Some((_, a)) = fold {
                                ui.label(RichText::new(format!("{a}–{j}")).monospace().color(weak));
                                return;
                            }
                            let t = if j == 0 {
                                "OBJ".to_string()
                            } else if j + 1 == c.rows.len() {
                                "IMA".to_string()
                            } else if r.stop {
                                format!("STO {j}")
                            } else {
                                j.to_string()
                            };
                            ui.label(RichText::new(t).monospace());
                        });
                        row.col(|ui| {
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
                        row.col(|ui| {
                            let t = if fold.is_some() {
                                tr("ブロック")
                            } else {
                                r.typ.as_str()
                            };
                            ui.label(RichText::new(t).small().color(weak));
                        });
                        for col in Col::ALL {
                            row.col(|ui| {
                                // たたんだブロックの行: 厚さ（ブロックの後の間隔）だけ見せて変えられる。ほかはダブルクリックで中へ
                                if let (Some((i, a)), false) = (fold, col == Col::T) {
                                    let text = if col == Col::Comment {
                                        trf(
                                            "{} 面",
                                            &[&c.blocks[i]
                                                .surfs
                                                .iter()
                                                .filter(|&&x| a <= x && x <= j)
                                                .count()],
                                        )
                                    } else {
                                        String::new()
                                    };
                                    let resp = ui.add(
                                        egui::Label::new(RichText::new(text).color(weak))
                                            .sense(Sense::click())
                                            .truncate(),
                                    );
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
                                        st.cur = Some((j, col));
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
                                let editing = st.edit.as_mut().filter(|e| e.0 == j && e.1 == col);
                                if let Some(e) = editing {
                                    let te = egui::TextEdit::singleline(&mut e.2)
                                        .desired_width(f32::INFINITY);
                                    let te = if col.numeric() {
                                        te.horizontal_align(egui::Align::RIGHT)
                                    } else {
                                        te
                                    };
                                    let resp = ui.add(te);
                                    if !e.3 {
                                        resp.request_focus();
                                        if let Some(mut s) =
                                            egui::TextEdit::load_state(ui.ctx(), resp.id)
                                        {
                                            let all = egui::text::CCursorRange::two(
                                                egui::text::CCursor::new(0),
                                                egui::text::CCursor::new(e.2.chars().count()),
                                            );
                                            s.cursor.set_char_range(Some(all));
                                            s.store(ui.ctx(), resp.id);
                                        }
                                        e.3 = true;
                                    }
                                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                                        st.edit = None;
                                    } else if resp.lost_focus() {
                                        commit = Some((j, col, e.2.clone()));
                                    }
                                    return;
                                }
                                let text = col.text(r);
                                let is_cur = st.cur == Some((j, col));
                                let ro = lock
                                    .as_ref()
                                    .and(locked_cell(c.blocks, c.locked, j, col))
                                    .map(|n| trf("{} はロック中", &[&n]))
                                    .or_else(|| (c.read_only)(j, col));
                                let mut rt = RichText::new(&text);
                                if col.numeric() {
                                    rt = rt.monospace();
                                }
                                if ro.is_some() {
                                    rt = rt.color(weak);
                                }
                                let layout = if col.numeric() {
                                    egui::Layout::right_to_left(egui::Align::Center)
                                } else {
                                    egui::Layout::left_to_right(egui::Align::Center)
                                };
                                let resp = ui
                                    .with_layout(layout, |ui| {
                                        ui.add(
                                            egui::Label::new(rt).sense(Sense::click()).truncate(),
                                        )
                                    })
                                    .inner;
                                if is_cur {
                                    ui.painter().rect_stroke(
                                        ui.max_rect(),
                                        2.0,
                                        egui::Stroke::new(1.5_f32, accent),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                let resp = match &ro {
                                    Some(why) => resp.on_hover_text(why),
                                    None => resp,
                                };
                                if resp.clicked() || resp.double_clicked() {
                                    clicked_any = true;
                                    st.cur = Some((j, col));
                                    if let Some(i) = blk {
                                        if st.sel.as_deref() != Some(c.blocks[i].name.as_str()) {
                                            st.sel = Some(c.blocks[i].name.clone());
                                            acts.push(Act::Select(st.sel.clone()));
                                        }
                                    }
                                }
                                if resp.double_clicked() {
                                    match &ro {
                                        Some(why) => acts.push(Act::Refused(why.clone())),
                                        None => start = Some((j, col)),
                                    }
                                }
                            });
                        }
                        // セルの文字の外（空いたところ・面・種類の列）のクリックでも行を選ぶ
                        let rr = row.response();
                        if rr.clicked() || rr.double_clicked() {
                            clicked_any = true;
                            let col = st.cur.map_or(Col::R, |x| x.1);
                            st.cur = Some((j, col));
                            st.edit = None;
                            if let Some(i) = blk {
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
        });
    if clicked_any {
        st.active = true;
    } else if ui.input(|i| i.pointer.any_click()) && !ui.ui_contains_pointer() {
        st.active = false;
    }

    // キー（表をクリックした後。書き換え中は入力欄が使う）
    if st.active && st.edit.is_none() && ui.memory(|m| m.focused().is_none()) {
        let (ins_k, del_k) = ui.input(|i| (i.key_pressed(Key::Insert), i.key_pressed(Key::Delete)));
        ins_btn |= ins_k;
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
                i.key_pressed(Key::L),
            )
        });
        if !rows.is_empty() {
            let (j, col) = st.cur.unwrap_or((rows[0], Col::R));
            let pos = rows.iter().position(|&x| x == j).unwrap_or(0);
            let ci = Col::ALL.iter().position(|&x| x == col).unwrap_or(1);
            let mut moved = None;
            if up && pos > 0 {
                moved = Some((rows[pos - 1], col));
            } else if down && pos + 1 < rows.len() {
                moved = Some((rows[pos + 1], col));
            } else if left && ci > 0 {
                moved = Some((j, Col::ALL[ci - 1]));
            } else if right && ci + 1 < Col::ALL.len() {
                moved = Some((j, Col::ALL[ci + 1]));
            } else if (up || down || left || right) && st.cur.is_none() {
                moved = Some((j, col));
            }
            if let Some(m) = moved {
                st.cur = Some(m);
                if let Some(i) = child_of(m.0) {
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
                } else if enter_k || f2 {
                    let ro = locked_cell(c.blocks, c.locked, j, col)
                        .map(|n| trf("{} はロック中", &[&n]))
                        .or_else(|| (c.read_only)(j, col));
                    match ro {
                        Some(why) => acts.push(Act::Refused(why)),
                        None => start = Some((j, col)),
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
                    let at = first.max(1);
                    acts.push(Act::Ops(vec![
                        serde_json::json!({ "op": "insert_surface", "at": at, "surface": {} }),
                    ]));
                    st.cur = Some((at, st.cur.map_or(Col::R, |x| x.1)));
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
    if let Some((j, col)) = start {
        st.cur = Some((j, col));
        // 書き換えは丸めない値から
        let r = &c.rows[j];
        let text = match col {
            Col::R if r.r.is_infinite() => String::new(),
            Col::R => crate::surf_edit::full(r.r),
            Col::T => crate::surf_edit::full(r.t),
            Col::Semi if r.semi_auto => "auto".into(),
            Col::Semi => crate::surf_edit::full(r.semi),
            Col::Conic => crate::surf_edit::full(r.k),
            _ => col.text(r),
        };
        st.edit = Some((j, col, text, false));
    }
    if let Some((j, col, text)) = commit {
        st.edit = None;
        st.active = true;
        let same = match col.parse(&text) {
            Ok(v) if col == Col::Semi && v == json!("auto") => c.rows[j].semi_auto,
            Ok(v) if col.numeric() => {
                let r = &c.rows[j];
                let cur = match col {
                    Col::R => {
                        if r.r.is_infinite() {
                            0.0
                        } else {
                            r.r
                        }
                    }
                    Col::T => r.t,
                    // 自動の面に数を書けば固定にする（値が同じでも）
                    Col::Semi if r.semi_auto => f64::NAN,
                    Col::Semi => r.semi,
                    _ => r.k,
                };
                v.as_f64() == Some(cur)
            }
            _ => text.trim() == col.text(&c.rows[j]),
        };
        if !same {
            match col.parse(&text) {
                Ok(value) => acts.push(Act::Edit { j, col, value }),
                Err(e) => acts.push(Act::Refused(e)),
            }
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
    // 選んだ面の設定（たたんだブロックの行・ブロックだけを選んだときは、ブロックの設定をパネルの側で）
    ui.separator();
    st.block_view = None;
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
                .show(ui, |ui| {
                    acts.extend(crate::surf_edit::ui(ui, &mut st.insp, j, ro, c.combo));
                });
        }
        Some(j) => {
            st.block_view = folded(j).map(|(i, _)| c.blocks[i].name.clone());
        }
        None => match st.sel.clone() {
            Some(b) => st.block_view = Some(b),
            None => {
                ui.weak(tr(
                    "面かブロックを選ぶと、その設定をここで変えられます（ブロックはダブルクリック・Enter で中へ、マウスの戻るボタン・Backspace で外へ）。",
                ));
            }
        },
    }
    acts
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
}
