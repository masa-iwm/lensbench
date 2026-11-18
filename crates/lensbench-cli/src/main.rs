//! lensbench のコマンドライン。視野・波長の番号は OpticStudio と同じく 1 始まり。
//!
//! 引数の解釈・文字の出力・serve の要求を運ぶ道（TCP・パイプ）を持ち、解析・系の読み方・serve の場は lensbench-engine。
//! 実行ファイルの入口は `src/bin/lensbench.rs` で、[`main`] を呼ぶだけ（非公開版などは拡張の解析を
//! [`lensbench_engine::register`] してから呼ぶ）。
#![allow(clippy::unwrap_used, clippy::panic)] // 入口の exe（計算の中身のクレートは検査の内）

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use lensbench_core::{catalog::LensCatalog, coating, glass, zmx, System};
use lensbench_export::{d2, mesh, scene};
use lensbench_formats::zmx_write;
use serde_json::{json, Value};

mod analysis_cmd;
mod error;
mod help_json;
mod progress;
mod text_out;
pub(crate) use error::AppError;
mod coat_cmd;
use coat_cmd::{coat_compact, coat_design, coat_kinks, coat_model, coatings};
#[cfg(not(target_arch = "wasm32"))]
use lensbench_cad::{self as cad, coat_remote};
pub mod ext;
mod run;
mod serve;
mod serve_ipc;
mod watch;

use lensbench_design::dls_spec::{Jac, Merit, OptimizeParams, Reference, VarSpec};
use lensbench_engine::layout::{self, Draw, Pick, RaySel, View};
use lensbench_engine::load::{coat_interfaces, plugins_json, save_value, AutoCoat, CoatInterfaces};
use lensbench_engine::{builtin_plugins, provenance};

/// 最適化の設定の既定値（旗の既定はここから。設定の型 1 か所）
static OPT: std::sync::LazyLock<OptimizeParams> = std::sync::LazyLock::new(OptimizeParams::default);
/// レイアウトの書き出しの既定値（serve の layout_export と同じ）
static D2: std::sync::LazyLock<d2::Options2D> = std::sync::LazyLock::new(d2::Options2D::default);
static D3: std::sync::LazyLock<mesh::Options3D> =
    std::sync::LazyLock::new(mesh::Options3D::default);
mod clap_ja;
mod search_cmd;

#[derive(Parser)]
#[command(name = "lensbench", version = lensbench_core::VERSION, about = "Sequential ray tracing and analysis")]
struct Cli {
    /// Number of threads for tracing and analysis (1 runs serially; if omitted, LENSBENCH_THREADS, otherwise RAYON_NUM_THREADS on all cores). Results do not depend on the thread count
    #[arg(long, global = true)]
    threads: Option<usize>,
    #[command(subcommand)]
    cmd: Cmd,
}

/// 系を読む解析の共通の引数（拡張のサブコマンドも `#[command(flatten)]` で使う、[`ext`]）
#[derive(Args, Clone)]
pub struct Common {
    /// A .zmx file, or a project file (.json) built from blocks
    pub file: PathBuf,
    /// Folders to search for glass catalogs (more than one allowed; LENSBENCH_GLASSCAT is also used)
    #[arg(long = "glass-dir")]
    pub glass_dir: Vec<PathBuf>,
    /// Lens catalogs for the catalog lenses ("vendor", "part") of a project file (.zmx files, or folders of .zmx; the folder name is the maker and the file name is the part number; more than one allowed)
    #[arg(long = "lens-catalog")]
    pub lens_catalog: Vec<PathBuf>,
    /// Multi-configuration number (from 1)
    #[arg(long, default_value_t = 1)]
    pub cfg: usize,
    /// State name of a project file (.json) (like a multi-configuration)
    #[arg(long)]
    pub state: Option<String>,
    /// Beam splitter path of a project file (T and R in the order the BSs are passed, e.g. TR; if omitted, the project's path and each BS's arm)
    #[arg(long = "beam-path")]
    pub path: Option<String>,
    /// Output JSON
    #[arg(long)]
    pub json: bool,
    /// File to write the result to (csv, json or npz by extension)
    #[arg(long)]
    pub save: Option<PathBuf>,
    /// Stop with an error when a ZPL macro solve of a .zmx can't be solved (by default the value is the same as OpticStudio's and a warning is shown)
    #[arg(long = "strict-macros")]
    pub strict_macros: bool,
}

impl Common {
    /// 系の読み方
    pub fn spec(&self) -> lensbench_engine::SysSpec {
        lensbench_engine::SysSpec {
            file: self.file.clone(),
            glass_dir: self.glass_dir.clone(),
            lens_catalog: self.lens_catalog.clone(),
            cfg: self.cfg,
            state: self.state.clone(),
            path: self.path.clone(),
            strict_macros: self.strict_macros,
            ..Default::default()
        }
    }
}

/// 重ね描き・多重構成の書き出しで並べる構成
#[derive(Args, Clone, Default)]
pub(crate) struct ConfigSel {
    /// .zmx multi-configurations (from 1, comma-separated, all for every one)
    #[arg(long)]
    cfgs: Option<String>,
    /// Project states (comma-separated names, all for every one)
    #[arg(long)]
    states: Option<String>,
    /// Beam splitter paths (comma-separated strings of T and R, all for every possible path)
    #[arg(long = "beam-paths")]
    paths: Option<String>,
}

impl ConfigSel {
    fn overlay(&self) -> lensbench_engine::load::Overlay {
        lensbench_engine::load::Overlay {
            cfgs: self.cfgs.clone(),
            states: self.states.clone(),
            paths: self.paths.clone(),
        }
    }
}

/// 重ね描きする構成の系と名前。重ね描きでなければ今の系 1 つ（名前は空）。読んだときの注意は標準エラーへ
fn overlay_systems(c: &Common, l: &ConfigSel) -> lensbench_core::Result<Vec<(String, System)>> {
    let (systems, notes) = lensbench_engine::load::overlay_systems(&c.spec(), &l.overlay())?;
    for w in notes {
        eprintln!("{}", lensbench_core::i18n::warning(&w));
    }
    Ok(systems)
}

#[derive(Subcommand)]
enum Cmd {
    /// Write the system
    ///
    /// A .json extension writes a system file (the complete lensbench format), .zmx writes .zmx. Reads .zmx, projects and system files (with --state and --cfg applied)
    Convert {
        #[command(flatten)]
        c: Common,
        /// File to write (.json or .zmx)
        #[arg(long, short)]
        out: PathBuf,
    },
    /// Write the system to .zmx (an expanded project, etc.)
    ///
    /// With --beam-paths, --states or --cfgs, put one configuration per item into a multi-configuration .zmx (per BS path, etc.; surfaces that differ are ignored with IGNR)
    Zmx {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        sel: ConfigSel,
        /// .zmx to write
        #[arg(long, short)]
        out: PathBuf,
    },
    /// Optimize variables with damped least squares (DLS)
    ///
    /// The merit function is the RMS spot or RMS wavefront over all fields (field weights) and wavelengths, plus the terms of --term and --terms. --out writes the optimized system to .zmx
    Optimize {
        #[command(flatten)]
        c: Common,
        /// Variables ("7.t", "lens2.tx", "G1.shift", etc.; a range is "7.t=0.5:3", one-sided "7.t=0.5:" or "7.t=:3"). More than one allowed
        #[arg(long = "var", required = true)]
        vars: Vec<String>,
        /// Default merit function: spot, wavefront or none (only the --term terms)
        #[arg(long, default_value_t = OPT.merit)]
        merit: Merit,
        /// Terms to add (JSON, e.g. '{"type":"paraxial","q":"efl","target":50,"weight":10}'; see the optimization terms in the docs for kinds and keys). More than one allowed
        #[arg(long = "term")]
        terms: Vec<String>,
        /// JSON file of terms to add (an array of terms)
        #[arg(long = "terms")]
        terms_file: Option<PathBuf>,
        /// Reference: centroid or chief (chief ray)
        #[arg(long = "reference", default_value_t = OPT.reference)]
        reference: Reference,
        /// Number of rings of the Gaussian pupil quadrature
        #[arg(long, default_value_t = OPT.rings)]
        rings: usize,
        /// Maximum number of iterations
        #[arg(long, default_value_t = OPT.iters)]
        iters: usize,
        /// Jacobian: auto (analytic for terms with 16 or more analytic columns, central differences for fewer), fd (all central differences) or forward (all forward differences)
        #[arg(long, default_value_t = OPT.jac)]
        jac: Jac,
        #[command(flatten)]
        dls: search_cmd::DlsArgs,
        /// .zmx to write the optimized system to
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Monte Carlo tolerance simulation
    ///
    /// Perturb the system with per-lens tolerance sets (commercial by default; makers' tolerances for catalog lenses) and individual tolerances, re-adjust the compensators, and give the distribution and sensitivity of the merit function (RMS wavefront, spot, MTF, etc.). Settings are JSON (tolerance settings in the docs; the --spec file and --set overrides)
    Tolerance {
        #[command(flatten)]
        c: Common,
        /// Settings JSON file
        #[arg(long)]
        spec: Option<PathBuf>,
        /// Settings override (a JSON object, e.g. '{"preset":"precision","compensators":["9.t"]}'). More than one allowed
        #[arg(long = "set")]
        set: Vec<String>,
        /// Number of trials (takes precedence over trials in the settings)
        #[arg(long)]
        trials: Option<usize>,
        /// Random seed (takes precedence over seed in the settings)
        #[arg(long)]
        seed: Option<u64>,
        /// File to write the system of the trial with the worst merit (after compensation) to. .json writes a system file (index tolerances included), others .zmx (index and dispersion shifts can't be written to .zmx)
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// CSV file to write the compensators of each trial to (with the merit and the size of the x and y tilt pair [arcmin])
        #[arg(long = "comp-csv")]
        comp_csv: Option<PathBuf>,
        /// JSONL file to write each evaluation (sensitivity and trials) to as it finishes. If the file is from the same calculation, continue from it (only the missing evaluations run; more --trials can be added)
        #[arg(long)]
        results: Option<PathBuf>,
        /// With --results, start over instead of continuing from the file
        #[arg(long = "no-resume")]
        no_resume: bool,
    },
    /// Zoom trajectory
    ///
    /// Solve the group positions (--var) for each state (the project's states or the .zmx configurations). The merit is the image RMS (all fields, all wavelengths) and the --efl and --pmag targets. --steps follows the path between states by continuation
    Zoom(Box<search_cmd::ZoomArgs>),
    /// Catalog lens swap search
    ///
    /// Run the combinations that replace the --spec slots (lens blocks) with candidates in parallel, optimize the variables (spacings, etc.) for each, and sort them by merit
    Swap(Box<search_cmd::SwapArgs>),
    /// Save a lens or block to a user catalog (JSON) (the file is created if missing)
    CatalogAdd {
        /// User catalog file
        catalog: PathBuf,
        /// Item name
        #[arg(long)]
        name: String,
        /// A .zmx or project file to take the lens from (with --lens), or a project file (with --block)
        #[arg(long)]
        from: PathBuf,
        /// Lens number (from 1, counting only lenses; the .zmx or project split in lens mode)
        #[arg(long)]
        lens: Option<usize>,
        /// Project block ids (two give the range between them in the top-level list; surface table changes in the blocks' surface_edits are baked in)
        #[arg(long, num_args = 1)]
        block: Vec<String>,
        /// Folders to search for glass catalogs (more than one allowed)
        #[arg(long = "glass-dir")]
        glass_dir: Vec<PathBuf>,
        /// Lens catalogs to search for the project's catalog lenses (same as info, etc.)
        #[arg(long = "lens-catalog")]
        lens_catalog: Vec<PathBuf>,
        /// .zmx multi-configuration number
        #[arg(long, default_value_t = 1)]
        cfg: usize,
        /// Description of the item
        #[arg(long, default_value = "")]
        description: String,
        /// Replace an item of the same name
        #[arg(long)]
        replace: bool,
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
    /// Search catalog items
    ///
    /// Filter the items of lens catalogs (ZMF, .zmx, lensbench .json) by kind, maker, words and specs. Specs are the items' `specs` in .json plus those added by `overlay` and `url_template` in a .json of the same maker (efl, diameter and thickness can also be compared with the listed values)
    CatalogSearch {
        /// Lens catalogs (files or folders, more than one allowed)
        #[arg(required = true)]
        lens_catalog: Vec<PathBuf>,
        /// Folders to search for glass catalogs (used to find lens focal lengths)
        #[arg(long = "glass-dir")]
        glass_dir: Vec<PathBuf>,
        /// Item kinds (lens, mirror, splitter, polarizer, waveplate, window, filter, prism, other; more than one allowed)
        #[arg(long, num_args = 1)]
        kind: Vec<String>,
        /// Maker (contains; case, spaces and symbols are ignored)
        #[arg(long, default_value = "")]
        vendor: String,
        /// Words in the part number, description, tags or specs (separated by spaces, all must be present)
        #[arg(long, default_value = "")]
        text: String,
        /// Spec conditions (more than one allowed, all must hold): `name` (present), `name=value` (numbers equal, range specs contain, strings contain), `name=a..b`, `name>=value`, `<=`, `>`, `<` (range specs by overlap). E.g. `wavelength_nm=633`, `extinction_ratio>=1000`
        #[arg(long, num_args = 1)]
        spec: Vec<String>,
        /// Only mechanical parts (tubes, mounts, cage plates) that hold an element of diameter [mm]: holding diameter at least the diameter and no more than 0.25 mm larger
        #[arg(long)]
        fits: Option<f64>,
        /// List the specs with fixed names
        #[arg(long)]
        list_specs: bool,
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
    /// Measure an item's STEP (.step, .stp)
    ///
    /// Thickness and diameter; glass and frame (`frame`) for mounted items; mechanical parts without glass (tubes, cage plates, mounts) as a whole frame (`mech`; `frame` with the bore, length, outer diameter, threads, rod spacing and the element held)
    CatalogStep {
        /// STEP files to measure (.step, .stp)
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
    /// Fetch one detailed STEP of an item from its official source (the item's `cad.step`) and put it in the cache
    ///
    /// From the second time the cache is used. The fetched STEP is measured like catalog-step and differences from the catalog's simple outline (`mech` length, outer diameter and shape) are shown
    CadStep {
        /// User catalog
        #[arg(long)]
        catalog: PathBuf,
        /// Name of the item
        #[arg(long)]
        item: String,
        /// Cache folder (if omitted, the environment variable LENSBENCH_CAD_CACHE, otherwise lensbench/cad in the OS settings location)
        #[arg(long)]
        cache: Option<PathBuf>,
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
    /// List the items of a user catalog
    CatalogList {
        /// User catalog file
        catalog: PathBuf,
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
    /// List the coatings of the surfaces
    ///
    /// Interfaces (coating|medium before|medium after) of all configurations and how they are handled. Also shows the interfaces that need a measured table (encrypted coatings, etc.)
    Coatings {
        /// .zmx file
        file: PathBuf,
        /// Folders to search for glass catalogs (more than one allowed)
        #[arg(long = "glass-dir")]
        glass_dir: Vec<PathBuf>,
        /// Measured coating tables and models (more than one allowed; same as --coatings of ghost and the analyses)
        #[arg(long)]
        coatings: Vec<PathBuf>,
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
    /// Find the kinks along wavelength (nodes of the dispersion tables of the coating materials) from measured tables (LBCT)
    ///
    /// Measure the wavelength grid about 4 times finer than the grid chosen from the coating thickness
    // 保守の道具は一覧の最後に
    #[command(display_order = 900)]
    CoatKinks {
        /// Measured tables (LBCT format, more than one allowed)
        tables: Vec<PathBuf>,
        /// KEYs to check (can be given more than once; by default the transmission from air and the mirror KEYs; on the glass side the residual is large at high β and detection is weak)
        #[arg(long)]
        key: Vec<String>,
        /// Smallest score to take as a kink candidate (how many times the median residual of a smooth fit). Lower finds weaker kinks
        #[arg(long, default_value_t = 100.0)]
        min_score: f64,
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
    /// Write the layout (section or 3D) and rays
    ///
    /// The format is chosen by the --out extension: 2D svg, png or dxf; 3D gltf, glb, obj or stl; step (stp) for CAD
    Layout {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        l: Box<LayoutArgs>,
        /// Files to write (more than one allowed)
        #[arg(long, short, required = true)]
        out: Vec<PathBuf>,
        /// 2D projection plane (yz or xz)
        #[arg(long, default_value = "yz")]
        plane: d2::Plane,
        /// PNG width [px]
        #[arg(long, default_value_t = D2.width)]
        width: u32,
        /// Number of divisions around (3D) and of surface lines (2D)
        #[arg(long, default_value_t = D2.segments)]
        segments: usize,
        /// Number of radial divisions (3D)
        #[arg(long, default_value_t = D3.rings)]
        rings: usize,
        /// Radius of the ray tubes [mm] (3D; for STL, 0.1% of the scene size if omitted)
        #[arg(long)]
        tube: Option<f64>,
        /// Draw mechanical parts with the detailed shape of their fetched STEP (3D glTF, OBJ, STL and STEP; parts with no STEP in the cache get the simple outline; fetch STEPs with lensbench cad-step)
        #[arg(long)]
        mech_detail: bool,
    },
    /// Make an encrypted coating model (LBCM 2) from measured tables (LBCT) taken with a set of model glasses
    // 保守の道具は一覧の最後に
    #[command(display_order = 900)]
    CoatModel(coat_cmd::CoatModelArgs),
    /// Make a coating model (LBCM) smaller
    ///
    /// Write binary (LBCM 3: values packed as integers in steps of 1e-9, 1e-12 for mirrors). Read the same as LBCM 2 (text)
    // 保守の道具は一覧の最後に
    #[command(display_order = 900)]
    CoatCompact {
        /// Original model (LBCM 2 or 3)
        model: PathBuf,
        /// Model to write (LBCM 3)
        #[arg(long, short)]
        out: PathBuf,
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
    /// Fit a multilayer to a spectral curve (reflectance, transmittance) and write an approximate coating (`.lbcs`, the coating file format)
    ///
    /// The curves are CSV (columns wl_um or wl_nm, angle_deg, pol (s, p, avg), quantity (R, T), value or value_pct, weight; angle_deg, pol, quantity and weight default to 0, avg, R and 1). Put in a coating file folder, it is used by the coating name
    CoatDesign {
        /// Curve CSVs (more than one allowed)
        targets: Vec<PathBuf>,
        /// Coating name (the COAT name to write)
        #[arg(long)]
        name: String,
        /// Materials (repeated from the outside, e.g. MGF2,TA2O5): MATE of a coating file, or glasses of a glass catalog
        #[arg(long, value_delimiter = ',')]
        materials: Vec<String>,
        /// Substrate: an index, or a MATE or glass name of a coating file
        #[arg(long)]
        substrate: String,
        /// Coating files to search for materials (COATING.DAT format)
        #[arg(long = "coating-file")]
        coating_file: Option<PathBuf>,
        /// Folders to search for glass catalogs (more than one allowed)
        #[arg(long = "glass-dir")]
        glass_dir: Vec<PathBuf>,
        /// Numbers of layers to try (e.g. 1..12)
        #[arg(long, default_value = "1..12")]
        layers: String,
        /// Number of starting points per number of layers
        #[arg(long, default_value_t = 20)]
        starts: usize,
        /// Maximum layer thickness [µm]
        #[arg(long = "d-max", default_value_t = 0.6)]
        d_max: f64,
        /// Choose the fewest layers within (1 + slack) times the best RMS
        #[arg(long, default_value_t = 0.1)]
        slack: f64,
        /// Approximate coating to write (.lbcs)
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
    /// Keep the system loaded and wait, repeating system changes and analyses over JSON-RPC from outside (127.0.0.1 only by default)
    ///
    /// Steps and format: docs/serve.md
    Serve(serve::ServeArgs),
    /// Watch the system file and run the analysis and write the result each time it changes
    ///
    /// Example: lensbench watch -- spot sys.zmx --json --save out.json
    Watch(watch::WatchArgs),
    /// Run a Python script without the GUI (starts an outside Python and passes glasses, coatings, thread count and output folder)
    ///
    /// Example: lensbench run --threads 4 --out result sweep.py -- --n 100. The exit code is the script's
    Run(Box<run::RunArgs>),
    /// List the bundled plugins (detectors, analyses) and describe their inputs and outputs
    Plugins {
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// Check plugins and analysis parts against reference data
    ///
    /// Give a description (*.verify.json) or a folder containing them. Exit code 1 if any check fails
    Verify {
        /// Description file or folder (folders are searched all the way down for *.verify.json)
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Skip checks with ci: false (run only locally)
        #[arg(long)]
        ci: bool,
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// 解析の登録簿の解析（[`analysis_cmd`]。clap の定義は [`cli_command`] が設定のスキーマから足す）
    #[command(skip)]
    Analysis(Box<analysis_cmd::AnalysisCmd>),
}

/// レイアウトの光線の選び方
#[derive(Args)]
struct LayoutArgs {
    /// Field numbers (from 1, comma-separated; all fields if omitted)
    #[arg(long)]
    fields: Option<String>,
    /// Wavelength numbers (from 1, comma-separated, all for every one; the primary wavelength if omitted)
    #[arg(long)]
    waves: Option<String>,
    /// Ray pattern (y, x, cross, grid, ring). If omitted, y for 2D, ring for 3D and STEP (the chief ray and the pupil edge)
    #[arg(long)]
    fan: Option<scene::Fan>,
    /// Number of rays (rays per direction for a fan or cross, the side for grid, the rays around for ring, plus the chief ray: n+1). If omitted, 7 for y, 12 for ring
    #[arg(long)]
    rays: Option<usize>,
    /// First surface to draw (if omitted, the object surface when a finite object is near, otherwise surface 1)
    #[arg(long)]
    first: Option<usize>,
    /// Last surface to draw (the image surface if omitted)
    #[arg(long)]
    last: Option<usize>,
    /// Ray colors (field, wave, cfg; cfg is per overlaid configuration). If omitted, cfg when overlaying, otherwise field
    #[arg(long)]
    color: Option<d2::ColorBy>,
    /// 重ね描きする構成
    #[command(flatten)]
    sel: ConfigSel,
    /// Offset per configuration dx,dy,dz [mm] (configuration k is shifted k times; same as the OpticStudio layout offset; overlaid by default)
    #[arg(long, allow_hyphen_values = true)]
    offset: Option<String>,
    /// Unfold along the light path (lay the sections after mirrors and BSs along the extension of the axis before the fold; 2D output only)
    #[arg(long)]
    unfold: bool,
}

impl LayoutArgs {
    /// 光線の選び方（既定は serve の layout と同じ [`lensbench_engine::layout`]）
    fn ray_sel(&self) -> lensbench_core::Result<RaySel> {
        let pick = |v: &Option<String>, what: &str| {
            v.as_deref()
                .map(|x| x.parse::<Pick>().map_err(|e| format!("{what} {e}")))
                .transpose()
        };
        Ok(RaySel {
            fields: pick(&self.fields, "field")?,
            waves: pick(&self.waves, "wavelength")?,
            fan: self.fan,
            rays: self.rays,
            first: self.first,
            last: self.last,
            unfold: self.unfold,
        })
    }
}

fn load(c: &Common) -> Result<System, AppError> {
    Ok(lensbench_engine::load::load_file_noted(&c.spec())?)
}

/// カンマ区切りの数
fn parse_list(s: &str, what: &str) -> Result<Vec<f64>, AppError> {
    let v: Vec<f64> = s
        .split(',')
        .filter(|x| !x.trim().is_empty())
        .map(|x| {
            x.trim()
                .parse::<f64>()
                .map_err(|_| AppError::usage(format!("can't read the number of {what} ({x})")))
        })
        .collect::<Result<_, _>>()?;
    if v.is_empty() {
        return Err(AppError::usage(format!("{what} is empty")));
    }
    Ok(v)
}

/// 文字で出すときは長い表を省く（PSF・波面の格子など。--json・--save では全体）
fn brief(v: &Value) -> Value {
    /// 入れ子の配列も数えた要素の数
    fn leaves(v: &Value) -> usize {
        match v {
            Value::Array(a) => a.iter().map(leaves).sum::<usize>().max(1),
            _ => 1,
        }
    }
    match v {
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), brief(x))).collect()),
        Value::Array(a) if leaves(v) > 1000 => {
            json!(lensbench_core::i18n::tr(&format!(
                "({} rows, {} elements; use --json or --save for everything)",
                a.len(),
                leaves(v)
            )))
        }
        Value::Array(a) => Value::Array(a.iter().map(brief).collect()),
        _ => v.clone(),
    }
}

impl Cmd {
    /// 系のファイルを読む解析の共通の引数
    fn common_mut(&mut self) -> Option<&mut Common> {
        match self {
            Cmd::Optimize { c, .. }
            | Cmd::Tolerance { c, .. }
            | Cmd::Convert { c, .. }
            | Cmd::Layout { c, .. } => Some(c),
            Cmd::Analysis(a) => Some(&mut a.c),
            _ => None,
        }
    }

    /// 結果を書き出すファイル（登録簿の解析は自分の形で書くので除く）
    fn save(&self) -> Option<PathBuf> {
        match self {
            Cmd::Optimize { c, .. } | Cmd::Tolerance { c, .. } | Cmd::Layout { c, .. } => {
                c.save.clone()
            }
            Cmd::Zoom(a) => a.save.clone(),
            Cmd::Swap(a) => a.save.clone(),
            _ => None,
        }
    }
}

thread_local! {
    /// 一部が失敗したコマンド（上げ・取り・読みの一部の誤り）の説明。結果は出したうえで終了コード 1 にする
    static PARTIAL_FAILURE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// 一部が失敗したことを記す（[`PARTIAL_FAILURE`]。CLI は結果を出した後に終了コード 1、serve は結果の JSON の errors で）
fn partial_failure(msg: impl Into<String>) {
    PARTIAL_FAILURE.with(|x| *x.borrow_mut() = Some(msg.into()));
}

fn run(cli: Cli) -> Result<(), AppError> {
    match cli.cmd {
        Cmd::Serve(a) => Ok(serve::serve(a, cli.threads)?),
        Cmd::Watch(a) => watch::watch(a),
        Cmd::Verify { paths, ci, json } => verify(&paths, ci, json),
        cmd => {
            let (json_out, out) = execute(cmd)?;
            print_out(json_out, &out);
            match PARTIAL_FAILURE.with(|x| x.borrow_mut().take()) {
                Some(m) => Err(AppError::Failed(m)),
                None => Ok(()),
            }
        }
    }
}

/// 照合: 記述を回して表か JSON を出し、通らなければ誤り
fn verify(paths: &[PathBuf], ci: bool, json_out: bool) -> Result<(), AppError> {
    let opt = lensbench_verify::Options { ci_only: ci };
    let reps = lensbench_verify::run_paths(paths, &builtin_plugins(), &opt)?;
    if json_out {
        let v: Vec<Value> = reps.iter().map(|r| r.to_json()).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "reports": v })).unwrap()
        );
    } else {
        for r in &reps {
            r.to_text().lines().for_each(text_out::say);
        }
    }
    let bad = reps.iter().filter(|r| !r.passed()).count();
    if reps.is_empty() {
        return Err("no verify description (*.verify.json) found".into());
    }
    if bad > 0 {
        return Err(format!("{bad} of {} checks fail", reps.len()).into());
    }
    Ok(())
}

/// catalog-search の表（メーカー・型番・種類・焦点距離・径・仕様・リンク）
fn print_catalog_table(v: &Value) {
    let rows = v["lenses"].as_array().cloned().unwrap_or_default();
    let f = |x: &Value| {
        x.as_f64()
            .map(|x| format!("{x:.4}"))
            .unwrap_or_else(|| "-".into())
    };
    text_out::say(&format!("{} item(s)", rows.len()));
    for r in &rows {
        let specs = r["specs"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| match v {
                        Value::String(t) => format!("{k}={t}"),
                        o => format!("{k}={o}"),
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        println!(
            "{:<14} {:<16} {:<9} efl={:<10} d={:<8} {}{}{}",
            r["vendor"].as_str().unwrap_or(""),
            r["part"].as_str().unwrap_or(""),
            lensbench_catalog::spec::row_category(r),
            f(&r["efl"]),
            f(&r["diameter"]),
            specs,
            r["url"]
                .as_str()
                .map(|u| format!(" {u}"))
                .unwrap_or_default(),
            r["error"]
                .as_str()
                .map(|e| lensbench_core::i18n::tr(&format!(" error: {e}")))
                .unwrap_or_default(),
        );
    }
}

/// 結果を標準出力に出す
fn print_out(json_out: bool, out: &Value) {
    if out.is_null() {
        return;
    }
    let mut w = std::io::BufWriter::new(std::io::stdout().lock());
    let r = if json_out {
        writeln!(w, "{}", serde_json::to_string_pretty(out).unwrap())
    } else {
        text_out::print_text(&mut w, &brief(&text_out::tolerance_brief(out)), 0)
    };
    // 読み手が先に閉じた（`| head` など）ときは黙って終える
    let e = r.and_then(|_| w.flush()).err();
    if let Some(e) = e.filter(|e| e.kind() != std::io::ErrorKind::BrokenPipe) {
        let m = format!("lensbench: output: {e}");
        eprintln!("{}", lensbench_core::i18n::tr(&m));
    }
}

/// 解析のコマンドを 1 つ実行して結果を返す（--save があれば書き出す）。watch・ext::run_cli もこれを使う
fn execute(cmd: Cmd) -> Result<(bool, Value), AppError> {
    let e = AppError::from;
    let save = cmd.save();
    provenance::clear();
    PARTIAL_FAILURE.with(|x| *x.borrow_mut() = None);
    let (json_out, out): (bool, Value) = match cmd {
        Cmd::Serve(_) | Cmd::Watch(_) | Cmd::Run(_) => {
            return Err("serve, watch and run can't be nested".into())
        }
        // 記述の中のパス（基準データ・系）を serve の --root で確かめられないので、コマンドラインからだけ
        Cmd::Verify { .. } => return Err("verify is only available from the command line".into()),
        Cmd::Plugins { json } => (json, help_json::localize_docs(plugins_json(), json)),
        Cmd::Analysis(a) => {
            let s = load(&a.c)?;
            let def = analyses()
                .get(&a.name)
                .cloned()
                .ok_or_else(|| format!("no analysis {}", a.name))?;
            let (out, to_save) = def
                .run_save(
                    &lensbench_analysis::Cx {
                        system: &s,
                        source: Some(&lensbench_engine::source::FileSource::new(
                            &a.c.spec(),
                            None,
                        )),
                    },
                    &a.params,
                    a.c.save.as_deref(),
                )
                .map_err(e)?;
            if let (Some(p), Some(v)) = (&a.c.save, &to_save) {
                save_value(p, v).map_err(AppError::io)?;
            }
            // 注意は結果の warnings・notes にも入る。コマンドラインでは標準エラーにも出す（serve は serve_analyze を通る）
            for k in ["warnings", "notes"] {
                for w in out[k].as_array().into_iter().flatten() {
                    if let Some(w) = w.as_str() {
                        eprintln!("{}", lensbench_core::i18n::warning(w));
                    }
                }
            }
            (a.c.json, out)
        }
        Cmd::Zoom(a) => (a.json, search_cmd::zoom(&a)?),
        Cmd::Swap(a) => (a.json, search_cmd::swap(&a)?),
        Cmd::Convert { c, out } => {
            let s = load(&c)?;
            let zmx = out
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("zmx"));
            if zmx {
                warn_notes(zmx_write::save(&out, &s).map_err(e)?);
            } else {
                lensbench_project::sysfile::save(&out, &s, &[])?;
                warn_notes(lensbench_project::sysfile::save_notes(&s));
            }
            (
                c.json,
                json!({
                    "written": out.display().to_string(),
                    "lensbench": lensbench_core::VERSION,
                    "system_hash": lensbench_project::sysfile::system_hash(&s)?,
                }),
            )
        }
        Cmd::Zmx { c, sel, out } => {
            let systems = overlay_systems(&c, &sel)?;
            let v: Vec<System> = systems.iter().map(|x| x.1.clone()).collect();
            zmx_write::save_configs(&out, &v).map_err(e)?;
            (
                c.json,
                json!({
                    "file": out.display().to_string(),
                    "configs": systems.iter().map(|x| x.0.clone()).collect::<Vec<_>>(),
                }),
            )
        }
        Cmd::Layout {
            c,
            l,
            out,
            plane,
            width,
            segments,
            rings,
            tube,
            mech_detail,
        } => {
            let systems = overlay_systems(&c, &l.sel)?;
            let offset = match &l.offset {
                None => [0.0; 3],
                Some(v) => {
                    layout::offset(&parse_list(v, "--offset")?).map_err(AppError::usage_of)?
                }
            };
            let draw = Draw {
                plane,
                color: l.color,
                width: Some(width),
                segments: Some(segments),
                rings: Some(rings),
                tube,
                mech_detail: mech_detail.then(lensbench_cad::detail::details).flatten(),
            };
            let (o2, o3) = draw
                .options(l.sel.overlay().any())
                .map_err(AppError::usage_of)?;
            let rsel = l.ray_sel()?;
            // 拡張子を先に確かめ、2D 用と 3D 用の場面を要るものだけ作る
            let fmts = out
                .iter()
                .map(|p| {
                    lensbench_export::LayoutFormat::from_path(p).ok_or_else(|| {
                        AppError::usage(format!(
                            "{}: the extension must be svg, png, dxf, gltf, glb, obj, stl, step or stp",
                            p.display()
                        ))
                    })
                })
                .collect::<Result<Vec<_>, AppError>>()?;
            if l.unfold && fmts.iter().any(|f| !f.is_2d()) {
                return Err(AppError::usage("--unfold is only for 2D (svg, png, dxf)"));
            }
            let mut scenes: [Option<scene::Scene>; 2] = [None, None];
            for f in &fmts {
                let k = usize::from(!f.is_2d());
                if scenes[k].is_none() {
                    let mut parts = Vec::new();
                    // 重ね描きでは描き始める面をそろえる（物体面から描けない構成があれば全部面 1 から）
                    let first0 = (l.first.is_none()
                        && systems.iter().any(|x| scene::auto_first(&x.1) != 0))
                    .then_some(1);
                    for (name, sy) in &systems {
                        let view = match (f.is_2d(), plane) {
                            (false, _) => View::D3,
                            (true, d2::Plane::XZ) => View::XZ,
                            (true, d2::Plane::YZ) => View::YZ,
                        };
                        let mut so = rsel.options(sy, view).map_err(e)?;
                        so.first = so.first.or(first0);
                        let sc = scene::build(sy, &so).map_err(|x| {
                            if name.is_empty() {
                                e(x)
                            } else {
                                AppError::Core(x.at(name))
                            }
                        })?;
                        parts.push((name.clone(), sc));
                    }
                    scenes[k] = Some(scene::overlay(parts, offset).map_err(e)?);
                }
            }
            let mut written = Vec::new();
            for (p, f) in out.iter().zip(&fmts) {
                let sc = scenes[usize::from(!f.is_2d())].as_ref().unwrap();
                let w = lensbench_export::write_layout(p, sc, &o2, &o3)?;
                let mut row = json!({ "file": p.display().to_string(), "format": format!("{f:?}").to_lowercase() });
                // 写した品の STEP の長さを直せなかった実体など
                if !w.notes.is_empty() {
                    row["notes"] = json!(w.notes);
                }
                written.push(row);
            }
            let sc = scenes.iter().flatten().next().unwrap();
            let (lo, hi) = sc.bounds();
            (
                c.json,
                json!({
                    "written": written,
                    "configs": sc.configs,
                    "solids": sc.solids.iter().map(|x| x.name.clone()).collect::<Vec<_>>(),
                    "surfaces": sc.plates.iter().map(|x| x.name.clone()).collect::<Vec<_>>(),
                    "mechanics": sc.mechs.iter().map(|x| x.name.clone()).collect::<Vec<_>>(),
                    "rays_2d": scenes[0].as_ref().map(|x| x.rays.len()),
                    "rays_3d": scenes[1].as_ref().map(|x| x.rays.len()),
                    "bounds_mm": [lo, hi],
                }),
            )
        }
        Cmd::Optimize {
            c,
            vars,
            merit,
            terms,
            terms_file,
            reference,
            rings,
            iters,
            jac,
            dls,
            out,
        } => {
            use lensbench_optim as optim;
            let s = load(&c)?;
            let mut specs: Vec<Value> = Vec::new();
            for t in &terms {
                specs.push(
                    serde_json::from_str(t)
                        .map_err(|x| AppError::usage(format!("--term {t}: {x}")))?,
                );
            }
            if let Some(f) = &terms_file {
                let txt = lensbench_core::vfs::read_to_string(f).map_err(AppError::io_at(f))?;
                let v: Value = serde_json::from_str(&txt)
                    .map_err(|x| AppError::usage(format!("{}: {x}", f.display())))?;
                match v {
                    Value::Array(a) => specs.extend(a),
                    v => specs.push(v),
                }
            }
            // serve の中（--root・--allow-remote）なら zplm の file・macro_dirs の場所を確かめる
            for v in specs.iter_mut() {
                lensbench_engine::Files::Any.check_terms(v)?;
            }
            let op = OptimizeParams {
                vars: VarSpec::parse_all(&vars).map_err(AppError::usage_of)?,
                merit,
                terms: specs,
                reference,
                rings,
                iters,
                jac,
                qr: dls.qr,
                gtol: dls.gtol,
                active_set: dls.active_set,
                stopped_as_vignetted: dls.stopped_as_vignetted,
                ..OptimizeParams::default()
            };
            let (p, n_default) = op.problem(vec![s.clone()]).map_err(e)?;
            p.check().map_err(e)?;
            let o = op.dls_opts();
            let r = progress::with(c.json, || optim::dls(&p, &o)).map_err(e)?;
            if let Some(path) = &out {
                warn_notes(zmx_write::save(path, &r.systems[0]).map_err(e)?);
            }
            let stop = r.stop.as_str();
            let vs: Vec<Value> = p
                .vars
                .iter()
                .enumerate()
                .map(|(q, v)| json!({ "name": s.param_name(v.param), "x0": r.x0[q], "x": r.x[q], "inactive": r.inactive.contains(&q) }))
                .collect();
            if let Some(n) = &r.note {
                eprintln!("{}", lensbench_core::i18n::warning(n));
            }
            // 足した項の値（最適化の前と後）
            let tv0 = p.term_values(&r.x0).map_err(e)?;
            let tv = p.term_values(&r.x).map_err(e)?;
            let ts: Vec<Value> = op
                .terms
                .iter()
                .enumerate()
                .map(|(i, v)| json!({ "term": v, "value0": tv0[n_default + i].value(), "value": tv[n_default + i].value(), "value_kind": tv[n_default + i].kind_name() }))
                .collect();
            (
                c.json,
                json!({
                    "merit": merit.as_str(), "merit0": r.merit0, "merit_final": r.merit, "iterations": r.iterations,
                    "evals": r.evals, "stop": stop, "note": r.note, "vars": vs, "terms": ts, "history": r.history,
                }),
            )
        }
        Cmd::Tolerance {
            c,
            spec,
            set,
            trials,
            seed,
            out,
            comp_csv,
            results,
            no_resume,
        } => {
            let s = load(&c)?;
            let mut v = match &spec {
                Some(f) => {
                    let txt = lensbench_core::vfs::read_to_string(f).map_err(AppError::io_at(f))?;
                    serde_json::from_str::<Value>(&txt)
                        .map_err(|x| AppError::usage(format!("{}: {x}", f.display())))?
                }
                None => json!({}),
            };
            for t in &set {
                let o: Value = serde_json::from_str(t)
                    .map_err(|x| AppError::usage(format!("--set {t}: {x}")))?;
                let Value::Object(o) = o else {
                    return Err(AppError::usage(format!("--set {t} must be an object")));
                };
                let m = v.as_object_mut().ok_or("settings must be an object")?;
                m.extend(o);
            }
            if let Some(n) = trials {
                v["trials"] = json!(n);
            }
            if let Some(n) = seed {
                v["seed"] = json!(n);
            }
            if let Some(t) = v.get_mut("terms") {
                lensbench_engine::Files::Any.check_terms(t)?;
            }
            let (vendors, reversed) = lensbench_engine::load::tolerance_vendors(&c.spec());
            let ms =
                lensbench_design::tolerance::spec_from_json_with(&v, vec![s], &vendors, &reversed)?;
            let oo = lensbench_design::tolerance::out_opts(&v)?;
            let header = match &results {
                Some(_) => lensbench_design::tolerance::runs_header(
                    &v,
                    &ms.problem.configs,
                    &vendors,
                    &reversed,
                )?,
                None => String::new(),
            };
            let ro = lensbench_runs::RunOpts {
                out: results.as_deref(),
                header,
                resume: !no_resume,
                ..Default::default()
            };
            let run = || progress::with(c.json, || lensbench_design::montecarlo::run_to(&ms, &ro));
            let r = match oo.threads {
                Some(t) => lensbench_core::with_threads(Some(t), run),
                None => run(),
            }
            .map_err(e)?;
            if let Some(path) = &out {
                let worst = r
                    .trials
                    .iter()
                    .filter_map(|t| t.eval.as_ref().map(|x| (x.merit, t)))
                    .max_by(|a, b| a.0.total_cmp(&b.0));
                let Some((_, t)) = worst else {
                    return Err("--out: no trial could be traced".into());
                };
                let cs = lensbench_design::montecarlo::perturbed(&ms, &t.deltas).map_err(e)?;
                let mut p = ms.problem.clone();
                p.configs = cs;
                let x = &t.eval.as_ref().unwrap().comp;
                let sys = p.systems(x).map_err(e)?;
                // .json は系ファイル（屈折率のずれ dn・disp も書ける）、ほかは .zmx
                if path
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("json"))
                {
                    lensbench_project::sysfile::save(path, &sys[0], &[])?;
                    warn_notes(lensbench_project::sysfile::save_notes(&sys[0]));
                } else {
                    warn_notes(
                        zmx_write::save(path, &sys[0])
                            .map_err(|x| format!("--out: {x} (write a .json system file)"))?,
                    );
                }
            }
            let out = lensbench_design::tolerance::result_json(&ms, &r, &oo);
            if let Some(path) = &comp_csv {
                let csv = lensbench_design::tolerance::compensators_csv(&out)?;
                lensbench_core::vfs::write(path, csv).map_err(AppError::io_at(path))?;
            }
            (c.json, out)
        }
        Cmd::CatalogAdd {
            catalog,
            name,
            from,
            lens,
            block,
            glass_dir,
            lens_catalog,
            cfg,
            description,
            replace,
            json,
        } => {
            let cat = lensbench_project::UserCatalog::load_or_new(&catalog)?;
            if cat.items.contains_key(&name) && !replace {
                return Err(AppError::usage(format!(
                    "item {name} already exists (use --replace)"
                )));
            }
            if block.len() > 2 {
                return Err(AppError::usage(
                    "--block takes 1 or 2 values (the first and last of the range)",
                ));
            }
            let item = match (lens, block.is_empty()) {
                (Some(k), true) => {
                    let c = Common {
                        file: from.clone(),
                        glass_dir: glass_dir.clone(),
                        lens_catalog: lens_catalog.clone(),
                        cfg,
                        state: None,
                        path: None,
                        json,
                        save: None,
                        strict_macros: false,
                    };
                    lensbench_project::lens_item(&load(&c)?, k, &description)?
                }
                (None, false) => {
                    let txt = lensbench_core::vfs::read_to_string(&from)
                        .map_err(AppError::io_at(&from))?;
                    let p = lensbench_project::Project::from_json(&txt)?;
                    let cat = LensCatalog::load_registered(&lens_catalog)?;
                    let look = |v: &str, p: &str| cat.zmx(v, p);
                    let env = lensbench_project::Env {
                        step_files: None,
                        base: from.parent().map(|x| x.to_path_buf()).unwrap_or_default(),
                        glass_dirs: glass_dir.clone(),
                        catalog: Some(&look),
                    };
                    lensbench_project::blocks_item(&p, &env, &block, &description)?
                }
                _ => return Err(AppError::usage("give one of --lens and --block")),
            };
            // 読み直して足す（品を作る間にほかで足した品を消さない）
            let cat = lensbench_project::UserCatalog::update(&catalog, |cat| {
                if cat.items.contains_key(&name) && !replace {
                    return Err(format!("item {name} already exists (use --replace)").into());
                }
                cat.items.insert(name.clone(), item);
                Ok(())
            })?;
            (
                json,
                json!({ "catalog": catalog, "added": name, "items": cat.items.keys().collect::<Vec<_>>() }),
            )
        }
        Cmd::CatalogSearch {
            lens_catalog,
            glass_dir,
            kind,
            vendor,
            text,
            spec,
            fits,
            list_specs,
            json,
        } => {
            use lensbench_catalog::spec::{CatalogQuery, ItemKind, SpecFilter, SPEC_NAMES};
            if list_specs {
                let v: Vec<Value> = SPEC_NAMES
                    .iter()
                    .map(|(n, u, d)| json!({ "name": n, "unit": u, "doc": d }))
                    .collect();
                if !json {
                    for (n, u, d) in SPEC_NAMES {
                        println!("{n:<18} {u:<7} {d}");
                    }
                    return Ok((false, Value::Null));
                }
                return Ok((true, json!({ "specs": v })));
            }
            let q = CatalogQuery {
                kinds: kind
                    .iter()
                    .map(|k| {
                        ItemKind::parse(k)
                            .ok_or_else(|| AppError::usage(format!("--kind: unknown kind {k}")))
                    })
                    .collect::<Result<_, _>>()?,
                vendor,
                text,
                specs: spec
                    .iter()
                    .map(|s| SpecFilter::parse(s))
                    .collect::<Result<_, _>>()?,
                fits,
            };
            let v = lensbench_engine::browse::search(&lens_catalog, &glass_dir, &q)?;
            if !json {
                print_catalog_table(&v);
                return Ok((false, Value::Null));
            }
            (json, v)
        }
        Cmd::CatalogStep { files, json } => {
            let v: Vec<Value> = files
                .iter()
                .map(|f| {
                    let r = lensbench_core::vfs::read(f)
                        .map_err(lensbench_core::Error::io_at(f.display()))
                        .and_then(|b| {
                            lensbench_catalog::step::read_step(&String::from_utf8_lossy(&b))
                        });
                    match r {
                        Ok(s) => {
                            let mut v = serde_json::to_value(&s).unwrap_or(Value::Null);
                            v["file"] = json!(f.display().to_string());
                            v
                        }
                        Err(e) => {
                            json!({ "file": f.display().to_string(), "error": String::from(e) })
                        }
                    }
                })
                .collect();
            let bad = v.iter().filter(|x| x.get("error").is_some()).count();
            if bad > 0 {
                partial_failure(format!("{bad} STEP files could not be read"));
            }
            if !json {
                for x in &v {
                    println!("{}", x["file"].as_str().unwrap_or(""));
                    match x["error"].as_str() {
                        Some(e) => text_out::say(&format!("  error: {e}")),
                        None => {
                            let (t, d) = (&x["thickness"], &x["diameter"]);
                            text_out::say(&match x["frame"].is_null() {
                                true => format!("  thickness {t} mm, diameter {d}"),
                                false => format!("  thickness {t} mm, diameter {d}, with frame"),
                            });
                            text_out::say(&format!("  {}", x["note"].as_str().unwrap_or("")));
                        }
                    }
                }
                return Ok((false, Value::Null));
            }
            (true, json!({ "files": v }))
        }
        Cmd::CadStep {
            catalog,
            item,
            cache,
            json,
        } => {
            #[cfg(target_arch = "wasm32")]
            {
                let _ = (catalog, item, cache, json);
                return Err("cad-step is not available in the web version".into());
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let cat = lensbench_project::UserCatalog::load(&catalog)?;
                let it = cat
                    .items
                    .get(&item)
                    .ok_or_else(|| format!("no item {item} in {}", catalog.display()))?;
                let dir = cache.unwrap_or_else(cad::default_cache_dir);
                let r = match cad::fetch_item(&dir, &item, it) {
                    Ok(r) => r,
                    Err(e) if it.url.is_empty() => return Err(e.into()),
                    Err(e) => return Err(format!("{e} (product page: {})", it.url).into()),
                };
                if !json {
                    let p = r.path.display();
                    text_out::say(&match r.source {
                        cad::Source::Cache => format!("{p} (cache)"),
                        cad::Source::Store => format!("{p} (from the store)"),
                        cad::Source::Vendor => p.to_string(),
                        cad::Source::VendorUploaded => format!("{p} (also uploaded to the store)"),
                    });
                    for n in &r.store_notes {
                        text_out::say(&format!("  store: {n}"));
                    }
                    if let Some(m) = &r.measured {
                        text_out::say(&format!("  {}", m.note));
                    }
                    if r.notes.is_empty() {
                        text_out::say("  matches the simple outline");
                    }
                    for n in &r.notes {
                        text_out::say(&format!("  mismatch: {n}"));
                    }
                    return Ok((false, Value::Null));
                }
                (true, r.to_json())
            }
        }
        Cmd::CatalogList { catalog, json } => {
            let cat = lensbench_project::UserCatalog::load(&catalog)?;
            let v: Vec<Value> = cat
                .items
                .iter()
                .map(|(k, it)| json!({ "name": k, "description": it.description, "glass_catalogs": it.glass_catalogs, "blocks": it.blocks.len() }))
                .collect();
            (json, json!({ "items": v }))
        }
        Cmd::Coatings {
            file,
            glass_dir,
            coatings: table,
            json,
        } => (json, coatings(&file, &glass_dir, &table)?),
        Cmd::CoatKinks {
            tables,
            key,
            min_score,
            json,
        } => (json, coat_kinks(&tables, &key, min_score)?),
        Cmd::CoatModel(a) => (a.json, coat_model(&a)?),
        Cmd::CoatCompact { model, out, json } => (json, coat_compact(&model, &out)?),
        Cmd::CoatDesign {
            targets,
            name,
            materials,
            substrate,
            coating_file,
            glass_dir,
            layers,
            starts,
            d_max,
            slack,
            out,
            json,
        } => (
            json,
            coat_design(&CoatDesignArgs {
                targets: &targets,
                name: &name,
                materials: &materials,
                substrate: &substrate,
                coating_file: coating_file.as_deref(),
                glass_dir: &glass_dir,
                layers: &layers,
                starts,
                d_max,
                slack,
                out: out.as_deref(),
            })?,
        ),
    };
    if let Some(p) = &save {
        save_value(p, &out).map_err(AppError::io)?;
    }
    Ok((json_out, out))
}

struct CoatDesignArgs<'a> {
    targets: &'a [PathBuf],
    name: &'a str,
    materials: &'a [String],
    substrate: &'a str,
    coating_file: Option<&'a std::path::Path>,
    glass_dir: &'a [PathBuf],
    layers: &'a str,
    starts: usize,
    d_max: f64,
    slack: f64,
    out: Option<&'a std::path::Path>,
}

/// 標準出力の読み手が先に閉じた（`lensbench … | head`）ときの println! の panic は、backtrace を出さず黙って終える（終了コード 0。ほかの panic はいつもどおり）
fn quiet_broken_pipe() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if msg.contains("failed printing to stdout") && msg.contains("Broken pipe")
            || msg.contains("os error 32")
        {
            std::process::exit(0);
        }
        prev(info);
    }));
}

/// コマンドラインの入口（`lensbench` の実行ファイル）。clap の定義に解析の登録簿のサブコマンド（拡張の `Extensions::analysis` で足した解析も）を足し、
/// help（英語が元）を表示の言語に訳したもの
fn cli_command() -> clap::Command {
    cli_command_with(|_| true)
}

/// [`cli_command`] で、解析のサブコマンドの引数は `full` が真の名前だけ組む（ほかは名前と 1 行の説明だけ）。
/// 引数は設定のスキーマから作るので、起動のたびに全部組むと数 ms かかる（`--version`・`info` などにも）
fn cli_command_with(full: impl Fn(&str) -> bool) -> clap::Command {
    use clap::CommandFactory;
    let mut c = Cli::command().after_long_help(error::exit_code_help());
    for a in analyses().iter() {
        let m = a.meta();
        c = c.subcommand(if full(m.name) {
            analysis_cmd::command(a.as_ref())
        } else {
            clap::Command::new(m.name).about(m.about)
        });
    }
    let c = negative_numbers(c);
    // 英語の表示では訳さない（元が英語なので、木を回すだけ無駄になる）
    if lensbench_core::i18n::is_ja() {
        help_json::localize(c, &lensbench_core::i18n::translate_doc)
    } else {
        c
    }
}

/// `--dist -5`・`--pmag -0.5` のような負の数を値として受ける（入れ子のサブコマンドも）
fn negative_numbers(c: clap::Command) -> clap::Command {
    c.allow_negative_numbers(true)
        .mut_subcommands(negative_numbers)
}

/// 解析の登録簿（同梱の解析と、exe の入口で足した拡張の解析）
pub(crate) fn analyses() -> std::sync::Arc<lensbench_analysis::Analyses> {
    lensbench_engine::analyses()
}

/// 引数を解釈する（解析の登録簿のサブコマンドも）
fn parse_cli<I, T>(argv: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    use clap::FromArgMatches;
    let argv: Vec<std::ffi::OsString> = argv.into_iter().map(Into::into).collect();
    // 引数のどこかに名前が出る解析だけ引数まで組む（`spot …`・`help spot`・`--threads 2 spot …`）。
    // 名前の出ない解析は呼ばれないので、名前と説明（`--help` の一覧・候補の提案に使う）だけでよい
    let named = |n: &str| argv.iter().any(|a| a.as_os_str() == n);
    let m = cli_command_with(named).try_get_matches_from(&argv)?;
    if let Some((name, sub)) = m.subcommand() {
        if let Some(a) = analyses().get(name).cloned() {
            let threads = m.try_get_one::<usize>("threads").ok().flatten().copied();
            let cmd = analysis_cmd::parse(a.as_ref(), sub).map_err(|x| {
                // clap が「error: 」を付けるので、種類の前置きを付けない文（訳す）
                let m = lensbench_core::i18n::tr(&String::from(x));
                cli_command_with(named).error(clap::error::ErrorKind::ValueValidation, m)
            })?;
            return Ok(Cli {
                threads,
                cmd: Cmd::Analysis(Box::new(cmd)),
            });
        }
    }
    Cli::from_arg_matches(&m)
}

/// 拡張を入れてから CLI を動かす（外のクレートの exe の入口）
pub fn main_with(ext: lensbench_engine::Extensions) -> ExitCode {
    if let Err(e) = ext.install() {
        eprintln!(
            "lensbench: {}",
            lensbench_core::i18n::translate(&e.to_string())
        );
        return ExitCode::FAILURE;
    }
    main()
}

pub fn main() -> ExitCode {
    quiet_broken_pipe();
    // 隠し: clap の定義を JSON で（マニュアルの CLI のページの元）
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if help_json::wants(&argv[1.min(argv.len())..]) {
        print_out(true, &help_json::help_json(cli_command()));
        return ExitCode::SUCCESS;
    }
    let cli = parse_cli(argv).unwrap_or_else(|e| clap_ja::fail(e));
    #[cfg(not(target_arch = "wasm32"))]
    coat_remote::install();
    // run はスレッド数を自分では使わず、起動する Python に渡す
    if let Cmd::Run(a) = &cli.cmd {
        return match run::run(a, cli.threads) {
            // 0〜255 の外（Windows の異常終了の NTSTATUS は負など）は 1 にする（0 に丸めると成功に見える）
            Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            // 設定の誤りは 2（ファイルが無いなども設定の誤りとして 2、Python の終了コードと分ける）
            Err(m) => {
                error::report("lensbench run", &AppError::usage(m));
                ExitCode::from(2)
            }
        };
    }
    lensbench_core::init_from_env();
    match lensbench_core::with_threads(cli.threads, || run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error::report("lensbench", &e);
            ExitCode::from(e.exit_code())
        }
    }
}

/// .zmx に書けなかったもの（[`zmx_write::write_notes`]）を標準エラーに出す
fn warn_notes(notes: Vec<String>) {
    for n in notes {
        eprintln!("{}", lensbench_core::i18n::warning(n));
    }
}
