//! 子午・球欠の組（ANA-09）
/// 子午（像面の y 方向の周波数）と球欠（x 方向）の組。順を取り違えないよう名前で持つ（ANA-09）
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TanSag<T> {
    pub tan: T,
    pub sag: T,
}

impl<T> TanSag<T> {
    pub fn new(tan: T, sag: T) -> Self {
        TanSag { tan, sag }
    }
}

impl TanSag<f64> {
    /// 小さいほう（既定の周波数の上限などに使う）
    pub fn min(self) -> f64 {
        self.tan.min(self.sag)
    }
}
