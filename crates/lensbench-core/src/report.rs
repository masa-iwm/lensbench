//! システムデータ（OpticStudio のオペランド名）。

use crate::math::*;
use crate::system::{FieldSel, FieldType, System, ENPP_MAX};

/// 実光線の作動 F 値（OpticStudio の WFNO）
pub fn working_fnum(s: &System) -> f64 {
    let r = s.trace(
        FieldSel::Norm(0.0, 0.0),
        &[0.0, 1.0, -1.0, 0.0, 0.0],
        &[0.0, 0.0, 0.0, 1.0, -1.0],
        Some(s.primary),
        None,
        false,
    );
    let k0 = r.rays[0].lmn;
    let m: f64 = r.rays[1..]
        .iter()
        .map(|x| {
            let c = dot(x.lmn, k0).clamp(-1.0, 1.0);
            1.0 - c * c
        })
        .sum::<f64>()
        / 4.0;
    1.0 / (2.0 * s.image_index(s.primary) * m.sqrt())
}

#[derive(Debug, Clone)]
pub struct SystemData {
    pub effl: f64,
    pub epdi: f64,
    pub enpp: f64,
    pub expd: f64,
    pub expp: f64,
    pub wfno: f64,
    pub pwfn: f64,
    pub isfn: f64,
    pub pimh: f64,
    pub pmag: f64,
    pub totr: f64,
    pub obna: f64,
}

pub fn system_data(s: &System) -> SystemData {
    let p = s.paraxial();
    let hmax = s.fields.iter().map(|f| f.x.hypot(f.y)).fold(0.0, f64::max);
    let pimh = match s.field_type {
        FieldType::Angle => {
            let t = hmax.to_radians().tan();
            if s.infinite() {
                p.efl.abs() * t
            } else {
                (p.pmag * (s.surfaces[0].t + p.enpp) * t).abs()
            }
        }
        FieldType::ObjH => p.pmag.abs() * hmax,
        _ => hmax,
    };
    let mut z = vec![0.0];
    for d in &s.surfaces[1..s.image()] {
        if !d.ignore {
            z.push(z.last().unwrap() + d.t);
        }
    }
    let totr =
        z.iter().cloned().fold(f64::MIN, f64::max) - z.iter().cloned().fold(f64::MAX, f64::min);
    let n0 = s.surfaces[0].n[s.primary];
    let obna = if s.infinite() {
        n0 * p.epd / 2.0 / ENPP_MAX
    } else {
        match p.u_obj {
            None => 0.0,
            Some(u) => n0 * u.abs().atan().sin(),
        }
    };
    SystemData {
        effl: p.efl,
        epdi: p.epd,
        enpp: p.enpp,
        expd: s.exit_pupil_diameter(),
        expp: p.expp,
        wfno: working_fnum(s),
        pwfn: p.wfno,
        isfn: p.isfn,
        pimh,
        pmag: p.pmag,
        totr,
        obna,
    }
}
