//! 2 次元の格子の値。解析の 2 次元の結果（波面・PSF・干渉縞など）は全部この形で持つ。
//!
//! 値は行優先の平らな並び（行 = y、`data[iy * nx + ix]`）で、各軸の座標を並びで持つ（標本 i の座標 `x[i]`）。
//! Python へは写さずに numpy の (ny, nx) の配列にでき、プラグインの配列への変換も 1 つで済む。

/// 2 次元の格子の値。座標の並び x（長さ nx）・y（長さ ny）と、行優先の値 data（長さ nx·ny、行 = y）
#[derive(Debug, Clone, PartialEq)]
pub struct Grid2<T = f64> {
    x: Vec<f64>,
    y: Vec<f64>,
    data: Vec<T>,
}

impl<T> Grid2<T> {
    /// 座標と行優先の値から作る（長さが合わなければ panic: 作る側の誤り）
    pub fn new(x: Vec<f64>, y: Vec<f64>, data: Vec<T>) -> Self {
        assert_eq!(
            data.len(),
            x.len() * y.len(),
            "Grid2: {} values for {} × {}",
            data.len(),
            x.len(),
            y.len()
        );
        Grid2 { x, y, data }
    }

    /// 行の並び（`rows[iy][ix]`）から作る
    pub fn from_rows(x: Vec<f64>, y: Vec<f64>, rows: Vec<Vec<T>>) -> Self {
        assert!(
            rows.len() == y.len() && rows.iter().all(|r| r.len() == x.len()),
            "Grid2: rows do not match {} × {}",
            x.len(),
            y.len()
        );
        let data = rows.into_iter().flatten().collect();
        Grid2 { x, y, data }
    }

    /// x の標本の数（列の数）
    pub fn nx(&self) -> usize {
        self.x.len()
    }

    /// y の標本の数（行の数）
    pub fn ny(&self) -> usize {
        self.y.len()
    }

    /// x の座標の並び
    pub fn x(&self) -> &[f64] {
        &self.x
    }

    /// y の座標の並び
    pub fn y(&self) -> &[f64] {
        &self.y
    }

    /// 行優先の値（`data[iy * nx + ix]`）
    pub fn data(&self) -> &[T] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// (x, y, 行優先の値) に分ける
    pub fn into_parts(self) -> (Vec<f64>, Vec<f64>, Vec<T>) {
        (self.x, self.y, self.data)
    }

    /// 値 (ix, iy)
    pub fn get(&self, ix: usize, iy: usize) -> &T {
        assert!(ix < self.nx(), "Grid2: ix {ix} ≥ {}", self.nx());
        &self.data[iy * self.nx() + ix]
    }

    /// 行 iy（y = `y[iy]` の値の並び）
    pub fn row(&self, iy: usize) -> &[T] {
        let n = self.nx();
        &self.data[iy * n..(iy + 1) * n]
    }

    /// 行の並び（y の順）
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &[T]> + '_ {
        // nx = 0 でも ny 行を返す
        (0..self.ny()).map(move |iy| self.row(iy))
    }

    /// 値ごとに f をかけた同じ座標の格子
    pub fn map<U>(&self, f: impl FnMut(&T) -> U) -> Grid2<U> {
        Grid2 {
            x: self.x.clone(),
            y: self.y.clone(),
            data: self.data.iter().map(f).collect(),
        }
    }
}

/// 行 iy（`g[iy][ix]` で値を引ける）
impl<T> std::ops::Index<usize> for Grid2<T> {
    type Output = [T];
    fn index(&self, iy: usize) -> &[T] {
        self.row(iy)
    }
}

impl<T> std::ops::IndexMut<usize> for Grid2<T> {
    fn index_mut(&mut self, iy: usize) -> &mut [T] {
        let n = self.nx();
        &mut self.data[iy * n..(iy + 1) * n]
    }
}

impl<T: Clone> Grid2<T> {
    /// 行の並び（`rows[iy][ix]`）に写す
    pub fn to_rows(&self) -> Vec<Vec<T>> {
        self.rows().map(<[T]>::to_vec).collect()
    }
}

impl Grid2<f64> {
    /// 有限の値の最大（無ければ −∞）
    pub fn max(&self) -> f64 {
        self.data
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_flat_agree() {
        let g = Grid2::from_rows(
            vec![0.0, 1.0, 2.0],
            vec![10.0, 20.0],
            vec![vec![1, 2, 3], vec![4, 5, 6]],
        );
        assert_eq!((g.nx(), g.ny()), (3, 2));
        assert_eq!(g.data(), &[1, 2, 3, 4, 5, 6]);
        assert_eq!(*g.get(2, 1), 6);
        assert_eq!(g.row(1), &[4, 5, 6]);
        assert_eq!(g[1][2], 6);
        assert_eq!(g.to_rows(), vec![vec![1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(g.map(|v| v * 2).data(), &[2, 4, 6, 8, 10, 12]);
        let e: Grid2<f64> = Grid2::new(Vec::new(), vec![0.0, 1.0], Vec::new());
        assert_eq!(e.rows().len(), 2);
    }

    #[test]
    #[should_panic]
    fn wrong_length_panics() {
        Grid2::new(vec![0.0], vec![0.0, 1.0], vec![1.0]);
    }
}
