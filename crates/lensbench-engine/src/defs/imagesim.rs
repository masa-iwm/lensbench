//! 像の模擬（OpticStudio の Image Simulation 相当）: PNG を読んで写し、PNG・JSON に書く

use std::path::{Path, PathBuf};

use lensbench_analysis::imagesim::{image_sim, Aberrations, Flip, ImageSimOpts, Rotate, SimImage};
use lensbench_analysis::{Analysis, Cx, Meta, Saved};
use lensbench_core::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// 収差のかけ方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum Aber {
    /// 歪曲と相対照度だけ
    None,
    /// 回折 PSF
    #[default]
    Diffraction,
    /// 幾何 PSF
    Geometric,
    /// Huygens PSF
    Huygens,
}

/// 画像の反転
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(super) enum FlipP {
    /// 反転しない
    #[default]
    None,
    /// 上下
    TopBottom,
    /// 左右
    LeftRight,
    /// 上下と左右
    Both,
}

impl From<FlipP> for Flip {
    fn from(f: FlipP) -> Flip {
        match f {
            FlipP::None => Flip::None,
            FlipP::TopBottom => Flip::TopBottom,
            FlipP::LeftRight => Flip::LeftRight,
            FlipP::Both => Flip::Both,
        }
    }
}

/// 像の模擬の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ImageSimParams {
    /// Source image (PNG, gray or RGB; a built-in 512 × 512 grid chart if omitted)
    #[schemars(extend("x-kind" = "path"))]
    pub input: Option<PathBuf>,
    /// Full height of the source image in field units (√2 × the largest field if omitted, so that the corners reach the largest field, as in OpticStudio). The image is centered on the axis (or on the field given by field); row 0 (top) is +y
    pub field_height: Option<f64>,
    /// PSF grid points across the image in x (the PSF is interpolated between them)
    #[schemars(extend("x-min" = 1))]
    pub psf_x: usize,
    /// PSF grid points across the image in y
    #[schemars(extend("x-min" = 1))]
    pub psf_y: usize,
    /// Distortion and relative illumination grid points on a side
    #[schemars(extend("x-min" = 2))]
    pub map_grid: usize,
    /// Aberrations: diffraction (FFT PSF at each PSF grid point), geometric (ray density of a pupil grid at each PSF grid point), huygens (Huygens PSF on the image surface at each PSF grid point; on the GPU if there is one) or none (distortion and relative illumination only)
    pub aberrations: Aber,
    /// Pupil grid side of the PSF (automatic if omitted; 64 for the geometric PSF)
    pub pupil: Option<usize>,
    /// Image grid side of the PSF before it is binned to detector pixels
    #[schemars(extend("x-min" = 8))]
    pub psf_points: usize,
    /// Detector pixel size [lens units] (the on-axis image of one source pixel if omitted)
    pub pixel: Option<f64>,
    /// Detector width [pixels] (same as the source if omitted)
    pub width: Option<usize>,
    /// Detector height [pixels] (same as the source if omitted)
    pub height: Option<usize>,
    /// Include relative illumination
    pub relative_illumination: bool,
    /// Gray only: wavelength number (from 1; all weighted wavelengths if omitted)
    pub wave: Option<usize>,
    /// RGB only: wavelength numbers for R, G and B (from 1; the system's wavelengths 1 to 3, shortest as B, if omitted)
    pub rgb_waves: Option<Vec<usize>>,
    /// Field number (from 1) at the center of the source image; the detector is centered on its chief ray image (the axis if omitted)
    pub field: Option<usize>,
    /// Flip the source image: none, top-bottom, left-right or both (before the rotation)
    pub flip_source: FlipP,
    /// Rotate the source image counterclockwise by 0, 90, 180 or 270 degrees (after the flip)
    pub rotate: u32,
    /// Flip the detector image: none, top-bottom, left-right or both
    pub flip_image: FlipP,
    /// Include the detector image in the result (always in .json output)
    pub grid: bool,
}

impl Default for ImageSimParams {
    fn default() -> Self {
        ImageSimParams {
            input: None,
            field_height: None,
            psf_x: 5,
            psf_y: 5,
            map_grid: 17,
            aberrations: Aber::Diffraction,
            pupil: None,
            psf_points: 64,
            pixel: None,
            width: None,
            height: None,
            relative_illumination: true,
            wave: None,
            rgb_waves: None,
            field: None,
            flip_source: FlipP::None,
            rotate: 0,
            flip_image: FlipP::None,
            grid: false,
        }
    }
}

/// 像の模擬の結果
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(super) struct ImageSimResult {
    pub width: usize,
    pub height: usize,
    /// チャンネルの数（1: グレー、3: R・G・B）
    pub channels: usize,
    /// 検出器の画素の大きさ [レンズ単位]
    pub pixel: f64,
    /// 検出器の中心（中心の視野の主光線の像点。既定は軸上）
    pub center: [f64; 2],
    /// 元の画像の高さ（全体）[視野の単位]
    pub field_height: f64,
    /// 像の最大値（1 = 収差なしで一様な元の画像の軸上の明るさ。PNG は 1 で白、それより上は切る）
    pub max: f64,
    /// 検出器の像（チャンネルごとに行優先、行 0 が上 = 像面の +y。grid または .json のときだけ）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Vec<Vec<f64>>>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written: Option<String>,
    #[serde(skip)]
    #[schemars(skip)]
    full: Vec<Vec<f64>>,
}

/// 組み込みの試験の図（512 × 512 のグレー: 黒地に 64 画素ごとの白い線と、升目の中心の点）
fn chart() -> SimImage {
    let n = 512;
    let px = |i: usize, j: usize| -> f64 {
        let line = |k: usize| k % 64 < 2 || k == n - 1;
        let dot = |k: usize| (k % 64).abs_diff(32) <= 1;
        if line(i) || line(j) || (dot(i) && dot(j)) {
            1.0
        } else {
            0.0
        }
    };
    SimImage {
        width: n,
        height: n,
        channels: vec![(0..n * n).map(|k| px(k % n, k / n)).collect()],
    }
}

pub(super) struct ImageSim;

impl Analysis for ImageSim {
    type Params = ImageSimParams;
    type Output = ImageSimResult;
    fn meta(&self) -> Meta {
        Meta {
            name: "image-sim",
            about: "Image simulation: a source image through the system's diffraction PSF, distortion and relative illumination",
            help: "Places the source image (PNG; gray uses all weighted wavelengths, RGB one wavelength per channel) in the object field and images it onto a detector, as OpticStudio's Image Simulation: chief ray image positions (distortion and lateral color), relative illumination, and the FFT PSF on a grid of fields (interpolated between the grid points; FFT convolution). Brightness 1 is the on-axis value of an unaberrated uniform source of 1. --save writes the detector image as PNG (16 bit) or JSON",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &ImageSimParams) -> Result<ImageSimResult> {
        let s = cx.system;
        let src = match &p.input {
            Some(f) => {
                let (w, h, ch) = lensbench_export::image::read_png(f)
                    .map_err(|e| Error::Io(format!("{}: {e}", f.display())))?;
                SimImage {
                    width: w,
                    height: h,
                    channels: ch,
                }
            }
            None => chart(),
        };
        let fh = p
            .field_height
            .unwrap_or_else(|| lensbench_analysis::imagesim::default_field_height(s));
        let channel_waves = match &p.rgb_waves {
            None => None,
            Some(v) if v.len() == 3 && v.iter().all(|&w| w >= 1) => {
                Some([v[0] - 1, v[1] - 1, v[2] - 1])
            }
            Some(v) => {
                return Err(Error::Invalid(format!(
                    "rgb_waves {v:?}: give 3 wavelength numbers from 1"
                )))
            }
        };
        let rotate = match p.rotate {
            0 => Rotate::R0,
            90 => Rotate::R90,
            180 => Rotate::R180,
            270 => Rotate::R270,
            r => {
                return Err(Error::Invalid(format!(
                    "rotate {r}: give 0, 90, 180 or 270 degrees"
                )))
            }
        };
        let center_field = match p.field {
            None => (0.0, 0.0),
            Some(n) if n >= 1 && n <= s.fields.len() => (s.fields[n - 1].x, s.fields[n - 1].y),
            Some(n) => {
                let nf = s.fields.len();
                return Err(Error::Invalid(format!(
                    "field {n}: field numbers are from 1 ({nf} in all)"
                )));
            }
        };
        // 回した後の元の画像の大きさ（検出器の画素の数の既定）
        let (sw, sh) = if matches!(rotate, Rotate::R90 | Rotate::R270) {
            (src.height, src.width)
        } else {
            (src.width, src.height)
        };
        let size = match (p.width, p.height) {
            (None, None) => None,
            (w, h) => Some((w.unwrap_or(sw), h.unwrap_or(sh))),
        };
        let o = ImageSimOpts {
            field_height: fh,
            psf_grid: (p.psf_x, p.psf_y),
            map_grid: p.map_grid,
            pupil: p.pupil,
            psf_points: p.psf_points,
            pixel: p.pixel,
            size,
            aberrations: match p.aberrations {
                Aber::None => Aberrations::None,
                Aber::Diffraction => Aberrations::Diffraction,
                Aber::Geometric => Aberrations::Geometric,
                Aber::Huygens => Aberrations::Huygens,
            },
            relative_illumination: p.relative_illumination,
            channel_waves,
            wave: match p.wave {
                Some(0) => {
                    return Err(Error::Invalid("wave: wavelength numbers are from 1".into()))
                }
                w => w.map(|w| w - 1),
            },
            center_field,
            flip_source: p.flip_source.into(),
            rotate,
            flip_image: p.flip_image.into(),
        };
        let r = image_sim(s, &src, &o)?;
        let im = r.image;
        let max = im.channels.iter().flatten().copied().fold(0.0, f64::max);
        Ok(ImageSimResult {
            width: im.width,
            height: im.height,
            channels: im.channels.len(),
            pixel: r.pixel,
            center: [r.center.0, r.center.1],
            field_height: fh,
            max,
            image: p.grid.then(|| im.channels.clone()),
            warnings: r.warnings,
            written: None,
            full: im.channels,
        })
    }
    fn save(&self, out: &ImageSimResult, path: &Path) -> Option<Result<Saved>> {
        let ext = path
            .extension()
            .and_then(|x| x.to_str())
            .map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some("json") => {
                let mut v = json!(out);
                v["image"] = json!(out.full);
                Some(Ok(Saved::Json(v)))
            }
            Some("png") => Some(
                lensbench_export::image::write_png16_channels(
                    path, out.width, out.height, &out.full,
                )
                .map_err(|e| Error::Io(format!("{}: {e}", path.display())))
                .map(|_| {
                    let mut v = json!(out);
                    v["written"] = json!(path.display().to_string());
                    Saved::Written(v)
                }),
            ),
            _ => Some(Err(Error::Invalid(format!(
                "{}: the extension must be png or json",
                path.display()
            )))),
        }
    }
}
