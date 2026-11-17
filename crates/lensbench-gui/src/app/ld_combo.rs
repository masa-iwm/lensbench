//! 組合せの系のレンズデータの表: 変えられないセルの理由と、面の上書きのあるセル（印と個別に外す、文書モデル）

use super::*;

pub(super) fn read_only(
    is_combo: bool,
    rows: &[lens_data::Row],
    spans: &[Option<[usize; 2]>],
    picks: &[&combo::Pick],
    lead: bool,
    j: usize,
    col: lens_data::Col,
) -> Option<String> {
    use lens_data::Col;
    if !is_combo {
        return None;
    }
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
        Col::Conic => &["k"],
        Col::Par(_) => &["asph", "cx", "focal", "jones", "cb", "cb_order"],
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
