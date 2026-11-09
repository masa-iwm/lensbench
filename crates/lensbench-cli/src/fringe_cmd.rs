//! `lensbench fringe`: 単色の干渉縞（core の `fringe`）

use std::path::{Path, PathBuf};

use clap::Args;
use lensbench_core::coating::Coatings;
use lensbench_core::fringe::{self, DetectorOpts, Polarized, PupilOpts};
use lensbench_core::raygrid::InputPol;
use lensbench_core::{FieldSel, System};
use lensbench_export::image::{self, ImageFormat};
use serde_json::{json, Value};

#[derive(Args)]
pub struct FringeArgs {
    /// .zmx ファイル（多重構成）か、ブロックで組んだプロジェクトファイル（.json）
    pub file: PathBuf,
    #[arg(long = "glass-dir")]
    pub glass_dir: Vec<PathBuf>,
    /// プロジェクトファイルのカタログレンズを探すレンズカタログ
    #[arg(long = "lens-catalog")]
    pub lens_catalog: Vec<PathBuf>,
    /// プロジェクトファイルのビームスプリッターの経路（BS を通る順の T・R）
    #[arg(long)]
    pub path: Option<String>,
    /// 光束 1（試料）: .zmx は構成の番号（1 始まり）、プロジェクトファイルは状態の名前
    #[arg(long, default_value = "1")]
    pub beam1: String,
    /// 光束 2（参照）: 同上。0 か ref で参照球（--mode pupil だけ）
    #[arg(long, default_value = "2")]
    pub beam2: String,
    /// pupil（瞳上）か detector（像面上）
    #[arg(long, default_value = "pupil")]
    pub mode: String,
    /// 視野の番号（1 始まり）
    #[arg(long, default_value_t = 1)]
    pub field: usize,
    /// 波長の番号（1 始まり、省略で主波長）
    #[arg(long)]
    pub wave: Option<usize>,
    /// 瞳上: 格子の一辺（4 以上の偶数）
    #[arg(long, default_value_t = 64)]
    pub n: usize,
    /// 瞳上: OPD の差にかける数（ダブルパスなら 2）
    #[arg(long, default_value_t = 1.0)]
    pub scale: f64,
    /// 瞳上: 傾きの縞の数（瞳の直径あたり）
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    pub xtilt: f64,
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    pub ytilt: f64,
    /// 像面上: 瞳の光線の格子の一辺
    #[arg(long, default_value_t = 65)]
    pub pupil: usize,
    /// 像面上: 画素の数（一辺）
    #[arg(long, default_value_t = 256)]
    pub npx: usize,
    /// 像面上: 範囲の中心 x,y [mm]（省略で 2 つの光束が届く範囲の中心）
    #[arg(long, allow_hyphen_values = true)]
    pub center: Option<String>,
    /// 像面上: 範囲の一辺 [mm]（省略で 2 つの光束が届く範囲を覆う）
    #[arg(long)]
    pub width: Option<f64>,
    /// 像面上: 試料・参照の強度にかける数
    #[arg(long = "sample-power", default_value_t = 1.0)]
    pub sample_power: f64,
    #[arg(long = "reference-power", default_value_t = 1.0)]
    pub reference_power: f64,
    /// 像面上: 参照の経路に足す光路長 [mm]
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    pub delay: f64,
    /// 像面上: 偏光追跡の場で重ねる（入射偏光は --sample-input・--reference-input、膜は系の膜ファイル）
    #[arg(long)]
    pub polarized: bool,
    #[arg(long = "sample-input", default_value = "x")]
    pub sample_input: String,
    #[arg(long = "reference-input", default_value = "x")]
    pub reference_input: String,
    /// 書き出す量: 瞳上は intensity・delta、像面上は intensity・opd・visibility・sample・reference
    #[arg(long, default_value = "intensity")]
    pub quantity: String,
    /// 書き出すファイル（csv・json・tif・png）
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// tif を 32 bit float で
    #[arg(long)]
    pub float: bool,
    /// 結果の JSON に格子も入れる（"grid"、番号 j·nx + i、行 j は +y の向き（j = 0 が下）、データの無い画素は null。GUI の図）
    #[arg(long)]
    pub grid: bool,
}

fn reference_sphere(b: &str) -> bool {
    matches!(
        b.trim().to_ascii_lowercase().as_str(),
        "0" | "ref" | "reference"
    )
}

/// 計算して要約を返し、--out に書く。`load` は光束の名前から系を読む
pub fn run(a: &FringeArgs, load: &dyn Fn(&str) -> Result<System, String>) -> Result<Value, String> {
    let e = |e: lensbench_core::Error| e.to_string();
    let s1 = load(&a.beam1)?;
    let s2 = if reference_sphere(&a.beam2) {
        None
    } else {
        Some(load(&a.beam2)?)
    };
    if a.field == 0 || a.field > s1.fields.len() {
        return Err(format!(
            "視野 {} はありません（1〜{}）",
            a.field,
            s1.fields.len()
        ));
    }
    let field = FieldSel::Index(a.field - 1);
    let w = match a.wave {
        None => None,
        Some(k) if k >= 1 && k <= s1.wavelengths.len() => Some(k - 1),
        Some(k) => {
            return Err(format!(
                "波長 {k} はありません（1〜{}）",
                s1.wavelengths.len()
            ))
        }
    };
    match a.mode.as_str() {
        "pupil" => {
            let o = PupilOpts {
                n: a.n,
                scale: a.scale,
                xtilt: a.xtilt,
                ytilt: a.ytilt,
            };
            let r = fringe::interferogram(&s1, s2.as_ref(), field, w, &o).map_err(e)?;
            let n = r.p.len();
            let grid: Vec<f64> = match a.quantity.as_str() {
                "intensity" => r.intensity.concat(),
                "delta" => r.delta.concat(),
                q => return Err(format!("--quantity {q}（瞳上は intensity・delta）")),
            };
            let summary = json!({
                "mode": "pupil", "quantity": a.quantity, "nx": n, "ny": n,
                "pupil_min": r.p[0], "pupil_step": r.p[1] - r.p[0],
                "beam1": a.beam1, "beam2": s2.as_ref().map(|_| a.beam2.clone()),
                "scale": a.scale, "xtilt": a.xtilt, "ytilt": a.ytilt,
                "pv_waves": r.pv, "rms_waves": r.rms,
            });
            write(a, &grid, n, n, &summary)?;
            Ok(with_grid(a, summary, &grid))
        }
        "detector" => {
            let Some(s2) = s2 else {
                return Err("--mode detector では --beam2 に構成か状態を指定する".into());
            };
            let center = match &a.center {
                None => None,
                Some(t) => {
                    let v: Vec<f64> = t
                        .split(',')
                        .map(|x| x.trim().parse::<f64>())
                        .collect::<Result<_, _>>()
                        .map_err(|_| format!("--center {t}（x,y）"))?;
                    if v.len() != 2 {
                        return Err(format!("--center {t}（x,y）"));
                    }
                    Some((v[0], v[1]))
                }
            };
            let o = DetectorOpts {
                pupil: a.pupil,
                npx: a.npx,
                center,
                width: a.width,
                sample_power: a.sample_power,
                reference_power: a.reference_power,
                delay: a.delay,
            };
            let coat;
            let pol = if a.polarized {
                coat = Coatings::for_system(&s1).map_err(e)?;
                Some(Polarized {
                    sample: InputPol::parse(&a.sample_input).map_err(e)?,
                    reference: InputPol::parse(&a.reference_input).map_err(e)?,
                    coat: &coat,
                })
            } else {
                None
            };
            let r = fringe::detector(&s1, &s2, field, w, &o, pol).map_err(e)?;
            let grid: Vec<f64> = match a.quantity.as_str() {
                "intensity" => r.intensity.clone(),
                "opd" => r.opd.clone(),
                "visibility" => r.visibility.clone(),
                "sample" => r.i_sample.clone(),
                "reference" => r.i_reference.clone(),
                q => {
                    return Err(format!(
                        "--quantity {q}（像面上は intensity・opd・visibility・sample・reference）"
                    ))
                }
            };
            let both = (0..r.n * r.n)
                .filter(|&q| r.ok_sample[q] && r.ok_reference[q])
                .count();
            let vis: Vec<f64> = r
                .visibility
                .iter()
                .cloned()
                .filter(|v| v.is_finite())
                .collect();
            let summary = json!({
                "mode": "detector", "quantity": a.quantity, "nx": r.n, "ny": r.n,
                "pixel_mm": r.dx, "first_pixel_center_mm": [r.x0, r.y0],
                "top_left_pixel_center_mm": [r.x0, r.y0 + (r.n as f64 - 1.0) * r.dx],
                "wavelength_um": r.wavelength, "beam1": a.beam1, "beam2": a.beam2,
                "overlap_pixels": both,
                "visibility_mean": if vis.is_empty() { f64::NAN } else { vis.iter().sum::<f64>() / vis.len() as f64 },
                "fold_sample": r.fold.0, "fold_reference": r.fold.1,
                "polarized": a.polarized, "delay_mm": a.delay,
            });
            write(a, &grid, r.n, r.n, &summary)?;
            Ok(with_grid(a, summary, &grid))
        }
        m => Err(format!("--mode {m}（pupil・detector）")),
    }
}

/// --grid なら結果の JSON に格子を入れる（NaN は null）
fn with_grid(a: &FringeArgs, mut summary: Value, grid: &[f64]) -> Value {
    if a.grid {
        summary["grid"] = json!(grid);
    }
    summary
}

/// 格子（番号 j·nx + i、行 j は +y の向き）を --out に書く
fn write(
    a: &FringeArgs,
    grid: &[f64],
    nx: usize,
    ny: usize,
    summary: &Value,
) -> Result<(), String> {
    let Some(p) = &a.out else {
        return Ok(());
    };
    write_grid(p, a.float, grid, nx, ny, summary)
}

fn write_grid(
    p: &Path,
    float: bool,
    grid: &[f64],
    nx: usize,
    ny: usize,
    summary: &Value,
) -> Result<(), String> {
    let io = |x: std::io::Error| format!("{}: {x}", p.display());
    let ext = p
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "csv" => {
            let mut s = String::new();
            for j in 0..ny {
                let row: Vec<String> = grid[j * nx..(j + 1) * nx]
                    .iter()
                    .map(|v| {
                        if v.is_nan() {
                            "nan".into()
                        } else {
                            format!("{v:?}")
                        }
                    })
                    .collect();
                s += &row.join(",");
                s.push('\n');
            }
            lensbench_core::vfs::write(p, s).map_err(io)
        }
        "json" => {
            let mut v = summary.clone();
            v["grid"] = json!(grid
                .iter()
                .map(|x| if x.is_nan() { None } else { Some(*x) })
                .collect::<Vec<_>>());
            lensbench_core::vfs::write(p, serde_json::to_string(&v).unwrap()).map_err(io)
        }
        _ => {
            let fmt = ImageFormat::from_path(p, float)
                .ok_or_else(|| format!("--out の拡張子 {ext}（csv・json・tif・png）"))?;
            let (lo, hi) = image::data_range(grid);
            let mut side = summary.clone();
            side["file"] = json!(p.file_name().map(|x| x.to_string_lossy().to_string()));
            side["values"] = json!(if fmt == ImageFormat::TiffFloat {
                "grid values, NaN = no data".to_string()
            } else {
                format!("linear: 0 = {lo:?}, 65535 = {hi:?}; no data = 0")
            });
            side["rows"] = json!("row 0 = top = largest y; columns -> +x");
            let desc = serde_json::to_string_pretty(&side).unwrap();
            let soft = format!("lensbench {}", lensbench_core::VERSION);
            match fmt {
                ImageFormat::TiffFloat => {
                    let mut f = vec![0f32; nx * ny];
                    for row in 0..ny {
                        for i in 0..nx {
                            f[row * nx + i] = grid[(ny - 1 - row) * nx + i] as f32;
                        }
                    }
                    image::write_tiff(p, nx, ny, None, Some(&f), &desc, &soft).map_err(io)?;
                }
                _ => {
                    let u = image::to_u16(grid, nx, ny, Some((lo, hi)));
                    if fmt == ImageFormat::Png16 {
                        image::write_png16(p, nx, ny, &u).map_err(io)?;
                    } else {
                        image::write_tiff(p, nx, ny, Some(&u), None, &desc, &soft).map_err(io)?;
                    }
                }
            }
            image::write_sidecar(p, &(desc + "\n")).map_err(io)
        }
    }
}
