//! 展開した系 1 つにした編集（serve の `set`・`edit`）を、同じプロジェクトの別の状態・BS の経路の系へ、面の名前
//! （[`FaceId`]）で当て直す。面の番号は状態で変わる（玉を外すなど）ので、番号のままでは別の面に当たる

use crate::edit_spec::{EditOp, Ref};
use crate::FaceId;

/// 編集した系（元）の面の番号から、当て直す系（先）の面の番号への表。面を入れる・消す編集を当てるたびに両方で合わせる
#[derive(Debug, Clone)]
pub struct Retarget {
    /// 元の面 i → 先の面（先に無ければ None）
    pair: Vec<Option<usize>>,
    /// 先の面の数
    len: usize,
    /// 面の並びが同じ（番号のまま当ててよい）
    same: bool,
    /// 面を入れる・消す編集を当てられず、表がもう合わない（後の面の編集は当てない）
    broken: bool,
}

impl Retarget {
    /// from・to は元・先の系の面ごとの名前（[`crate::SurfOrigin::face`]。物体面・像面は None）
    pub fn new(from: &[Option<FaceId>], to: &[Option<FaceId>]) -> Self {
        let same = from == to;
        // 範囲（ブロック・向き・何回目・座標ブレークか）ごとの面の数。元と先で数が違う範囲（状態がブロックの中身を
        // 替えたなど）は、同じ番号が別の面を指すので、先に無いものとする
        type Run<'f> = (&'f crate::NodePath, bool, usize, bool);
        fn runs(v: &[Option<FaceId>]) -> std::collections::HashMap<Run<'_>, usize> {
            let mut n = std::collections::HashMap::new();
            for f in v.iter().flatten() {
                *n.entry((&f.node, f.back, f.pass, f.cb)).or_insert(0) += 1;
            }
            n
        }
        let (nf, nt) = (runs(from), runs(to));
        let mut at = std::collections::HashMap::new();
        for (k, f) in to.iter().enumerate() {
            if let Some(f) = f {
                let r = (&f.node, f.back, f.pass, f.cb);
                if nf.get(&r) == nt.get(&r) {
                    at.insert(f, k);
                }
            }
        }
        let n = from.len();
        let pair = from
            .iter()
            .enumerate()
            .map(|(i, f)| match f {
                Some(f) => at.get(f).copied(),
                // 物体面と像面
                None if i == 0 => Some(0),
                None if i + 1 == n && !to.is_empty() => Some(to.len() - 1),
                None => None,
            })
            .collect();
        Retarget {
            pair,
            len: to.len(),
            same,
            broken: false,
        }
    }

    /// 面の並びが同じで、編集を番号のまま当ててよいか
    pub fn is_same(&self) -> bool {
        self.same
    }

    /// 元の面 i の先の面
    fn surf(&self, i: usize) -> Result<usize, String> {
        if self.broken {
            return Err(
                "an earlier change that inserted or deleted surfaces was not applied".into(),
            );
        }
        self.pair
            .get(i)
            .copied()
            .flatten()
            .ok_or_else(|| format!("surface {i} is not in this system"))
    }

    fn r(&self, r: &Ref) -> Result<Ref, String> {
        Ok(match r {
            Ref::Index(i) => Ref::Index(self.surf(*i)?),
            Ref::Name(n) => Ref::Name(n.clone()),
        })
    }

    fn spans(&self, v: &[[usize; 2]]) -> Result<Vec<[usize; 2]>, String> {
        v.iter()
            .map(|&[a, b]| {
                let (x, y) = (self.surf(a)?, self.surf(b)?);
                if y < x || y - x != b - a {
                    return Err(format!(
                        "surfaces {a} to {b} are not in the same order in this system"
                    ));
                }
                Ok([x, y])
            })
            .collect()
    }

    /// パラメータの名前（`3.t`・`surf:3.t`）の面の番号を先の面に。名前で書いた面・玉・群・系のパラメータはそのまま
    pub fn param(&self, name: &str) -> Result<String, String> {
        if self.same {
            return Ok(name.to_string());
        }
        let s = name.trim();
        let Some((t, k)) = s.rsplit_once('.') else {
            return Ok(name.to_string());
        };
        let (pre, t) = match t.strip_prefix("surf:") {
            Some(x) => ("surf:", x),
            None => ("", t),
        };
        match t.trim().parse::<usize>() {
            Ok(i) => Ok(format!("{pre}{}.{k}", self.surf(i)?)),
            Err(_) => Ok(name.to_string()),
        }
    }

    /// 編集の操作 1 つを先の系の面に読み替える。当てられなければ理由を返す（面が先の系に無いなど）。
    /// 面を入れる・消す操作は、返した操作を先の系に当てる前提で表も合わせる
    pub fn op(&mut self, op: &EditOp) -> Result<EditOp, String> {
        use EditOp::*;
        if self.same {
            return Ok(op.clone());
        }
        let mut o = op.clone();
        match &mut o {
            InsertSurface { at, .. } => {
                let b = self.surf(*at).inspect_err(|_| self.broken = true)?;
                for p in self.pair.iter_mut().flatten() {
                    if *p >= b {
                        *p += 1;
                    }
                }
                self.pair.insert(*at, Some(b));
                self.len += 1;
                *at = b;
            }
            DeleteSurface { surface } => {
                let Ref::Index(a) = *surface else {
                    self.broken = true;
                    return Err("a surface deleted by name can't be found in this system".into());
                };
                let b = self.surf(a).inspect_err(|_| self.broken = true)?;
                self.pair.remove(a);
                for p in self.pair.iter_mut().flatten() {
                    if *p > b {
                        *p -= 1;
                    }
                }
                self.len -= 1;
                *surface = Ref::Index(b);
            }
            SetSurface { surface, .. }
            | SetSurfaceType { surface, .. }
            | SetMaterial { surface, .. }
            | RemoveSolve { surface, .. }
            | SetSurfaceData { surface, .. } => *surface = self.r(surface)?,
            RemovePickup { dst, .. } => *dst = self.r(dst)?,
            SetStop(x) => x.surface = self.r(&x.surface)?,
            AddMacroSolve(x) => x.surface = self.r(&x.surface)?,
            AddPickup(x) => {
                x.dst = self.r(&x.dst)?;
                x.src = self.r(&x.src)?;
            }
            AddSolve(x) => {
                x.surface = self.r(&x.surface)?;
                if let Some(q) = &x.other {
                    x.other = Some(self.r(q)?);
                }
            }
            SetParam { param, .. }
            | SetConfigRow { param, .. }
            | SetConfigMaterials { param, .. }
            | SetConfigCoatings { param, .. }
            | AddConfigRow { param }
            | RemoveConfigRow { param }
            | RemoveConfigPickup { param, .. } => *param = self.param(param)?,
            SetConfigPickup { param, row, .. } => {
                *param = self.param(param)?;
                if let Some(r) = row {
                    *r = self.param(r)?;
                }
            }
            SetSystemData { data } => {
                if let Some(g) = data.get("glrs").and_then(|v| v.as_u64()) {
                    data.insert("glrs".into(), self.surf(g as usize)?.into());
                }
            }
            DefineUnit { spans, .. } => *spans = self.spans(spans)?,
            SetUnit { spans, .. } => {
                if let Some(v) = spans {
                    *v = self.spans(v)?;
                }
            }
            AddMech { part } => {
                part.surface = self.surf(part.surface)?;
                part.holds = part
                    .holds
                    .iter()
                    .map(|&j| self.surf(j))
                    .collect::<Result<_, _>>()?;
            }
            SetMech { part, .. } => {
                if let Some(j) = part.get("surface").and_then(|v| v.as_u64()) {
                    part.insert("surface".into(), self.surf(j as usize)?.into());
                }
                if let Some(v) = part.get("holds").and_then(|v| v.as_array()) {
                    let h: Vec<usize> = v
                        .iter()
                        .map(|x| {
                            x.as_u64()
                                .ok_or_else(|| "holds must be surface numbers".to_string())
                                .and_then(|j| self.surf(j as usize))
                        })
                        .collect::<Result<_, _>>()?;
                    part.insert("holds".into(), serde_json::json!(h));
                }
            }
            RemoveUnit { .. }
            | RemoveMech { .. }
            | SetConfigCount { .. }
            | SetConfig { .. }
            | InsertConfig { .. }
            | RemoveConfig { .. }
            | MakeThermal { .. } => {}
        }
        debug_assert!(self.pair.iter().flatten().all(|&p| p < self.len));
        Ok(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodePath, NodeStep};

    fn f(block: usize, index: usize) -> Option<FaceId> {
        Some(FaceId {
            node: NodePath(vec![NodeStep::Block(block)]),
            back: false,
            pass: 0,
            cb: false,
            index,
        })
    }

    fn set(j: usize) -> EditOp {
        EditOp::SetSurface {
            surface: Ref::Index(j),
            fields: Default::default(),
        }
    }

    fn at(o: &EditOp) -> usize {
        match o {
            EditOp::SetSurface {
                surface: Ref::Index(j),
                ..
            }
            | EditOp::DeleteSurface {
                surface: Ref::Index(j),
            } => *j,
            EditOp::InsertSurface { at, .. } => *at,
            _ => unreachable!(),
        }
    }

    /// 先の系に無いブロック 1 の面は当てない。面を入れる・消すと両方の番号を合わせる
    #[test]
    fn follows_inserts_and_deletes() {
        let from = [None, f(0, 0), f(1, 0), f(1, 1), f(2, 0), f(2, 1), None];
        let to = [None, f(0, 0), f(2, 0), f(2, 1), None];
        let mut r = Retarget::new(&from, &to);
        assert!(!r.is_same());
        assert_eq!(at(&r.op(&set(4)).unwrap()), 2);
        assert_eq!(at(&r.op(&set(6)).unwrap()), 4, "像面");
        assert!(r.op(&set(2)).is_err());
        assert_eq!(r.param("5.t").unwrap(), "3.t");
        assert_eq!(r.param("surf:5.t").unwrap(), "surf:3.t");
        assert_eq!(r.param("L2.shift").unwrap(), "L2.shift");
        // 面 4 の前に 1 面: 元の 5 は 6、先の 3 は 4
        let ins = EditOp::InsertSurface {
            at: 4,
            surface: Default::default(),
        };
        assert_eq!(at(&r.op(&ins).unwrap()), 2);
        assert_eq!(at(&r.op(&set(4)).unwrap()), 2, "入れた面");
        assert_eq!(at(&r.op(&set(6)).unwrap()), 4);
        // 元の面 1 を消す: 元の 6 は 5、先の 4 は 3
        let del = EditOp::DeleteSurface {
            surface: Ref::Index(1),
        };
        assert_eq!(at(&r.op(&del).unwrap()), 1);
        assert_eq!(at(&r.op(&set(5)).unwrap()), 3);
        // 先に無い面を消すと、後の面の編集は当てない
        assert!(r
            .op(&EditOp::DeleteSurface {
                surface: Ref::Index(1)
            })
            .is_err());
        assert!(r.op(&set(4)).is_err());
    }

    /// 並びが同じなら番号のまま
    #[test]
    fn same_layout_keeps_numbers() {
        let v = [None, f(0, 0), f(1, 0), None];
        let mut r = Retarget::new(&v, &v);
        assert!(r.is_same());
        assert_eq!(at(&r.op(&set(9)).unwrap()), 9);
    }
}
