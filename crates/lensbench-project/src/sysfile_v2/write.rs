//! 系 → 処方（系ファイル v2 の `system`）

use super::*;
use lensbench_core::surface::{self as cs, SurfType, Surface};
use lensbench_core::system::{self as sy, ApertureKind, FieldCond, FieldType};
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
            kind: match s.aperture.0 {
                ApertureKind::Epd => ApertureType::Epd,
                ApertureKind::FNum => ApertureType::Fnum,
                ApertureKind::Na => ApertureType::Na,
                ApertureKind::Float => ApertureType::Float,
                ApertureKind::Pwfn => ApertureType::Pwfn,
                ApertureKind::Cone => ApertureType::Cone,
            },
            value: s.aperture.1,
        },
        telecentric: s.telecentric,
        aiming: match s.aiming {
            sy::Aiming::Off => AimingSpec::Off,
            sy::Aiming::Paraxial => AimingSpec::Paraxial,
            sy::Aiming::Real => AimingSpec::Real,
        },
        temperature_c: s.temperature,
        pressure_atm: s.pressure,
        adjust_index: s.adjust_index,
        afocal: s.afocal,
        afocal_unit: match s.afocal_unit {
            sy::AfocalUnit::Microradians => AfocalUnitSpec::Microradians,
            sy::AfocalUnit::Milliradians => AfocalUnitSpec::Milliradians,
            sy::AfocalUnit::Radians => AfocalUnitSpec::Radians,
            sy::AfocalUnit::ArcSeconds => AfocalUnitSpec::ArcSeconds,
            sy::AfocalUnit::ArcMinutes => AfocalUnitSpec::ArcMinutes,
            sy::AfocalUnit::Degrees => AfocalUnitSpec::Degrees,
        },
        vig_interp: match s.vig_interp {
            sy::VigInterp::OpticStudio => VigInterpSpec::OpticStudio,
            sy::VigInterp::Continuous => VigInterpSpec::Continuous,
        },
        floating_clip: s.floating_clip,
        glrs: s.glrs,
        ropd: s.ropd,
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
    let (kind, solve) = match s.field_type {
        FieldType::Angle => (FieldKind::Angle, None),
        FieldType::ObjH => (FieldKind::ObjectHeight, None),
        FieldType::ParH => (FieldKind::ParaxialImageHeight, None),
        FieldType::RealH => (FieldKind::RealImageHeight, None),
        FieldType::Solve(c) => (
            FieldKind::Solve,
            Some(match c {
                FieldCond::Position { surf } => FieldSolve::Position { surface: surf },
                FieldCond::DirSum { s1, s2 } => FieldSolve::DirSum { s1, s2 },
                FieldCond::DirDiff { s1, s2 } => FieldSolve::DirDiff { s1, s2 },
            }),
        ),
    };
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
        norm: match s.field_norm {
            sy::FieldNorm::Radial => FieldNorm::Radial,
            sy::FieldNorm::Rect => FieldNorm::Rect,
        },
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
    d.asph.get(i).copied().unwrap_or(0.0)
}

fn row(s: &System, j: usize) -> Result<Row, String> {
    let d = &s.surfaces[j];
    let mut r = Row {
        object: j == 0,
        image: j + 1 == s.surfaces.len(),
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
        ..Default::default()
    };
    // 絞りの大きさで決める開口（Float By Stop Size）の絞りの半直径は開口そのもの（光線から求め直さない）なので値を書く
    let float_stop = s.aperture.0 == ApertureKind::Float && j == s.stop();
    r.semi = if d.semi_auto && !float_stop {
        Some(Semi::Auto(Auto::Auto))
    } else if zero(&d.semi) {
        None
    } else {
        Some(Semi::Value(d.semi))
    };
    r.aperture = aperture(d);
    if d.is_coord_break() {
        if d.mirror {
            return Err("a coordinate break can't be a mirror".into());
        }
        r.coordinate_break = Some(Tilt {
            decenter: [d.cb[0], d.cb[1]],
            tilt: [d.cb[2], d.cb[3], d.cb[4]],
            order: d.cb_order,
        });
        return Ok(r);
    }
    let (c, k) = (d.c, d.k);
    let mirror = d.mirror;
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
    let (shape, action) = match d.typ {
        SurfType::Standard | SurfType::Gradium | SurfType::Gradient9 => (conic(), plain_action()?),
        SurfType::EvenAsph => (
            Some(Shape::EvenAsphere {
                c,
                k,
                a: d.asph.clone(),
            }),
            plain_action()?,
        ),
        SurfType::Toroidal => (
            Some(Shape::Toroid {
                c,
                k,
                a: d.asph.clone(),
                cx: d.cx,
            }),
            plain_action()?,
        ),
        SurfType::ZernikeStd | SurfType::ZernikeFringe => {
            let z = ZernikeShape {
                c,
                k,
                a: d.asph.clone(),
                terms: d.zern.as_deref().cloned(),
                grid: d.grid.as_deref().cloned(),
            };
            (
                Some(if d.typ == SurfType::ZernikeStd {
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
                a: d.asph.clone(),
                terms: d.zern.as_deref().cloned(),
                grid: d.grid.as_deref().cloned(),
                grid_file: d.grid_file.clone(),
            }),
            plain_action()?,
        ),
        SurfType::OddAsph => (
            Some(Shape::OddAsphere {
                c,
                k,
                b: d.asph.clone(),
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
                a: d.asph.clone(),
            }),
            own_action(Action::Binary2 {
                phase: d.bin2.as_deref().cloned(),
                mirror,
            })?,
        ),
        SurfType::Fresnel => (
            None,
            own_action(Action::Fresnel {
                c,
                k,
                a: d.asph.clone(),
                mirror,
            })?,
        ),
        SurfType::CylFresnel => (
            Some(Shape::Cylinder {
                c,
                k,
                a: d.asph.clone(),
            }),
            own_action(Action::CylinderFresnel {
                bend: d.cylf.as_deref().cloned(),
                mirror,
            })?,
        ),
        SurfType::Paraxial => (
            None,
            own_action(Action::Paraxial {
                f: d.focal,
                opd_mode: d.opd_mode,
                mirror,
            })?,
        ),
        SurfType::Jones => (
            None,
            own_action(Action::Jones {
                matrix: d.jones,
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
        SurfType::User => (
            None,
            own_action(match &d.user {
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
        r.medium = medium(s, j)?;
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

/// モデルガラスの名前 "nd,vd[,dpgf]" の (nd, vd, dpgf)（core の media.rs と同じ読み方）
pub(crate) fn model_glass(mat: &str) -> Option<(f64, f64, f64)> {
    let v: Vec<f64> = mat
        .trim()
        .split(',')
        .map(|x| x.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .ok()?;
    ((2..=3).contains(&v.len()) && v.iter().all(|x| x.is_finite()) && v[0] > 0.0)
        .then(|| (v[0], v[1], v.get(2).copied().unwrap_or(0.0)))
}

/// 面 j の後の媒質（鏡でない面）
fn medium(s: &System, j: usize) -> Result<Option<Medium>, String> {
    let d = &s.surfaces[j];
    if let Some(g) = &d.grin {
        return Ok(Some(Medium::Data(MediumData::Grin(Box::new(Grin {
            name: d.mat.clone(),
            profile: g.kind.clone(),
            dt: g.dt,
        })))));
    }
    let m = d.mat.trim();
    if m.is_empty() {
        return Ok(None);
    }
    let base = |w: usize| -> f64 {
        if d.n_base.is_empty() {
            d.n.get(w).copied().unwrap_or(f64::NAN)
        } else {
            d.n_base[w]
        }
    };
    // 分散のない媒質は名前でなく面の値（名前は丸めてある）
    let flat = |nd: f64| {
        Medium::Data(MediumData::Model {
            nd,
            vd: 0.0,
            dpgf: 0.0,
        })
    };
    if let Some((nd, vd, dpgf)) = model_glass(m) {
        return Ok(Some(if vd == 0.0 {
            flat(base(s.primary))
        } else {
            Medium::Data(MediumData::Model { nd, vd, dpgf })
        }));
    }
    if m.eq_ignore_ascii_case("AIR") || s.glasses.contains_key(m) {
        return Ok(Some(Medium::Glass(d.mat.clone())));
    }
    // カタログに無い名前で、どの波長でも同じ値の媒質（屈折率だけ決めた媒質）
    let n0 = base(0);
    if !d.n.is_empty() && (0..s.wavelengths.len()).all(|w| base(w).to_bits() == n0.to_bits()) {
        return Ok(Some(flat(base(s.primary))));
    }
    Ok(Some(Medium::Glass(d.mat.clone())))
}
