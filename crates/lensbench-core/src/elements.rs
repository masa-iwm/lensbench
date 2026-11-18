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

/// 面を前後反転できるか（[`Surface::flip_z`] と [`Surface::mirror_x`] で形を y 軸のまわりに 180° 回せる種類: 理想レンズ・
/// GRIN・Jones・座標ブレーク・利用者定義の面のほか全部）。Binary 2 の位相は ρ だけの関数で、薄い位相の板の遅れは光の向きに
/// よらないので係数はそのまま。理想レンズは前後の媒質で屈折力が変わるので、玉の中にあれば反転しない（空気中の面の並びの
/// 復路では使える）
pub fn reversible(s: &Surface) -> bool {
    use crate::surface::SurfType as T;
    matches!(
        s.typ(),
        T::Standard
            | T::EvenAsph
            | T::OddAsph
            | T::Toroidal
            | T::Fresnel
            | T::Binary2
            | T::ZernikeStd
            | T::ZernikeFringe
            | T::GridSag
            | T::CylFresnel
            | T::TiltSurf
            | T::Irregular
            | T::Gradium
            | T::Gradient9
    )
}

/// 玉を前後反転する（y 軸のまわりに 180° 回した向き: 形は [`Surface::flip_z`] と [`Surface::mirror_x`]、開口の
/// x の偏心の符号が反転）
pub fn reverse(e: &Element) -> Result<Element> {
    reverse_with(e, true)
}

/// 同じ玉を鏡の後の座標（z を逆に向けた座標、厚さが負）で、光が戻る順に書く（復路）: 面の順を逆にし、形は
/// [`Surface::flip_z`] だけ（x はそのまま、開口も動かさない）。鏡の後に面を足すときの裏返しと合わせて、往路と同じ形になる
pub fn reverse_folded(e: &Element) -> Result<Element> {
    reverse_with(e, false)
}

fn reverse_with(e: &Element, mirror: bool) -> Result<Element> {
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
        if matches!(s.action(), Action::Jones(_)) {
            invalid!("a lens with a Jones surface is not reversed");
        }
        // 利用者定義の面はモデルが裏返しに対応すれば反転できる（対応しなければ flip_z・mirror_x の誤り）
        if !reversible(&s) && s.typ() != crate::surface::SurfType::User {
            invalid!("a lens with a paraxial surface is not reversed");
        }
        if s.bire.is_some() {
            invalid!("a lens with a birefringent plate is not reversed");
        }
        s.flip_z()?;
        if mirror {
            s.mirror_x()?;
            // 格子は映したので、元の .DAT とは違う
            if let Some(f) = s.grid_file_mut() {
                f.clear();
            }
        }
        if mirror {
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
        }
        // 面の後の媒質と厚さは、反転前の 1 つ前の面の後のもの。屈折率分布も媒質なので、1 つ前の面から移す
        move_grin(&mut s, e, n, i, mirror)?;
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

/// 反転した玉の i 番目の面 s（元の n − 1 − i 番目、裏返し済み）に、後ろの媒質の屈折率分布を移す。媒質は元の
/// 1 つ前の面 q の後ろにあったもの: q の局所座標で裏返し、q の厚さだけずらして s の局所座標に置く。形の傾き
/// （Gradient 9 の tan）は面の形なので s のものを残す
fn move_grin(s: &mut Surface, e: &Element, n: usize, i: usize, mirror: bool) -> Result<()> {
    use crate::grin::GrinKind;
    use crate::surface::SurfType as T;
    let unresolved =
        |q: &Surface| matches!(q.typ(), T::Gradium | T::Gradient9) && q.grin().is_none();
    let medium = if i + 1 < n {
        let q = &e.surfaces[n - 2 - i];
        if unresolved(q) {
            invalid!("a lens with a GRIN surface without a profile is not reversed");
        }
        q.grin().map(|g| {
            let mut g = g.clone();
            g.flip_z();
            if mirror {
                g.mirror_x();
            }
            if let GrinKind::Gradium { dz, .. } = &mut g.kind {
                // 分布は z′ + Δz で決まる: q の局所座標 z は s の局所座標 z − t
                *dz -= q.t;
            }
            g
        })
    } else {
        let last = &e.surfaces[n - 1];
        if last.grin().is_some() || unresolved(last) {
            invalid!("a lens with a GRIN medium after its last surface is not reversed");
        }
        None
    };
    if unresolved(s) {
        invalid!("a lens with a GRIN surface without a profile is not reversed");
    }
    // 面の形の傾き（裏返し済み）。Tilted は平面なので、曲率・コーニックは形に効かない
    let (tan, plane) = match (s.typ(), s.grin().map(|g| &g.kind)) {
        (T::TiltSurf, _) => ([s.asph()[0], s.asph()[1]], true),
        (_, Some(GrinKind::Grad9 { tan, .. })) => (*tan, false),
        _ => ([0.0; 2], false),
    };
    let tilted = tan != [0.0; 2];
    match medium {
        Some(mut g) => {
            if !matches!(
                s.typ(),
                T::Standard | T::Gradium | T::Gradient9 | T::TiltSurf
            ) {
                invalid!(
                    "a lens whose GRIN medium ends at a surface other than a standard or tilted one is not reversed"
                );
            }
            match &mut g.kind {
                GrinKind::Grad9 { tan: t, .. } => *t = tan,
                GrinKind::Gradium { .. } if tilted => {
                    invalid!("a lens whose Gradium medium ends at a tilted surface is not reversed")
                }
                GrinKind::Gradium { .. } => {}
            }
            if plane {
                s.c = 0.0;
                s.k = 0.0;
            }
            s.set_type(g.surf_type());
            *s.grin_mut().expect("GRIN surface") = Some(std::sync::Arc::new(g));
        }
        None if s.grin().is_some() => {
            if let Some(g) = s.grin_mut() {
                *g = None;
            }
            if !tilted {
                s.set_type(T::Standard);
            } else if s.c == 0.0 {
                // 傾けた平面は Tilted で書ける
                s.set_type(T::TiltSurf);
                s.set_asph(tan.to_vec());
            } else {
                invalid!(
                    "a lens with a tilted curved Gradient 9 surface after a homogeneous medium is not reversed"
                );
            }
        }
        None => {}
    }
    Ok(())
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
    use std::sync::Arc;

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
        // GRIN の媒質のある玉も反転しない（光の向きで屈折率分布の向きが変わる）
        let mut g = e[0].clone();
        g.surfaces[0].set_type(crate::surface::SurfType::Gradium);
        assert!(reverse(&g).is_err());
    }

    /// 形の違う面の例（Zernike Standard・Fringe・Grid Sag・Tilted・Irregular・Cylinder Fresnel）。どれも x について
    /// 対称でない
    fn freeform_surfaces() -> Vec<Surface> {
        use crate::freeform::{GridSag, ZernSag};
        use crate::surface::SurfType as T;
        use std::sync::Arc;
        let zern = |fringe: bool| {
            let coef: Vec<f64> = (1..=28)
                .map(|j| 1e-3 * ((j * 7 % 11) as f64 - 5.0))
                .collect();
            let mut z = ZernSag::new(fringe, 8.0, coef, true);
            z.dx = 0.7;
            z.dy = -0.4;
            z.rebuild();
            Arc::new(z)
        };
        let mut v = Vec::new();
        for (t, fringe) in [(T::ZernikeStd, false), (T::ZernikeFringe, true)] {
            let mut s = Surface {
                c: 0.02,
                k: -0.5,
                semi: 6.0,
                ..Default::default()
            };
            s.set_type(t);
            s.set_asph(vec![0.0, 1e-5, -2e-7]);
            *s.zern_mut().unwrap() = Some(zern(fringe));
            v.push(s);
        }
        // 格子: 双三次（微分も使う）、z = 0.01 x + 0.002 x y + 0.001 x² y の値と微分。値のない点は無し
        let (nx, ny, d) = (9usize, 7usize, 1.5);
        let mut g = GridSag {
            nx,
            ny,
            dx: d,
            dy: d,
            xdec: 0.4,
            ydec: -0.3,
            z: Vec::new(),
            nodata: vec![false; nx * ny],
            linear: false,
        };
        for iy in 0..ny {
            for ix in 0..nx {
                let x = (ix as f64 - (nx - 1) as f64 / 2.0) * d;
                let y = (iy as f64 - (ny - 1) as f64 / 2.0) * d;
                g.z.push([
                    0.01 * x + 0.002 * x * y + 0.001 * x * x * y,
                    0.01 + 0.002 * y + 0.002 * x * y,
                    0.002 * x + 0.001 * x * x,
                    0.002 + 0.002 * x,
                ]);
            }
        }
        let mut s = Surface {
            c: -0.01,
            semi: 5.0,
            ..Default::default()
        };
        s.set_type(T::GridSag);
        s.set_asph(vec![0.0, 2e-5]);
        *s.grid_mut().unwrap() = Some(Arc::new(g));
        *s.grid_file_mut().unwrap() = "a.dat".into();
        *s.zern_mut().unwrap() = Some(zern(false));
        v.push(s);
        let mut s = Surface::default();
        s.set_type(T::TiltSurf);
        s.set_asph(vec![0.03, -0.02]);
        v.push(s);
        let mut s = Surface {
            c: 0.015,
            k: 0.3,
            semi: 7.0,
            ..Default::default()
        };
        s.set_type(T::Irregular);
        s.set_asph(vec![0.3, -0.2, 1.5, -2.5, 0.02, -0.03, 0.015, 25.0]);
        v.push(s);
        let mut s = Surface {
            c: 0.01,
            semi: 6.0,
            ..Default::default()
        };
        s.set_type(T::CylFresnel);
        s.set_asph(vec![1e-4, 1e-6]);
        *s.cylf_mut().unwrap() = Some(Arc::new(crate::surface::CylFresnel {
            c: 0.03,
            k: -1.0,
            coef: vec![1e-5],
        }));
        v.push(s);
        v
    }

    #[test]
    fn freeform_surfaces_turn_around_the_y_axis() {
        use crate::freeform::sag_xy;
        for s in freeform_surfaces() {
            assert!(reversible(&s), "{:?}", s.typ());
            let mut r = s.clone();
            r.flip_z().unwrap();
            r.mirror_x().unwrap();
            let mut f = s.clone();
            f.flip_z().unwrap();
            for &(x, y) in &[(1.3, 0.7), (-2.1, 1.9), (0.4, -3.2), (-3.3, -1.1)] {
                let (z, zx, zy) = sag_xy(&s, -x, y);
                let (zr, zrx, zry) = sag_xy(&r, x, y);
                let tol = 1e-12 * (1.0 + z.abs());
                // 回した形 z'(x, y) = −z(−x, y)
                assert!((zr + z).abs() < tol, "{:?} {zr} {z}", s.typ());
                assert!((zrx - zx).abs() < 1e-9, "{:?} {zrx} {zx}", s.typ());
                assert!((zry + zy).abs() < 1e-9, "{:?} {zry} {zy}", s.typ());
                // z について裏返した形 z'(x, y) = −z(x, y)
                let (z, zx, zy) = sag_xy(&s, x, y);
                let (zf, zfx, zfy) = sag_xy(&f, x, y);
                assert!((zf + z).abs() < tol && (zfx + zx).abs() < 1e-9 && (zfy + zy).abs() < 1e-9);
            }
            if let Some(c) = s.cylf() {
                let rc = r.cylf().unwrap();
                assert_eq!((rc.c, rc.coef[0]), (-c.c, -c.coef[0]));
            }
            // 2 回で元に戻る
            r.flip_z().unwrap();
            r.mirror_x().unwrap();
            assert_eq!(r, s);
        }
    }

    #[test]
    fn a_freeform_lens_is_reversed_and_folded_back() {
        let s = two_lenses();
        let e = split(&s).unwrap();
        for f in freeform_surfaces() {
            let mut l = e[0].clone();
            let (t, mat) = (l.surfaces[0].t, l.surfaces[0].mat.clone());
            l.surfaces[0] = Surface {
                t,
                mat,
                ..f.clone()
            };
            let r = reverse(&l).unwrap();
            // 格子は映したので .DAT の名前は消す
            assert_eq!(r.surfaces[1].grid_file(), "");
            assert_eq!(r.surfaces[1].typ(), f.typ());
            let rr = reverse(&r).unwrap();
            assert_eq!(rr.surfaces[0].zern(), l.surfaces[0].zern());
            assert_eq!(rr.surfaces[0].grid(), l.surfaces[0].grid());
            assert_eq!(rr.surfaces[0].asph(), l.surfaces[0].asph());
            // 復路: z について裏返すだけ。鏡の後の裏返しをもう一度掛けると往路と同じ形
            let mut b = reverse_folded(&l).unwrap();
            b.surfaces[1].flip_z().unwrap();
            assert_eq!(b.surfaces[1].zern(), l.surfaces[0].zern());
            assert_eq!(b.surfaces[1].grid(), l.surfaces[0].grid());
            assert_eq!(b.surfaces[1].asph(), l.surfaces[0].asph());
            assert_eq!(b.surfaces[1].grid_file(), l.surfaces[0].grid_file());
        }
    }

    fn lens(surfaces: Vec<Surface>) -> Element {
        Element {
            kind: Kind::Lens,
            from: 0,
            surfaces,
            n: Vec::new(),
            t: 0.0,
            reversed: false,
        }
    }

    fn grin_surface(t: crate::surface::SurfType, c: f64, th: f64, g: crate::grin::Grin) -> Surface {
        let mut s = Surface {
            c,
            t: th,
            ..Default::default()
        };
        s.set_type(t);
        *s.grin_mut().unwrap() = Some(Arc::new(g));
        s
    }

    /// GRIN の玉の反転: 媒質は後ろの面に移り、反転した玉の屈折率は元の玉を y 軸のまわりに 180° 回した点（復路は
    /// z だけ映した点）のもの。2 回反転すると元に戻る
    #[test]
    fn a_grin_lens_is_reversed_with_its_medium() {
        use crate::grin::tests::gradium;
        use crate::surface::SurfType as T;
        let th = 4.0;
        let e = lens(vec![
            grin_surface(T::Gradium, 0.02, th, gradium([0.1, -0.05, 0.01, 0.02])),
            Surface {
                c: -0.01,
                ..Default::default()
            },
        ]);
        let g = e.surfaces[0].grin().unwrap();
        let r = reverse(&e).unwrap();
        assert_eq!(r.surfaces[0].typ(), T::Gradium);
        assert_eq!((r.surfaces[0].c, r.surfaces[0].t), (0.01, th));
        assert_eq!((r.surfaces[1].typ(), r.surfaces[1].c), (T::Standard, -0.02));
        assert!(r.surfaces[1].grin().is_none());
        let b = reverse_folded(&e).unwrap();
        let rr = reverse(&r).unwrap();
        let (gr, gb, grr) = (
            r.surfaces[0].grin().unwrap(),
            b.surfaces[0].grin().unwrap(),
            rr.surfaces[0].grin().unwrap(),
        );
        assert_eq!((rr.surfaces[0].c, rr.surfaces[1].c), (0.02, -0.01));
        for p in [[0.4, -0.3, 1.2], [-0.2, 0.5, 2.5], [0.1, 0.1, 3.6]] {
            for lam in [0.48, 0.55, 0.65] {
                let (n, d) = g.eval(lam, p).unwrap();
                let (nr, dr) = gr.eval(lam, [-p[0], p[1], th - p[2]]).unwrap();
                assert!((n - nr).abs() < 1e-12, "{p:?} {n} {nr}");
                assert!((d[0] + dr[0]).abs() < 1e-10 && (d[1] - dr[1]).abs() < 1e-10);
                assert!((d[2] + dr[2]).abs() < 1e-10, "{d:?} {dr:?}");
                let (nb, db) = gb.eval(lam, [p[0], p[1], th - p[2]]).unwrap();
                assert!((n - nb).abs() < 1e-12 && (d[0] - db[0]).abs() < 1e-10);
                let (n2, d2) = grr.eval(lam, p).unwrap();
                assert!((n - n2).abs() < 1e-12 && (d[2] - d2[2]).abs() < 1e-10);
            }
        }
        // 最後の面の後ろ（玉の外）の分布は移す先がない
        let mut bad = e.clone();
        bad.surfaces[1] = grin_surface(T::Gradium, -0.01, 0.0, gradium([0.0; 4]));
        assert!(reverse(&bad).is_err());
    }

    /// Gradient 9 の面の形の傾きは面に残る: 傾けた平面は Tilted と行き来し、傾けた曲面の後ろが一様な媒質になる
    /// 反転は書けない
    #[test]
    fn a_gradient9_lens_keeps_the_surface_tilts() {
        use crate::grin::{tests::g9, GrinKind};
        use crate::surface::SurfType as T;
        let tilted = |c: f64, tan: [f64; 2]| {
            let mut g = g9();
            if let GrinKind::Grad9 { tan: t, .. } = &mut g.kind {
                *t = tan;
            }
            grin_surface(T::Gradient9, c, 5.0, g)
        };
        let mut s1 = Surface {
            c: 0.3,
            ..Default::default()
        };
        s1.set_type(T::TiltSurf);
        s1.set_asph(vec![0.03, 0.01]);
        let e = lens(vec![tilted(0.0, [0.01, -0.02]), s1]);
        let tan = |s: &Surface| match s.grin().map(|g| &g.kind) {
            Some(GrinKind::Grad9 { tan, .. }) => *tan,
            _ => panic!("{:?}", s.typ()),
        };
        let r = reverse(&e).unwrap();
        assert_eq!(r.surfaces[0].typ(), T::Gradient9);
        assert_eq!(tan(&r.surfaces[0]), [0.03, -0.01]);
        assert_eq!((r.surfaces[0].c, r.surfaces[0].t), (0.0, 5.0));
        assert_eq!(r.surfaces[1].typ(), T::TiltSurf);
        assert_eq!(r.surfaces[1].asph(), &[0.01, 0.02]);
        let rr = reverse(&r).unwrap();
        assert_eq!(rr.surfaces[0].typ(), T::Gradient9);
        assert_eq!(tan(&rr.surfaces[0]), [0.01, -0.02]);
        assert_eq!(rr.surfaces[1].typ(), T::TiltSurf);
        assert_eq!(rr.surfaces[1].asph(), &[0.03, 0.01]);
        assert_eq!(rr.surfaces[0].grin(), e.surfaces[0].grin());
        let e = lens(vec![tilted(0.02, [0.01, -0.02]), Surface::default()]);
        assert!(reverse(&e).is_err());
        let e = lens(vec![tilted(0.02, [0.0; 2]), Surface::default()]);
        assert_eq!(reverse(&e).unwrap().surfaces[1].typ(), T::Standard);
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
