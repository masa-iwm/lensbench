//! 実像高・条件で定める視野の主光線の物体を解く（system.rs から分けた）
use super::*;

/// 視野の解のキャッシュの件数の上限（視野の掃引で増え続けないよう、超えたら捨てて解き直す。KRN-23）
const REALH_MAX: usize = 4096;

impl System {
    /// 条件 c の目標値 (x, y) を満たす主光線の物体を解く（実像高と条件で定める視野）。結果は系にキャッシュする
    pub(super) fn cond_spec(&self, c: FieldCond, x: f64, y: f64) -> Result<ObjSpec> {
        let img_s = self.image();
        let surfs = match c {
            FieldCond::Position { surf } => vec![surf],
            FieldCond::DirSum { s1, s2 } | FieldCond::DirDiff { s1, s2 } => vec![s1, s2],
        };
        for &j in &surfs {
            if j == 0 || j > img_s || self.surfaces[j].ignore {
                invalid!("field condition surface {j} (surfaces not ignored, 1 to {img_s})");
            }
        }
        let wave = self.field_wave.unwrap_or(self.primary);
        let key = (c, wave, x.to_bits(), y.to_bits());
        if let Some(v) = self.cache.realh.lock_ok().get(&key) {
            return Ok(*v);
        }
        // 初期値: 像面の位置なら近軸の像高から、ほかは軸上から
        let mut v = match c {
            FieldCond::Position { surf } if surf == img_s => {
                let h1 = self.paraxial().chief_image_per_unit;
                [x / h1, y / h1]
            }
            _ => [0.0, 0.0],
        };
        let spec = |v: [f64; 2]| {
            if self.infinite() {
                ObjSpec::Dir(normalize([v[0], v[1], 1.0]))
            } else {
                ObjSpec::Point(v)
            }
        };
        let img = |v: [f64; 2]| -> [f64; 2] {
            match c {
                FieldCond::Position { surf } => {
                    let r = self.trace_spec(
                        spec(v),
                        &[0.0],
                        &[0.0],
                        self.field_wave.unwrap_or(self.primary),
                        TraceOpts::new().vig([0.0; 5]).upto(surf),
                    );
                    [r.rays[0].xyz[0], r.rays[0].xyz[1]]
                }
                FieldCond::DirSum { s1, s2 } | FieldCond::DirDiff { s1, s2 } => {
                    let r = self.trace_spec(
                        spec(v),
                        &[0.0],
                        &[0.0],
                        self.field_wave.unwrap_or(self.primary),
                        TraceOpts::new().vig([0.0; 5]).upto(s1.max(s2)).record(),
                    );
                    if r.rays[0].err != 0 {
                        return [f64::NAN; 2];
                    }
                    let (Some(a), Some(b)) = (r.rec_at(s1), r.rec_at(s2)) else {
                        return [f64::NAN; 2];
                    };
                    let (a, b) = (a.lmn[0], b.lmn[0]);
                    if matches!(c, FieldCond::DirSum { .. }) {
                        [a[0] + b[0], a[1] + b[1]]
                    } else {
                        [a[0] - b[0], a[1] - b[1]]
                    }
                }
            }
        };
        // 収束は残差 < 1e-13（<= の幅は 1e-13 のすぐ下の数）
        let nw = crate::newton2::Newton2 {
            tol: f64::from_bits(1e-13f64.to_bits() - 1),
            max_it: 40,
            diff: crate::newton2::Diff {
                h: 1e-7,
                rel: true,
                central: false,
            },
            line: crate::newton2::Line::Full,
        };
        v = nw.solve(img, v, [x, y]).v;
        let q = img(v);
        let e = (q[0] - x).abs().max((q[1] - y).abs());
        if !(e < 1e-9 * x.abs().max(y.abs()).max(1.0)) {
            match c {
                FieldCond::Position { surf } if surf == img_s => {
                    invalid!("real image height field ({x}, {y}): no real chief ray reaches it (off by {e})")
                }
                _ => {
                    invalid!("field ({x}, {y}) with condition {c:?}: can't be solved (off by {e})")
                }
            }
        }
        let s = spec(v);
        let mut m = self.cache.realh.lock_ok();
        // 視野の掃引で増え続けないよう、上限で捨てる（解き直せばよい、KRN-23）
        if m.len() >= REALH_MAX {
            m.clear();
        }
        m.insert(key, s);
        Ok(s)
    }
}
