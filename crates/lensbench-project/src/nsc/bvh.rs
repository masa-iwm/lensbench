//! 境界の箱の木（BVH）: 平らな配列の節（ポインタの木にしない。GPU へ移せる形）

use lensbench_core::math::V3;

/// 節: 箱と、葉なら面の範囲（`items[first..first + count]`）、内の節なら子（左は次の節、右は `right`）
#[derive(Debug, Clone, Copy)]
pub(super) struct Node {
    pub lo: V3,
    pub hi: V3,
    /// 葉の面の数（0 なら内の節）
    pub count: u32,
    /// 葉: 最初の面の位置、内の節: 右の子の節の番号
    pub index: u32,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Bvh {
    pub nodes: Vec<Node>,
    /// 葉が指す面の番号
    pub items: Vec<u32>,
}

const LEAF: usize = 2;

impl Bvh {
    /// 箱（面の番号つき）から作る。中心の並びの中央で、広がりの大きい軸で分ける
    pub(super) fn build(boxes: &[(usize, [V3; 2])]) -> Bvh {
        let mut b = Bvh::default();
        if boxes.is_empty() {
            return b;
        }
        let mut v: Vec<(usize, [V3; 2])> = boxes.to_vec();
        b.split(&mut v);
        b
    }

    fn split(&mut self, v: &mut [(usize, [V3; 2])]) -> usize {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for (_, bx) in v.iter() {
            for a in 0..3 {
                lo[a] = lo[a].min(bx[0][a]);
                hi[a] = hi[a].max(bx[1][a]);
            }
        }
        let me = self.nodes.len();
        self.nodes.push(Node {
            lo,
            hi,
            count: 0,
            index: 0,
        });
        if v.len() <= LEAF {
            self.nodes[me].count = v.len() as u32;
            self.nodes[me].index = self.items.len() as u32;
            self.items.extend(v.iter().map(|(i, _)| *i as u32));
            return me;
        }
        let ctr = |bx: &[V3; 2], a: usize| bx[0][a] + bx[1][a];
        let (mut clo, mut chi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for (_, bx) in v.iter() {
            for a in 0..3 {
                clo[a] = clo[a].min(ctr(bx, a));
                chi[a] = chi[a].max(ctr(bx, a));
            }
        }
        let axis = (0..3)
            .max_by(|&a, &b| (chi[a] - clo[a]).total_cmp(&(chi[b] - clo[b])))
            .unwrap_or(0);
        // 同じ中心は番号の順（作り方を決めておく）
        v.sort_by(|x, y| {
            ctr(&x.1, axis)
                .total_cmp(&ctr(&y.1, axis))
                .then(x.0.cmp(&y.0))
        });
        let mid = v.len() / 2;
        let (l, r) = v.split_at_mut(mid);
        self.split(l);
        let right = self.split(r);
        self.nodes[me].index = right as u32;
        me
    }

    /// 光線 (p, 1/k) が箱に入る距離の範囲が [0, tmax) と重なる面を、近い順を問わず f に渡す。f は今の最小の距離を
    /// 返し、それより遠い箱は飛ばす
    pub(super) fn visit(&self, p: V3, inv: V3, mut tmax: f64, mut f: impl FnMut(usize) -> f64) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = [0u32; 64];
        let mut sp = 1;
        stack[0] = 0;
        while sp > 0 {
            sp -= 1;
            let id = stack[sp] as usize;
            let n = &self.nodes[id];
            if !slab(n.lo, n.hi, p, inv, tmax) {
                continue;
            }
            if n.count > 0 {
                let s = n.index as usize;
                for &i in &self.items[s..s + n.count as usize] {
                    tmax = tmax.min(f(i as usize));
                }
            } else {
                // 深さは面の数の log2 程度（64 を超えない）
                stack[sp] = n.index;
                stack[sp + 1] = id as u32 + 1;
                sp += 2;
            }
        }
    }
}

/// 箱と光線の交わり（スラブ法）
fn slab(lo: V3, hi: V3, p: V3, inv: V3, tmax: f64) -> bool {
    let (mut t0, mut t1) = (0.0f64, tmax);
    for a in 0..3 {
        let (mut ta, mut tb) = ((lo[a] - p[a]) * inv[a], (hi[a] - p[a]) * inv[a]);
        if ta.is_nan() || tb.is_nan() {
            // 方向の成分 0 で面の上: 箱の中にいれば通す
            if p[a] < lo[a] || p[a] > hi[a] {
                return false;
            }
            continue;
        }
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
        if t0 > t1 {
            return false;
        }
    }
    true
}
