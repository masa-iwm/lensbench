//! 追跡の結果を (波長, 光線) の列に並べる（lib.rs から分けた）

use lensbench_core::system::{RayBuf, Trace};

/// Arrange the traces per wavelength into (wavelength, ray) arrays (each column copied once)
pub(crate) struct Soa {
    pub(crate) nw: usize,
    pub(crate) n: usize,
    /// (name, value, count per ray)
    pub(crate) f: Vec<(&'static str, Vec<f64>, usize)>,
    pub(crate) err: Vec<i32>,
    pub(crate) vig: Vec<i32>,
    pub(crate) ok: Vec<bool>,
    pub(crate) rec_surf: Vec<i64>,
    pub(crate) rec: Vec<(&'static str, Vec<f64>)>,
}

/// Columns returned by default (local coordinates of the last surface, optical path length)
const COLS: [&str; 9] = ["x", "y", "z", "l", "m", "n", "opl", "opl_os", "lens_opd"];

/// Columns added with globals=True (global position and direction)
const GLOBAL_COLS: [&str; 6] = ["gx", "gy", "gz", "gl", "gm", "gn"];

/// Columns added with record (intersections at the surfaces on the way, in this order)
const REC_COLS: [&str; 10] = [
    "rec_x", "rec_y", "rec_z", "rec_l", "rec_m", "rec_n", "rec_gx", "rec_gy", "rec_gz", "rec_opl",
];

impl Soa {
    pub(crate) fn new(traces: &[Trace], n: usize, globals: bool, record: bool) -> Soa {
        let nw = traces.len();
        // 光線を 1 回だけ走査して全部の列を埋める
        let ng = if globals { GLOBAL_COLS.len() } else { 0 };
        let mut cols: Vec<Vec<f64>> = (0..COLS.len() + ng)
            .map(|_| Vec::with_capacity(nw * n))
            .collect();
        let mut err = Vec::with_capacity(nw * n);
        let mut vig = Vec::with_capacity(nw * n);
        let mut ok = Vec::with_capacity(nw * n);
        for t in traces {
            for (i, r) in t.rays.iter().enumerate().take(n) {
                let v = [
                    r.xyz[0],
                    r.xyz[1],
                    r.xyz[2],
                    r.lmn[0],
                    r.lmn[1],
                    r.lmn[2],
                    r.opl,
                    t.opl_os[i],
                    r.lens_opd,
                ];
                for (c, x) in cols.iter_mut().zip(v) {
                    c.push(x);
                }
                if globals {
                    let g = [r.p[0], r.p[1], r.p[2], r.k[0], r.k[1], r.k[2]];
                    for (c, x) in cols[COLS.len()..].iter_mut().zip(g) {
                        c.push(x);
                    }
                }
                err.push(r.status.code() as i32);
                vig.push(r.vig as i32);
                ok.push(t.ok(i));
            }
        }
        let names = COLS
            .iter()
            .chain(if globals { &GLOBAL_COLS[..] } else { &[] });
        let mut f: Vec<_> = names.zip(cols).map(|(&k, v)| (k, v, 1)).collect();
        if globals {
            let mut p0 = Vec::with_capacity(nw * n * 3);
            let mut k0 = Vec::with_capacity(nw * n * 3);
            for t in traces {
                p0.extend(t.p0.iter().flatten());
                k0.extend(t.k0.iter().flatten());
            }
            f.push(("p0", p0, 3));
            f.push(("k0", k0, 3));
        }
        let rec_surf: Vec<i64> = traces
            .first()
            .filter(|_| record)
            .map(|t| t.rec.iter().map(|r| r.surf as i64).collect())
            .unwrap_or_default();
        let ns = rec_surf.len();
        // 記録の列も光線を 1 回だけ走査して全部埋める
        let rec = if !record {
            Vec::new()
        } else {
            let mut cols: Vec<Vec<f64>> = (0..REC_COLS.len())
                .map(|_| Vec::with_capacity(nw * ns * n))
                .collect();
            for t in traces {
                for r in &t.rec {
                    for i in 0..r.opl.len() {
                        let v = [
                            r.xyz[i][0],
                            r.xyz[i][1],
                            r.xyz[i][2],
                            r.lmn[i][0],
                            r.lmn[i][1],
                            r.lmn[i][2],
                            r.p[i][0],
                            r.p[i][1],
                            r.p[i][2],
                            r.opl[i],
                        ];
                        for (c, x) in cols.iter_mut().zip(v) {
                            c.push(x);
                        }
                    }
                }
            }
            REC_COLS.iter().copied().zip(cols).collect()
        };
        Soa {
            nw,
            n,
            f,
            err,
            vig,
            ok,
            rec_surf,
            rec,
        }
    }
}

impl Soa {
    /// 記録・微分なしの追跡の列（[`RayBuf`]）から。値は [`Soa::new`] と同じ
    pub(crate) fn from_bufs(bufs: &[RayBuf], n: usize, globals: bool) -> Soa {
        let nw = bufs.len();
        let col = |get: &dyn Fn(&RayBuf) -> &[f64]| -> Vec<f64> {
            let mut v = Vec::with_capacity(nw * n);
            for b in bufs {
                v.extend_from_slice(&get(b)[..n]);
            }
            v
        };
        let comp = |get: &dyn Fn(&RayBuf) -> &[lensbench_core::math::V3], c: usize| -> Vec<f64> {
            let mut v = Vec::with_capacity(nw * n);
            for b in bufs {
                v.extend(get(b)[..n].iter().map(|x| x[c]));
            }
            v
        };
        let mut f: Vec<(&'static str, Vec<f64>, usize)> = vec![
            ("x", comp(&|b| &b.xyz, 0), 1),
            ("y", comp(&|b| &b.xyz, 1), 1),
            ("z", comp(&|b| &b.xyz, 2), 1),
            ("l", comp(&|b| &b.lmn, 0), 1),
            ("m", comp(&|b| &b.lmn, 1), 1),
            ("n", comp(&|b| &b.lmn, 2), 1),
            ("opl", col(&|b| &b.opl), 1),
            ("opl_os", col(&|b| &b.opl_os), 1),
            ("lens_opd", col(&|b| &b.lens_opd), 1),
        ];
        if globals {
            for (k, c) in [("gx", 0), ("gy", 1), ("gz", 2)] {
                f.push((k, comp(&|b| &b.p, c), 1));
            }
            for (k, c) in [("gl", 0), ("gm", 1), ("gn", 2)] {
                f.push((k, comp(&|b| &b.k, c), 1));
            }
            let flat = |get: &dyn Fn(&RayBuf) -> &[lensbench_core::math::V3]| -> Vec<f64> {
                let mut v = Vec::with_capacity(nw * n * 3);
                for b in bufs {
                    v.extend(get(b)[..n].iter().flatten());
                }
                v
            };
            f.push(("p0", flat(&|b| &b.p0), 3));
            f.push(("k0", flat(&|b| &b.k0), 3));
        }
        let mut err = Vec::with_capacity(nw * n);
        let mut vig = Vec::with_capacity(nw * n);
        let mut ok = Vec::with_capacity(nw * n);
        for b in bufs {
            err.extend(b.status[..n].iter().map(|s| s.code() as i32));
            vig.extend(b.vig[..n].iter().map(|&v| v as i32));
            ok.extend((0..n).map(|i| b.ok(i)));
        }
        Soa {
            nw,
            n,
            f,
            err,
            vig,
            ok,
            rec_surf: Vec::new(),
            rec: Vec::new(),
        }
    }
}

/// 追跡の結果（記録・微分なしなら列、ほかは光線ごと）
pub(crate) enum Traced {
    Rays(Vec<Trace>),
    Cols(Vec<RayBuf>),
}

impl Traced {
    /// 波長の番号 k の光線 i が届いたか・その光路長
    pub(crate) fn ok_opl(&self, k: usize, i: usize) -> (bool, f64) {
        match self {
            Traced::Rays(t) => (t[k].ok(i), t[k].rays[i].opl),
            Traced::Cols(b) => (b[k].ok(i), b[k].opl[i]),
        }
    }
}
