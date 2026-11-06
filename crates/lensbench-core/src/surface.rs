//! 面のデータ（OpticStudio のレンズデータエディタの 1 行に当たる）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfType {
    Standard,
    EvenAsph,
    CoordBrk,
    Jones,
    Paraxial,
}

/// 傾き・偏心（dx dy tx ty tz [deg]）と順序（0: 偏心→X→Y→Z、1: Z→Y→X→偏心）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tilt {
    pub d: [f64; 5],
    pub order: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum After {
    None,
    /// 「前」の変換を戻す
    Reverse,
    Tilt(Tilt),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Aperture {
    None,
    /// 最小・最大半径、偏心 x y
    Circular {
        min: f64,
        max: f64,
        dx: f64,
        dy: f64,
    },
    /// x・y の半幅、偏心
    Rect {
        hx: f64,
        hy: f64,
        dx: f64,
        dy: f64,
    },
    /// 自動の半径（既定では切らない）
    Floating {
        semi: f64,
    },
    /// 円形遮蔽（最小〜最大半径の間を遮る）
    Obsc {
        min: f64,
        max: f64,
        dx: f64,
        dy: f64,
    },
}

#[derive(Debug, Clone)]
pub struct Surface {
    pub typ: SurfType,
    pub ignore: bool,
    /// 次の面までの厚さ [mm]（物体面は無限遠なら inf）
    pub t: f64,
    pub mirror: bool,
    pub comment: String,
    /// 材料名（空なら空気、座標ブレークは "-"）
    pub mat: String,
    pub coat: String,
    pub stop: bool,
    pub semi: f64,
    pub c: f64,
    pub k: f64,
    /// 偶数次非球面の係数 r^2, r^4, …
    pub asph: Vec<f64>,
    pub cb: [f64; 5],
    pub cb_order: u8,
    pub focal: f64,
    pub opd_mode: i32,
    pub jones: [f64; 8],
    pub before: Option<Tilt>,
    pub after: After,
    pub ap: Aperture,
    /// 面の後の媒質の屈折率（波長ごと）
    pub n: Vec<f64>,
}

impl Default for Surface {
    fn default() -> Self {
        Surface {
            typ: SurfType::Standard,
            ignore: false,
            t: 0.0,
            mirror: false,
            comment: String::new(),
            mat: String::new(),
            coat: String::new(),
            stop: false,
            semi: 0.0,
            c: 0.0,
            k: 0.0,
            asph: Vec::new(),
            cb: [0.0; 5],
            cb_order: 0,
            focal: 0.0,
            opd_mode: 0,
            jones: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            before: None,
            after: After::None,
            ap: Aperture::None,
            n: Vec::new(),
        }
    }
}

impl Surface {
    pub fn has_asph(&self) -> bool {
        self.asph.iter().any(|&a| a != 0.0)
    }
}
