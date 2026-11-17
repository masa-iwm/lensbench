//! 多色を受ける解析の波長の選び方

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

    /// 多色で重みのある波長が無ければ誤り（全点が NaN の掃引にしない）
    pub fn weighted(self, s: &crate::system::System) -> crate::error::Result<Waves> {
        if self == Waves::All && !s.weights.iter().any(|&w| w > 0.0) {
            crate::invalid!("no weighted wavelength");
        }
        Ok(self)
    }
}

/// 利用者の 1 始まりの番号 n（CLI・serve・Python・プラグイン・評価関数の項）を、波長が nw 個の系の 0 始まりの番号にする。
/// 0・範囲外はどの口でも同じ誤り（docs/conventions.md）
pub fn index(n: usize, nw: usize) -> crate::Result<usize> {
    if n >= 1 && n <= nw {
        return Ok(n - 1);
    }
    Err(crate::Error::Invalid(format!(
        "no wavelength {n} (1 to {nw})"
    )))
}

/// 1 始まりの番号（None は省いた）を 0 始まりにする
pub fn index_opt(n: Option<usize>, nw: usize) -> crate::Result<Option<usize>> {
    n.map(|n| index(n, nw)).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_numbers_are_one_based() {
        assert_eq!(index(1, 3).unwrap(), 0);
        assert_eq!(index(3, 3).unwrap(), 2);
        for n in [0, 4] {
            let e = index(n, 3).unwrap_err();
            assert_eq!(
                e.message(),
                Some(format!("no wavelength {n} (1 to 3)").as_str())
            );
        }
        assert_eq!(index_opt(None, 1).unwrap(), None);
    }
}
