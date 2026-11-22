//! 面の表で置いた絞り以外の面の操作（ソルブ・ピックアップ）を、ブロックの `surface_ops`（[`crate::FaceMods`]）に
//! 足す・外す。同じ面・量の操作は 1 つにまとめ、ブロックの元（.zmx など）にあるものを置き換える・外すときは外す
//! 操作も置く（置いたものを外したとき、元のものに戻らない）。GUI の組合せの要素が使う

use crate::edit_spec::{BlockRef, PickupOp, Ref, SolveOp, SolveWhat, SurfaceOp};

/// 面にソルブを置く。shown は面の表にそのソルブが出ていたか。要素で置いたものでなければ元（.zmx など）の
/// ソルブなので、外す操作も置く（置いたソルブを外したとき、元のソルブに戻らない）
pub fn put_solve(ops: &mut Vec<SurfaceOp>, o: SolveOp<BlockRef>, shown: bool) {
    let (at, c) = (o.surface.clone(), o.is_curv());
    let own =
        |q: &SurfaceOp| matches!(q, SurfaceOp::AddSolve(q) if q.surface == at && q.is_curv() == c);
    let from_source = shown && !ops.iter().any(own);
    ops.retain(|q| !own(q));
    let removed = ops.iter().any(|q| solve_of(q, &at, c));
    if from_source && !removed {
        let what = if c { SolveWhat::C } else { SolveWhat::T };
        ops.push(SurfaceOp::RemoveSolve { surface: at, what });
    }
    ops.push(SurfaceOp::AddSolve(o));
}

/// 面にピックアップを置く（[`put_solve`] と同じく、元のピックアップなら外す操作も置く）
pub fn put_pickup(ops: &mut Vec<SurfaceOp>, o: PickupOp<BlockRef>, shown: bool) {
    let at = o.dst.clone();
    let what = o.what.clone();
    let own = |q: &SurfaceOp| matches!(q, SurfaceOp::AddPickup(q) if q.dst == at && q.what == what);
    let from_source = shown && !ops.iter().any(own);
    ops.retain(|q| !own(q));
    let removed = ops.iter().any(|q| pickup_of(q, &at, &what));
    if from_source && !removed {
        ops.push(SurfaceOp::RemovePickup { dst: at, what });
    }
    ops.push(SurfaceOp::AddPickup(o));
}

/// 面 i のソルブを外す: 要素で置いたソルブなら外し、無ければ要素の元（.zmx など）のソルブを外す操作を置く
pub fn drop_solve(ops: &mut Vec<SurfaceOp>, i: usize, what: SolveWhat) {
    drop_solve_at(ops, BlockRef::Sys(Ref::Index(i)), what)
}

/// [`drop_solve`] と同じ。面は here（復路の面を指すブロックの面など）
pub fn drop_solve_at(ops: &mut Vec<SurfaceOp>, here: BlockRef, what: SolveWhat) {
    let c = what == SolveWhat::C;
    let own = |q: &SurfaceOp| matches!(q, SurfaceOp::AddSolve(q) if q.surface == here && q.is_curv() == c);
    if ops.iter().any(own) {
        ops.retain(|q| !own(q));
    } else {
        ops.retain(|q| !solve_of(q, &here, c));
        ops.push(SurfaceOp::RemoveSolve {
            surface: here,
            what,
        });
    }
}

/// 面 i のピックアップ what を外す（ソルブと同じく、要素で置いたものが無ければ要素の元のものを外す）
pub fn drop_pickup(ops: &mut Vec<SurfaceOp>, i: usize, what: String) {
    drop_pickup_at(ops, BlockRef::Sys(Ref::Index(i)), what)
}

/// [`drop_pickup`] と同じ。面は here
pub fn drop_pickup_at(ops: &mut Vec<SurfaceOp>, here: BlockRef, what: String) {
    let own =
        |q: &SurfaceOp| matches!(q, SurfaceOp::AddPickup(q) if q.dst == here && q.what == what);
    if ops.iter().any(own) {
        ops.retain(|q| !own(q));
    } else {
        ops.retain(|q| !pickup_of(q, &here, &what));
        ops.push(SurfaceOp::RemovePickup { dst: here, what });
    }
}

/// 面 at の量（曲率か厚さ）のソルブを置く・外す操作か
fn solve_of(q: &SurfaceOp, at: &BlockRef, curv: bool) -> bool {
    match q {
        SurfaceOp::AddSolve(q) => q.surface == *at && q.is_curv() == curv,
        SurfaceOp::RemoveSolve { surface, what } => {
            surface == at && (*what == SolveWhat::C) == curv
        }
        _ => false,
    }
}

/// 面 at の量 what のピックアップを置く・外す操作か
fn pickup_of(q: &SurfaceOp, at: &BlockRef, what: &str) -> bool {
    match q {
        SurfaceOp::AddPickup(q) => q.dst == *at && q.what == what,
        SurfaceOp::RemovePickup { dst, what: w } => dst == at && w == what,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solve(i: usize) -> SolveOp<BlockRef> {
        SolveOp {
            surface: BlockRef::Sys(Ref::Index(i)),
            other: None,
            kind: Some("marginal_height".into()),
            value: None,
            param: None,
            macro_ref: None,
        }
    }

    fn pickup(i: usize) -> PickupOp<BlockRef> {
        PickupOp {
            dst: BlockRef::Sys(Ref::Index(i)),
            src: BlockRef::Sys(Ref::Index(1)),
            what: "t".into(),
            scale: None,
            offset: None,
            from: None,
        }
    }

    /// 同じ面・量のソルブは 1 つ。元のソルブを置き換えると外す操作も置き、置いたものを外しても元に戻らない
    #[test]
    fn solves_replace_and_remove_the_source() {
        let mut ops = Vec::new();
        put_solve(&mut ops, solve(2), false);
        put_solve(&mut ops, solve(2), true);
        assert_eq!(ops.len(), 1);
        drop_solve(&mut ops, 2, SolveWhat::T);
        assert!(ops.is_empty());
        // 元のソルブ（表に出ている）を置き換える
        put_solve(&mut ops, solve(3), true);
        assert!(matches!(ops[0], SurfaceOp::RemoveSolve { .. }));
        assert_eq!(ops.len(), 2);
        drop_solve(&mut ops, 3, SolveWhat::T);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], SurfaceOp::RemoveSolve { .. }));
        // 元のソルブを外すのは 1 回だけ（曲率のソルブは別）
        drop_solve(&mut ops, 3, SolveWhat::T);
        drop_solve(&mut ops, 3, SolveWhat::C);
        assert_eq!(ops.len(), 2);
    }

    /// ピックアップもソルブと同じ
    #[test]
    fn pickups_replace_and_remove_the_source() {
        let mut ops = Vec::new();
        put_pickup(&mut ops, pickup(2), true);
        put_pickup(&mut ops, pickup(2), true);
        assert_eq!(ops.len(), 2);
        drop_pickup(&mut ops, 2, "t".into());
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], SurfaceOp::RemovePickup { .. }));
    }
}
