//! 多重構成のエディタ（OpticStudio の Multi-Configuration Editor に当たる表）。
//!
//! 行 = パラメータ（serve の `set` と同じ名前: `3.t`・`2.c`・`G1.shift` など）、列 = 構成。表は serve の
//! `config_table` から作り、変更は serve の `edit` の操作（`set_config_count`・`set_config_row`・`add_config_row`・
//! `remove_config_row`・`set_config`・`insert_config`・`remove_config`・`set_config_pickup`・`remove_config_pickup`）で送る。.zmx の多重構成は `config_diff` で表に取り込む。

use egui::{Key, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use serde_json::{json, Value};

use crate::i18n::{tr, trf};
use crate::theme;

/// serve の config_table の結果
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    /// 構成の数（表が無ければ 1）
    pub n: usize,
    /// 今の構成（1 始まり）
    pub active: usize,
    /// (パラメータの名前, 構成ごとの値)
    pub rows: Vec<(String, Vec<f64>)>,
    /// 行ごとの構成のピックアップ
    pub picks: Vec<Vec<Pick>>,
    /// .zmx の多重構成の数と、読んだ構成
    pub zmx_cfgs: usize,
    pub zmx_cfg: usize,
}

/// 構成のピックアップ: 構成 k（1 始まり）の値 = 構成 from の値 × scale + offset
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pick {
    pub k: usize,
    pub from: usize,
    pub scale: f64,
    pub offset: f64,
}

impl Table {
    /// 行 i の構成 k（0 始まり）のピックアップ
    fn pick(&self, i: usize, k: usize) -> Option<Pick> {
        self.picks.get(i)?.iter().find(|p| p.k == k + 1).copied()
    }
}

pub fn parse(v: &Value) -> Table {
    let us = |k: &str| v[k].as_u64().unwrap_or(1).max(1) as usize;
    Table {
        n: us("n"),
        active: us("active"),
        rows: v["rows"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|r| {
                        let vals = r["values"]
                            .as_array()
                            .map(|x| x.iter().map(|y| y.as_f64().unwrap_or(f64::NAN)).collect())
                            .unwrap_or_default();
                        (r["param"].as_str().unwrap_or_default().to_string(), vals)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        picks: v["rows"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|r| {
                        r["picks"]
                            .as_array()
                            .map(|ps| {
                                ps.iter()
                                    .map(|p| Pick {
                                        k: p["k"].as_u64().unwrap_or(0) as usize,
                                        from: p["from"].as_u64().unwrap_or(0) as usize,
                                        scale: p["scale"].as_f64().unwrap_or(1.0),
                                        offset: p["offset"].as_f64().unwrap_or(0.0),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default(),
        zmx_cfgs: us("zmx_cfgs"),
        zmx_cfg: us("zmx_cfg"),
    }
}

/// パネルの操作
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// serve の edit の操作
    Ops(Vec<Value>),
    /// .zmx の多重構成を表に取り込む（serve の config_diff）
    Import,
    /// 断った理由
    Refused(String),
}

#[derive(Debug, Clone, Default)]
pub struct State {
    pub table: Option<Table>,
    /// 今のセル（行、構成 0 始まり）
    cur: Option<(usize, usize)>,
    /// 書き換え中のセル（行、構成 0 始まり、文字、入力欄にフォーカスを渡したか、全部を選んで始めるか）
    edit: Option<(usize, usize, String, bool, bool)>,
    /// 表をクリックした後（キーを表が使う）
    active: bool,
    /// 足す行のパラメータの名前
    new_param: String,
    /// 選んだ列（構成 0 始まり。見出しのクリック）
    sel_col: Option<usize>,
    /// 名前の欄にフォーカスを渡す
    focus_name: bool,
    /// ピックアップの欄（セル、元の構成、倍率、ずれ）
    pick_form: Option<((usize, usize), String, String, String)>,
    /// Shift+F10 で開いたセルの欄（行、構成、出す位置）と、開いたフレーム（同じフレームのクリックで閉じない）
    kbd_menu: Option<(usize, usize, egui::Pos2)>,
    kbd_fresh: bool,
    /// 今のセルの左下（Shift+F10 の欄の位置）
    cur_pos: Option<egui::Pos2>,
    /// 名前の欄の補完の候補で選んでいる番号
    comp_pick: usize,
    /// 取り込みで表にできなかった違い
    pub notes: Vec<String>,
}

/// 丸めない数の文字
fn full(x: f64) -> String {
    if x.is_finite() {
        format!("{x}")
    } else {
        "inf".into()
    }
}

/// 表の数（見る用、有効 6 桁）
fn short(x: f64) -> String {
    if !x.is_finite() {
        return "inf".into();
    }
    let s = format!("{x:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.into()
    }
}

/// 値の文字を数に（inf も）
fn parse_num(t: &str) -> Result<f64, String> {
    let t = t.trim();
    match t.to_ascii_lowercase().as_str() {
        "inf" | "infinity" => Ok(f64::INFINITY),
        "-inf" | "-infinity" => Ok(f64::NEG_INFINITY),
        _ => t.parse().map_err(|_| trf("数でない: {}", &[&t])),
    }
}

/// 行 r の構成 k の値を v にした set_config_row
fn set_cell(row: &(String, Vec<f64>), k: usize, v: f64) -> Value {
    let mut vals = row.1.clone();
    if let Some(x) = vals.get_mut(k) {
        *x = v;
    }
    json!({ "op": "set_config_row", "param": row.0, "values": vals })
}

/// 書き換えの確定: (行, 構成, 文字, 確定後に移る向き)
type Commit = (usize, usize, String, Option<(i32, i32)>);

/// パラメータの名前の説明（3.t → 面 3 の厚さ、G1.shift → G1 の移動）
pub fn param_desc(name: &str) -> String {
    let Some((who, k)) = name.trim().rsplit_once('.') else {
        return match name.trim() {
            "objx" | "objy" => tr("物体の位置").into(),
            n if n.starts_with("wave") => tr("波長").into(),
            _ => String::new(),
        };
    };
    let what = match k.to_ascii_lowercase().as_str() {
        "c" => tr("曲率").to_string(),
        "r" => tr("曲率半径").into(),
        "k" => tr("コーニック").into(),
        "t" => tr("厚さ").into(),
        "dx" => tr("偏心 X").into(),
        "dy" => tr("偏心 Y").into(),
        "tx" => tr("傾き X").into(),
        "ty" => tr("傾き Y").into(),
        "tz" => tr("傾き Z").into(),
        "dn" => tr("屈折率のずれ").into(),
        "disp" => tr("分散の変化").into(),
        "shift" => tr("移動").into(),
        a if a.starts_with('a') && a[1..].parse::<u8>().is_ok() => trf("r^{} の係数", &[&&a[1..]]),
        z if z.starts_with('z') && z[1..].parse::<u16>().is_ok() => trf("Zernike {}", &[&&z[1..]]),
        _ => return String::new(),
    };
    if who.trim().parse::<usize>().is_ok() {
        trf("面 {} の{}", &[&who.trim(), &what])
    } else {
        trf("{} の{}", &[&who.trim(), &what])
    }
}

/// 名前の候補を作るための系の面・玉・群
#[derive(Debug, Clone, Default)]
pub struct Cands {
    /// 面ごとの (面タイプ, コメント)
    pub surfs: Vec<(String, String)>,
    /// 玉・群の名前
    pub units: Vec<String>,
}

/// 面タイプで使えるパラメータの種類
fn surf_kinds(typ: &str) -> Vec<String> {
    let mut v: Vec<String> = ["r", "t", "k", "dn", "disp"]
        .iter()
        .map(|x| x.to_string())
        .collect();
    match typ {
        "EVENASPH" | "ODDASPHE" => v.extend((1..=8).map(|i| format!("a{}", 2 * i))),
        "COORDBRK" => {
            v = ["t", "dx", "dy", "tx", "ty", "tz"]
                .iter()
                .map(|x| x.to_string())
                .collect();
        }
        "SZERNSAG" | "FZERNSAG" => v.extend((1..=37).map(|i| format!("z{i}"))),
        _ => {}
    }
    v
}

/// 玉・群のパラメータの種類
const UNIT_KINDS: [&str; 8] = ["shift", "dx", "dy", "tx", "ty", "tz", "dn", "disp"];

/// 表の行にできるパラメータの名前（全部）
fn all_params(c: &Cands) -> Vec<String> {
    let mut v = Vec::new();
    for (j, (typ, _)) in c.surfs.iter().enumerate() {
        for k in surf_kinds(typ) {
            v.push(format!("{j}.{k}"));
        }
    }
    for u in &c.units {
        for k in UNIT_KINDS {
            v.push(format!("{u}.{k}"));
        }
    }
    v.push("objx".into());
    v.push("objy".into());
    v
}

/// 打った文字に合う名前（前方一致を先に、次に名前・説明の部分一致）。多すぎるときは先頭だけ
fn complete(c: &Cands, text: &str) -> Vec<String> {
    let q = text.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let (mut head, mut rest) = (Vec::new(), Vec::new());
    for n in all_params(c) {
        let l = n.to_lowercase();
        if l.starts_with(&q) {
            head.push(n);
        } else if l.contains(&q) || param_desc(&n).to_lowercase().contains(&q) {
            rest.push(n);
        }
    }
    head.extend(rest);
    head.truncate(200);
    head
}

/// 名前を階層で選ぶ欄（面 → 種類、玉・群 → 種類、物体の位置）。選んだ名前を返す
fn param_menu(ui: &mut Ui, c: &Cands) -> Option<String> {
    let mut out = None;
    ui.menu_button(tr("面"), |ui| {
        egui::ScrollArea::vertical()
            .max_height(400.0)
            .show(ui, |ui| {
                for (j, (typ, comment)) in c.surfs.iter().enumerate() {
                    let label = if comment.is_empty() {
                        format!("{j}  {typ}")
                    } else {
                        format!("{j}  {typ}  {comment}")
                    };
                    ui.menu_button(label, |ui| {
                        for k in surf_kinds(typ) {
                            let n = format!("{j}.{k}");
                            if ui.button(format!("{k}  {}", param_desc(&n))).clicked() {
                                out = Some(n);
                                ui.close_menu();
                            }
                        }
                    });
                }
            });
    });
    ui.add_enabled_ui(!c.units.is_empty(), |ui| {
        ui.menu_button(tr("玉・群"), |ui| {
            for u in &c.units {
                ui.menu_button(u, |ui| {
                    for k in UNIT_KINDS {
                        let n = format!("{u}.{k}");
                        if ui.button(format!("{k}  {}", param_desc(&n))).clicked() {
                            out = Some(n);
                            ui.close_menu();
                        }
                    }
                });
            }
        });
    });
    ui.menu_button(tr("物体の位置"), |ui| {
        for n in ["objx", "objy"] {
            if ui.button(n).clicked() {
                out = Some(n.to_string());
                ui.close_menu();
            }
        }
    });
    out
}

/// 構成を位置 at（1 始まり、新しい構成の番号）に挿入する操作。値は構成 from（1 始まり）、pickup なら from のピックアップ
fn insert_op(at: usize, from: usize, pickup: bool) -> Value {
    json!({ "op": "insert_config", "at": at, "from": from, "pickup": pickup })
}

/// 構成 k（1 始まり）を消す操作
fn remove_op(k: usize) -> Value {
    json!({ "op": "remove_config", "k": k })
}

/// 構成の挿入の選択肢（構成 k の前後、ピックアップ付き）。選んだ操作を返す
fn insert_menu(ui: &mut Ui, k: usize) -> Option<Value> {
    let mut op = None;
    if ui.button(trf("構成 {} の前に構成を挿入", &[&k])).clicked() {
        op = Some(insert_op(k, k, false));
    }
    if ui.button(trf("構成 {} の後に構成を挿入", &[&k])).clicked() {
        op = Some(insert_op(k + 1, k, false));
    }
    if ui
        .button(trf("構成 {} の前にピックアップ付きで挿入", &[&k]))
        .on_hover_text(trf(
            "新しい構成の値を、どの行も構成 {} のピックアップにする（元を変えると追従）",
            &[&k],
        ))
        .clicked()
    {
        op = Some(insert_op(k, k, true));
    }
    if ui
        .button(trf("構成 {} の後にピックアップ付きで挿入", &[&k]))
        .on_hover_text(trf(
            "新しい構成の値を、どの行も構成 {} のピックアップにする（元を変えると追従）",
            &[&k],
        ))
        .clicked()
    {
        op = Some(insert_op(k + 1, k, true));
    }
    if op.is_some() {
        ui.close_menu();
    }
    op
}

/// 表を文字に（タブ区切り、1 行目は見出し。Excel に貼れる）
fn copy_text(t: &Table, rows: &[usize]) -> String {
    rows.iter()
        .filter_map(|&i| t.rows.get(i))
        .map(|(n, v)| {
            std::iter::once(n.clone())
                .chain(v.iter().map(|x| full(*x)))
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 貼り付け: タブ区切りの文字を今のセル (i, k) から右下へ。行の最初がパラメータの名前ならその行に（無ければ足す）
fn paste_ops(t: &Table, i: usize, k: usize, text: &str) -> Result<Vec<Value>, String> {
    let mut ops = Vec::new();
    let mut rows: Vec<(String, Vec<f64>)> = t.rows.clone();
    for (li, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let cells: Vec<&str> = line.split('\t').collect();
        // 最初の欄が数でなければパラメータの名前（Ctrl+C で写した行）
        let (row, start, vals) = match parse_num(cells[0]) {
            Ok(_) => (i + li, k, &cells[..]),
            Err(_) => {
                let name = cells[0].trim();
                let r = match rows.iter().position(|x| x.0 == name) {
                    Some(r) => r,
                    None => {
                        ops.push(json!({ "op": "add_config_row", "param": name }));
                        let n = t.n;
                        rows.push((name.to_string(), vec![f64::NAN; n]));
                        rows.len() - 1
                    }
                };
                (r, 0, &cells[1..])
            }
        };
        let Some(r) = rows.get_mut(row) else { break };
        for (c, x) in vals.iter().enumerate() {
            if let Some(slot) = r.1.get_mut(start + c) {
                *slot = parse_num(x)?;
            }
        }
        if r.1.iter().all(|x| !x.is_nan()) {
            ops.push(json!({ "op": "set_config_row", "param": r.0, "values": r.1 }));
        }
    }
    Ok(ops)
}

/// セルの右クリック（Shift+F10）の欄
#[allow(clippy::too_many_arguments)]
fn cell_menu(
    ui: &mut Ui,
    st: &mut State,
    t: &Table,
    i: usize,
    k: usize,
    start: &mut Option<(usize, usize, Option<String>)>,
    menu_ops: &mut Vec<Value>,
    acts: &mut Vec<Act>,
    remove: &mut Option<usize>,
) {
    let weak = theme::weak_text(ui.visuals());
    let name = &t.rows[i].0;
    let v = t.rows[i].1.get(k).copied().unwrap_or(f64::NAN);
    let pick = t.pick(i, k);
    if ui.button(tr("書き換える（F2）")).clicked() {
        *start = Some((i, k, None));
        ui.close_menu();
    }
    if ui
        .add_enabled(t.n > 1, egui::Button::new(tr("この値を全部の構成に")))
        .clicked()
    {
        menu_ops.push(json!({
            "op": "set_config_row", "param": name, "values": vec![v; t.n],
        }));
        ui.close_menu();
    }
    if k + 1 != t.active
        && ui
            .button(trf("構成 {} を系に入れる", &[&(k + 1)]))
            .clicked()
    {
        menu_ops.push(json!({ "op": "set_config", "k": k + 1 }));
        ui.close_menu();
    }
    // 構成のピックアップ
    if t.n > 1 {
        ui.separator();
        ui.label(RichText::new(tr("ピックアップ")).strong());
        if let Some(p) = pick {
            ui.horizontal(|ui| {
                ui.label(trf(
                    "構成 {} × {} + {}",
                    &[&p.from, &full(p.scale), &full(p.offset)],
                ));
                if ui.small_button(tr("外す")).clicked() {
                    menu_ops.push(json!({
                        "op": "remove_config_pickup", "param": name, "k": k + 1,
                    }));
                    ui.close_menu();
                }
            });
        }
        if st.pick_form.as_ref().map_or(true, |f| f.0 != (i, k)) {
            let from = pick.map_or(if k == 0 { 2 } else { 1 }, |p| p.from);
            let (sc, of) = pick.map_or((1.0, 0.0), |p| (p.scale, p.offset));
            st.pick_form = Some(((i, k), from.to_string(), full(sc), full(of)));
        }
        if let Some(f) = st.pick_form.as_mut() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("元の構成")).color(weak));
                ui.add(egui::TextEdit::singleline(&mut f.1).desired_width(30.0));
                ui.label(RichText::new(tr("倍率")).color(weak));
                ui.add(egui::TextEdit::singleline(&mut f.2).desired_width(50.0));
                ui.label(RichText::new(tr("ずれ")).color(weak));
                ui.add(egui::TextEdit::singleline(&mut f.3).desired_width(50.0));
            });
            if ui.button(tr("+ ピックアップにする")).clicked() {
                match (
                    f.1.trim().parse::<usize>(),
                    parse_num(&f.2),
                    parse_num(&f.3),
                ) {
                    (Ok(from), Ok(sc), Ok(of)) => menu_ops.push(json!({
                        "op": "set_config_pickup", "param": name, "k": k + 1,
                        "from": from, "scale": sc, "offset": of,
                    })),
                    _ => acts.push(Act::Refused(
                        tr("ピックアップの元の構成・倍率・ずれは数で").into(),
                    )),
                }
                ui.close_menu();
            }
        }
    }
    ui.separator();
    if ui.button(tr("行をコピー（Ctrl+C）")).clicked() {
        ui.ctx().copy_text(copy_text(t, &[i]));
        st.kbd_menu = None;
        ui.close_menu();
    }
    if ui.button(tr("行を消す（Delete）")).clicked() {
        *remove = Some(i);
        ui.close_menu();
    }
}

pub fn ui(ui: &mut Ui, st: &mut State, combo: bool, cands: &Cands) -> Vec<Act> {
    let mut acts = Vec::new();
    let weak = theme::weak_text(ui.visuals());
    let accent = ui.visuals().selection.stroke.color;
    let Some(t) = st.table.clone() else {
        ui.add_space(8.0);
        ui.label(tr("系を開くと、多重構成の表が出ます。"));
        return acts;
    };
    if st.cur.is_some_and(|(i, k)| i >= t.rows.len() || k >= t.n) {
        st.cur = None;
        st.edit = None;
    }
    if combo {
        ui.label(
            RichText::new(tr(
                "組合せの系は作り直すと表が消えます。組合せの構成はプロジェクトの状態（states）で扱います。表を使うには系ファイルに保存して開いてください。",
            ))
            .small()
            .color(weak),
        );
    }
    // 構成の数・今の構成
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("構成")).color(weak));
        for k in 1..=t.n {
            if ui
                .selectable_label(k == t.active, k.to_string())
                .on_hover_text(tr(
                    "この構成の値を系に入れる（解析・図・面の表が追従）。Ctrl+A で次、Ctrl+Shift+A で前の構成",
                ))
                .clicked()
                && k != t.active
            {
                acts.push(Act::Ops(vec![json!({ "op": "set_config", "k": k })]));
            }
        }
    });
    // 挿入・削除（標準は行、▼で構成）。挿入・削除の元の構成は選んだ列、無ければ今のセルの列、無ければ今の構成
    let refk = st
        .sel_col
        .or(st.cur.map(|c| c.1))
        .unwrap_or(t.active - 1)
        .min(t.n - 1)
        + 1;
    let mut bar_ops: Vec<Value> = Vec::new();
    let mut bar_remove: Option<usize> = None;
    ui.horizontal_wrapped(|ui| {
        let resp = ui.add(
            egui::TextEdit::singleline(&mut st.new_param)
                .hint_text("3.t・2.c・STOP.k・G1.shift")
                .desired_width(150.0),
        );
        if std::mem::take(&mut st.focus_name) {
            resp.request_focus();
        }
        // 補完: 打った文字に合う名前の一覧（↑ ↓ で選び Enter で足す、Tab で欄に入れる、クリックでも）
        let mut picked: Option<String> = None;
        let mut enter = resp.lost_focus() && ui.input(|x| x.key_pressed(Key::Enter));
        if resp.has_focus() || enter {
            let m = complete(cands, &st.new_param);
            let exact = m.iter().any(|x| x.eq_ignore_ascii_case(st.new_param.trim()));
            if resp.changed() {
                st.comp_pick = 0;
            }
            if st.comp_pick >= m.len() {
                st.comp_pick = 0;
            }
            let (up, down, tab) = ui.input(|x| {
                (
                    x.key_pressed(Key::ArrowUp),
                    x.key_pressed(Key::ArrowDown),
                    x.key_pressed(Key::Tab),
                )
            });
            if up && st.comp_pick > 0 {
                st.comp_pick -= 1;
            }
            if down && st.comp_pick + 1 < m.len() {
                st.comp_pick += 1;
            }
            if !m.is_empty() && !(exact && m.len() == 1) {
                let area = egui::Area::new(resp.id.with("comp"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(resp.rect.left_bottom())
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_min_width(240.0);
                            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                                for (k, n) in m.iter().enumerate() {
                                    let r = ui.selectable_label(
                                        k == st.comp_pick,
                                        format!("{n}  {}", param_desc(n)),
                                    );
                                    if k == st.comp_pick && (up || down) {
                                        r.scroll_to_me(None);
                                    }
                                    if r.clicked() {
                                        picked = Some(n.clone());
                                    }
                                }
                            });
                        });
                    });
                let _ = area;
                if tab {
                    if let Some(n) = m.get(st.comp_pick) {
                        st.new_param = n.clone();
                        st.focus_name = true;
                    }
                }
                if enter && !exact {
                    if let Some(n) = m.get(st.comp_pick) {
                        st.new_param = n.clone();
                    }
                }
            }
        }
        let pm = ui.menu_button("…", |ui| param_menu(ui, cands));
        pm.response
            .on_hover_text(tr("パラメータを面・玉・群から選んで行を挿入"));
        if let Some(Some(n)) = pm.inner {
            picked = Some(n);
        }
        if let Some(n) = picked {
            st.new_param = n;
            enter = true;
        }
        let add = ui
            .button(tr("行を挿入"))
            .on_hover_text(tr(
                "左の名前のパラメータを表に足す（名前は Python・serve の set と同じ。どの構成も今の値から）。レンズデータのセルの右クリックの「多重構成の行にする」でも足せる",
            ))
            .clicked();
        if add && st.new_param.trim().is_empty() {
            st.focus_name = true;
        }
        if (add || enter) && !st.new_param.trim().is_empty() {
            if t.n < 2 {
                bar_ops.push(json!({ "op": "set_config_count", "n": 2 }));
            }
            bar_ops.push(json!({ "op": "add_config_row", "param": st.new_param.trim() }));
            st.new_param.clear();
        }
        ui.menu_button("▼", |ui| {
            if let Some(op) = insert_menu(ui, refk) {
                bar_ops.push(op);
            }
        })
        .response
        .on_hover_text(tr("構成の挿入（前・後、ピックアップ付き）"));
        ui.separator();
        let cur_row = st.cur.map(|c| c.0);
        if ui
            .add_enabled(cur_row.is_some(), egui::Button::new(tr("行を削除")))
            .on_hover_text(tr("選んだ行を表から消す（系の値は今のまま）。Delete キーでも"))
            .clicked()
        {
            bar_remove = cur_row;
        }
        ui.menu_button("▼", |ui| {
            if ui
                .add_enabled(t.n > 1, egui::Button::new(trf("構成 {} を削除", &[&refk])))
                .clicked()
            {
                bar_ops.push(remove_op(refk));
                ui.close_menu();
            }
            if ui
                .add_enabled(t.n > 1, egui::Button::new(tr("最後の構成を削除")))
                .clicked()
            {
                bar_ops.push(remove_op(t.n));
                ui.close_menu();
            }
        })
        .response
        .on_hover_text(tr("構成の削除（列の見出しをクリックして選び Delete でも）"));
        let d = param_desc(&st.new_param);
        if !d.is_empty() {
            ui.label(RichText::new(d).small().color(weak));
        }
    });
    if t.zmx_cfgs > 1 && t.rows.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(trf(
                "この .zmx は {} 個の構成を持ち、構成 {} を読んでいます。",
                &[&t.zmx_cfgs, &t.zmx_cfg],
            ));
            if ui
                .button(tr("表に取り込む"))
                .on_hover_text(tr(
                    "各構成を読み、構成で違う値（曲率・コーニック・厚さ・非球面・傾き偏心・dn・disp）を表の行にする",
                ))
                .clicked()
            {
                acts.push(Act::Import);
            }
        });
    }
    for n in &st.notes {
        ui.label(RichText::new(n).small().color(ui.visuals().warn_fg_color));
    }
    ui.add_space(4.0);

    // 表（セルのクリックで選び、ダブルクリック・Enter・F2・打鍵で書き換え。レンズデータの表と同じ操作）
    // (行, 構成, 文字, 確定後に移る向き)
    let mut commit: Option<Commit> = None;
    let mut cancel = false;
    let mut start: Option<(usize, usize, Option<String>)> = None;
    let mut clicked_any = false;
    let mut menu_ops: Vec<Value> = Vec::new();
    let mut remove: Option<usize> = None;
    let table_resp = egui::ScrollArea::horizontal()
        .id_salt("mce_h")
        .show(ui, |ui| {
            // 文字を選べるラベルはクリックを取ってしまう（セルの空き・行のクリックが行に届かない）
            ui.style_mut().interaction.selectable_labels = false;
            let mut table = TableBuilder::new(ui)
                .id_salt("mce")
                .auto_shrink([false, true])
                .striped(true)
                .resizable(true)
                .column(Column::initial(90.0).at_least(60.0));
            for _ in 0..t.n {
                table = table.column(Column::initial(90.0).at_least(50.0));
            }
            table
                .column(Column::exact(24.0))
                .header(22.0, |mut h| {
                    h.col(|ui| {
                        ui.strong(tr("パラメータ"));
                    });
                    for k in 1..=t.n {
                        h.col(|ui| {
                            let rt = RichText::new(trf("構成 {}", &[&k])).strong();
                            let r = ui.interact(ui.max_rect(), ui.id().with("mce_head"), Sense::click());
                            if st.sel_col == Some(k - 1) {
                                ui.painter().rect_filled(ui.max_rect(), 2.0, ui.visuals().selection.bg_fill);
                            }
                            ui.label(if k == t.active { rt.color(accent) } else { rt });
                            let r = r.on_hover_text(tr(
                                "クリックで列を選ぶ（Delete で構成を削除）。ダブルクリックでこの構成を系に入れる。右クリックで挿入・削除",
                            ));
                            if r.clicked() || r.secondary_clicked() {
                                clicked_any = true;
                                st.sel_col = Some(k - 1);
                                st.cur = None;
                            }
                            if r.double_clicked() && k != t.active {
                                acts.push(Act::Ops(vec![json!({ "op": "set_config", "k": k })]));
                            }
                            r.context_menu(|ui| {
                                if ui.button(tr("この構成を系に入れる")).clicked() {
                                    menu_ops.push(json!({ "op": "set_config", "k": k }));
                                    ui.close_menu();
                                }
                                ui.separator();
                                if let Some(op) = insert_menu(ui, k) {
                                    menu_ops.push(op);
                                }
                                ui.separator();
                                if ui
                                    .add_enabled(t.n > 1, egui::Button::new(tr("この構成を消す（Delete）")))
                                    .clicked()
                                {
                                    menu_ops.push(remove_op(k));
                                    ui.close_menu();
                                }
                            });
                        });
                    }
                    h.col(|_| {});
                })
                .body(|body| {
                    body.rows(20.0, t.rows.len(), |mut row| {
                        let i = row.index();
                        let (name, vals) = &t.rows[i];
                        row.set_selected(st.cur.is_some_and(|c| c.0 == i));
                        row.col(|ui| {
                            let r = ui.interact(ui.max_rect(), ui.id().with("mce_name"), Sense::click());
                            ui.label(RichText::new(name).monospace());
                            let d = param_desc(name);
                            let r = if d.is_empty() { r } else { r.on_hover_text(d) };
                            if r.clicked() {
                                clicked_any = true;
                                st.cur = Some((i, st.cur.map_or(0, |c| c.1)));
                                st.sel_col = None;
                            }
                            r.context_menu(|ui| {
                                if ui.button(tr("行を消す（系の値は今のまま）")).clicked() {
                                    remove = Some(i);
                                    ui.close_menu();
                                }
                            });
                        });
                        for k in 0..t.n {
                            row.col(|ui| {
                                if let Some(e) = st.edit.as_mut().filter(|e| e.0 == i && e.1 == k) {
                                    let (tab, shift, up, down, enter, esc) = ui.input(|x| {
                                        (
                                            x.key_pressed(Key::Tab),
                                            x.modifiers.shift,
                                            x.key_pressed(Key::ArrowUp),
                                            x.key_pressed(Key::ArrowDown),
                                            x.key_pressed(Key::Enter),
                                            x.key_pressed(Key::Escape),
                                        )
                                    });
                                    let resp = ui.add(
                                        egui::TextEdit::singleline(&mut e.2)
                                            .desired_width(f32::INFINITY)
                                            .horizontal_align(egui::Align::RIGHT)
                                            .lock_focus(true),
                                    );
                                    if !e.3 {
                                        resp.request_focus();
                                        if let Some(mut s) =
                                            egui::TextEdit::load_state(ui.ctx(), resp.id)
                                        {
                                            let n = e.2.chars().count();
                                            // Enter・F2・ダブルクリックは全部を選んで、打った文字から始めたときは後ろにカーソル
                                            let r = if e.4 {
                                                egui::text::CCursorRange::two(
                                                    egui::text::CCursor::new(0),
                                                    egui::text::CCursor::new(n),
                                                )
                                            } else {
                                                egui::text::CCursorRange::one(egui::text::CCursor::new(n))
                                            };
                                            s.cursor.set_char_range(Some(r));
                                            s.store(ui.ctx(), resp.id);
                                        }
                                        e.3 = true;
                                    }
                                    let mv = if tab {
                                        Some((0, if shift { -1 } else { 1 }))
                                    } else if up {
                                        Some((-1, 0))
                                    } else if down || enter {
                                        Some((if enter { 0 } else { 1 }, 0))
                                    } else {
                                        None
                                    };
                                    if esc {
                                        cancel = true;
                                    } else if mv.is_some() || resp.lost_focus() {
                                        commit = Some((i, k, e.2.clone(), mv));
                                        resp.surrender_focus();
                                    }
                                    return;
                                }
                                let v = vals.get(k).copied().unwrap_or(f64::NAN);
                                let pick = t.pick(i, k);
                                if st.sel_col == Some(k) {
                                    ui.painter().rect_filled(ui.max_rect(), 0.0, ui.visuals().selection.bg_fill);
                                }
                                // セル全体でクリックを受ける（文字の外でも）
                                let hover = match pick {
                                    Some(p) => trf(
                                        "{}（構成 {} × {} + {} のピックアップ）",
                                        &[&full(v), &p.from, &full(p.scale), &full(p.offset)],
                                    ),
                                    None => full(v),
                                };
                                let resp = ui
                                    .interact(ui.max_rect(), ui.id().with(("mce", i, k)), Sense::click())
                                    .on_hover_text(hover);
                                let mut rt = RichText::new(short(v)).monospace();
                                if k + 1 == t.active {
                                    rt = rt.color(accent);
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(rt);
                                        if pick.is_some() {
                                            ui.label(RichText::new("P").small().strong().color(weak));
                                        }
                                    },
                                );
                                if st.cur == Some((i, k)) {
                                    st.cur_pos = Some(ui.max_rect().left_bottom());
                                    ui.painter().rect_stroke(
                                        ui.max_rect(),
                                        2.0,
                                        egui::Stroke::new(1.5_f32, accent),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                if resp.clicked() || resp.secondary_clicked() || resp.double_clicked() {
                                    clicked_any = true;
                                    st.cur = Some((i, k));
                                    st.sel_col = None;
                                }
                                if resp.double_clicked() {
                                    start = Some((i, k, None));
                                }
                                resp.context_menu(|ui| {
                                    cell_menu(
                                        ui,
                                        st,
                                        &t,
                                        i,
                                        k,
                                        &mut start,
                                        &mut menu_ops,
                                        &mut acts,
                                        &mut remove,
                                    );
                                });
                            });
                        }
                        row.col(|ui| {
                            if ui
                                .small_button("✖")
                                .on_hover_text(tr("行を消す（系の値は今のまま）"))
                                .clicked()
                            {
                                remove = Some(i);
                            }
                        });
                    });
                });
        });
    let _ = table_resp;
    // Shift+F10: 今のセルの右クリックの欄
    if st.active
        && st.edit.is_none()
        && ui.memory(|m| m.focused().is_none())
        && ui.input(|x| x.modifiers.shift && x.key_pressed(Key::F10))
    {
        if let Some((i, k)) = st.cur {
            let pos = st.cur_pos.unwrap_or_else(|| ui.min_rect().left_top());
            st.kbd_menu = Some((i, k, pos));
            st.kbd_fresh = true;
        }
    }
    if let Some((i, k, pos)) = st.kbd_menu.filter(|m| m.0 < t.rows.len() && m.1 < t.n) {
        let n0 = (menu_ops.len(), acts.len());
        let area = egui::Area::new(ui.id().with("mce_kbd_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_max_width(320.0);
                    cell_menu(
                        ui,
                        st,
                        &t,
                        i,
                        k,
                        &mut start,
                        &mut menu_ops,
                        &mut acts,
                        &mut remove,
                    );
                });
            });
        let fresh = std::mem::take(&mut st.kbd_fresh);
        let done = (menu_ops.len(), acts.len()) != n0 || start.is_some() || remove.is_some();
        if done
            || ui.input(|x| x.key_pressed(Key::Escape))
            || (!fresh && area.response.clicked_elsewhere())
        {
            st.kbd_menu = None;
        }
    } else {
        st.kbd_menu = None;
    }
    if clicked_any || !menu_ops.is_empty() {
        st.active = true;
    } else if ui.input(|x| x.pointer.any_click()) && !ui.ui_contains_pointer() {
        st.active = false;
    }
    if cancel {
        st.edit = None;
        st.active = true;
    }
    // 列を選んで Delete: 構成を削除
    if st.active
        && st.edit.is_none()
        && ui.memory(|m| m.focused().is_none())
        && t.n > 1
        && ui.input(|x| x.key_pressed(Key::Delete))
    {
        if let Some(k) = st.sel_col.take() {
            acts.push(Act::Ops(vec![remove_op(k + 1)]));
        }
    }
    if st.sel_col.is_some_and(|k| k >= t.n) {
        st.sel_col = None;
    }
    // キー（表をクリックした後。書き換え中・ほかの入力欄にフォーカスがあるときは入力欄が使う）
    if st.active && st.edit.is_none() && ui.memory(|m| m.focused().is_none()) && !t.rows.is_empty()
    {
        let (up, down, left, right, enter, f2, del, esc) = ui.input(|x| {
            (
                x.key_pressed(Key::ArrowUp),
                x.key_pressed(Key::ArrowDown),
                x.key_pressed(Key::ArrowLeft) || (x.key_pressed(Key::Tab) && x.modifiers.shift),
                x.key_pressed(Key::ArrowRight) || (x.key_pressed(Key::Tab) && !x.modifiers.shift),
                x.key_pressed(Key::Enter),
                x.key_pressed(Key::F2),
                x.key_pressed(Key::Delete),
                x.key_pressed(Key::Escape),
            )
        });
        let (copy, paste, typed) = ui.input(|x| {
            let copy = x
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut));
            let paste = x.events.iter().find_map(|e| match e {
                egui::Event::Paste(s) => Some(s.clone()),
                _ => None,
            });
            let typed = if x.modifiers.command || x.modifiers.ctrl || x.modifiers.alt {
                None
            } else {
                x.events.iter().find_map(|e| match e {
                    egui::Event::Text(s) if !s.is_empty() => Some(s.clone()),
                    _ => None,
                })
            };
            (copy, paste, typed)
        });
        let (i, k) = st.cur.unwrap_or((0, 0));
        let n_rows = t.rows.len();
        if up || down || left || right {
            st.cur = Some(if st.cur.is_none() {
                (0, 0)
            } else if up {
                (i.saturating_sub(1), k)
            } else if down {
                ((i + 1).min(n_rows - 1), k)
            } else if left {
                (i, k.saturating_sub(1))
            } else {
                (i, (k + 1).min(t.n - 1))
            });
        } else if let Some(c) = st.cur {
            if enter || f2 || typed.is_some() {
                start = Some((c.0, c.1, typed));
            } else if del && st.sel_col.is_none() {
                remove = Some(c.0);
            } else if esc {
                st.cur = None;
            }
            if copy {
                ui.ctx().copy_text(copy_text(&t, &[c.0]));
            }
            if let Some(text) = paste {
                match paste_ops(&t, c.0, c.1, &text) {
                    Ok(ops) if !ops.is_empty() => acts.push(Act::Ops(ops)),
                    Ok(_) => {}
                    Err(e) => acts.push(Act::Refused(e)),
                }
            }
        }
    }
    if let Some((i, k, typed)) = start {
        let v = t.rows[i].1.get(k).copied().unwrap_or(f64::NAN);
        st.cur = Some((i, k));
        st.edit = Some(match typed {
            Some(s) => (i, k, s, false, false),
            None => (i, k, full(v), false, true),
        });
    }
    if let Some((i, k, text, mv)) = commit {
        st.edit = None;
        st.active = true;
        if let Some(r) = t.rows.get(i) {
            match parse_num(&text) {
                // set_config_row は今の構成の値を系に入れ直す（ほかの構成の値を変えても系はそのまま）
                Ok(v) if r.1.get(k) != Some(&v) => acts.push(Act::Ops(vec![set_cell(r, k, v)])),
                Ok(_) => {}
                Err(e) => acts.push(Act::Refused(e)),
            }
        }
        if let Some((dr, dc)) = mv {
            let ni = (i as i32 + dr).clamp(0, t.rows.len() as i32 - 1) as usize;
            let nk = (k as i32 + dc).clamp(0, t.n as i32 - 1) as usize;
            st.cur = Some((ni, nk));
        }
    }
    if !menu_ops.is_empty() {
        acts.push(Act::Ops(menu_ops));
    }
    if !bar_ops.is_empty() {
        acts.push(Act::Ops(bar_ops));
    }
    let remove = remove.or(bar_remove);
    if let Some(i) = remove {
        acts.push(Act::Ops(vec![
            json!({ "op": "remove_config_row", "param": t.rows[i].0 }),
        ]));
    }
    if t.n < 2 && t.zmx_cfgs < 2 {
        ui.label(
            RichText::new(tr(
                "構成は 1 つです。挿入の ▼ で構成を足すか、行を挿入すると構成 2 つの表になります。",
            ))
            .small()
            .color(weak),
        );
    }
    acts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_cell() {
        let t = parse(&json!({
            "n": 2, "active": 2, "zmx_cfgs": 2, "zmx_cfg": 2,
            "rows": [{ "param": "2.t", "values": [90.0, 80.0] }],
        }));
        assert_eq!((t.n, t.active, t.zmx_cfgs), (2, 2, 2));
        assert_eq!(t.rows[0], ("2.t".to_string(), vec![90.0, 80.0]));
        assert_eq!(
            set_cell(&t.rows[0], 0, 85.0),
            json!({ "op": "set_config_row", "param": "2.t", "values": [85.0, 80.0] })
        );
        assert_eq!(parse_num(" inf ").unwrap(), f64::INFINITY);
        assert!(parse_num("x").is_err());
        assert_eq!(short(0.1 + 0.2), "0.3");
        assert_eq!(short(-0.0), "0");
    }

    #[test]
    fn config_ops_and_text() {
        let t = parse(&json!({
            "n": 3, "active": 3,
            "rows": [{ "param": "2.t", "values": [1.0, 2.0, 3.0] }, { "param": "G1.shift", "values": [0.0, 0.5, 1.0] }],
        }));
        assert_eq!(t.pick(0, 1), None);
        assert_eq!(
            insert_op(2, 1, true),
            json!({ "op": "insert_config", "at": 2, "from": 1, "pickup": true })
        );
        assert_eq!(remove_op(3), json!({ "op": "remove_config", "k": 3 }));
        let t2 = parse(&json!({
            "n": 2, "active": 1,
            "rows": [{ "param": "2.t", "values": [1.0, 2.0], "picks": [{ "k": 2, "from": 1, "scale": 2.0, "offset": 0.0 }] }],
        }));
        assert_eq!(
            t2.pick(0, 1),
            Some(Pick {
                k: 2,
                from: 1,
                scale: 2.0,
                offset: 0.0
            })
        );
        // コピーと貼り付け（名前つきの行は名前の行へ、数だけなら今のセルから右へ）
        let txt = copy_text(&t, &[1]);
        assert_eq!(txt, "G1.shift\t0\t0.5\t1");
        let ops = paste_ops(&t, 0, 0, &txt.replace("0.5", "0.7")).unwrap();
        assert_eq!(
            ops,
            vec![json!({ "op": "set_config_row", "param": "G1.shift", "values": [0.0, 0.7, 1.0] })]
        );
        let ops = paste_ops(&t, 0, 1, "5\t6").unwrap();
        assert_eq!(
            ops,
            vec![json!({ "op": "set_config_row", "param": "2.t", "values": [1.0, 5.0, 6.0] })]
        );
        assert!(paste_ops(&t, 0, 0, "1\tx").is_err());
        let c = Cands {
            surfs: vec![
                ("STANDARD".into(), String::new()),
                ("EVENASPH".into(), "L1".into()),
                ("COORDBRK".into(), String::new()),
            ],
            units: vec!["G1".into()],
        };
        let all = all_params(&c);
        assert!(all.contains(&"1.a4".to_string()) && all.contains(&"2.tx".to_string()));
        assert!(!all.contains(&"2.r".to_string()));
        assert_eq!(complete(&c, "g1.s"), vec!["G1.shift".to_string()]);
        assert_eq!(
            complete(&c, "2.t")[..2],
            ["2.t".to_string(), "2.tx".to_string()]
        );
        assert!(complete(&c, "").is_empty());
        assert_eq!(param_desc("3.t"), "面 3 の厚さ");
        assert_eq!(param_desc("G1.shift"), "G1 の移動");
    }
}
