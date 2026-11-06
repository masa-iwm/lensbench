//! 材料のピックアップ（.zmx の GLAS 名前 2 元の面 …）。Ghost Focus Generator が保存するゴーストの系は戻りの面の
//! 材料をこの形で書く（名前と nd・vd は保存時のもので、使わない）。

use lensbench_core::param::PickKind;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::{zmx, zmx_write};

const HEAD: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 2 2 0 0 0\n\
XFLN 0 1\nYFLN 0 2\nFWGN 1 1\nWAVM 1 0.5876 1\nWAVM 2 0.4861 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";

/// 単レンズの後ろの平面鏡で折り返し、同じレンズを戻る系。戻りの面 5 は面 1 の材料（分散ありのモデルガラス）を、
/// 面 4 は面 2（空気）を、面 6 は面 5 をピックアップする（連なり）
fn text(m4: &str, m5: &str, m6: &str) -> String {
    format!(
        "{HEAD}SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS ___BLANK 1 0 1.62 36.4 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 10\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ -10\n  GLAS MIRROR 0 0 1.5 40 0 0 0 0 0 0\n\
SURF 4\n  TYPE STANDARD\n  CURV -0.01\n  DISZ -4\n{m4}\
SURF 5\n  TYPE STANDARD\n  CURV 0.02\n  DISZ -2\n{m5}\
SURF 6\n  TYPE STANDARD\n  CURV 0.0\n  DISZ -1\n{m6}\
SURF 7\n  TYPE STANDARD\n  CURV 0.0\n  DISZ -20\n\
SURF 8\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    )
}

fn picked() -> String {
    text(
        "  GLAS ___BLANK 2 2 1.5 40 0 0 0 0 0 0\n",
        "  GLAS ___BLANK 2 1 1.5 40 0 0 0 0 0 0\n",
        "  GLAS F2 2 4 1.5 40 0 0 0 0 0 0\n",
    )
}

/// 同じ材料を直接書いた系
fn explicit() -> String {
    let g = "  GLAS ___BLANK 1 0 1.62 36.4 0 0 0 0 0\n";
    text("", g, "")
}

fn build(txt: &str) -> System {
    let raw = zmx::parse_text(txt).unwrap();
    zmx::build(&raw, &Default::default(), &[], None).unwrap()
}

fn bits(s: &System) -> Vec<u64> {
    let px = [0.0, 0.5, -0.3, 0.2];
    let py = [0.0, 0.1, 0.6, -0.7];
    let mut v = Vec::new();
    for w in 0..s.wavelengths.len() {
        for f in 0..s.fields.len() {
            let t = s.trace(FieldSel::Index(f), &px, &py, Some(w), None, false);
            for r in &t.rays {
                assert_eq!(r.err, 0);
                v.extend([r.xyz[0], r.xyz[1], r.lmn[0], r.opl].map(f64::to_bits));
            }
        }
    }
    v
}

#[test]
fn resolves_to_source_material() {
    let s = build(&picked());
    let e = build(&explicit());
    for k in 0..s.surfaces.len() {
        assert_eq!(s.surfaces[k].n, e.surfaces[k].n, "面 {k}");
    }
    assert_eq!(s.surfaces[5].mat, s.surfaces[1].mat);
    assert_eq!(s.surfaces[4].n, vec![1.0, 1.0]);
    assert_eq!(s.surfaces[6].n, vec![1.0, 1.0]);
    assert_eq!(bits(&s), bits(&e));
    let k = |d: usize| -> Vec<PickKind> {
        s.pickups
            .iter()
            .filter(|q| q.dst == d)
            .map(|q| q.what)
            .collect()
    };
    assert_eq!(k(5), vec![PickKind::Dn, PickKind::Disp]);
}

/// 元の面の屈折率のずれ（dn・disp）に追従し、ピックアップ先そのものは変えられない
#[test]
fn follows_index_change() {
    let s = build(&picked());
    let c = s
        .with_params(&[
            (s.param("1.dn").unwrap(), 2e-3),
            (s.param("1.disp").unwrap(), 0.05),
        ])
        .unwrap();
    assert_eq!(c.surfaces[5].dn, 2e-3);
    assert_eq!(c.surfaces[5].disp, 0.05);
    assert_eq!(c.surfaces[5].n, c.surfaces[1].n);
    assert_ne!(c.surfaces[5].n, s.surfaces[5].n);
    let e = s.with_params(&[(s.param("5.dn").unwrap(), 1e-3)]);
    assert!(e.unwrap_err().to_string().contains("ピックアップ"));
}

/// 書き出して読み戻してもつながりが残る
#[test]
fn round_trip_keeps_links() {
    let s = build(&picked());
    let b = build(&zmx_write::write(&s).unwrap());
    assert_eq!(s.pickups, b.pickups);
    assert_eq!(bits(&s), bits(&b));
}

#[test]
fn cycle_and_missing_source_are_errors() {
    let t = picked().replace(
        "  GLAS ___BLANK 1 0 1.62 36.4 0 0 0 0 0\n",
        "  GLAS ___BLANK 2 5 1.5 40 0 0 0 0 0 0\n",
    );
    let e = zmx::build(
        &zmx::parse_text(&t).unwrap(),
        &Default::default(),
        &[],
        None,
    );
    assert!(e.unwrap_err().to_string().contains("循環"));
    let t = picked().replace("___BLANK 2 2 ", "___BLANK 2 99 ");
    assert!(zmx::build(
        &zmx::parse_text(&t).unwrap(),
        &Default::default(),
        &[],
        None
    )
    .is_err());
}
