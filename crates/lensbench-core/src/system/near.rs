//! 平面の点の最近傍を引く格子（エイミングの継続法）

/// 格子の 1 軸の箱の数の上限（これを超える軸では、範囲の外の点を最後の箱に詰める）
const MAX_CELLS: usize = 4096;

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
        let nx = ((w / cell) as usize + 1).min(MAX_CELLS);
        let ny = ((h / cell) as usize + 1).min(MAX_CELLS);
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
        // q が格子の範囲の外にある分（軸ごと）。箱の数が頭打ちの軸は最後の箱が lo + n·cell より先まで続くので、上側の外の
        // 分は数えない
        let out = |v: f64, lo: f64, n: usize| {
            let hi = if n >= MAX_CELLS {
                f64::INFINITY
            } else {
                lo + n as f64 * self.cell
            };
            (lo - v).max(v - hi).max(0.0)
        };
        let (ox, oy) = (
            out(q[0], self.lo[0], self.nx),
            out(q[1], self.lo[1], self.ny),
        );
        for r in 0..=rmax {
            // 輪 r の箱は、どれも x か y の一方で q の箱から (r − 1)·cell 以上離れ、範囲の外の q からはその外の分も
            // 離れている（q が範囲の外で遠いと (r − 1)·cell だけでは止まらず、毎回格子を全部見ていた）
            if let Some((_, d)) = best {
                let a = (r as f64 - 1.0).max(0.0) * self.cell;
                let lb = (ox + a).hypot(oy).min(ox.hypot(oy + a));
                if lb > d {
                    break;
                }
            }
            // 輪が格子を全部覆った後の輪には箱が無い
            if r > 0 && cx < r && cy < r && cx + r >= self.nx && cy + r >= self.ny {
                break;
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

    /// q から距離 r 以内の点ごとに f(番号, 距離)（距離は [`Self::nearest`] と同じ式）
    pub(super) fn within(
        &self,
        pts: &[[f64; 2]],
        q: [f64; 2],
        r: f64,
        mut f: impl FnMut(usize, f64),
    ) {
        let (x0, y0) = self.at([q[0] - r, q[1] - r]);
        let (x1, y1) = self.at([q[0] + r, q[1] + r]);
        for y in y0..=y1 {
            for x in x0..=x1 {
                for &j in &self.cells[y * self.nx + x] {
                    let d = ((pts[j][0] - q[0]).powi(2) + (pts[j][1] - q[1]).powi(2)).sqrt();
                    if d <= r {
                        f(j, d);
                    }
                }
            }
        }
    }

    /// q から距離 r 以内で最も近い点と距離（同じ距離なら番号の小さい方。[`Self::nearest`] がその距離を返すなら同じ点）
    pub(super) fn nearest_within(
        &self,
        pts: &[[f64; 2]],
        q: [f64; 2],
        r: f64,
    ) -> Option<(usize, f64)> {
        let mut best: Option<(usize, f64)> = None;
        self.within(pts, q, r, |j, d| {
            let better = match best {
                None => true,
                Some((bj, bd)) => d < bd || (d == bd && j < bj),
            };
            if better {
                best = Some((j, d));
            }
        });
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 細長い点の集まり（x の箱の数が頭打ち）でも、範囲の外・最後の箱に詰められた所の q の最近接が総当たりと同じ
    #[test]
    fn nearest_matches_brute_force_on_a_capped_grid() {
        // 決まった擬似乱数（xorshift）
        let mut st = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = || {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            (st >> 11) as f64 / (1u64 << 53) as f64
        };
        let pts: Vec<[f64; 2]> = (0..100_000).map(|_| [1000.0 * rnd(), rnd()]).collect();
        let idx: Vec<usize> = (0..pts.len()).collect();
        let g = NearGrid::new(&pts, &idx);
        assert_eq!(g.nx, MAX_CELLS);
        for _ in 0..2000 {
            let q = [600.0 + 400.0 * rnd(), 1.0 + 0.3 * rnd()];
            let brute = pts
                .iter()
                .enumerate()
                .map(|(j, p)| (j, ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()))
                .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
                .unwrap();
            assert_eq!(g.nearest(&pts, q, None), Some(brute), "q {q:?}");
        }
    }
}
