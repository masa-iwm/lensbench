//! 組合せの合流・グローバル座標に置く BS・光束の折り返しの並び、系の設定の硝材置換テンプレート

use super::*;
use crate::combo::beams;

/// 見本 P20（マッハツェンダー）: 反射の腕の端に BS2 の口 D へ入る合流、BS2 はグローバル座標に置いてある
const P20: &str = "examples/projects/P20_mach_zehnder.json";

impl Drive {
    /// 組合せの図で、今の経路の要素 id をポインタで押して選ぶ
    fn select_element(&mut self, id: &str) {
        let ids = self.app().c.doc.combo.ids();
        let k = ids
            .iter()
            .position(|x| x == id)
            .unwrap_or_else(|| panic!("[{}] {id} is not on the path: {ids:?}", self.flow));
        let p = self
            .app()
            .c
            .p
            .combo_view
            .mid(k)
            .unwrap_or_else(|| panic!("[{}] {id} is not drawn in the figure", self.flow));
        self.click_at(p);
        assert_eq!(
            self.app().c.p.combo_view.sel(),
            Some(k),
            "[{}] {id} was not selected",
            self.flow
        );
    }

    /// 文言 anchor の部品（行の札）と同じ高さで一番近い label の部品を押す（行ごとに同じ名前のボタンがあるとき）
    fn press_in_row(&mut self, label: &str, anchor: &str) {
        let a = self.find(anchor, None);
        let r = self.target(
            &format!("{label:?} in the row of {anchor:?}"),
            named(label, None),
            |v| {
                (0..v.len())
                    .filter(|&i| v[i].center().x > a.left())
                    .min_by(|&x, &y| {
                        let d = |i: usize| {
                            (v[i].center().y - a.center().y).abs() * 100.0
                                + (v[i].center().x - a.center().x).abs()
                        };
                        d(x).total_cmp(&d(y))
                    })
            },
        );
        self.click_at(r.center());
    }

    /// 組合せの場所 loc の要素の `extra`
    fn extra_at(&self, loc: &[usize]) -> serde_json::Value {
        self.app()
            .c
            .doc
            .combo
            .get(loc)
            .expect("element")
            .extra
            .clone()
    }
}

/// 合流: 経路を RT にして図の合流を押す → 入る BS と口の一覧で変える → 合流の要素に入る。元の値に戻すと元の系
#[test]
fn flow_combo_join() {
    let mut d = Drive::new("join", open(P20));
    let before = d.app().c.doc.combo.project();
    d.choose("Path", 0, "RT");
    assert_eq!(
        d.app().c.doc.combo.path,
        "RT",
        "[join] the path was not set"
    );
    d.select_element("B1");
    let loc = [0, 1, 1];
    d.choose("Port", 0, "A");
    assert_eq!(d.extra_at(&loc)["port"], "a", "[join] {}", d.extra_at(&loc));
    d.choose("Join BS", 0, "BS1");
    assert_eq!(
        d.extra_at(&loc)["splitter"],
        "BS1",
        "[join] {}",
        d.extra_at(&loc)
    );
    d.expect("Port", "the fields went away after choosing in a list");
    d.choose("Join BS", 0, "BS2");
    d.choose("Port", 0, "D");
    d.choose("Path", 0, "Default");
    assert_eq!(d.app().c.doc.combo.project(), before, "[join] not back");
}

/// 合流を足す: BS1 の反射の腕の合流を ✖ で外し、腕の「＋」のメニューの「Join a BS」で足し直す
#[test]
fn flow_combo_join_add() {
    let mut d = Drive::new("join add", open(P20));
    d.select_element("BS1");
    let arm = |d: &Drive| {
        d.app().c.doc.combo.get(&[0]).expect("BS1").ports[1]
            .picks
            .clone()
    };
    assert!(
        arm(&d).last().is_some_and(|p| p.is_join()),
        "[join add] {:?}",
        arm(&d).len()
    );
    d.press("Join BS2 port D ✖");
    assert!(
        !arm(&d).iter().any(|p| p.is_join()),
        "[join add] the join was not removed"
    );
    d.press_in_row("＋", "Beyond port C (reflected)");
    d.press("Join a BS");
    let a = arm(&d);
    assert!(
        a.last()
            .is_some_and(|p| p.is_join() && p.extra["splitter"] == "BS1"),
        "[join add] the join was not added: {:?}",
        a.iter().map(|p| p.label()).collect::<Vec<_>>()
    );
}

/// グローバル座標に置く: 図の BS2 を押す → 切ると `at` が消える → 入れ直すと今の置き方の `at` → 原点 z を打つ
#[test]
fn flow_combo_bs_global() {
    let mut d = Drive::new("bs global", open(P20));
    d.select_element("BS2");
    let loc = [0, 0, 1];
    let o = |d: &Drive| -> Option<Vec<f64>> {
        d.extra_at(&loc)["at"]["o"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
    };
    let was = o(&d).expect("BS2 is placed in global coordinates in the sample");
    d.press("Place in global coordinates");
    assert_eq!(o(&d), None, "[bs global] at was not removed");
    d.press("Place in global coordinates");
    d.settle();
    let now = o(&d).expect("[bs global] at was not written again");
    assert!(
        now.iter().zip(&was).all(|(a, b)| (a - b).abs() < 1e-6),
        "[bs global] {now:?} != {was:?}"
    );
    d.type_into("Origin z", 0, "90");
    assert_eq!(
        o(&d).map(|v| v[2]),
        Some(90.0),
        "[bs global] {}",
        d.extra_at(&loc)
    );
}

/// 光束の折り返し: 光束を足す → 折り返す玉を選ぶ → 「+」で次の折り返し → 2 つ目の玉を選ぶ
#[test]
fn flow_combo_beam_turns() {
    let mut d = Drive::new("beam turns", open("examples/projects/P18_swap_two.json"));
    let lenses = beams::lens_ids(&d.app().c.doc.combo);
    assert!(lenses.len() >= 2, "{lenses:?}");
    d.press("Beams");
    d.press("+ Add a beam");
    d.choose("Turn back at", 0, &lenses[1]);
    let turn = |d: &Drive| {
        beams::beams_of(&d.app().c.doc.combo).expect("beams")[0]
            .turn
            .clone()
    };
    assert_eq!(turn(&d).len(), 1, "[beam turns] the turn was not set");
    d.press_in_row("+", "Turn back at");
    assert_eq!(
        turn(&d).len(),
        2,
        "[beam turns] the second turn was not added"
    );
    d.choose("Turn 2 at", 0, &lenses[0]);
    let t = turn(&d);
    assert_eq!(
        (t[0].block.as_str(), t[1].block.as_str()),
        (lenses[1].as_str(), lenses[0].as_str()),
        "[beam turns] {t:?}"
    );
    d.expect("+ Add a beam", "the window closed");
}

/// 硝材置換のテンプレート: 系の設定で見出しを開く → 使う → 相対価格の上限を打つ → 系の設定に入る
#[test]
fn flow_glass_template() {
    let mut d = Drive::new(
        "glass template",
        open("examples/projects/P6_states_focus_group.json"),
    );
    d.menu(&["Panels", "System settings"]);
    d.press("Glass substitution template");
    d.press("Use the template");
    d.type_into("Max", 0, "2.5");
    let g = d.app().c.doc.combo.extra.glass_template;
    let g = g.expect("[glass template] not set");
    assert!(
        g.enabled && g.check[0] && g.max[0] == 2.5,
        "[glass template] {g:?}"
    );
    d.expect(
        "Glass substitution template (on)",
        "the heading does not show the template is on",
    );
}
