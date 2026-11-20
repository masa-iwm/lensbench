//! 組合せの保存の形（文書モデル段階 4）: ファイルはブロック本体が元。`gui` の欄には見る状態
//! （`gui.view`: ロックした要素・今の状態・ブロックに書かないカタログの品名）とパネルの設定だけを残し、開くときは
//! ブロックから組合せを作る

use super::*;

/// 組合せを書くプロジェクト（ブロックと `gui.view`）
pub(super) fn project_file(c: &Combo) -> Value {
    let mut v = c.project();
    let mut view = Map::new();
    let (mut locked, mut place, mut parts) = (Vec::new(), Vec::new(), Map::new());
    for (loc, id) in c.ids_all() {
        let Some(p) = c.get(&loc) else { continue };
        // カタログの鏡・平板などは系のブロックに品名を書かない（カタログの玉は source に書く）
        if p.kind != combo::ElemKind::Catalog && !p.part.is_empty() {
            parts.insert(id.clone(), json!([p.vendor, p.part]));
        }
        if p.locked {
            locked.push(Value::from(id.clone()));
        }
        if p.lock_place {
            place.push(Value::from(id));
        }
    }
    if !locked.is_empty() {
        view.insert("locked".into(), locked.into());
    }
    if !place.is_empty() {
        view.insert("lock_place".into(), place.into());
    }
    if !parts.is_empty() {
        view.insert("parts".into(), parts.into());
    }
    if let Some(s) = &c.state {
        view.insert("state".into(), s.clone().into());
    }
    if let Some(m) = crate::state_table::picks_json(c) {
        view.insert("mc_picks".into(), m);
    }
    if !c.mc_comments.is_empty() {
        view.insert("mc_comments".into(), json!(c.mc_comments));
    }
    let order = crate::state_table::order_json(c);
    if !order.is_empty() {
        view.insert("mc_order".into(), json!(order));
    }
    if !view.is_empty() {
        v["gui"] = json!({ "view": view });
    }
    v
}

/// 開いたプロジェクトの `gui.view` を、ブロックから作った組合せに戻す
pub(super) fn apply_view(c: &mut Combo, gui: Option<&Value>) {
    let Some(view) = gui.and_then(|g| g.get("view")) else {
        return;
    };
    let has = |k: &str, id: &str| {
        view[k]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(id)))
    };
    crate::state_table::picks_from_json(c, view.get("mc_picks"));
    if let Some(v) = view.get("mc_comments") {
        c.mc_comments = serde_json::from_value(v.clone()).unwrap_or_default();
    }
    if let Some(v) = view.get("mc_order") {
        let v: Vec<(Option<String>, String)> =
            serde_json::from_value(v.clone()).unwrap_or_default();
        crate::state_table::order_from_json(c, &v);
    }
    for (loc, id) in c.ids_all() {
        let (l, pl) = (has("locked", &id), has("lock_place", &id));
        if let Some(p) = c.get_mut(&loc) {
            p.locked = l;
            p.lock_place = pl;
            if let [Value::String(v), Value::String(n)] =
                view["parts"][&id].as_array().map_or(&[][..], |a| &a[..])
            {
                if p.part.is_empty() {
                    (p.vendor, p.part) = (v.clone(), n.clone());
                }
            }
        }
    }
    c.state = view["state"]
        .as_str()
        .filter(|s| c.states.iter().any(|x| x == s))
        .map(str::to_string);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 保存の形からブロックで組合せに戻すと、見る状態も戻る
    #[test]
    fn view_round_trip() {
        let mut c = Combo::default();
        let mut a = combo::Pick::new(combo::ElemKind::Plate);
        a.locked = true;
        let mut b = combo::Pick::new(combo::ElemKind::Mirror);
        b.lock_place = true;
        (b.vendor, b.part) = ("TH".into(), "CM254-100".into());
        c.picks = vec![a, b];
        c.states = vec!["near".into()];
        c.state = Some("near".into());
        c.mc_rank = [("stop".to_string(), 0), ("comment:0".to_string(), 1)].into();
        c.picks[1].mc_rank = [("air".to_string(), 2)].into();
        let v = project_file(&c);
        assert!(v.pointer("/gui/combo").is_none());
        // GUI が書くプロジェクトファイルにも形式の印と版が付く
        assert_eq!(v["format"], lensbench_project::PROJECT_FORMAT);
        assert_eq!(
            v["format_version"],
            lensbench_project::PROJECT_FORMAT_VERSION
        );
        let mut d = Combo::from_project(&v, std::path::Path::new("")).unwrap();
        apply_view(&mut d, v.get("gui"));
        assert_eq!(
            d.picks
                .iter()
                .map(|p| (p.locked, p.lock_place))
                .collect::<Vec<_>>(),
            [(true, false), (false, true)]
        );
        assert_eq!(
            (d.picks[1].vendor.as_str(), d.picks[1].part.as_str()),
            ("TH", "CM254-100")
        );
        assert_eq!(d.state.as_deref(), Some("near"));
        // 多重構成の表で入れ替えた行の並びも戻る
        assert_eq!(d.mc_rank, c.mc_rank);
        assert_eq!(d.picks[1].mc_rank, c.picks[1].mc_rank);
    }

    /// 要素の種類ごとに、保存の形に書いて開き直すと元の組合せに戻る（id は付く）
    #[test]
    fn each_kind_round_trips() {
        use combo::{ApShape, ElemKind, Pick};
        let mut picks = Vec::new();
        for k in [
            ElemKind::Lens,
            ElemKind::Plate,
            ElemKind::Mirror,
            ElemKind::Stop,
            ElemKind::Surfaces,
        ] {
            picks.push(Pick::new(k));
        }
        for (i, ap) in [
            ApShape::Circle,
            ApShape::Rect,
            ApShape::Ellipse,
            ApShape::Obscuration,
            ApShape::Spider,
        ]
        .into_iter()
        .enumerate()
        {
            // 形ごとに使う欄だけ変える（ほかの形の欄はブロックに書かない）
            let mut p = Pick::new(ElemKind::Aperture);
            p.ap_mut().shape = ap;
            match ap {
                ApShape::Circle | ApShape::Obscuration => {
                    (p.semi, p.ap_mut().min) = (3.0 + i as f64, 1.0)
                }
                ApShape::Rect | ApShape::Ellipse => p.ap_mut().half = [2.0 + i as f64, 1.5],
                ApShape::Spider => {
                    (p.semi, p.ap_mut().spider_width, p.ap_mut().spider_arms) = (6.0, 0.4, 3)
                }
            }
            p.decenter = [0.5, -0.25];
            p.air = 2.0;
            picks.push(p);
        }
        // OFF の開口（詰める・空気で残す）
        for off_air in [false, true] {
            let mut p = Pick::new(ElemKind::Aperture);
            (p.on, p.off_air, p.decenter) = (false, off_air, [0.3, 0.0]);
            picks.push(p);
        }
        let mut cat = Pick::new(ElemKind::Mirror);
        (cat.vendor, cat.part) = ("TH".into(), "CM254-100".into());
        cat.locked = true;
        picks.push(cat);
        let c = Combo {
            picks,
            ..Default::default()
        };
        let v = project_file(&c);
        let mut d = Combo::from_project(&v, std::path::Path::new("")).unwrap();
        apply_view(&mut d, v.get("gui"));
        for p in &mut d.picks {
            p.id.clear();
            p.air_id.clear();
        }
        for (k, (a, b)) in c.picks.iter().zip(&d.picks).enumerate() {
            let (a, b) = (
                serde_json::to_value(a).unwrap(),
                serde_json::to_value(b).unwrap(),
            );
            assert!(a == b, "{k}: {}", crate::combo_from::first_diff(&a, &b, ""));
        }
        assert_eq!(c, d);
    }
}
