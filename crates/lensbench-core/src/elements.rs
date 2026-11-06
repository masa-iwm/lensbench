//! 系を玉（レンズ）・鏡・座標ブレーク・その他の面と、空気間隔に分ける（zmx の玉モード）。
//!
//! - 玉: 空気から入り、空気へ出るまでの面の連なり（貼り合わせも 1 玉。中に鏡があれば裏面鏡などの 1 玉）。
//!   最後の面の後の厚さは玉に含めず、続く空気間隔にする。
//! - 鏡: 前後とも空気の鏡面。
//! - 座標ブレーク、空気中のその他の面（ダミー面、理想レンズ、Jones 面など）は 1 面ずつ。
//! - 物体面と像面は分けない。物体面の厚さは系に残す。
//!
//! [`assemble`] は分けた並びを面の列に戻す。並び替え・反転・差し替えをした並びでもよい。
//! 組み直した系はピックアップ（[`crate::param::Pickup`]）・玉・群（[`crate::units`]）を引き継がない（玉は作り直す）。

use crate::error::Result;
use crate::invalid;
use crate::surface::{Aperture, SurfType, Surface};
use crate::system::System;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Lens,
    Mirror,
    CoordBreak,
    /// 空気中の、上のどれでもない面
    Surface,
    /// 空気間隔（直前の要素の最後の面の厚さ）
    Air,
}

#[derive(Debug, Clone)]
pub struct Element {
    pub kind: Kind,
    /// 元の系での面番号（組み直した並びでは参考）
    pub from: usize,
    /// 面（玉なら最後の面の厚さは 0）。空気間隔は空
    pub surfaces: Vec<Surface>,
    /// 空気間隔の厚さ [mm]
    pub t: f64,
    /// 反転してあるか
    pub reversed: bool,
}

impl Element {
    pub fn air(t: f64) -> Element {
        Element {
            kind: Kind::Air,
            from: 0,
            surfaces: Vec::new(),
            t,
            reversed: false,
        }
    }

    /// 玉の材料（面の後の媒質、最後の空気を除く）
    pub fn materials(&self) -> Vec<&str> {
        let n = self.surfaces.len();
        self.surfaces[..n.saturating_sub(1)]
            .iter()
            .map(|s| s.mat.as_str())
            .collect()
    }

    /// 玉の中心厚の合計 [mm]
    pub fn length(&self) -> f64 {
        self.surfaces.iter().map(|s| s.t).sum()
    }

    /// 前後を空気とした玉だけの近軸の焦点距離（波長 w）。無限大なら inf、鏡を含む玉は NaN
    pub fn efl(&self, w: usize) -> f64 {
        if self.surfaces.iter().any(|s| s.mirror) {
            return f64::NAN;
        }
        // [y; n u] の行列
        let mut m = [[1.0, 0.0], [0.0, 1.0]];
        let mut n0 = 1.0;
        let last = self.surfaces.len() - 1;
        for (i, s) in self.surfaces.iter().enumerate() {
            let n1 = if i == last {
                1.0
            } else {
                s.n.get(w).copied().unwrap_or(1.0)
            };
            let p = match s.typ {
                SurfType::Paraxial => 1.0 / s.focal,
                SurfType::User => s
                    .user
                    .as_ref()
                    .map_or(0.0, |u| u.instance().paraxial_power()),
                SurfType::CoordBrk => 0.0,
                _ => (n1 - n0) * s.c,
            };
            m = mul(&[[1.0, 0.0], [-p, 1.0]], &m);
            if i < last {
                m = mul(&[[1.0, s.t / n1], [0.0, 1.0]], &m);
            }
            n0 = n1;
        }
        if m[1][0] == 0.0 {
            f64::INFINITY
        } else {
            -1.0 / m[1][0]
        }
    }
}

fn mul(a: &[[f64; 2]; 2], b: &[[f64; 2]; 2]) -> [[f64; 2]; 2] {
    [
        [
            a[0][0] * b[0][0] + a[0][1] * b[1][0],
            a[0][0] * b[0][1] + a[0][1] * b[1][1],
        ],
        [
            a[1][0] * b[0][0] + a[1][1] * b[1][0],
            a[1][0] * b[0][1] + a[1][1] * b[1][1],
        ],
    ]
}

fn is_air_mat(m: &str) -> bool {
    m.is_empty() || m.eq_ignore_ascii_case("AIR")
}

/// 面 j の後の媒質が空気か（鏡・座標ブレーク・無視する面は前の媒質を引き継ぐ）
pub fn air_after(s: &System) -> Vec<bool> {
    let mut out = Vec::with_capacity(s.surfaces.len());
    let mut prev = true;
    for (j, d) in s.surfaces.iter().enumerate() {
        let a = if j == 0 {
            is_air_mat(&d.mat)
        } else if d.mirror || d.typ == SurfType::CoordBrk || d.ignore {
            prev
        } else {
            is_air_mat(&d.mat)
        };
        out.push(a);
        prev = a;
    }
    out
}

/// 面 1 から像面の手前までを要素に分ける
pub fn split(s: &System) -> Result<Vec<Element>> {
    let air = air_after(s);
    if !air[0] {
        invalid!("物体空間が空気でない系は玉に分けない");
    }
    let ima = s.image();
    if !air[ima - 1] {
        invalid!("像空間が空気でない系は玉に分けない");
    }
    let sf = &s.surfaces;
    let mut out = Vec::new();
    let mut j = 1;
    while j < ima {
        let d = &sf[j];
        let (kind, k) = if !air[j] {
            // 空気へ出る面まで
            let mut k = j;
            while !air[k] {
                k += 1;
            }
            (Kind::Lens, k)
        } else if d.typ == SurfType::CoordBrk {
            (Kind::CoordBreak, j)
        } else if d.mirror {
            (Kind::Mirror, j)
        } else {
            (Kind::Surface, j)
        };
        let mut v: Vec<Surface> = sf[j..=k].to_vec();
        let gap = v.last().unwrap().t;
        v.last_mut().unwrap().t = 0.0;
        out.push(Element {
            kind,
            from: j,
            surfaces: v,
            t: 0.0,
            reversed: false,
        });
        if gap != 0.0 {
            out.push(Element::air(gap));
        }
        j = k + 1;
    }
    Ok(out)
}

/// 玉を前後反転する（y 軸のまわりに 180° 回した向き: 曲率・非球面・x の偏心の符号が反転）
pub fn reverse(e: &Element) -> Result<Element> {
    if e.kind != Kind::Lens {
        invalid!("反転できるのは玉だけ");
    }
    if e.surfaces
        .iter()
        .any(|s| s.mirror || s.typ == SurfType::CoordBrk)
    {
        notimpl_rev()?;
    }
    if e.surfaces
        .iter()
        .any(|s| s.before.is_some() || !matches!(s.after, crate::surface::After::None))
    {
        invalid!("面の傾き・偏心がある玉は反転しない");
    }
    let n = e.surfaces.len();
    let mut v = Vec::with_capacity(n);
    for i in 0..n {
        let mut s = e.surfaces[n - 1 - i].clone();
        s.c = -s.c;
        s.cx = -s.cx;
        s.asph.iter_mut().for_each(|a| *a = -*a);
        if s.typ == SurfType::Jones {
            invalid!("Jones 面のある玉は反転しない");
        }
        s.ap = match s.ap {
            Aperture::Circular { min, max, dx, dy } => Aperture::Circular {
                min,
                max,
                dx: -dx,
                dy,
            },
            Aperture::Rect { hx, hy, dx, dy } => Aperture::Rect {
                hx,
                hy,
                dx: -dx,
                dy,
            },
            Aperture::Ellipse { hx, hy, dx, dy } => Aperture::Ellipse {
                hx,
                hy,
                dx: -dx,
                dy,
            },
            Aperture::Obsc { min, max, dx, dy } => Aperture::Obsc {
                min,
                max,
                dx: -dx,
                dy,
            },
            a => a,
        };
        // 面の後の媒質と厚さは、反転前の 1 つ前の面の後のもの
        if i + 1 < n {
            let q = &e.surfaces[n - 2 - i];
            s.t = q.t;
            s.mat = q.mat.clone();
            s.n = q.n.clone();
        } else {
            let last = &e.surfaces[n - 1];
            s.t = 0.0;
            s.mat = last.mat.clone();
            s.n = last.n.clone();
        }
        v.push(s);
    }
    Ok(Element {
        surfaces: v,
        reversed: !e.reversed,
        ..e.clone()
    })
}

fn notimpl_rev() -> Result<()> {
    crate::notimpl!("鏡・座標ブレークを含む玉の反転")
}

/// 要素の並びを面の列に戻した系（物体面・像面と系のデータは base のもの）
pub fn assemble(base: &System, els: &[Element]) -> Result<System> {
    let mut sf: Vec<Surface> = vec![base.surfaces[0].clone()];
    for e in els {
        match e.kind {
            Kind::Air => {
                let last = sf.last_mut().unwrap();
                last.t += e.t;
            }
            _ => sf.extend(e.surfaces.iter().cloned()),
        }
    }
    sf.push(base.surfaces[base.image()].clone());
    let mut s = base.clone();
    s.surfaces = sf;
    // 面の番号が変わるので、ピックアップ・厚さのソルブは引き継がない（値はそのまま）
    s.pickups.clear();
    s.solves.clear();
    s.units.clear();
    if !s.surfaces.iter().any(|d| d.stop) {
        invalid!("組み直した系に絞りがない");
    }
    s.changed()?;
    s.units = s.lens_units();
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys(txt: &str) -> System {
        let raw = crate::zmx::parse_text(txt).unwrap();
        crate::zmx::build(&raw, &Default::default(), &[], None).unwrap()
    }

    const HEAD: &str =
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
                        XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
                        SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";

    /// 単玉（モデルガラス nd 1.5168）、空気 5、貼り合わせ（1.5168 / 1.7）、像面
    fn two_lenses() -> System {
        let t = format!(
            "{HEAD}SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n\
             SURF 2\n  TYPE STANDARD\n  CURV -0.005\n  DISZ 5\n\
             SURF 3\n  TYPE STANDARD\n  CURV 0.015\n  DISZ 3\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n\
             SURF 4\n  TYPE STANDARD\n  CURV -0.03\n  DISZ 2\n  GLAS ___BLANK 1 0 1.7 0 0 0 0 0 0\n\
             SURF 5\n  TYPE STANDARD\n  CURV -0.004\n  DISZ 80\n\
             SURF 6\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
        );
        sys(&t)
    }

    #[test]
    fn a_cemented_doublet_is_one_lens() {
        let s = two_lenses();
        let e = split(&s).unwrap();
        let kinds: Vec<Kind> = e.iter().map(|x| x.kind).collect();
        assert_eq!(kinds, vec![Kind::Lens, Kind::Air, Kind::Lens, Kind::Air]);
        assert_eq!(e[2].surfaces.len(), 3);
        assert_eq!((e[1].t, e[3].t), (5.0, 80.0));
        // 組み直すと元と同じ面の列
        let a = assemble(&s, &e).unwrap();
        for (x, y) in a.surfaces.iter().zip(&s.surfaces) {
            assert_eq!((x.c, x.t, &x.mat), (y.c, y.t, &y.mat));
        }
        assert_eq!(a.paraxial().efl, s.paraxial().efl);
    }

    #[test]
    fn a_reversed_lens_keeps_its_focal_length() {
        let s = two_lenses();
        let e = split(&s).unwrap();
        for i in [0, 2] {
            let r = reverse(&e[i]).unwrap();
            assert!((r.efl(0) - e[i].efl(0)).abs() < 1e-9 * e[i].efl(0).abs());
            assert_eq!(r.length(), e[i].length());
            let rr = reverse(&r).unwrap();
            for (x, y) in rr.surfaces.iter().zip(&e[i].surfaces) {
                assert_eq!((x.c, x.t, &x.mat, &x.n), (y.c, y.t, &y.mat, &y.n));
            }
        }
        // 単玉の焦点距離（厚肉の式）
        let (n, c1, c2, d): (f64, f64, f64, f64) = (1.5168, 0.02, -0.005, 4.0);
        let p = (n - 1.0) * (c1 - c2) + (n - 1.0).powi(2) * d * c1 * c2 / n;
        assert!((e[0].efl(0) - 1.0 / p).abs() < 1e-9);
        // 玉を入れ替えて反転した系は組める（絞りは最初の玉の前面のまま付いていく）
        let mut v = vec![
            reverse(&e[2]).unwrap(),
            e[1].clone(),
            e[0].clone(),
            e[3].clone(),
        ];
        v[0].surfaces[0].stop = false;
        v[2].surfaces[0].stop = true;
        let a = assemble(&s, &v).unwrap();
        assert_eq!(a.surfaces.len(), s.surfaces.len());
        assert!(a.paraxial().efl.is_finite());
    }
}
