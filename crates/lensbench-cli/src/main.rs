//! lensbench のコマンドライン。視野・波長の番号は OpticStudio と同じく 1 始まり。

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use lensbench_core::analysis::{self, ZernKind};
use lensbench_core::{report, zmx, FieldSel, System};
use serde_json::{json, Value};

#[derive(Parser)]
#[command(name = "lensbench", version = lensbench_core::VERSION, about = "順次光線追跡と解析")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args)]
struct Common {
    /// .zmx ファイル
    file: PathBuf,
    /// ガラスカタログを探すフォルダ（複数可。LENSBENCH_GLASSCAT も使う）
    #[arg(long = "glass-dir")]
    glass_dir: Vec<PathBuf>,
    /// マルチコンフィグの番号（1 始まり）
    #[arg(long, default_value_t = 1)]
    cfg: usize,
    /// JSON で出力する
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct Sel {
    /// 視野の番号（1 始まり、省略で全視野）
    #[arg(long)]
    field: Option<usize>,
    /// 波長の番号（1 始まり、省略で多色）
    #[arg(long)]
    wave: Option<usize>,
}

#[derive(Subcommand)]
enum Cmd {
    /// システムデータ（EFL、F 値、瞳など）
    Info(Common),
    /// スポット（RMS・GEO 半径）
    Spot {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// ガウス求積のリング数
        #[arg(long, default_value_t = 6)]
        rings: usize,
    },
    /// 波面収差（主光線基準の OPD、P-V・RMS [波]）
    Wavefront {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// 格子の一辺
        #[arg(long, default_value_t = 64)]
        n: usize,
    },
    /// Zernike 係数 [波]
    Zernike {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// standard または fringe
        #[arg(long, default_value = "standard")]
        kind: String,
        /// 項数
        #[arg(long, default_value_t = 37)]
        terms: usize,
        /// 格子の一辺
        #[arg(long, default_value_t = 64)]
        n: usize,
    },
}

fn load(c: &Common) -> Result<System, String> {
    if c.cfg == 0 {
        return Err("--cfg は 1 以上".into());
    }
    let opt = zmx::LoadOptions {
        cfg: c.cfg,
        catalog_dirs: c.glass_dir.clone(),
        ..Default::default()
    };
    zmx::load_with(&c.file, &opt).map_err(|e| e.to_string())
}

fn fields(s: &System, sel: &Sel) -> Result<Vec<usize>, String> {
    match sel.field {
        None => Ok((0..s.fields.len()).collect()),
        Some(i) if i >= 1 && i <= s.fields.len() => Ok(vec![i - 1]),
        Some(i) => Err(format!("視野 {i} はありません（1〜{}）", s.fields.len())),
    }
}

fn wave(s: &System, sel: &Sel) -> Result<Option<usize>, String> {
    match sel.wave {
        None => Ok(None),
        Some(i) if i >= 1 && i <= s.wavelengths.len() => Ok(Some(i - 1)),
        Some(i) => Err(format!(
            "波長 {i} はありません（1〜{}）",
            s.wavelengths.len()
        )),
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let e = |e: lensbench_core::Error| e.to_string();
    let (json_out, out): (bool, Value) = match cli.cmd {
        Cmd::Info(c) => {
            let s = load(&c)?;
            let d = report::system_data(&s);
            (
                c.json,
                json!({
                    "name": s.name, "wavelengths_um": s.wavelengths, "primary": s.primary + 1,
                    "fields": s.fields.iter().map(|f| [f.x, f.y]).collect::<Vec<_>>(),
                    "effl": d.effl, "epd": d.epdi, "enpp": d.enpp, "exp_diameter": d.expd,
                    "expp": d.expp, "wfno": d.wfno, "paraxial_wfno": d.pwfn, "image_fno": d.isfn,
                    "paraxial_image_height": d.pimh, "paraxial_mag": d.pmag,
                    "total_track": d.totr, "object_na": d.obna,
                }),
            )
        }
        Cmd::Spot { c, s: sel, rings } => {
            let s = load(&c)?;
            let w = wave(&s, &sel)?;
            let mut v = Vec::new();
            for i in fields(&s, &sel)? {
                let sp = analysis::spot(&s, FieldSel::Index(i), w, rings).map_err(e)?;
                v.push(json!({
                    "field": i + 1, "rms_centroid": sp.rms_centroid, "rms_chief": sp.rms_chief,
                    "geo": sp.geo, "centroid": [sp.centroid.0, sp.centroid.1],
                    "chief": [sp.chief.0, sp.chief.1],
                }));
            }
            (c.json, json!({ "unit": "lens", "spot": v }))
        }
        Cmd::Wavefront { c, s: sel, n } => {
            let s = load(&c)?;
            let w = wave(&s, &sel)?;
            let mut v = Vec::new();
            for i in fields(&s, &sel)? {
                let wf = analysis::wavefront(&s, FieldSel::Index(i), w, n, false).map_err(e)?;
                v.push(json!({ "field": i + 1, "pv": wf.pv, "rms": wf.rms }));
            }
            (c.json, json!({ "unit": "waves", "wavefront": v }))
        }
        Cmd::Zernike {
            c,
            s: sel,
            kind,
            terms,
            n,
        } => {
            let k = match kind.to_ascii_lowercase().as_str() {
                "standard" => ZernKind::Standard,
                "fringe" => ZernKind::Fringe,
                _ => return Err(format!("--kind は standard か fringe（{kind}）")),
            };
            if k == ZernKind::Fringe && terms > 37 {
                return Err("Fringe は 37 項まで".into());
            }
            let s = load(&c)?;
            let w = wave(&s, &sel)?;
            let mut v = Vec::new();
            for i in fields(&s, &sel)? {
                let z =
                    analysis::zernike(&s, FieldSel::Index(i), w, k, terms, n, false).map_err(e)?;
                v.push(json!({
                    "field": i + 1, "coef": z.coef, "pv": z.pv, "rms_chief": z.rms_chief,
                    "rms_centroid": z.rms_centroid, "rms_fit": z.rms_fit, "strehl": z.strehl,
                }));
            }
            (
                c.json,
                json!({ "unit": "waves", "kind": kind, "zernike": v }),
            )
        }
    };
    if json_out {
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        print_text(&out, 0);
    }
    Ok(())
}

/// JSON をそのまま読める形で出す（GUI ができるまでの簡易表示）
fn print_text(v: &Value, ind: usize) {
    let pad = "  ".repeat(ind);
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match x {
                    Value::Object(_) | Value::Array(_) if !is_flat(x) => {
                        println!("{pad}{k}:");
                        print_text(x, ind + 1);
                    }
                    _ => println!("{pad}{k}: {}", fmt(x)),
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                println!("{pad}-");
                print_text(x, ind + 1);
            }
        }
        _ => println!("{pad}{}", fmt(v)),
    }
}

fn is_flat(v: &Value) -> bool {
    match v {
        Value::Array(a) => a.iter().all(|x| x.is_number() || is_flat_pair(x)),
        _ => false,
    }
}

fn is_flat_pair(v: &Value) -> bool {
    matches!(v, Value::Array(a) if a.iter().all(Value::is_number))
}

fn fmt(v: &Value) -> String {
    match v {
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            if n.is_i64() || n.is_u64() {
                n.to_string()
            } else {
                format!("{f:.6e}")
            }
        }
        Value::Array(a) => format!("[{}]", a.iter().map(fmt).collect::<Vec<_>>().join(", ")),
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(m) => {
            eprintln!("lensbench: {m}");
            ExitCode::FAILURE
        }
    }
}
