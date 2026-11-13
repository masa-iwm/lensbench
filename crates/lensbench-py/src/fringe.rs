//! 単色の干渉縞（`lensbench_core::fringe`）を Python から使う。

use lensbench_core::fringe::{self as core, DetectorOpts, Polarized, PupilOpts};
use lensbench_core::raygrid::InputPol;
use lensbench_core::with_threads;
use numpy::ndarray::Array2;
use numpy::IntoPyArray;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::{err, field_sel, wave_index, FieldArg, PySystem};

/// 瞳上の干渉縞（OpticStudio の Interferogram に当たる）: 光束 1（beam1）と光束 2（beam2、None なら参照球）の
/// 波面（それぞれの主光線・参照球に対する OPD [波]）を同じ正規化瞳座標で引き、
/// Δ = scale·(W1 − W2) + (xtilt·px + ytilt·py)/2（傾きは瞳の直径あたりの縞の数）、強度 (1 + cos 2πΔ)/2。多重構成なら `System.config(k)` の系を渡す。
/// 格子は OpticStudio の Interferogram と同じ（p_i = (i − n/2 + 1)/(n/2 − 1)、瞳は入射瞳の正規化座標 = OS の
/// Use Exit Pupil を外した設定）。返す dict: p（格子の正規化瞳座標）、delta・intensity（(n, n)、行 = py、外は NaN）、
/// pv・rms（scale·(W1 − W2) の値 [波]、傾きを除く）
#[pyfunction]
#[pyo3(signature = (beam1, beam2=None, field=FieldArg::Index(1), wave=None, n=64, scale=1.0, xtilt=0.0, ytilt=0.0,
    threads=None))]
#[allow(clippy::too_many_arguments)]
fn interferogram<'py>(
    py: Python<'py>,
    beam1: PyRef<'py, PySystem>,
    beam2: Option<PyRef<'py, PySystem>>,
    field: FieldArg,
    wave: Option<usize>,
    n: usize,
    scale: f64,
    xtilt: f64,
    ytilt: f64,
    threads: Option<usize>,
) -> PyResult<Bound<'py, PyDict>> {
    let s1 = beam1.sys();
    let s2 = beam2.map(|b| b.sys());
    let f = field_sel(&s1, field)?;
    let w = wave.map(|w| wave_index(&s1, w)).transpose()?;
    let o = PupilOpts {
        n,
        scale,
        xtilt,
        ytilt,
    };
    let r = py
        .detach(|| {
            with_threads(threads, || {
                core::interferogram(&s1, s2.as_deref(), f, w, &o)
            })
        })
        .map_err(err)?;
    let m = r.p.len();
    let grid = |g: &Vec<Vec<f64>>| Array2::from_shape_fn((m, m), |(iy, ix)| g[iy][ix]);
    let d = PyDict::new(py);
    d.set_item("p", r.p.clone().into_pyarray(py))?;
    d.set_item("delta", grid(&r.delta).into_pyarray(py))?;
    d.set_item("intensity", grid(&r.intensity).into_pyarray(py))?;
    d.set_item("pv", r.pv)?;
    d.set_item("rms", r.rms)?;
    Ok(d)
}

/// 像面上の干渉縞: 試料（sample）と参照（reference）の 2 本の経路の光線の格子（瞳 pupil²、一様に照らす）を像面の
/// 画素（npx²、中心 center = (x, y)・一辺 width [mm]、省略で 2 つが届く範囲）へ補間し、場を足す。
/// polarized なら偏光追跡の場（入射偏光 sample_input・reference_input、膜は試料の系の膜）で重ねる。
/// 返す dict: x・y（画素の中心 [mm]）、intensity・i_sample・i_reference（(npx, npx)、行 = y）、opd [波]・
/// visibility（両方届く画素だけ、ほかは NaN）、ok_sample・ok_reference、wavelength [µm]、fold（折り返しの割合）
#[pyfunction]
#[pyo3(signature = (sample, reference, field=FieldArg::Index(1), wave=None, pupil=65, npx=256, center=None, width=None,
    sample_power=1.0, reference_power=1.0, delay=0.0, polarized=false, sample_input="x", reference_input="x",
    threads=None))]
#[allow(clippy::too_many_arguments)]
fn fringe_detector<'py>(
    py: Python<'py>,
    sample: PyRef<'py, PySystem>,
    reference: PyRef<'py, PySystem>,
    field: FieldArg,
    wave: Option<usize>,
    pupil: usize,
    npx: usize,
    center: Option<(f64, f64)>,
    width: Option<f64>,
    sample_power: f64,
    reference_power: f64,
    delay: f64,
    polarized: bool,
    sample_input: &str,
    reference_input: &str,
    threads: Option<usize>,
) -> PyResult<Bound<'py, PyDict>> {
    let ss = sample.sys();
    let sr = reference.sys();
    let f = field_sel(&ss, field)?;
    let w = wave.map(|w| wave_index(&ss, w)).transpose()?;
    let o = DetectorOpts {
        pupil,
        npx,
        center,
        width,
        sample_power,
        reference_power,
        delay,
    };
    let coat = if polarized {
        Some(sample.coatings(&ss)?)
    } else {
        None
    };
    let pol = if let Some(coat) = coat.as_deref() {
        Some(Polarized {
            sample: InputPol::parse(sample_input).map_err(err)?,
            reference: InputPol::parse(reference_input).map_err(err)?,
            coat,
        })
    } else {
        None
    };
    let r = py
        .detach(|| with_threads(threads, || core::detector(&ss, &sr, f, w, &o, pol)))
        .map_err(err)?;
    let n = r.n;
    let a = |v: &[f64]| Array2::from_shape_fn((n, n), |(j, i)| v[j * n + i]);
    let b = |v: &[bool]| Array2::from_shape_fn((n, n), |(j, i)| v[j * n + i]);
    let d = PyDict::new(py);
    let xs: Vec<f64> = (0..n).map(|i| r.x0 + i as f64 * r.dx).collect();
    let ys: Vec<f64> = (0..n).map(|j| r.y0 + j as f64 * r.dx).collect();
    d.set_item("x", xs.into_pyarray(py))?;
    d.set_item("y", ys.into_pyarray(py))?;
    d.set_item("intensity", a(&r.intensity).into_pyarray(py))?;
    d.set_item("i_sample", a(&r.i_sample).into_pyarray(py))?;
    d.set_item("i_reference", a(&r.i_reference).into_pyarray(py))?;
    d.set_item("opd", a(&r.opd).into_pyarray(py))?;
    d.set_item("visibility", a(&r.visibility).into_pyarray(py))?;
    d.set_item("ok_sample", b(&r.ok_sample).into_pyarray(py))?;
    d.set_item("ok_reference", b(&r.ok_reference).into_pyarray(py))?;
    d.set_item("wavelength", r.wavelength)?;
    d.set_item("fold", r.fold)?;
    Ok(d)
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(interferogram, m)?)?;
    m.add_function(wrap_pyfunction!(fringe_detector, m)?)?;
    Ok(())
}
