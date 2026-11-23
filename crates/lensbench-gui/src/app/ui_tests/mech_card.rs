//! 選んだ機構部品の札のケラレ（mech-check の結果）

use super::*;

/// P19 で部品を選ぶと mech-check を頼み、札に視野ごとの通る割合とその部品が最初に切る割合が出る。選びを外しても
/// 結果は残り、系を変えると選んでいる間に取り直す
#[test]
fn selected_part_card_shows_mech_vignetting() {
    let _g = serial();
    let r = root();
    let mut h = harness(Args {
        open: Some(r.join("examples/projects/P19_tube_cage.json")),
        glass_dirs: vec![r.join("tests/glass").to_string_lossy().into_owned()],
        ..Args::default()
    });
    // 開いた系（20° の光線が部品で全部切れるので、解析の一部は「光線なし」の誤りになる）
    assert_eq!(h.state().test_state()["system"], true);
    // 選ぶ前は頼まない
    assert!(h.state().c.sys.mech_check.is_none());
    let ctx = h.ctx.clone();
    crate::combo::mech::select(&ctx, Some("cage"));
    settle(&mut h);
    settle(&mut h);
    let v = crate::combo::mech::check(&ctx).expect("put");
    let lines = crate::combo::mech::check_lines("cage", &v);
    // 視野 3（20°）は通らず、cage が最初に切るのは 70.3 %（tube と合わせて切るのは 91.5 %）
    assert!(
        lines.iter().any(|l| l == "⚠ 3, 1: 0.0 / 70.3 (91.5)"),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l == "1, 1: 100.0 / 0.0 (0.0)"),
        "{lines:?}"
    );
    let tube = crate::combo::mech::check_lines("tube", &v);
    assert!(
        tube.iter().any(|l| l == "⚠ 3, 1: 0.0 / 29.7 (29.7)"),
        "{tube:?}"
    );
    assert_eq!(
        crate::combo::mech::check_lines("other", &v),
        Vec::<String>::new()
    );
    // 同じ系では頼み直さない
    let gen = h.state().c.sys.mech_check.as_ref().unwrap().0;
    assert_eq!(gen, h.state().c.sys_gen);
}

/// 計算中・誤りの札の行
#[test]
fn mech_card_lines_while_computing_and_on_error() {
    let _g = serial();
    i18n::set(i18n::Lang::En);
    assert_eq!(
        crate::combo::mech::check_lines("cage", &serde_json::Value::Null),
        ["Mechanical vignetting: computing…"]
    );
    assert_eq!(
        crate::combo::mech::check_lines("cage", &json!({ "error": "no mechanical parts" })),
        ["Mechanical vignetting: no mechanical parts"]
    );
    // 収まりの注意はその部品（part か other）の分だけ。誤りは ⚠
    let v = json!({
        "parts": [{ "name": "cage" }],
        "results": [],
        "fit": [
            { "part": "cage", "other": null, "error": true, "message": "cage: held element protrudes" },
            { "part": "tube", "other": "cage", "error": false, "message": "tube and cage overlap" },
            { "part": "tube", "other": null, "error": false, "message": "tube only" },
        ],
    });
    let lines = crate::combo::mech::check_lines("cage", &v);
    assert_eq!(
        &lines[1..],
        ["⚠ cage: held element protrudes", "tube and cage overlap"]
    );
}
