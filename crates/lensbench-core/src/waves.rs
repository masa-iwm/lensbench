//! 多色を受ける解析の波長の選び方（ANA-07）

/// 多色を受ける解析（スポット・MTF・PSF・RMS など）の波長。単色だけの解析（OPD・波面・Zernike・干渉縞）は
/// `Option<usize>` で、None は主波長
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waves {
    /// 全波長を重みで足した多色
    All,
    /// 波長 1 つ（0 始まりの番号）
    One(usize),
}

impl Waves {
    /// None を多色とする（CLI・serve・Python の「wave を省くと多色」の口）
    pub fn all_or(w: Option<usize>) -> Waves {
        w.map_or(Waves::All, Waves::One)
    }

    /// 波長 1 つなら Some
    pub fn one(self) -> Option<usize> {
        match self {
            Waves::All => None,
            Waves::One(w) => Some(w),
        }
    }

    /// 多色で重みのある波長が無ければ誤り（全点が NaN の掃引にしない、ANA-23）
    pub fn weighted(self, s: &crate::system::System) -> crate::error::Result<Waves> {
        if self == Waves::All && !s.weights.iter().any(|&w| w > 0.0) {
            crate::invalid!("重みのある波長がない");
        }
        Ok(self)
    }
}
