//! 系を玉（レンズ）・鏡・座標ブレーク・その他の面と、空気間隔に分ける（zmx の玉モード）。
//!
//! - 玉: 空気から入り、空気へ出るまでの面の連なり（貼り合わせも 1 玉。中に鏡があれば裏面鏡などの 1 玉）。
//!   最後の面の後の厚さは玉に含めず、続く空気間隔にする。
//! - 鏡: 前後とも空気の鏡面。
//! - 座標ブレーク、空気中のその他の面（ダミー面、理想レンズ、Jones 面など）は 1 面ずつ。
//! - 物体面と像面は分けない。物体面の厚さは系に残す。
//!
//! [`assemble`] は分けた並びを面の列に戻す。並び替え・反転・差し替えをした並びでもよい。
//! 組み直した系はピックアップ（[`crate::pickup::Pickup`]）・玉・群（[`crate::units`]）を引き継がない（玉は作り直す）。

use crate::action::Action;
use crate::error::Result;
use crate::invalid;
use crate::surface::{Aperture, Surface};
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
    /// 面の後の媒質の屈折率（面ごと・波長ごと、[`split`] で分けた系の値）。系から分けていない玉は空
    /// （[`Element::efl`] は 1 とみなす）
    pub n: Vec<Vec<f64>>,
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
            n: Vec::new(),
            t,
            reversed: false,
        }
    }

    /// 玉の材料の名前（面の後の媒質、最後の空気を除く）
    pub fn materials(&self) -> Vec<String> {
        let n = self.surfaces.len();
        self.surfaces[..n.saturating_sub(1)]
            .iter()
            .map(|s| s.mat.name())
            .collect()
    }

    /// 玉の中心厚の合計 [mm]
    pub fn length(&self) -> f64 {
        self.surfaces.iter().map(|s| s.t).sum()
    }

    /// 前後を空気とした玉だけの近軸の焦点距離（波長 w）。無限大なら inf、鏡を含む玉は NaN
    pub fn efl(&self, w: usize) -> f64 {
        if self.surfaces.iter().any(|s| s.is_mirror()) {
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
                self.n.get(i).and_then(|v| v.get(w)).copied().unwrap_or(1.0)
            };
            let p = match s.action() {
                _ if s.is_coord_break() => 0.0,
                Action::Paraxial { f, .. } => 1.0 / f,
                Action::User(u) => u.map_or(0.0, |u| u.instance().paraxial_power()),
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

fn is_air_mat(m: &crate::surface::Material) -> bool {
    m.is_air()
}

/// 面 j の後の媒質が空気か（鏡・座標ブレーク・無視する面は前の媒質を引き継ぐ）
pub fn air_after(s: &System) -> Vec<bool> {
    let mut out = Vec::with_capacity(s.surfaces.len());
    let mut prev = true;
    for (j, d) in s.surfaces.iter().enumerate() {
        let a = if j == 0 {
            is_air_mat(&d.mat)
        } else if d.is_mirror() || d.is_coord_break() || d.ignore {
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
        invalid!("a system whose object space is not air is not split into lenses");
    }
    let ima = s.image();
    if !air[ima - 1] {
        invalid!("a system whose image space is not air is not split into lenses");
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
        } else if d.is_coord_break() {
            (Kind::CoordBreak, j)
        } else if d.is_mirror() {
            (Kind::Mirror, j)
        } else {
            (Kind::Surface, j)
        };
        let mut v: Vec<Surface> = sf[j..=k].to_vec();
        let gap = v.last().expect("non-empty element").t;
        v.last_mut().expect("non-empty element").t = 0.0;
        out.push(Element {
            kind,
            from: j,
            surfaces: v,
            n: (j..=k).map(|i| s.n(i).to_vec()).collect(),
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

/// 面を前後反転できるか（曲率・非球面の係数の符号を変えるだけで済む形: 標準面・偶数次・奇数次非球面・トロイダル面・
/// Fresnel・Binary 2（位相は ρ だけの関数で、薄い位相の板の遅れは光の向きによらないので係数はそのまま）、GRIN でない媒質）。理想レンズは前後の媒質で
/// 屈折力が変わるので、玉の中にあれば反転しない（空気中の面の並びの復路では使える）。Zernike・Grid Sag・Irregular・Tilted・Cylinder Fresnel などは、反転に形ごとの
/// 変換が要るので反転しない
pub fn reversible(s: &Surface) -> bool {
    use crate::surface::SurfType as T;
    matches!(
        s.typ(),
        T::Standard | T::EvenAsph | T::OddAsph | T::Toroidal | T::Fresnel | T::Binary2
    ) && s.grin().is_none()
}

/// 玉を前後反転する（y 軸のまわりに 180° 回した向き: 曲率・非球面・x の偏心の符号が反転）
pub fn reverse(e: &Element) -> Result<Element> {
    if e.kind != Kind::Lens {
        invalid!("only a lens can be reversed");
    }
    if e.surfaces
        .iter()
        .any(|s| s.is_mirror() || s.is_coord_break())
    {
        notimpl_rev()?;
    }
    if e.surfaces
        .iter()
        .any(|s| s.before.is_some() || !matches!(s.after, crate::surface::After::None))
    {
        invalid!("a lens with surface tilts or decenters is not reversed");
    }
    let n = e.surfaces.len();
    let mut v = Vec::with_capacity(n);
    let mut nv = Vec::with_capacity(n);
    for i in 0..n {
        let mut s = e.surfaces[n - 1 - i].clone();
        s.c = -s.c;
        if let Some(cx) = s.cx_mut() {
            *cx = -*cx;
        }
        s.asph.iter_mut().for_each(|a| *a = -*a);
        if matches!(s.action(), Action::Jones(_)) {
            invalid!("a lens with a Jones surface is not reversed");
        }
        if !reversible(&s) {
            invalid!("a lens with a Zernike, grid sag, tilted, irregular, cylinder Fresnel or paraxial surface or a GRIN medium is not reversed");
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
            // x を反転すると腕の向き θ は π − θ になる。偶数本なら同じ並び、奇数本は最初の腕が −x になり表せない
            Aperture::Spider {
                width,
                arms,
                dx,
                dy,
            } => {
                if arms % 2 == 1 {
                    invalid!(
                        "a lens with a spider aperture of an odd number of arms is not reversed"
                    );
                }
                Aperture::Spider {
                    width,
                    arms,
                    dx: -dx,
                    dy,
                }
            }
            a => a,
        };
        // 面の後の媒質と厚さは、反転前の 1 つ前の面の後のもの
        if i + 1 < n {
            let q = &e.surfaces[n - 2 - i];
            s.t = q.t;
            s.mat = q.mat.clone();
            nv.extend(e.n.get(n - 2 - i).cloned());
        } else {
            let last = &e.surfaces[n - 1];
            s.t = 0.0;
            s.mat = last.mat.clone();
            nv.extend(e.n.get(n - 1).cloned());
        }
        v.push(s);
    }
    Ok(Element {
        surfaces: v,
        n: nv,
        reversed: !e.reversed,
        ..e.clone()
    })
}

fn notimpl_rev() -> Result<()> {
    crate::notimpl!("reversing a lens with a mirror or coordinate break")
}

/// 要素の並びを面の列に戻した系（物体面・像面と系のデータは base のもの）
pub fn assemble(base: &System, els: &[Element]) -> Result<System> {
    let mut sf: Vec<Surface> = vec![base.surfaces[0].clone()];
    for e in els {
        match e.kind {
            Kind::Air => {
                let last = sf.last_mut().expect("non-empty element");
                last.t += e.t;
            }
            _ => sf.extend(e.surfaces.iter().cloned()),
        }
    }
    sf.push(base.surfaces[base.image()].clone());
    let mut s = base.clone();
    s.rx_mut().surfaces = sf;
    // 面の番号が変わるので、ピックアップ・厚さのソルブは引き継がない（値はそのまま）
    s.rx_mut().pickups.clear();
    s.rx_mut().solves.clear();
    s.rx_mut().units.clear();
    if !s.surfaces.iter().any(|d| d.stop) {
        invalid!("the rebuilt system has no stop");
    }
    s.changed()?;
    s.rx_mut().units = s.lens_units();
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

    /// Binary 2 の面のある玉は反転でき、焦点距離が変わらない（位相の係数はそのまま）。Zernike・理想レンズの面のある玉は
    /// 反転しない
    #[test]
    fn binary2_lens_reverses_and_zernike_does_not() {
        let s = two_lenses();
        let e = split(&s).unwrap();
        let mut b = e[0].clone();
        b.surfaces[1].set_type(crate::surface::SurfType::Binary2);
        *b.surfaces[1].bin2_mut().unwrap() = Some(std::sync::Arc::new(crate::surface::Binary2 {
            order: 1.0,
            norm: 10.0,
            coef: vec![-20.0],
        }));
        // 系の焦点距離（近軸の追跡は位相の面の屈折力も入る）
        let efl = |x: &Element| {
            let mut x = x.clone();
            x.surfaces[0].stop = true;
            assemble(&s, &[x, e[3].clone()]).unwrap().paraxial().efl
        };
        let r = reverse(&b).unwrap();
        assert!((efl(&b) - efl(&e[0])).abs() > 1e-3 * efl(&e[0]).abs());
        assert!((efl(&r) - efl(&b)).abs() < 1e-9 * efl(&b).abs());
        assert_eq!(r.surfaces[0].bin2(), b.surfaces[1].bin2());
        // 理想レンズの面のある玉は反転しない（前後の媒質で屈折力が変わる）
        let mut q = e[0].clone();
        q.surfaces[0].form = crate::surface::Form::Paraxial {
            focal: 80.0,
            opd_mode: 0,
        };
        assert!(reverse(&q).is_err());
        let mut z = e[0].clone();
        z.surfaces[0].set_type(crate::surface::SurfType::ZernikeStd);
        assert!(reverse(&z).is_err());
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
                assert_eq!((x.c, x.t, &x.mat), (y.c, y.t, &y.mat));
            }
            assert_eq!(rr.n, e[i].n);
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
