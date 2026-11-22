//! 系 → 処方（系ファイル v2 の `system`）

use super::*;
use lensbench_core::surface::{self as cs, Material, SurfType, Surface};
use lensbench_core::system::ApertureKind;
use lensbench_core::System;

/// 系の処方。`catalogs` は使ったガラスのカタログ（[`catalogs::refs`]）
pub(crate) fn prescription(s: &System) -> lensbench_core::Result<Prescription> {
    let nw = s.wavelengths.len();
    if s.weights.len() != nw || s.primary >= nw {
        return Err("the numbers of wavelengths, weights and primary don't match".into());
    }
    let surfaces = (0..s.surfaces.len())
        .map(|j| row(s, j).map_err(|e| format!("surface {j}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Prescription {
        name: s.name.clone(),
        wavelengths: s
            .wavelengths
            .iter()
            .zip(&s.weights)
            .map(|(&um, &weight)| Wave { um, weight })
            .collect(),
        primary: s.primary + 1,
        fields: fields(s),
        aperture: ApertureSpec {
            kind: s.aperture.0,
            value: s.aperture.1,
        },
        telecentric: s.telecentric,
        aiming: s.aiming,
        temperature_c: s.temperature,
        pressure_atm: s.pressure,
        adjust_index: s.adjust_index,
        afocal: s.afocal,
        afocal_unit: s.afocal_unit,
        vig_interp: s.vig_interp,
        floating_clip: s.floating_clip,
        glrs: s.glrs,
        ropd: s.ropd,
        aim_pupil: s.aim_pupil,
        input_pol: s.input_pol,
        apodization: s.apodization,
        coating_file: s
            .coating_file
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        coat_tables: s
            .coat_tables
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        surfaces,
        pickups: s.pickups.clone(),
        solves: s.solves.clone(),
        macros: s.macros.clone(),
        macro_parms: s.macro_parms.clone(),
        units: s.units.clone(),
        mechanics: s.mechanics.clone(),
        configs: s.configs.clone(),
        catalogs: catalogs::refs(s)?,
    })
}

fn fields(s: &System) -> Fields {
    let (kind, solve) = FieldKind::of(s.field_type);
    let weights: Vec<f64> = s.fields.iter().map(|f| f.w).collect();
    let vig: Vec<[f64; 5]> = s
        .fields
        .iter()
        .map(|f| [f.vdx, f.vdy, f.vcx, f.vcy, f.van])
        .collect();
    Fields {
        kind,
        points: s.fields.iter().map(|f| [f.x, f.y]).collect(),
        weights: if weights.iter().all(is_one_f) {
            Vec::new()
        } else {
            weights
        },
        solve,
        norm: s.field_norm,
        vignetting: if vig.iter().all(zeros) {
            Vec::new()
        } else {
            vig
        },
    }
}

fn tilt(t: &cs::Tilt) -> Tilt {
    Tilt {
        decenter: [t.d[0], t.d[1]],
        tilt: [t.d[2], t.d[3], t.d[4]],
        order: t.order,
    }
}

/// asph の i 番目（無ければ 0）
fn p(d: &Surface, i: usize) -> f64 {
    d.asph().get(i).copied().unwrap_or(0.0)
}

fn row(s: &System, j: usize) -> Result<Row, String> {
    // 絞りの大きさで決める開口（Float By Stop Size）の絞りの半直径は開口そのもの（光線から求め直さない）なので値を書く
    let float_stop = s.aperture.0 == ApertureKind::Float && j == s.stop();
    let mut r = surface_row(&s.surfaces[j], float_stop)?;
    r.object = j == 0;
    r.image = j + 1 == s.surfaces.len();
    Ok(r)
}

/// 面 1 つの行（物体面・像面の印は付けない）。float_stop は半直径を自動にしない（浮動開口の絞り）
pub(crate) fn surface_row(d: &Surface, float_stop: bool) -> Result<Row, String> {
    let mut r = Row {
        comment: d.comment.clone(),
        coating: d.coat.clone(),
        stop: d.stop,
        before: d.before.as_ref().map(tilt),
        after: match &d.after {
            cs::After::None => None,
            cs::After::Reverse => Some(After::Reverse(Reverse::Reverse)),
            cs::After::Tilt(t) => Some(After::Tilt(tilt(t))),
        },
        ignore: d.ignore,
        t: d.t,
        dn: d.dn,
        disp: d.disp,
        dnd: d.dnd,
        dvd: d.dvd,
        tce: d.tce,
        mech: d.mech_semi,
        chip: d.chip_zone,
        ..Default::default()
    };
    r.semi = if d.semi_auto && !float_stop {
        Some(Semi::Auto(Auto::Auto))
    } else if zero(&d.semi) {
        None
    } else {
        Some(Semi::Value(d.semi))
    };
    r.aperture = aperture(d);
    if d.is_coord_break() {
        if d.is_mirror() {
            return Err("a coordinate break can't be a mirror".into());
        }
        r.coordinate_break = Some(Tilt {
            decenter: [d.cb()[0], d.cb()[1]],
            tilt: [d.cb()[2], d.cb()[3], d.cb()[4]],
            order: d.cb_order(),
        });
        return Ok(r);
    }
    let (c, k) = (d.c, d.k);
    let mirror = d.is_mirror();
    // 鏡・ゴーストの反射・複屈折材の板は形だけを持つ面タイプの働き
    let plain_action = || -> Result<Option<Action>, String> {
        Ok(match (&d.bire, mirror, &d.reflect) {
            (Some(_), true, _) => {
                return Err("a mirror on a birefringent surface can't be written".into())
            }
            (Some(cs::Bire::In { mode, axis, .. }), false, _) => Some(Action::BirefringentIn {
                mode: *mode,
                axis: *axis,
            }),
            (Some(cs::Bire::Out), false, _) => Some(Action::BirefringentOut),
            (None, true, Some(g)) => Some(Action::GhostReflect {
                medium: g.mat.clone(),
            }),
            (None, true, None) => Some(Action::Reflect),
            (None, false, _) => None,
        })
    };
    let own_action = |a: Action| -> Result<Option<Action>, String> {
        if d.reflect.is_some() {
            return Err("a ghost reflection on this surface type can't be written".into());
        }
        Ok(Some(a))
    };
    let conic = || (!(zero(&c) && zero(&k))).then_some(Shape::Conic { c, k });
    let (shape, action) = match d.typ() {
        SurfType::Standard | SurfType::Gradium | SurfType::Gradient9 => (conic(), plain_action()?),
        SurfType::EvenAsph => (
            Some(Shape::EvenAsphere {
                c,
                k,
                a: d.asph().to_vec(),
            }),
            plain_action()?,
        ),
        SurfType::Toroidal => (
            Some(Shape::Toroid {
                c,
                k,
                a: d.asph().to_vec(),
                cx: d.cx(),
            }),
            plain_action()?,
        ),
        SurfType::ZernikeStd | SurfType::ZernikeFringe => {
            let z = ZernikeShape {
                c,
                k,
                a: d.asph().to_vec(),
                terms: d.zern().cloned(),
            };
            (
                Some(if d.typ() == SurfType::ZernikeStd {
                    Shape::ZernikeStandard(z)
                } else {
                    Shape::ZernikeFringe(z)
                }),
                plain_action()?,
            )
        }
        SurfType::GridSag => (
            Some(Shape::GridSag {
                c,
                k,
                a: d.asph().to_vec(),
                terms: d.zern().cloned(),
                grid: d.grid().cloned(),
                grid_file: d.grid_file().to_string(),
            }),
            plain_action()?,
        ),
        SurfType::OddAsph => (
            Some(Shape::OddAsphere {
                c,
                k,
                b: d.asph().to_vec(),
            }),
            plain_action()?,
        ),
        SurfType::TiltSurf => (
            Some(Shape::Tilted {
                tan: [p(d, 0), p(d, 1)],
                c,
                k,
            }),
            plain_action()?,
        ),
        SurfType::Irregular => (
            Some(Shape::Irregular {
                c,
                k,
                decenter: [p(d, 0), p(d, 1)],
                tilt: [p(d, 2), p(d, 3)],
                zs: p(d, 4),
                za: p(d, 5),
                zc: p(d, 6),
                angle: p(d, 7),
            }),
            plain_action()?,
        ),
        SurfType::Binary2 => (
            Some(Shape::EvenAsphere {
                c,
                k,
                a: d.asph().to_vec(),
            }),
            own_action(Action::Binary2 {
                phase: d.bin2().cloned(),
                mirror,
            })?,
        ),
        SurfType::Fresnel => (
            None,
            own_action(Action::Fresnel {
                c,
                k,
                a: d.asph().to_vec(),
                mirror,
            })?,
        ),
        SurfType::CylFresnel => (
            Some(Shape::Cylinder {
                c,
                k,
                a: d.asph().to_vec(),
            }),
            own_action(Action::CylinderFresnel {
                bend: d.cylf().cloned(),
                mirror,
            })?,
        ),
        SurfType::Paraxial => (
            None,
            own_action(Action::Paraxial {
                f: d.focal(),
                opd_mode: d.opd_mode(),
                mirror,
            })?,
        ),
        SurfType::Jones => (
            None,
            own_action(Action::Jones {
                matrix: d.jones(),
                retarder: d.retarder.as_ref().map(|r| Retarder {
                    axis_deg: r.axis_deg,
                    tx: r.tx,
                    ty: r.ty,
                    retardance: r.retardance,
                    order: r.order,
                    wl0: r.wl0,
                    plates: r
                        .plates
                        .iter()
                        .map(|p| Plate {
                            material: p.material.clone(),
                            thickness: p.thickness,
                            axis_deg: p.axis_deg,
                        })
                        .collect(),
                }),
                mirror,
            })?,
        ),
        SurfType::UserShape => (
            Some(match d.user_shape() {
                Some(u) => Shape::User {
                    c,
                    k,
                    model: u.model.clone(),
                    config: u.config.clone(),
                    flip_z: u.flip_z,
                    mirror_x: u.mirror_x,
                },
                None => Shape::User {
                    c,
                    k,
                    model: String::new(),
                    config: Default::default(),
                    flip_z: false,
                    mirror_x: false,
                },
            }),
            plain_action()?,
        ),
        SurfType::User => (
            None,
            own_action(match &d.user() {
                Some(u) => Action::User {
                    model: u.model.clone(),
                    config: u.config.clone(),
                    mirror,
                },
                None => Action::User {
                    model: String::new(),
                    config: Default::default(),
                    mirror,
                },
            })?,
        ),
        SurfType::CoordBrk => unreachable!("座標ブレークは上で返した"),
    };
    r.shape = shape;
    r.action = action;
    if !mirror {
        r.medium = medium(d)?;
    }
    Ok(r)
}

fn aperture(d: &Surface) -> Option<Aperture> {
    Some(match d.ap {
        cs::Aperture::None => return None,
        cs::Aperture::Circular { min, max, dx, dy } => Aperture::Circular { min, max, dx, dy },
        cs::Aperture::Rect { hx, hy, dx, dy } => Aperture::Rect { hx, hy, dx, dy },
        cs::Aperture::Floating { semi } => Aperture::Floating {
            // 自動の半直径なら面の値と一緒に求め直す
            semi: (!d.semi_auto && semi.to_bits() != d.semi.to_bits()).then_some(semi),
        },
        cs::Aperture::Ellipse { hx, hy, dx, dy } => Aperture::Ellipse { hx, hy, dx, dy },
        cs::Aperture::Obsc { min, max, dx, dy } => Aperture::Obscuration { min, max, dx, dy },
        cs::Aperture::Spider {
            width,
            arms,
            dx,
            dy,
        } => Aperture::Spider {
            width,
            arms,
            dx,
            dy,
        },
    })
}

/// 面の後の媒質（鏡でない面）
fn medium(d: &Surface) -> Result<Option<Medium>, String> {
    if let Some(g) = &d.grin() {
        return Ok(Some(Medium::Data(MediumData::Grin(Box::new(Grin {
            name: d.mat.name(),
            profile: g.kind.clone(),
            dt: g.dt,
        })))));
    }
    Ok(match &d.mat {
        Material::Air | Material::Mirror | Material::Inherit => None,
        Material::Glass(m) => Some(Medium::Glass(m.clone())),
        Material::Model { nd, vd, dpgf } => Some(Medium::Data(MediumData::Model {
            nd: *nd,
            vd: *vd,
            dpgf: *dpgf,
        })),
        // 分散のない媒質
        Material::Index(nd) => Some(Medium::Data(MediumData::Model {
            nd: *nd,
            vd: 0.0,
            dpgf: 0.0,
        })),
    })
}
