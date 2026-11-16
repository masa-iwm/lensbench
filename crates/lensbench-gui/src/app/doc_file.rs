//! 組合せの保存の形（文書モデル段階 4、GPN-01）: ファイルはブロック本体が元。`gui` の欄には見る状態
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

/// 前の版のプロジェクトの今の形（gui.combo が無ければ None）。パネルの設定など gui のほかの鍵は残す
#[cfg(any(test, not(target_arch = "wasm32")))]
fn converted(old: &Value) -> Result<Option<Value>, String> {
    let Some(cv) = old.pointer("/gui/combo") else {
        return Ok(None);
    };
    let mut c: Combo = serde_json::from_value(cv.clone()).map_err(|e| format!("gui.combo: {e}"))?;
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
            p.ap = ap;
            match ap {
                ApShape::Circle | ApShape::Obscuration => {
                    (p.semi, p.ap_min) = (3.0 + i as f64, 1.0)
                }
                ApShape::Rect | ApShape::Ellipse => p.ap_half = [2.0 + i as f64, 1.5],
                ApShape::Spider => (p.semi, p.spider_width, p.spider_arms) = (6.0, 0.4, 3),
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
}
