//! 処方（系ファイル v2 の `system`）→ 系。屈折率・自動の半直径・ピックアップ・ソルブは .zmx の読込と同じく
//! 読んだ後に求める（[`System::refresh`]）

use super::*;
use lensbench_core::glass::Glass;
use lensbench_core::surface::{self as cs, Material, SurfType, Surface};
use lensbench_core::system::{self as sy, FieldCond, FieldType};
use lensbench_core::System;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// 処方から系を作る。glasses は `catalogs` から読んだガラス
pub(crate) fn system(
    p: Prescription,
    glasses: Arc<HashMap<String, Glass>>,
) -> Result<System, String> {
    let nw = p.wavelengths.len();
    if nw == 0 {
        return Err("wavelengths: no wavelength".into());
    }
    if p.primary == 0 || p.primary > nw {
        return Err(format!("primary: {} ({nw} wavelengths)", p.primary));
    }
    let ns = p.surfaces.len();
    if ns < 3 {
        return Err(format!("surfaces: too few surfaces ({ns})"));
    }
    let mut surfaces = Vec::with_capacity(ns);
    for (j, r) in p.surfaces.into_iter().enumerate() {
        if r.object && j != 0 {
            return Err(format!("surfaces[{j}]: only the first row is the object"));
        }
        if r.image && j + 1 != ns {
            return Err(format!("surfaces[{j}]: only the last row is the image"));
        }
        surfaces.push(surface(r).map_err(|e| format!("surfaces[{j}]: {e}"))?);
    }
    let f = &p.fields;
    let npt = f.points.len();
    if !f.weights.is_empty() && f.weights.len() != npt {
        return Err(format!(
            "fields.weights: {} weights ({npt} points)",
            f.weights.len()
        ));
    }
    if !f.vignetting.is_empty() && f.vignetting.len() != npt {
        return Err(format!(
            "fields.vignetting: {} rows ({npt} points)",
            f.vignetting.len()
        ));
    }
    let fields: Vec<sy::Field> = f
        .points
        .iter()
        .enumerate()
        .map(|(i, &[x, y])| {
            let v = f.vignetting.get(i).copied().unwrap_or([0.0; 5]);
            sy::Field {
                x,
                y,
                w: f.weights.get(i).copied().unwrap_or(1.0),
                vdx: v[0],
                vdy: v[1],
                vcx: v[2],
                vcy: v[3],
                van: v[4],
            }
        })
        .collect();
    let field_type = match (f.kind, f.solve) {
        (FieldKind::Solve, Some(c)) => FieldType::Solve(match c {
            FieldSolve::Position { surface } => FieldCond::Position { surf: surface },
            FieldSolve::DirSum { s1, s2 } => FieldCond::DirSum { s1, s2 },
            FieldSolve::DirDiff { s1, s2 } => FieldCond::DirDiff { s1, s2 },
        }),
        (FieldKind::Solve, None) => return Err("fields: type solve needs solve".into()),
        (_, Some(_)) => return Err("fields: solve is only for type solve".into()),
        (k, None) => k.plain().ok_or("fields: type solve needs solve")?,
    };
    let aperture = (p.aperture.kind, p.aperture.value);
    let aiming = p.aiming;
    let mut s = System::new(
        surfaces,
        p.wavelengths.iter().map(|w| w.um).collect(),
        p.wavelengths.iter().map(|w| w.weight).collect(),
        p.primary - 1,
        fields,
        field_type,
        f.norm,
        aperture,
        p.telecentric,
        aiming,
        glasses,
    )
    .map_err(|e| e.to_string())?
    .into_prescription();
    s.name = p.name;
    s.temperature = p.temperature_c;
    s.pressure = p.pressure_atm;
    s.adjust_index = p.adjust_index;
    s.glrs = p.glrs;
    s.ropd = p.ropd;
    s.apodization = p.apodization;
    s.afocal = p.afocal;
    s.afocal_unit = p.afocal_unit;
    s.vig_interp = p.vig_interp;
    s.floating_clip = p.floating_clip;
    s.coating_file = p.coating_file.map(PathBuf::from);
    s.coat_tables = p.coat_tables.into_iter().collect();
    s.pickups = p.pickups;
    s.solves = p.solves;
    s.macros = p.macros;
    s.macro_parms = p.macro_parms;
    s.units = p.units;
    s.mechanics = p.mechanics;
    s.configs = p.configs;
    // 範囲の外を指すもので refresh が panic しないよう先に確かめる
    crate::sysfile::check_refs(&s)?;
    // 波長板の複屈折材の板・ゴーストの反射の媒質をガラスから引く
    let glasses = s.glasses.clone();
    for (j, d) in s.surfaces.iter_mut().enumerate() {
        if let Some(r) = &mut d.retarder {
            r.resolve(&glasses)
                .map_err(|e| format!("surface {j}: {e}"))?;
        }
    }
    for j in 0..s.surfaces.len() {
        if let Some(g) = &s.surfaces[j].reflect {
            let n = s
                .material_index(&Material::parse(&g.mat))
                .map_err(|e| format!("surface {j}: ghost reflection medium: {e}"))?;
            if let Some(g) = &mut s.surfaces[j].reflect {
                g.n = n;
            }
        }
    }
    s.build().map_err(|e| e.to_string())
}

fn core_tilt(t: &Tilt) -> cs::Tilt {
    cs::Tilt {
        d: [
            t.decenter[0],
            t.decenter[1],
            t.tilt[0],
            t.tilt[1],
            t.tilt[2],
        ],
        order: t.order,
    }
}

/// 名前のある値から asph（8 個）
fn named(v: &[f64]) -> Vec<f64> {
    let mut a = v.to_vec();
    a.resize(8, 0.0);
    a
}

/// 行 1 つの面（ガラスから引くもの: 波長板の複屈折材・ゴーストの反射の媒質は引かない）
pub(crate) fn surface(r: Row) -> Result<Surface, String> {
    let mut d = Surface {
        comment: r.comment,
        coat: r.coating,
        stop: r.stop,
        ignore: r.ignore,
        t: r.t,
        dn: r.dn,
        disp: r.disp,
        before: r.before.as_ref().map(core_tilt),
        after: match &r.after {
            None => cs::After::None,
            Some(After::Reverse(_)) => cs::After::Reverse,
            Some(After::Tilt(t)) => cs::After::Tilt(core_tilt(t)),
        },
        ..Default::default()
    };
    match r.semi {
        None => {}
        Some(Semi::Value(v)) => d.semi = v,
        Some(Semi::Auto(_)) => d.semi_auto = true,
    }
    if let Some(ap) = r.aperture {
        d.ap = match ap {
            Aperture::Circular { min, max, dx, dy } => cs::Aperture::Circular { min, max, dx, dy },
            Aperture::Rect { hx, hy, dx, dy } => cs::Aperture::Rect { hx, hy, dx, dy },
            Aperture::Floating { semi } => cs::Aperture::Floating {
                semi: semi.unwrap_or(d.semi),
            },
            Aperture::Ellipse { hx, hy, dx, dy } => cs::Aperture::Ellipse { hx, hy, dx, dy },
            Aperture::Obscuration { min, max, dx, dy } => cs::Aperture::Obsc { min, max, dx, dy },
            Aperture::Spider {
                width,
                arms,
                dx,
                dy,
            } => cs::Aperture::Spider {
                width,
                arms,
                dx,
                dy,
            },
        };
    }
    if let Some(cb) = r.coordinate_break {
        if r.shape.is_some() || r.action.is_some() || r.medium.is_some() {
            return Err("a coordinate break has no shape, action or medium".into());
        }
        d.typ = SurfType::CoordBrk;
        d.cb = [
            cb.decenter[0],
            cb.decenter[1],
            cb.tilt[0],
            cb.tilt[1],
            cb.tilt[2],
        ];
        d.cb_order = cb.order;
        d.mat = "-".into();
        return Ok(d);
    }
    // 形
    let mut cyl = false;
    let shaped = r.shape.is_some();
    match r.shape {
        None => {}
        Some(Shape::Conic { c, k }) => (d.c, d.k) = (c, k),
        Some(Shape::EvenAsphere { c, k, a }) => {
            d.typ = SurfType::EvenAsph;
            (d.c, d.k, d.asph) = (c, k, a);
        }
        Some(Shape::OddAsphere { c, k, b }) => {
            d.typ = SurfType::OddAsph;
            (d.c, d.k, d.asph) = (c, k, b);
        }
        Some(Shape::Toroid { c, k, a, cx }) => {
            d.typ = SurfType::Toroidal;
            (d.c, d.k, d.asph, d.cx) = (c, k, a, cx);
        }
        Some(Shape::ZernikeStandard(z)) => {
            d.typ = SurfType::ZernikeStd;
            zernike(&mut d, z);
        }
        Some(Shape::ZernikeFringe(z)) => {
            d.typ = SurfType::ZernikeFringe;
            zernike(&mut d, z);
        }
        Some(Shape::GridSag {
            c,
            k,
            a,
            terms,
            grid,
            grid_file,
        }) => {
            d.typ = SurfType::GridSag;
            (d.c, d.k, d.asph) = (c, k, a);
            d.zern = terms.map(Arc::new);
            d.grid = grid.map(Arc::new);
            d.grid_file = grid_file;
        }
        Some(Shape::Tilted { tan, c, k }) => {
            d.typ = SurfType::TiltSurf;
            (d.c, d.k) = (c, k);
            d.asph = named(&tan);
        }
        Some(Shape::Irregular {
            c,
            k,
            decenter,
            tilt,
            zs,
            za,
            zc,
            angle,
        }) => {
            d.typ = SurfType::Irregular;
            (d.c, d.k) = (c, k);
            d.asph = named(&[
                decenter[0],
                decenter[1],
                tilt[0],
                tilt[1],
                zs,
                za,
                zc,
                angle,
            ]);
        }
        Some(Shape::Cylinder { c, k, a }) => {
            cyl = true;
            (d.c, d.k, d.asph) = (c, k, a);
        }
    }
    let shape_typ = d.typ;
    let standard = |what: &str| -> Result<(), String> {
        if shape_typ != SurfType::Standard || cyl {
            return Err(format!("the {what} action takes no shape of this kind"));
        }
        Ok(())
    };
    // 働き
    let mut is_mirror = false;
    match r.action {
        None => {}
        Some(Action::Reflect) => is_mirror = true,
        Some(Action::GhostReflect { medium }) => {
            is_mirror = true;
            d.reflect = Some(cs::Reflect {
                mat: medium,
                n: Vec::new(),
            });
        }
        Some(Action::Paraxial {
            f,
            opd_mode,
            mirror,
        }) => {
            if shaped {
                return Err("a paraxial lens has no shape".into());
            }
            d.typ = SurfType::Paraxial;
            (d.focal, d.opd_mode, is_mirror) = (f, opd_mode, mirror);
        }
        Some(Action::Jones {
            matrix,
            retarder,
            mirror,
        }) => {
            if shaped {
                return Err("a Jones matrix surface has no shape".into());
            }
            d.typ = SurfType::Jones;
            d.jones = matrix;
            is_mirror = mirror;
            d.retarder = retarder.map(|r| cs::Retarder {
                axis_deg: r.axis_deg,
                tx: r.tx,
                ty: r.ty,
                retardance: r.retardance,
                order: r.order,
                wl0: r.wl0,
                plates: r
                    .plates
                    .into_iter()
                    .map(|p| cs::Plate {
                        material: p.material,
                        thickness: p.thickness,
                        axis_deg: p.axis_deg,
                        o: None,
                        e: None,
                    })
                    .collect(),
            });
        }
        Some(Action::Binary2 { phase, mirror }) => {
            if !matches!(shape_typ, SurfType::Standard | SurfType::EvenAsph) || cyl {
                return Err("Binary 2 takes an even asphere shape".into());
            }
            d.typ = SurfType::Binary2;
            d.bin2 = phase.map(Arc::new);
            is_mirror = mirror;
        }
        Some(Action::Fresnel { c, k, a, mirror }) => {
            if shaped {
                return Err(
                    "a Fresnel surface has no shape (its c, k, a are in the action)".into(),
                );
            }
            d.typ = SurfType::Fresnel;
            (d.c, d.k, d.asph, is_mirror) = (c, k, a, mirror);
        }
        Some(Action::CylinderFresnel { bend, mirror }) => {
            if shaped && !cyl {
                return Err("a cylinder Fresnel surface takes a cylinder shape".into());
            }
            d.typ = SurfType::CylFresnel;
            d.cylf = bend.map(Arc::new);
            is_mirror = mirror;
            cyl = false;
        }
        Some(Action::BirefringentIn { mode, axis }) => {
            standard("birefringent_in")?;
            d.bire = Some(cs::Bire::In {
                mode,
                axis,
                ne: Vec::new(),
            });
        }
        Some(Action::BirefringentOut) => {
            standard("birefringent_out")?;
            d.bire = Some(cs::Bire::Out);
        }
        Some(Action::User {
            model,
            config,
            mirror,
        }) => {
            if shaped {
                return Err("a user-defined surface has no shape".into());
            }
            d.typ = SurfType::User;
            is_mirror = mirror;
            if !model.is_empty() {
                d.user = Some(
                    lensbench_core::usersurf::UserSurf::new(&model, config)
                        .map_err(|e| format!("action.user: {e}"))?,
                );
            }
        }
    }
    if cyl {
        return Err("a cylinder shape needs the cylinder_fresnel action".into());
    }
    // 媒質
    if is_mirror {
        if r.medium.is_some() {
            return Err("a mirror keeps the medium before it (write no medium)".into());
        }
        d.mat = Material::Mirror;
        return Ok(d);
    }
    match r.medium {
        None => {}
        Some(Medium::Glass(m)) => {
            d.mat = if m.trim().eq_ignore_ascii_case("AIR") {
                Material::Air
            } else {
                Material::Glass(m)
            }
        }
        Some(Medium::Data(MediumData::Model { nd, vd, dpgf })) => {
            if !(nd.is_finite() && nd > 0.0 && vd.is_finite() && dpgf.is_finite()) {
                return Err(format!("model glass nd {nd}, vd {vd}, dpgf {dpgf}"));
            }
            d.mat = if vd == 0.0 {
                Material::Index(nd)
            } else {
                Material::Model { nd, vd, dpgf }
            };
        }
        Some(Medium::Data(MediumData::Grin(g))) => {
            let g = *g;
            if !matches!(d.typ, SurfType::Standard) || d.bire.is_some() {
                return Err("a GRIN medium follows a conic surface".into());
            }
            let grin = lensbench_core::grin::Grin {
                kind: g.profile,
                dt: g.dt,
            };
            d.typ = grin.surf_type();
            d.grin = Some(Arc::new(grin));
            d.mat = Material::Glass(g.name);
        }
    }
    Ok(d)
}

fn zernike(d: &mut Surface, z: ZernikeShape) {
    (d.c, d.k, d.asph) = (z.c, z.k, z.a);
    d.zern = z.terms.map(Arc::new);
    d.grid = z.grid.map(Arc::new);
}
