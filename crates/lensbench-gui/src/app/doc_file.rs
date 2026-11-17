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

/// 前の版の保存（`gui.combo` に組合せの写し）を今の形に直す、一度きりの変換（`lensbench-gui --convert-combo
/// ファイル…`、11/16 masa）。ロック・今の状態・カタログの鏡などの品名も gui.view へ移す。元は `.bak` に残す。
/// 手元のファイルを直し終えたら、次の版でこの変換ごと消す（過去互換は残さない）
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn convert_old(files: &[String]) -> std::process::ExitCode {
    let mut ok = true;
    for f in files {
        match convert_file(std::path::Path::new(f)) {
            Ok(true) => println!("{f}: converted ({f}.bak keeps the original)"),
            Ok(false) => println!("{f}: already in the current form"),
            Err(e) => {
                eprintln!("{f}: {e}");
                ok = false;
            }
        }
    }
    if ok {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

/// ファイル 1 つを直す（直したら true、gui.combo が無ければ false）
#[cfg(not(target_arch = "wasm32"))]
fn convert_file(f: &std::path::Path) -> Result<bool, String> {
    let txt = lensbench_core::vfs::read_to_string(f).map_err(|e| e.to_string())?;
    let old: Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
    let Some(new) = converted(&old)? else {
        return Ok(false);
    };
    let bak = std::path::PathBuf::from(format!("{}.bak", f.display()));
    lensbench_core::vfs::write(&bak, &txt).map_err(|e| format!("{}: {e}", bak.display()))?;
    let out = serde_json::to_string_pretty(&new).map_err(|e| e.to_string())? + "\n";
    lensbench_core::vfs::write(f, out).map_err(|e| e.to_string())?;
    Ok(true)
}

/// 前の版の BS・開口の欄（要素に平たく持っていた form・ratio・ap・ap_half など）を今の
/// [`crate::combo::Bs`]（要素の bs）・[`crate::combo::ApData`]（要素の ap）へ
#[cfg(any(test, not(target_arch = "wasm32")))]
fn nest_kind_fields(picks: &mut Value) {
    const BS: [&str; 10] = [
        "form",
        "ratio",
        "bs_flip",
        "wedge",
        "split_back",
        "bs_angle",
        "split",
        "pbs",
        "ar",
        "faces",
    ];
    const AP: [(&str, &str); 5] = [
        ("ap", "shape"),
        ("ap_half", "half"),
        ("ap_min", "min"),
        ("spider_width", "spider_width"),
        ("spider_arms", "spider_arms"),
    ];
    for p in picks.as_array_mut().into_iter().flatten() {
        let Some(o) = p.as_object_mut() else { continue };
        let kind = o
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let bs: serde_json::Map<String, Value> = BS
            .iter()
            .filter_map(|k| o.remove(*k).map(|v| (k.to_string(), v)))
            .collect();
        if kind == "splitter" && !o.contains_key("bs") {
            let mut bs = bs;
            // 前の版の BS の一辺・厚さと硝材は、玉と同じ t・glass
            for (k, n) in [("t", "size"), ("glass", "glass")] {
                if let Some(v) = o.remove(k) {
                    bs.insert(n.into(), v);
                }
            }
            o.insert("bs".into(), Value::Object(bs));
        }
        // 鏡の曲率半径（前の版は玉と同じ r1）・コーニック・往路を戻る
        if kind == "mirror" && !o.contains_key("mirror") {
            let m: serde_json::Map<String, Value> =
                [("r1", "radius"), ("conic", "conic"), ("back", "back")]
                    .iter()
                    .filter_map(|(k, n)| o.remove(*k).map(|v| (n.to_string(), v)))
                    .collect();
            o.insert("mirror".into(), Value::Object(m));
        }
        // 自作の玉・平板の曲率半径・中心厚・硝材
        if matches!(kind.as_str(), "lens" | "plate") && !o.contains_key("custom") {
            let c: serde_json::Map<String, Value> = ["r1", "r2", "t", "glass", "semi_auto"]
                .iter()
                .filter_map(|k| o.remove(*k).map(|v| (k.to_string(), v)))
                .collect();
            o.insert("custom".into(), Value::Object(c));
        }
        // .zmx の玉・取り込みの番号・面の範囲・群
        if matches!(kind.as_str(), "zmx" | "zmx_range") && !o.contains_key("zmx") {
            let z: serde_json::Map<String, Value> = ["lens", "config", "range", "zgroups"]
                .iter()
                .filter_map(|k| o.remove(*k).map(|v| (k.to_string(), v)))
                .collect();
            o.insert("zmx".into(), Value::Object(z));
        }
        // 今の形の ap は object（前の版は形の名前の文字列）
        if !o.get("ap").is_some_and(Value::is_object) {
            let ap: serde_json::Map<String, Value> = AP
                .iter()
                .filter_map(|(k, n)| o.remove(*k).map(|v| (n.to_string(), v)))
                .collect();
            if kind == "aperture" {
                o.insert("ap".into(), Value::Object(ap));
            }
        }
        // 種類の欄は data の下に 1 つ（前の版は要素に並べていた）
        if !o.contains_key("data") {
            if let Some((k, v)) = ["custom", "mirror", "bs", "ap", "zmx"]
                .iter()
                .find_map(|k| o.remove(*k).map(|v| (k.to_string(), v)))
            {
                o.insert("data".into(), json!({ k: v }));
            }
        }
        for b in o
            .get_mut("ports")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            nest_kind_fields(&mut b["picks"]);
        }
    }
}

/// 前の版のプロジェクトの今の形（gui.combo が無ければ None）。パネルの設定など gui のほかの鍵は残す
#[cfg(any(test, not(target_arch = "wasm32")))]
fn converted(old: &Value) -> Result<Option<Value>, String> {
    let Some(cv) = old.pointer("/gui/combo") else {
        return Ok(None);
    };
    let mut cv = cv.clone();
    nest_kind_fields(&mut cv["picks"]);
    let mut c: Combo = serde_json::from_value(cv).map_err(|e| format!("gui.combo: {e}"))?;
    // 前の開き方と同じく、面の上書きを今の並びに合わせてから中身へ入れる
    c.drop_stale_edits();
    c.bake_edits();
    let mut v = project_file(&c);
    if let Some(g) = old["gui"].as_object() {
        for (k, x) in g.iter().filter(|(k, _)| *k != "combo") {
            v["gui"][k] = x.clone();
        }
    }
    Ok(Some(v))
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

    /// 前の版の gui.combo からロック・状態・品名・パネルの設定を移す
    #[test]
    fn converts_old_saves() {
        let mut c = Combo::default();
        let mut m = combo::Pick::new(combo::ElemKind::Mirror);
        (m.vendor, m.part, m.locked) = ("TH".into(), "CM254-100".into(), true);
        c.picks = vec![m, combo::Pick::new(combo::ElemKind::Plate)];
        c.states = vec!["near".into()];
        c.state = Some("near".into());
        let mut old = c.project();
        old["gui"] = json!({ "combo": serde_json::to_value(&c).unwrap(), "zoom": { "x": 1 } });
        let v = converted(&old).unwrap().unwrap();
        assert!(v.pointer("/gui/combo").is_none());
        assert_eq!(v["gui"]["zoom"], json!({ "x": 1 }));
        let mut d = Combo::from_project(&v, std::path::Path::new("")).unwrap();
        apply_view(&mut d, v.get("gui"));
        for p in &mut d.picks {
            p.id.clear();
            p.air_id.clear();
        }
        assert_eq!(d, c);
        assert_eq!(converted(&v).unwrap(), None);
    }

    /// 前の版の BS・開口・鏡・.zmx・自作の玉の欄は要素に平たく書いてあった（form・ratio・ap・r1・conic・range など）。
    /// 今の bs・ap・mirror・zmx・custom へ移して読む
    #[test]
    fn converts_flat_kind_fields() {
        let mut c = Combo::default();
        let mut b = combo::Pick::new(combo::ElemKind::Splitter);
        b.bs_mut().ratio = 0.3;
        b.bs_mut().form = combo::BsForm::Plate;
        let mut a = combo::Pick::new(combo::ElemKind::Aperture);
        a.ap_mut().shape = combo::ApShape::Rect;
        a.ap_mut().half = [2.0, 1.0];
        b.bs_mut().size = 3.0;
        b.bs_mut().glass = "N-SF11".into();
        let mut m = combo::Pick::new(combo::ElemKind::Mirror);
        m.mirror_mut().radius = -100.0;
        m.mirror_mut().conic = -1.0;
        m.mirror_mut().back = true;
        let mut z = combo::Pick::new(combo::ElemKind::ZmxRange);
        z.path = "a.zmx".into();
        z.zmx_mut().range = [2, 5];
        z.zmx_mut().config = 2;
        let mut l = combo::Pick::new(combo::ElemKind::Lens);
        l.custom_mut().r2 = -80.0;
        l.custom_mut().glass = "F2".into();
        c.picks = vec![b, combo::Pick::new(combo::ElemKind::Plate), a, m, z, l];
        let mut cv = serde_json::to_value(&c).unwrap();
        for p in cv["picks"].as_array_mut().unwrap() {
            let o = p.as_object_mut().unwrap();
            if let Some(d) = o.remove("data") {
                for (k, v) in d.as_object().unwrap() {
                    o.insert(k.clone(), v.clone());
                }
            }
            // 前の版はどの要素も全部の欄を書いた。BS の一辺・厚さと硝材は玉と同じ t・glass、鏡の曲率半径は r1
            let bs = o
                .remove("bs")
                .unwrap_or_else(|| serde_json::to_value(combo::Bs::default()).unwrap());
            let splitter = o["kind"] == "splitter";
            for (k, v) in bs.as_object().unwrap() {
                match k.as_str() {
                    "size" if splitter => o.insert("t".into(), v.clone()),
                    "glass" if splitter => o.insert("glass".into(), v.clone()),
                    "size" | "glass" => None,
                    _ => o.insert(k.clone(), v.clone()),
                };
            }
            let mirror: &[(&str, &str)] = &[("radius", "r1"), ("conic", "conic"), ("back", "back")];
            let zmx: &[(&str, &str)] = &[
                ("lens", "lens"),
                ("config", "config"),
                ("range", "range"),
                ("zgroups", "zgroups"),
            ];
            let custom: &[(&str, &str)] = &[
                ("r1", "r1"),
                ("r2", "r2"),
                ("t", "t"),
                ("glass", "glass"),
                ("semi_auto", "semi_auto"),
            ];
            for (key, names) in [("mirror", mirror), ("zmx", zmx), ("custom", custom)] {
                if let Some(m) = o.remove(key) {
                    for (n, k) in names {
                        o.insert(k.to_string(), m[*n].clone());
                    }
                }
            }
            let ap = o
                .remove("ap")
                .unwrap_or_else(|| serde_json::to_value(combo::ApData::default()).unwrap());
            for (k, n) in [("ap", "shape"), ("ap_half", "half"), ("ap_min", "min")] {
                o.insert(k.into(), ap[n].clone());
            }
        }
        let mut old = c.project();
        old["gui"] = json!({ "combo": cv });
        let v = converted(&old).unwrap().unwrap();
        let d = Combo::from_project(&v, std::path::Path::new("")).unwrap();
        assert_eq!(d.picks[0].bs().ratio, 0.3);
        assert_eq!(d.picks[0].bs().form, combo::BsForm::Plate);
        assert!(matches!(
            d.picks[1].data.as_deref(),
            Some(combo::KindData::Custom(_))
        ));
        assert_eq!(d.picks[2].ap().shape, combo::ApShape::Rect);
        assert_eq!(d.picks[2].ap().half, [2.0, 1.0]);
        assert_eq!(
            (d.picks[0].bs().size, d.picks[0].bs().glass.as_str()),
            (3.0, "N-SF11")
        );
        assert_eq!(*d.picks[3].mirror(), *c.picks[3].mirror());
        assert_eq!(
            (d.picks[4].zmx().range, d.picks[4].zmx().config),
            ([2, 5], 2)
        );
    }
}
