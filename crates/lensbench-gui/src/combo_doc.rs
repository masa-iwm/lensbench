//! 組合せ（GUI が持つ文書）と、カタログから足す位置。カタログのパネルは一覧だけを持ち、
//! 組合せは [`crate::app`] の Core が持つ

use crate::catalog::{pick_from, LensRow, RowKind};
use crate::combo::Combo;
use crate::i18n::{tr, trf};

/// 組合せと、カタログから足すときの位置
#[derive(Default)]
pub(crate) struct ComboDoc {
    pub combo: Combo,
    /// 足す玉をこの要素の直後へ（組合せの図で選んでいる要素。無ければ後ろ）
    pub insert_after: Option<usize>,
    /// 最後に足した要素（経路の番号。呼び手が受け取る）
    pub added: Option<usize>,
    /// 最後に玉を足した: 系ができたら像面を近軸の焦点へ動かす
    pub refocus: bool,
    /// 組合せの版。[`ComboDoc::sync`] が前の画面から変わったのを見たら 1 つ進める（写しと同じかは版で見て、
    /// 毎フレーム組合せを丸ごと比べない）
    pub rev: u64,
    /// 版を進めたときの組合せ
    seen: Option<Combo>,
}

/// 組合せの写し（最後に系を作った・保存した組合せ）と、今の組合せと比べた版とその答え。版が同じ間は比べ直さない
#[derive(Default)]
pub(crate) struct Snap {
    combo: Option<Combo>,
    /// 今の組合せと比べた版と、そのときの答え
    seen_at: std::cell::Cell<Option<(u64, bool)>>,
}

impl Snap {
    pub(crate) fn new(combo: Option<Combo>) -> Snap {
        Snap {
            combo,
            seen_at: Default::default(),
        }
    }

    pub(crate) fn is_none(&self) -> bool {
        self.combo.is_none()
    }

    /// doc の今の組合せと同じか。比べた版のままなら前の答え（同じ・違う）を返す（版は画面ごとの [`ComboDoc::sync`] で進むので、
    /// 同じ画面の中で組合せを変えた直後は、次の sync まで前の答えのことがある）
    pub(crate) fn same(&self, doc: &ComboDoc) -> bool {
        if let Some((rev, same)) = self.seen_at.get() {
            if rev == doc.rev {
                return same;
            }
        }
        let same = self.combo.as_ref() == Some(&doc.combo);
        self.seen_at.set(Some((doc.rev, same)));
        same
    }
}

impl ComboDoc {
    pub(crate) fn new(combo: Combo) -> Self {
        Self {
            combo,
            ..Default::default()
        }
    }

    /// 画面ごとに 1 回呼ぶ: 組合せが前の版と違えば版を進める（組合せを比べるのはここだけ）
    pub(crate) fn sync(&mut self) {
        if self.seen.as_ref() != Some(&self.combo) {
            self.seen = Some(self.combo.clone());
            self.rev += 1;
        }
    }

    /// 素子 `lens` に合わせて機構部品を足す: 組合せにその素子があれば、そこに置いて保持する
    pub(crate) fn add_mech_for(&mut self, mech: &LensRow, lens: &LensRow, lenses: &[LensRow]) {
        let chain = self.combo.chain_view().picks;
        let same = |p: &crate::combo::Pick| {
            p.kind == crate::combo::ElemKind::Catalog
                && p.vendor == lens.vendor
                && p.part == lens.part
        };
        // 同じ玉が 2 つ以上あれば、選んでいる方を優先
        let k = self
            .insert_after
            .filter(|&k| chain.get(k).is_some_and(|p| same(p)))
            .or_else(|| chain.iter().position(|p| same(p)));
        let keep = self.insert_after;
        if k.is_some() {
            self.insert_after = k;
        }
        self.add_row(mech, lenses);
        self.insert_after = keep;
    }

    /// カタログの行を足す（未完の品は呼び手が除く）。`lenses` は機構部品が保持する玉を探す一覧
    pub(crate) fn add_row(&mut self, r: &LensRow, lenses: &[LensRow]) {
        // 機構部品は要素の並びでなく「機構部品」へ（選んでいる要素に置く）
        if r.kind == RowKind::Mech {
            let ids = self.combo.ids();
            let at = self
                .insert_after
                .and_then(|k| ids.get(k).cloned())
                .or_else(|| ids.iter().find(|x| !x.is_empty()).cloned())
                .unwrap_or_default();
            // 名前は組合せの中で別々に（つなぐ相手を名前で選ぶので）
            let mut name = r.part.clone();
            let mut n = 2;
            while self.combo.mechanics.iter().any(|m| m.name == name) {
                name = format!("{} {n}", r.part);
                n += 1;
            }
            // 置く要素がカタログの玉で、その径が部品の保持径に入るなら、その玉を保持して座に合わせる
            let held = ids
                .iter()
                .position(|x| !at.is_empty() && *x == at)
                .and_then(|k| self.combo.el(k))
                .filter(|p| p.kind == crate::combo::ElemKind::Catalog)
                .and_then(|p| {
                    lenses.iter().find(|l| {
                        l.vendor == p.vendor && l.part == p.part && l.kind != RowKind::Mech
                    })
                })
                .is_some_and(|l| lensbench_catalog::spec::fits_row(&r.raw, l.diameter));
            self.combo.mechanics.push(crate::combo::MechPick {
                name,
                catalog: r.source.clone().unwrap_or_default(),
                item: r.part.clone(),
                holds: if held { vec![at.clone()] } else { Vec::new() },
                seat: held,
                at,
                ..Default::default()
            });
            return;
        }
        let at = self.insert_after.map_or(self.combo.len(), |k| k + 1);
        let n = self.combo.len();
        let at = if at >= n && n > 0 {
            // 最後に足す: 前の最後の要素の後（像面までだった空気）は小さな間隔にし、足した玉の後の空気は
            // 系ができたら近軸の焦点まで（2 枚目で像面が遠くに残って像が崩れない）
            let chain = self.combo.chain();
            if let Some(g) = chain.last().and_then(|s| s.after.clone()) {
                if matches!(g, crate::combo_tree::Gap::Air(_)) {
                    if let Some(v) = self.combo.gap(&g) {
                        *v = 5.0;
                    }
                }
            }
            self.refocus = true;
            self.combo.insert_chain_with(at, pick_from(r), false)
        } else {
            if n == 0 {
                self.refocus = true;
            }
            crate::combo::insert_pick(&mut self.combo, at, pick_from(r))
        };
        // 続けて足すと、足した玉の後ろへ
        self.insert_after = Some(at);
        self.added = Some(at);
    }
}

impl ComboDoc {
    /// 経路の k 番目の玉をカタログの玉の行 `r` に差し替える（id・前後の間隔・反転・群は残す）。返すのは状態の行に出す文
    pub(crate) fn replace_row(&mut self, k: usize, r: &LensRow) -> Result<String, String> {
        let old = self
            .combo
            .el(k)
            .filter(|p| p.is_lens_like())
            .ok_or_else(|| tr("Only a lens can be replaced with a catalog lens").to_string())?;
        let new = pick_from(r);
        if !new.is_lens_like() || r.kind != RowKind::Lens {
            return Err(tr("Only a catalog lens can replace a lens").into());
        }
        let label = old.label();
        let p = old.replaced_by(new);
        let msg = trf("Replaced {} with {}", &[&label, &p.label()]);
        if let Some(x) = self.combo.el_mut(k) {
            *x = p;
        }
        Ok(msg)
    }

    /// 機構部品 `name` の品をカタログの行 `r`（機構部品）に差し替える（置く所・保持・座・つなぎ・反転は残す）
    pub(crate) fn replace_mech(&mut self, name: &str, r: &LensRow) -> Result<String, String> {
        if r.kind != RowKind::Mech {
            return Err(tr("Only a mechanical part can replace a mechanical part").into());
        }
        let m = self
            .combo
            .mechanics
            .iter_mut()
            .find(|m| m.name == name)
            .ok_or_else(|| trf("No mechanical part {}", &[&name]))?;
        let msg = trf(
            "Replaced {} with {}",
            &[&format!("{name} ({})", m.item), &r.part],
        );
        m.catalog = r.source.clone().unwrap_or_default();
        m.item = r.part.clone();
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// カタログの玉を最後に足すと、前の最後の玉の後（像面までだった空気）は 5 mm になり、像面は系ができたら近軸の
    /// 焦点へ（refocus）。1 枚目の後の空気は焦点距離のまま残らない（2 枚目で像面が遠くに残らない）
    #[test]
    fn adding_at_the_end_refocuses() {
        let row = |part: &str, efl: f64| LensRow {
            vendor: "V".into(),
            part: part.into(),
            efl,
            diameter: 25.4,
            thickness: 3.0,
            pieces: 1,
            ..Default::default()
        };
        let mut d = ComboDoc::default();
        d.add_row(&row("A", 45.0), &[]);
        assert!(std::mem::take(&mut d.refocus));
        d.add_row(&row("B", 100.0), &[]);
        assert!(d.refocus);
        assert_eq!(d.combo.picks.len(), 2);
        assert_eq!(d.combo.picks[0].air, 5.0);
    }

    /// P19 の L1（自作の玉）をカタログの玉に、tube を別の長さのチューブに差し替える。id・後の空気・保持・座・置く所は
    /// 残り、組合せは手で書いたプロジェクト（L1 をカタログの品、tube の item を替えたもの）を読んだものと同じ
    #[test]
    fn replace_keeps_settings_and_matches_hand_written() {
        let _g = crate::serve_lock();
        crate::i18n::set(crate::i18n::Lang::En);
        let ex = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let dir = ex.join("projects");
        let text = std::fs::read_to_string(dir.join("P19_tube_cage.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut d = ComboDoc::new(Combo::from_project(&v, &dir).unwrap());
        // 長さ 50 のチューブを足したカタログ（試験ごとに別の名前）
        let tmp = std::env::temp_dir().join(format!("lensbench-replace-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut cat: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(ex.join("lenses/LBP/parts.json")).unwrap(),
        )
        .unwrap();
        let mut t50 = cat["items"]["TUBE-25-30"].clone();
        t50["mech"]["length"] = 50.into();
        t50["mech"]["bores"][0]["z1"] = 50.into();
        cat["items"]["TUBE-25-50"] = t50;
        let cat_path = tmp.join("parts.json");
        std::fs::write(&cat_path, cat.to_string()).unwrap();
        let cat_s = cat_path.display().to_string();

        let ids = d.combo.ids();
        let k = ids.iter().position(|x| x == "L1").unwrap();
        let air = d.combo.el(k).unwrap().air;
        let lens = LensRow {
            vendor: "LB".into(),
            part: "SL-254-50".into(),
            source: Some(cat_s.clone()),
            kind: RowKind::Lens,
            efl: 50.0,
            diameter: 25.4,
            thickness: 5.0,
            pieces: 1,
            ..Default::default()
        };
        let msg = d.replace_row(k, &lens).unwrap();
        assert!(
            msg.starts_with("Replaced ") && msg.ends_with("LB SL-254-50"),
            "{msg}"
        );
        let p = d.combo.el(k).unwrap();
        assert_eq!(
            (p.kind, p.id.as_str(), p.air, p.part.as_str()),
            (crate::combo::ElemKind::Catalog, "L1", air, "SL-254-50")
        );
        let tube = LensRow {
            part: "TUBE-25-50".into(),
            source: Some(cat_s.clone()),
            kind: RowKind::Mech,
            ..Default::default()
        };
        d.replace_mech("tube", &tube).unwrap();
        // 機構部品でない行・玉でない要素は差し替えない
        assert!(d.replace_mech("tube", &lens).is_err());
        assert!(d.replace_mech("nope", &tube).is_err());
        let stop = ids.iter().position(|x| x == "stop").unwrap();
        assert!(d.replace_row(stop, &lens).is_err());

        let mut hand = v.clone();
        let b = hand["blocks"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|b| b["id"] == "L1")
            .unwrap();
        *b = serde_json::json!({ "type": "user", "id": "L1", "catalog": cat_s, "item": "SL-254-50" });
        hand["mechanics"][0]["catalog"] = cat_s.clone().into();
        hand["mechanics"][0]["item"] = "TUBE-25-50".into();
        let want = Combo::from_project(&hand, &dir).unwrap();
        assert_eq!(d.combo.project(), want.project());
        let m = &d.combo.mechanics[0];
        assert_eq!(
            (m.at.as_str(), m.holds.clone()),
            ("stop", vec!["L1".to_string()])
        );
        // 差し替えた組合せで系ができる（保持する L1 はユーザーカタログの品になった）
        let f = tmp.join("p.json");
        std::fs::write(&f, d.combo.project().to_string()).unwrap();
        let gd = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/glass");
        let lay = lensbench_engine::preview_layout(
            &serde_json::json!({ "file": f.display().to_string(), "glass_dirs": [gd.display().to_string()] }),
            &serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(lay["mechs"].as_array().map(Vec::len), Some(2), "{lay}");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
