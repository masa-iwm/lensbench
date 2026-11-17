//! lensbench の Python バインディング。同じプロセスの中で追跡し、結果を numpy 配列で返す。
//!
//! 視野・波長・面の番号は CLI と同じ（視野・波長は 1 始まり、面は OpticStudio の面番号）。
//! 系はオブジェクトごとに独立で、グローバルな状態を持たない（プロセス並列にしてよい）。
//! 追跡の間は GIL を外す。パラメータの変更、光路長の微分、物点・瞳座標の解きも使える。
//!
//! `threads=1` は呼び出したスレッドで逐次に計算し、スレッドのプールを使わない。このため
//! `multiprocessing` の fork で作った子プロセスの中でも動く。fork の前に親で並列
//! （`threads` を省くか 2 以上）で呼んだあとは、子プロセスでの並列は使えない（プールのスレッドが子に
//! 引き継がれず、止まる）。子で並列にしたいときは forkserver・spawn で子プロセスを作る。

#![warn(unreachable_pub)]
#![allow(clippy::unwrap_used, clippy::panic)] // 入口の exe（計算の中身のクレートは検査の内）

mod edit;
mod errors;
pub mod ext;
mod interference;
mod mtfs;
use mtfs::{field_axis, mtf_dict, sweep_dict};
mod names;
use crate::errors::{index_err, runtime_err, type_err, value_err};
use names::warn_all;
mod analyses;
mod optargs;
use optargs::{opt_problem, VarArg};
mod plugin;
mod pyjson;
mod runs;
mod search;
mod soa;
use soa::{Soa, Traced};
mod userterm;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use lensbench_analysis::polgrid::{self, PolGrid};
use lensbench_core::catalog::LensCatalog;
use lensbench_core::coating::{Coatings, C64};
use lensbench_core::solve::{RayProblem, Residual, Unknown};
use lensbench_core::system::{
    Aiming, Cols, Field, FieldCond, FieldSel, FieldType, Launch, ObjSpec, PolIn, RayBuf,
};
use lensbench_core::{analysis, psf, sweep, with_threads, zmx, RecSel, System};
use lensbench_core::{param::Param, RwLockExt, Waves};
use numpy::ndarray::Array;
use numpy::{AllowTypeChange, IntoPyArray, PyArrayLike1, PyArrayLike2};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyTuple};
use {lensbench_analysis::huygens, lensbench_optim as optim};

/// Coating tables and models used when coatings= is omitted: the environment variable LENSBENCH_COATINGS (OS path separator, `lensbench run --coatings`)
fn env_coatings() -> Vec<PathBuf> {
    std::env::var_os("LENSBENCH_COATINGS")
        .map(|v| {
            std::env::split_paths(&v)
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Settings passed by `lensbench run`. All are read from environment variables; empty or None if not set
///
/// - glass_dirs: folders to search for glass catalogs (LENSBENCH_GLASSCAT; System.load and others also read it)
/// - coatings: coating tables and models (LENSBENCH_COATINGS; the default of coatings= of System.load)
/// - threads: number of threads (LENSBENCH_THREADS; the default of calculations with threads= omitted)
/// - out: output folder (LENSBENCH_OUT); for the script's own use
/// - log: log file (LENSBENCH_LOG)
/// - args: arguments passed to the script (same as sys.argv[1:])
/// - under_run: whether started from `lensbench run`
#[pyfunction]
fn run_settings<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
    let paths = |k: &str| -> Vec<String> {
        std::env::var_os(k)
            .map(|v| {
                std::env::split_paths(&v)
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|p| p.display().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    let opt = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let d = PyDict::new(py);
    d.set_item("glass_dirs", paths("LENSBENCH_GLASSCAT"))?;
    d.set_item("coatings", paths("LENSBENCH_COATINGS"))?;
    d.set_item("threads", lensbench_core::par::default_threads())?;
    d.set_item("out", opt("LENSBENCH_OUT"))?;
    d.set_item("log", opt("LENSBENCH_LOG"))?;
    let argv: Vec<String> = py.import("sys")?.getattr("argv")?.extract()?;
    d.set_item("args", argv.into_iter().skip(1).collect::<Vec<_>>())?;
    d.set_item(
        "under_run",
        std::env::var("LENSBENCH_RUN").is_ok_and(|v| v == "1"),
    )?;
    Ok(d)
}

/// Print the progress on one line to standard error: `lensbench:progress 3/10 message`.
/// `lensbench run` picks up this line and logs it; a future GUI will use it to show progress
#[pyfunction]
#[pyo3(signature = (done, total, message=""))]
fn progress(py: Python<'_>, done: usize, total: usize, message: &str) -> PyResult<()> {
    let line = if message.is_empty() {
        format!("lensbench:progress {done}/{total}\n")
    } else {
        format!("lensbench:progress {done}/{total} {message}\n")
    };
    let err = py.import("sys")?.getattr("stderr")?;
    err.call_method1("write", (line,))?;
    err.call_method0("flush")?;
    Ok(())
}

/// Run a long calculation in another thread and watch for Ctrl-C (signals) in the calling thread (Python's main thread).
/// When a signal comes, set stop, wait for the calculation to end and return an exception such as KeyboardInterrupt.
/// The calculation runs under a cancel token (lensbench_core::par::with_cancel) and stops at the next check (each iteration or trial)
pub fn interruptible<R: Send>(py: Python<'_>, f: impl FnOnce() -> R + Send) -> PyResult<R> {
    let stop = lensbench_core::par::CancelToken::new();
    std::thread::scope(|sc| {
        let (tx, rx) = std::sync::mpsc::channel();
        let t = stop.clone();
        sc.spawn(move || {
            // 受け手が先に去っていても（例外で返った後）送りの失敗は気にしない
            let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || lensbench_core::par::with_cancel(Some(&t), f),
            )));
        });
        let tick = std::time::Duration::from_millis(100);
        // Receiver は Sync でないので、GIL を外す間は持ち出して返してもらう
        let mut rx = Some(rx);
        loop {
            let (got, r) = py.detach(|| {
                let r = rx.take().unwrap();
                (r.recv_timeout(tick), r)
            });
            rx = Some(r);
            match got {
                Ok(r) => return r.map_err(|_| runtime_err("the computing thread panicked")),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(runtime_err("the computing thread ended"))
                }
            }
            if let Err(e) = py.check_signals() {
                stop.cancel();
                // 計算が stop を見て止まるのを待つ（scope の終わりで join する）
                let r = rx.take().unwrap();
                let _ = py.detach(move || r.recv());
                return Err(e);
            }
        }
    })
}

pub use errors::{err, ErrSrc};

/// An optical system (read from .zmx or a project file .json)
#[pyclass(module = "lensbench", name = "System", frozen)]
pub struct PySystem {
    /// The system. The lock is held only while cloning or replacing the Arc (tracing uses the cloned Arc without the GIL).
    /// Changes are made to a cloned system that then replaces it, so a running trace keeps the previous system
    sys: RwLock<Arc<System>>,
    coat_files: Vec<PathBuf>,
    /// The coatings of the system (read when first needed). Emptied by edits that change coatings, media or the coating file
    coat: RwLock<CoatCell>,
}

/// The coatings read for a system (shared by cloned and derived systems)
pub type CoatCell = Arc<OnceLock<Result<Coatings, String>>>;

/// A reference to the coatings read ([`PySystem::coatings`])
pub struct CoatRef(CoatCell);

impl std::ops::Deref for CoatRef {
    type Target = Coatings;
    fn deref(&self) -> &Coatings {
        // coatings() が Ok を確かめてから作る
        self.0
            .get()
            .and_then(|r| r.as_ref().ok())
            .expect("膜を読んだ後")
    }
}

/// Whether the parts of the system that coating loading depends on changed (coatings, media boundaries, coating file, the system's coating table)
fn coat_changed(a: &System, b: &System) -> bool {
    a.coating_file != b.coating_file
        || a.coat_tables != b.coat_tables
        || lensbench_core::coating::media_keys(a).0 != lensbench_core::coating::media_keys(b).0
}

/// A field: a number (from 1) or a normalized field (hx, hy). bool is rejected
#[derive(FromPyObject, Clone)]
pub enum FieldArg {
    Flag(bool),
    Index(usize),
    Norm((f64, f64)),
}

/// An object: an object point (x, y) [mm] of a finite object (local coordinates of the object surface) or a direction (l, m, n) of an infinite object
#[derive(FromPyObject, Clone)]
pub enum ObjArg {
    Point((f64, f64)),
    Dir((f64, f64, f64)),
}

/// pol: True (primary wavelength) or a wavelength number
#[derive(FromPyObject, Clone)]
enum PolArg {
    Flag(bool),
    Wave(usize),
}

/// pol_grid: the number of points n for both axes or (nx, ny)
#[derive(FromPyObject, Clone)]
enum GridArg {
    N(usize),
    Nxy((usize, usize)),
}

/// record: True (all surfaces) or a list of surface numbers (from 1)
#[derive(FromPyObject, Clone)]
enum RecArg {
    Flag(bool),
    Surfs(Vec<usize>),
}

impl RecArg {
    fn surfs(self) -> (bool, Option<Vec<usize>>) {
        match self {
            RecArg::Flag(b) => (b, None),
            RecArg::Surfs(v) => (true, Some(v)),
        }
    }
}

/// Where the rays come from
#[derive(Clone, Copy)]
enum Source {
    Field(FieldSel),
    Object(ObjSpec),
}

/// How trace starts the rays (a field or an object, and the kind of pupil coordinates)
struct Prep {
    src: Source,
    spec: ObjSpec,
    vig: [f64; 5],
    hxy: (f64, f64),
    at_stop: bool,
    signed: bool,
}

impl Prep {
    fn new(
        s: &System,
        field: Option<FieldArg>,
        object: Option<ObjArg>,
        pupil: &str,
        npx: usize,
        npy: usize,
    ) -> PyResult<Prep> {
        let src = match (field, object) {
            (Some(f), None) => Source::Field(field_sel(s, f)?),
            (None, Some(o)) => Source::Object(obj_spec(s, o)?),
            _ => return Err(value_err("give exactly one of field and object")),
        };
        let (at_stop, signed) = match pupil.to_ascii_lowercase().as_str() {
            "normalized" => (false, false),
            "signed" => (false, true),
            "stop" => (true, false),
            p => {
                return Err(value_err(format!(
                    "pupil must be \"normalized\", \"signed\" or \"stop\" ({p})"
                )))
            }
        };
        if npx != npy {
            return Err(value_err(format!(
                "px and py have different lengths ({npx} and {npy})"
            )));
        }
        let (spec, vig, hxy) = match src {
            Source::Field(fs) => {
                let (hx, hy) = match fs {
                    FieldSel::Index(i) => s.hxhy(i),
                    FieldSel::Norm(a, b) => (a, b),
                };
                (
                    s.object_spec(hx, hy).map_err(err)?,
                    s.vig_factors(hx, hy).map_err(err)?,
                    (hx, hy),
                )
            }
            Source::Object(o) => (o, [0.0; 5], (0.0, 0.0)),
        };
        Ok(Prep {
            src,
            spec,
            vig,
            hxy,
            at_stop,
            signed,
        })
    }

    fn launch<'a>(&self, px: &'a [f64], py: &'a [f64]) -> Launch<'a> {
        if self.at_stop {
            Launch::Stop {
                spec: self.spec,
                x: px,
                y: py,
            }
        } else {
            Launch::Pupil {
                spec: self.spec,
                signed: self.signed,
                px,
                py,
                vig: Some(self.vig),
                hxy: Some(self.hxy),
            }
        }
    }

    /// Add the field (hx, hy) or the object to the result dict
    fn finish(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        match self.src {
            Source::Field(_) => {
                d.set_item("hx", self.hxy.0)?;
                d.set_item("hy", self.hxy.1)?;
            }
            Source::Object(o) => d.set_item("object", obj_out(o))?,
        }
        Ok(())
    }
}

pub fn obj_spec(s: &System, o: ObjArg) -> PyResult<ObjSpec> {
    match o {
        ObjArg::Point((x, y)) => {
            if s.infinite() {
                return Err(value_err(
                    "with an object at infinity, object is a direction (l, m, n)",
                ));
            }
            Ok(ObjSpec::Point([x, y]))
        }
        ObjArg::Dir((l, m, n)) => {
            if !s.infinite() {
                return Err(value_err("a finite object takes a point (x, y)"));
            }
            let r = (l * l + m * m + n * n).sqrt();
            if r.is_nan() || r <= 0.0 || n.is_nan() || n <= 0.0 {
                return Err(value_err("direction (l, m, n) needs n > 0"));
            }
            Ok(ObjSpec::Dir([l / r, m / r, n / r]))
        }
    }
}

pub fn obj_out(o: ObjSpec) -> Vec<f64> {
    match o {
        ObjSpec::Point(p) => p.to_vec(),
        ObjSpec::Dir(d) => d.to_vec(),
    }
}

pub fn field_sel(s: &System, f: FieldArg) -> PyResult<FieldSel> {
    match f {
        FieldArg::Flag(b) => Err(type_err(format!(
            "a field is a number (1-based) or (hx, hy); a bool ({b}) can't be used"
        ))),
        FieldArg::Index(i) => {
            let n = s.fields.len();
            if i == 0 || i > n {
                return Err(index_err(format!("no field {i} (1 to {n})")));
            }
            Ok(FieldSel::Index(i - 1))
        }
        FieldArg::Norm((x, y)) => Ok(FieldSel::Norm(x, y)),
    }
}

/// The RMS values into a dict (lists of values; Python values are made by put)
fn rms_items<'py>(
    d: &Bound<'py, PyDict>,
    r: &[sweep::Rms],
    put: impl Fn(Vec<f64>) -> PyResult<Bound<'py, PyAny>>,
) -> PyResult<()> {
    let col = |f: fn(&sweep::Rms) -> f64| r.iter().map(f).collect::<Vec<f64>>();
    d.set_item("spot", put(col(|v| v.spot))?)?;
    d.set_item("spot_x", put(col(|v| v.spot_x))?)?;
    d.set_item("spot_y", put(col(|v| v.spot_y))?)?;
    d.set_item("wfe", put(col(|v| v.wfe))?)?;
    d.set_item("strehl", put(col(|v| v.strehl))?)?;
    Ok(())
}

pub fn wave_index(s: &System, w: usize) -> PyResult<usize> {
    lensbench_core::waves::index(w, s.wavelengths.len()).map_err(|e| index_err(String::from(e)))
}

/// A list of changes: {"3.t": 1.0, ...} or [("3.t", 1.0), ...]
pub fn parse_changes(s: &System, c: &Bound<'_, PyAny>) -> PyResult<Vec<(Param, f64)>> {
    let v: Vec<(String, f64)> = match c.cast::<PyDict>() {
        Ok(d) => d
            .iter()
            .map(|(k, v)| Ok((k.extract()?, v.extract()?)))
            .collect::<PyResult<_>>()?,
        Err(_) => c.extract()?,
    };
    v.into_iter()
        .map(|(k, x)| Ok((s.param(&k).map_err(err)?, x)))
        .collect()
}

fn floats(a: &PyArrayLike1<'_, f64, AllowTypeChange>) -> Vec<f64> {
    a.as_slice()
        .map_or_else(|_| a.as_array().iter().copied().collect(), <[f64]>::to_vec)
}

/// A surface by number or name
#[derive(FromPyObject)]
enum SurfArg {
    Index(usize),
    Name(String),
}

fn surf_of(s: &System, a: SurfArg) -> PyResult<usize> {
    match a {
        SurfArg::Index(j) => s.find_surface(&j.to_string()).map_err(err),
        SurfArg::Name(n) => s.find_surface(&n).map_err(err),
    }
}

impl PySystem {
    /// The current system (a cloned Arc)
    pub fn sys(&self) -> Arc<System> {
        self.sys.read_ok().clone()
    }

    /// Make from a system (the coating file, the coatings read)
    pub fn new(sys: System, coat_files: Vec<PathBuf>, coat: CoatCell) -> Self {
        PySystem {
            sys: RwLock::new(Arc::new(sys)),
            coat_files,
            coat: RwLock::new(coat),
        }
    }

    /// The coatings read (carried over to derived systems)
    pub fn coat_cell(&self) -> CoatCell {
        self.coat.read_ok().clone()
    }

    /// Clone the system, change it with f and replace it only on success (a running trace keeps the previous system).
    /// If something that coating loading depends on changes, drop the coatings read and read them again when next needed
    fn mutate<R>(&self, f: impl FnOnce(&mut System) -> lensbench_core::Result<R>) -> PyResult<R> {
        let mut g = self.sys.write_ok();
        let mut n: System = (**g).clone();
        let r = f(&mut n).map_err(err)?;
        if coat_changed(&g, &n) {
            *self.coat.write_ok() = CoatCell::default();
        }
        *g = Arc::new(n);
        Ok(r)
    }

    /// The coatings of the system (read on the first call)
    pub fn coatings(&self, s: &System) -> PyResult<CoatRef> {
        let cell = self.coat_cell();
        cell.get_or_init(|| {
            let mut c = Coatings::for_system(s)?;
            for p in &self.coat_files {
                c.load_file(p)?;
            }
            Ok(c)
        })
        .as_ref()
        .map_err(err)?;
        Ok(CoatRef(cell))
    }
}

/// Arguments shared by trace and trace_rays
struct TraceOpts {
    ws: Vec<usize>,
    last: Option<usize>,
    record: bool,
    /// Surfaces to record (all if None)
    rec_surfs: Option<Vec<usize>>,
    pw: Option<usize>,
    pol_phase: bool,
    /// Polarization trace on a coarse pupil grid (nx, ny), interpolated
    pol_grid: Option<(usize, usize)>,
    globals: bool,
    params: Option<Vec<Param>>,
    threads: Option<usize>,
    /// Put the detector at z = dz in the local coordinates of the last surface
    dz: Option<f64>,
    /// With grad, also return the derivative of the landing point d(x, y, l, m)/dp
    land: bool,
}

impl TraceOpts {
    #[allow(clippy::too_many_arguments)]
    fn new(
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
    fn run_trace<'py>(
        &self,
        gil: Python<'py>,
        s: &Arc<System>,
        launch: &Launch,
        n: usize,
        o: &TraceOpts,
    ) -> PyResult<Bound<'py, PyDict>> {
        let coat = match o.pw {
            Some(_) => Some(self.coatings(s)?),
            None => None,
        };
        let out = gil
            .detach(|| with_threads(o.threads, || compute(s, coat.as_deref(), launch, n, o)))
            .map_err(err)?;
        to_dict(gil, s, o, out)
    }
}

/// A trace result (before making the dict; made without the GIL)
type TraceOut = (Soa, Option<PolOut>, Vec<f64>, Vec<f64>);

/// Polarization trace result: Jones (ray, 2, 2), z components (ray, 2), and when interpolated on a grid, the grid points missing inside
/// the pupil and the rays that could not be interpolated because of them
type PolOut = (Vec<C64>, Vec<C64>, Option<(Vec<[f64; 2]>, Vec<bool>)>);

/// Trace with the launch (n rays) (without the GIL)
fn compute(
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
            let kw = 2.0 * std::f64::consts::PI / (sys.wavelengths[w] * 1e-3);
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
                        z.extend_from_slice(&v[4..]);
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

/// Make the dict of a trace result
fn to_dict<'py>(
    gil: Python<'py>,
    s: &System,
    o: &TraceOpts,
    out: TraceOut,
) -> PyResult<Bound<'py, PyDict>> {
    let (soa, jones, g, gl) = out;
    let d = PyDict::new(gil);
    let (nw, n) = (soa.nw, soa.n);
    for (name, v, k) in soa.f {
        if k == 1 {
            d.set_item(name, a2(gil, v, (nw, n)))?;
        } else {
            d.set_item(name, a3(gil, v, (nw, n, k)))?;
        }
    }
    d.set_item("err", a2(gil, soa.err, (nw, n)))?;
    d.set_item("vig", a2(gil, soa.vig, (nw, n)))?;
    d.set_item("ok", a2(gil, soa.ok, (nw, n)))?;
    d.set_item(
        "waves",
        o.ws.iter()
            .map(|&w| w as i64 + 1)
            .collect::<Vec<_>>()
            .into_pyarray(gil),
    )?;
    d.set_item("last", o.last.unwrap_or(s.image()))?;
    if let Some(z) = o.dz {
        d.set_item("dz", z)?;
    }
    if o.record {
        let ns = soa.rec_surf.len();
        d.set_item("rec_surfaces", soa.rec_surf.into_pyarray(gil))?;
        for (name, v) in soa.rec {
            d.set_item(name, a3(gil, v, (nw, ns, n)))?;
        }
    }
    if let Some((j, z, g)) = jones {
        d.set_item("jones", a3(gil, j, (n, 2, 2)))?;
        d.set_item("jones_z", a2(gil, z, (n, 2)))?;
        d.set_item("jones_wavelength", o.pw.unwrap() + 1)?;
        if let (Some((miss, bad)), Some(g)) = (g, o.pol_grid) {
            d.set_item("pol_grid", g)?;
            let m = miss.len();
            let a = Array::from_shape_vec((m, 2), miss.into_iter().flatten().collect())
                .expect("shape")
                .into_pyarray(gil);
            d.set_item("pol_grid_missing", a)?;
            d.set_item("pol_grid_bad", bad.into_pyarray(gil))?;
        }
    }
    if let Some(ps) = &o.params {
        let np = ps.len();
        d.set_item("grad", a3(gil, g, (nw, n, np)))?;
        if o.land {
            let a = Array::from_shape_vec((nw, n, np, 4), gl)
                .expect("shape")
                .into_pyarray(gil);
            d.set_item("grad_land", a)?;
        }
        d.set_item(
            "grad_params",
            ps.iter().map(|p| p.name()).collect::<Vec<_>>(),
        )?;
    }
    Ok(d)
}

fn a2<T: numpy::Element>(py: Python<'_>, v: Vec<T>, s: (usize, usize)) -> Bound<'_, PyAny> {
    Array::from_shape_vec(s, v)
        .expect("shape")
        .into_pyarray(py)
        .into_any()
}

fn a3<T: numpy::Element>(py: Python<'_>, v: Vec<T>, s: (usize, usize, usize)) -> Bound<'_, PyAny> {
    Array::from_shape_vec(s, v)
        .expect("shape")
        .into_pyarray(py)
        .into_any()
}

/// For each surface of the expanded system, the original surface and the number of the reflection
type GhostOrigin = (Vec<Option<usize>>, (usize, usize));

#[pymethods]
impl PySystem {
    /// Read a system. path is a .zmx or a project file (.json).
    ///
    /// - glass_dirs: folders to search for glass catalogs (AGF)
    /// - cfg: the multi-configuration number of a .zmx (from 1)
    /// - state: the name of a state of a project file
    /// - beam_path: the beam splitter path of a project file (T and R in the order the BSs are passed, e.g. "TR").
    ///   The possible paths are `System.beam_paths(path)`
    /// - lens_catalogs: lens catalogs (.zmx or folders) to search for the catalog lenses of a project file
    /// - coatings: files of coating tables (LBCT) and models (LBCM) used in polarization traces
    /// - coating_file: replaces the system's coating file (COATING.DAT format)
    /// - wavelengths: replaces the wavelengths [µm] (.zmx only; InvalidError for a system file or project file. Weights are 1, the primary wavelength number is kept)
    #[staticmethod]
    #[pyo3(signature = (path, glass_dirs=None, cfg=1, state=None, lens_catalogs=None, coatings=None, coating_file=None, wavelengths=None, beam_path=None))]
    #[allow(clippy::too_many_arguments)]
    fn load(
        py: Python<'_>,
        path: PathBuf,
        glass_dirs: Option<Vec<PathBuf>>,
        cfg: usize,
        state: Option<String>,
        lens_catalogs: Option<Vec<PathBuf>>,
        coatings: Option<Vec<PathBuf>>,
        coating_file: Option<PathBuf>,
        wavelengths: Option<Vec<f64>>,
        beam_path: Option<String>,
    ) -> PyResult<PySystem> {
        let glass_dirs = glass_dirs.unwrap_or_default();
        let lens_catalogs = lens_catalogs.unwrap_or_default();
        let o = py.detach(|| {
            lensbench_project::open::open(
                &path,
                &lensbench_project::open::OpenSpec {
                    glass_dirs: &glass_dirs,
                    lens_catalogs: &lens_catalogs,
                    cfg,
                    state: state.as_deref(),
                    split_path: beam_path.as_deref(),
                    wavelengths,
                    names: lensbench_project::open::ArgNames::PYTHON,
                    step_files: None,
                },
            )
        });
        let lensbench_project::open::Opened {
            system: mut sys,
            warnings: warn,
            coatings: saved_coatings,
            ..
        } = o.map_err(err)?;
        warn_all(py, &warn)?;
        if coating_file.is_some() {
            sys.edit_raw(|r| {
                r.coating_file = coating_file;
                Ok(())
            })
            .map_err(err)?;
        }
        Ok(PySystem::new(
            sys,
            coatings.unwrap_or_else(|| {
                if saved_coatings.is_empty() {
                    env_coatings()
                } else {
                    saved_coatings
                }
            }),
            Default::default(),
        ))
    }

    /// Save to a system file (lensbench's own JSON, format version 2). What you wrote is saved: surfaces, glass names, parameter changes, pickups,
    /// solves, lenses and groups, the multi-configuration table, dn and disp; reading it back with System.load traces bit for bit the same.
    /// Glasses are written by name with the name and hash of their catalogs (the catalogs are searched again when reading).
    /// The paths of the coating tables and models (coatings=) are written too
    /// A warning is given for a catalog with no file whose values match the system's glasses (its hash is left empty and can't be checked on reading)
    fn save(&self, py: Python<'_>, path: PathBuf) -> PyResult<()> {
        let s = self.sys();
        lensbench_project::sysfile::save(&path, &s, &self.coat_files).map_err(err)?;
        warn_all(py, &lensbench_project::sysfile::save_notes(&s))
    }

    /// The content of the system file (a JSON string). Warns like save
    fn to_json(&self, py: Python<'_>) -> PyResult<String> {
        let s = self.sys();
        warn_all(py, &lensbench_project::sysfile::save_notes(&s))?;
        lensbench_project::sysfile::to_string(&s, &self.coat_files).map_err(err)
    }

    /// Make a system from the JSON string of a system file (the coatings paths are used as they are).
    /// glass_dirs: folders to search for the glass catalogs first (then LENSBENCH_GLASSCAT and the catalogs already read)
    #[staticmethod]
    #[pyo3(signature = (text, glass_dirs=None))]
    fn from_json(
        py: Python<'_>,
        text: &str,
        glass_dirs: Option<Vec<PathBuf>>,
    ) -> PyResult<PySystem> {
        let (s, m, w) =
            lensbench_project::sysfile::from_str_with(text, &glass_dirs.unwrap_or_default())
                .map_err(err)?;
        warn_all(py, &w)?;
        Ok(PySystem::new(s, m.coatings, Default::default()))
    }

    /// The hash of the system ("sha256:…", the same as the system file). Stamp it on analysis results to check later that the system is the same
    #[getter]
    fn hash(&self) -> PyResult<String> {
        lensbench_project::sysfile::system_hash(&self.sys()).map_err(err)
    }

    /// Expand a project (a system built from blocks) from a JSON string or dict, without a file.
    ///
    /// - project: a JSON string of the same form as a project file, or a dict
    /// - base: the folder that relative paths (such as the path of a zmx block) are based on (current folder if omitted)
    /// - the rest as in load. Pickups of an imported .zmx that can't be kept in the expansion (referring outside the range and so on) are
    ///   fixed at their values and reported in warnings
    #[staticmethod]
    #[pyo3(signature = (project, base=None, glass_dirs=None, state=None, lens_catalogs=None, coatings=None, coating_file=None, beam_path=None))]
    #[allow(clippy::too_many_arguments)]
    fn from_project(
        py: Python<'_>,
        project: &Bound<'_, PyAny>,
        base: Option<PathBuf>,
        glass_dirs: Option<Vec<PathBuf>>,
        state: Option<String>,
        lens_catalogs: Option<Vec<PathBuf>>,
        coatings: Option<Vec<PathBuf>>,
        coating_file: Option<PathBuf>,
        beam_path: Option<String>,
    ) -> PyResult<PySystem> {
        let txt: String = if let Ok(t) = project.extract::<String>() {
            t
        } else if project.cast::<PyDict>().is_ok() {
            pyjson::from_py(project)?.to_string()
        } else {
            return Err(type_err("project: a JSON string or dict"));
        };
        let base = base.unwrap_or_else(|| PathBuf::from("."));
        let glass_dirs = glass_dirs.unwrap_or_default();
        let lens_catalogs = lens_catalogs.unwrap_or_default();
        let r = py.detach(|| -> lensbench_core::Result<(System, Vec<String>)> {
            let cat = LensCatalog::load_registered(&lens_catalogs)?;
            let look = |v: &str, p: &str| cat.zmx(v, p);
            let p = lensbench_project::Project::from_json(&txt)?;
            let env = lensbench_project::Env::in_dir(&base, &glass_dirs, Some(&look));
            let o = lensbench_project::ExpandOpts {
                state: state.as_deref(),
                path: beam_path.as_deref(),
                ..Default::default()
            };
            let x = lensbench_project::expand(&p, &env, &o)?;
            Ok((x.system, x.notes))
        });
        let (mut sys, warn) = r.map_err(err)?;
        warn_all(py, &warn)?;
        if coating_file.is_some() {
            sys.edit_raw(|r| {
                r.coating_file = coating_file;
                Ok(())
            })
            .map_err(err)?;
        }
        Ok(PySystem::new(
            sys,
            coatings.unwrap_or_else(env_coatings),
            Default::default(),
        ))
    }

    /// The beam splitter paths of a project file (.json) (strings of T and R in the order the BSs are passed).
    /// Empty without a BS. Read each with `load(path, beam_path=path)`
    #[staticmethod]
    #[pyo3(signature = (path, glass_dirs=None, state=None, lens_catalogs=None))]
    fn beam_paths(
        py: Python<'_>,
        path: PathBuf,
        glass_dirs: Option<Vec<PathBuf>>,
        state: Option<String>,
        lens_catalogs: Option<Vec<PathBuf>>,
    ) -> PyResult<Vec<String>> {
        names::beam_paths_of(py, path, glass_dirs, state, lens_catalogs)
    }

    /// Read the systems of several configurations of a multi-configuration .zmx at once. configs is a list of configuration numbers (from 1);
    /// all if omitted (an empty list is an error). Returns a list of System, each the same as `load(path, cfg=k)`.
    /// Use it to trace switches such as the sample and reference arms or the stop diameter (APMX, APER and so on) together with
    /// `trace_many(..., object=point)` on the same object point and pupil grid
    #[staticmethod]
    #[pyo3(signature = (path, configs=None, glass_dirs=None, coatings=None, coating_file=None))]
    fn load_configs(
        py: Python<'_>,
        path: PathBuf,
        configs: Option<Vec<usize>>,
        glass_dirs: Option<Vec<PathBuf>>,
        coatings: Option<Vec<PathBuf>>,
        coating_file: Option<PathBuf>,
    ) -> PyResult<Vec<PySystem>> {
        if path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        {
            return Err(value_err(
                "load_configs is only for .zmx (load project files per state with load(state=...))",
            ));
        }
        let opt = zmx::LoadOptions {
            catalog_dirs: glass_dirs.unwrap_or_default(),
            ..Default::default()
        };
        // 空の list は全部（None）と取り違えやすいので断る
        if configs.as_ref().is_some_and(|c| c.is_empty()) {
            return Err(value_err(
                "configs is empty (leave configs out for all configurations)",
            ));
        }
        let cfgs = configs.unwrap_or_default();
        let v = py
            .detach(|| zmx::load_configs(&path, &opt, &cfgs))
            .map_err(err)?;
        let coat = coatings.unwrap_or_else(env_coatings);
        v.into_iter()
            .map(|mut s| {
                if coating_file.is_some() {
                    s.edit_raw(|r| {
                        r.coating_file = coating_file.clone();
                        Ok(())
                    })
                    .map_err(err)?;
                }
                Ok(PySystem::new(s, coat.clone(), Default::default()))
            })
            .collect()
    }

    #[getter]
    fn name(&self) -> String {
        self.sys().name.clone()
    }

    /// Wavelengths [µm]
    #[getter]
    fn wavelengths(&self) -> Vec<f64> {
        self.sys().wavelengths.clone()
    }

    /// Warnings for materials whose index is computed at wavelengths outside the valid range of the dispersion formula (LD of the AGF; the formula is extended)
    fn dispersion_warnings(&self) -> Vec<String> {
        self.sys().dispersion_warnings()
    }

    /// ZPL Macro Solves fixed at their saved values without solving (no macro, unsupported commands and so on), with the reasons
    fn macro_warnings(&self) -> Vec<String> {
        self.sys().macro_warnings()
    }

    #[getter]
    fn weights(&self) -> Vec<f64> {
        self.sys().weights.clone()
    }

    /// Primary wavelength number (from 1)
    #[getter]
    fn primary(&self) -> usize {
        self.sys().primary + 1
    }

    /// Fields (x, y)
    #[getter]
    fn fields(&self) -> Vec<(f64, f64)> {
        self.sys().fields.iter().map(|f| (f.x, f.y)).collect()
    }

    /// Surface number of the image surface
    #[getter]
    fn image(&self) -> usize {
        self.sys().image()
    }

    /// Surface number of the stop
    #[getter]
    fn stop(&self) -> usize {
        self.sys().stop()
    }

    /// Whether the entrance pupil of a finite object is before the object (systems where pupil="normalized" is reversed from "signed")
    #[getter]
    fn pupil_reversed(&self) -> bool {
        self.sys().pupil_reversed()
    }

    /// Whether the object is at infinity
    #[getter]
    fn infinite(&self) -> bool {
        self.sys().infinite()
    }

    /// Ray aiming ("off", "paraxial", "real")
    #[getter]
    fn aiming(&self) -> &'static str {
        match self.sys().aiming {
            Aiming::Off => "off",
            Aiming::Paraxial => "paraxial",
            Aiming::Real => "real",
        }
    }

    /// Whether rays outside floating apertures (surfaces with fixed semi-diameters, FLAP in .zmx) are vignetted (default True, as in OpticStudio).
    /// Set False to compare with the older model that doesn't vignette at semi-diameters
    #[getter]
    fn floating_clip(&self) -> bool {
        self.sys().floating_clip
    }

    #[setter]
    fn set_floating_clip(&self, on: bool) -> PyResult<()> {
        self.mutate(|s| {
            s.edit_raw(|r| {
                r.floating_clip = on;
                Ok(())
            })
        })
    }

    /// Cardinal points (like OpticStudio's Cardinal Points). wave is a wavelength number (from 1; primary wavelength if omitted).
    ///
    /// The paraxial system from surface 1 to the surface before the image (Y-Z plane). Object-space positions are from the vertex of surface 1 and image-space positions from the
    /// vertex of the image surface, each along its local z [mm] (as in OpticStudio; with an odd number of mirrors n_image and f_image are negative).
    /// `efl` is in air (the same as EFFL), `f_object` and `f_image` are focal lengths in the media (negative in object space).
    /// `bfl` is from the last surface and `ffl` from surface 1 to the focus.
    /// inf for an afocal system. Returns the values of the system after changes such as `set_param`.
    #[pyo3(signature = (wave=None))]
    fn cardinal<'py>(&self, py: Python<'py>, wave: Option<usize>) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let w = match wave {
            Some(w) => wave_index(&s, w)?,
            None => s.primary,
        };
        let c = lensbench_core::report::cardinal(&s, w);
        let d = PyDict::new(py);
        for (k, v) in [
            ("n_object", c.n_object),
            ("n_image", c.n_image),
            ("efl", c.efl),
            ("f_object", c.f_object),
            ("f_image", c.f_image),
            ("focal_object", c.focal_object),
            ("principal_object", c.principal_object),
            ("anti_principal_object", c.anti_principal_object),
            ("nodal_object", c.nodal_object),
            ("anti_nodal_object", c.anti_nodal_object),
            ("focal_image", c.focal_image),
            ("principal_image", c.principal_image),
            ("anti_principal_image", c.anti_principal_image),
            ("nodal_image", c.nodal_image),
            ("anti_nodal_image", c.anti_nodal_image),
            ("bfl", c.bfl),
            ("ffl", c.ffl),
        ] {
            d.set_item(k, v)?;
        }
        Ok(d)
    }

    /// Vignetting by the mechanical parts (mechanics of the project). The same as the CLI's `mech-check`.
    ///
    /// Traces the rays inside the unit circle of a square pupil grid (grid rays on a side) for each field in fields and wavelength in waves (both from 1; all if omitted)
    /// and counts the fraction [%] that leaves the bores of the parts. Returns a dict:
    /// `pupil_rays` (rays per field and wavelength), `parts` (name, surface, clipped_max_pct,
    /// first_max_pct and bores of each part), `results` (field, wave, rays, failed, optical_pct, passed_pct and
    /// parts=[{name, clipped_pct, first_pct}] per field and wavelength). clipped counts the rays that leave a part's bore (also those cut earlier),
    /// first the rays first cut by that part in the order the light passes, optical the rays cut by an aperture of the system before the parts.
    /// `fit` checks the placement: a list of dicts of kind (collision, protrude, loose, overlap, rod, rod_mismatch), error,
    /// part, other, surface, value [mm] and message.
    #[pyo3(signature = (fields=None, waves=None, grid=21))]
    fn mech_check<'py>(
        &self,
        py: Python<'py>,
        fields: Option<Vec<usize>>,
        waves: Option<Vec<usize>>,
        grid: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        if s.mechanics.is_empty() {
            return Err(value_err(
                "no mechanical parts (write them in the project mechanics)",
            ));
        }
        if grid < 2 {
            return Err(value_err("grid must be 2 or more"));
        }
        let nf = s.fields.len();
        let fs: Vec<usize> = match fields {
            Some(v) => v
                .into_iter()
                .map(|f| {
                    if f == 0 || f > nf {
                        Err(index_err(format!("no field {f} (1 to {nf})")))
                    } else {
                        Ok(f - 1)
                    }
                })
                .collect::<PyResult<_>>()?,
            None => (0..nf).collect(),
        };
        let ws: Vec<usize> = match waves {
            Some(v) => v
                .into_iter()
                .map(|w| wave_index(&s, w))
                .collect::<PyResult<_>>()?,
            None => (0..s.wavelengths.len()).collect(),
        };
        let (px, py_) = lensbench_core::mech::pupil_grid(grid);
        let v = py
            .detach(|| lensbench_core::mech::vignetting(&s, &s.mechanics, &fs, &ws, &px, &py_))
            .map_err(err)?;
        let pct = |n: usize, d: usize| {
            if d == 0 {
                0.0
            } else {
                100.0 * n as f64 / d as f64
            }
        };
        let d = PyDict::new(py);
        d.set_item("pupil_rays", px.len())?;
        let parts = pyo3::types::PyList::empty(py);
        for (k, p) in s.mechanics.iter().enumerate() {
            let e = PyDict::new(py);
            e.set_item("name", &p.name)?;
            e.set_item("surface", p.surface)?;
            e.set_item(
                "clipped_max_pct",
                v.iter()
                    .map(|r| pct(r.clipped[k], r.launched))
                    .fold(0.0, f64::max),
            )?;
            e.set_item(
                "first_max_pct",
                v.iter()
                    .map(|r| pct(r.first[k], r.launched))
                    .fold(0.0, f64::max),
            )?;
            let bores = pyo3::types::PyList::empty(py);
            for b in &p.bores {
                let x = PyDict::new(py);
                x.set_item("z0", b.z0)?;
                x.set_item("z1", b.z1)?;
                x.set_item("diameter", 2.0 * b.radius)?;
                bores.append(x)?;
            }
            e.set_item("bores", bores)?;
            parts.append(e)?;
        }
        d.set_item("parts", parts)?;
        let rows = pyo3::types::PyList::empty(py);
        for r in &v {
            let e = PyDict::new(py);
            e.set_item("field", r.field + 1)?;
            e.set_item("wave", r.wave + 1)?;
            e.set_item("rays", r.launched)?;
            e.set_item("failed", r.failed)?;
            e.set_item("optical_pct", pct(r.optical, r.launched))?;
            e.set_item("passed_pct", pct(r.passed, r.launched))?;
            let ps = pyo3::types::PyList::empty(py);
            for (k, p) in s.mechanics.iter().enumerate() {
                let x = PyDict::new(py);
                x.set_item("name", &p.name)?;
                x.set_item("clipped_pct", pct(r.clipped[k], r.launched))?;
                x.set_item("first_pct", pct(r.first[k], r.launched))?;
                ps.append(x)?;
            }
            e.set_item("parts", ps)?;
            rows.append(e)?;
        }
        d.set_item("results", rows)?;
        let fit = pyo3::types::PyList::empty(py);
        for f in lensbench_core::mech::fit_check(&s, &s.mechanics).map_err(err)? {
            let e = PyDict::new(py);
            e.set_item("kind", f.kind)?;
            e.set_item("error", f.error)?;
            e.set_item("part", &s.mechanics[f.part].name)?;
            e.set_item("other", f.other.map(|o| s.mechanics[o].name.clone()))?;
            e.set_item("surface", f.surface)?;
            e.set_item("value", f.value)?;
            e.set_item("message", &f.message)?;
            fit.append(e)?;
        }
        d.set_item("fit", fit)?;
        Ok(d)
    }

    /// Coupling efficiency into a single-mode fiber (like OpticStudio's FICL). The source is the object point source (local [mm] on the object surface), the receiver the local point receiver on the image surface,
    /// both Gaussian modes with a far-field 1/e² NA (na_receiver defaults to na_source). Directions with sin θ ≤ na_max in object space (min(3·NA/n, 0.999) if omitted) are
    /// sampled on a grid × grid and the overlap integral is taken in the direction space of the receiver. dict: eta (coupling efficiency), system (fraction of the source power reaching the receiver), receiver (eta/system),
    /// phase_step (largest phase difference between neighboring grid points [rad]; a warning above π/2, increase grid). With pol=True also the polarized overlap with coatings jones ((2, 2) complex, [exit x/y, input x/y];
    /// the coupling of input polarization e is |jones @ e|²). With clip_stop=True (default) light passing outside the system stop (a circle of the stop radius without an aperture) counts as system loss (as FICL). Finite-object systems only
    #[pyo3(signature = (na_source, na_receiver=None, wave=None, source=(0.0, 0.0), receiver=(0.0, 0.0), grid=101, na_max=None, pol=false, clip_stop=true))]
    #[allow(clippy::too_many_arguments)]
    fn fiber_coupling<'py>(
        &self,
        py: Python<'py>,
        na_source: f64,
        na_receiver: Option<f64>,
        wave: Option<usize>,
        source: (f64, f64),
        receiver: (f64, f64),
        grid: usize,
        na_max: Option<f64>,
        pol: bool,
        clip_stop: bool,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let w = wave.map_or(Ok(s.primary), |w| wave_index(&s, w))?;
        let o = lensbench_core::fiber::FiberOpts {
            na_source,
            na_receiver: na_receiver.unwrap_or(na_source),
            source: [source.0, source.1],
            receiver: [receiver.0, receiver.1],
            grid,
            na_max,
            clip_stop,
        };
        let coat = if pol { Some(self.coatings(&s)?) } else { None };
        let c = py
            .detach(|| lensbench_core::fiber::fiber_coupling(&s, w, &o, coat.as_deref()))
            .map_err(err)?;
        warn_all(py, &c.warning().into_iter().collect::<Vec<_>>())?;
        let d = PyDict::new(py);
        d.set_item("eta", c.eta)?;
        d.set_item("system", c.system)?;
        d.set_item("receiver", c.receiver)?;
        d.set_item("phase_step", c.phase_step)?;
        if let Some(j) = c.jones {
            let a = Array::from_shape_fn((2, 2), |(i, k)| j[i][k]);
            d.set_item("jones", a.into_pyarray(py))?;
        }
        Ok(d)
    }

    /// System data (lowercase OpticStudio operand names: effl, epdi, enpp, expd, expp, wfno, pwfn, isfn,
    /// pimh, pmag, totr, obsn, isna). At the primary wavelength, values of the system after changes
    fn system_data<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let r = py.detach(|| lensbench_core::report::system_data(&s));
        let d = PyDict::new(py);
        for (k, v) in [
            ("effl", r.effl),
            ("epdi", r.epdi),
            ("enpp", r.enpp),
            ("expd", r.expd),
            ("expp", r.expp),
            ("wfno", r.wfno),
            ("pwfn", r.pwfn),
            ("isfn", r.isfn),
            ("pimh", r.pimh),
            ("pmag", r.pmag),
            ("totr", r.totr),
            ("obsn", r.obsn),
            ("isna", r.isna),
        ] {
            d.set_item(k, v)?;
        }
        Ok(d)
    }

    /// Normalized field (hx, hy) of field number i (from 1)
    fn hxhy(&self, i: usize) -> PyResult<(f64, f64)> {
        let s = self.sys();
        match field_sel(&s, FieldArg::Index(i))? {
            FieldSel::Index(j) => Ok(s.hxhy(j)),
            FieldSel::Norm(..) => unreachable!(),
        }
    }

    /// The object of a field (number or (hx, hy)): an object point (x, y) [mm] for a finite object, a direction (l, m, n) at infinity
    fn object_of(&self, field: FieldArg) -> PyResult<Vec<f64>> {
        let s = self.sys();
        let (hx, hy) = match field_sel(&s, field)? {
            FieldSel::Index(j) => s.hxhy(j),
            FieldSel::Norm(a, b) => (a, b),
        };
        Ok(obj_out(s.object_spec(hx, hy).map_err(err)?))
    }

    /// Find a surface by number or name (comment) and return its surface number
    fn find_surface(&self, target: &str) -> PyResult<usize> {
        self.sys().find_surface(target).map_err(err)
    }

    /// Lenses and groups: dicts of name, spans (list of surface ranges [first, last]), pivot, motion (dx dy tx ty tz), shift and auto
    fn units<'py>(&self, gil: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let s = self.sys();
        s.units
            .iter()
            .map(|u| {
                let d = PyDict::new(gil);
                d.set_item("name", &u.name)?;
                d.set_item(
                    "spans",
                    u.spans.iter().map(|x| (x[0], x[1])).collect::<Vec<_>>(),
                )?;
                d.set_item("pivot", u.pivot.to_vec())?;
                d.set_item("motion", u.motion.to_vec())?;
                d.set_item("shift", u.shift)?;
                d.set_item("auto", u.auto)?;
                Ok(d)
            })
            .collect()
    }

    /// Define a lens or group. spans is a list of surface ranges (first, last) (two when the same lens is passed twice, such as the way out
    /// and back of a double pass), pivot the center of rotation (coordinates with the vertex of the first surface of the first range as origin). Then changed with "name.dx" and so on
    #[pyo3(signature = (name, spans, pivot=(0.0, 0.0, 0.0)))]
    fn define_unit(
        &self,
        name: &str,
        spans: Vec<(usize, usize)>,
        pivot: (f64, f64, f64),
    ) -> PyResult<()> {
        let mut g = self.sys.write_ok();
        let mut n = g.with_params(&[]).map_err(err)?;
        let sp: Vec<[usize; 2]> = spans.into_iter().map(|(a, b)| [a, b]).collect();
        n.define_unit(name, &sp, [pivot.0, pivot.1, pivot.2])
            .map_err(err)?;
        *g = Arc::new(n);
        Ok(())
    }

    /// Change the center of rotation of a lens or group
    fn set_unit_pivot(&self, name: &str, pivot: (f64, f64, f64)) -> PyResult<()> {
        let mut g = self.sys.write_ok();
        let Some(u) = g.find_unit(name) else {
            return Err(value_err(format!("no lens or group {name}")));
        };
        let mut n = g.with_params(&[]).map_err(err)?;
        n.set_unit_pivot(u, [pivot.0, pivot.1, pivot.2])
            .map_err(err)?;
        *g = Arc::new(n);
        Ok(())
    }

    /// The current value of a parameter. spec is "surface.kind" (e.g. "3.t", "L1 front.c"). kind is c r k a2 to a16 t dx dy tx ty tz dn disp.
    /// Lenses and groups are "name.kind" (e.g. "lens2.tx", "G1.shift") with kind dx dy tx ty tz shift dn disp
    fn get_param(&self, spec: &str) -> PyResult<f64> {
        let s = self.sys();
        let p = s.param(spec).map_err(err)?;
        s.get_param(p).map_err(err)
    }

    /// Change a parameter (without rebuilding the system). Pickup targets are solved again from their source surfaces (a pickup target itself can't be changed)
    fn set_param(&self, spec: &str, value: f64) -> PyResult<()> {
        let mut g = self.sys.write_ok();
        let p = g.param(spec).map_err(err)?;
        let n = g.with_params(&[(p, value)]).map_err(err)?;
        *g = Arc::new(n);
        Ok(())
    }

    /// Change several at once: {"3.t": 10.0, "5.dx": 0.01} or [("3.t", 10.0), ...]
    fn set_params(&self, changes: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut g = self.sys.write_ok();
        let c = parse_changes(&g, changes)?;
        let n = g.with_params(&c).map_err(err)?;
        *g = Arc::new(n);
        Ok(())
    }

    /// Return a new changed system (the original system is not changed; a light copy)
    fn with_params(&self, changes: &Bound<'_, PyAny>) -> PyResult<PySystem> {
        let s = self.sys();
        let c = parse_changes(&s, changes)?;
        let n = s.with_params(&c).map_err(err)?;
        Ok(PySystem::new(n, self.coat_files.clone(), self.coat_cell()))
    }

    // ---------------------------------------------------------------------- 系の編集
    /// A dict of the values of surface s (type comment t c r k asph cx material mirror coating semi stop ignore aperture cb cb_order
    /// focal jones dn disp n). r is the radius of curvature (inf for a plane), aperture a dict or None
    fn surface<'py>(&self, py: Python<'py>, s: SurfArg) -> PyResult<Bound<'py, PyDict>> {
        let sys = self.sys();
        let j = surf_of(&sys, s)?;
        edit::fields_dict(py, sys.surface_fields(j).map_err(err)?)
    }

    /// Number of surfaces (including the object and image surfaces)
    #[getter]
    fn surface_count(&self) -> usize {
        self.sys().surfaces.len()
    }

    /// Insert a surface before surface at (at is 1 to the image surface). Values as keywords (the names of surface(): type material r t and so on).
    /// The medium after the inserted surface is its material (air if omitted)
    #[pyo3(signature = (at, **kw))]
    fn insert_surface(&self, at: usize, kw: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let f = edit::surf_kwargs(kw)?;
        self.mutate(|s| {
            s.insert_surface(at, lensbench_core::surface::Surface::default())?;
            s.set_surface_fields(at, &f)
        })
    }

    /// Delete surface s. Returns a list describing the pickups, solves and multi-configuration rows that referred to it (also as warnings)
    fn delete_surface(&self, py: Python<'_>, s: SurfArg) -> PyResult<Vec<String>> {
        let j = surf_of(&self.sys(), s)?;
        let notes = self.mutate(|s| s.delete_surface(j))?;
        warn_all(py, &notes)?;
        Ok(notes)
    }

    /// Change values of surface s with keywords (the names of surface(), except mirror, stop and n; material for the material)
    #[pyo3(signature = (s, **kw))]
    fn set_surface(&self, s: SurfArg, kw: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let j = surf_of(&self.sys(), s)?;
        let f = edit::surf_kwargs(kw)?;
        self.mutate(|s| s.set_surface_fields(j, &f))
    }

    /// The type of surface s (STANDARD EVENASPH COORDBRK JONESMAT PARAXIAL TOROIDAL FRESNELS). Values the new type doesn't have go back to 0
    fn set_surface_type(&self, s: SurfArg, kind: &str) -> PyResult<()> {
        let j = surf_of(&self.sys(), s)?;
        let t = lensbench_core::edit::surf_type_parse(kind).map_err(err)?;
        self.mutate(|s| s.set_surface_type(j, t))
    }

    /// The material of surface s: a glass name (the system's catalogs; add more with add_glass_catalogs), "nd,vd[,dpgf]", "" or "AIR", "MIRROR"
    fn set_material(&self, s: SurfArg, material: &str) -> PyResult<()> {
        let j = surf_of(&self.sys(), s)?;
        self.mutate(|s| s.set_material(j, material))
    }

    /// Make surface s the stop
    fn set_stop(&self, s: SurfArg) -> PyResult<()> {
        let j = surf_of(&self.sys(), s)?;
        self.mutate(|s| s.set_stop(j))
    }

    /// Read glass catalogs (AGF names, e.g. "SCHOTT") and add them to the system. dirs are where to search (default places if omitted).
    /// Returns the names of the catalogs not found
    #[pyo3(signature = (names, dirs=None))]
    fn add_glass_catalogs(
        &self,
        names: Vec<String>,
        dirs: Option<Vec<PathBuf>>,
    ) -> PyResult<Vec<String>> {
        let dirs = dirs.unwrap_or_default();
        self.mutate(|s| Ok(s.add_glass_catalogs(&names, &dirs)))
    }

    /// Pickups: dicts of dst, src, what (c t k p1… before0–4 after0–4 and so on), scale and offset, with src_what when the source quantity differs from what
    fn pickups<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        self.sys()
            .pickups
            .iter()
            .map(|p| {
                let d = PyDict::new(py);
                d.set_item("dst", p.dst)?;
                d.set_item("src", p.src)?;
                d.set_item("what", edit::pick_name(p.what))?;
                d.set_item("scale", p.scale)?;
                d.set_item("offset", p.offset)?;
                if let Some(w) = p.from {
                    d.set_item("src_what", edit::pick_name(w))?;
                }
                Ok(d)
            })
            .collect()
    }

    /// Add a pickup: what of surface dst = scale × src_what of surface src + offset (replaces one with the same target and quantity).
    /// what is c r t k p1… dn disp semi before0–4 after0–4 (tilts and decenters before and after the surface: dx dy tx ty tz).
    /// src_what defaults to what (e.g. to take the after of a surface from the before of the source surface: what="after0", src_what="before0")
    #[pyo3(signature = (dst, src, what, scale=1.0, offset=0.0, src_what=None))]
    fn add_pickup(
        &self,
        dst: SurfArg,
        src: SurfArg,
        what: &str,
        scale: f64,
        offset: f64,
        src_what: Option<&str>,
    ) -> PyResult<()> {
        let s0 = self.sys();
        let mut p = lensbench_core::pickup::Pickup::new(
            surf_of(&s0, dst)?,
            surf_of(&s0, src)?,
            edit::pick_kind(what)?,
            scale,
            offset,
        );
        if let Some(w) = src_what {
            let w = edit::pick_kind(w)?;
            p.from = (w != p.what).then_some(w);
        }
        self.mutate(|s| s.add_pickup(p))
    }

    /// Remove the pickup of what on surface dst (the value stays as it is)
    fn remove_pickup(&self, dst: SurfArg, what: &str) -> PyResult<()> {
        let j = surf_of(&self.sys(), dst)?;
        let w = edit::pick_kind(what)?;
        self.mutate(|s| s.remove_pickup(j, w))
    }

    /// Solves: dicts of surf, what ("t" thickness / "c" curvature), kind (name of the kind), value, param and other
    fn solves<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        self.sys()
            .solves
            .iter()
            .map(|q| {
                let (name, value, param, other) = lensbench_core::solves::solve_kind_info(q.kind);
                let d = PyDict::new(py);
                d.set_item("surf", q.surf)?;
                d.set_item("what", if q.kind.is_curv() { "c" } else { "t" })?;
                d.set_item("kind", name)?;
                d.set_item("value", value)?;
                d.set_item("param", param)?;
                d.set_item("other", other)?;
                Ok(d)
            })
            .collect()
    }

    /// Add a solve on surface surf (replaces one on the same surface and quantity). kind is, for thickness, marginal_height (default),
    /// marginal_height_zone, chief_height, pupil_position, edge, opd, position, compensator, center_of_curvature;
    /// for curvature, marginal_angle, chief_angle, marginal_normal, chief_normal, aplanatic, element_power,
    /// concentric_surface, concentric_radius, f_number. value is its value (height, angle, length and so on), param is
    /// the pupil zone of marginal_height_zone and opd or the height of edge, other the surface for kinds that refer to another surface
    #[pyo3(signature = (surf, kind="marginal_height", value=0.0, other=None, param=0.0))]
    fn add_solve(
        &self,
        surf: SurfArg,
        kind: &str,
        value: f64,
        other: Option<SurfArg>,
        param: f64,
    ) -> PyResult<()> {
        let s0 = self.sys();
        let other = other.map(|f| surf_of(&s0, f)).transpose()?;
        let q = lensbench_core::solves::Solve {
            surf: surf_of(&s0, surf)?,
            kind: lensbench_core::solves::solve_kind_parse(kind, value, param, other)
                .map_err(err)?,
        };
        self.mutate(|s| s.add_solve(q))
    }

    /// Remove the solve of what ("t" thickness / "c" curvature) on surface surf (the value stays as it is)
    #[pyo3(signature = (surf, what="t"))]
    fn remove_solve(&self, surf: SurfArg, what: &str) -> PyResult<()> {
        let j = surf_of(&self.sys(), surf)?;
        let k = match what {
            "t" => lensbench_core::param::Kind::Thick,
            "c" => lensbench_core::param::Kind::Curv,
            _ => return Err(value_err(format!("what: t or c ({what})"))),
        };
        self.mutate(|s| s.remove_solve(j, k))
    }

    /// Number of configurations (1 without a table)
    #[getter]
    fn config_count(&self) -> usize {
        self.sys().configs().map_or(1, |c| c.n)
    }

    /// The configuration in the current system (from 1)
    #[getter]
    fn active_config(&self) -> usize {
        self.sys().configs().map_or(1, |c| c.active)
    }

    /// Set the number of configurations to n (added configurations take the current values; 1 removes the table)
    fn set_config_count(&self, n: usize) -> PyResult<()> {
        self.mutate(|s| s.set_config_count(n))
    }

    /// The multi-configuration table: a list of (parameter name, values per configuration)
    fn configs(&self) -> Vec<(String, Vec<f64>)> {
        let s = self.sys();
        s.configs()
            .map(|c| {
                c.rows
                    .iter()
                    .map(|r| (s.param_name(r.param), r.values.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Add a row to the table (replaces the same parameter): spec is the same name as in set_param, values the values per configuration
    fn set_config_row(&self, spec: &str, values: Vec<f64>) -> PyResult<()> {
        self.mutate(|s| {
            let p = s.param(spec)?;
            s.set_config_row(p, values)
        })
    }

    /// Remove a row of the table (the system's value stays as it is)
    fn remove_config_row(&self, spec: &str) -> PyResult<()> {
        self.mutate(|s| {
            let p = s.param(spec)?;
            s.remove_config_row(p)
        })
    }

    /// A new system with the values of configuration k (from 1) (the original system is not changed)
    fn config(&self, k: usize) -> PyResult<PySystem> {
        let n = self.sys().config(k).map_err(err)?;
        Ok(PySystem::new(n, self.coat_files.clone(), self.coat_cell()))
    }

    /// Switch the current system to configuration k
    fn set_config(&self, k: usize) -> PyResult<()> {
        self.mutate(|s| {
            *s = s.config(k)?;
            Ok(())
        })
    }

    /// Ghost: the system with the path reflected at surface a and then at surface b (b < a) unfolded into sequential surfaces (coating coefficients and polarization traces work as they are).
    /// Surfaces by number or comment. The original surface of each surface of the unfolded system is in ghost_origin
    fn ghost(&self, a: SurfArg, b: SurfArg) -> PyResult<PySystem> {
        let s = self.sys();
        let (a, b) = (surf_of(&s, a)?, surf_of(&s, b)?);
        let g = lensbench_analysis::ghost::ghost_system(&s, a, b).map_err(err)?;
        Ok(PySystem::new(
            g.sys,
            self.coat_files.clone(),
            self.coat_cell(),
        ))
    }

    /// For each surface of a ghost system (ghost(a, b)) the original surface number (None for coordinate breaks put in between), and the reflecting surfaces (a, b)
    fn ghost_origin(&self, a: SurfArg, b: SurfArg) -> PyResult<GhostOrigin> {
        let s = self.sys();
        let (a, b) = (surf_of(&s, a)?, surf_of(&s, b)?);
        let g = lensbench_analysis::ghost::ghost_system(&s, a, b).map_err(err)?;
        Ok((g.origin, g.at))
    }

    /// Pairs of double reflections [(a, b), ...] (a > b, a reflects first). Surfaces first..=last (1 to the image surface if omitted; the image surface only with a coating)
    #[pyo3(signature = (first=1, last=None))]
    fn ghost_pairs(&self, first: usize, last: Option<usize>) -> Vec<(usize, usize)> {
        let s = self.sys();
        let last = last.unwrap_or(s.image());
        lensbench_analysis::ghost::pairs(&s, first, last)
    }

    /// The field type: "angle", "object_height", "paraxial_image_height", "real_image_height",
    /// or ("solve", ("dirsum", surface1, surface2)) and so on for a field given by a condition
    #[getter]
    fn field_type<'py>(&self, gil: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let s = self.sys();
        let name = match s.field_type {
            FieldType::Angle => "angle",
            FieldType::ObjH => "object_height",
            FieldType::ParH => "paraxial_image_height",
            FieldType::RealH => "real_image_height",
            FieldType::Solve(c) => {
                let c = match c {
                    FieldCond::Position { surf } => ("position", surf).into_pyobject(gil)?,
                    FieldCond::DirSum { s1, s2 } => ("dirsum", s1, s2).into_pyobject(gil)?,
                    FieldCond::DirDiff { s1, s2 } => ("dirdiff", s1, s2).into_pyobject(gil)?,
                };
                return Ok(("solve", c).into_pyobject(gil)?.into_any());
            }
        };
        Ok(name.into_pyobject(gil)?.into_any())
    }

    /// Change the field type and values. kind is the same name as field_type. For "solve" give the condition in cond
    /// (("position", surface), ("dirsum", surface1, surface2), ("dirdiff", surface1, surface2); surfaces by number or name);
    /// the object whose chief ray at the primary wavelength makes the field (x, y) the target of the condition is solved (again when parameters change).
    /// fields also replaces the field (x, y) values (weights and vignetting factors go back to the defaults)
    #[pyo3(signature = (kind, cond=None, fields=None))]
    fn set_field_type(
        &self,
        kind: &str,
        cond: Option<Bound<'_, PyTuple>>,
        fields: Option<Vec<(f64, f64)>>,
    ) -> PyResult<()> {
        let mut g = self.sys.write_ok();
        let ft = match (kind.to_ascii_lowercase().as_str(), &cond) {
            ("angle", None) => FieldType::Angle,
            ("object_height", None) => FieldType::ObjH,
            ("paraxial_image_height", None) => FieldType::ParH,
            ("real_image_height", None) => FieldType::RealH,
            ("solve", Some(c)) => FieldType::Solve(edit::field_cond(&g, c)?),
            ("solve", None) => return Err(value_err("solve needs cond")),
            (_, Some(_)) => return Err(value_err("cond only when kind is solve")),
            (k, None) => {
                return Err(value_err(format!(
                    "field type {k} (angle, object_height, paraxial_image_height, real_image_height, solve)"
                )))
            }
        };
        let mut n = g.with_params(&[]).map_err(err)?.into_prescription();
        n.field_type = ft;
        if let Some(f) = fields {
            if f.is_empty() {
                return Err(value_err("no fields given"));
            }
            n.fields = f.into_iter().map(|(x, y)| Field::xy(x, y)).collect();
        }
        let n = n.build_raw().map_err(err)?;
        *g = Arc::new(n);
        Ok(())
    }

    /// Trace rays and return a dict of numpy arrays.
    ///
    /// - field: a field number (from 1) or a normalized field (hx, hy). None when object is given
    /// - px, py: normalized pupil coordinates (1-D, same length; lists and float32 also work)
    /// - waves: a list of wavelength numbers (from 1). All wavelengths if omitted
    /// - last: stop at this surface (the image surface if omitted)
    /// - record: True also returns the intersections at all surfaces on the way (`rec_*`, shape (wavelength, surface, ray)). A list of surface numbers
    ///   gives only those surfaces (except ignored surfaces and those after last; faster for systems with many surfaces)
    /// - pol: the wavelength number for a polarization trace (True for the primary wavelength). Returns complex128 (ray, 2, 2) in `jones`.
    ///   Columns are the input polarization (unit vectors of local x and y on the object side), rows the x and y components of the exit field in local coordinates of the last surface.
    ///   The z components are `jones_z` ((ray, 2)). With coating amplitudes and phases; the phase convention is E ∝ exp(−ikL). NaN for stopped rays
    /// - pol_phase: True multiplies jones by the propagation phase exp(−ik·OPL) (not by default; the optical path length is `opl`)
    /// - pol_grid: does the polarization trace on a coarse grid of normalized pupil coordinates [−1, 1]² (n points or (nx, ny), at least 4 per axis, e.g. 25) and
    ///   interpolates the Jones matrices without the propagation phase to all rays with a 2-D cubic spline (not-a-knot) (faster than a polarization trace
    ///   of all rays). Stopped or vignetted grid points are filled from the nearest valid point of the grid. Grid points missing inside the pupil (px² + py² ≤ 1)
    ///   are returned in `pol_grid_missing` ((point, 2) of px, py); rays in cells within 2 points of them get NaN jones
    ///   and True in `pol_grid_bad` ((ray,) bool). Only when pupil is "normalized" or "signed"
    /// - globals: True also returns the global positions and directions gx, gy, gz, gl, gm, gn and the start points and directions p0, k0 ((wavelength, ray, 3))
    /// - object: the object directly instead of a field: an object point (x, y) [mm] for a finite object, a direction (l, m, n) for an infinite object.
    ///   No vignetting factors are applied
    /// - grad: a list of parameters ("3.t", "objx" and so on) to differentiate the optical path length by. Returns dOPL/dp in `grad` ((wavelength, ray, parameter)).
    ///   The derivative with the start point and the intersection with the last surface (local x, y) fixed. objx and objy for finite objects only
    /// - land: with grad, True also returns in `grad_land` ((wavelength, ray, parameter, 4)) the derivative of the landing point and direction at the last surface
    ///   d(x, y, l, m)/dp. The derivative with the start position and direction fixed (the same ray is launched and only the system changes;
    ///   not solved again even with pupil="stop" or aimed rays)
    /// - dz: puts the detector on the plane z = dz in local coordinates of the last surface (a plane following the tilts and decenters of the surface). x, y, z, opl,
    ///   opl_os and the global positions become the values on that plane, grad the derivative with x and y on that plane fixed, and grad_land is on that plane too.
    ///   record is not changed. Can't be used with pol_phase=True
    /// - threads: number of threads (1 for sequential; all cores if omitted). The results are bit for bit the same for any number of threads
    /// - pupil: with "normalized" (default) px and py are normalized pupil coordinates (as in OpticStudio; in finite-object systems with the entrance pupil before
    ///   the object, px = 1 passes the −x side of the stop, and the direction flips across telecentricity). "signed" is
    ///   the pupil coordinate with the sign flipped for those systems (px = 1 always on the same side of the stop; opposite in sign to normalized for systems where
    ///   `System.pupil_reversed` is True). With "stop" they are local coordinates [mm] on the stop surface, and
    ///   the real ray through that point is solved for each ray (independent of the system's aiming and entrance pupil position; no vignetting factors;
    ///   rays that can't be solved have err at the stop surface)
    ///
    /// Returned arrays (shape (wavelength, ray)): x, y, z, l, m, n (position and direction cosines in local coordinates of the last surface),
    /// opl (optical path length), opl_os (OpticStudio's raw optical path length), lens_opd (phase of ideal lenses), err and vig (number of the surface where the ray stopped
    /// or was vignetted, 0 if it passed), ok (err and vig both 0). Also waves (numbers), last, hx and hy for a field,
    /// object for an object. With record, rec_surfaces and rec_x … rec_opl (rec_gx, rec_gy, rec_gz are global)
    #[pyo3(signature = (field, px, py, waves=None, last=None, record=RecArg::Flag(false), pol=None, pol_phase=false, globals=false, object=None, grad=None, threads=None, pupil="normalized", dz=None, land=false, pol_grid=None))]
    #[allow(clippy::too_many_arguments)]
    fn trace<'py>(
        &self,
        gil: Python<'py>,
        field: Option<FieldArg>,
        px: PyArrayLike1<'py, f64, AllowTypeChange>,
        py: PyArrayLike1<'py, f64, AllowTypeChange>,
        waves: Option<Vec<usize>>,
        last: Option<usize>,
        record: RecArg,
        pol: Option<PolArg>,
        pol_phase: bool,
        globals: bool,
        object: Option<ObjArg>,
        grad: Option<Vec<String>>,
        threads: Option<usize>,
        pupil: &str,
        dz: Option<f64>,
        land: bool,
        pol_grid: Option<GridArg>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let (px, pyv) = (floats(&px), floats(&py));
        let pre = Prep::new(&s, field, object, pupil, px.len(), pyv.len())?;
        let mut opts = TraceOpts::new(
            &s, waves, last, record, pol, pol_phase, globals, grad, threads, dz, land,
        )?;
        if let Some(g) = pol_grid {
            let g = match g {
                GridArg::N(k) => (k, k),
                GridArg::Nxy(v) => v,
            };
            if opts.pw.is_none() {
                return Err(value_err("give pol_grid together with pol"));
            }
            if pre.at_stop {
                return Err(value_err(
                    "pol_grid only when pupil is \"normalized\" or \"signed\"",
                ));
            }
            if g.0 < polgrid::MIN_POINTS || g.1 < polgrid::MIN_POINTS {
                return Err(value_err(format!(
                    "pol_grid needs {} points or more on each axis ({:?})",
                    polgrid::MIN_POINTS,
                    g
                )));
            }
            opts.pol_grid = Some(g);
        }
        let d = self.run_trace(gil, &s, &pre.launch(&px, &pyv), px.len(), &opts)?;
        pre.finish(&d)?;
        Ok(d)
    }

    /// Trace from start positions and directions given directly. The returned dict is the same as [`trace`] (without hx, hy and object).
    ///
    /// - start, direction: (ray, 3) arrays. Directions are normalized (a zero vector is a stopped ray). The optical path length is from the start position
    /// - frame: "object" (default; local coordinates of the object surface for a finite object, the coordinates before the transform of surface 1 for an infinite object) or "global"
    /// - the other arguments are as in trace. objx and objy of grad are derivatives with the start positions moving with the object surface
    #[pyo3(signature = (start, direction, waves=None, last=None, record=RecArg::Flag(false), pol=None, pol_phase=false, globals=false, frame="object", grad=None, threads=None, dz=None, land=false))]
    #[allow(clippy::too_many_arguments)]
    fn trace_rays<'py>(
        &self,
        gil: Python<'py>,
        start: PyArrayLike2<'py, f64, AllowTypeChange>,
        direction: PyArrayLike2<'py, f64, AllowTypeChange>,
        waves: Option<Vec<usize>>,
        last: Option<usize>,
        record: RecArg,
        pol: Option<PolArg>,
        pol_phase: bool,
        globals: bool,
        frame: &str,
        grad: Option<Vec<String>>,
        threads: Option<usize>,
        dz: Option<f64>,
        land: bool,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let rows =
            |a: &PyArrayLike2<'py, f64, AllowTypeChange>, name: &str| -> PyResult<Vec<[f64; 3]>> {
                let v = a.as_array();
                if v.ncols() != 3 {
                    return Err(value_err(format!(
                        "the shape of {name} must be (rays, 3) ({:?})",
                        v.shape()
                    )));
                }
                Ok(v.rows().into_iter().map(|r| [r[0], r[1], r[2]]).collect())
            };
        let (mut p, mut k) = (rows(&start, "start")?, rows(&direction, "direction")?);
        if p.len() != k.len() {
            return Err(value_err(format!(
                "start and direction have different numbers of rays ({} and {})",
                p.len(),
                k.len()
            )));
        }
        match frame.to_ascii_lowercase().as_str() {
            "global" => {}
            "object" => {
                let f = s.object_frame();
                let mv = |r: &[[f64; 3]; 3], v: [f64; 3]| {
                    [0, 1, 2].map(|c| r[c][0] * v[0] + r[c][1] * v[1] + r[c][2] * v[2])
                };
                for q in p.iter_mut() {
                    let g = mv(&f.r, *q);
                    *q = [f.o[0] + g[0], f.o[1] + g[1], f.o[2] + g[2]];
                }
                for q in k.iter_mut() {
                    *q = mv(&f.r, *q);
                }
            }
            f => {
                return Err(value_err(format!(
                    "frame must be \"object\" or \"global\" ({f})"
                )))
            }
        }
        let launch = Launch::Rays { p: &p, k: &k };
        let opts = TraceOpts::new(
            &s, waves, last, record, pol, pol_phase, globals, grad, threads, dz, land,
        )?;
        self.run_trace(gil, &s, &launch, p.len(), &opts)
    }

    /// Solve a real ray with two unknowns and two residuals by Newton's method.
    ///
    /// - unknown: "object" (x and y of the object point [mm], finite object) or "pupil" (pupil coordinates px and py)
    /// - residual: one of ("position", surface, (x, y)) (local position on the surface − target), ("dirsum", surface1, surface2) (sum of the direction cosines
    ///   l and m) and ("dirdiff", surface1, surface2) (difference)
    /// - object: the object (an object point (x, y) or a direction (l, m, n)). The initial value when the unknown is the object point. The object of field if omitted
    /// - field: the field when object is omitted (number or (hx, hy); on axis if omitted)
    /// - pupil: pupil coordinates. The initial value when the unknown is the pupil coordinates
    /// - wave: wavelength number (primary wavelength if omitted)
    ///
    /// Returned dict: object, pupil, residual, iterations (can be passed as they are to trace(None, [px], [py], object=...))
    #[pyo3(signature = (unknown, residual, object=None, field=None, pupil=(0.0, 0.0), wave=None, tol=1e-12, max_iter=50))]
    #[allow(clippy::too_many_arguments)]
    fn solve_ray<'py>(
        &self,
        gil: Python<'py>,
        unknown: &str,
        residual: &Bound<'py, PyTuple>,
        object: Option<ObjArg>,
        field: Option<FieldArg>,
        pupil: (f64, f64),
        wave: Option<usize>,
        tol: f64,
        max_iter: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let unknown = match unknown.to_ascii_lowercase().as_str() {
            "object" => Unknown::Object,
            "pupil" => Unknown::Pupil,
            u => return Err(value_err(format!("unknown must be object or pupil ({u})"))),
        };
        let kind: String = residual.get_item(0)?.extract()?;
        let res = match (kind.to_ascii_lowercase().as_str(), residual.len()) {
            ("position", 3) => Residual::Position {
                surf: residual.get_item(1)?.extract()?,
                target: {
                    let (x, y): (f64, f64) = residual.get_item(2)?.extract()?;
                    [x, y]
                },
            },
            ("dirsum", 3) => Residual::DirSum {
                s1: residual.get_item(1)?.extract()?,
                s2: residual.get_item(2)?.extract()?,
            },
            ("dirdiff", 3) => Residual::DirDiff {
                s1: residual.get_item(1)?.extract()?,
                s2: residual.get_item(2)?.extract()?,
            },
            _ => {
                return Err(value_err(
                    "residual must be (\"position\", surface, (x, y)), (\"dirsum\", surface1, surface2) or (\"dirdiff\", surface1, surface2)",
                ))
            }
        };
        let obj = match (object, field) {
            (Some(o), None) => obj_spec(&s, o)?,
            (None, f) => {
                let (hx, hy) = match f {
                    None => (0.0, 0.0),
                    Some(f) => match field_sel(&s, f)? {
                        FieldSel::Index(j) => s.hxhy(j),
                        FieldSel::Norm(a, b) => (a, b),
                    },
                };
                s.object_spec(hx, hy).map_err(err)?
            }
            (Some(_), Some(_)) => return Err(value_err("not both object and field")),
        };
        let w = match wave {
            None => s.primary,
            Some(w) => wave_index(&s, w)?,
        };
        let mut prob = RayProblem::new(unknown, res, obj, w);
        prob.pupil = [pupil.0, pupil.1];
        prob.tol = tol;
        prob.max_iter = max_iter;
        let sys = &*s;
        let sol = gil.detach(|| sys.solve_ray(&prob)).map_err(err)?;
        let d = PyDict::new(gil);
        d.set_item("object", obj_out(sol.object))?;
        d.set_item("pupil", (sol.pupil[0], sol.pupil[1]))?;
        d.set_item("residual", (sol.residual[0], sol.residual[1]))?;
        d.set_item("iterations", sol.iterations)?;
        Ok(d)
    }

    /// Diffraction MTF (the same as the CLI's mtf). field is a number (from 1) or a normalized field (hx, hy), wave a wavelength number (polychromatic
    /// if omitted), n a guide for the number of quadrature points of the pupil overlap (automatic if omitted). method is "auto" (default; "interp", or "fft" when n is omitted,
    /// the quadrature has more than 1024 points and there are 8 or more frequencies), "interp" (interpolated quadrature), "fft" (autocorrelation of the pupil function by FFT, all frequencies at once;
    /// fast with many frequencies, about 1e-4 from the quadrature) or "direct" (for checking).
    /// Returned dict: freqs, tangential, sagittal (numpy), cutoff (tangential and sagittal cutoff frequencies [cycles/mm] at the calculated wavelength; the primary wavelength for polychromatic).
    /// each_wave=True also returns waves (a list of dicts per wavelength, faster than computing them separately) besides the polychromatic one (can't be used with wave).
    /// axes is "field" (default; tangential along the field direction) or "pupil" (tangential along the pupil y, as in OpticStudio's FFT MTF); they differ only for fields with an x component.
    #[pyo3(signature = (field, freqs, wave=None, n=None, threads=None, method="auto", each_wave=false, axes="field"))]
    #[allow(clippy::too_many_arguments)]
    fn mtf<'py>(
        &self,
        py: Python<'py>,
        field: FieldArg,
        freqs: PyArrayLike1<'_, f64, AllowTypeChange>,
        wave: Option<usize>,
        n: Option<usize>,
        threads: Option<usize>,
        method: &str,
        each_wave: bool,
        axes: &str,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let f = field_sel(&s, field)?;
        let w = Waves::all_or(wave.map(|w| wave_index(&s, w)).transpose()?);
        let fs = floats(&freqs);
        let (method, axes) = (mtfs::method(method)?, mtfs::axes(axes)?);
        if each_wave {
            if w != Waves::All {
                return Err(value_err("each_wave can't be used with wave"));
            }
            let mw = py
                .detach(|| {
                    with_threads(threads, || {
                        analysis::mtf_waves_amp(&s, f, n, &fs, method, None, axes)
                    })
                })
                .map_err(err)?;
            let d = mtf_dict(py, &mw.poly)?;
            let waves = mw
                .waves
                .iter()
                .map(|m| mtf_dict(py, m))
                .collect::<PyResult<Vec<_>>>()?;
            d.set_item("waves", waves)?;
            return Ok(d);
        }
        let m = py
            .detach(|| {
                with_threads(threads, || {
                    analysis::mtf_amp(&s, f, w, n, &fs, method, None, None, axes)
                })
            })
            .map_err(err)?;
        mtf_dict(py, &m)
    }

    /// Through-focus MTF: the diffraction MTF with the image surface shifted by dz [lens units] (changes the thickness before the image surface;
    /// thickness solves are not solved again). tangential and sagittal are (frequency, dz) arrays. Points that can't be calculated are NaN, with
    /// the reasons in warnings. axes is "field" (default; tangential along the field direction) or "pupil" (tangential along the pupil y, as in OpticStudio's FFT MTF); they differ only for fields with an x component.
    #[pyo3(signature = (field, freqs, dz, wave=None, n=None, threads=None, axes="field"))]
    #[allow(clippy::too_many_arguments)]
    fn mtf_focus<'py>(
        &self,
        py: Python<'py>,
        field: FieldArg,
        freqs: PyArrayLike1<'_, f64, AllowTypeChange>,
        dz: PyArrayLike1<'_, f64, AllowTypeChange>,
        wave: Option<usize>,
        n: Option<usize>,
        threads: Option<usize>,
        axes: &str,
    ) -> PyResult<Bound<'py, PyDict>> {
        let (s, ax) = (self.sys(), mtfs::axes(axes)?);
        let f = field_sel(&s, field)?;
        let w = Waves::all_or(wave.map(|w| wave_index(&s, w)).transpose()?).weighted(&s);
        let (w, fs, z) = (w.map_err(err)?, floats(&freqs), floats(&dz));
        let m = py.detach(|| {
            with_threads(threads, || {
                sweep::through_focus_mtf(&s, f, w, n, &fs, &z, ax)
            })
        });
        sweep_dict(py, "dz", &m)
    }

    /// Field MTF: the diffraction MTF for each normalized field h (0 to 1, in the direction axis "y" or "x").
    /// tangential and sagittal are (frequency, h) arrays, field_value the field values (system units).
    /// axes is "field" (default; tangential along the field direction) or "pupil" (tangential along the pupil y, as in OpticStudio's FFT MTF); they differ only for fields with an x component.
    #[pyo3(signature = (freqs, h, axis="y", wave=None, n=None, threads=None, axes="field"))]
    #[allow(clippy::too_many_arguments)]
    fn mtf_field<'py>(
        &self,
        py: Python<'py>,
        freqs: PyArrayLike1<'_, f64, AllowTypeChange>,
        h: PyArrayLike1<'_, f64, AllowTypeChange>,
        axis: &str,
        wave: Option<usize>,
        n: Option<usize>,
        threads: Option<usize>,
        axes: &str,
    ) -> PyResult<Bound<'py, PyDict>> {
        let (s, dirs) = (self.sys(), mtfs::axes(axes)?);
        let ax = field_axis(axis)?;
        let w = Waves::all_or(wave.map(|w| wave_index(&s, w)).transpose()?).weighted(&s);
        let (w, fs, hs) = (w.map_err(err)?, floats(&freqs), floats(&h));
        let m =
            py.detach(|| with_threads(threads, || sweep::field_mtf(&s, ax, &hs, w, n, &fs, dirs)));
        let d = sweep_dict(py, "h", &m)?;
        let fv: Vec<f64> = hs.iter().map(|&x| ax.value(&s, x)).collect();
        d.set_item("field_value", fv.into_pyarray(py))?;
        Ok(d)
    }

    /// Return the system optimized by damped least squares (DLS) (the original system is not changed). vars is a list of variables, each "7.t" or
    /// ("7.t", lower, upper) (None for no bound on that side). The default merit function merit is "spot" (RMS spot) or "wavefront" (RMS wavefront),
    /// over all fields (field weights) and wavelengths. None for only the terms. jac is "auto" (analytic for terms with 16 or more analytic columns, central differences for the others),
    /// "fd" (all central differences) or "forward" (all forward differences, half the traces of central differences). qr solves each step by QR instead of the normal equations, gtol stops when the largest cosine between the residuals and a variable's column is below it (0: no test), active_set fixes variables a step would take out of range at the bound.
    /// stopped_as_vignetted drops stopped rays (total internal reflection, missing a surface) like vignetted rays instead of failing the evaluation.
    ///
    /// terms is a list of dicts of terms to add, of the form {"type": "ray", "field": 1, "py": 1, "q": "y", "target": 0, "weight": 1};
    /// the types are spot, wavefront, mtf, paraxial (q: efl, pmag, enpp, epd, expp, expd, wfno, pwfn, isfn, pimh, totr,
    /// obsn, isna), ray (q: x, y, z, l, m, n in local coordinates of surface surf), centroid (centroid of the beam on the image surface, q: x, y), opd, zernike, coupling, zplm (OPTRETURN of a ZPL macro) and
    /// plugin (output of a registered plugin; the same as the CLI's --term).
    /// {"type": "user", "fn": f, "jac": g, "name": ..., "weight": ...} is a term defined by Python functions: f(systems) takes
    /// a list of System and returns the residuals per system as a (len(systems), number of residuals) array (the systems for differences are passed together in one call).
    /// With g(system, names), the partial derivatives of the residuals (number of residuals, len(names)) are computed with it.
    ///
    /// Returned dict: system (optimized system), x0, x (variable values), merit0, merit, history (merit per iteration),
    /// iterations, evals, stop ("converged", "small_step", "small_gradient", "no_progress", "max_iter", "jacobian_failed"), note (details of why it stopped),
    /// inactive (numbers of variables considered ineffective and not moved), term_values0, term_values (values per term of terms), term_value_kinds ("quantity": the value itself for terms with a target, "rms": √Σr² for the others)
    #[pyo3(signature = (vars, merit=Some("spot"), reference="centroid", rings=3, iters=50, jac="auto", threads=None, terms=None, qr=false, gtol=0.0, active_set=false, stopped_as_vignetted=false))]
    #[allow(clippy::too_many_arguments)]
    fn optimize<'py>(
        &self,
        py: Python<'py>,
        vars: Vec<VarArg>,
        merit: Option<&str>,
        reference: &str,
        rings: usize,
        iters: usize,
        jac: &str,
        threads: Option<usize>,
        terms: Option<Bound<'py, PyAny>>,
        qr: bool,
        gtol: f64,
        active_set: bool,
        stopped_as_vignetted: bool,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let (mut p, users, n_default) = opt_problem(
            py,
            &s,
            &vars,
            merit,
            reference,
            rings,
            jac,
            terms.as_ref(),
            &self.coat_files,
        )?;
        p.stopped_as_vignetted = stopped_as_vignetted;
        let o = lensbench_design::dls_spec::DlsChoice {
            qr,
            gtol,
            active_set,
        }
        .opts(iters);
        let r = interruptible(py, || with_threads(threads, || optim::dls(&p, &o)))?;
        userterm::raise_user_errors(&users)?;
        let r = r.map_err(err)?;
        let pair = |t: &optim::TermValue| (t.value(), t.kind_name());
        let tv = |x: &[f64]| -> PyResult<(Vec<f64>, Vec<&str>)> {
            let v = py
                .detach(|| with_threads(threads, || p.term_values(x)))
                .map_err(err);
            userterm::raise_user_errors(&users)?;
            Ok(v?[n_default..].iter().map(pair).unzip())
        };
        let ((tv0, _), (tv1, kinds)) = (tv(&r.x0)?, tv(&r.x)?);
        let d = PyDict::new(py);
        let names: Vec<String> = p.vars.iter().map(|v| s.param_name(v.param)).collect();
        let sys = r.systems.into_iter().next().unwrap();
        d.set_item(
            "system",
            PySystem::new(sys, self.coat_files.clone(), self.coat_cell()),
        )?;
        d.set_item("names", names)?;
        d.set_item("x0", r.x0)?;
        d.set_item("x", r.x)?;
        d.set_item("merit0", r.merit0)?;
        d.set_item("merit", r.merit)?;
        d.set_item("history", r.history)?;
        d.set_item("iterations", r.iterations)?;
        d.set_item("evals", r.evals)?;
        d.set_item("stop", r.stop.as_str())?;
        d.set_item("note", r.note)?;
        d.set_item("inactive", r.inactive)?;
        d.set_item("term_values0", tv0)?;
        d.set_item("term_values", tv1)?;
        d.set_item("term_value_kinds", kinds)?;
        Ok(d)
    }

    /// Monte Carlo tolerance simulation. spec is a dict of the same settings as the CLI's `lensbench tolerance --spec`
    /// (trials, seed, dist, preset, catalog, catalog_tols, lenses, groups, tols, compensators, comp_iters, merit, rings,
    /// ref, terms, sensitivity, percentiles, samples). None for the default (the general tolerance set on all lenses, RMS wavefront, 100 trials).
    /// terms is a list as in optimize, added after the terms of spec (Python function terms {"type": "user", ...} also work,
    /// for using measured values and the like as the criterion). vendors maps block ids to vendors (used for the default tolerances of catalog lenses).
    /// configs are additional configurations (this system is configuration 1; perturbations are the same in all configurations). reversed is the list of ids of blocks of lenses
    /// placed reversed (used to decide front and back of wedge_face in spec by the lens's original orientation; systems made with from_project carry
    /// the reversed mark, so it isn't needed).
    ///
    /// Returned dict: nominal (merit, terms, comp), merit (statistics: n, mean, std, min, max, percentiles), cdf (merit
    /// in ascending order), terms (label, nominal, stats, cdf per term), compensators (name, nominal, stats, values per trial),
    /// tilt_magnitudes (magnitude of the tx, ty pair of compensators [arcmin]), merit_values (merit per trial), sensitivity (change of the merit at lo and hi of each tolerance,
    /// worst first), rss_estimate, correlation (correlation r with the perturbation and r_abs with |perturbation|), tolerances, trials, failed,
    /// errors (with samples true, also deltas and eval per trial)
    #[pyo3(signature = (spec=None, terms=None, vendors=None, configs=None, threads=None, reversed=None))]
    #[allow(clippy::too_many_arguments)]
    fn tolerance<'py>(
        &self,
        py: Python<'py>,
        spec: Option<Bound<'py, PyAny>>,
        terms: Option<Bound<'py, PyAny>>,
        vendors: Option<std::collections::HashMap<String, String>>,
        configs: Option<Vec<PyRef<'py, PySystem>>>,
        threads: Option<usize>,
        reversed: Option<Vec<String>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let v: serde_json::Value = match &spec {
            None => serde_json::Value::Null,
            Some(d) => pyjson::from_py(d)?,
        };
        let mut systems = vec![(*self.sys()).clone()];
        for c in configs.iter().flatten() {
            systems.push((*c.sys()).clone());
        }
        let vendors = vendors.unwrap_or_default();
        let reversed: std::collections::HashSet<String> =
            reversed.unwrap_or_default().into_iter().collect();
        let mut ms =
            lensbench_design::tolerance::spec_from_json_with(&v, systems, &vendors, &reversed)
                .map_err(err)?;
        let oo = lensbench_design::tolerance::out_opts(&v).map_err(err)?;
        let users = match &terms {
            Some(t) => userterm::add_terms(py, &mut ms.problem, t, &self.coat_files)?,
            None => Vec::new(),
        };
        let th = threads.or(oo.threads);
        let r = interruptible(py, || {
            with_threads(th, || lensbench_design::montecarlo::run(&ms))
        })?;
        userterm::raise_user_errors(&users)?;
        let r = r.map_err(err)?;
        let out = lensbench_design::tolerance::result_json(&ms, &r, &oo);
        pyjson::to_py(py, &out)
    }

    /// Return the residuals r of the optimization merit and the Jacobian J ([residual][variable]) at the current variable values (for running the iterations yourself.
    /// Arguments as in optimize; terms work too). The merit is the square root of Σ r² / Σ weights. Residuals are in term order
    /// (terms of the default merit, then terms in order), each multiplied by the square root of its weight
    #[pyo3(signature = (vars, merit=Some("spot"), reference="centroid", rings=3, jac="auto", threads=None, terms=None))]
    #[allow(clippy::too_many_arguments)]
    fn merit_jacobian<'py>(
        &self,
        py: Python<'py>,
        vars: Vec<VarArg>,
        merit: Option<&str>,
        reference: &str,
        rings: usize,
        jac: &str,
        threads: Option<usize>,
        terms: Option<Bound<'py, PyAny>>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>)> {
        let s = self.sys();
        let (p, users, _) = opt_problem(
            py,
            &s,
            &vars,
            merit,
            reference,
            rings,
            jac,
            terms.as_ref(),
            &self.coat_files,
        )?;
        let x = p.x0().map_err(err)?;
        let rj = py.detach(|| with_threads(threads, || p.jacobian(&x)));
        userterm::raise_user_errors(&users)?;
        let (r, j) = rj.map_err(err)?;
        let (m, n) = (r.len(), x.len());
        Ok((r.into_pyarray(py).into_any(), a2(py, j, (m, n))))
    }

    /// RMS: spot radius (spot, spot_x, spot_y [lens units]), wavefront error wfe [waves], and the Strehl ratio approximation
    /// strehl = exp(−(2π wfe)²). The abscissa vs is "field" (x is the normalized field along axis), "focus" (x is the image surface shift),
    /// "wavelength" (x is the wavelength [µm], monochromatic) or None (one point at field). reference is "centroid" or "chief",
    /// rings the number of rings of the Gaussian quadrature of the pupil. field is used when vs is focus, wavelength or None. With field_wave=True,
    /// the fields of real image height (and fields given by conditions) of monochromatic field and wavelength sweeps are solved with the chief ray at the analysis wavelength (as in OpticStudio)
    #[pyo3(signature = (field=None, vs=None, x=None, wave=None, axis="y", reference="centroid", rings=6, threads=None, field_wave=false))]
    #[allow(clippy::too_many_arguments)]
    fn rms<'py>(
        &self,
        py: Python<'py>,
        field: Option<FieldArg>,
        vs: Option<&str>,
        x: Option<PyArrayLike1<'_, f64, AllowTypeChange>>,
        wave: Option<usize>,
        axis: &str,
        reference: &str,
        rings: usize,
        threads: Option<usize>,
        field_wave: bool,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let o = sweep::RmsOpts {
            rings,
            field_wave,
            reference: match reference {
                "centroid" => sweep::RmsRef::Centroid,
                "chief" => sweep::RmsRef::Chief,
                _ => {
                    return Err(value_err(format!(
                        "reference must be \"centroid\" or \"chief\" ({reference})"
                    )))
                }
            },
        };
        let w = Waves::all_or(wave.map(|w| wave_index(&s, w)).transpose()?);
        let xs = x.map(|a| floats(&a));
        let need_x = || {
            xs.clone()
                .ok_or_else(|| value_err("vs needs x (the axis values)"))
        };
        let fsel = |f: Option<FieldArg>| -> PyResult<FieldSel> {
            match f {
                Some(f) => field_sel(&s, f),
                None => Err(value_err("field is needed")),
            }
        };
        let r = match vs {
            None => {
                let f = fsel(field)?;
                let v = py
                    .detach(|| with_threads(threads, || sweep::rms(&s, f, w, o)))
                    .map_err(err)?;
                let d = PyDict::new(py);
                rms_items(&d, &[v], |v| Ok(v[0].into_pyobject(py)?.into_any()))?;
                return Ok(d);
            }
            Some("field") => {
                if field.is_some() {
                    return Err(value_err("field is not used with vs=\"field\""));
                }
                let ax = field_axis(axis)?;
                let h = need_x()?;
                py.detach(|| with_threads(threads, || sweep::rms_vs_field(&s, ax, &h, w, o)))
            }
            Some("focus") => {
                let f = fsel(field)?;
                let z = need_x()?;
                py.detach(|| with_threads(threads, || sweep::rms_vs_focus(&s, f, &z, w, o)))
            }
            Some("wavelength") => {
                if wave.is_some() {
                    return Err(value_err("wave is not used with vs=\"wavelength\""));
                }
                let f = fsel(field)?;
                let l = need_x()?;
                py.detach(|| with_threads(threads, || sweep::rms_vs_wavelength(&s, f, &l, o)))
            }
            Some(v) => {
                return Err(value_err(format!(
                    "vs must be \"field\", \"focus\" or \"wavelength\" ({v})"
                )))
            }
        };
        let d = PyDict::new(py);
        d.set_item("x", r.x.clone().into_pyarray(py))?;
        rms_items(&d, &r.rms, |v| Ok(v.into_pyarray(py).into_any()))?;
        d.set_item("warnings", r.warnings)?;
        Ok(d)
    }

    /// Run an analysis of the registry by name, with the same settings and results as the CLI subcommand of that name and serve's
    /// analyze (lensbench.analyses() lists the names and settings). The settings are keyword arguments with the keys of the settings
    /// (field and wave are numbers from 1, e.g. max_freq=100); omitted ones take the defaults of the analysis definition
    /// (lensbench.defaults(name)). Analyses that read other configurations or the system file (interference, etc.) can't run here.
    /// threads is the number of threads. Returns the result as a dict (lists, not NumPy arrays)
    #[pyo3(signature = (name, threads=None, **settings))]
    fn analyze<'py>(
        &self,
        py: Python<'py>,
        name: &str,
        threads: Option<usize>,
        settings: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reg = lensbench_engine::analyses();
        let a = reg.get(name).cloned().ok_or_else(|| {
            value_err(format!(
                "no analysis {name} (lensbench.analyses() lists the names)"
            ))
        })?;
        let p = match settings {
            Some(d) => pyjson::from_py(d.as_any())?,
            None => serde_json::Value::Null,
        };
        let s = self.sys();
        let v = py
            .detach(|| with_threads(threads, || a.run_json(&lensbench_analysis::Cx::new(&s), &p)))
            .map_err(err)?;
        pyjson::to_py(py, &v)
    }

    /// Diffraction PSF: DFT of the pupil function (the same as MTF). The intensity i is a (y, x) array, normalized so that the center of the polychromatic PSF
    /// of the same pupil without aberrations is 1 (peak is a guide to the Strehl ratio). x and y are coordinates from the image point center of the chief ray at the primary
    /// wavelength [lens units] on the plane given by plane: "image" (default) the image surface (local x, y; the same frequencies as mtf), "chief" the plane perpendicular to the
    /// chief ray (as OpticStudio's FFT PSF; finer by 1/cos in oblique fields). axes are the grid directions (unit vectors in image space).
    /// pupil is the side of the pupil grid (automatic if omitted: phase difference between neighboring samples at most π/2), image the side of the image grid, pixel the spacing
    /// of the image grid (1 / (2 × cutoff frequency) on that plane if omitted). Also returns plane, pupil (the value used), phase_step (largest phase difference between neighboring pupil samples [rad]),
    /// enclosed (fraction of the light inside the image grid) and warnings (notes such as undersampling)
    #[pyo3(signature = (field, wave=None, pupil=None, image=128, pixel=None, plane="image", threads=None))]
    #[allow(clippy::too_many_arguments)]
    fn psf<'py>(
        &self,
        py: Python<'py>,
        field: FieldArg,
        wave: Option<usize>,
        pupil: Option<usize>,
        image: usize,
        pixel: Option<f64>,
        plane: &str,
        threads: Option<usize>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let f = field_sel(&s, field)?;
        let w = Waves::all_or(wave.map(|w| wave_index(&s, w)).transpose()?);
        let o = psf::PsfOpts {
            pupil,
            image,
            pixel,
            plane: interference::psf_plane(plane)?,
        };
        let p = py
            .detach(|| with_threads(threads, || psf::psf(&s, f, w, o)))
            .map_err(err)?;
        interference::psf_dict(py, p)
    }

    /// Geometric MTF: from the distribution of ray positions. n is a guide for the number of pupil quadrature points; diff_limit applies the diffraction-limited MTF.
    /// The returned dict has the same form as mtf. axes is "field" (default; tangential along the field direction) or "pupil" (tangential along the pupil y, as in OpticStudio's FFT MTF); they differ only for fields with an x component.
    #[pyo3(signature = (field, freqs, wave=None, n=128, diff_limit=false, threads=None, axes="field"))]
    #[allow(clippy::too_many_arguments)]
    fn geo_mtf<'py>(
        &self,
        py: Python<'py>,
        field: FieldArg,
        freqs: PyArrayLike1<'_, f64, AllowTypeChange>,
        wave: Option<usize>,
        n: usize,
        diff_limit: bool,
        threads: Option<usize>,
        axes: &str,
    ) -> PyResult<Bound<'py, PyDict>> {
        let (s, ax) = (self.sys(), mtfs::axes(axes)?);
        let f = field_sel(&s, field)?;
        let w = Waves::all_or(wave.map(|w| wave_index(&s, w)).transpose()?);
        let fs = floats(&freqs);
        let m = py
            .detach(|| {
                with_threads(threads, || {
                    psf::geometric_mtf_axes(&s, f, w, n, &fs, diff_limit, ax)
                })
            })
            .map_err(err)?;
        mtf_dict(py, &m)
    }

    /// Huygens PSF: sums the secondary waves from each point of the reference sphere of the exit pupil on the image surface (for tilted image surfaces and strongly aberrated systems).
    /// The returned dict has the same form as psf; x and y are local image surface coordinates relative to center (axes are x̂, ŷ)
    #[pyo3(signature = (field, wave=None, pupil=None, image=128, pixel=None, threads=None))]
    #[allow(clippy::too_many_arguments)]
    fn huygens_psf<'py>(
        &self,
        py: Python<'py>,
        field: FieldArg,
        wave: Option<usize>,
        pupil: Option<usize>,
        image: usize,
        pixel: Option<f64>,
        threads: Option<usize>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let f = field_sel(&s, field)?;
        let w = Waves::all_or(wave.map(|w| wave_index(&s, w)).transpose()?);
        let o = huygens::HuygensOpts {
            pupil,
            image,
            pixel,
        };
        let p = py
            .detach(|| with_threads(threads, || huygens::huygens_psf(&s, f, w, o)))
            .map_err(err)?;
        interference::psf_dict(py, p)
    }

    /// Huygens MTF: Fourier transform of the Huygens PSF (frequencies on the image surface). The returned dict has the same form as mtf.
    /// axes is "field" (default; tangential along the field direction) or "pupil" (tangential along the pupil y, as in OpticStudio's FFT MTF); they differ only for fields with an x component.
    #[pyo3(signature = (field, freqs, wave=None, pupil=None, image=128, pixel=None, threads=None, axes="field"))]
    #[allow(clippy::too_many_arguments)]
    fn huygens_mtf<'py>(
        &self,
        py: Python<'py>,
        field: FieldArg,
        freqs: PyArrayLike1<'_, f64, AllowTypeChange>,
        wave: Option<usize>,
        pupil: Option<usize>,
        image: usize,
        pixel: Option<f64>,
        threads: Option<usize>,
        axes: &str,
    ) -> PyResult<Bound<'py, PyDict>> {
        let (s, ax) = (self.sys(), mtfs::axes(axes)?);
        let f = field_sel(&s, field)?;
        let w = Waves::all_or(wave.map(|w| wave_index(&s, w)).transpose()?);
        let fs = floats(&freqs);
        let o = huygens::HuygensOpts {
            pupil,
            image,
            pixel,
        };
        let m = py
            .detach(|| {
                with_threads(threads, || {
                    huygens::huygens_mtf_axes(&s, f, w, o, &fs, None, ax)
                })
            })
            .map_err(err)?;
        mtf_dict(py, &m)
    }

    /// pickle: turns the system into the JSON bytes of a system file and passes them with the file names of the coating tables and models
    /// and the folders of the glass catalogs. The receiving side reads the coating file, tables and catalogs again from the same paths
    /// (for another process on the same machine). With parameter changes, glasses, aiming and pickups, the round-tripped system traces bit
    /// for bit the same as the original
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Reduced<'py>> {
        let s = self.sys();
        let b = lensbench_project::sysfile::to_string(&s, &self.coat_files).map_err(err)?;
        let dirs: Vec<PathBuf> = lensbench_project::sysfile::catalog_files(&s)
            .iter()
            .filter_map(|p| p.parent().map(|d| std::path::Path::new(".").join(d)))
            .collect();
        let f = py.import("lensbench")?.getattr("_system_from_state")?;
        let state = (
            PyBytes::new(py, b.as_bytes()),
            self.coat_files.clone(),
            dirs,
        );
        Ok((f, state))
    }

    fn __repr__(&self) -> String {
        let s = self.sys();
        format!(
            "System({:?}, surfaces={}, fields={}, wavelengths={})",
            s.name,
            s.image() + 1,
            s.fields.len(),
            s.wavelengths.len()
        )
    }
}

/// Trace many systems together in one call. Returns a list of dicts, one per system,
/// each bit for bit the same as `systems[i].trace(field, px, py, ...)` (for any number of threads).
///
/// The arguments are as in System.trace, the same for all systems (grad parameter names are resolved per system).
/// Parallel over both systems and rays, so all cores are used even with few rays per system.
/// threads is the total number of threads (1 for sequential)
#[pyfunction]
#[pyo3(signature = (systems, field, px, py, waves=None, last=None, record=RecArg::Flag(false), pol=None, pol_phase=false, globals=false, object=None, grad=None, threads=None, pupil="normalized", dz=None, land=false))]
#[allow(clippy::too_many_arguments)]
fn trace_many<'py>(
    gil: Python<'py>,
    systems: Vec<Py<PySystem>>,
    field: Option<FieldArg>,
    px: PyArrayLike1<'py, f64, AllowTypeChange>,
    py: PyArrayLike1<'py, f64, AllowTypeChange>,
    waves: Option<Vec<usize>>,
    last: Option<usize>,
    record: RecArg,
    pol: Option<PolArg>,
    pol_phase: bool,
    globals: bool,
    object: Option<ObjArg>,
    grad: Option<Vec<String>>,
    threads: Option<usize>,
    pupil: &str,
    dz: Option<f64>,
    land: bool,
) -> PyResult<Vec<Bound<'py, PyDict>>> {
    let (px, pyv) = (floats(&px), floats(&py));
    let n = px.len();
    // 系ごとの準備（引数の検査・膜の読み込みは GIL のあるうちに）
    let mut jobs = Vec::with_capacity(systems.len());
    for p in &systems {
        let me = p.get();
        let s = me.sys();
        let pre = Prep::new(&s, field.clone(), object.clone(), pupil, n, pyv.len())?;
        let o = TraceOpts::new(
            &s,
            waves.clone(),
            last,
            record.clone(),
            pol.clone(),
            pol_phase,
            globals,
            grad.clone(),
            threads,
            dz,
            land,
        )?;
        let coat = match o.pw {
            Some(_) => Some(me.coatings(&s)?),
            None => None,
        };
        jobs.push((s, pre, o, coat));
    }
    let outs = gil.detach(|| {
        with_threads(threads, || {
            lensbench_core::par::map(&jobs, |(s, pre, o, coat)| {
                compute(s, coat.as_deref(), &pre.launch(&px, &pyv), n, o)
            })
        })
    });
    let mut res = Vec::with_capacity(jobs.len());
    for ((s, pre, o, _), out) in jobs.iter().zip(outs) {
        let d = to_dict(gil, s, o, out.map_err(err)?)?;
        pre.finish(&d)?;
        res.push(d);
    }
    Ok(res)
}

/// The value returned by `__reduce__`: (function to rebuild, (system bytes, coating files, glass catalog folders))
type Reduced<'py> = (
    Bound<'py, PyAny>,
    (Bound<'py, PyBytes>, Vec<PathBuf>, Vec<PathBuf>),
);

/// Rebuild a system from a pickle (the values of [`PySystem::__reduce__`]). glass_dirs is missing in pickles of older versions
#[pyfunction]
#[pyo3(signature = (state, coatings, glass_dirs=None))]
fn _system_from_state(
    state: &[u8],
    coatings: Vec<PathBuf>,
    glass_dirs: Option<Vec<PathBuf>>,
) -> PyResult<PySystem> {
    let txt =
        std::str::from_utf8(state).map_err(|e| value_err(format!("reloading the system: {e}")))?;
    // 長さと系の約束は sysfile::check で確かめる（壊れたバイト列で追跡が panic しないよう）
    let (sys, _, _) =
        lensbench_project::sysfile::from_str_with(txt, &glass_dirs.unwrap_or_default())
            .map_err(|e| value_err(format!("reloading the system: {e}")))?;
    Ok(PySystem::new(sys, coatings, Default::default()))
}

/// Python API version (major, minor). major goes up with changes that break compatibility, minor with additions
/// ("Versions and compatibility" in docs/python.md). Write changes in the change history of docs/python.md
pub const API_VERSION: (u32, u32) = (2, 1);

// 大きな配列を追跡ごとに確保し直しても OS からページを取り直さないように。別の拡張モジュールに
// 組み込むとき（default-features = false）は、その側で global_allocator を決める
#[cfg(feature = "module")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// lensbench: sequential ray tracing (Python bindings)
#[cfg(feature = "module")]
#[pymodule]
fn lensbench(m: &Bound<'_, PyModule>) -> PyResult<()> {
    init_module(m)
}

/// Add the functions and classes to the module. To build it into another extension module, call this from its `#[pymodule]` and
/// add to the defaults of the extension table (`lensbench_core::ext::update_default`, `lensbench_engine::Extensions::install`),
/// before or after: the bundled plugins are added only under names not in the table yet, so the extension's plugin wins either way
pub fn init_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // threads= を省いた計算の既定（lensbench run --threads が渡す）
    lensbench_core::init_from_env();
    errors::register(m)?;
    m.add_class::<PySystem>()?;
    m.add_function(wrap_pyfunction!(trace_many, m)?)?;
    m.add_function(wrap_pyfunction!(_system_from_state, m)?)?;
    m.add_function(wrap_pyfunction!(run_settings, m)?)?;
    m.add_function(wrap_pyfunction!(progress, m)?)?;
    interference::register(m)?;
    plugin::register(m)?;
    runs::register(m)?;
    search::register(m)?;
    analyses::register(m)?;
    m.add("__version__", lensbench_core::VERSION)?;
    m.add("API_VERSION", API_VERSION)?;
    Ok(())
}
