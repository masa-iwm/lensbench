//! パラメータの名前（`3.t`・`G1.shift` など）を打つ欄の補完と、面・玉・群から階層で選ぶ欄。
//!
//! 多重構成・最適化・公差・入れ替え・ズーム・系のパネルで共通に使う。候補は欄に入力があるとき・欄を開いたときだけ作る。

use crate::i18n::{tr, trf};
use egui::{Key, Response, Ui};
use lensbench_core::surface::SurfType;

/// 名前の候補を作るための系の面・玉・群
#[derive(Debug, Clone, Default)]
pub struct Sys {
    /// 面ごとの (面タイプ, コメント, 鏡か)
    pub surfs: Vec<(String, String, bool)>,
    /// 玉・群の名前
    pub units: Vec<String>,
    /// 波長の数（wave1〜）
    pub waves: usize,
}

/// どの名前を候補にするか
#[derive(Debug, Clone, Copy)]
pub enum Set<'a> {
    /// 多重構成の行
    Config,
    /// 最適化・ズームの変数（組合せの系なら面の t・r・k だけ）
    Var { combo: bool },
    /// 公差・系のパネル（物体の位置・波長も）
    Any,
    /// 玉・群の名前だけ（公差の玉・群の欄）
    Units,
    /// 入れ替えの変数: 組合せの間隔 (id, 説明) の `id.t` と玉・群
    Swap { gaps: &'a [(String, String)] },
}

/// パラメータの名前の説明（3.t → 面 3 の厚さ、G1.shift → G1 の移動）
pub fn param_desc(name: &str) -> String {
    let Some((who, k)) = name.trim().rsplit_once('.') else {
        let n = name.trim();
        return match n {
            "objx" => tr("物体の位置 X").into(),
            "objy" => tr("物体の位置 Y").into(),
            _ if n.starts_with("wave") => trf("波長 {}", &[&&n[4..]]),
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

/// 面タイプの係数（偶数次非球面・自由曲面 a2〜a16、Zernike z1〜z37）
fn coef_kinds(typ: &str) -> Vec<String> {
    let t = lensbench_core::edit::surf_type_parse(typ).ok();
    let mut v = Vec::new();
    if t.is_some_and(|t| t == SurfType::EvenAsph || lensbench_core::freeform::is_freeform(t)) {
        v.extend((1..=8).map(|i| format!("a{}", 2 * i)));
    }
    if matches!(t, Some(SurfType::ZernikeStd | SurfType::ZernikeFringe)) {
        v.extend((1..=37).map(|i| format!("z{i}")));
    }
    v
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// 面 j の種類 k を core が受け付けるか（core の `System::check_param` と同じ条件）
fn surf_ok(sys: &Sys, j: usize, k: &str) -> bool {
    let (typ, _, mirror) = &sys.surfs[j];
    let t = lensbench_core::edit::surf_type_parse(typ).ok();
    let curved = t.is_some_and(|t| {
        matches!(t, SurfType::Standard | SurfType::EvenAsph)
            || lensbench_core::freeform::is_freeform(t)
    });
    let cb = t == Some(SurfType::CoordBrk);
    match k {
        "r" | "c" | "k" => j > 0 && curved,
        "dx" | "dy" | "tx" | "ty" | "tz" => j > 0,
        "t" => j + 1 < sys.surfs.len(),
        "dn" | "disp" => !mirror && !cb,
        _ => true,
    }
}

/// 面 j で候補にする種類
fn surf_kinds(sys: &Sys, j: usize, set: Set) -> Vec<String> {
    let typ = sys.surfs[j].0.as_str();
    let cb = typ == "COORDBRK";
    let mut v = match set {
        Set::Config if cb => strs(&["t", "dx", "dy", "tx", "ty", "tz"]),
        Set::Config => strs(&["r", "t", "k", "dn", "disp"]),
        Set::Var { combo: true } => strs(&["t", "r", "k"]),
        Set::Var { .. } if cb => strs(&["t", "dx", "dy", "tx", "ty", "tz"]),
        Set::Var { .. } => strs(&["r", "c", "t", "k", "dx", "dy", "tx", "ty", "tz"]),
        Set::Any if cb => strs(&["t", "dx", "dy", "tx", "ty", "tz"]),
        Set::Any => strs(&[
            "r", "c", "t", "k", "dx", "dy", "tx", "ty", "tz", "dn", "disp",
        ]),
        Set::Units | Set::Swap { .. } => return Vec::new(),
    };
    if !cb && !matches!(set, Set::Var { combo: true }) {
        v.extend(coef_kinds(typ));
    }
    v.retain(|k| surf_ok(sys, j, k));
    v
}

/// 玉・群で候補にする種類
fn unit_kinds(set: Set) -> &'static [&'static str] {
    match set {
        Set::Config | Set::Any => &["shift", "dx", "dy", "tx", "ty", "tz", "dn", "disp"],
        Set::Var { combo: true } | Set::Units => &[],
        Set::Var { .. } | Set::Swap { .. } => &["shift", "dx", "dy", "tx", "ty", "tz"],
    }
}

/// 候補の (名前, 説明) の全部
pub fn names(sys: &Sys, set: Set) -> Vec<(String, String)> {
    let mut v: Vec<String> = Vec::new();
    if let Set::Swap { gaps } = set {
        let mut out: Vec<(String, String)> = gaps
            .iter()
            .map(|(id, d)| (format!("{id}.t"), d.clone()))
            .collect();
        for u in &sys.units {
            for k in unit_kinds(set) {
                let n = format!("{u}.{k}");
                let d = param_desc(&n);
                out.push((n, d));
            }
        }
        return out;
    }
    if let Set::Units = set {
        return sys
            .units
            .iter()
            .map(|u| (u.clone(), String::new()))
            .collect();
    }
    for j in 0..sys.surfs.len() {
        for k in surf_kinds(sys, j, set) {
            v.push(format!("{j}.{k}"));
        }
    }
    for u in &sys.units {
        for k in unit_kinds(set) {
            v.push(format!("{u}.{k}"));
        }
    }
    if matches!(set, Set::Config | Set::Any) {
        v.push("objx".into());
        v.push("objy".into());
    }
    if matches!(set, Set::Any) {
        v.extend((1..=sys.waves).map(|i| format!("wave{i}")));
    }
    v.into_iter()
        .map(|n| {
            let d = param_desc(&n);
            (n, d)
        })
        .collect()
}

/// 打った文字に合う候補（前方一致を先に、次に名前・説明の部分一致）。多すぎるときは先頭だけ
pub fn complete(all: Vec<(String, String)>, text: &str) -> Vec<(String, String)> {
    let q = text.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let (mut head, mut rest) = (Vec::new(), Vec::new());
    for (n, d) in all {
        let l = n.to_lowercase();
        if l.starts_with(&q) {
            head.push((n, d));
        } else if l.contains(&q) || d.to_lowercase().contains(&q) {
            rest.push((n, d));
        }
    }
    head.extend(rest);
    head.truncate(200);
    head
}

/// 欄の補完の状態（↑↓ で選んでいる候補・Tab の後に欄へ戻す）
#[derive(Clone, Copy, Default)]
struct CompState {
    pick: usize,
    refocus: bool,
}

/// 補完つきの名前の欄。戻り値の bool は名前を決めた（Enter・候補のクリック）とき true
///
/// 打つと候補（説明付き）が出て、↑↓ で選び、Tab で欄に入れ、Enter・クリックで決める（打った名前が候補と同じなら Enter はそのまま）。
pub fn edit(
    ui: &mut Ui,
    s: &mut String,
    width: f32,
    hint: &str,
    sys: &Sys,
    set: Set,
) -> (Response, bool) {
    let resp = ui.add(
        egui::TextEdit::singleline(s)
            .hint_text(hint)
            .desired_width(width),
    );
    let sid = resp.id.with("param_comp");
    let mut st: CompState = ui.data(|d| d.get_temp(sid)).unwrap_or_default();
    if std::mem::take(&mut st.refocus) {
        resp.request_focus();
    }
    let enter = resp.lost_focus() && ui.input(|x| x.key_pressed(Key::Enter));
    let mut chosen = enter;
    if resp.has_focus() || enter {
        let m = complete(names(sys, set), s);
        let exact = m.iter().any(|x| x.0.eq_ignore_ascii_case(s.trim()));
        if resp.changed() || st.pick >= m.len() {
            st.pick = 0;
        }
        let (up, down, tab) = ui.input(|x| {
            (
                x.key_pressed(Key::ArrowUp),
                x.key_pressed(Key::ArrowDown),
                x.key_pressed(Key::Tab),
            )
        });
        if up && st.pick > 0 {
            st.pick -= 1;
        }
        if down && st.pick + 1 < m.len() {
            st.pick += 1;
        }
        if !m.is_empty() && !(exact && m.len() == 1) {
            let mut clicked = None;
            egui::Area::new(resp.id.with("comp"))
                .order(egui::Order::Foreground)
                .fixed_pos(resp.rect.left_bottom())
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(240.0);
                        egui::ScrollArea::vertical()
                            .max_height(240.0)
                            .show(ui, |ui| {
                                for (k, (n, d)) in m.iter().enumerate() {
                                    let r = ui.selectable_label(k == st.pick, format!("{n}  {d}"));
                                    if k == st.pick && (up || down) {
                                        r.scroll_to_me(None);
                                    }
                                    if r.clicked() {
                                        clicked = Some(n.clone());
                                    }
                                }
                            });
                    });
                });
            if let Some(n) = clicked {
                *s = n;
                chosen = true;
            } else if tab {
                if let Some(n) = m.get(st.pick) {
                    *s = n.0.clone();
                    st.refocus = true;
                }
            } else if enter && !exact {
                if let Some(n) = m.get(st.pick) {
                    *s = n.0.clone();
                }
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(sid, st));
    (resp, chosen)
}

/// 名前を階層で選ぶ欄の中身（面 → 面番号 → 種類、玉・群 → 名前 → 種類、ほか）。選んだ名前を返す
pub fn menu(ui: &mut Ui, sys: &Sys, set: Set) -> Option<String> {
    let mut out = None;
    let mut pick = |ui: &mut Ui, n: &str, label: String| {
        if ui.button(label).clicked() {
            out = Some(n.to_string());
            ui.close_menu();
        }
    };
    if let Set::Swap { gaps } = set {
        if !gaps.is_empty() {
            ui.menu_button(tr("間隔"), |ui| {
                for (id, d) in gaps {
                    let n = format!("{id}.t");
                    pick(ui, &n, format!("{n}  {d}"));
                }
            });
        }
    }
    if let Set::Units = set {
        for u in &sys.units {
            pick(ui, u, u.clone());
        }
        return out;
    }
    let any_surf = (0..sys.surfs.len()).any(|j| !surf_kinds(sys, j, set).is_empty());
    if any_surf {
        ui.menu_button(tr("面"), |ui| {
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    for (j, (typ, comment, _)) in sys.surfs.iter().enumerate() {
                        let kinds = surf_kinds(sys, j, set);
                        if kinds.is_empty() {
                            continue;
                        }
                        let label = if comment.is_empty() {
                            format!("{j}  {typ}")
                        } else {
                            format!("{j}  {typ}  {comment}")
                        };
                        ui.menu_button(label, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(400.0)
                                .show(ui, |ui| {
                                    for k in kinds {
                                        let n = format!("{j}.{k}");
                                        pick(ui, &n, format!("{k}  {}", param_desc(&n)));
                                    }
                                });
                        });
                    }
                });
        });
    }
    let uk = unit_kinds(set);
    if !uk.is_empty() {
        ui.add_enabled_ui(!sys.units.is_empty(), |ui| {
            ui.menu_button(tr("玉・群"), |ui| {
                for u in &sys.units {
                    ui.menu_button(u, |ui| {
                        for k in uk {
                            let n = format!("{u}.{k}");
                            pick(ui, &n, format!("{k}  {}", param_desc(&n)));
                        }
                    });
                }
            });
        });
    }
    if matches!(set, Set::Config | Set::Any) {
        ui.menu_button(tr("物体の位置"), |ui| {
            for n in ["objx", "objy"] {
                pick(ui, n, format!("{n}  {}", param_desc(n)));
            }
        });
    }
    if matches!(set, Set::Any) && sys.waves > 0 {
        ui.menu_button(tr("波長"), |ui| {
            for i in 1..=sys.waves {
                let n = format!("wave{i}");
                pick(ui, &n, format!("{n}  {}", param_desc(&n)));
            }
        });
    }
    out
}

/// 階層で選ぶ […] ボタン。選んだ名前を返す
pub fn menu_button(ui: &mut Ui, sys: &Sys, set: Set, hover: &str) -> Option<String> {
    let r = ui.menu_button("…", |ui| menu(ui, sys, set));
    r.response.on_hover_text(hover);
    r.inner.flatten()
}

/// 補完つきの欄と […] を並べる（欄の名前を […] で選んだ名前に置き換える）。戻り値の bool は名前を決めたとき true
pub fn edit_pick(
    ui: &mut Ui,
    s: &mut String,
    width: f32,
    hint: &str,
    sys: &Sys,
    set: Set,
) -> (Response, bool) {
    let (resp, mut chosen) = edit(ui, s, width, hint, sys, set);
    if let Some(n) = menu_button(ui, sys, set, tr("面・玉・群から選ぶ")) {
        *s = n;
        chosen = true;
    }
    (resp, chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 面 0 物体、1 偶数次非球面、2 座標ブレーク、3 トロイダル、4 鏡、5 像面
    fn sys() -> Sys {
        Sys {
            surfs: vec![
                ("STANDARD".into(), String::new(), false),
                ("EVENASPH".into(), "L1".into(), false),
                ("COORDBRK".into(), String::new(), false),
                ("TOROIDAL".into(), String::new(), false),
                ("STANDARD".into(), String::new(), true),
                ("STANDARD".into(), String::new(), false),
            ],
            units: vec!["G1".into()],
            waves: 2,
        }
    }

    #[test]
    fn names_by_set() {
        let s = sys();
        let has = |set: Set, n: &str| names(&s, set).iter().any(|x| x.0 == n);
        assert!(has(Set::Config, "1.a4") && has(Set::Config, "2.tx"));
        assert!(!has(Set::Config, "2.r") && !has(Set::Config, "wave1"));
        assert!(
            has(Set::Var { combo: false }, "1.c") && has(Set::Var { combo: false }, "G1.shift")
        );
        assert!(!has(Set::Var { combo: false }, "objx"));
        let combo = names(&s, Set::Var { combo: true });
        assert!(combo
            .iter()
            .all(|x| x.0.ends_with(".t") || x.0.ends_with(".r") || x.0.ends_with(".k")));
        assert!(has(Set::Any, "wave2") && has(Set::Any, "1.dn") && !has(Set::Any, "wave3"));
        assert_eq!(
            names(&s, Set::Units),
            vec![("G1".to_string(), String::new())]
        );
        let gaps = [("d1".to_string(), "lens1 の後".to_string())];
        let sw = names(&s, Set::Swap { gaps: &gaps });
        assert_eq!(sw[0], ("d1.t".to_string(), "lens1 の後".to_string()));
        assert!(sw.iter().any(|x| x.0 == "G1.shift"));
    }

    /// core の check_param が断る名前は候補に出さない（面 0 の曲率・傾き偏心、像面の厚さ、鏡の dn、曲率の無い面タイプ）
    #[test]
    fn names_follow_check_param() {
        let s = sys();
        for set in [
            Set::Config,
            Set::Var { combo: false },
            Set::Var { combo: true },
            Set::Any,
        ] {
            let n: Vec<String> = names(&s, set).into_iter().map(|x| x.0).collect();
            for bad in [
                "0.r", "0.c", "0.k", "0.dx", "0.tx", "3.r", "3.k", "5.t", "4.dn", "4.disp", "2.dn",
            ] {
                assert!(!n.contains(&bad.to_string()), "{set:?} {bad}");
            }
            assert!(
                n.contains(&"0.t".to_string()) && n.contains(&"4.t".to_string()),
                "{set:?}"
            );
        }
        let any: Vec<String> = names(&s, Set::Any).into_iter().map(|x| x.0).collect();
        assert!(any.contains(&"3.t".to_string()) && any.contains(&"3.dn".to_string()));
        assert!(any.contains(&"5.r".to_string()) && any.contains(&"4.r".to_string()));
        assert!(!any.contains(&"3.a4".to_string()) && any.contains(&"1.a16".to_string()));
    }

    #[test]
    fn completion_order() {
        let s = sys();
        let c = |t: &str| -> Vec<String> {
            complete(names(&s, Set::Config), t)
                .into_iter()
                .map(|x| x.0)
                .collect()
        };
        assert_eq!(c("g1.s"), vec!["G1.shift".to_string()]);
        assert_eq!(c("2.t")[..2], ["2.t".to_string(), "2.tx".to_string()]);
        assert!(c("").is_empty());
        assert_eq!(param_desc("3.t"), "面 3 の厚さ");
        assert_eq!(param_desc("G1.shift"), "G1 の移動");
        assert_eq!(param_desc("wave2"), "波長 2");
    }
}
