//! 往路を戻る鏡にまとめるダブルパスの、戻りの面のソルブ

use super::*;
use lensbench_project::edit_ops::{BlockRef, SurfaceOp};

/// 戻りの最後の面に厚さのソルブ（周辺光線の高さ）のある .zmx も往路を戻る鏡にまとめ、ソルブは往きの玉の面 0 の
/// 復路の面（ブロックの面の back）の操作になる（split の中で元の系と同じことも確かめる）
#[test]
fn a_solve_on_a_return_face_folds_into_the_returning_mirror() {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let glass = vec![root.join("tests/glass")];
    let c = Combo::from_zmx(
        &root.join("tests/double_pass/return_solve.zmx"),
        &glass,
        &[],
    )
    .unwrap();
    let kinds: Vec<ElemKind> = c.picks.iter().map(|p| p.kind).collect();
    assert_eq!(kinds, [ElemKind::Lens, ElemKind::Mirror], "{kinds:?}");
    assert!(c.picks[1].mirror().back);
    let solves: Vec<&SurfaceOp> = c.picks[0]
        .ops
        .iter()
        .filter(|o| matches!(o, SurfaceOp::AddSolve(_)))
        .collect();
    assert_eq!(solves.len(), 1, "{:?}", c.picks[0].ops);
    let SurfaceOp::AddSolve(q) = solves[0] else {
        unreachable!()
    };
    assert!(
        matches!(&q.surface, BlockRef::Block(f) if f.back && f.surface == 0),
        "{:?}",
        q.surface
    );
}
