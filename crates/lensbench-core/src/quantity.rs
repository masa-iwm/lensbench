//! 取り違えやすい 2 つの量の型: 波長 [`Wavelength`]（系は µm で持ち、光路長・位相は mm で計算する）と
//! 角度 [`Angle`]（入出力は deg、三角関数は rad）。
//!
//! 長さはどこでも mm なので素の f64 のまま。波長と角度だけは同じ f64 に 2 つの単位が混ざり、`* 1e-3` や
//! `to_radians()` を書き忘れても型では止まらないので、変換をこの型の名前のある関数に集める。

use std::f64::consts::PI;

/// 波長（中身は µm）
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Wavelength(f64);

impl Wavelength {
    /// µm で与える（系の波長・.zmx・CLI の単位）
    pub const fn from_um(um: f64) -> Self {
        Self(um)
    }

    /// mm で与える
    pub fn from_mm(mm: f64) -> Self {
        Self(mm * 1e3)
    }

    /// µm
    pub const fn um(self) -> f64 {
        self.0
    }

    /// mm（光路長と同じ単位。光路長 / これ = 波の数）
    pub fn mm(self) -> f64 {
        self.0 * 1e-3
    }

    /// 真空の波数 2π/λ [rad/mm]
    pub fn k_mm(self) -> f64 {
        2.0 * PI / self.mm()
    }
}

/// 角度（中身は rad）
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct Angle(f64);

impl Angle {
    /// deg で与える（系の視野・傾き・入出力の単位）
    pub fn from_deg(deg: f64) -> Self {
        Self(deg.to_radians())
    }

    /// rad で与える
    pub const fn from_rad(rad: f64) -> Self {
        Self(rad)
    }

    pub fn deg(self) -> f64 {
        self.0.to_degrees()
    }

    pub const fn rad(self) -> f64 {
        self.0
    }

    pub fn sin_cos(self) -> (f64, f64) {
        self.0.sin_cos()
    }

    pub fn cos(self) -> f64 {
        self.0.cos()
    }

    pub fn tan(self) -> f64 {
        self.0.tan()
    }
}

impl crate::system::System {
    /// 波長 w（0 から）。範囲の外なら panic（`wavelengths[w]` と同じ）
    pub fn wave(&self, w: usize) -> Wavelength {
        Wavelength::from_um(self.wavelengths[w])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wavelength_units() {
        let w = Wavelength::from_um(0.5875618);
        assert_eq!(w.um(), 0.5875618);
        assert!((w.mm() - 0.5875618e-3).abs() < 1e-18);
        assert!((Wavelength::from_mm(w.mm()).um() - w.um()).abs() < 1e-15);
        assert!((w.k_mm() * w.mm() - 2.0 * PI).abs() < 1e-12);
    }

    #[test]
    fn angle_units() {
        let a = Angle::from_deg(30.0);
        assert!((a.rad() - PI / 6.0).abs() < 1e-15);
        assert!((a.deg() - 30.0).abs() < 1e-12);
        assert!((a.sin_cos().0 - 0.5).abs() < 1e-15);
        assert!((Angle::from_deg(45.0).tan() - 1.0).abs() < 1e-15);
    }
}
