//! 組合せの系のレンズデータの表: 変えられないセルの理由と、面の上書きのあるセル（印と個別に外す、文書モデル）

use super::*;

pub(super) fn read_only(
    rows: &[lens_data::Row],
    spans: &[Option<[usize; 2]>],
    picks: &[&combo::Pick],
    lead: bool,
    j: usize,
    col: lens_data::Col,
) -> Option<String> {
    use lens_data::Col;
    let img = rows.len().saturating_sub(1);
    if j == 0 || j >= img {
        return Some(tr("Set the object and image surfaces in the combination settings").into());
    }
    let Some(k) = spans
        .iter()
        .position(|s| s.is_some_and(|s| s[0] <= j && j <= s[1]))
    else {
        if lead && rows[j].stop && col == Col::T {
            return None;
        }
        return Some(tr("Surface outside the combination elements (only the thickness of the leading stop can be changed)").into());
    };
    if picks.get(k).is_some_and(|p| p.fixed())
        && col == Col::T
        && spans[k].is_some_and(|s| s[1] == j)
    {
        return Some(tr("The gap after an element locked with its position cannot change").into());
    }
    if picks
        .get(k)
        .is_some_and(|p| p.kind == combo::ElemKind::Splitter)
    {
        return Some(tr("Set the BS in the element settings").into());
    }
    let end = spans[k].map(|s| s[1]).unwrap_or(j);
    if col == Col::T && j == end {
        return None;
    }
    if rows[j].is_cb() {
        return Some(tr("Set decenter and tilt in the element settings").into());
    }
    None
}

/// 列 col の値の上書きの名前（面の上書き `edits` の field。パラメータの列は面タイプのパラメータの全部）
pub(super) fn override_fields(col: lens_data::Col) -> &'static [&'static str] {
    use lens_data::Col;
    match col {
        Col::Type => &["type"],
        Col::Comment => &["comment"],
        Col::R => &["r", "c"],
        Col::T => &["t"],
        Col::Material => &["material"],
        Col::Coating => &["coating"],
        Col::Semi => &["semi", "semi_auto"],
        Col::Chip => &["chip"],
        Col::Mech => &["mech", "mech_auto"],
        Col::Conic => &["k"],
        Col::Par(_) => &["asph", "cx", "focal", "jones", "cb", "cb_order"],
    }
}

/// レイアウト図の寸法のうち、系の厚みがソルブ・ピックアップで決まるもの（要素 g の後の空気は要素の最後の面の厚み、
/// 先頭に自動で置く絞りから最初の要素までは絞りの面の厚み。レンズデータの表で変えられないのと同じ）
pub(super) fn solved_gaps(
    rows: &[lens_data::Row],
    spans: &[Option<[usize; 2]>],
    picks: &[&combo::Pick],
    lead: bool,
) -> Vec<combo::SolvedGap> {
    let at = |gap: usize, j: usize| {
        let r = rows.get(j)?;
        let (mark, what) = lens_data::Col::T.solved(r)?;
        Some(combo::SolvedGap {
            gap,
            value: r.t,
            mark,
            what,
        })
    };
    let mut out: Vec<combo::SolvedGap> = spans
        .iter()
        .enumerate()
        .filter(|(k, _)| {
            picks
                .get(*k)
                .is_some_and(|p| p.kind != combo::ElemKind::Splitter)
        })
        .filter_map(|(k, sp)| at(k, (*sp)?[1]))
        .collect();
    if lead {
        let inside = |j: usize| spans.iter().flatten().any(|s| s[0] <= j && j <= s[1]);
        if let Some(j) = rows.iter().position(|r| r.stop).filter(|&j| !inside(j)) {
            out.extend(at(combo::STOP_GAP, j));
        }
    }
    out
}

impl Core {
    /// 今の組合せの系で、ソルブ・ピックアップで厚みが決まる寸法（[`solved_gaps`]）
    pub(super) fn solved_gaps(&self) -> Vec<combo::SolvedGap> {
        let lead = self.doc.combo.lead_stop();
        let live = self.doc.combo.span_ids();
        let spans = lens_data::elem_spans(&live, &self.sys.elem_blocks, lead, &self.sys.ld_rows);
        let picks = self.doc.combo.chain_view().picks;
        solved_gaps(&self.sys.ld_rows, &spans, &picks, lead)
    }
}

/// 面の値 key がソルブ・ピックアップで決まっていれば、変えられない理由（開いた系で serve が断るのと同じ。
/// 組合せでは上書きを入れると系の作り直しが誤りで止まるか、ソルブが値を戻してしまう）
pub(super) fn solved_reason(row: &lens_data::Row, key: &str) -> Option<String> {
    use lens_data::Col;
    let col = match key {
        "t" => Col::T,
        "r" | "c" => Col::R,
        "semi" => Col::Semi,
        "k" => Col::Conic,
        "material" => Col::Material,
        "coating" => Col::Coating,
        _ => return None,
    };
    let (_, what) = col.solved(row)?;
    Some(trf(
        "{} decides this value. Remove it first (right-click the cell)",
        &[&what],
    ))
}

/// 面の表の行 r の面の値 field（面の上書きの JSON の形。平面の曲率半径は inf）。多重構成の表の面の行の基本の値
pub(crate) fn face_value(r: &lens_data::Row, field: &str) -> Option<Value> {
    Some(match field {
        "r" if r.r == 0.0 || !r.r.is_finite() => json!({ "$f": "inf" }),
        "r" => json!(r.r),
        "t" if r.t.is_finite() => json!(r.t),
        "t" => json!({ "$f": if r.t > 0.0 { "inf" } else { "-inf" } }),
        "k" => json!(r.k),
        "semi" => json!(r.semi),
        "ignore" => json!(r.ignore),
        "material" => json!(r.material),
        "coating" => json!(r.coating),
        "comment" => json!(r.comment),
        "asph" => json!(r.asph),
        "cb" => json!(r.cb),
        "aperture" => r.aperture.clone(),
        _ => return None,
    })
}

/// 組合せの系の面 j の要素と、要素の面の番号（座標ブレークを除く）
pub(super) fn face_of(
    rows: &[lens_data::Row],
    spans: &[Option<[usize; 2]>],
    j: usize,
) -> Option<(usize, usize)> {
    let k = spans
        .iter()
        .position(|s| s.is_some_and(|s| s[0] <= j && j <= s[1]))?;
    let a = spans[k]?[0];
    if !rows.get(j).is_some_and(|r| !r.is_cb()) {
        return None;
    }
    Some((k, (a..j).filter(|&i| !rows[i].is_cb()).count()))
}

/// 往路の 2 回目以降・復路の行を、要素の往路の範囲の面の行へ写したもの（[`pass_row`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PassRow {
    /// 同じ面の往路（要素の範囲）の行
    pub(super) own: usize,
    /// 復路の行の後ろの媒質・間隔を持つ往路の行（往路の 1 つ前の面。要素の最初の面なら None）
    pub(super) prev: Option<usize>,
    /// 復路の行
    pub(super) back: bool,
}

impl PassRow {
    /// 欄 field を書く往路の行（復路の行の媒質・間隔は 1 つ前の面）。書けなければ None
    pub(super) fn row_for(&self, field: &str) -> Option<usize> {
        if self.back && matches!(field, "t" | "material" | "dn" | "disp") {
            self.prev
        } else {
            Some(self.own)
        }
    }
}

/// 面の表の行 j が、要素の往路の範囲（spans、要素の id は ids）の外で、同じ要素のブロックを往路でもう一度通る範囲
/// （fwd）か復路の範囲（back、範囲の面を往路の逆の順に通る）にあれば、同じ面の往路の行。座標ブレークの行・範囲の中の
/// 行・どの要素にも無い行は None
pub(super) fn pass_row(
    rows: &[lens_data::Row],
    spans: &[Option<[usize; 2]>],
    ids: &[String],
    fwd: &[(String, Vec<[usize; 2]>)],
    back: &[(String, Vec<[usize; 2]>)],
    j: usize,
) -> Option<PassRow> {
    if !rows.get(j).is_some_and(|r| !r.is_cb())
        || spans.iter().flatten().any(|s| s[0] <= j && j <= s[1])
    {
        return None;
    }
    let faces = |r: [usize; 2]| -> Vec<usize> {
        (r[0]..=r[1].min(rows.len().saturating_sub(1)))
            .filter(|&i| !rows[i].is_cb())
            .collect()
    };
    let runs = |v: &[(String, Vec<[usize; 2]>)], id: &str| -> Vec<[usize; 2]> {
        v.iter()
            .filter(|(n, _)| n == id)
            .flat_map(|(_, r)| r.iter().copied())
            .filter(|r| r[0] <= j && j <= r[1])
            .collect()
    };
    for (k, id) in ids.iter().enumerate() {
        let Some(s) = spans.get(k).copied().flatten() else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let own = faces(s);
        if let Some(r) = runs(fwd, id).first() {
            let i = faces(*r).iter().position(|&x| x == j)?;
            return Some(PassRow {
                own: *own.get(i)?,
                prev: None,
                back: false,
            });
        }
        if let Some(r) = runs(back, id).first() {
            let f = faces(*r);
            let q = f.iter().position(|&x| x == j)?;
            let i = f.len() - 1 - q;
            return Some(PassRow {
                own: *own.get(i)?,
                prev: i.checked_sub(1).and_then(|p| own.get(p).copied()),
                back: true,
            });
        }
    }
    None
}

/// 面 j の列 col に面の上書きがあるか（組合せの系）
pub(super) fn overridden(
    rows: &[lens_data::Row],
    spans: &[Option<[usize; 2]>],
    picks: &[&combo::Pick],
    j: usize,
    col: lens_data::Col,
) -> bool {
    let Some((k, i)) = face_of(rows, spans, j) else {
        return false;
    };
    let names = override_fields(col);
    picks.get(k).is_some_and(|p| {
        p.edits
            .iter()
            .any(|e| e.surface == i && names.contains(&e.field.as_str()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_data::{Col, Row};

    #[test]
    fn override_marks_skip_cb_rows() {
        let row = |t: &str| Row {
            typ: t.into(),
            ..Default::default()
        };
        // 物体・要素（面・CB・面）・像
        let rows = vec![
            row("STANDARD"),
            row("STANDARD"),
            row("COORDBRK"),
            row("STANDARD"),
            row("STANDARD"),
        ];
        let spans = vec![Some([1, 3])];
        let mut p = combo::Pick::default();
        p.edits.push(combo::SurfEdit {
            surface: 1,
            field: "semi_auto".into(),
            value: serde_json::json!(false),
        });
        let picks = vec![&p];
        assert_eq!(face_of(&rows, &spans, 3), Some((0, 1)));
        assert_eq!(face_of(&rows, &spans, 2), None);
        assert!(overridden(&rows, &spans, &picks, 3, Col::Semi));
        assert!(!overridden(&rows, &spans, &picks, 3, Col::R));
        assert!(!overridden(&rows, &spans, &picks, 1, Col::Semi));
        assert!(!overridden(&rows, &spans, &picks, 2, Col::Semi));
    }

    /// 復路の行は同じ面の往路の行へ、後ろの媒質・間隔は往路の 1 つ前の面の行へ。往路の 2 回目は同じ番号の面
    #[test]
    fn later_pass_rows_map_to_the_element_span() {
        let row = |t: &str| Row {
            typ: t.into(),
            ..Default::default()
        };
        // 物体・玉 L（面 1〜3）・鏡 4・復路の L（5〜7）・像。L の往路の 2 回目に見立てた範囲 8〜10
        let mut rows: Vec<Row> = (0..12).map(|_| row("STANDARD")).collect();
        rows[9] = row("COORDBRK");
        let spans = vec![Some([1, 3])];
        let ids = vec!["L".to_string()];
        let back = vec![("L".to_string(), vec![[5, 7]])];
        let none: Vec<(String, Vec<[usize; 2]>)> = Vec::new();
        let at = |j| pass_row(&rows, &spans, &ids, &none, &back, j);
        assert_eq!(at(2), None, "範囲の中");
        assert_eq!(at(4), None, "鏡");
        assert_eq!(
            at(5),
            Some(PassRow {
                own: 3,
                prev: Some(2),
                back: true
            })
        );
        let first = at(7).unwrap();
        assert_eq!(
            first,
            PassRow {
                own: 1,
                prev: None,
                back: true
            }
        );
        assert_eq!(first.row_for("r"), Some(1));
        assert_eq!(first.row_for("t"), None);
        assert_eq!(at(6).unwrap().row_for("material"), Some(1));
        let fwd = vec![("L".to_string(), vec![[1, 3], [8, 10]])];
        let at = |j| pass_row(&rows, &spans, &ids, &fwd, &none, j);
        assert_eq!(at(9), None, "座標ブレーク");
        assert_eq!(
            at(10),
            Some(PassRow {
                own: 2,
                prev: None,
                back: false
            })
        );
        assert_eq!(at(10).unwrap().row_for("t"), Some(2));
    }

    /// ソルブ・ピックアップで決まる値は変えない（A0 の面 7 の厚さ）
    #[test]
    fn solved_values_are_refused() {
        let r = Row {
            solves: vec![("t".into(), "marginal_height 0".into())],
            pickups: vec![("k".into(), "from 1".into())],
            ..Default::default()
        };
        assert!(solved_reason(&r, "t").is_some());
        assert!(solved_reason(&r, "k").is_some());
        assert!(solved_reason(&r, "r").is_none());
        assert!(solved_reason(&r, "comment").is_none());
    }
}
