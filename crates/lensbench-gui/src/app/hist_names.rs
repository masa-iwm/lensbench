//! 元に戻す・やり直すの段の名前と中身: 組合せの前後の違い（[`Edit::between`] の場所と値）を、利用者の言葉の
//! 「何が・どの値からどの値へ」に直す（例「要素 2 (…) surface 1 thickness: 5 → 6」「Wavelength 2 [µm]: 0.5876 → 0.6563」）

use std::collections::{BTreeMap, BTreeSet};

use super::super::edit::{Edit, Seg};
use super::*;
use crate::state_table::{field_label, SysQ, PLACE};

/// 中身の行の数の上限（多いときは「ほか N か所」）
const MAX_DETAILS: usize = 12;

/// 元に戻すの 1 段の名前と中身
#[derive(Debug, Clone)]
pub(crate) struct Step {
    /// 一覧の 1 行（操作の種類と、変わった所の要約）
    pub title: String,
    /// 変わった所と前後の値（1 行ずつ。ホバーで見せる）
    pub details: Vec<String>,
    /// 積んだ時刻（一覧に「何分前」と出す）
    pub at: web_time::Instant,
    /// 文書を替えた段（新規・開く）。一覧で文書の中の段と区切る
    pub doc: bool,
}

impl Step {
    pub(crate) fn new(title: impl Into<String>, details: Vec<String>) -> Step {
        Step {
            title: title.into(),
            details,
            at: web_time::Instant::now(),
            doc: false,
        }
    }

    /// 文書を替えた段（新規・開く）
    pub(crate) fn doc(title: &str) -> Step {
        Step {
            doc: true,
            ..Step::new(title, Vec::new())
        }
    }

    /// 積んでからの時間（「12 s ago」「3 min ago」「2 h ago」）
    pub(crate) fn age(&self) -> String {
        let s = self.at.elapsed().as_secs();
        if s < 60 {
            trf("{} s ago", &[&s])
        } else if s < 3600 {
            trf("{} min ago", &[&(s / 60)])
        } else {
            trf("{} h ago", &[&(s / 3600)])
        }
    }

    /// ホバーの文: 名前、変わった所の一覧
    pub(crate) fn tip(&self) -> String {
        let mut t = self.title.clone();
        for d in &self.details {
            t.push_str("\n• ");
            t.push_str(d);
        }
        t
    }
}

/// a から b への変更の段。kind は操作の種類（最適化の反映・差し替えなど。欄の書き換えは None）
pub(crate) fn describe(a: &Combo, b: &Combo, kind: Option<&str>) -> Step {
    let base = super::combo_change(a, b);
    let mut details = if a.picks.len() == b.picks.len() {
        changes(a, b)
    } else {
        Vec::new()
    };
    let title = match details.as_slice() {
        [] => base,
        [one] => one.clone(),
        [first, rest @ ..] => trf("{} and {} more", &[first, &rest.len()]),
    };
    if details.len() > MAX_DETAILS {
        let more = details.len() - MAX_DETAILS;
        details.truncate(MAX_DETAILS);
        details.push(trf("{} more", &[&more]));
    }
    let title = match kind {
        Some(k) => format!("{k}: {title}"),
        None => title,
    };
    Step::new(title, details)
}

/// 変わった所の行（「何: 前 → 後」）。面の上書きは面と値の名前ごと
fn changes(a: &Combo, b: &Combo) -> Vec<String> {
    let Some(e) = Edit::between(a, b) else {
        return Vec::new();
    };
    let (va, vb) = (
        serde_json::to_value(a).unwrap_or(Value::Null),
        serde_json::to_value(b).unwrap_or(Value::Null),
    );
    let mut out = Vec::new();
    // 面の上書きの並びは並びごとに 1 度だけ比べる
    let mut seen_edits: Vec<&[Seg]> = Vec::new();
    for op in &e.ops {
        if let Some(list) = edits_list(&op.path) {
            if !seen_edits.contains(&list) {
                seen_edits.push(list);
                out.extend(edit_changes(b, list, at(&va, list), at(&vb, list)));
            }
            continue;
        }
        // 多重構成の表の並びの順位・面の上書きの印は、利用者の変更の結果として変わる所（中身に出さない）
        if op
            .path
            .iter()
            .any(|s| matches!(s, Seg::Key(k) if k == "mc_rank" || k == "edits_for"))
        {
            continue;
        }
        let old = at(&va, &op.path);
        let new = op.value.as_ref();
        out.push(format!(
            "{}: {} → {}",
            what(b, &op.path),
            show_val(old),
            show_val(new)
        ));
    }
    out
}

/// path の所の値
fn at<'a>(root: &'a Value, path: &[Seg]) -> Option<&'a Value> {
    path.iter().try_fold(root, |v, s| match (s, v) {
        (Seg::Key(k), Value::Object(m)) => m.get(k),
        (Seg::Index(i), Value::Array(a)) => a.get(*i),
        _ => None,
    })
}

/// 面の上書きの並び（`picks/k/edits`・`picks/k/state_edits/状態`）の中の場所なら、その並びの場所
fn edits_list(path: &[Seg]) -> Option<&[Seg]> {
    match path {
        [Seg::Key(p), Seg::Index(_), Seg::Key(e), ..] if p == "picks" && e == "edits" => {
            Some(&path[..3])
        }
        [Seg::Key(p), Seg::Index(_), Seg::Key(e), Seg::Key(_), ..]
            if p == "picks" && e == "state_edits" =>
        {
            Some(&path[..4])
        }
        _ => None,
    }
}

/// 面の上書きの並びの前後を、面と値の名前ごとに比べた行
fn edit_changes(b: &Combo, list: &[Seg], old: Option<&Value>, new: Option<&Value>) -> Vec<String> {
    let map = |v: Option<&Value>| -> BTreeMap<(u64, String), Value> {
        v.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let s = e.get("surface")?.as_u64()?;
                let f = e.get("field")?.as_str()?.to_string();
                Some(((s, f), e.get("value").cloned().unwrap_or(Value::Null)))
            })
            .collect()
    };
    let (ma, mb) = (map(old), map(new));
    let el = what(b, &list[..2]);
    let state = match list.get(3) {
        Some(Seg::Key(st)) => format!(" [{st}]"),
        _ => String::new(),
    };
    let keys: BTreeSet<&(u64, String)> = ma.keys().chain(mb.keys()).collect();
    keys.into_iter()
        .filter(|k| ma.get(*k) != mb.get(*k))
        .map(|(s, f)| {
            format!(
                "{}{state}: {} → {}",
                trf("{} surface {} {}", &[&el, &(s + 1), &field_label(f, None)]),
                show_val(ma.get(&(*s, f.clone()))),
                show_val(mb.get(&(*s, f.clone())))
            )
        })
        .collect()
}

/// 場所の名前（要素・系の値の欄の名前。知らない場所は鍵をつないだもの）
fn what(c: &Combo, path: &[Seg]) -> String {
    let key = |i: usize| match path.get(i) {
        Some(Seg::Key(k)) => Some(k.as_str()),
        _ => None,
    };
    let idx = |i: usize| match path.get(i) {
        Some(Seg::Index(k)) => Some(*k),
        _ => None,
    };
    match (key(0), idx(1)) {
        (Some("picks"), Some(k)) => {
            let label = c.picks.get(k).map(|p| p.label()).unwrap_or_default();
            let el = trf("Element {} ({})", &[&(k + 1), &label]);
            if path.len() <= 2 {
                return el;
            }
            return format!("{el} {}", pick_field(&path[2..]));
        }
        (Some("waves"), Some(i)) => match key(2) {
            Some("um") => return SysQ::Wave(i).label(),
            Some("w") => return SysQ::WaveW(i).label(),
            _ => {}
        },
        (Some("fields"), Some(i)) => match key(2) {
            Some("x") => return SysQ::FieldX(i).label(),
            Some("y") => return SysQ::FieldY(i).label(),
            Some("w") => return SysQ::FieldW(i).label(),
            _ => {}
        },
        (Some("state_sys"), _) => {
            if let Some(Seg::Key(st)) = path.get(1) {
                return format!("{} [{st}]", what(c, &path[2..]));
            }
        }
        (Some("groups"), Some(g)) if key(2) == Some("shift") => {
            let name = c.groups.get(g).map(|g| g.name.as_str()).unwrap_or_default();
            return format!("{name} {}", tr("shift"));
        }
        _ => {}
    }
    match (key(0), path.len()) {
        (Some("aperture"), 1) => SysQ::ApType.label(),
        (Some("ap_value"), 1) => SysQ::ApValue.label(),
        (Some("field_kind"), 1) => SysQ::FieldType.label(),
        (Some("primary"), 1) => SysQ::Primary.label(),
        (Some("object_distance"), 1) => SysQ::ObjDist.label(),
        (Some("telecentric"), 1) => SysQ::Telecentric.label(),
        (Some("epd"), 1) => tr("Stop diameter").into(),
        (Some("waves"), 1) => tr("Wavelengths").into(),
        (Some("fields"), 1) => tr("Fields").into(),
        (Some("state"), 1) => tr("Current state").into(),
        (Some("states"), _) => tr("States").into(),
        _ => join(path),
    }
}

/// 要素の中の場所の名前
fn pick_field(rest: &[Seg]) -> String {
    let key = |i: usize| match rest.get(i) {
        Some(Seg::Key(k)) => Some(k.as_str()),
        _ => None,
    };
    let idx = |i: usize| match rest.get(i) {
        Some(Seg::Index(k)) => Some(*k),
        _ => None,
    };
    let state = |i: usize| match rest.get(i) {
        Some(Seg::Key(st)) => format!(" [{st}]"),
        _ => String::new(),
    };
    match key(0) {
        Some("air") => tr("air").into(),
        Some("state_air") => format!("{}{}", tr("air"), state(1)),
        Some("semi") => field_label("semi", None),
        Some("decenter") => idx(1)
            .and_then(|i| PLACE.get(i))
            .copied()
            .unwrap_or("decenter")
            .into(),
        Some("tilt") => idx(1)
            .and_then(|i| PLACE.get(i + 2))
            .copied()
            .unwrap_or("tilt")
            .into(),
        Some("state_place") => format!("{}{}", join(&rest[2.min(rest.len())..]), state(1)),
        Some("on") => "ON".into(),
        Some("state_on") => format!("ON{}", state(1)),
        Some("coat") => field_label("coating", None),
        Some("surfs") => match (idx(1), key(2)) {
            (Some(i), Some(f)) => trf("surface {} {}", &[&(i + 1), &surf_field(f)]),
            _ => join(rest),
        },
        Some("state_stop") => format!("{}{}", tr("Stop surface"), state(1)),
        Some("stop_front") => tr("Stop surface").into(),
        _ => join(rest),
    }
}

/// 自作の玉の面の欄（r1・r2・t・glass などの鍵）の名前
fn surf_field(f: &str) -> String {
    match f {
        "glass" => field_label("material", None),
        "r" | "t" | "k" | "semi" | "coating" => field_label(f, None),
        _ => f.to_string(),
    }
}

/// 知らない場所: 鍵と番号をつないだもの（番号は 1 始まり）
fn join(path: &[Seg]) -> String {
    path.iter()
        .map(|s| match s {
            Seg::Key(k) => k.clone(),
            Seg::Index(i) => (i + 1).to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 値の表示（数は有効数字 6 桁、並び・中身は要約、無いは —）
fn show_val(v: Option<&Value>) -> String {
    let Some(v) = v else {
        return "—".into();
    };
    if let Some(x) = crate::widgets::json_num(v) {
        return crate::plots::fmt_sig(x, 6);
    }
    match v {
        Value::Null => "—".into(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) if s.is_empty() => "\"\"".into(),
        Value::String(s) => s.clone(),
        Value::Array(a) if a.len() <= 4 && a.iter().all(|x| !x.is_array() && !x.is_object()) => {
            format!(
                "[{}]",
                a.iter()
                    .map(|x| show_val(Some(x)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        Value::Array(a) => trf("{} items", &[&a.len()]),
        Value::Object(_) => "{…}".into(),
        Value::Number(n) => n.to_string(),
    }
}

/// 操作の種類の名前（段の名前の頭）。欄の書き換え・表示だけの操作は None
pub(in crate::app) fn msg_kind(msg: &Msg) -> Option<&'static str> {
    Some(match msg {
        Msg::Opt(_) | Msg::RunOpt => tr("Optimize"),
        Msg::Swap(_) => tr("Swap search"),
        Msg::Zoom(_) => tr("Zoom"),
        Msg::Tol(_) | Msg::RunTol => tr("Tolerance"),
        Msg::Catalog(_) => tr("Catalog"),
        Msg::LensData(_) => tr("Lens Data"),
        Msg::Combo(_) => tr("Combination"),
        Msg::FocusImage => tr("Image at paraxial focus"),
        Msg::CycleConfig(_) => tr("State"),
        Msg::SetParam(..) | Msg::AddParam(_) | Msg::RemoveParam(_) => tr("Parameters"),
        Msg::Replayed(_, m) => return msg_kind(m),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Combo {
        let mut c = Combo::default();
        c.insert_plane(0);
        c.insert_plane(1);
        c
    }

    /// 段の名前: 系の値・要素の値・面の上書きは「何: 前 → 後」、変わった所が多いと「ほか N か所」、操作の種類を頭に
    #[test]
    fn steps_name_what_changed() {
        let a = sample();
        let mut b = a.clone();
        b.waves[0].um = 0.6563;
        let s = describe(&a, &b, None);
        assert!(
            s.title.contains("Wavelength 1") && s.title.contains("→ 0.6563"),
            "{s:?}"
        );
        let mut b = a.clone();
        b.picks[1].air = 12.5;
        let s = describe(&a, &b, Some("Optimize"));
        assert!(s.title.starts_with("Optimize: Element 2 ("), "{s:?}");
        assert!(
            s.title.ends_with("air: 1 → 12.5") || s.title.contains("→ 12.5"),
            "{s:?}"
        );
        // 面の上書き（面の表・多重構成の表の面の行）
        let mut b = a.clone();
        b.picks[0].edits.push(crate::combo::SurfEdit {
            surface: 0,
            field: "t".into(),
            value: json!(6.0),
        });
        let s = describe(&a, &b, None);
        assert!(s.title.contains("surface 1 thickness: — → 6"), "{s:?}");
        let mut c = b.clone();
        c.picks[0].edits[0].value = json!(7.0);
        let s = describe(&b, &c, None);
        assert!(s.title.contains("surface 1 thickness: 6 → 7"), "{s:?}");
        // 多くの所: 1 行目＋ほか、中身は全部
        let mut b = a.clone();
        b.waves[0].um = 0.5;
        b.fields[1].y = 9.0;
        b.epd = 3.0;
        let s = describe(&a, &b, None);
        assert_eq!(s.details.len(), 3, "{s:?}");
        assert!(s.title.contains("2 more"), "{s:?}");
        assert!(s.tip().lines().count() == 4);
        // 要素の追加・削除は今までの名前
        let mut b = a.clone();
        b.insert_plane(2);
        assert!(describe(&a, &b, None).title.starts_with("Add element"));
        assert!(Step::new("x", Vec::new()).age().contains('s'));
    }
}
