//! 光路長のパラメータ偏微分 dOPL/dp を、1 回の追跡の交点から求める。
//!
//! フェルマーの原理: 実際の光線の光路長は、端点を固定したまま途中の交点を面に沿って動かしても 1 次で変わらない。
//! よって系のパラメータを変えたときの光路長の変化は、各面の交点を「面に貼り付けたまま」（面の局所座標の
//! x・y を固定し、z は面の形に従う）動かし、光路長をそのまま微分すればよい:
//!
//!   dOPL/dp = Σ_面 δ_j · (n_in k_in − n_out k_out) + Σ_区間 (dn/dp) s − n_0 k_0 · δ_0
//!
//! ここで δ_j は面 j の交点の動き（局所座標）、k は面の前後の方向（局所座標）、s は区間の長さ。
//! 最後の面は出ていく光がないので n_in k_in だけ、物点は −n_0 k_0 · δ_0 だけを持つ。
//!
//! 求める量: 追跡の光路長 `RayOut::opl`（物点、無限遠物体では面 1 の頂点を通り光線に垂直な面から、最後の面まで）を、
//! 始点（無限遠物体では始点の面）と最後の面での交点（その面の局所座標の x・y）を固定して微分したもの。
//! 座標は物体に固定する（面 1 より前の物体空間の座標系。大域座標の基準面 GLRS には依らない）。
//! 光線の開始方向や瞳座標を固定した微分ではない（最後の面での交点の移動分 n k·Δ を含まない）。
//!
//! 対象のパラメータは [`crate::param`] と同じ（物点の移動 objx・objy は有限物体のみ）。理想レンズの面を通る追跡は未対応。

use crate::error::Result;
use crate::math::*;
use crate::param::{Kind, Param, ThickSolve};
use crate::surface::{After, SurfType, Tilt};
use crate::system::{FieldSel, Trace};
use crate::System;
use crate::{invalid, notimpl};
use rayon::prelude::*;

/// 回転行列の角度 [deg] による微分
fn drot(ax: usize, a_deg: f64) -> M3 {
    let a = a_deg.to_radians();
    let (s, c) = a.sin_cos();
    let k = std::f64::consts::PI / 180.0;
    let m = match ax {
        0 => [[0.0, 0.0, 0.0], [0.0, -s, -c], [0.0, c, -s]],
        1 => [[-s, 0.0, c], [0.0, 0.0, 0.0], [-c, 0.0, -s]],
        _ => [[-s, -c, 0.0], [c, -s, 0.0], [0.0, 0.0, 0.0]],
    };
    m.map(|r| r.map(|v| v * k))
}

fn madd(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = a[i][j] + b[i][j];
        }
    }
    r
}

/// 座標系と、その 1 つのパラメータによる微分
#[derive(Debug, Clone, Copy)]
pub(crate) struct DFrame {
    pub(crate) o: V3,
    pub(crate) r: M3,
    pub(crate) d_o: V3,
    pub(crate) d_r: M3,
}

const Z3: M3 = [[0.0; 3]; 3];

/// 傾き偏心 t（その値の微分 dt）をかける。system::apply_cb の微分
fn apply_cb_d(f: DFrame, t: &Tilt, dt: &[f64; 5]) -> DFrame {
    let [dx, dy, tx, ty, tz] = t.d;
    let (rx, ry, rz) = (rot(0, tx), rot(1, ty), rot(2, tz));
    let (drx, dry, drz) = (
        drot(0, tx).map(|r| r.map(|v| v * dt[2])),
        drot(1, ty).map(|r| r.map(|v| v * dt[3])),
        drot(2, tz).map(|r| r.map(|v| v * dt[4])),
    );
    let sh = [dx, dy, 0.0];
    let dsh = [dt[0], dt[1], 0.0];
    if t.order == 0 {
        let o = add(f.o, mv(&f.r, sh));
        let d_o = add(add(f.d_o, mv(&f.d_r, sh)), mv(&f.r, dsh));
        let q = mm(&mm(&rx, &ry), &rz);
        let dq = madd(
            &madd(&mm(&mm(&drx, &ry), &rz), &mm(&mm(&rx, &dry), &rz)),
            &mm(&mm(&rx, &ry), &drz),
        );
        DFrame {
            o,
            r: mm(&f.r, &q),
            d_o,
            d_r: madd(&mm(&f.d_r, &q), &mm(&f.r, &dq)),
        }
    } else {
        let q = mm(&mm(&rz, &ry), &rx);
        let dq = madd(
            &madd(&mm(&mm(&drz, &ry), &rx), &mm(&mm(&rz, &dry), &rx)),
            &mm(&mm(&rz, &ry), &drx),
        );
        let r = mm(&f.r, &q);
        let d_r = madd(&mm(&f.d_r, &q), &mm(&f.r, &dq));
        DFrame {
            o: add(f.o, mv(&r, sh)),
            r,
            d_o: add(add(f.d_o, mv(&d_r, sh)), mv(&r, dsh)),
            d_r,
        }
    }
}

/// 物体に固定した座標系での面ごとの座標系と、パラメータ p による微分（system::frames と同じ組み立て）
pub(crate) fn raw_frames_d(sys: &System, p: Param) -> Vec<Option<DFrame>> {
    let sf = &sys.surfaces;
    let mut out = vec![None; sf.len()];
    let t0 = sf[0].t;
    if t0.is_finite() {
        out[0] = Some(DFrame {
            o: [0.0, 0.0, -t0],
            r: I3,
            d_o: if p == Param::new(0, Kind::Thick) {
                [0.0, 0.0, -1.0]
            } else {
                [0.0; 3]
            },
            d_r: Z3,
        });
    }
    let mut f = DFrame {
        o: [0.0; 3],
        r: I3,
        d_o: [0.0; 3],
        d_r: Z3,
    };
    // 玉・群の範囲の入口（動かす前）
    let mut entry: Vec<Vec<Option<DFrame>>> = sys
        .units
        .iter()
        .map(|u| vec![None; u.spans.len()])
        .collect();
    for (j, d) in sf.iter().enumerate().skip(1) {
        for (u, x) in sys.units.iter().enumerate() {
            for (k, sp) in x.spans.iter().enumerate() {
                if sp[0] == j {
                    entry[u][k] = Some(f);
                }
            }
        }
        if d.ignore {
            continue;
        }
        let mut dt = [0.0; 5];
        let mine = p.surf == j && !p.kind.is_unit();
        if mine {
            if let Some(i) = p.kind.tilt_index() {
                dt[i] = 1.0;
            }
        }
        let tilt_here = mine && p.kind.tilt_index().is_some();
        if d.typ == SurfType::CoordBrk {
            f = apply_cb_d(
                f,
                &Tilt {
                    d: d.cb,
                    order: d.cb_order,
                },
                &dt,
            );
            out[j] = Some(f);
        } else {
            let pre = f;
            // 傾き偏心のない面に与える微分は、param::set と同じく「前」を 0 で作り、後は「前を戻す」
            let before = d.before.or(tilt_here.then_some(Tilt {
                d: [0.0; 5],
                order: 0,
            }));
            let after = if tilt_here && d.before.is_none() && d.after == After::None {
                After::Reverse
            } else {
                d.after
            };
            if let Some(b) = &before {
                f = apply_cb_d(f, b, &dt);
            }
            out[j] = Some(f);
            match &after {
                After::Reverse => f = pre,
                After::Tilt(a) => f = apply_cb_d(f, a, &[0.0; 5]),
                After::None => {}
            }
        }
        let ez = col(&f.r, 2);
        let dthick = if mine && p.kind == Kind::Thick {
            1.0
        } else {
            0.0
        };
        f.d_o = add(f.d_o, add(scale(col(&f.d_r, 2), d.t), scale(ez, dthick)));
        f.o = add(f.o, scale(ez, d.t));
    }
    // 玉・群の剛体の動き（units::apply_frames と同じ順）と、その微分
    let dp = match p.kind {
        Kind::Unit(k) => k.motion_index().map(|i| (p.surf, i)),
        _ => None,
    };
    let mut ord = crate::units::order(&sys.units);
    if let Some((u, _)) = dp {
        // 動いていない玉・群の動きの微分も要る（値 0 のまわり）
        if !ord.contains(&u) {
            let mut all: Vec<usize> = ord.clone();
            all.push(u);
            all.sort_by_key(|&i| {
                (
                    sys.units[i]
                        .spans
                        .iter()
                        .map(|s| s[1] + 1 - s[0])
                        .sum::<usize>(),
                    i,
                )
            });
            ord = all;
        }
    }
    for u in ord {
        let Some(e) = entry[u][0] else { continue };
        let unit = &sys.units[u];
        let (rm, t) = unit.local();
        let (drm, dt) = match dp {
            Some((v, i)) if v == u => unit.local_d(i),
            _ => (Z3, [0.0; 3]),
        };
        // Q = E R Eᵀ、c = Eo + E t − Q Eo とその微分
        let et = transpose(&e.r);
        let det = transpose(&e.d_r);
        let q = mm(&mm(&e.r, &rm), &et);
        let dq = madd(
            &madd(&mm(&mm(&e.d_r, &rm), &et), &mm(&mm(&e.r, &drm), &et)),
            &mm(&mm(&e.r, &rm), &det),
        );
        let c = sub(add(e.o, mv(&e.r, t)), mv(&q, e.o));
        let dc = sub(
            sub(add(add(e.d_o, mv(&e.d_r, t)), mv(&e.r, dt)), mv(&dq, e.o)),
            mv(&q, e.d_o),
        );
        for s in &unit.spans {
            for x in out[s[0]..=s[1]].iter_mut().flatten() {
                *x = DFrame {
                    o: add(c, mv(&q, x.o)),
                    r: mm(&q, &x.r),
                    d_o: add(add(dc, mv(&dq, x.o)), mv(&q, x.d_o)),
                    d_r: madd(&mm(&dq, &x.r), &mm(&q, &x.d_r)),
                };
            }
        }
    }
    out
}

/// サグの、形のパラメータによる微分（r2 = x² + y²）
pub(crate) fn dsag(c: f64, k: f64, kind: Kind, r2: f64) -> f64 {
    let root = (1.0 - (1.0 + k) * c * c * r2).max(0.0).sqrt();
    match kind {
        Kind::Curv | Kind::Radius => {
            let dc = r2 / (1.0 + root)
                + if c != 0.0 {
                    c * c * r2 * r2 * (1.0 + k) / ((1.0 + root) * (1.0 + root) * root)
                } else {
                    0.0
                };
            if kind == Kind::Radius {
                // dc/dR = −c²
                -c * c * dc
            } else {
                dc
            }
        }
        Kind::Conic => c * c * c * r2 * r2 / (2.0 * root * (1.0 + root) * (1.0 + root)),
        Kind::Asph(i) => r2.powi(i as i32 + 1),
        _ => 0.0,
    }
}

/// 面 d の矢高の、形のパラメータ kind による微分（面の局所座標の x・y で）
pub(crate) fn shape_dz(d: &crate::surface::Surface, kind: Kind, x: f64, y: f64) -> f64 {
    match (kind, &d.zern) {
        (Kind::Zern(j), Some(z)) => z.term(j as usize, x, y).0,
        (Kind::Zern(_), None) => 0.0,
        _ => dsag(d.c, d.k, kind, x * x + y * y),
    }
}

/// 1 つのパラメータの、面ごとの交点の動き δ = a + b x（局所座標）と、媒質の屈折率の微分
pub(crate) struct PrepParam {
    pub(crate) kind: Kind,
    pub(crate) surf: usize,
    /// 面ごとの交点の動き（物体に固定した座標系で δ = A + Ω X、X は交点の位置。None は動かない）
    pub(crate) glob: Vec<Option<(V3, M3)>>,
    /// 面ごとの、面の後の媒質の屈折率の微分（波長 w）
    pub(crate) dn: Vec<f64>,
    /// 物点の動き（物体面の局所座標）
    pub(crate) d0: V3,
}

impl System {
    /// 面 j の後の媒質の屈折率を決める面（鏡は前の媒質を引き継ぐ。座標ブレークは媒質を持たない）
    pub(crate) fn medium_owner(&self) -> Vec<usize> {
        let mut own = vec![0usize; self.surfaces.len()];
        let mut cur = 0usize;
        for (j, d) in self.surfaces.iter().enumerate() {
            if j > 0 && (d.mirror || d.typ == SurfType::CoordBrk) {
                own[j] = cur;
            } else {
                own[j] = j;
                if !d.ignore {
                    cur = j;
                }
            }
        }
        own
    }

    pub(crate) fn prep_param(&self, p: Param, w: usize, owner: &[usize]) -> Result<PrepParam> {
        self.check_param(p)?;
        let ns = self.surfaces.len();
        let mut pp = PrepParam {
            kind: p.kind,
            surf: p.surf,
            glob: vec![None; ns],
            dn: vec![0.0; ns],
            d0: [0.0; 3],
        };
        match p.kind {
            Kind::ObjX | Kind::ObjY => {
                if self.infinite() {
                    invalid!("無限遠物体で物点の移動 {}", p.kind.name());
                }
                pp.d0 = if p.kind == Kind::ObjX {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 1.0, 0.0]
                };
            }
            Kind::Unit(k) if k.motion_index().is_none() => {
                invalid!(
                    "内部エラー: 玉・群の {} は面のパラメータに展開してから",
                    k.name()
                );
            }
            Kind::Thick | Kind::Dx | Kind::Dy | Kind::Tx | Kind::Ty | Kind::Tz | Kind::Unit(_) => {
                if p.kind == Kind::Thick && p.surf == 0 && self.infinite() {
                    invalid!("無限遠物体の物体距離");
                }
                let fr = raw_frames_d(self, p);
                for (j, f) in fr.iter().enumerate() {
                    if let Some(f) = f {
                        if j == 0 {
                            pp.d0 = f.d_o;
                            continue;
                        }
                        // 局所 x の点は X = o + R x、動きは d_o + d_r x = d_o + Ω (X − o)（Ω = d_r Rᵀ）
                        let om = mm(&f.d_r, &transpose(&f.r));
                        let a = sub(f.d_o, mv(&om, f.o));
                        if a != [0.0; 3] || om != Z3 {
                            pp.glob[j] = Some((a, om));
                        }
                    }
                }
            }
            Kind::Dn | Kind::Disp => {
                let m = p.surf;
                let v = if p.kind == Kind::Dn {
                    1.0
                } else {
                    self.base_index(m, w) - self.base_index(m, self.primary)
                };
                for j in 0..ns {
                    if owner[j] == m {
                        pp.dn[j] = v;
                    }
                }
            }
            _ => {}
        }
        Ok(pp)
    }

    /// 面ごとの記録つきの追跡 t（`record = true`、波長 w）から、光線ごとの dOPL/dp を求める。
    /// 結果は光線ごとに params の順（行優先 [光線][パラメータ]）。止まった光線は NaN
    ///
    /// ピックアップ元のパラメータは、ピックアップ先が一緒に動いた分を含む（[`crate::param`] で変えたときと同じ）。
    /// ピックアップ先のパラメータは、元とのつながりを無視した偏微分。
    pub fn opl_grad(&self, t: &Trace, w: usize, params: &[Param]) -> Result<Vec<f64>> {
        self.grad_with_pickups(params, t.rays.len(), |p| self.opl_grad_raw(t, w, p))
    }

    /// p がソルブで決まる量を動かしうるか（ソルブの微分を足す面のパラメータか）
    fn moves_solves(&self, p: Param) -> bool {
        !(self.solves.is_empty()
            || p.kind.is_object()
            || p.kind.is_unit()
            || self.pickup_of(p).is_some()
            || self.solves.iter().any(|q| {
                let k = if p.kind == Kind::Radius {
                    Kind::Curv
                } else {
                    p.kind
                };
                q.surf == p.surf && q.kind.param_kind() == k
            }))
    }

    /// params それぞれを変えたとき一緒に動くパラメータと倍率: ピックアップの先と、ソルブで決まる
    /// 厚さ・曲率（とそのピックアップの先）。ソルブの量の変わり方 ds/dp は陰関数の式で
    /// （[`System::solve_sensitivity`]）、その式が使えないとき（条件がソルブの量で変わらないなど）は
    /// ソルブを解き直した系の中心差分
    fn full_chains(&self, params: &[Param]) -> Result<Vec<Vec<(Param, f64)>>> {
        // 玉・群のパラメータの組合せは面のパラメータに分ける
        let bases: Vec<Vec<(Param, f64)>> = params
            .iter()
            .map(|&p| {
                if p.kind.is_unit() && !p.kind.is_unit_motion() && !self.solves.is_empty() {
                    self.unit_linear(p)
                } else {
                    Ok(vec![(p, 1.0)])
                }
            })
            .collect::<Result<_>>()?;
        let mut need: Vec<Param> = Vec::new();
        for &(q, _) in bases.iter().flatten() {
            if self.moves_solves(q) && !need.contains(&q) {
                need.push(q);
            }
        }
        let qs = self.active_solves();
        let sens: Vec<Vec<f64>> = match self.solve_sensitivity(&need) {
            Ok(v) => v,
            Err(_) => need
                .iter()
                .map(|&p| self.solve_sensitivity_resolved(p, &qs))
                .collect::<Result<_>>()?,
        };
        bases
            .iter()
            .zip(params)
            .map(|(b, &p)| {
                if b.len() == 1 && b[0].0 == p && !need.contains(&p) {
                    return self.pickup_chain(p);
                }
                let mut out = Vec::new();
                for &(q, f) in b {
                    for (r, g) in self.pickup_chain(q)? {
                        out.push((r, f * g));
                    }
                    let Some(i) = need.iter().position(|&x| x == q) else {
                        continue;
                    };
                    for (sq, &dt) in qs.iter().zip(&sens[i]) {
                        if dt != 0.0 && dt.is_finite() {
                            let r = Param::new(sq.surf, sq.kind.param_kind());
                            for (r, g) in self.pickup_chain(r)? {
                                out.push((r, f * dt * g));
                            }
                        }
                    }
                }
                Ok(out)
            })
            .collect()
    }

    /// ソルブで決まる量（qs の順）の p への微分を、ソルブを解き直した系の中心差分で
    fn solve_sensitivity_resolved(&self, p: Param, qs: &[ThickSolve]) -> Result<Vec<f64>> {
        let v = self.get_param(p)?;
        let h = 1e-6 * (1.0 + v.abs());
        let (a, b) = (
            self.with_params_fast(&[(p, v + h)])?,
            self.with_params_fast(&[(p, v - h)])?,
        );
        qs.iter()
            .map(|q| {
                let r = Param::new(q.surf, q.kind.param_kind());
                Ok((a.get_param(r)? - b.get_param(r)?) / (2.0 * h))
            })
            .collect()
    }

    /// パラメータごとの微分（行優先 [行][パラメータ]）を求める関数 raw に、ピックアップ・厚さのソルブで一緒に動く
    /// パラメータを足した微分を返す（ピックアップ元の微分 = Σ 倍率 × 先の微分）。微分を返すほかの関数でも使う
    pub(crate) fn grad_with_pickups(
        &self,
        params: &[Param],
        rows: usize,
        raw: impl Fn(&[Param]) -> Result<Vec<f64>>,
    ) -> Result<Vec<f64>> {
        let linear_unit = |p: &Param| p.kind.is_unit() && !p.kind.is_unit_motion();
        if !self.macro_parms.is_empty() {
            crate::notimpl!("パラメータの ZPL マクロのソルブがある系の解析的な微分");
        }
        if self.pickups.is_empty() && self.solves.is_empty() && !params.iter().any(linear_unit) {
            return raw(params);
        }
        let chains = self.full_chains(params)?;
        if chains
            .iter()
            .zip(params)
            .all(|(c, p)| c.len() == 1 && c[0] == (*p, 1.0))
        {
            return raw(params);
        }
        let mut flat: Vec<Param> = Vec::new();
        let idx: Vec<Vec<(usize, f64)>> = chains
            .iter()
            .map(|c| {
                c.iter()
                    .map(|&(q, f)| {
                        let i = flat.iter().position(|&x| x == q).unwrap_or_else(|| {
                            flat.push(q);
                            flat.len() - 1
                        });
                        (i, f)
                    })
                    .collect()
            })
            .collect();
        let g = raw(&flat)?;
        let (np, nf) = (params.len(), flat.len());
        let mut out = vec![0.0; rows * np];
        for r in 0..rows {
            for (q, c) in idx.iter().enumerate() {
                out[r * np + q] = c.iter().map(|&(i, f)| f * g[r * nf + i]).sum();
            }
        }
        Ok(out)
    }

    fn opl_grad_raw(&self, t: &Trace, w: usize, params: &[Param]) -> Result<Vec<f64>> {
        let n = t.rays.len();
        let np = params.len();
        let all = (1..=t.last).filter(|&j| !self.surfaces[j].ignore).count();
        if n > 0 && t.rec.len() != all {
            invalid!("dOPL/dp には全面の記録（record）つきの追跡が要る");
        }
        for j in 1..=t.last {
            let d = &self.surfaces[j];
            if !d.ignore && matches!(d.typ, SurfType::Paraxial | SurfType::User) {
                notimpl!("理想レンズ・利用者定義の面（面 {j}）を通る光線の dOPL/dp");
            }
            if !d.ignore
                && matches!(
                    d.typ,
                    SurfType::Fresnel
                        | SurfType::CylFresnel
                        | SurfType::Binary2
                        | SurfType::Gradium
                        | SurfType::Gradient9
                )
            {
                notimpl!("面 {j}（{:?}）を通る光線の dOPL/dp", d.typ);
            }
        }
        let owner = self.medium_owner();
        let pre: Vec<PrepParam> = params
            .iter()
            .map(|&p| self.prep_param(p, w, &owner))
            .collect::<Result<_>>()?;
        let fr = self.frames();
        let sf = &self.surfaces;
        // 面ごとの前後の屈折率と、前の媒質を決める面
        let ns = t.rec.len();
        let mut n_in = vec![0.0; ns];
        let mut n_out = vec![0.0; ns];
        let mut own_in = vec![0usize; ns];
        for (k, rs) in t.rec.iter().enumerate() {
            let j = rs.surf;
            n_in[k] = self.index_before(j, w);
            let d = &sf[j];
            n_out[k] = if d.mirror || d.typ == SurfType::CoordBrk {
                n_in[k]
            } else {
                d.n[w]
            };
            // index_before と同じ: 直前の無視しない・座標ブレークでない面
            let prev = (0..j)
                .rev()
                .find(|&i| i == 0 || (!sf[i].ignore && sf[i].typ != SurfType::CoordBrk))
                .unwrap_or(0);
            own_in[k] = prev;
        }
        let f0 = fr.surf[0];
        // 物体に固定した座標系での各面の座標系（パラメータの動き glob と同じ座標）
        let base = raw_frames_d(self, Param::new(0, Kind::Dn));
        let br: Vec<(V3, M3)> = t
            .rec
            .iter()
            .map(|rs| {
                let f = base[rs.surf].expect("記録した面の座標系");
                (f.o, f.r)
            })
            .collect();
        // パラメータごとに、同じ剛体運動 (A, Ω) が続く記録の範囲 [s, e) をまとめる
        // （厚さ・偏心・傾きは下流の面を一緒に動かすので、範囲は少ない）
        let same = |p: &(V3, M3), q: &(V3, M3)| {
            (0..3).all(|i| (p.0[i] - q.0[i]).abs() <= 1e-12 * (1.0 + p.0[i].abs()))
                && (0..3).all(|i| {
                    (0..3).all(|j| (p.1[i][j] - q.1[i][j]).abs() <= 1e-14 * (1.0 + p.1[i][j].abs()))
                })
        };
        struct Run {
            s: usize,
            e: usize,
            a: V3,
            om: M3,
            rot: bool,
        }
        let runs: Vec<Vec<Run>> = pre
            .iter()
            .map(|pp| {
                let mut out: Vec<Run> = Vec::new();
                for (k, rs) in t.rec.iter().enumerate() {
                    let Some(m) = pp.glob[rs.surf] else { continue };
                    if let Some(r) = out.last_mut() {
                        if r.e == k && same(&(r.a, r.om), &m) {
                            r.e = k + 1;
                            continue;
                        }
                    }
                    out.push(Run {
                        s: k,
                        e: k + 1,
                        a: m.0,
                        om: m.1,
                        rot: m.1 != Z3,
                    });
                }
                out
            })
            .collect();
        // 屈折率の微分がかかる区間（疎）と、サグの微分がかかる記録の番号
        let dn_ks: Vec<Vec<(usize, f64)>> = pre
            .iter()
            .map(|pp| {
                (0..ns)
                    .filter_map(|k| {
                        let v = pp.dn[own_in[k]];
                        (v != 0.0).then_some((k, v))
                    })
                    .collect()
            })
            .collect();
        let sag_k: Vec<Option<usize>> = pre
            .iter()
            .map(|pp| {
                if matches!(
                    pp.kind,
                    Kind::Curv | Kind::Radius | Kind::Conic | Kind::Asph(_) | Kind::Zern(_)
                ) {
                    t.rec.iter().position(|rs| rs.surf == pp.surf)
                } else {
                    None
                }
            })
            .collect();
        let any_rot = runs.iter().flatten().any(|r| r.rot);
        let one = |i: usize| -> Vec<f64> {
            let mut g = vec![0.0; np];
            if t.rays[i].err != 0 {
                return vec![f64::NAN; np];
            }
            // 面ごとの gv（局所）と区間の長さ、G = R gv と G Xᵀ の後ろからの累積和
            let mut gvl = vec![[0.0; 3]; ns];
            let mut seg = vec![0.0; ns];
            let mut sg = vec![[0.0; 3]; ns + 1];
            let mut sgx = if any_rot {
                vec![[[0.0; 3]; 3]; ns + 1]
            } else {
                Vec::new()
            };
            let mut kprev: V3 = t.k0[i];
            let mut opl_prev = 0.0;
            for (k, rs) in t.rec.iter().enumerate() {
                let j = rs.surf;
                let kout = rs.lmn[i];
                // 入ってくる方向（この面の局所座標）
                let kin = if k == 0 {
                    mtv(&fr.surf[j].unwrap().r, kprev)
                } else {
                    match fr.rel[j] {
                        Some(q) if q.shift.is_none() => mv(&q.r, kprev),
                        _ => kprev,
                    }
                };
                gvl[k] = if k + 1 == ns {
                    scale(kin, n_in[k])
                } else {
                    sub(scale(kin, n_in[k]), scale(kout, n_out[k]))
                };
                seg[k] = (rs.opl[i] - opl_prev) / n_in[k];
                kprev = kout;
                opl_prev = rs.opl[i];
            }
            for k in (0..ns).rev() {
                let (o, r) = &br[k];
                let gk = mv(r, gvl[k]);
                sg[k] = add(sg[k + 1], gk);
                if any_rot {
                    let xk = add(*o, mv(r, t.rec[k].xyz[i]));
                    let mut m = sgx[k + 1];
                    for a in 0..3 {
                        for b in 0..3 {
                            m[a][b] += gk[a] * xk[b];
                        }
                    }
                    sgx[k] = m;
                }
            }
            for q in 0..np {
                let mut v = 0.0;
                for r in &runs[q] {
                    let dg = sub(sg[r.s], sg[r.e]);
                    v += dot(r.a, dg);
                    if r.rot {
                        // Σ (Ω X)·G = Σ_ab Ω_ab (G Xᵀ)_ab
                        for a in 0..3 {
                            for b in 0..3 {
                                v += r.om[a][b] * (sgx[r.s][a][b] - sgx[r.e][a][b]);
                            }
                        }
                    }
                }
                for &(k, dn) in &dn_ks[q] {
                    v += dn * seg[k];
                }
                if let Some(k) = sag_k[q] {
                    let x = t.rec[k].xyz[i];
                    let d = &sf[t.rec[k].surf];
                    v += shape_dz(d, pre[q].kind, x[0], x[1]) * gvl[k][2];
                }
                g[q] = v;
            }
            // 物点の動き
            if let Some(f0) = f0 {
                let k0 = mtv(&f0.r, t.k0[i]);
                let n0 = n_in[0];
                for (q, pp) in pre.iter().enumerate() {
                    if pp.d0 != [0.0; 3] {
                        g[q] -= n0 * dot(k0, pp.d0);
                    }
                }
            }
            g
        };
        let rows: Vec<Vec<f64>> = if n >= 256 && !crate::par::sequential() {
            (0..n).into_par_iter().with_min_len(64).map(one).collect()
        } else {
            (0..n).map(one).collect()
        };
        Ok(rows.concat())
    }

    /// 視野の光線を追跡し、光線ごとの dOPL/dp も返す（[`System::opl_grad`]）
    #[allow(clippy::too_many_arguments)]
    pub fn trace_grad(
        &self,
        field: FieldSel,
        px: &[f64],
        py: &[f64],
        w: Option<usize>,
        last: Option<usize>,
        params: &[Param],
    ) -> Result<(Trace, Vec<f64>)> {
        let w = w.unwrap_or(self.primary);
        let t = self.try_trace(field, px, py, Some(w), last, true)?;
        let g = self.opl_grad(&t, w, params)?;
        Ok((t, g))
    }
}

#[cfg(test)]
mod tests {
    use crate::param::Param;
    use crate::System;

    const HEAD: &str =
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 3 1 0 0 0\n\
XFLN 0 0 0\nYFLN 0 14 20\nFWGN 1 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";

    /// 三枚玉（絞りは面 5、tests/zmx_solves2.rs と同じ）。curv は CURV 行の置き換え、extra は足す行
    fn triplet(curv: &[(usize, &str)], extra: &[(usize, &str)]) -> System {
        let g1 = "  GLAS ___BLANK 1 0 1.6204 60.3 0 0 0 0 0\n";
        let g2 = "  GLAS ___BLANK 1 0 1.62 36.4 0 0 0 0 0\n";
        let base: [(f64, &str, &str); 7] = [
            (1.0 / 23.6, "3.5", g1),
            (-1.0 / 360.0, "5.8", ""),
            (-1.0 / 23.4, "1.2", g2),
            (1.0 / 21.5, "2.3", ""),
            (0.0, "2.6", ""),
            (1.0 / 88.0, "3.2", g1),
            (-1.0 / 19.1, "45.4", ""),
        ];
        let mut t = HEAD.to_string();
        for (i, (c, d, g)) in base.iter().enumerate() {
            let k = i + 1;
            t += &format!("SURF {k}\n");
            if k == 5 {
                t += "  STOP\n";
            }
            t += "  TYPE STANDARD\n";
            match curv.iter().find(|x| x.0 == k) {
                Some((_, l)) => t += &format!("  {l}\n"),
                None => t += &format!("  CURV {c:e}\n"),
            }
            t += &format!("  DISZ {d}\n{g}  DIAM 5 1 0 0 1 \"\"\n");
            for (_, l) in extra.iter().filter(|x| x.0 == k) {
                t += &format!("  {l}\n");
            }
        }
        t += "SURF 8\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
        let raw = crate::zmx::parse_text(&t).unwrap();
        crate::zmx::build(&raw, &Default::default(), &[], None).unwrap()
    }

    /// 陰関数の式による ds/dpが、ソルブを解き直した系の中心差分（従来の求め方）と合う。
    /// 曲率・厚さのソルブの種類ごとと、ソルブが連なる系で
    #[test]
    fn solve_sensitivity_matches_resolved_differences() {
        let curv: [&str; 9] = [
            "2 -0.1", "3 -0.1", "5 0", "6 0", "7 0", "8 0.02", "9 3", "10 6", "11 4.5",
        ];
        let thick: [(usize, &str); 8] = [
            (6, "CHZH 0.5 0"),
            (7, "TPUP 0 0"),
            (6, "EDGE 1.5 4"),
            (6, "TOLE 5 10"),
            (4, "TCOM 2 8"),
            (6, "TCOC 6 0"),
            (7, "MAZH 0.2 0.7"),
            (7, "MAZH 0 0"),
        ];
        let mut systems: Vec<(String, System)> = Vec::new();
        for (i, code) in curv.iter().enumerate() {
            let k = if i == 1 || i == 5 || i == 7 { 7 } else { 6 };
            let line = format!("CURV 0.5 {code} 0 0 \"\"");
            systems.push((
                format!("CURV {code}"),
                triplet(&[(k, &line)], &[(7, "MAZH 0 0")]),
            ));
        }
        for (k, line) in thick {
            systems.push((line.to_string(), triplet(&[], &[(k, line)])));
        }
        // 位置・補償・曲率の F/#・像面の厚さが連なる系（面 4 の補償は位置のソルブで決まる面 2 の厚さを使う）
        systems.push((
            "連なる".into(),
            triplet(
                &[(6, "CURV 0.5 11 4.5 0 0 \"\"")],
                &[(2, "TOLE 1 10"), (4, "TCOM 2 8"), (7, "MAZH 0 0")],
            ),
        ));
        let mut worst = 0.0f64;
        for (name, s) in &systems {
            let qs = s.active_solves();
            let ps: Vec<Param> = (1..s.image())
                .flat_map(|j| ["c", "t", "dn", "k"].map(|k| format!("{j}.{k}")))
                .filter_map(|n| s.param(&n).ok())
                .filter(|&p| s.moves_solves(p))
                .collect();
            let a = s
                .solve_sensitivity(&ps)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            for (p, ai) in ps.iter().zip(&a) {
                let b = s.solve_sensitivity_resolved(*p, &qs).unwrap();
                for (x, y) in ai.iter().zip(&b) {
                    let e = (x - y).abs() / (1.0 + y.abs());
                    worst = worst.max(e);
                    assert!(e < 1e-6, "{name} {}: {x} {y}", p.name());
                }
            }
        }
        eprintln!("ds/dp の最大の差（相対）: {worst:.2e}");
    }
}
