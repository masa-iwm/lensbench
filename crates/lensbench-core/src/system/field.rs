//! 視野の正規化・ケラレ係数・物体の決め方・絞り半径

use super::*;
use crate::quantity::Angle;

impl System {
    pub fn field_norm_values(&self) -> (f64, f64) {
        let or1 = |v: f64| if v == 0.0 { 1.0 } else { v };
        match self.field_norm {
            FieldNorm::Rect => (
                or1(self.fields.iter().map(|f| f.x.abs()).fold(0.0, f64::max)),
                or1(self.fields.iter().map(|f| f.y.abs()).fold(0.0, f64::max)),
            ),
            FieldNorm::Radial => {
                let r = or1(self
                    .fields
                    .iter()
                    .map(|f| f.x.hypot(f.y))
                    .fold(0.0, f64::max));
                (r, r)
            }
        }
    }

    /// 視野 i の正規化視野
    /// 視野の指定の正規化座標。視野・波長の番号が範囲の外なら誤り（入力で落ちない）
    #[doc(hidden)]
    pub fn field_sel_norm(&self, field: FieldSel, w: usize) -> Result<(f64, f64)> {
        if w >= self.wavelengths.len() {
            let (wn, nw) = (w + 1, self.wavelengths.len());
            invalid!("wavelength number {wn} ({nw} in all)");
        }
        match field {
            FieldSel::Index(i) if i >= self.fields.len() => {
                invalid!("field number {} ({} in all)", i + 1, self.fields.len())
            }
            FieldSel::Index(i) => Ok(self.hxhy(i)),
            FieldSel::Norm(a, b) => Ok((a, b)),
        }
    }

    /// 視野 i の正規化座標（i は範囲の中。範囲の外は panic）
    pub fn hxhy(&self, i: usize) -> (f64, f64) {
        let (nx, ny) = self.field_norm_values();
        (self.fields[i].x / nx, self.fields[i].y / ny)
    }

    /// 視野 (hx, hy) のケラレ係数 [VDX, VDY, VCX, VCY, VAN]（視野の補間込み）。
    ///
    /// 既定（[`VigInterp::Continuous`]）は段差のない補間。[`VigInterp::OpticStudio`] なら OpticStudio 2026 R1.01 と
    /// 同じ規則（25 系 5010 点を ZOS-API で測って 2e-13 で一致）:
    /// - 視野の物理座標 (X, Y) = 正規化視野 × 正規化の値（Radial は最大の半径、Rectangular は x・y それぞれの最大。
    ///   最大が 0 の向きは 0）
    /// - 全視野が x = 0 で y の符号がそろい、X² + Y² ≤ max|y|² なら、視野を |y| の順に並べ、r² = X² + Y² の区分線形で
    ///   補間する（r ではなく r²。最も内側の視野より内は、その視野の値）
    /// - それ以外（x ≠ 0 の視野がある、y の符号が混ざる、外側の点）は補間せず、物理座標の L1 距離
    ///   |X − x_i| + |Y − y_i| が最小の視野の値（同じ距離なら番号の小さい視野）
    pub fn vig_factors(&self, hx: f64, hy: f64) -> Result<[f64; 5]> {
        let vals = |f: &Field| [f.vdx, f.vdy, f.vcx, f.vcy, f.van];
        let (fx, fy) = self.field_norm_values();
        for f in &self.fields {
            if (f.x / fx - hx).abs() < 1e-12 && (f.y / fy - hy).abs() < 1e-12 {
                return Ok(vals(f));
            }
        }
        if self
            .fields
            .iter()
            .all(|f| vals(f).iter().all(|&v| v == 0.0))
        {
            return Ok([0.0; 5]);
        }
        if !(hx.is_finite() && hy.is_finite()) {
            invalid!("vignetting factors of field ({hx}, {hy})");
        }
        // 物理座標（正規化の値の 0 は割り算のための 1 にせず 0 のまま）
        let max_x = self.fields.iter().map(|f| f.x.abs()).fold(0.0, f64::max);
        let max_y = self.fields.iter().map(|f| f.y.abs()).fold(0.0, f64::max);
        let (sx, sy) = match self.field_norm {
            FieldNorm::Rect => (max_x, max_y),
            FieldNorm::Radial => {
                let r = self
                    .fields
                    .iter()
                    .map(|f| f.x.hypot(f.y))
                    .fold(0.0, f64::max);
                (r, r)
            }
        };
        let (x, y) = (hx * sx, hy * sy);
        let r2 = x * x + y * y;
        let y_only = self.fields.iter().all(|f| f.x == 0.0)
            && (self.fields.iter().all(|f| f.y >= 0.0) || self.fields.iter().all(|f| f.y <= 0.0));
        let cont = self.vig_interp == VigInterp::Continuous;
        if y_only && (cont || r2 <= max_y * max_y) {
            let mut ord: Vec<&Field> = self.fields.iter().collect();
            ord.sort_by(|a, b| a.y.abs().total_cmp(&b.y.abs()));
            let first = ord[0];
            if r2 <= first.y * first.y {
                return Ok(vals(first));
            }
            for w in ord.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (a2, b2) = (a.y * a.y, b.y * b.y);
                if r2 <= b2 && b2 > a2 {
                    let t = (r2 - a2) / (b2 - a2);
                    let (va, vb) = (vals(a), vals(b));
                    return Ok(std::array::from_fn(|k| va[k] + (vb[k] - va[k]) * t));
                }
            }
            return Ok(vals(ord[ord.len() - 1]));
        }
        if cont {
            let pts: Vec<[f64; 2]> = self.fields.iter().map(|f| [f.x, f.y]).collect();
            let w = interp_weights(&pts, [x, y]);
            let mut out = [0.0; 5];
            for (i, wi) in w {
                let v = vals(&self.fields[i]);
                for k in 0..5 {
                    out[k] += wi * v[k];
                }
            }
            return Ok(out);
        }
        let mut best = (f64::INFINITY, 0usize);
        for (i, f) in self.fields.iter().enumerate() {
            let d = (x - f.x).abs() + (y - f.y).abs();
            if d < best.0 {
                best = (d, i);
            }
        }
        Ok(vals(&self.fields[best.1]))
    }

    /// 物体の指定（無限遠: 方向、有限: 物体面の点）
    pub fn object_spec(&self, hx: f64, hy: f64) -> Result<ObjSpec> {
        // 視野がすべて 0（軸上だけ）の系では、正規化視野は軸上のまま（OpticStudio 2026 R1 と同じ。正規化の値の
        // 1 は割り算のためだけ）
        let (nx, ny) = if self.on_axis_only() {
            (0.0, 0.0)
        } else {
            self.field_norm_values()
        };
        self.object_spec_abs(hx * nx, hy * ny)
    }

    /// 視野がすべて 0（軸上の 1 点だけ）
    pub(super) fn on_axis_only(&self) -> bool {
        self.fields.iter().all(|f| f.x == 0.0 && f.y == 0.0)
    }

    /// 角度 deg [°] の視野へ近軸主光線（無限遠の物体は物体空間の傾き 1、有限の物体は物体の高さ 1）を合わせる倍率。
    /// 有限の物体では入射瞳の中心へ向かう傾き tan の主光線の物体の高さ −(t₀ + 入射瞳の位置) tan（[`Self::object_spec_abs`] と同じ）
    pub fn angle_chief_scale(&self, deg: f64) -> f64 {
        let t = Angle::from_deg(deg).tan();
        if self.infinite() {
            t
        } else {
            -(self.surfaces[0].t + self.paraxial().enpp) * t
        }
    }

    /// 視野の値 (x, y)（視野の種類の単位、正規化しない）の物体の指定
    pub(super) fn object_spec_abs(&self, x: f64, y: f64) -> Result<ObjSpec> {
        let par = self.paraxial();
        let (tx, ty) = match self.field_type {
            FieldType::RealH => {
                return self.cond_spec(FieldCond::Position { surf: self.image() }, x, y)
            }
            FieldType::Solve(c) => return self.cond_spec(c, x, y),
            FieldType::Angle => (Angle::from_deg(x).tan(), Angle::from_deg(y).tan()),
            FieldType::ParH => {
                let h1 = par.chief_parimage_per_unit;
                if self.infinite() {
                    (x / h1, y / h1)
                } else {
                    return Ok(ObjSpec::Point([x / h1, y / h1]));
                }
            }
            FieldType::ObjH => return Ok(ObjSpec::Point([x, y])),
        };
        if self.infinite() {
            // 角度の視野が 90° を超えると光は物体側の後ろから来る（方向余弦 n < 0。OpticStudio と同じ）。
            // tan のままでは向きが反対になるので符号を戻す。ちょうど 90° は向きが決まらない
            if self.field_type == FieldType::Angle {
                let c = Angle::from_deg(x).cos() * Angle::from_deg(y).cos();
                if c == 0.0 || !c.is_finite() {
                    notimpl!("angle field ({x}°, {y}°): a field of exactly 90°");
                }
                if c < 0.0 {
                    return Ok(ObjSpec::Dir(normalize([-tx, -ty, -1.0])));
                }
            }
            return Ok(ObjSpec::Dir(normalize([tx, ty, 1.0])));
        }
        let l = self.surfaces[0].t + par.enpp;
        Ok(ObjSpec::Point([-l * tx, -l * ty]))
    }

    // ---------------------------------------------------------------------------------- 光線
    /// 入射瞳面の点 (ex, ey) [mm] を通る光線の開始位置・方向（大域）
    pub fn start_ray(&self, spec: ObjSpec, ex: f64, ey: f64) -> (V3, V3) {
        StartGeom::new(self, spec).ray(ex, ey)
    }

    /// 瞳座標 1 が狙う絞りの半径 (rx, ry)。符号つき: 入射瞳の上端を通る近軸光線が絞りで負の側に来る系
    /// （入射瞳が物体より前の有限物体系など）では負（OpticStudio と同じく、px = 1 の光線が絞りの −x 側を通る）
    pub fn stop_radius(&self) -> (f64, f64) {
        *self.cache.rs.get_or_init(|| {
            let par = self.paraxial();
            let st = self.stop();
            // 入射瞳の +x の点を通る近軸光線（有限物体は物点から、入射瞳が物体より前なら遠ざかる向き）の絞りでの高さの符号
            let h = if self.infinite() {
                self.ptrace(1.0, 0.0, self.primary, Some(st)).0[st]
            } else {
                let u = 1.0 / (par.enpp.min(ENPP_MAX) + self.surfaces[0].t);
                self.ptrace(u * self.surfaces[0].t, u, self.primary, Some(st))
                    .0[st]
            };
            let sgn = if h < 0.0 { -1.0 } else { 1.0 };
            if self.aperture.0 == ApertureKind::Float {
                let s = sgn * self.surfaces[self.stop()].semi;
                (s, s)
            } else if self.aiming == Aiming::Real {
                let spec = if self.infinite() {
                    ObjSpec::Dir([0.0, 0.0, 1.0])
                } else {
                    ObjSpec::Point([0.0, 0.0])
                };
                let epd = self.paraxial().epd;
                let st = self.stop();
                let plan = batch::Plan::new(self, self.frames(), self.primary, st, false);
                let go = |ex: f64, ey: f64| {
                    let (p, k) = self.start_ray(spec, ex, ey);
                    batch::trace_one_plain(&plan, p, k)
                };
                (go(epd / 2.0, 0.0).xyz[0], go(0.0, epd / 2.0).xyz[1])
            } else {
                let r = sgn * par.stop_radius;
                (r, r)
            }
        })
    }
}
