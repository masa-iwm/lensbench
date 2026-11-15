//! 平面の点の最近傍を引く格子（エイミングの継続法）

/// 平面の点の部分集合の最近傍を引く格子（エイミングの継続法の、解けた光線の探索）
pub(super) struct NearGrid {
    pub(super) lo: [f64; 2],
    pub(super) cell: f64,
    pub(super) nx: usize,
    pub(super) ny: usize,
    pub(super) cells: Vec<Vec<usize>>,
}

impl NearGrid {
    pub(super) fn new(pts: &[[f64; 2]], idx: &[usize]) -> NearGrid {
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for &i in idx {
            for c in 0..2 {
                lo[c] = lo[c].min(pts[i][c]);
                hi[c] = hi[c].max(pts[i][c]);
            }
        }
        let m = idx.len().max(1) as f64;
        let (w, h) = ((hi[0] - lo[0]).max(0.0), (hi[1] - lo[1]).max(0.0));
        // 1 つの箱に平均 2 点ほど
        let mut cell = ((w * h).max(w.max(h).powi(2) / m) * 2.0 / m).sqrt();
        if !(cell > 0.0 && cell.is_finite()) {
            cell = 1.0;
        }
        let nx = ((w / cell) as usize + 1).min(4096);
        let ny = ((h / cell) as usize + 1).min(4096);
        let mut g = NearGrid {
            lo: if idx.is_empty() { [0.0; 2] } else { lo },
            cell,
            nx,
            ny,
            cells: vec![Vec::new(); nx * ny],
        };
        for &i in idx {
            let (cx, cy) = g.at(pts[i]);
            g.cells[cy * nx + cx].push(i);
        }
        g
    }

    pub(super) fn at(&self, q: [f64; 2]) -> (usize, usize) {
        let f = |v: f64, lo: f64, n: usize| (((v - lo) / self.cell).max(0.0) as usize).min(n - 1);
        (f(q[0], self.lo[0], self.nx), f(q[1], self.lo[1], self.ny))
    }

    /// q に最も近い点（except を除く）と距離。同じ距離なら番号の小さい方（結果は順によらない）
    pub(super) fn nearest(
        &self,
        pts: &[[f64; 2]],
        q: [f64; 2],
        except: Option<usize>,
    ) -> Option<(usize, f64)> {
        let (cx, cy) = self.at(q);
        let mut best: Option<(usize, f64)> = None;
        let rmax = self.nx.max(self.ny);
        for r in 0..=rmax {
            // 輪 r の箱はどれも q から (r − 1)·cell より遠い
            if let Some((_, d)) = best {
                if (r as f64 - 1.0) * self.cell > d {
                    break;
                }
            }
            let (x0, x1) = (cx as i64 - r as i64, cx as i64 + r as i64);
            let (y0, y1) = (cy as i64 - r as i64, cy as i64 + r as i64);
            for y in y0..=y1 {
                if y < 0 || y >= self.ny as i64 {
                    continue;
                }
                for x in x0..=x1 {
                    if x < 0 || x >= self.nx as i64 || (y != y0 && y != y1 && x != x0 && x != x1) {
                        continue;
                    }
                    for &j in &self.cells[y as usize * self.nx + x as usize] {
                        if Some(j) == except {
                            continue;
                        }
                        let d = ((pts[j][0] - q[0]).powi(2) + (pts[j][1] - q[1]).powi(2)).sqrt();
                        let better = match best {
                            None => true,
                            Some((bj, bd)) => d < bd || (d == bd && j < bj),
                        };
                        if better {
                            best = Some((j, d));
                        }
                    }
                }
            }
        }
        best
    }
}
