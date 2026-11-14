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

mod edit;
mod errors;
pub mod ext;
mod fringe;
mod plugin;
mod runs;
mod search;
mod userterm;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use lensbench_core::catalog::LensCatalog;
use lensbench_core::coating::{Coatings, C64};
use lensbench_core::param::Param;
use lensbench_core::polgrid::{self, PolGrid};
use lensbench_core::solve::{RayProblem, Residual, Unknown};
use lensbench_core::system::{
    Aiming, Field, FieldCond, FieldSel, FieldType, Launch, ObjSpec, PolTrace, Trace,
};
use lensbench_core::{analysis, huygens, optim, psf, sweep, with_threads, zmx, RecSel, System};
use numpy::ndarray::Array;
use numpy::{AllowTypeChange, IntoPyArray, PyArrayLike1, PyArrayLike2};
use pyo3::exceptions::{PyIndexError, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyTuple};

/// coatings= を省いたときの膜の実測表・モデル: 環境変数 LENSBENCH_COATINGS（OS のパス区切り、`lensbench run --coatings`）
fn env_coatings() -> Vec<PathBuf> {
    std::env::var_os("LENSBENCH_COATINGS")
        .map(|v| {
            std::env::split_paths(&v)
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// `lensbench run`が渡す設定。どれも環境変数から読み、無ければ空か None
///
/// - glass_dirs: ガラスカタログを探すフォルダ（LENSBENCH_GLASSCAT。System.load などは自分でも読む）
/// - coatings: 膜の実測表・モデル（LENSBENCH_COATINGS。System.load の coatings= の既定）
/// - threads: スレッド数（LENSBENCH_THREADS。threads= を省いた計算の既定）
/// - out: 出力先のフォルダ（LENSBENCH_OUT）。スクリプトが自分で使う
/// - log: ログのファイル（LENSBENCH_LOG）
/// - args: スクリプトに渡した引数（sys.argv[1:] と同じ）
/// - under_run: `lensbench run` から起動されたか
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

/// 進み具合を 1 行で標準エラーに出す: `lensbench:progress 3/10 メッセージ`。
/// `lensbench run` はこの行を拾ってログに残し、将来の GUI は進捗の表示に使う
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

/// 長い計算を別のスレッドで回し、呼んだスレッド（Python の主スレッド）で Ctrl-C（シグナル）を見張る。
/// シグナルが来たら stop を立てて計算の終わりを待ち、KeyboardInterrupt などの例外を返す。
/// 計算は stop を反復・試行ごとに見て止まる（optim::dls_with・tolerance::run_with の進み具合の口）
pub fn interruptible<R: Send>(
    py: Python<'_>,
    f: impl FnOnce(&AtomicBool) -> R + Send,
) -> PyResult<R> {
    let stop = AtomicBool::new(false);
    std::thread::scope(|sc| {
        let (tx, rx) = std::sync::mpsc::channel();
        let stop = &stop;
        sc.spawn(move || {
            // 受け手が先に去っていても（例外で返った後）送りの失敗は気にしない
            let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || f(stop),
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
                Ok(r) => {
                    return r.map_err(|_| PyRuntimeError::new_err("計算のスレッドが panic した"))
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(PyRuntimeError::new_err("計算のスレッドが終わった"))
                }
            }
            if let Err(e) = py.check_signals() {
                stop.store(true, Ordering::Relaxed);
                // 計算が stop を見て止まるのを待つ（scope の終わりで join する）
                let r = rx.take().unwrap();
                let _ = py.detach(move || r.recv());
                return Err(e);
            }
        }
    })
}

pub use errors::{err, ErrSrc};

/// 光学系（.zmx またはプロジェクトファイル .json から読む）
#[pyclass(module = "lensbench", name = "System", frozen)]
pub struct PySystem {
    /// 系。ロックは Arc の複製・差し替えの間だけ持つ（追跡は複製した Arc で、GIL を外して行う）。
    /// 変更は複製した系に施して差し替えるので、走っている追跡は前の系のまま
    sys: RwLock<Arc<System>>,
    coat_files: Vec<PathBuf>,
    /// 系の膜（初めて要ったときに読む）。膜・媒質・膜ファイルの変わる編集で空に戻す
    coat: RwLock<CoatCell>,
}

/// 系の膜を読んだもの（複製した系・派生した系で共有する）
pub type CoatCell = Arc<OnceLock<Result<Coatings, String>>>;

/// 読んだ膜への参照（[`PySystem::coatings`]）
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

/// 膜の読み込みに関わる系の部分が変わったか（膜・媒質の境界、膜ファイル、系の膜の表）
fn coat_changed(a: &System, b: &System) -> bool {
    a.coating_file != b.coating_file
        || a.coat_tables != b.coat_tables
        || lensbench_core::coating::media_keys(a).0 != lensbench_core::coating::media_keys(b).0
}

/// 視野の指定: 番号（1 始まり）か正規化視野 (hx, hy)。bool は弾く
#[derive(FromPyObject, Clone)]
pub enum FieldArg {
    Flag(bool),
    Index(usize),
    Norm((f64, f64)),
}

/// 物体の指定: 有限物体の物点 (x, y) [mm]（物体面の局所座標）か、無限遠物体の方向 (l, m, n)
#[derive(FromPyObject, Clone)]
pub enum ObjArg {
    Point((f64, f64)),
    Dir((f64, f64, f64)),
}

/// pol の指定: True（主波長）か波長の番号
#[derive(FromPyObject, Clone)]
enum PolArg {
    Flag(bool),
    Wave(usize),
}

/// pol_grid の指定: 両軸の点数 n か (nx, ny)
#[derive(FromPyObject, Clone)]
enum GridArg {
    N(usize),
    Nxy((usize, usize)),
}

/// record の指定: True（全面）か面の番号（1 始まり）の列
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

/// 光線の出どころ
#[derive(Clone, Copy)]
enum Source {
    Field(FieldSel),
    Object(ObjSpec),
}

/// trace の光線の始め方（視野か物体と、瞳の座標の種類）
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
            _ => {
                return Err(PyValueError::new_err(
                    "field か object のどちらか 1 つを与える",
                ))
            }
        };
        let (at_stop, signed) = match pupil.to_ascii_lowercase().as_str() {
            "normalized" => (false, false),
            "signed" => (false, true),
            "stop" => (true, false),
            p => {
                return Err(PyValueError::new_err(format!(
                    "pupil は \"normalized\"・\"signed\"・\"stop\" のどれか（{p}）"
                )))
            }
        };
        if npx != npy {
            return Err(PyValueError::new_err(format!(
                "px と py の長さが違う（{npx} と {npy}）"
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

    /// 結果の dict に視野（hx・hy）か物体を足す
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
                return Err(PyValueError::new_err(
                    "無限遠物体では object は方向 (l, m, n)",
                ));
            }
            Ok(ObjSpec::Point([x, y]))
        }
        ObjArg::Dir((l, m, n)) => {
            if !s.infinite() {
                return Err(PyValueError::new_err("有限物体では object は物点 (x, y)"));
            }
            let r = (l * l + m * m + n * n).sqrt();
            if r.is_nan() || r <= 0.0 || n.is_nan() || n <= 0.0 {
                return Err(PyValueError::new_err("方向 (l, m, n) は n > 0"));
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
        FieldArg::Flag(b) => Err(PyTypeError::new_err(format!(
            "視野は番号（1 始まり）か (hx, hy)。bool（{b}）は使えない"
        ))),
        FieldArg::Index(i) => {
            let n = s.fields.len();
            if i == 0 || i > n {
                return Err(PyIndexError::new_err(format!(
                    "視野 {i} はありません（1〜{n}）"
                )));
            }
            Ok(FieldSel::Index(i - 1))
        }
        FieldArg::Norm((x, y)) => Ok(FieldSel::Norm(x, y)),
    }
}

fn field_axis(a: &str) -> PyResult<sweep::FieldAxis> {
    match a {
        "x" => Ok(sweep::FieldAxis::X),
        "y" => Ok(sweep::FieldAxis::Y),
        _ => Err(PyValueError::new_err(format!(
            "axis は \"x\" か \"y\"（{a}）"
        ))),
    }
}

fn mtf_dict<'py>(py: Python<'py>, m: &analysis::Mtf) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("freqs", m.freqs.clone().into_pyarray(py))?;
    d.set_item("tangential", m.tan.clone().into_pyarray(py))?;
    d.set_item("sagittal", m.sag.clone().into_pyarray(py))?;
    d.set_item("cutoff", m.cutoff)?;
    Ok(d)
}

fn sweep_dict<'py>(
    py: Python<'py>,
    xname: &str,
    m: &sweep::MtfSweep,
) -> PyResult<Bound<'py, PyDict>> {
    let nf = m.freqs.len();
    let nx = m.x.len();
    let t = Array::from_shape_fn((nf, nx), |(k, j)| m.tan[k][j]);
    let g = Array::from_shape_fn((nf, nx), |(k, j)| m.sag[k][j]);
    let d = PyDict::new(py);
    d.set_item("freqs", m.freqs.clone().into_pyarray(py))?;
    d.set_item(xname, m.x.clone().into_pyarray(py))?;
    d.set_item("tangential", t.into_pyarray(py))?;
    d.set_item("sagittal", g.into_pyarray(py))?;
    d.set_item("warnings", m.warnings.clone())?;
    Ok(d)
}

/// RMS の各量を dict に（値の列 → Python の値は put で作る）
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
    let n = s.wavelengths.len();
    if w == 0 || w > n {
        return Err(PyIndexError::new_err(format!(
            "波長 {w} はありません（1〜{n}）"
        )));
    }
    Ok(w - 1)
}

/// 最適化の変数: "7.t" か ("7.t", 下限, 上限)
#[derive(FromPyObject, Clone)]
enum VarArg {
    Name(String),
    Range((String, Option<f64>, Option<f64>)),
}

#[allow(clippy::too_many_arguments)]
fn opt_problem(
    py: Python<'_>,
    s: &System,
    vars: &[VarArg],
    merit: Option<&str>,
    reference: &str,
    rings: usize,
    jac: &str,
    terms: Option<&Bound<'_, PyAny>>,
    coat_files: &[PathBuf],
) -> PyResult<(optim::Problem, Vec<Arc<userterm::PyUserTerm>>, usize)> {
    let mut p = optim::Problem::single(s.clone());
    for v in vars {
        let (name, lo, hi) = match v {
            VarArg::Name(n) => (n.clone(), None, None),
            VarArg::Range((n, lo, hi)) => (n.clone(), *lo, *hi),
        };
        let param = s.param(&name).map_err(err)?;
        p.vars.push(
            optim::Var::new(param)
                .bounds(lo.unwrap_or(f64::NEG_INFINITY), hi.unwrap_or(f64::INFINITY)),
        );
    }
    let kind = match merit {
        Some("spot") => Some(optim::DefaultMerit::Spot),
        Some("wavefront") => Some(optim::DefaultMerit::Wavefront),
        None => None,
        Some(m) => {
            return Err(PyValueError::new_err(format!(
                "merit は \"spot\"・\"wavefront\"・None（{m}）"
            )))
        }
    };
    let reference = match reference {
        "centroid" => sweep::RmsRef::Centroid,
        "chief" => sweep::RmsRef::Chief,
        _ => {
            return Err(PyValueError::new_err(format!(
                "reference は \"centroid\" か \"chief\"（{reference}）"
            )))
        }
    };
    p.jac = optim::JacMode::parse(jac).ok_or_else(|| {
        PyValueError::new_err(format!(
            "jac は \"auto\"・\"fd\"・\"forward\" のどれか（{jac}）"
        ))
    })?;
    if let Some(kind) = kind {
        p.add_default(kind, rings, reference);
    }
    let n_default = p.terms.len();
    let users = match terms {
        Some(t) => userterm::add_terms(py, &mut p, t, coat_files)?,
        None => Vec::new(),
    };
    p.check().map_err(err)?;
    Ok((p, users, n_default))
}

/// 変更の列: {"3.t": 1.0, ...} か [("3.t", 1.0), ...]
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
    a.as_array().iter().copied().collect()
}

/// 面を番号か名前で
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
    /// 今の系（複製した Arc）
    pub fn sys(&self) -> Arc<System> {
        self.sys.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 系から作る（膜のファイル、膜を読んだもの）
    pub fn new(sys: System, coat_files: Vec<PathBuf>, coat: CoatCell) -> Self {
        PySystem {
            sys: RwLock::new(Arc::new(sys)),
            coat_files,
            coat: RwLock::new(coat),
        }
    }

    /// 読んだ膜（派生した系に引き継ぐ）
    pub fn coat_cell(&self) -> CoatCell {
        self.coat.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 系を複製して f で変え、うまくいったときだけ差し替える（走っている追跡は前の系のまま）。
    /// 膜の読み込みに関わる所が変われば、読んだ膜を捨てて次に要ったときに読み直す
    fn mutate<R>(&self, f: impl FnOnce(&mut System) -> lensbench_core::Result<R>) -> PyResult<R> {
        let mut g = self.sys.write().unwrap_or_else(|e| e.into_inner());
        let mut n: System = (**g).clone();
        let r = f(&mut n).map_err(err)?;
        if coat_changed(&g, &n) {
            *self.coat.write().unwrap_or_else(|e| e.into_inner()) = CoatCell::default();
        }
        *g = Arc::new(n);
        Ok(r)
    }

    /// 系の膜（初めて呼んだときに読む）
    pub fn coatings(&self, s: &System) -> PyResult<CoatRef> {
        let cell = self.coat_cell();
        cell.get_or_init(|| {
            let mut c = Coatings::for_system(s).map_err(|e| e.to_string())?;
            for p in &self.coat_files {
                c.load_file(p).map_err(|e| e.to_string())?;
            }
            Ok(c)
        })
        .as_ref()
        .map_err(err)?;
        Ok(CoatRef(cell))
    }
}

/// 波長ごとの追跡の結果を (波長, 光線) の配列に並べる（列ごとに 1 回ずつ写す）
struct Soa {
    nw: usize,
    n: usize,
    /// (名前, 値, 光線ごとの数)
    f: Vec<(&'static str, Vec<f64>, usize)>,
    err: Vec<i32>,
    vig: Vec<i32>,
    ok: Vec<bool>,
    rec_surf: Vec<i64>,
    rec: Vec<(&'static str, Vec<f64>)>,
}

type RayGet = fn(&Trace, usize) -> f64;

/// 既定で返す列（最後の面の局所座標、光路長）
const COLS: [(&str, RayGet); 9] = [
    ("x", |t, i| t.rays[i].xyz[0]),
    ("y", |t, i| t.rays[i].xyz[1]),
    ("z", |t, i| t.rays[i].xyz[2]),
    ("l", |t, i| t.rays[i].lmn[0]),
    ("m", |t, i| t.rays[i].lmn[1]),
    ("n", |t, i| t.rays[i].lmn[2]),
    ("opl", |t, i| t.rays[i].opl),
    ("opl_os", |t, i| t.opl_os[i]),
    ("lens_opd", |t, i| t.rays[i].lens_opd),
];

/// globals=True で足す列（大域の位置・方向）
const GLOBAL_COLS: [(&str, RayGet); 6] = [
    ("gx", |t, i| t.rays[i].p[0]),
    ("gy", |t, i| t.rays[i].p[1]),
    ("gz", |t, i| t.rays[i].p[2]),
    ("gl", |t, i| t.rays[i].k[0]),
    ("gm", |t, i| t.rays[i].k[1]),
    ("gn", |t, i| t.rays[i].k[2]),
];

type RecGet = fn(&lensbench_core::system::RecSurf, usize) -> f64;

const REC_COLS: [(&str, RecGet); 10] = [
    ("rec_x", |r, i| r.xyz[i][0]),
    ("rec_y", |r, i| r.xyz[i][1]),
    ("rec_z", |r, i| r.xyz[i][2]),
    ("rec_l", |r, i| r.lmn[i][0]),
    ("rec_m", |r, i| r.lmn[i][1]),
    ("rec_n", |r, i| r.lmn[i][2]),
    ("rec_gx", |r, i| r.p[i][0]),
    ("rec_gy", |r, i| r.p[i][1]),
    ("rec_gz", |r, i| r.p[i][2]),
    ("rec_opl", |r, i| r.opl[i]),
];

impl Soa {
    fn new(traces: &[Trace], n: usize, globals: bool, record: bool) -> Soa {
        let nw = traces.len();
        let col = |g: RayGet| {
            let mut v = Vec::with_capacity(nw * n);
            for t in traces {
                v.extend((0..n).map(|i| g(t, i)));
            }
            v
        };
        let mut f: Vec<_> = COLS.iter().map(|&(k, g)| (k, col(g), 1)).collect();
        if globals {
            f.extend(GLOBAL_COLS.iter().map(|&(k, g)| (k, col(g), 1)));
            let mut p0 = Vec::with_capacity(nw * n * 3);
            let mut k0 = Vec::with_capacity(nw * n * 3);
            for t in traces {
                p0.extend(t.p0.iter().flatten());
                k0.extend(t.k0.iter().flatten());
            }
            f.push(("p0", p0, 3));
            f.push(("k0", k0, 3));
        }
        let ints = |g: fn(&Trace, usize) -> i32| {
            let mut v = Vec::with_capacity(nw * n);
            for t in traces {
                v.extend((0..n).map(|i| g(t, i)));
            }
            v
        };
        let err = ints(|t, i| t.rays[i].err as i32);
        let vig = ints(|t, i| t.rays[i].vig as i32);
        let mut ok = Vec::with_capacity(nw * n);
        for t in traces {
            ok.extend((0..n).map(|i| t.ok(i)));
        }
        let rec_surf: Vec<i64> = traces
            .first()
            .filter(|_| record)
            .map(|t| t.rec.iter().map(|r| r.surf as i64).collect())
            .unwrap_or_default();
        let ns = rec_surf.len();
        let rec = if !record {
            Vec::new()
        } else {
            REC_COLS
                .iter()
                .map(|&(k, g)| {
                    let mut v = Vec::with_capacity(nw * ns * n);
                    for t in traces {
                        for r in &t.rec {
                            v.extend((0..r.opl.len()).map(|i| g(r, i)));
                        }
                    }
                    (k, v)
                })
                .collect()
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

/// trace・trace_rays の共通の引数
struct TraceOpts {
    ws: Vec<usize>,
    last: Option<usize>,
    record: bool,
    /// 記録する面（None なら全面）
    rec_surfs: Option<Vec<usize>>,
    pw: Option<usize>,
    pol_phase: bool,
    /// 偏光追跡を粗い瞳格子 (nx, ny) で行って補間する
    pol_grid: Option<(usize, usize)>,
    globals: bool,
    params: Option<Vec<Param>>,
    threads: Option<usize>,
    /// 検出面を最後の面の局所座標の z = dz に置く
    dz: Option<f64>,
    /// grad と一緒に着地点の微分 d(x, y, l, m)/dp を返す
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
                return Err(PyIndexError::new_err(format!(
                    "面 {l} はありません（1〜{img}）"
                )));
            }
        }
        let pw = match pol {
            None | Some(PolArg::Flag(false)) => None,
            Some(PolArg::Flag(true)) => Some(s.primary),
            Some(PolArg::Wave(w)) => Some(wave_index(s, w)?),
        };
        if pw.is_some() && last.is_some_and(|l| l != img) {
            return Err(PyValueError::new_err(
                "pol は last を省いたとき（像面まで）だけ",
            ));
        }
        let params: Option<Vec<Param>> = grad
            .as_ref()
            .map(|g| g.iter().map(|k| s.param(k)).collect())
            .transpose()
            .map_err(err)?;
        if land && params.is_none() {
            return Err(PyValueError::new_err("land は grad と一緒に与える"));
        }
        if let Some(z) = dz {
            if !z.is_finite() {
                return Err(PyValueError::new_err(format!("dz が有限でない（{z}）")));
            }
            if pw.is_some() && pol_phase {
                return Err(PyValueError::new_err(
                    "dz と pol_phase=True は一緒に使えない（ずらした分の伝搬の位相を入れていない）",
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
    /// 光線の始め方 launch（n 本）で追跡し、結果の dict を作る（trace・trace_rays の共通部分）
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

/// 追跡の結果（dict にする前。GIL なしで作る）
type TraceOut = (Soa, Option<PolOut>, Vec<f64>, Vec<f64>);

/// 偏光追跡の結果: Jones (光線, 2, 2)、z 成分 (光線, 2)、格子で補間したときは瞳の内側で欠けた格子点と、
/// そのために補間できなかった光線
type PolOut = (Vec<C64>, Vec<C64>, Option<(Vec<[f64; 2]>, Vec<bool>)>);

/// 光線の始め方 launch（n 本）で追跡する（GIL なし）
fn compute(
    sys: &System,
    coat: Option<&Coatings>,
    launch: &Launch,
    n: usize,
    o: &TraceOpts,
) -> lensbench_core::Result<TraceOut> {
    let rec_trace = o.record || o.params.is_some();
    let mut traces = Vec::with_capacity(o.ws.len());
    let mut g = Vec::new();
    let mut gl = Vec::new();
    for &w in &o.ws {
        // 記録する面を選んだとき: dOPL/dp には全面の記録が要るので、そのときは全面で追って後で選ぶ
        let sel = match (&o.rec_surfs, o.params.is_some()) {
            (Some(v), false) => RecSel::Surfs(v),
            _ => RecSel::from(rec_trace),
        };
        let mut t = sys.trace_launch_rec(launch, w, o.last, sel);
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
            let t = match o.ws.iter().position(|&q| q == w) {
                Some(k) if o.dz.is_none() => &traces[k],
                _ => {
                    own = sys.trace_launch_rec(launch, w, o.last, RecSel::Off);
                    &own
                }
            };
            let kw = 2.0 * std::f64::consts::PI / (sys.wavelengths[w] * 1e-3);
            let nan = C64::new(f64::NAN, f64::NAN);
            let mut j = Vec::with_capacity(n * 4);
            let mut z = Vec::with_capacity(n * 2);
            let mut bad = Vec::with_capacity(n);
            for i in 0..n {
                let v = if t.ok(i) { pg.eval(px[i], py[i]) } else { None };
                bad.push(t.ok(i) && v.is_none());
                match v {
                    Some(mut v) => {
                        if o.pol_phase {
                            let ph = C64::new(0.0, -kw * t.rays[i].opl).exp();
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
            let p: PolTrace = sys.trace_pol_launch(launch, w, &[x, y], c, o.pol_phase)?;
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
    Ok((Soa::new(&traces, n, o.globals, o.record), jones, g, gl))
}

/// 追跡の結果の dict を作る
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
        "wavelengths",
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

/// 展開の説明を Python の警告（UserWarning）にする
fn warn_all(py: Python<'_>, warn: &[String]) -> PyResult<()> {
    if warn.is_empty() {
        return Ok(());
    }
    let w = py.import("warnings")?;
    for m in warn {
        w.call_method1("warn", (m.as_str(),))?;
    }
    Ok(())
}

fn a3<T: numpy::Element>(py: Python<'_>, v: Vec<T>, s: (usize, usize, usize)) -> Bound<'_, PyAny> {
    Array::from_shape_vec(s, v)
        .expect("shape")
        .into_pyarray(py)
        .into_any()
}

/// 展開した系の面ごとの元の面と、反射の面の番号
type GhostOrigin = (Vec<Option<usize>>, (usize, usize));

#[pymethods]
impl PySystem {
    /// 系を読む。path は .zmx かプロジェクトファイル（.json）。
    ///
    /// - glass_dirs: ガラスカタログ（AGF）を探すフォルダ
    /// - cfg: .zmx のマルチコンフィグの番号（1 始まり）
    /// - state: プロジェクトファイルの状態の名前
    /// - split_path: プロジェクトファイルのビームスプリッターの経路（BS を通る順の T・R、例 "TR"）。
    ///   とれる経路は `System.split_paths(path)`
    /// - lens_catalogs: プロジェクトファイルのカタログレンズを探すレンズカタログ（.zmx またはフォルダ）
    /// - coatings: 偏光追跡で使う膜の実測表（LBCT）・モデル（LBCM）のファイル
    /// - coating_file: 系の膜ファイル（COATING.DAT 形式）を置き換える
    /// - wavelengths: 波長 [µm] を置き換える（.zmx だけ。重みは 1、主波長の番号はそのまま）
    #[staticmethod]
    #[pyo3(signature = (path, glass_dirs=None, cfg=1, state=None, lens_catalogs=None, coatings=None, coating_file=None, wavelengths=None, split_path=None))]
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
        split_path: Option<String>,
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
                    split_path: split_path.as_deref(),
                    wavelengths,
                },
            )
        });
        let lensbench_project::open::Opened {
            system: mut sys,
            warnings: warn,
            coatings: saved_coatings,
        } = o.map_err(err)?;
        warn_all(py, &warn)?;
        if coating_file.is_some() {
            sys.coating_file = coating_file;
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

    /// 系ファイル（lensbench 独自の JSON）に保存する。面・ガラスのデータ・パラメータの変更・ピックアップ・
    /// ソルブ・玉と群・多重構成の表・dn と disp を欠けなく書き、System.load で読み直すと追跡はビット一致する。
    /// 膜の実測表・モデル（coatings=）のパスも書く
    fn save(&self, path: PathBuf) -> PyResult<()> {
        let s = self.sys();
        lensbench_project::sysfile::save(&path, &s, &self.coat_files).map_err(err)
    }

    /// 系ファイルの中身（JSON の文字列）
    fn to_json(&self) -> PyResult<String> {
        lensbench_project::sysfile::to_string(&self.sys(), &self.coat_files).map_err(err)
    }

    /// 系ファイルの JSON の文字列から系を作る（coatings のパスはそのまま使う）
    #[staticmethod]
    fn from_json(py: Python<'_>, text: &str) -> PyResult<PySystem> {
        let (s, m, w) = lensbench_project::sysfile::from_str(text).map_err(err)?;
        warn_all(py, &w)?;
        Ok(PySystem::new(s, m.coatings, Default::default()))
    }

    /// 系の hash（"sha256:…"、系ファイルと同じ）。解析の結果に刻んで、後から同じ系か確かめる
    #[getter]
    fn hash(&self) -> PyResult<String> {
        lensbench_project::sysfile::system_hash(&self.sys()).map_err(err)
    }

    /// プロジェクト（ブロックで組む系）を、ファイルを介さずに JSON の文字列か dict から展開する。
    ///
    /// - project: プロジェクトファイルと同じ形の JSON の文字列、または dict
    /// - base: 相対パス（zmx ブロックの path など）の基準のフォルダ（省略でカレント）
    /// - そのほかは load と同じ。取り込んだ .zmx のピックアップのうち展開で残せないもの（範囲の外を参照するなど）は
    ///   値で固定し、warnings で知らせる
    #[staticmethod]
    #[pyo3(signature = (project, base=None, glass_dirs=None, state=None, lens_catalogs=None, coatings=None, coating_file=None, split_path=None))]
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
        split_path: Option<String>,
    ) -> PyResult<PySystem> {
        let txt: String = if let Ok(t) = project.extract::<String>() {
            t
        } else if project.cast::<PyDict>().is_ok() {
            py.import("json")?
                .call_method1("dumps", (project,))?
                .extract()?
        } else {
            return Err(PyTypeError::new_err("project は JSON の文字列か dict"));
        };
        let base = base.unwrap_or_else(|| PathBuf::from("."));
        let glass_dirs = glass_dirs.unwrap_or_default();
        let lens_catalogs = lens_catalogs.unwrap_or_default();
        let r = py.detach(|| -> Result<(System, Vec<String>), String> {
            let cat = LensCatalog::load_registered(&lens_catalogs)?;
            let look = |v: &str, p: &str| cat.zmx(v, p);
            lensbench_project::expand_json_path(
                &txt,
                &base,
                &glass_dirs,
                Some(&look),
                state.as_deref(),
                split_path.as_deref(),
            )
        });
        let (mut sys, warn) = r.map_err(err)?;
        warn_all(py, &warn)?;
        if coating_file.is_some() {
            sys.coating_file = coating_file;
        }
        Ok(PySystem::new(
            sys,
            coatings.unwrap_or_else(env_coatings),
            Default::default(),
        ))
    }

    /// プロジェクトファイル（.json）のビームスプリッターの経路の一覧（BS を通る順の T・R の文字列）。
    /// BS がなければ空。それぞれ `load(path, split_path=経路)` で読む
    #[staticmethod]
    #[pyo3(signature = (path, glass_dirs=None, state=None, lens_catalogs=None))]
    fn split_paths(
        py: Python<'_>,
        path: PathBuf,
        glass_dirs: Option<Vec<PathBuf>>,
        state: Option<String>,
        lens_catalogs: Option<Vec<PathBuf>>,
    ) -> PyResult<Vec<String>> {
        let glass_dirs = glass_dirs.unwrap_or_default();
        let lens_catalogs = lens_catalogs.unwrap_or_default();
        py.detach(|| -> Result<Vec<String>, String> {
            let txt =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let mut p = lensbench_project::Project::from_json(&txt)?;
            if let Some(st) = &state {
                p = p.with_state(st)?;
            }
            let cat = LensCatalog::load_registered(&lens_catalogs)?;
            let look = |v: &str, q: &str| cat.zmx(v, q);
            let env = lensbench_project::Env {
                base: path.parent().map(|x| x.to_path_buf()).unwrap_or_default(),
                glass_dirs,
                catalog: Some(&look),
            };
            lensbench_project::paths(&p, &env)
        })
        .map_err(err)
    }

    /// 多重構成の .zmx から複数の構成の系をまとめて読む。configs は構成の番号（1 始まり）の列で、
    /// 省くと全部（空の list はエラー）。返すのは System の list で、それぞれ `load(path, cfg=k)` と同じ。
    /// 試料腕・参照腕や絞りの径の切替（APMX・APER など）を、`trace_many(..., object=物点)` で同じ物点・
    /// 同じ瞳の格子でまとめて追跡するのに使う
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
            return Err(PyValueError::new_err(
                "load_configs は .zmx だけ（プロジェクトファイルは load(state=...) で状態ごとに）",
            ));
        }
        let opt = zmx::LoadOptions {
            catalog_dirs: glass_dirs.unwrap_or_default(),
            ..Default::default()
        };
        // 空の list は全部（None）と取り違えやすいので断る
        if configs.as_ref().is_some_and(|c| c.is_empty()) {
            return Err(PyValueError::new_err(
                "configs が空（全部の構成なら configs を省く）",
            ));
        }
        let cfgs = configs.unwrap_or_default();
        let v = py
            .detach(|| zmx::load_configs(&path, &opt, &cfgs))
            .map_err(err)?;
        let coat = coatings.unwrap_or_else(env_coatings);
        Ok(v.into_iter()
            .map(|mut s| {
                if coating_file.is_some() {
                    s.coating_file = coating_file.clone();
                }
                PySystem::new(s, coat.clone(), Default::default())
            })
            .collect())
    }

    #[getter]
    fn name(&self) -> String {
        self.sys().name.clone()
    }

    /// 波長 [µm]
    #[getter]
    fn wavelengths(&self) -> Vec<f64> {
        self.sys().wavelengths.clone()
    }

    /// 分散式の有効範囲（AGF の LD）の外の波長で屈折率を求めている材料の警告（式を延ばして計算している）
    fn dispersion_warnings(&self) -> Vec<String> {
        self.sys().dispersion_warnings()
    }

    /// ZPL Macro Solve のうち、解かずに保存された値で固定したもの（マクロがない・対応していない命令など）の理由
    fn macro_warnings(&self) -> Vec<String> {
        self.sys().macro_warnings()
    }

    #[getter]
    fn weights(&self) -> Vec<f64> {
        self.sys().weights.clone()
    }

    /// 主波長の番号（1 始まり）
    #[getter]
    fn primary(&self) -> usize {
        self.sys().primary + 1
    }

    /// 視野 (x, y)
    #[getter]
    fn fields(&self) -> Vec<(f64, f64)> {
        self.sys().fields.iter().map(|f| (f.x, f.y)).collect()
    }

    /// 像面の面番号
    #[getter]
    fn image(&self) -> usize {
        self.sys().image()
    }

    /// 絞りの面番号
    #[getter]
    fn stop(&self) -> usize {
        self.sys().stop()
    }

    /// 有限物体で入射瞳が物体より前にあるか（pupil="normalized" の向きが "signed" と逆になる系）
    #[getter]
    fn pupil_reversed(&self) -> bool {
        self.sys().pupil_reversed()
    }

    /// 無限遠物体か
    #[getter]
    fn infinite(&self) -> bool {
        self.sys().infinite()
    }

    /// 光線のエイミング（"off"・"paraxial"・"real"）
    #[getter]
    fn aiming(&self) -> &'static str {
        match self.sys().aiming {
            Aiming::Off => "off",
            Aiming::Paraxial => "paraxial",
            Aiming::Real => "real",
        }
    }

    /// 浮動開口（半直径を固定した面・.zmx の FLAP）の外の光線をけるか（既定 True。OpticStudio と同じ）。
    /// 半直径でけらない従来のモデルと比べるときに False にする
    #[getter]
    fn floating_clip(&self) -> bool {
        self.sys().floating_clip
    }

    #[setter]
    fn set_floating_clip(&self, on: bool) -> PyResult<()> {
        self.mutate(|s| {
            s.floating_clip = on;
            s.changed()
        })
    }

    /// 基点（OpticStudio の Cardinal Points 相当）。wave は波長の番号（1 始まり、省略で主波長）。
    ///
    /// 面 1 から像面の手前の面までの近軸の系（Y-Z 面）。物体空間の位置は面 1 の頂点から、像空間の位置は像面の
    /// 頂点から、それぞれの局所 z で測る [mm]（OpticStudio と同じ。鏡が奇数枚なら n_image と f_image は負）。
    /// `efl` は空気換算（EFFL と同じ）、`f_object`・`f_image` は媒質の中の焦点距離（物体空間は負）。
    /// `bfl` は最後の面から、`ffl` は面 1 から焦点までの距離。
    /// 無焦点の系では inf。`set_param` などの変更後の系の値を返す。
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

    /// 機構部品（プロジェクトの mechanics）のケラレ。CLI の `mech-check` と同じ。
    ///
    /// 瞳の正方格子（一辺 grid 本）の単位円の中の光線を、視野 fields・波長 waves（どちらも 1 始まり、省略で全部）
    /// ごとに追い、部品の穴の外に出た割合 [%] を数える。戻り値は dict:
    /// `pupil_rays`（1 視野・波長あたりの光線数）、`parts`（部品ごとの name・surface・clipped_max_pct・
    /// first_max_pct・bores）、`results`（視野・波長ごとの field・wave・rays・failed・optical_pct・passed_pct・
    /// parts=[{name, clipped_pct, first_pct}]）。clipped は部品の穴の外に出た光線（先に切れたものも数える）、
    /// first は光の通る順で最初にその部品で切れた光線、optical は部品より先に系の開口で切れた光線。
    /// `fit` は配置の確認: kind（collision・protrude・loose・overlap・rod・rod_mismatch）・error・
    /// part・other・surface・value [mm]・message の dict の並び。
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
            return Err(PyValueError::new_err(
                "機構部品がない（プロジェクトの mechanics に書く）",
            ));
        }
        if grid < 2 {
            return Err(PyValueError::new_err("grid は 2 以上"));
        }
        let nf = s.fields.len();
        let fs: Vec<usize> = match fields {
            Some(v) => v
                .into_iter()
                .map(|f| {
                    if f == 0 || f > nf {
                        Err(PyIndexError::new_err(format!(
                            "視野 {f} はありません（1〜{nf}）"
                        )))
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

    /// 単一モードファイバへの結合効率（OpticStudio の FICL 相当）。送り側は物点 source（物体面の局所 [mm]）、
    /// 受け側は像面の局所の点 receiver、どちらも遠視野の 1/e² の NA で決まるガウスモード（na_receiver を省くと
    /// na_source と同じ）。物体空間の sin θ ≤ na_max（省略で min(3·NA/n, 0.999)）を grid × grid の方向で標本化し、
    /// 受け側の方向空間で重なり積分をとる。返す dict: eta（結合効率）・system（受け側まで届いた送り側の電力の比）・
    /// receiver（eta/system）。pol=True なら膜込みの偏光の重なり jones（(2, 2) complex、[出射 x/y, 入射 x/y]）も返し、
    /// 入射偏光 e の結合は |jones @ e|²。clip_stop=True（既定）なら系の絞り（絞り面に開口がなければ絞り半径の円）の外を
    /// 通る光を系の損失に数える（OpticStudio の FICL と同じ）。有限物体の系だけ
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
        let w = match wave {
            Some(w) => wave_index(&s, w)?,
            None => s.primary,
        };
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
        let d = PyDict::new(py);
        d.set_item("eta", c.eta)?;
        d.set_item("system", c.system)?;
        d.set_item("receiver", c.receiver)?;
        if let Some(j) = c.jones {
            let a = Array::from_shape_fn((2, 2), |(i, k)| j[i][k]);
            d.set_item("jones", a.into_pyarray(py))?;
        }
        Ok(d)
    }

    /// システムデータ（OpticStudio のオペランド名の小文字: effl・epdi・enpp・expd・expp・wfno・pwfn・isfn・
    /// pimh・pmag・totr・obna・isna）。主波長、変更後の系の値
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
            ("obna", r.obna),
            ("isna", r.isna),
        ] {
            d.set_item(k, v)?;
        }
        Ok(d)
    }

    /// 視野番号 i（1 始まり）の正規化視野 (hx, hy)
    fn hxhy(&self, i: usize) -> PyResult<(f64, f64)> {
        let s = self.sys();
        match field_sel(&s, FieldArg::Index(i))? {
            FieldSel::Index(j) => Ok(s.hxhy(j)),
            FieldSel::Norm(..) => unreachable!(),
        }
    }

    /// 視野（番号か (hx, hy)）の物体: 有限物体なら物点 (x, y) [mm]、無限遠なら方向 (l, m, n)
    fn object_of(&self, field: FieldArg) -> PyResult<Vec<f64>> {
        let s = self.sys();
        let (hx, hy) = match field_sel(&s, field)? {
            FieldSel::Index(j) => s.hxhy(j),
            FieldSel::Norm(a, b) => (a, b),
        };
        Ok(obj_out(s.object_spec(hx, hy).map_err(err)?))
    }

    /// 面を番号か名前（コメント）で探し、面番号を返す
    fn find_surface(&self, target: &str) -> PyResult<usize> {
        self.sys().find_surface(target).map_err(err)
    }

    /// 玉・群の一覧: name・spans（面の範囲 [最初, 最後] の並び）・pivot・motion（dx dy tx ty tz）・shift・auto の dict
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

    /// 玉・群を定義する。spans は面の範囲 (最初, 最後) の並び（ダブルパスの往きと戻りなど、同じ玉を 2 回通るなら
    /// 2 つ）、pivot は回転の中心（最初の範囲の最初の面の頂点が原点の座標）。以後 "名前.dx" などで変えられる
    #[pyo3(signature = (name, spans, pivot=(0.0, 0.0, 0.0)))]
    fn define_unit(
        &self,
        name: &str,
        spans: Vec<(usize, usize)>,
        pivot: (f64, f64, f64),
    ) -> PyResult<()> {
        let mut g = self.sys.write().unwrap_or_else(|e| e.into_inner());
        let mut n = g.with_params(&[]).map_err(err)?;
        let sp: Vec<[usize; 2]> = spans.into_iter().map(|(a, b)| [a, b]).collect();
        n.define_unit(name, &sp, [pivot.0, pivot.1, pivot.2])
            .map_err(err)?;
        *g = Arc::new(n);
        Ok(())
    }

    /// 玉・群の回転の中心を変える
    fn set_unit_pivot(&self, name: &str, pivot: (f64, f64, f64)) -> PyResult<()> {
        let mut g = self.sys.write().unwrap_or_else(|e| e.into_inner());
        let Some(u) = g.find_unit(name) else {
            return Err(PyValueError::new_err(format!("玉・群 {name} がない")));
        };
        let mut n = g.with_params(&[]).map_err(err)?;
        n.set_unit_pivot(u, [pivot.0, pivot.1, pivot.2])
            .map_err(err)?;
        *g = Arc::new(n);
        Ok(())
    }

    /// パラメータの今の値。spec は "面.種類"（例 "3.t"、"L1 front.c"）。種類は c r k a2〜a16 t dx dy tx ty tz dn disp。
    /// 玉・群は "名前.種類"（例 "lens2.tx"、"G1.shift"）で、種類は dx dy tx ty tz shift dn disp
    fn get_param(&self, spec: &str) -> PyResult<f64> {
        let s = self.sys();
        let p = s.param(spec).map_err(err)?;
        s.get_param(p).map_err(err)
    }

    /// パラメータを変える（系を作り直さない）。ピックアップ先は元の面に合わせて解き直す（ピックアップ先そのものは変えられない）
    fn set_param(&self, spec: &str, value: f64) -> PyResult<()> {
        let mut g = self.sys.write().unwrap_or_else(|e| e.into_inner());
        let p = g.param(spec).map_err(err)?;
        let n = g.with_params(&[(p, value)]).map_err(err)?;
        *g = Arc::new(n);
        Ok(())
    }

    /// まとめて変える: {"3.t": 10.0, "5.dx": 0.01} か [("3.t", 10.0), ...]
    fn set_params(&self, changes: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut g = self.sys.write().unwrap_or_else(|e| e.into_inner());
        let c = parse_changes(&g, changes)?;
        let n = g.with_params(&c).map_err(err)?;
        *g = Arc::new(n);
        Ok(())
    }

    /// 変えた系を新しく返す（元の系は変えない。軽い複製）
    fn with_params(&self, changes: &Bound<'_, PyAny>) -> PyResult<PySystem> {
        let s = self.sys();
        let c = parse_changes(&s, changes)?;
        let n = s.with_params(&c).map_err(err)?;
        Ok(PySystem::new(n, self.coat_files.clone(), self.coat_cell()))
    }

    // ---------------------------------------------------------------------- 系の編集
    /// 面 s の値の dict（type comment t c r k asph cx material mirror coating semi stop ignore aperture cb cb_order
    /// focal jones dn disp n）。r は曲率半径（平面は inf）、aperture は dict か None
    fn surface<'py>(&self, py: Python<'py>, s: SurfArg) -> PyResult<Bound<'py, PyDict>> {
        let sys = self.sys();
        let j = surf_of(&sys, s)?;
        edit::fields_dict(py, sys.surface_fields(j).map_err(err)?)
    }

    /// 面の数（物体面と像面を含む）
    #[getter]
    fn surface_count(&self) -> usize {
        self.sys().surfaces.len()
    }

    /// 面 at の前に面を入れる（at は 1〜像面）。値はキーワードで（surface() の名前。type material r t など）。
    /// 入れた面の後ろの媒質はその面の材料（省略で空気）
    #[pyo3(signature = (at, **kw))]
    fn insert_surface(&self, at: usize, kw: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let f = edit::surf_kwargs(kw)?;
        self.mutate(|s| {
            s.insert_surface(at, lensbench_core::surface::Surface::default())?;
            s.set_surface_fields(at, &f)
        })
    }

    /// 面 s を消す。消した面を参照していたピックアップ・ソルブ・多重構成の行の説明の list を返す（警告にも出す）
    fn delete_surface(&self, py: Python<'_>, s: SurfArg) -> PyResult<Vec<String>> {
        let j = surf_of(&self.sys(), s)?;
        let notes = self.mutate(|s| s.delete_surface(j))?;
        warn_all(py, &notes)?;
        Ok(notes)
    }

    /// 面 s の値をキーワードでまとめて変える（surface() の名前。mirror・stop・n は除く。材料は material）
    #[pyo3(signature = (s, **kw))]
    fn set_surface(&self, s: SurfArg, kw: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let j = surf_of(&self.sys(), s)?;
        let f = edit::surf_kwargs(kw)?;
        self.mutate(|s| s.set_surface_fields(j, &f))
    }

    /// 面 s の種類（STANDARD EVENASPH COORDBRK JONESMAT PARAXIAL TOROIDAL FRESNELS）。新しい種類に無い値は 0 に戻す
    fn set_surface_type(&self, s: SurfArg, kind: &str) -> PyResult<()> {
        let j = surf_of(&self.sys(), s)?;
        let t = lensbench_core::edit::surf_type_parse(kind).map_err(err)?;
        self.mutate(|s| s.set_surface_type(j, t))
    }

    /// 面 s の材料: ガラス名（系のカタログ、add_glass_catalogs で足せる）・"nd,vd[,dpgf]"・"" か "AIR"・"MIRROR"
    fn set_material(&self, s: SurfArg, material: &str) -> PyResult<()> {
        let j = surf_of(&self.sys(), s)?;
        self.mutate(|s| s.set_material(j, material))
    }

    /// 面 s を絞りにする
    fn set_stop(&self, s: SurfArg) -> PyResult<()> {
        let j = surf_of(&self.sys(), s)?;
        self.mutate(|s| s.set_stop(j))
    }

    /// ガラスのカタログ（AGF の名前、例 "SCHOTT"）を読んで系に足す。dirs は探す場所（省略で既定の場所）。
    /// 見つからなかったカタログの名前を返す
    #[pyo3(signature = (names, dirs=None))]
    fn add_glass_catalogs(
        &self,
        names: Vec<String>,
        dirs: Option<Vec<PathBuf>>,
    ) -> PyResult<Vec<String>> {
        let dirs = dirs.unwrap_or_default();
        self.mutate(|s| Ok(s.add_glass_catalogs(&names, &dirs)))
    }

    /// ピックアップの一覧: dst・src・what（c t k p1〜 before0〜4 after0〜4 など）・scale・offset の dict。元の量が what と違えば src_what も
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

    /// ピックアップを付ける: 面 dst の what = scale × 面 src の src_what + offset（同じ先・量のものは置き換える）。
    /// what は c r t k p1〜 dn disp semi before0〜4 after0〜4（面の前・後の傾き偏心 dx dy tx ty tz）。
    /// src_what を省くと what と同じ量（例: 面の後を元の面の前から引くなら what="after0", src_what="before0"）
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
        let mut p = lensbench_core::param::Pickup::new(
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

    /// 面 dst の what のピックアップを外す（値は今のまま）
    fn remove_pickup(&self, dst: SurfArg, what: &str) -> PyResult<()> {
        let j = surf_of(&self.sys(), dst)?;
        let w = edit::pick_kind(what)?;
        self.mutate(|s| s.remove_pickup(j, w))
    }

    /// ソルブの一覧: surf・what（"t" 厚さ / "c" 曲率）・kind（種類の名前）・value・param・other の dict
    fn solves<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        self.sys()
            .solves
            .iter()
            .map(|q| {
                let (name, param, other) = lensbench_core::edit::solve_kind_info(q.kind);
                let d = PyDict::new(py);
                d.set_item("surf", q.surf)?;
                d.set_item("what", if q.kind.is_curv() { "c" } else { "t" })?;
                d.set_item("kind", name)?;
                d.set_item("value", q.height)?;
                d.set_item("param", param)?;
                d.set_item("other", other)?;
                Ok(d)
            })
            .collect()
    }

    /// 面 surf にソルブを付ける（同じ面・同じ量のものは置き換える）。kind は厚さの marginal_height（既定）・
    /// marginal_height_zone・chief_height・pupil_position・edge・opd・position・compensator・center_of_curvature、
    /// 曲率の marginal_angle・chief_angle・marginal_normal・chief_normal・aplanatic・element_power・
    /// concentric_surface・concentric_radius・f_number。value はその値（高さ・角度・長さなど）、param は
    /// marginal_height_zone・opd の瞳の帯・edge の高さ、other は別の面を参照する種類の面
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
        let q = lensbench_core::param::ThickSolve {
            surf: surf_of(&s0, surf)?,
            height: value,
            kind: lensbench_core::edit::solve_kind_parse(kind, param, other).map_err(err)?,
        };
        self.mutate(|s| s.add_solve(q))
    }

    /// 面 surf の what（"t" 厚さ / "c" 曲率）のソルブを外す（値は今のまま）
    #[pyo3(signature = (surf, what="t"))]
    fn remove_solve(&self, surf: SurfArg, what: &str) -> PyResult<()> {
        let j = surf_of(&self.sys(), surf)?;
        let k = match what {
            "t" => lensbench_core::param::Kind::Thick,
            "c" => lensbench_core::param::Kind::Curv,
            _ => return Err(PyValueError::new_err(format!("what は t か c（{what}）"))),
        };
        self.mutate(|s| s.remove_solve(j, k))
    }

    /// 多重構成の数（表が無ければ 1）
    #[getter]
    fn config_count(&self) -> usize {
        self.sys().configs().map_or(1, |c| c.n)
    }

    /// 今の系に入っている構成（1 始まり）
    #[getter]
    fn active_config(&self) -> usize {
        self.sys().configs().map_or(1, |c| c.active)
    }

    /// 構成の数を n にする（増やした構成の値は今の値、1 で表を消す）
    fn set_config_count(&self, n: usize) -> PyResult<()> {
        self.mutate(|s| s.set_config_count(n))
    }

    /// 多重構成の表: (パラメータ名, 構成ごとの値) の list
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

    /// 表の行を足す（同じパラメータは置き換える）: spec は set_param と同じ名前、values は構成ごとの値
    fn set_config_row(&self, spec: &str, values: Vec<f64>) -> PyResult<()> {
        self.mutate(|s| {
            let p = s.param(spec)?;
            s.set_config_row(p, values)
        })
    }

    /// 表の行を消す（系の値は今のまま）
    fn remove_config_row(&self, spec: &str) -> PyResult<()> {
        self.mutate(|s| {
            let p = s.param(spec)?;
            s.remove_config_row(p)
        })
    }

    /// 構成 k（1 始まり）の値を入れた新しい系（元の系は変えない）
    fn config(&self, k: usize) -> PyResult<PySystem> {
        let n = self.sys().config(k).map_err(err)?;
        Ok(PySystem::new(n, self.coat_files.clone(), self.coat_cell()))
    }

    /// 今の系を構成 k に切り替える
    fn set_config(&self, k: usize) -> PyResult<()> {
        self.mutate(|s| {
            *s = s.config(k)?;
            Ok(())
        })
    }

    /// ゴースト: 面 a で反射し、面 b で反射する（b < a）経路を順次の面に展開した系（膜の係数・偏光追跡もそのまま使える）。
    /// 面は番号かコメント。展開した系の面ごとの元の面は ghost_origin で
    fn ghost(&self, a: SurfArg, b: SurfArg) -> PyResult<PySystem> {
        let s = self.sys();
        let (a, b) = (surf_of(&s, a)?, surf_of(&s, b)?);
        let g = lensbench_core::ghost::ghost_system(&s, a, b).map_err(err)?;
        Ok(PySystem::new(
            g.sys,
            self.coat_files.clone(),
            self.coat_cell(),
        ))
    }

    /// ゴーストの系（ghost(a, b)）の面ごとの元の面の番号（挟んだ座標ブレークは None）と、反射の面の番号 (a, b)
    fn ghost_origin(&self, a: SurfArg, b: SurfArg) -> PyResult<GhostOrigin> {
        let s = self.sys();
        let (a, b) = (surf_of(&s, a)?, surf_of(&s, b)?);
        let g = lensbench_core::ghost::ghost_system(&s, a, b).map_err(err)?;
        Ok((g.origin, g.at))
    }

    /// 2 回反射の組 [(a, b), ...]（a > b、a が先に反射）。first..=last の面（省略で 1〜像面。像面は膜があるときだけ）
    #[pyo3(signature = (first=1, last=None))]
    fn ghost_pairs(&self, first: usize, last: Option<usize>) -> Vec<(usize, usize)> {
        let s = self.sys();
        let last = last.unwrap_or(s.image());
        lensbench_core::ghost::pairs(&s, first, last)
    }

    /// 視野の種類: "angle"・"object_height"・"paraxial_image_height"・"real_image_height"、
    /// 条件で定める視野なら ("solve", ("dirsum", 面1, 面2)) など
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

    /// 視野の種類と値を変える。kind は field_type と同じ名前。"solve" なら cond に条件
    /// （("position", 面)、("dirsum", 面1, 面2)、("dirdiff", 面1, 面2)、面は番号か名前）を与え、
    /// 視野の (x, y) が条件の目標値になる物体を主波長の主光線で解く（パラメータを変えると解き直す）。
    /// fields を与えると視野の (x, y) も置き換える（重み・ケラレ係数は既定に戻る）
    #[pyo3(signature = (kind, cond=None, fields=None))]
    fn set_field_type(
        &self,
        kind: &str,
        cond: Option<Bound<'_, PyTuple>>,
        fields: Option<Vec<(f64, f64)>>,
    ) -> PyResult<()> {
        let mut g = self.sys.write().unwrap_or_else(|e| e.into_inner());
        let ft = match (kind.to_ascii_lowercase().as_str(), &cond) {
            ("angle", None) => FieldType::Angle,
            ("object_height", None) => FieldType::ObjH,
            ("paraxial_image_height", None) => FieldType::ParH,
            ("real_image_height", None) => FieldType::RealH,
            ("solve", Some(c)) => FieldType::Solve(edit::field_cond(&g, c)?),
            ("solve", None) => return Err(PyValueError::new_err("solve には cond が要る")),
            (_, Some(_)) => return Err(PyValueError::new_err("cond は kind が solve のときだけ")),
            (k, None) => {
                return Err(PyValueError::new_err(format!(
                    "視野の種類 {k}（angle・object_height・paraxial_image_height・real_image_height・solve）"
                )))
            }
        };
        let mut n = g.with_params(&[]).map_err(err)?;
        n.field_type = ft;
        if let Some(f) = fields {
            if f.is_empty() {
                return Err(PyValueError::new_err("視野が空"));
            }
            n.fields = f.into_iter().map(|(x, y)| Field::xy(x, y)).collect();
        }
        n.changed().map_err(err)?;
        *g = Arc::new(n);
        Ok(())
    }

    /// 光線を追跡し、numpy 配列の dict を返す。
    ///
    /// - field: 視野の番号（1 始まり）か正規化視野 (hx, hy)。object を与えるときは None
    /// - px, py: 正規化瞳座標（1 次元、同じ長さ。list・float32 も可）
    /// - wavelengths: 波長の番号（1 始まり）の列。省くと全波長
    /// - last: この面で止める（省くと像面）
    /// - record: True なら途中の全面の交点も返す（`rec_*`、形は (波長, 面, 光線)）。面の番号の列なら
    ///   その面だけ（無視する面・last より後は除く。面の多い系で速い）
    /// - pol: 偏光追跡する波長の番号（True なら主波長）。`jones` に (光線, 2, 2) の complex128 を返す。
    ///   列は入射偏光（物体側の局所 x・y の単位ベクトル）、行は最後の面の局所座標の出射電場の x・y 成分。
    ///   z 成分は `jones_z`（(光線, 2)）。膜の振幅・位相込み、位相の約束は E ∝ exp(−ikL)。止まった光線は NaN
    /// - pol_phase: True なら jones に伝搬の位相 exp(−ik·OPL) を掛ける（既定は掛けない。光路長は `opl`）
    /// - pol_grid: 偏光追跡を正規化瞳座標 [−1, 1]² の粗い格子（点数 n か (nx, ny)、各軸 4 点以上。例 25）で行い、
    ///   伝搬の位相を除いた Jones 行列を 2 次元の 3 次スプライン（not-a-knot）で全光線へ補間する（全光線の偏光
    ///   追跡より速い）。止まった・けられた格子点は格子で最も近い有効な点で埋める。瞳の内側（px² + py² ≤ 1）で欠けた
    ///   格子点は `pol_grid_missing`（(点, 2) の px・py）に返し、そこから 2 点の範囲のセルにある光線は jones を NaN に
    ///   して `pol_grid_bad`（(光線,) の bool）を True にする。pupil が "normalized"・"signed" のときだけ
    /// - globals: True なら大域の位置・方向 gx, gy, gz, gl, gm, gn と開始点・方向 p0, k0（(波長, 光線, 3)）も返す
    /// - object: 視野の代わりに物体を直接: 有限物体なら物点 (x, y) [mm]、無限遠物体なら方向 (l, m, n)。
    ///   ケラレ係数はかけない
    /// - grad: 光路長を微分するパラメータ（"3.t"、"objx" など）の列。`grad` に dOPL/dp（(波長, 光線, パラメータ)）を返す。
    ///   始点と最後の面の交点（局所 x・y）を固定した微分。objx・objy は有限物体のみ
    /// - land: True なら grad と一緒に `grad_land`（(波長, 光線, パラメータ, 4)）に最後の面での着地点・方向の微分
    ///   d(x, y, l, m)/dp を返す。開始位置と開始方向を固定した微分（同じ光線を打ち出して系だけ変える。
    ///   pupil="stop" や照準した光線でも、解き直さない）
    /// - dz: 検出面を最後の面の局所座標の z = dz の平面に置く（面の傾き・偏心に従う平面）。x, y, z・opl・
    ///   opl_os・大域の位置をその平面での値にし、grad はその平面の x・y を固定した微分、grad_land もその平面で。
    ///   記録（record）は変えない。pol_phase=True とは一緒に使えない
    /// - threads: スレッド数（1 で逐次。省くと全コア）。結果はスレッド数によらず同じビット
    /// - pupil: "normalized"（既定）なら px・py は正規化瞳座標（OpticStudio と同じ。入射瞳が物体より前の
    ///   有限物体系では px = 1 が絞りの −x 側を通り、テレセントリックをまたぐと向きが反転する）。"signed" は
    ///   その系で符号を反転した瞳座標（px = 1 が常に絞りの同じ側。`System.pupil_reversed` が True の系で
    ///   normalized と符号が逆）。"stop" なら絞り面の局所座標 [mm] で、
    ///   その点を通る実光線を光線ごとに解く（系のエイミング・入射瞳の位置によらない。ケラレ係数はかけない。
    ///   解けない光線は err が絞りの面）
    ///
    /// 返す配列（形は (波長, 光線)）: x, y, z, l, m, n（最後の面の局所座標の位置・方向余弦）,
    /// opl（光路長）, opl_os（OpticStudio の生の光路長）, lens_opd（理想レンズの位相）, err・vig（止まった・
    /// けられた面の番号、0 は通過）, ok（err も vig も 0）。ほかに wavelengths（番号）, last、視野なら hx・hy、
    /// object なら object。record のとき rec_surfaces と rec_x … rec_opl（rec_gx・rec_gy・rec_gz は大域）
    #[pyo3(signature = (field, px, py, wavelengths=None, last=None, record=RecArg::Flag(false), pol=None, pol_phase=false, globals=false, object=None, grad=None, threads=None, pupil="normalized", dz=None, land=false, pol_grid=None))]
    #[allow(clippy::too_many_arguments)]
    fn trace<'py>(
        &self,
        gil: Python<'py>,
        field: Option<FieldArg>,
        px: PyArrayLike1<'py, f64, AllowTypeChange>,
        py: PyArrayLike1<'py, f64, AllowTypeChange>,
        wavelengths: Option<Vec<usize>>,
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
            &s,
            wavelengths,
            last,
            record,
            pol,
            pol_phase,
            globals,
            grad,
            threads,
            dz,
            land,
        )?;
        if let Some(g) = pol_grid {
            let g = match g {
                GridArg::N(k) => (k, k),
                GridArg::Nxy(v) => v,
            };
            if opts.pw.is_none() {
                return Err(PyValueError::new_err("pol_grid は pol と一緒に与える"));
            }
            if pre.at_stop {
                return Err(PyValueError::new_err(
                    "pol_grid は pupil が \"normalized\" か \"signed\" のときだけ",
                ));
            }
            if g.0 < polgrid::MIN_POINTS || g.1 < polgrid::MIN_POINTS {
                return Err(PyValueError::new_err(format!(
                    "pol_grid は各軸 {} 点以上（{:?}）",
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

    /// 開始位置と方向を直接与えて追跡する。返す dict は [`trace`] と同じ（hx・hy・object はない）。
    ///
    /// - start, direction: (光線, 3) の配列。方向は規格化する（0 ベクトルは止まった光線）。光路長は開始位置から
    /// - frame: "object"（既定。有限物体なら物体面の局所座標、無限遠物体なら面 1 の変換の前の座標系）か "global"（大域）
    /// - ほかの引数は trace と同じ。grad の objx・objy は開始位置を物体面と一緒に動かした微分
    #[pyo3(signature = (start, direction, wavelengths=None, last=None, record=RecArg::Flag(false), pol=None, pol_phase=false, globals=false, frame="object", grad=None, threads=None, dz=None, land=false))]
    #[allow(clippy::too_many_arguments)]
    fn trace_rays<'py>(
        &self,
        gil: Python<'py>,
        start: PyArrayLike2<'py, f64, AllowTypeChange>,
        direction: PyArrayLike2<'py, f64, AllowTypeChange>,
        wavelengths: Option<Vec<usize>>,
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
                    return Err(PyValueError::new_err(format!(
                        "{name} の形は (光線, 3)（{:?}）",
                        v.shape()
                    )));
                }
                Ok(v.rows().into_iter().map(|r| [r[0], r[1], r[2]]).collect())
            };
        let (mut p, mut k) = (rows(&start, "start")?, rows(&direction, "direction")?);
        if p.len() != k.len() {
            return Err(PyValueError::new_err(format!(
                "start と direction の本数が違う（{} と {}）",
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
                return Err(PyValueError::new_err(format!(
                    "frame は \"object\" か \"global\"（{f}）"
                )))
            }
        }
        let launch = Launch::Rays { p: &p, k: &k };
        let opts = TraceOpts::new(
            &s,
            wavelengths,
            last,
            record,
            pol,
            pol_phase,
            globals,
            grad,
            threads,
            dz,
            land,
        )?;
        self.run_trace(gil, &s, &launch, p.len(), &opts)
    }

    /// 未知数 2 つ・残差 2 つの実光線を Newton 法で解く。
    ///
    /// - unknown: "object"（物点の x・y [mm]、有限物体）か "pupil"（瞳座標 px・py）
    /// - residual: ("position", 面, (x, y))（面での局所座標の位置 − 目標）、("dirsum", 面1, 面2)（方向余弦
    ///   l・m の和）、("dirdiff", 面1, 面2)（差）のどれか
    /// - object: 物体（物点 (x, y) か方向 (l, m, n)）。未知数が物点なら初期値。省くと field の物体
    /// - field: object を省いたときの視野（番号か (hx, hy)、省くと軸上）
    /// - pupil: 瞳座標。未知数が瞳座標なら初期値
    /// - wavelength: 波長の番号（省くと主波長）
    ///
    /// 返す dict: object、pupil、residual、iterations（そのまま trace(None, [px], [py], object=...) に渡せる）
    #[pyo3(signature = (unknown, residual, object=None, field=None, pupil=(0.0, 0.0), wavelength=None, tol=1e-12, max_iter=50))]
    #[allow(clippy::too_many_arguments)]
    fn solve_ray<'py>(
        &self,
        gil: Python<'py>,
        unknown: &str,
        residual: &Bound<'py, PyTuple>,
        object: Option<ObjArg>,
        field: Option<FieldArg>,
        pupil: (f64, f64),
        wavelength: Option<usize>,
        tol: f64,
        max_iter: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let unknown = match unknown.to_ascii_lowercase().as_str() {
            "object" => Unknown::Object,
            "pupil" => Unknown::Pupil,
            u => {
                return Err(PyValueError::new_err(format!(
                    "unknown は object か pupil（{u}）"
                )))
            }
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
                return Err(PyValueError::new_err(
                    "residual は (\"position\", 面, (x, y))、(\"dirsum\", 面1, 面2)、(\"dirdiff\", 面1, 面2)",
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
            (Some(_), Some(_)) => {
                return Err(PyValueError::new_err("object と field は一緒に与えない"))
            }
        };
        let w = match wavelength {
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

    /// 回折 MTF（CLI の mtf と同じ）。field は番号（1 始まり）か正規化視野 (hx, hy)、wave は波長の番号（省略で
    /// 多色）、n は瞳の重なりの求積の点の数の目安（省略で自動）。method は "auto"（既定。"interp" で、n を省いて
    /// 求積の点数が 1024 を超え、周波数が 8 個以上なら "fft"）・"interp"（補間の求積）・"fft"（瞳関数の自己相関を FFT で全周波数まとめて。
    /// 周波数が多いとき速い、求積との差 1e-4 程度）・"direct"（確かめ用）。
    /// 返す dict: freqs・tangential・sagittal（numpy）・cutoff（x・y の遮断周波数 [cycles/mm]）。
    /// each_wave=True なら多色に加えて waves（波長ごとの dict のリスト。別々に求めるより速い）も返す（wave と一緒に使えない）
    #[pyo3(signature = (field, freqs, wave=None, n=None, threads=None, method="auto", each_wave=false))]
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
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let f = field_sel(&s, field)?;
        let w = wave.map(|w| wave_index(&s, w)).transpose()?;
        let fs = floats(&freqs);
        let method = match method {
            "auto" => analysis::MtfMethod::Auto,
            "interp" => analysis::MtfMethod::Interp,
            "fft" => analysis::MtfMethod::Fft,
            "direct" => analysis::MtfMethod::Direct,
            m => {
                return Err(PyValueError::new_err(format!(
                    "method は \"auto\"・\"interp\"・\"fft\"・\"direct\"（{m}）"
                )))
            }
        };
        if each_wave {
            if w.is_some() {
                return Err(PyValueError::new_err("each_wave は wave と一緒に使えない"));
            }
            let mw = py
                .detach(|| with_threads(threads, || analysis::mtf_waves(&s, f, n, &fs, method)))
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
            .detach(|| with_threads(threads, || analysis::mtf_with(&s, f, w, n, &fs, method)))
            .map_err(err)?;
        mtf_dict(py, &m)
    }

    /// スルーフォーカス MTF: 像面を dz [レンズ単位] ずつずらした回折 MTF（像面の手前の厚さを変える。
    /// 厚さのソルブは解き直さない）。tangential・sagittal は (周波数, dz) の配列。計算できない点は NaN で、
    /// 理由は warnings
    #[pyo3(signature = (field, freqs, dz, wave=None, n=None, threads=None))]
    #[allow(clippy::too_many_arguments)]
    fn through_focus_mtf<'py>(
        &self,
        py: Python<'py>,
        field: FieldArg,
        freqs: PyArrayLike1<'_, f64, AllowTypeChange>,
        dz: PyArrayLike1<'_, f64, AllowTypeChange>,
        wave: Option<usize>,
        n: Option<usize>,
        threads: Option<usize>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let f = field_sel(&s, field)?;
        let w = wave.map(|w| wave_index(&s, w)).transpose()?;
        let (fs, z) = (floats(&freqs), floats(&dz));
        let m =
            py.detach(|| with_threads(threads, || sweep::through_focus_mtf(&s, f, w, n, &fs, &z)));
        sweep_dict(py, "dz", &m)
    }

    /// フィールド MTF: 正規化視野 h（0〜1、axis "y" か "x" の向き）ごとの回折 MTF。
    /// tangential・sagittal は (周波数, h) の配列、field_value は視野の値（系の単位）
    #[pyo3(signature = (freqs, h, axis="y", wave=None, n=None, threads=None))]
    #[allow(clippy::too_many_arguments)]
    fn field_mtf<'py>(
        &self,
        py: Python<'py>,
        freqs: PyArrayLike1<'_, f64, AllowTypeChange>,
        h: PyArrayLike1<'_, f64, AllowTypeChange>,
        axis: &str,
        wave: Option<usize>,
        n: Option<usize>,
        threads: Option<usize>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let ax = field_axis(axis)?;
        let w = wave.map(|w| wave_index(&s, w)).transpose()?;
        let (fs, hs) = (floats(&freqs), floats(&h));
        let m = py.detach(|| with_threads(threads, || sweep::field_mtf(&s, ax, &hs, w, n, &fs)));
        let d = sweep_dict(py, "h", &m)?;
        let fv: Vec<f64> = hs.iter().map(|&x| ax.value(&s, x)).collect();
        d.set_item("field_value", fv.into_pyarray(py))?;
        Ok(d)
    }

    /// 減衰最小二乗（DLS）で最適化した系を返す（元の系は変えない）。vars は変数の列で、各要素は "7.t" か
    /// ("7.t", 下限, 上限)（None で片側なし）。既定の評価関数 merit は "spot"（RMS スポット）か "wavefront"（RMS 波面）で、
    /// 全視野（視野の重み）・多色。None なら terms の項だけ。jac は "auto"（解析的に求められる列が 16 個以上の項は解析的に、少ない項は中心差分）か
    /// "fd"（すべて中心差分）か "forward"（すべて前進差分、中心差分の半分の追跡）。
    ///
    /// termsは足す項の dict の並び。{"type": "ray", "field": 1, "py": 1, "q": "y", "target": 0, "weight": 1} の
    /// 形で、種類は spot・wavefront・mtf・paraxial（q: efl・pmag・enpp・epd・expp・expd・wfno・pwfn・isfn・pimh・totr・
    /// obna・isna）・ray（q: x・y・z・l・m・n、面 surf の局所座標）・centroid（像面の光束の重心、q: x・y）・opd・zernike・coupling・zplm（ZPL マクロの OPTRETURN）・
    /// plugin（登録したプラグインの出力。CLI の --term と同じ）。
    /// {"type": "user", "fn": f, "jac": g, "name": ..., "weight": ...} は Python の関数で定義する項: f(systems) は
    /// System の list を受け取り、系ごとの残差を (len(systems), 残差の数) の配列で返す（差分の系はまとめて 1 回で渡す）。
    /// g(system, names) を与えると残差の偏微分 (残差の数, len(names)) をそれで求める。
    ///
    /// 返り値の dict: system（最適化した系）・x0・x（変数の値）・merit0・merit・history（反復ごとの評価関数）・
    /// iterations・evals・stop（"converged"・"small_step"・"no_progress"・"max_iter"・"jacobian_failed"）・note（止まった理由の補足）・
    /// inactive（効かないとみなして動かさなかった変数の番号）・term_values0・term_values
    /// （terms の項ごとの値: 目標のある項は値、ほかは √Σr²）
    #[pyo3(signature = (vars, merit=Some("spot"), reference="centroid", rings=3, iters=50, jac="auto", threads=None, terms=None))]
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
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let (p, users, n_default) = opt_problem(
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
        let o = optim::DlsOpts {
            max_iter: iters,
            ..Default::default()
        };
        let r = interruptible(py, |stop| {
            with_threads(threads, || {
                optim::dls_with(&p, &o, |_| !stop.load(Ordering::Relaxed))
            })
        })?;
        userterm::raise_user_errors(&users)?;
        let r = r.map_err(err)?;
        let tv = |x: &[f64]| -> PyResult<Vec<f64>> {
            let v = py
                .detach(|| with_threads(threads, || p.term_values(x)))
                .map_err(err);
            userterm::raise_user_errors(&users)?;
            Ok(v?[n_default..].to_vec())
        };
        let (tv0, tv1) = (tv(&r.x0)?, tv(&r.x)?);
        let d = PyDict::new(py);
        let stop = r.stop.as_str();
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
        d.set_item("stop", stop)?;
        d.set_item("note", r.note)?;
        d.set_item("inactive", r.inactive)?;
        d.set_item("term_values0", tv0)?;
        d.set_item("term_values", tv1)?;
        Ok(d)
    }

    /// モンテカルロ公差シミュレーション。spec は CLI の `lensbench tolerance --spec` と同じ設定の dict
    /// （trials・seed・dist・preset・catalog・catalog_tols・lenses・groups・tols・compensators・comp_iters・merit・rings・
    /// ref・terms・sensitivity・percentiles・samples）。None なら既定（一般の公差の組を全部の玉に、RMS 波面、100 回）。
    /// terms は optimize の terms と同じ並びで、spec の terms の後に足す（Python の関数の項 {"type": "user", ...} も使える。
    /// 測定値などを評価量にするときに）。vendors はブロックの id → メーカーの dict（カタログの玉の既定の公差に使う）。
    /// configs は追加の構成（この系が構成 1、ずれはすべての構成に同じく与える）。reversed は裏返して置いた玉のブロックの
    /// id の並び（spec の wedge_face の前・後ろを玉のもとの向きで決めるのに使う。from_project で作った系は裏返しの印を
    /// 持つので要らない）。
    ///
    /// 返り値の dict: nominal（merit・terms・comp）・merit（統計: n・mean・std・min・max・percentiles）・cdf（評価関数の
    /// 昇順）・terms（項ごとの label・nominal・stats・cdf）・compensators（name・nominal・stats・試行ごとの values）・
    /// tilt_magnitudes（補償子の tx・ty の組の大きさ [分]）・merit_values（試行ごとの評価関数）・sensitivity（公差ごとの lo・hi での評価関数の
    /// 変化、悪い順）・rss_estimate・correlation（ずれとの相関 r と |ずれ| との相関 r_abs）・tolerances・trials・failed・
    /// errors（samples が true なら試行ごとの deltas・eval も）
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
        let json = py.import("json")?;
        let v: serde_json::Value = match &spec {
            None => serde_json::Value::Null,
            Some(d) => {
                let txt: String = json.call_method1("dumps", (d,))?.extract()?;
                serde_json::from_str(&txt).map_err(|e| PyValueError::new_err(e.to_string()))?
            }
        };
        let mut systems = vec![(*self.sys()).clone()];
        for c in configs.iter().flatten() {
            systems.push((*c.sys()).clone());
        }
        let vendors = vendors.unwrap_or_default();
        let reversed: std::collections::HashSet<String> =
            reversed.unwrap_or_default().into_iter().collect();
        let mut ms =
            lensbench_project::tolerance::spec_from_json_with(&v, systems, &vendors, &reversed)
                .map_err(PyValueError::new_err)?;
        let oo = lensbench_project::tolerance::out_opts(&v).map_err(PyValueError::new_err)?;
        let users = match &terms {
            Some(t) => userterm::add_terms(py, &mut ms.problem, t, &self.coat_files)?,
            None => Vec::new(),
        };
        let th = threads.or(oo.threads);
        let r = interruptible(py, |stop| {
            with_threads(th, || {
                lensbench_core::tolerance::run_with(&ms, |_, _| !stop.load(Ordering::Relaxed))
            })
        })?;
        userterm::raise_user_errors(&users)?;
        let r = r.map_err(err)?;
        let out = lensbench_project::tolerance::result_json(&ms, &r, &oo);
        json.call_method1("loads", (out.to_string(),))
    }

    /// 最適化の評価関数の残差 r とヤコビアン J（[残差][変数]）を、今の変数の値で返す（反復は自前で回す使い方。
    /// 引数は optimize と同じ。terms の項も使える）。Σ r² / Σ 重み の平方根が評価関数の値。残差は項の順
    /// （既定の評価関数の項、terms の順）で、各項に重みの平方根を掛けたもの
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

    /// RMS: スポット半径（spot・spot_x・spot_y [レンズ単位]）、波面収差 wfe [波]、ストレール比の近似
    /// strehl = exp(−(2π wfe)²)。横軸 vs は "field"（x は正規化視野、axis の向き）・"focus"（x は像面のずらし）・
    /// "wavelength"（x は波長 [µm]、単色）・None（1 点、field の値）。reference は "centroid" か "chief"、
    /// rings は瞳のガウス求積のリング数。field は vs が focus・wavelength・None のとき使う。field_wave=True で、
    /// 単色の field・wavelength の掃引の実像高（と条件で定める視野）の視野を解析の波長の主光線で解く（OpticStudio と同じ）
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
                    return Err(PyValueError::new_err(format!(
                        "reference は \"centroid\" か \"chief\"（{reference}）"
                    )))
                }
            },
        };
        let w = wave.map(|w| wave_index(&s, w)).transpose()?;
        let xs = x.map(|a| floats(&a));
        let need_x = || {
            xs.clone()
                .ok_or_else(|| PyValueError::new_err("vs を使うときは x（横軸の値の列）が要る"))
        };
        let fsel = |f: Option<FieldArg>| -> PyResult<FieldSel> {
            match f {
                Some(f) => field_sel(&s, f),
                None => Err(PyValueError::new_err("field が要る")),
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
                    return Err(PyValueError::new_err("vs=\"field\" では field は使わない"));
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
                    return Err(PyValueError::new_err(
                        "vs=\"wavelength\" では wave は使わない",
                    ));
                }
                let f = fsel(field)?;
                let l = need_x()?;
                py.detach(|| with_threads(threads, || sweep::rms_vs_wavelength(&s, f, &l, o)))
            }
            Some(v) => {
                return Err(PyValueError::new_err(format!(
                    "vs は \"field\"・\"focus\"・\"wavelength\"（{v}）"
                )))
            }
        };
        let d = PyDict::new(py);
        d.set_item("x", r.x.clone().into_pyarray(py))?;
        rms_items(&d, &r.rms, |v| Ok(v.into_pyarray(py).into_any()))?;
        d.set_item("warnings", r.warnings)?;
        Ok(d)
    }

    /// 回折 PSF: 瞳関数（MTF と同じ）の DFT。強度 i は (y, x) の配列で、収差のない同じ瞳の多色 PSF の中心を
    /// 1 とする（peak はストレール比の目安）。x・y は主波長の主光線の像点 center からの、主光線に垂直な面の座標
    /// [レンズ単位]（OpticStudio の FFT PSF と同じ。向きは axes = [e1, e2]、像空間の単位ベクトル）。
    /// pupil は瞳の格子の一辺（省略で自動: 隣り合う標本の位相差が π/2 以下）、image は像の格子の一辺、delta は像の格子の
    /// 間隔（省略で 1 / (2 × 遮断周波数)）。pupil（使った値）・phase_step（瞳の隣り合う標本の位相差の最大 [rad]）・
    /// enclosed（像の格子に入った光量の割合）・warnings（標本不足などの注意）も返す
    #[pyo3(signature = (field, wave=None, pupil=None, image=128, delta=None, threads=None))]
    #[allow(clippy::too_many_arguments)]
    fn psf<'py>(
        &self,
        py: Python<'py>,
        field: FieldArg,
        wave: Option<usize>,
        pupil: Option<usize>,
        image: usize,
        delta: Option<f64>,
        threads: Option<usize>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let f = field_sel(&s, field)?;
        let w = wave.map(|w| wave_index(&s, w)).transpose()?;
        let o = psf::PsfOpts {
            pupil,
            image,
            delta,
        };
        let p = py
            .detach(|| with_threads(threads, || psf::psf(&s, f, w, o)))
            .map_err(err)?;
        let ny = p.i.len();
        let nx = p.x.len();
        let a = Array::from_shape_fn((ny, nx), |(iy, ix)| p.i[iy][ix]);
        let d = PyDict::new(py);
        d.set_item("x", p.x.into_pyarray(py))?;
        d.set_item("y", p.y.into_pyarray(py))?;
        d.set_item("i", a.into_pyarray(py))?;
        d.set_item("center", p.center)?;
        d.set_item("axes", p.axes)?;
        d.set_item("delta", p.delta)?;
        d.set_item("peak", p.peak)?;
        d.set_item("pupil", p.pupil)?;
        d.set_item("phase_step", p.phase_step)?;
        d.set_item("enclosed", p.enclosed)?;
        d.set_item("warnings", p.warnings)?;
        Ok(d)
    }

    /// 幾何 MTF: 光線の位置の分布から。n は瞳の求積の点の数の目安、diff_limit なら回折限界の MTF をかける。
    /// 返す dict は mtf と同じ形
    #[pyo3(signature = (field, freqs, wave=None, n=128, diff_limit=false, threads=None))]
    #[allow(clippy::too_many_arguments)]
    fn geometric_mtf<'py>(
        &self,
        py: Python<'py>,
        field: FieldArg,
        freqs: PyArrayLike1<'_, f64, AllowTypeChange>,
        wave: Option<usize>,
        n: usize,
        diff_limit: bool,
        threads: Option<usize>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let f = field_sel(&s, field)?;
        let w = wave.map(|w| wave_index(&s, w)).transpose()?;
        let fs = floats(&freqs);
        let m = py
            .detach(|| with_threads(threads, || psf::geometric_mtf(&s, f, w, n, &fs, diff_limit)))
            .map_err(err)?;
        mtf_dict(py, &m)
    }

    /// Huygens PSF: 射出瞳の参照球の各点からの 2 次波を像面の上で足す（傾いた像面・収差の大きい系向け）。
    /// 返す dict は psf と同じ形で、x・y は中心 center からの像面の局所座標の差（axes は x̂・ŷ）
    #[pyo3(signature = (field, wave=None, pupil=None, image=128, delta=None, threads=None))]
    #[allow(clippy::too_many_arguments)]
    fn huygens_psf<'py>(
        &self,
        py: Python<'py>,
        field: FieldArg,
        wave: Option<usize>,
        pupil: Option<usize>,
        image: usize,
        delta: Option<f64>,
        threads: Option<usize>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let f = field_sel(&s, field)?;
        let w = wave.map(|w| wave_index(&s, w)).transpose()?;
        let o = huygens::HuygensOpts {
            pupil,
            image,
            delta,
        };
        let p = py
            .detach(|| with_threads(threads, || huygens::huygens_psf(&s, f, w, o)))
            .map_err(err)?;
        let ny = p.i.len();
        let nx = p.x.len();
        let a = Array::from_shape_fn((ny, nx), |(iy, ix)| p.i[iy][ix]);
        let d = PyDict::new(py);
        d.set_item("x", p.x.into_pyarray(py))?;
        d.set_item("y", p.y.into_pyarray(py))?;
        d.set_item("i", a.into_pyarray(py))?;
        d.set_item("center", p.center)?;
        d.set_item("axes", p.axes)?;
        d.set_item("delta", p.delta)?;
        d.set_item("peak", p.peak)?;
        d.set_item("pupil", p.pupil)?;
        d.set_item("phase_step", p.phase_step)?;
        d.set_item("enclosed", p.enclosed)?;
        d.set_item("warnings", p.warnings)?;
        Ok(d)
    }

    /// Huygens MTF: Huygens PSF のフーリエ変換（像面の上の周波数）。返す dict は mtf と同じ形
    #[pyo3(signature = (field, freqs, wave=None, pupil=None, image=128, delta=None, threads=None))]
    #[allow(clippy::too_many_arguments)]
    fn huygens_mtf<'py>(
        &self,
        py: Python<'py>,
        field: FieldArg,
        freqs: PyArrayLike1<'_, f64, AllowTypeChange>,
        wave: Option<usize>,
        pupil: Option<usize>,
        image: usize,
        delta: Option<f64>,
        threads: Option<usize>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let s = self.sys();
        let f = field_sel(&s, field)?;
        let w = wave.map(|w| wave_index(&s, w)).transpose()?;
        let fs = floats(&freqs);
        let o = huygens::HuygensOpts {
            pupil,
            image,
            delta,
        };
        let m = py
            .detach(|| with_threads(threads, || huygens::huygens_mtf(&s, f, w, o, &fs)))
            .map_err(err)?;
        mtf_dict(py, &m)
    }

    /// pickle: 系を系ファイルの JSON のバイト列にし、膜の実測表・モデルのファイル名と一緒に渡す。
    /// 読み直した側では膜ファイルと実測表を同じパスから読み直す（同じ計算機の別プロセス向け）。パラメータ変更・ガラス・
    /// 照準・ピックアップを含めて、往復した系の追跡は元の系とビット一致する
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Reduced<'py>> {
        let s = self.sys();
        let b = lensbench_project::sysfile::to_string(&s, &self.coat_files).map_err(err)?;
        let f = py.import("lensbench")?.getattr("_system_from_state")?;
        Ok((f, (PyBytes::new(py, b.as_bytes()), self.coat_files.clone())))
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

/// 多数の系を 1 回の呼び出しでまとめて追跡する。返すのは系ごとの dict の list で、
/// それぞれ `systems[i].trace(field, px, py, ...)` と同じビット（スレッド数にもよらない）。
///
/// 引数は System.trace と同じで、全部の系に同じものを使う（grad のパラメータ名も系ごとに解く）。
/// 系の単位と光線の単位の両方で並列にするので、系ごとの光線が少なくてもコアを使い切る。
/// threads は全体のスレッド数（1 で逐次）
#[pyfunction]
#[pyo3(signature = (systems, field, px, py, wavelengths=None, last=None, record=RecArg::Flag(false), pol=None, pol_phase=false, globals=false, object=None, grad=None, threads=None, pupil="normalized", dz=None, land=false))]
#[allow(clippy::too_many_arguments)]
fn trace_many<'py>(
    gil: Python<'py>,
    systems: Vec<Py<PySystem>>,
    field: Option<FieldArg>,
    px: PyArrayLike1<'py, f64, AllowTypeChange>,
    py: PyArrayLike1<'py, f64, AllowTypeChange>,
    wavelengths: Option<Vec<usize>>,
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
            wavelengths.clone(),
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

/// `__reduce__` の返す値: (作り直す関数, (系のバイト列, 膜のファイル))
type Reduced<'py> = (Bound<'py, PyAny>, (Bound<'py, PyBytes>, Vec<PathBuf>));

/// pickle から系を作り直す（[`PySystem::__reduce__`] の対）
#[pyfunction]
fn _system_from_state(state: &[u8], coatings: Vec<PathBuf>) -> PyResult<PySystem> {
    let txt = std::str::from_utf8(state)
        .map_err(|e| PyValueError::new_err(format!("系の読み直し: {e}")))?;
    // 長さと系の約束は sysfile::check で確かめる（壊れたバイト列で追跡が panic しないよう）
    let (sys, _, _) = lensbench_project::sysfile::from_str(txt)
        .map_err(|e| PyValueError::new_err(format!("系の読み直し: {e}")))?;
    Ok(PySystem::new(sys, coatings, Default::default()))
}

/// Python API の版 (major, minor)。互換を壊す変更で major、足すだけの変更で minor を上げる
/// （docs/python.md の「版と互換」）。変えたら docs/python.md の変更履歴に書く
pub const API_VERSION: (u32, u32) = (1, 1);

// 大きな配列を追跡ごとに確保し直しても OS からページを取り直さないように。別の拡張モジュールに
// 組み込むとき（default-features = false）は、その側で global_allocator を決める
#[cfg(feature = "module")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// lensbench: 順次光線追跡（Python バインディング）
#[cfg(feature = "module")]
#[pymodule]
fn lensbench(m: &Bound<'_, PyModule>) -> PyResult<()> {
    init_module(m)
}

/// モジュールに関数・クラスを足す。別の拡張モジュールに組み込むときは、その側の `#[pymodule]` でこれを呼んでから
/// `lensbench_core::catalog::register_source`・`lensbench_core::plugin::global` へ登録する
/// （先に登録すると、同梱のプラグインと同じ名前は同梱のものに戻る）
pub fn init_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // threads= を省いた計算の既定（lensbench run --threads が渡す）
    lensbench_core::init_from_env();
    errors::register(m)?;
    m.add_class::<PySystem>()?;
    m.add_function(wrap_pyfunction!(trace_many, m)?)?;
    m.add_function(wrap_pyfunction!(_system_from_state, m)?)?;
    m.add_function(wrap_pyfunction!(run_settings, m)?)?;
    m.add_function(wrap_pyfunction!(progress, m)?)?;
    fringe::register(m)?;
    plugin::register(m)?;
    runs::register(m)?;
    search::register(m)?;
    m.add("__version__", lensbench_core::VERSION)?;
    m.add("API_VERSION", API_VERSION)?;
    Ok(())
}
