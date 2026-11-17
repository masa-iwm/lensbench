//! 偏光追跡の入口（面ごとの偏光の準備）

use super::*;
use crate::action::Action;

impl System {
    /// 偏光追跡での面ごとの処理を波長 w について用意する（境界の係数、Jones 行列、媒質の吸収）
    pub fn pol_surfaces(&self, w: usize, coat: &Coatings) -> Result<Vec<PolSurf>> {
        let (keys, before) = coating::media_keys(self);
        let wl = self.wavelengths[w];
        let mut out = Vec::with_capacity(self.surfaces.len());
        for (j, d) in self.surfaces.iter().enumerate() {
            let it = self
                .glasses
                .get(&before[j])
                .and_then(|g| coating::internal_t(g, wl));
            let kind = if d.ignore || j == 0 || d.is_coord_break() {
                PolKind::None
            } else {
                match d.action() {
                    Action::Jones(m) => {
                        let a = m.jones8(wl);
                        PolKind::Jones([
                            C64::new(a[0], a[1]),
                            C64::new(a[2], a[3]),
                            C64::new(a[4], a[5]),
                            C64::new(a[6], a[7]),
                        ])
                    }
                    Action::Paraxial { .. } | Action::User(_) => PolKind::None,
                    _ => match &keys[j] {
                        Some(k) => PolKind::Iface(
                            coat.prepare(
                                self,
                                k,
                                wl,
                                self.index_before(j, w),
                                match &d.reflect {
                                    Some(r) if d.is_mirror() => r.n[w],
                                    _ => self.n(j)[w],
                                },
                            )
                            .map_err(|e| e.at(self.surf_label(j)))?,
                        ),
                        None => PolKind::None,
                    },
                }
            };
            let mut ps = PolSurf::new(kind, it);
            if !d.ignore && j > 0 {
                ps.cry = match &d.bire {
                    Some(Bire::In { mode, axis, ne }) => {
                        if !matches!(mode, 0 | 2) {
                            return Err(Error::Invalid(format!(
                                "{}: birefringent plate Mode {mode} is not supported (only 0 and 2)",
                                self.surf_label(j)
                            )));
                        }
                        let ne = ne.get(w).copied().ok_or_else(|| {
                            Error::Invalid(format!(
                                "{}: no index for the extraordinary material {}",
                                self.surf_label(j),
                                Bire::e_name(&d.mat.name())
                            ))
                        })?;
                        Some(CryKind::In {
                            mode: *mode,
                            axis: *axis,
                            no: self.n(j)[w],
                            ne,
                        })
                    }
                    Some(Bire::Out) => Some(CryKind::Out),
                    None => None,
                };
            }
            out.push(ps);
        }
        // 板どうしの貼り合わせ: 出口の面の厚さが 0 で、次の有効な面が同じ形・同じ座標の膜のない入口なら、
        // 境界は 1 つとして出口の面で係数を 1 回かける
        for j in 1..self.surfaces.len() {
            let d = &self.surfaces[j];
            if !matches!(out[j].cry, Some(CryKind::Out)) || d.is_mirror() || d.t != 0.0 {
                continue;
            }
            if !matches!(d.after, After::None) {
                continue;
            }
            let Some(k) = (j + 1..self.surfaces.len()).find(|&k| !self.surfaces[k].ignore) else {
                continue;
            };
            let e = &self.surfaces[k];
            let same_shape = e.typ() == d.typ()
                && e.c == d.c
                && e.k == d.k
                && e.asph() == d.asph()
                && e.cx() == d.cx()
                && d.zern().is_none()
                && d.grid().is_none()
                && e.zern().is_none()
                && e.grid().is_none();
            if e.is_mirror() || !e.coat.is_empty() || e.before.is_some() || !same_shape {
                continue;
            }
            if let Some(CryKind::In { axis, no, ne, .. }) = out[k].cry {
                out[j].cement = Some((axis, no, ne));
                out[k].cemented = true;
            }
        }
        Ok(out)
    }

    /// 偏光追跡: 入力の偏光 pol.pols（物体の座標系の (Ex, Ey)）ごとの電場を、最後の面の局所座標で返す。
    /// pol.phase なら伝搬の位相 exp(−i k OPL) 込み（OpticStudio の偏光追跡と同じ約束）。phase = false なら
    /// 膜・境界・Jones 面の振幅と位相だけ（光路長は `trace.rays[i].opl` にある）
    pub fn try_trace_pol(
        &self,
        field: FieldSel,
        px: &[f64],
        py: &[f64],
        w: Option<usize>,
        pol: PolIn,
    ) -> Result<PolTrace> {
        let w = w.unwrap_or(self.primary);
        let (hx, hy) = self.field_sel_norm(field, w)?;
        let spec = self.object_spec(hx, hy)?;
        let vig = self.vig_factors(hx, hy)?;
        self.trace_pol_spec(
            spec,
            px,
            py,
            w,
            TraceOpts::new().vig(vig).hxy((hx, hy)),
            pol,
        )
    }

    /// 物体の指定（物点・方向）と瞳座標の偏光追跡（[`Self::try_trace_pol`] の物体を直接与える形）。
    /// o はケラレ係数・止める面・面ごとの記録・視野（[`Self::trace_spec`] と同じ。o.vig が None なら視野 o.hxy のもの）
    pub fn trace_pol_spec(
        &self,
        spec: ObjSpec,
        px: &[f64],
        py: &[f64],
        w: usize,
        o: TraceOpts,
        pol: PolIn,
    ) -> Result<PolTrace> {
        let ps = self.pol_surfaces(w, pol.coat)?;
        let (mut t, e, err) = self.trace_spec_inner(
            spec,
            px,
            py,
            w,
            o.vig,
            o.upto,
            o.record,
            o.hxy,
            Some((pol.pols, &ps, pol.phase)),
        );
        if let Some(e) = err {
            return Err(e);
        }
        t.hx_hy = o.hxy.unwrap_or((0.0, 0.0));
        Ok(PolTrace { trace: t, e })
    }
}

/// 偏光追跡の入力: 入力の偏光（物体の座標系の (Ex, Ey)）、膜、伝搬の位相をかけるか
#[derive(Debug, Clone, Copy)]
pub struct PolIn<'a> {
    pub pols: &'a [[C64; 2]],
    pub coat: &'a Coatings,
    /// 伝搬の位相 exp(−i k OPL) をかける（既定 true）
    pub phase: bool,
}

impl<'a> PolIn<'a> {
    pub fn new(pols: &'a [[C64; 2]], coat: &'a Coatings) -> Self {
        Self {
            pols,
            coat,
            phase: true,
        }
    }
    /// 伝搬の位相をかけない
    pub fn no_phase(self) -> Self {
        self.phase(false)
    }
    pub fn phase(mut self, phase: bool) -> Self {
        self.phase = phase;
        self
    }
}
