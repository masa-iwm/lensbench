//! 単色の干渉縞（`lensbench_analysis::interference`）を Python から使う。

use lensbench_analysis::interference::{self as core, DetectorOpts, Polarized, PupilOpts};
use lensbench_analysis::psf::{Psf, PsfPlane};
use lensbench_analysis::raygrid::InputPol;
use lensbench_core::with_threads;
use numpy::IntoPyArray;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::errors::value_err;
use crate::{err, field_sel, wave_index, FieldArg, PySystem};

/// Interferogram on the pupil (like OpticStudio's Interferogram): the wavefronts (OPD [waves] relative to each chief ray and reference sphere) of beam 1 (beam1) and beam 2 (beam2; the reference sphere if None)
/// are subtracted at the same normalized pupil coordinates,
/// Δ = scale·(W1 − W2) + (xtilt·px + ytilt·py)/2 (tilts in fringes per pupil diameter), intensity (1 + cos 2πΔ)/2. For a multi-configuration system pass the system of `System.config(k)`.
/// The grid is the same as OpticStudio's Interferogram (p_i = (i − n/2 + 1)/(n/2 − 1); the pupil is normalized entrance pupil coordinates = OS with
/// Use Exit Pupil off). Returned dict: p (normalized pupil coordinates of the grid), delta and intensity ((n, n), rows = py, NaN outside),
/// pv and rms (of scale·(W1 − W2) [waves], without the tilts)
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
    let d = PyDict::new(py);
    d.set_item("p", r.delta.x().to_vec().into_pyarray(py))?;
    d.set_item("delta", grid_array(r.delta).into_pyarray(py))?;
    d.set_item("intensity", grid_array(r.intensity).into_pyarray(py))?;
    d.set_item("pv", r.pv)?;
    d.set_item("rms", r.rms)?;
    Ok(d)
}

/// Interferogram on the image surface: interpolates the ray grids (pupil², uniformly illuminated) of the two paths, sample and reference, onto the
/// pixels of the image surface (npx², center center = (x, y), side width [mm]; the range both reach if omitted) and adds the fields.
/// With polarized, superposes the fields of polarization traces (input polarizations sample_input and reference_input; coatings from the sample system).
/// Returned dict: x and y (pixel centers [mm]), intensity, i_sample and i_reference ((npx, npx), rows = y), opd [waves] and
/// visibility (only pixels both reach, NaN elsewhere), ok_sample and ok_reference, wavelength [µm], fold (fraction folded back)
#[pyfunction]
#[pyo3(signature = (sample, reference, field=FieldArg::Index(1), wave=None, pupil=65, npx=256, center=None, width=None,
    sample_power=1.0, reference_power=1.0, delay=0.0, polarized=false, sample_input="x", reference_input="x",
    threads=None))]
#[allow(clippy::too_many_arguments)]
fn interference_detector<'py>(
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
    crate::names::warn_coat(py, coat.as_deref())?;
    let d = PyDict::new(py);
    d.set_item("x", r.intensity.x().to_vec().into_pyarray(py))?;
    d.set_item("y", r.intensity.y().to_vec().into_pyarray(py))?;
    d.set_item("intensity", grid_array(r.intensity).into_pyarray(py))?;
    d.set_item("i_sample", grid_array(r.i_sample).into_pyarray(py))?;
    d.set_item("i_reference", grid_array(r.i_reference).into_pyarray(py))?;
    d.set_item("opd", grid_array(r.opd).into_pyarray(py))?;
    d.set_item("visibility", grid_array(r.visibility).into_pyarray(py))?;
    d.set_item("ok_sample", grid_array(r.ok_sample).into_pyarray(py))?;
    d.set_item("ok_reference", grid_array(r.ok_reference).into_pyarray(py))?;
    d.set_item("wavelength", r.wavelength)?;
    d.set_item("fold", r.fold)?;
    Ok(d)
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(interferogram, m)?)?;
    m.add_function(wrap_pyfunction!(interference_detector, m)?)?;
    Ok(())
}

/// Values of a 2-D grid → an array of shape (ny, nx) (uses the row-major order as it is)
pub(crate) fn grid_array<T>(g: lensbench_core::grid::Grid2<T>) -> numpy::ndarray::Array2<T> {
    let (ny, nx) = (g.ny(), g.nx());
    let (_, _, data) = g.into_parts();
    numpy::ndarray::Array2::from_shape_vec((ny, nx), data).expect("Grid2 は ny × nx")
}

/// psf の plane（"image" か "chief"）
pub(crate) fn psf_plane(s: &str) -> PyResult<PsfPlane> {
    match s {
        "image" => Ok(PsfPlane::Image),
        "chief" => Ok(PsfPlane::ChiefPerp),
        x => Err(value_err(format!(
            "plane must be \"image\" or \"chief\" ({x})"
        ))),
    }
}

/// psf・huygens_psf の返す dict
pub(crate) fn psf_dict(py: Python<'_>, p: Psf) -> PyResult<Bound<'_, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("x", p.i.x().to_vec().into_pyarray(py))?;
    d.set_item("y", p.i.y().to_vec().into_pyarray(py))?;
    d.set_item("i", grid_array(p.i).into_pyarray(py))?;
    d.set_item("center", p.center)?;
    let plane = match p.plane {
        PsfPlane::Image => "image",
        PsfPlane::ChiefPerp => "chief",
    };
    d.set_item("plane", plane)?;
    d.set_item("axes", p.axes)?;
    d.set_item("pixel", p.pixel)?;
    d.set_item("peak", p.peak)?;
    d.set_item("pupil", p.pupil)?;
    d.set_item("phase_step", p.phase_step)?;
    d.set_item("enclosed", p.enclosed)?;
    d.set_item("warnings", p.warnings)?;
    if let Some(dev) = p.device {
        d.set_item("device", dev)?;
    }
    Ok(d)
}
