//! trace・trace_rays の引数と、GIL なしの追跡（[`compute`]）

use super::*;

/// Arguments shared by trace and trace_rays
pub(crate) struct TraceOpts {
    pub(crate) ws: Vec<usize>,
    pub(crate) last: Option<usize>,
    pub(crate) record: bool,
    /// Surfaces to record (all if None)
    pub(crate) rec_surfs: Option<Vec<usize>>,
    pub(crate) pw: Option<usize>,
    pub(crate) pol_phase: bool,
    /// Polarization trace on a coarse pupil grid (nx, ny), interpolated
    pub(crate) pol_grid: Option<(usize, usize)>,
    pub(crate) globals: bool,
    pub(crate) params: Option<Vec<Param>>,
    pub(crate) threads: Option<usize>,
    /// Put the detector at z = dz in the local coordinates of the last surface
    pub(crate) dz: Option<f64>,
    /// With grad, also return the derivative of the landing point d(x, y, l, m)/dp
    pub(crate) land: bool,
}

impl TraceOpts {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        s: &System,
        wavelengths: Option<Vec<usize>>,
        last: Option<usize>,
        record: RecArg,
        pol: Option<PolArg>,
        pol_phase: bool,
        globals: bool,
        grad: Option<Vec<String>>,
        threads: Option<usize>,
        dz: Option<f64>,
        land: bool,
    ) -> PyResult<TraceOpts> {
        let (record, rec_surfs) = record.surfs();
        let ws: Vec<usize> = match wavelengths {
            None => (0..s.wavelengths.len()).collect(),
            Some(v) => v
                .into_iter()
                .map(|w| wave_index(s, w))
                .collect::<PyResult<_>>()?,
        };
        let img = s.image();
        if let Some(l) = last {
            if l == 0 || l > img {
                return Err(index_err(format!("no surface {l} (1 to {img})")));
            }
        }
        let pw = match pol {
            None | Some(PolArg::Flag(false)) => None,
            Some(PolArg::Flag(true)) => Some(s.primary),
            Some(PolArg::Wave(w)) => Some(wave_index(s, w)?),
        };
        if pw.is_some() && last.is_some_and(|l| l != img) {
            return Err(value_err(
                "pol only when last is left out (up to the image)",
            ));
        }
        let params: Option<Vec<Param>> = grad
            .as_ref()
            .map(|g| g.iter().map(|k| s.param(k)).collect())
            .transpose()
            .map_err(err)?;
        if land && params.is_none() {
            return Err(value_err("give land together with grad"));
        }
        if let Some(z) = dz {
            if !z.is_finite() {
                return Err(value_err(format!("dz is not finite ({z})")));
            }
            if pw.is_some() && pol_phase {
                return Err(value_err(
                    "dz and pol_phase=True can't be used together (the propagation phase of the shift is not included)",
                ));
            }
        }
        Ok(TraceOpts {
            ws,
            last,
            record,
            rec_surfs,
            pw,
            pol_phase,
            pol_grid: None,
            globals,
            params,
            threads,
            dz,
            land,
        })
    }
}

impl PySystem {
    /// Trace with the launch (n rays) and make the result dict (shared part of trace and trace_rays)
    pub(crate) fn run_trace<'py>(
        &self,
        gil: Python<'py>,
        s: &Arc<System>,
        launch: &Launch,
        n: usize,
        o: &TraceOpts,
    ) -> PyResult<Bound<'py, PyDict>> {
        let coat = o.pw.map(|_| self.coatings(s)).transpose()?;
        let out = gil
            .detach(|| with_threads(o.threads, || compute(s, coat.as_deref(), launch, n, o)))
            .map_err(err)?;
        names::warn_coat(gil, coat.as_deref())?;
        to_dict(gil, s, o, out)
    }
}

/// A trace result (before making the dict; made without the GIL)
pub(crate) type TraceOut = (Soa, Option<PolOut>, Vec<f64>, Vec<f64>);

/// Polarization trace result: Jones (ray, 2, 2), z components (ray, 2), and when interpolated on a grid, the grid points missing inside
/// the pupil and the rays that could not be interpolated because of them
pub(crate) type PolOut = (Vec<C64>, Vec<C64>, Option<(Vec<[f64; 2]>, Vec<bool>)>);

/// Trace with the launch (n rays) (without the GIL)
pub(crate) fn compute(
    sys: &System,
    coat: Option<&Coatings>,
    launch: &Launch,
    n: usize,
    o: &TraceOpts,
) -> lensbench_core::Result<TraceOut> {
    let rec_trace = o.record || o.params.is_some();
    let mut g = Vec::new();
    let mut gl = Vec::new();
    // 記録・微分・検出器のずらしが無ければ列に直接書く（光線ごとの結果を作らない）
    let traced = if !rec_trace && o.dz.is_none() {
        let cols = Cols::XYZ
            | Cols::LMN
            | Cols::OPL
            | Cols::OPL_OS
            | Cols::LENS_OPD
            | if o.globals {
                Cols::PK | Cols::START
            } else {
                Cols::empty()
            };
        let bufs =
            o.ws.iter()
                .map(|&w| {
                    let mut b = RayBuf::with_cols(cols);
                    sys.trace_launch_into(launch, w, o.last, &mut b);
                    b
                })
                .collect();
        Traced::Cols(bufs)
    } else {
        let mut traces = Vec::with_capacity(o.ws.len());
        for &w in &o.ws {
            // 記録する面を選んだとき: dOPL/dp には全面の記録が要るので、そのときは全面で追って後で選ぶ
            let sel = match (&o.rec_surfs, o.params.is_some()) {
                (Some(v), false) => RecSel::Surfs(v),
                _ => RecSel::from(rec_trace),
            };
            let mut t = sys.trace_launch(launch, w, o.last, sel);
            if let Some(ps) = &o.params {
                let mut gw = sys.opl_grad(&t, w, ps)?;
                if let Some(z) = o.dz {
                    sys.opl_grad_shift(&t, w, ps, z, &mut gw)?;
                }
                g.extend(gw);
                if o.land {
                    gl.extend(sys.land_grad(&t, w, ps, o.dz)?);
                }
            }
            if let Some(z) = o.dz {
                sys.shift_end(&mut t, w, z)?;
            }
            if let Some(v) = &o.rec_surfs {
                t.rec.retain(|r| v.contains(&r.surf));
            }
            traces.push(t);
        }
        Traced::Rays(traces)
    };
    let jones = match (o.pw, coat) {
        (Some(w), Some(c)) if o.pol_grid.is_some() => {
            let (nx, ny) = o.pol_grid.unwrap();
            let pg = PolGrid::trace(sys, launch, w, c, nx, ny)?;
            let (px, py) = match *launch {
                Launch::Pupil { px, py, .. } => (px, py),
                _ => unreachable!("PolGrid::trace が瞳座標以外を断る"),
            };
            // 止まった・けられた光線と伝搬の位相は、その波長の普通の追跡から
            let own;
            let (t, k) = match o.ws.iter().position(|&q| q == w) {
                Some(k) if o.dz.is_none() => (&traced, k),
                _ => {
                    let mut b = RayBuf::with_cols(Cols::OPL);
                    sys.trace_launch_into(launch, w, o.last, &mut b);
                    own = Traced::Cols(vec![b]);
                    (&own, 0)
                }
            };
            let kw = sys.wave(w).k_mm();
            let nan = C64::new(f64::NAN, f64::NAN);
            let mut j = Vec::with_capacity(n * 4);
            let mut z = Vec::with_capacity(n * 2);
            let mut bad = Vec::with_capacity(n);
            for i in 0..n {
                let (ok, opl) = t.ok_opl(k, i);
                let v = if ok { pg.eval(px[i], py[i]) } else { None };
                bad.push(ok && v.is_none());
                match v {
                    Some(mut v) => {
                        if o.pol_phase {
                            let ph = C64::new(0.0, -kw * opl).exp();
                            v.iter_mut().for_each(|c| *c *= ph);
                        }
                        j.extend_from_slice(&v[..4]);
                        z.extend_from_slice(&v[4..6]);
                    }
                    None => {
                        j.extend([nan; 4]);
                        z.extend([nan; 2]);
                    }
                }
            }
            Some((j, z, Some((pg.missing(), bad))))
        }
        (Some(w), Some(c)) => {
            let x = [C64::new(1.0, 0.0), C64::new(0.0, 0.0)];
            let y = [C64::new(0.0, 0.0), C64::new(1.0, 0.0)];
            let p = sys.trace_pol_launch(
                launch,
                w,
                PolIn::new(&[x, y], c).phase(o.pol_phase),
                RecSel::Off,
            )?;
            // (光線, 行 = 出射の x・y, 列 = 入射の x・y) と z 成分 (光線, 列)
            let mut j = Vec::with_capacity(n * 4);
            let mut z = Vec::with_capacity(n * 2);
            for e in &p.e {
                let (ex, ey) = (e[0], e[1]);
                j.extend([ex[0], ey[0], ex[1], ey[1]]);
                z.extend([ex[2], ey[2]]);
            }
            Some((j, z, None))
        }
        _ => None,
    };
    let soa = match &traced {
        Traced::Rays(t) => Soa::new(t, n, o.globals, o.record),
        Traced::Cols(b) => Soa::from_bufs(b, n, o.globals),
    };
    Ok((soa, jones, g, gl))
}
