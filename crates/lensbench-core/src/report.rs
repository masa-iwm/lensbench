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
    /// 像空間の NA: n′·sin θ′（θ′ は近軸マージナル光線の像空間での実際の角度、tan θ′ = u′、主波長、定めた共役で）。
    /// OpticStudio のヘルプの定義どおり。OpticStudio 2026 R1.01 の表示は sin(atan(n′u′)) で、像空間が空気でないと
    /// これと違う（像面が N-BK7 中の系で 0.0516110、この値は 0.0516499）
    pub isna: f64,
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
        FieldType::Solve(crate::system::FieldCond::Position { surf }) if surf == s.image() => hmax,
        // 視野の値が像高でない条件
        FieldType::Solve(_) => f64::NAN,
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
    let umi = p.marginal.1[s.image() - 1];
    let isna = s.image_index(s.primary) * umi.abs().atan().sin();
    SystemData {
        effl: p.efl,
        epdi: p.epd,
        enpp: p.enpp,
        expd: s.exit_pupil_diameter(),
        // 頂点の尖った面がある系は OpticStudio の EXPP も実光線の射出瞳（OPD の基準と同じ）
        expp: if s.cusp_pupil() {
            s.exit_pupil()
        } else {
            p.expp
        },
        wfno: working_fnum(s),
        pwfn: p.wfno,
        isfn: p.isfn,
        pimh,
        pmag: p.pmag,
        totr,
        obna,
        isna,
    }
}

/// 基点（OpticStudio の Cardinal Points 相当）。面 1 から像面の手前の最後の面までの近軸の系（Y-Z 面）で、波長 w。
///
/// 物体空間の点は面 1 の頂点から、像空間の点は像面の頂点から、それぞれの面の局所 z で測る [mm]（OpticStudio と同じ。
/// 鏡が奇数枚なら像空間の z は光の進む向きと逆）。焦点距離は媒質の中の値（物体空間は負）で、空気換算の `efl` =
/// 1/パワーとは `f_image = n′·efl`、`f_object = −n·efl` の関係。鏡が奇数枚なら n′ は負（OpticStudio と同じ）。
/// 無焦点の系（パワー 0）では焦点距離と位置が無限大になる。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cardinal {
    /// 物体空間・像空間の屈折率
    pub n_object: f64,
    pub n_image: f64,
    /// 空気換算の焦点距離（EFFL と同じ定義）
    pub efl: f64,
    pub f_object: f64,
    pub f_image: f64,
    /// 前側（物体空間）: 焦点・主点・反主点・節点・反節点（面 1 から）
    pub focal_object: f64,
    pub principal_object: f64,
    pub anti_principal_object: f64,
    pub nodal_object: f64,
    pub anti_nodal_object: f64,
    /// 後側（像空間）: 同じ順（像面から）
    pub focal_image: f64,
    pub principal_image: f64,
    pub anti_principal_image: f64,
    pub nodal_image: f64,
    pub anti_nodal_image: f64,
    /// 最後の面から後側焦点までの距離（バックフォーカス、最後の面の局所 z）
    pub bfl: f64,
    /// 面 1 から前側焦点までの距離（フロントフォーカス、負なら面 1 より前）
    pub ffl: f64,
}

pub fn cardinal(s: &System, w: usize) -> Cardinal {
    let img = s.image();
    let sf = &s.surfaces;
    // 像面の手前の、無視しない最後の面
    let last = (1..img).rev().find(|&j| !sf[j].ignore).unwrap_or(0);
    let n0 = sf[0].n[w];
    let n1 = s.image_index(w);
    let (ya, ua) = s.ptrace(1.0, 0.0, w, Some(last));
    let (_, ub) = s.ptrace(0.0, 1.0, w, Some(last));
    // 換算した傾き (y, n·u) の行列: 面 1 の頂点 → 最後の面の頂点（面の後）
    let (m11, m21) = (ya[last], n1 * ua[last]);
    let m22 = n1 * ub[last] / n0;
    let phi = -m21;
    // 鏡の数の偶奇（展開した向き → 物理的な向き）
    let sgn = sf[1..=last]
        .iter()
        .filter(|d| !d.ignore && d.mirror)
        .fold(1.0, |a, _| -a);
    let inf = f64::INFINITY;
    if phi == 0.0 || !phi.is_finite() {
        return Cardinal {
            n_object: n0,
            n_image: n1,
            efl: inf,
            f_object: -inf,
            f_image: inf,
            focal_object: inf,
            principal_object: inf,
            anti_principal_object: inf,
            nodal_object: inf,
            anti_nodal_object: inf,
            focal_image: inf,
            principal_image: inf,
            anti_principal_image: inf,
            nodal_image: inf,
            anti_nodal_image: inf,
            bfl: inf,
            ffl: inf,
        };
    }
    let efl = 1.0 / phi;
    // 鏡の数が奇数なら像空間の屈折率を負とする（OpticStudio と同じ）。像空間の位置は最後の面の局所 z（物理的な向き）
    let n1s = sgn * n1;
    let (fo, fi) = (-n0 * efl, n1s * efl);
    // 像空間（最後の面から、展開した向き）: 焦点 F′ は平行光の交わる点。物理的な向きに直して主点 P′ = F′ − f′
    let bfl = sgn * m11 * n1 * efl;
    let fz = bfl;
    let pz = fz - fi;
    // 物体空間（面 1 から）: 焦点 F は出ていく光が平行になる点、主点 P = F − f
    let ffl = -m22 * n0 * efl;
    let fz0 = ffl;
    let pz0 = fz0 - fo;
    // 節点は主点から (n′ − n)/φ、反主点は焦点について主点の反対側、反節点は焦点について節点の反対側
    let dn = (n1s - n0) * efl;
    let (nz0, nz) = (pz0 + dn, pz + dn);
    // 最後の面から像面までの局所 z の距離
    let t_img = if last >= 1 { sf[last].t } else { 0.0 };
    Cardinal {
        n_object: n0,
        n_image: n1s,
        efl,
        f_object: fo,
        f_image: fi,
        focal_object: fz0,
        principal_object: pz0,
        anti_principal_object: 2.0 * fz0 - pz0,
        nodal_object: nz0,
        anti_nodal_object: 2.0 * fz0 - nz0,
        focal_image: fz - t_img,
        principal_image: pz - t_img,
        anti_principal_image: 2.0 * fz - pz - t_img,
        nodal_image: nz - t_img,
        anti_nodal_image: 2.0 * fz - nz - t_img,
        bfl,
        ffl,
    }
}
