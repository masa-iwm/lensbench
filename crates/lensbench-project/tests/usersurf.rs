//! 利用者定義の面を編集の操作で付け、系ファイルに書いて読み直す

use lensbench_core::{zmx, FieldSel};
use lensbench_project::{edit_ops, sysfile};
use serde_json::json;

const ZMX: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 4\nFTYP 0 0 3 1 0 0 0\n\
XFLN 0 0 0\nYFLN 0 15 30\nFWGN 1 1 1\nWAVM 1 0.55 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ 50\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";

#[test]
fn edit_and_system_file_round_trip() {
    let raw = zmx::parse_text(ZMX).unwrap();
    let mut s = zmx::build(&raw, &zmx::LoadOptions::default(), &[], None).unwrap();
    // 形のデータ（user）が無いうちは面タイプを変えられない
    assert!(edit_ops::apply(
        &mut s,
        &json!([{"op": "set_surface_type", "surface": 1, "type": "USER"}])
    )
    .is_err());
    edit_ops::apply(
        &mut s,
        &json!([
            {"op": "set_surface_data", "surface": 1,
             "data": {"user": {"model": "ideal_mapping", "config": {"f": 50, "mapping": "theta"}}}},
            {"op": "set_surface_type", "surface": 1, "type": "USER"},
        ]),
    )
    .unwrap();
    let t = s.trace(FieldSel::Index(2), &[0.0], &[0.0], None, None, false);
    assert!((t.rays[0].xyz[1] - 50.0 * 30f64.to_radians()).abs() < 1e-9);
    let d = edit_ops::surface_data(&s, 1).unwrap();
    assert_eq!(d["data"]["user"]["model"], json!("ideal_mapping"));
    assert_eq!(d["data"]["user"]["config"]["mapping"], json!("theta"));
    // 系ファイルに書いて読み直すと同じ追跡
    let txt = sysfile::to_string(&s, &[]).unwrap();
    let (s2, _, _) = sysfile::from_str(&txt).unwrap();
    let t2 = s2.trace(
        FieldSel::Index(2),
        &[0.0, 0.3],
        &[0.0, -0.5],
        None,
        None,
        false,
    );
    let t1 = s.trace(
        FieldSel::Index(2),
        &[0.0, 0.3],
        &[0.0, -0.5],
        None,
        None,
        false,
    );
    for (a, b) in t1.rays.iter().zip(&t2.rays) {
        assert_eq!((a.xyz, a.lmn, a.lens_opd), (b.xyz, b.lmn, b.lens_opd));
    }
    // 知らないモデル・誤った設定は誤り
    let bad = txt.replace("ideal_mapping", "no_such_model");
    assert!(sysfile::from_str(&bad).is_err());
    assert!(edit_ops::apply(
        &mut s,
        &json!([{"op": "set_surface_data", "surface": 1,
                 "data": {"user": {"model": "ideal_mapping", "config": {"f": 0}}}}])
    )
    .is_err());
}
