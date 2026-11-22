//! 像の模擬（NumPy の配列を受けて返す）

use lensbench_analysis::imagesim::{self as core, Aberrations, ImageSimOpts, SimImage};
use lensbench_core::{with_threads, System};
use numpy::ndarray::{Array2, Array3};
use numpy::{AllowTypeChange, IntoPyArray, PyArrayLikeDyn};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::err;
use crate::errors::value_err;

/// 設定（Python の引数をまとめたもの）
pub(crate) struct Args {
    pub field_height: Option<f64>,
    pub psf_grid: (usize, usize),
    pub aberrations: String,
    pub pixel: Option<f64>,
    pub size: Option<(usize, usize)>,
    pub relative_illumination: bool,
    pub rgb_waves: Option<[usize; 3]>,
    pub wave: Option<usize>,
    pub pupil: Option<usize>,
    pub psf_points: usize,
    pub map_grid: usize,
    pub threads: Option<usize>,
}

pub(crate) fn run<'py>(
    py: Python<'py>,
    s: &System,
    image: PyArrayLikeDyn<'py, f64, AllowTypeChange>,
    a: Args,
) -> PyResult<Bound<'py, PyDict>> {
    let v = image.as_array();
    let sh = v.shape().to_vec();
    let (h, w, nc) = match sh.as_slice() {
        [h, w] => (*h, *w, 1),
        [h, w, 3] => (*h, *w, 3),
        _ => {
            return Err(value_err(format!(
                "image: shape {sh:?} must be (height, width) or (height, width, 3)"
            )))
        }
    };
    let channels = (0..nc)
        .map(|c| {
            (0..h * w)
                .map(|k| {
                    if nc == 1 {
                        v[[k / w, k % w]]
                    } else {
                        v[[k / w, k % w, c]]
                    }
                })
                .collect()
        })
        .collect();
    let src = SimImage {
        width: w,
        height: h,
        channels,
    };
    let aberrations = match a.aberrations.as_str() {
        "diffraction" => Aberrations::Diffraction,
        "none" => Aberrations::None,
        x => return Err(value_err(format!("aberrations {x:?}: diffraction or none"))),
    };
    let channel_waves = match a.rgb_waves {
        Some(r) if r.iter().all(|&x| x >= 1) => Some(r.map(|x| x - 1)),
        Some(r) => {
            return Err(value_err(format!(
                "rgb_waves {r:?}: wavelength numbers from 1"
            )))
        }
        None => None,
    };
    let o = ImageSimOpts {
        field_height: a
            .field_height
            .unwrap_or_else(|| core::default_field_height(s)),
        psf_grid: a.psf_grid,
        map_grid: a.map_grid,
        pupil: a.pupil,
        psf_points: a.psf_points,
        pixel: a.pixel,
        size: a.size,
        aberrations,
        relative_illumination: a.relative_illumination,
        channel_waves,
        wave: match a.wave {
            Some(0) => return Err(value_err("wave: wavelength numbers are from 1")),
            w => w.map(|w| w - 1),
        },
    };
    let fh = o.field_height;
    let r = py
        .detach(|| with_threads(a.threads, || core::image_sim(s, &src, &o)))
        .map_err(err)?;
    let im = r.image;
    let d = PyDict::new(py);
    let (w1, h1) = (im.width, im.height);
    if im.channels.len() == 1 {
        let a = Array2::from_shape_vec((h1, w1), im.channels[0].clone())
            .map_err(|e| value_err(e.to_string()))?;
        d.set_item("image", a.into_pyarray(py))?;
    } else {
        let a = Array3::from_shape_fn((h1, w1, 3), |(y, x, c)| im.channels[c][y * w1 + x]);
        d.set_item("image", a.into_pyarray(py))?;
    }
    d.set_item("pixel", r.pixel)?;
    d.set_item("center", r.center)?;
    d.set_item("field_height", fh)?;
    d.set_item("warnings", r.warnings)?;
    Ok(d)
}
