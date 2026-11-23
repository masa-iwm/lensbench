//! 組合せの群: 名前の一覧・欄の補完の候補と、レンズデータの表から群にする・群を解除する操作。
//! 群は要素ごとに外から内の名前の並び（[`Pick::group`]）で持ち、続く同じ道の要素を群のブロック（入れ子も）にする。
//! 群は BS の 1 つの腕（同じ並び）の中に閉じる

use crate::combo::{Combo, ElemKind, Pick};
use crate::combo_tree::Loc;
use crate::i18n::{tr, trf};

/// 欄の文字（外から内を `/` で区切る）を群の道に
pub(crate) fn group_path_of(text: &str) -> Vec<String> {
    text.split('/')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 要素の群の欄を読む: 名前の並び（外から内）か、前の版の名前 1 つの文字（空は群に入れない）
pub(crate) fn group_path<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum G {
        One(String),
        Path(Vec<String>),
    }
    let v = match <G as serde::Deserialize>::deserialize(d)? {
        G::One(s) => vec![s],
        G::Path(v) => v,
    };
    Ok(v.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

/// 群の名前に使えるか（空・`/` を含む名前は使えない）
fn name_ok(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err(tr("Enter a group name").into());
    }
    if name.contains('/') {
        return Err(tr("A group name cannot contain /").into());
    }
    Ok(())
}

impl Combo {
    /// 要素の群の名前（外の群から、出てくる順に 1 回ずつ）
    pub(crate) fn group_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for p in self.all().iter().filter_map(|l| self.get(l)) {
            for g in &p.group {
                if !names.contains(g) {
                    names.push(g.clone());
                }
            }
        }
        names
    }

    /// 要素 `me` の群の欄の補完の候補: ほかの要素の群の道（外の群だけのものも、`G1`・`G1/G2`）
    pub(crate) fn group_choices(&self, me: &[usize]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for l in self.all().iter().filter(|l| l.as_slice() != me) {
            let Some(p) = self.get(l) else { continue };
            for n in 1..=p.group.len() {
                let t = p.group[..n].join("/");
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
        out
    }

    /// 並びの中の要素 `locs`（道の順）を群 `name` にする。群は同じ並びの続いた要素だけ。外の群の中の一部を
    /// 群にする（入れ子）・群を含む範囲を群にする（外へ入れ子）のどちらもでき、群の一部だけにかかる範囲は断る
    pub(crate) fn make_group(&mut self, locs: &[Loc], name: &str) -> Result<(), String> {
        let name = name.trim();
        name_ok(name)?;
        let ids: Vec<String> = self.ids_all().into_iter().map(|x| x.1).collect();
        if ids.iter().any(|i| i == name) {
            return Err(trf("{} is the same as an element name", &[&name]));
        }
        if self.group_names().iter().any(|g| g == name) {
            return Err(trf("A group named {} already exists", &[&name]));
        }
        let (first, last) = match (locs.first(), locs.last()) {
            (Some(a), Some(b)) => (a.clone(), b.clone()),
            _ => return Err(tr("Select whole elements to make a group").into()),
        };
        let (a, prefix) = first.split_last().expect("loc");
        let (b, prefix_b) = last.split_last().expect("loc");
        let one_list = prefix == prefix_b
            && locs.len() == b + 1 - a
            && locs
                .iter()
                .enumerate()
                .all(|(i, l)| l.split_last() == Some((&(a + i), prefix)));
        if !one_list {
            return Err(tr(
                "A group must be consecutive elements in one path (not across a beam splitter)",
            )
            .into());
        }
        let list = self.list_at(prefix).ok_or("loc")?;
        let range = &list[*a..=*b];
        if range
            .iter()
            .any(|p| p.kind == ElemKind::Splitter && p.branched)
        {
            return Err(tr("A beam splitter with arms cannot be put in a group").into());
        }
        // 範囲の要素に共通の外の群（深さ d）。範囲の端で、その次の段の群が範囲の外へ続いていれば一部だけ
        let d = common_depth(range);
        let cut = |inside: &Pick, outside: Option<&Pick>| {
            outside.is_some_and(|o| {
                inside.group.len() > d && o.group.len() > d && o.group[..=d] == inside.group[..=d]
            })
        };
        let before = a.checked_sub(1).and_then(|i| list.get(i));
        let after = list.get(b + 1);
        if cut(&range[0], before) || cut(&range[range.len() - 1], after) {
            return Err(tr(
                "The range covers only part of a group (select the whole group, or only inside it)",
            )
            .into());
        }
        let list = self.list_at_mut(prefix).ok_or("loc")?;
        for p in &mut list[*a..=*b] {
            p.group.insert(d, name.to_string());
        }
        // 役割・移動の欄（パネルが描くときに足すと、元に戻すの段が分かれる）
        if !self.groups.iter().any(|g| g.name == name) {
            self.groups.push(crate::combo::GroupDef {
                name: name.to_string(),
                ..Default::default()
            });
        }
        Ok(())
    }

    /// 群 `name` を解除する（中の要素・内側の群は残す）。役割・移動も消す
    pub(crate) fn ungroup(&mut self, name: &str) -> Result<(), String> {
        if !self.group_names().iter().any(|g| g == name) {
            return Err(trf("{} is not a group of the combination", &[&name]));
        }
        for l in self.all() {
            if let Some(p) = self.get_mut(&l) {
                p.group.retain(|g| g != name);
            }
        }
        self.groups.retain(|g| g.name != name);
        Ok(())
    }

    /// 群 `name` の役割・移動（解除で消えるもの）の説明。無ければ None
    pub(crate) fn group_settings(&self, name: &str) -> Option<String> {
        let d = self.groups.iter().find(|g| g.name == name)?;
        let mut v: Vec<String> = Vec::new();
        if !d.role.is_empty() {
            v.push(trf("role {}", &[&d.role]));
        }
        if d.shift != 0.0 {
            v.push(trf("shift {} mm", &[&d.shift]));
        }
        if !d.state_shift.is_empty() {
            v.push(trf("shift in {} states", &[&d.state_shift.len()]));
        }
        (!v.is_empty()).then(|| v.join(", "))
    }
}

/// 要素の並びに共通の外の群の深さ
fn common_depth(ps: &[Pick]) -> usize {
    let first = &ps[0].group;
    (0..first.len())
        .take_while(|&i| ps.iter().all(|p| p.group.get(i) == Some(&first[i])))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combo::GroupDef;

    fn lenses(n: usize) -> Combo {
        Combo {
            picks: (0..n).map(|_| Pick::new(ElemKind::Lens)).collect(),
            ..Default::default()
        }
    }

    fn locs(r: std::ops::RangeInclusive<usize>) -> Vec<Loc> {
        r.map(|i| vec![i]).collect()
    }

    fn paths(c: &Combo) -> Vec<String> {
        c.picks.iter().map(|p| p.group.join("/")).collect()
    }

    #[test]
    fn make_nest_and_ungroup() {
        let mut c = lenses(4);
        c.make_group(&locs(1..=3), "G1").unwrap();
        assert_eq!(paths(&c), ["", "G1", "G1", "G1"]);
        // 内へ入れ子
        c.make_group(&locs(2..=3), "G2").unwrap();
        assert_eq!(paths(&c), ["", "G1", "G1/G2", "G1/G2"]);
        // 外へ入れ子（群を全部含む範囲）
        c.make_group(&locs(0..=3), "G0").unwrap();
        assert_eq!(paths(&c), ["G0", "G0/G1", "G0/G1/G2", "G0/G1/G2"]);
        // 群の一部だけにかかる範囲は断る
        assert!(c.make_group(&locs(0..=1), "X").is_err());
        assert!(c.make_group(&locs(1..=2), "X").is_err());
        // 同じ名前・要素の名前・空・/ は断る
        assert!(c.make_group(&locs(3..=3), "G1").is_err());
        let id = c.ids()[0].clone();
        assert!(c.make_group(&locs(3..=3), &id).is_err());
        assert!(c.make_group(&locs(3..=3), " ").is_err());
        assert!(c.make_group(&locs(3..=3), "a/b").is_err());
        // 続かない要素は断る
        assert!(c.make_group(&[vec![0], vec![2]], "X").is_err());
        // 解除: 中の要素・内側の群は残り、役割・移動も消える
        // 群にしたときに役割・移動の欄もできる
        let d = c
            .groups
            .iter_mut()
            .find(|g| g.name == "G1")
            .expect("G1 def");
        d.role = "zoom".into();
        assert!(c.group_settings("G1").is_some());
        c.ungroup("G1").unwrap();
        assert_eq!(paths(&c), ["G0", "G0", "G0/G2", "G0/G2"]);
        assert!(c.groups.iter().all(|g| g.name != "G1"));
        assert!(c.ungroup("G1").is_err());
        assert_eq!(c.group_names(), ["G0", "G2"]);
        assert_eq!(c.group_choices(&[2]), ["G0", "G0/G2"]);
    }

    #[test]
    fn path_text_and_old_files() {
        assert_eq!(group_path_of(" G1 / G2 /"), ["G1", "G2"]);
        assert!(group_path_of("").is_empty());
        // 前の版の名前 1 つの文字も読む
        let mut v = serde_json::to_value(Pick::new(ElemKind::Lens)).unwrap();
        v["group"] = serde_json::json!("G1");
        let p: Pick = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(p.group, ["G1"]);
        v["group"] = serde_json::json!("");
        let p: Pick = serde_json::from_value(v.clone()).unwrap();
        assert!(p.group.is_empty());
        v["group"] = serde_json::json!(["G1", "G2"]);
        let p: Pick = serde_json::from_value(v).unwrap();
        assert_eq!(p.group, ["G1", "G2"]);
    }

    /// 入れ子の群は入れ子の群のブロックに書かれ（最後の要素の後の空気は群の外）、開き直すと同じ道に戻る。
    /// 内の群の状態の移動もプロジェクトで効く
    #[test]
    fn nested_groups_round_trip_and_move() {
        let mut c = lenses(4);
        for (i, p) in c.picks.iter_mut().enumerate() {
            p.air = 5.0 + i as f64;
        }
        c.make_group(&locs(1..=3), "G1").unwrap();
        c.make_group(&locs(2..=3), "G2").unwrap();
        c.states.push("tele".into());
        *c.groups
            .iter_mut()
            .find(|g| g.name == "G2")
            .expect("G2 def") = GroupDef {
            name: "G2".into(),
            role: "focus".into(),
            state_shift: [("tele".to_string(), 1.5)].into(),
            ..Default::default()
        };
        let v = c.project();
        let b = v["blocks"].as_array().unwrap();
        let g1 = b.iter().find(|x| x["id"] == "G1").expect("G1");
        assert_eq!(g1["type"], "group");
        let inner = g1["blocks"].as_array().unwrap();
        // L2・空気・G2（L3・空気・L4）。G2 の後の空気は G1 の外
        assert_eq!(inner.len(), 3, "{inner:?}");
        assert_eq!(inner[2]["type"], "group");
        assert_eq!(inner[2]["id"], "G2");
        assert_eq!(inner[2]["role"], "focus");
        assert_eq!(inner[2]["blocks"].as_array().unwrap().len(), 3);
        assert_eq!(v["states"][0]["set"]["G2"]["shift"], 1.5);
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let back = Combo::from_project(&v, &root).unwrap();
        assert_eq!(paths(&back), paths(&c));
        assert_eq!(
            back.groups
                .iter()
                .find(|g| g.name == "G2")
                .map(|g| g.role.as_str()),
            Some("focus")
        );
        // 状態 tele で G2 が 1.5 動く（前の空気が増え、後の空気が減る）
        let pr = lensbench_project::Project::from_json(&v.to_string()).unwrap();
        let env = lensbench_project::Env {
            step_files: None,
            base: root.clone(),
            glass_dirs: vec![root.join("tests/glass")],
            catalog: None,
        };
        let sys = |state| {
            lensbench_project::expand(
                &pr,
                &env,
                &lensbench_project::ExpandOpts {
                    state,
                    ..Default::default()
                },
            )
            .unwrap()
            .system
        };
        let (a, t) = (sys(None), sys(Some("tele")));
        let d: Vec<f64> = a
            .surfaces
            .iter()
            .zip(&t.surfaces)
            .map(|(x, y)| y.t - x.t)
            .filter(|x| x.abs() > 1e-12)
            .collect();
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(
            (d[0] - 1.5).abs() < 1e-12 && (d[1] + 1.5).abs() < 1e-12,
            "{d:?}"
        );
    }

    /// 入れ子の群（G1 の中に G2）を名前で引く: 最適化の変数・ズーム軌跡の解くパラメータ（`G1.shift`・`G2.shift`）と
    /// 公差の群の公差（偏心・傾き）が内側・外側のどちらの群でも効き、役割は両方に付けて保存・読込で残る
    #[test]
    fn nested_groups_by_name_in_optimize_zoom_and_tolerance() {
        let mut c = lenses(4);
        for (i, p) in c.picks.iter_mut().enumerate() {
            p.air = 5.0 + i as f64;
        }
        c.make_group(&locs(1..=3), "G1").unwrap();
        c.make_group(&locs(2..=3), "G2").unwrap();
        c.states = vec!["wide".into(), "tele".into()];
        for (name, role) in [("G1", "zoom"), ("G2", "focus")] {
            c.groups
                .iter_mut()
                .find(|g| g.name == name)
                .expect("group def")
                .role = role.into();
        }
        let v = c.project();
        // 役割は入れ子の両方に付き、開き直しても残る
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let back = Combo::from_project(&v, &root).unwrap();
        let role = |n: &str| {
            back.groups
                .iter()
                .find(|g| g.name == n)
                .map(|g| g.role.clone())
        };
        assert_eq!(role("G1").as_deref(), Some("zoom"));
        assert_eq!(role("G2").as_deref(), Some("focus"));
        let pr = lensbench_project::Project::from_json(&v.to_string()).unwrap();
        let env = lensbench_project::Env {
            step_files: None,
            base: root.clone(),
            glass_dirs: vec![root.join("tests/glass")],
            catalog: None,
        };
        let sys = |state: Option<&str>| {
            lensbench_project::expand(
                &pr,
                &env,
                &lensbench_project::ExpandOpts {
                    state,
                    ..Default::default()
                },
            )
            .unwrap()
            .system
        };
        let s = sys(None);
        // 系の群は入れ子の範囲（G2 は G1 の中）
        let span = |n: &str| {
            let u = s.find_unit(n).unwrap_or_else(|| panic!("unit {n}"));
            s.units[u].spans.clone()
        };
        let (g1, g2) = (span("G1"), span("G2"));
        assert_eq!(g1.len(), 1);
        assert_eq!(g2.len(), 1);
        assert!(g1[0][0] < g2[0][0] && g2[0][1] <= g1[0][1], "{g1:?} {g2:?}");
        // 名前で引いた移動: 前の間隔（群の最初の面の前）に足し、後の間隔（群の最後の面）から引く
        let moved = |name: &str| -> Vec<(usize, f64)> {
            let p = s.param(name).unwrap();
            let t = s.with_params(&[(p, 1.0)]).unwrap();
            s.surfaces
                .iter()
                .zip(&t.surfaces)
                .enumerate()
                .map(|(j, (a, b))| (j, b.t - a.t))
                .filter(|x| x.1.abs() > 1e-12)
                .collect()
        };
        for (name, sp) in [("G1.shift", g1[0]), ("G2.shift", g2[0])] {
            let d = moved(name);
            assert_eq!(d.len(), 2, "{name} {d:?}");
            assert_eq!(d[0].0, sp[0] - 1, "{name} {d:?}");
            assert!((d[0].1 - 1.0).abs() < 1e-12, "{name} {d:?}");
            assert_eq!(d[1].0, sp[1], "{name} {d:?}");
            assert!((d[1].1 + 1.0).abs() < 1e-12, "{name} {d:?}");
        }
        // 最適化の変数・ズーム軌跡の解くパラメータとして読める
        let o: lensbench_design::dls_spec::OptimizeParams = lensbench_design::dls_spec::read(
            &serde_json::json!({"vars": ["G1.shift", "G2.shift=-2:2"], "merit": "none"}),
        )
        .unwrap();
        let (prob, _) = o.problem(vec![s.clone()]).unwrap();
        assert_eq!(prob.vars.len(), 2);
        let z: lensbench_design::dls_spec::ZoomParams = lensbench_design::dls_spec::read(
            &serde_json::json!({"vars": ["G1.shift", "G2.shift"]}),
        )
        .unwrap();
        let states = [sys(Some("wide")), sys(Some("tele"))];
        z.spec(&states, &["wide".into(), "tele".into()]).unwrap();
        // 公差の群の公差は内側・外側の群の名前で引け、群ごとの偏心・傾きになる
        let spec = lensbench_design::tolerance::spec_from_json(
            &serde_json::json!({
                "preset": "none",
                "groups": {"G1": {"decenter": 0.1}, "G2": {"tilt": 0.2}},
            }),
            vec![s.clone()],
            &Default::default(),
        )
        .unwrap();
        let labels: Vec<&str> = spec.tols.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(
            labels,
            ["G2 tilt x", "G2 tilt y", "G1 decenter x", "G1 decenter y"]
        );
        let unit_of = |label: &str| {
            let t = spec.tols.iter().find(|t| t.label == label).unwrap();
            t.params[0].unit_index().map(|u| s.units[u].name.clone())
        };
        assert_eq!(unit_of("G1 decenter x").as_deref(), Some("G1"));
        assert_eq!(unit_of("G2 tilt x").as_deref(), Some("G2"));
        // 外側の群の偏心は内側の群の面も一緒に動かす
        let p = s.param("G1.dx").unwrap();
        let t = s.with_params(&[(p, 0.1)]).unwrap();
        let x = |sys: &lensbench_core::System, j: usize| sys.frames().surf[j].expect("frame").o[0];
        let j = g2[0][0];
        assert!((x(&t, j) - x(&s, j) - 0.1).abs() < 1e-9);
    }

    /// 内側の群が外側の群の途中にある形（G1 = L2〜L5 の中に G2 = L3〜L4）: G2 の後の間隔は G1 の中に残り、
    /// `G2.shift` は G1 の中の 2 つの間隔だけを動かす（G1 の前後の間隔は変えない）。状態の移動も同じ
    #[test]
    fn nested_group_in_the_middle_moves_inside() {
        let mut c = lenses(5);
        for (i, p) in c.picks.iter_mut().enumerate() {
            p.air = 5.0 + i as f64;
        }
        c.make_group(&locs(1..=4), "G1").unwrap();
        c.make_group(&locs(2..=3), "G2").unwrap();
        assert_eq!(paths(&c), ["", "G1", "G1/G2", "G1/G2", "G1"]);
        c.states.push("tele".into());
        c.groups
            .iter_mut()
            .find(|g| g.name == "G2")
            .expect("G2 def")
            .state_shift = [("tele".to_string(), 1.5)].into();
        let v = c.project();
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let back = Combo::from_project(&v, &root).unwrap();
        assert_eq!(paths(&back), paths(&c));
        let pr = lensbench_project::Project::from_json(&v.to_string()).unwrap();
        let env = lensbench_project::Env {
            step_files: None,
            base: root.clone(),
            glass_dirs: vec![root.join("tests/glass")],
            catalog: None,
        };
        let sys = |state: Option<&str>| {
            lensbench_project::expand(
                &pr,
                &env,
                &lensbench_project::ExpandOpts {
                    state,
                    ..Default::default()
                },
            )
            .unwrap()
            .system
        };
        let s = sys(None);
        let span = |n: &str| {
            let u = s.find_unit(n).unwrap_or_else(|| panic!("unit {n}"));
            s.units[u].spans[0]
        };
        let (g1, g2) = (span("G1"), span("G2"));
        assert!(g1[0] < g2[0] && g2[1] < g1[1], "{g1:?} {g2:?}");
        let diff = |t: &lensbench_core::System| -> Vec<(usize, f64)> {
            s.surfaces
                .iter()
                .zip(&t.surfaces)
                .enumerate()
                .map(|(j, (a, b))| (j, b.t - a.t))
                .filter(|x| x.1.abs() > 1e-12)
                .collect()
        };
        let by_name = |name: &str| {
            let p = s.param(name).unwrap();
            diff(&s.with_params(&[(p, 1.0)]).unwrap())
        };
        for (name, sp, d) in [("G1.shift", g1, 1.0), ("G2.shift", g2, 1.0)] {
            let m = by_name(name);
            assert_eq!(m.len(), 2, "{name} {m:?}");
            assert_eq!((m[0].0, m[1].0), (sp[0] - 1, sp[1]), "{name} {m:?}");
            assert!(
                (m[0].1 - d).abs() < 1e-12 && (m[1].1 + d).abs() < 1e-12,
                "{name} {m:?}"
            );
        }
        // G2 の後の間隔（G2 の最後の面）は G1 の中
        assert!(g2[1] < g1[1]);
        let m = diff(&sys(Some("tele")));
        assert_eq!(m.len(), 2, "{m:?}");
        assert_eq!((m[0].0, m[1].0), (g2[0] - 1, g2[1]), "{m:?}");
        assert!(
            (m[0].1 - 1.5).abs() < 1e-12 && (m[1].1 + 1.5).abs() < 1e-12,
            "{m:?}"
        );
    }
}
