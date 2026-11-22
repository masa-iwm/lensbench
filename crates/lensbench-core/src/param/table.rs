//! 面の多重構成の表だけの値（開口・面の前後の傾き偏心・モデルガラス・熱膨張係数）を面に入れる

use super::Kind;
use crate::material::Material;
use crate::surface::{After, Aperture, Tilt};

/// 開口・遮蔽の偏心（偏心を持たない開口は None）
pub(super) fn aperture_decenter(a: &Aperture) -> Option<(f64, f64)> {
    match *a {
        Aperture::Circular { dx, dy, .. }
        | Aperture::Rect { dx, dy, .. }
        | Aperture::Ellipse { dx, dy, .. }
        | Aperture::Obsc { dx, dy, .. }
        | Aperture::Spider { dx, dy, .. } => Some((dx, dy)),
        _ => None,
    }
}

fn set_aperture_decenter(a: &mut Aperture, y: bool, v: f64) {
    if let Aperture::Circular { dx, dy, .. }
    | Aperture::Rect { dx, dy, .. }
    | Aperture::Ellipse { dx, dy, .. }
    | Aperture::Obsc { dx, dy, .. }
    | Aperture::Spider { dx, dy, .. } = a
    {
        *if y { dy } else { dx } = v;
    }
}

/// 開口の種類の OpticStudio の番号（SurfaceApertureTypes の順）
pub(super) fn aperture_os_code(a: &Aperture) -> i64 {
    match a {
        Aperture::None => 0,
        Aperture::Circular { .. } => 1,
        Aperture::Obsc { .. } => 2,
        Aperture::Spider { .. } => 3,
        Aperture::Rect { .. } => 4,
        Aperture::Ellipse { .. } => 6,
        Aperture::Floating { .. } => 10,
    }
}

/// 開口の種類を番号 n（0〜2）にする。半径・偏心は今の円形の開口・遮蔽の値（なしにしたら捨てる）
fn set_aperture_os_code(a: &mut Aperture, n: i64) {
    let (min, max, dx, dy) = match *a {
        Aperture::Circular { min, max, dx, dy } | Aperture::Obsc { min, max, dx, dy } => {
            (min, max, dx, dy)
        }
        _ => {
            if n == 0 {
                *a = Aperture::None;
            }
            return;
        }
    };
    *a = match n {
        1 => Aperture::Circular { min, max, dx, dy },
        2 => Aperture::Obsc { min, max, dx, dy },
        _ => Aperture::None,
    };
}

/// 面の多重構成の表だけの値（開口の最小の半径・偏心・種類、面の前後の傾き偏心の値と順序、モデルガラスの値、熱膨張係数、機械半径、チップゾーン）を面 d に入れる（値は確かめない）。
/// 面の前後の傾き偏心を表の値で作るときは、面の後を変えない（OpticStudio と同じく、後は値 0 のまま）。無いものに 0 を入れるときは
/// 作らない。モデルガラスでない面のモデルガラスの値は何もしない（OpticStudio と同じ）。ほかの種類は何もせず false
pub fn set_surface_table_value(d: &mut crate::surface::Surface, k: Kind, v: f64) -> bool {
    match k {
        Kind::ApMin => {
            if let Aperture::Circular { min, .. } | Aperture::Obsc { min, .. } = &mut d.ap {
                *min = v;
            }
        }
        Kind::ApDx | Kind::ApDy => set_aperture_decenter(&mut d.ap, k == Kind::ApDy, v),
        Kind::ApType => set_aperture_os_code(&mut d.ap, v as i64),
        Kind::Tce => d.tce = v,
        Kind::MechSemi => d.mech_semi = Some(v),
        Kind::ChipZone => d.chip_zone = v,
        Kind::CoordRet | Kind::CoordRetSurf => {
            use crate::coord_return::{CoordReturn, ReturnMode};
            let mut c = d.coord_return.unwrap_or(CoordReturn {
                mode: ReturnMode::Off,
                surf: 0,
            });
            if k == Kind::CoordRet {
                c.mode = ReturnMode::from_os_code(v as i64).unwrap_or(ReturnMode::Off);
            } else {
                c.surf = v.max(0.0) as usize;
            }
            d.coord_return = (c.mode != ReturnMode::Off || c.surf != 0).then_some(c);
        }
        Kind::ModelNd | Kind::ModelVd | Kind::ModelDpgf => {
            if let Material::Model { nd, vd, dpgf } = &mut d.mat {
                *match k {
                    Kind::ModelNd => nd,
                    Kind::ModelVd => vd,
                    _ => dpgf,
                } = v;
            }
        }
        Kind::Before(_) | Kind::BeforeOrder if d.before.is_none() && v == 0.0 => {}
        Kind::After(_) | Kind::AfterOrder if d.after == After::None && v == 0.0 => {}
        Kind::Before(i) => {
            d.before
                .get_or_insert(Tilt {
                    d: [0.0; 5],
                    order: 0,
                })
                .d[i as usize] = v
        }
        Kind::BeforeOrder => {
            d.before
                .get_or_insert(Tilt {
                    d: [0.0; 5],
                    order: 0,
                })
                .order = v as u8
        }
        Kind::After(_) | Kind::AfterOrder => {
            if d.after == After::None {
                d.after = After::Tilt(Tilt {
                    d: [0.0; 5],
                    order: 0,
                });
            }
            if let After::Tilt(t) = &mut d.after {
                match k {
                    Kind::After(i) => t.d[i as usize] = v,
                    _ => t.order = v as u8,
                }
            }
        }
        _ => return false,
    }
    true
}
