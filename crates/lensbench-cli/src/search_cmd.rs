//! `lensbench zoom`（ズーム軌跡）と `lensbench swap`（カタログレンズの差し替え探索）

use std::path::PathBuf;

use clap::Args;
use lensbench_core::catalog::LensCatalog;
use lensbench_core::{zmx, System};
use lensbench_design::dls_spec::{ImageMerit, Reference, Targets, VarSpec, ZoomMethod, ZoomParams};
use lensbench_design::swap::{SwapSearch, SwapSpec};
use lensbench_project::{Env, Project};
use serde_json::{json, Value};

/// ズーム軌跡の設定の既定値（旗の既定はここから、XCT-02）
static ZOOM: std::sync::LazyLock<ZoomParams> = std::sync::LazyLock::new(ZoomParams::default);

#[derive(Args)]
pub struct ZoomArgs {
    /// .zmx (each multi-configuration is a state) or a project file (.json, states are the states)
    pub file: PathBuf,
    /// Folders to search for glass catalogs (more than one allowed; LENSBENCH_GLASSCAT is also used)
    #[arg(long = "glass-dir")]
    pub glass_dir: Vec<PathBuf>,
    /// Lens catalogs for the catalog lenses ("vendor", "part") of a project file (.zmx files, or folders of .zmx; the folder name is the maker and the file name is the part number; more than one allowed)
    #[arg(long = "lens-catalog")]
    pub lens_catalog: Vec<PathBuf>,
    /// States, comma-separated (state names for a project, configuration numbers for .zmx). All if omitted
    #[arg(long)]
    pub states: Option<String>,
    /// Parameters to solve for each state ("G2.shift", "7.t"; a range is "G2.shift=-5:20"). More than one allowed
    #[arg(long = "var", required = true)]
    pub vars: Vec<String>,
    /// Focal length targets per state (comma-separated, one per state; "-" for no target)
    #[arg(long, conflicts_with = "pmag", allow_hyphen_values = true)]
    pub efl: Option<String>,
    /// Paraxial magnification targets per state (finite object; written like --efl)
    #[arg(long, allow_hyphen_values = true)]
    pub pmag: Option<String>,
    /// Number of points between neighboring states
    #[arg(long, default_value_t = ZOOM.steps)]
    pub steps: usize,
    /// Image merit: spot or wavefront
    #[arg(long, default_value_t = ZOOM.merit)]
    pub merit: ImageMerit,
    /// Reference: centroid or chief
    #[arg(long = "reference", default_value_t = ZOOM.reference)]
    pub reference: Reference,
    /// Number of rings of the Gaussian pupil quadrature
    #[arg(long, default_value_t = ZOOM.rings)]
    pub rings: usize,
    /// Maximum iterations per point
    #[arg(long, default_value_t = ZOOM.iters)]
    pub iters: usize,
    /// Tolerance from the target (relative to the target)
    #[arg(long, default_value_t = ZOOM.tol)]
    pub tol: f64,
    /// How to meet the targets: penalty (raise the weight of the target term) or lagrange (augmented Lagrangian: shift the target of the term)
    #[arg(long, default_value_t = ZOOM.method)]
    pub method: ZoomMethod,
    #[command(flatten)]
    pub dls: DlsArgs,
    /// Output JSON
    #[arg(long)]
    pub json: bool,
    /// Write the result (json, csv or npz; csv and npz are the trajectory table)
    #[arg(long)]
    pub save: Option<PathBuf>,
}

/// DLS の設定の旗（optimize・zoom）
#[derive(Args, Clone, Copy)]
pub struct DlsArgs {
    /// Solve each step by QR instead of the normal equations (no squaring of the condition number; the path can differ by rounding on ill-conditioned problems)
    #[arg(long)]
    pub qr: bool,
    /// Stop when the largest cosine between the residuals and a variable's column is below this (0: no gradient test)
    #[arg(long, default_value_t = 0.0, value_parser = lensbench_design::dls_spec::parse_non_negative)]
    pub gtol: f64,
    /// Bounds: fix variables that a step would take out of range at the bound and solve again for the rest (instead of clipping each component)
    #[arg(long = "active-set")]
    pub active_set: bool,
    /// Drop stopped rays (total internal reflection, missing a surface) like vignetted rays instead of failing the evaluation; the counts are in the result
    #[arg(long = "stopped-as-vignetted")]
    pub stopped_as_vignetted: bool,
}

#[derive(Args)]
pub struct SwapArgs {
    /// Project file (.json)
    pub file: PathBuf,
    /// Folders to search for glass catalogs (more than one allowed; LENSBENCH_GLASSCAT is also used)
    #[arg(long = "glass-dir")]
    pub glass_dir: Vec<PathBuf>,
    /// Lens catalogs for the catalog lenses ("vendor", "part") of a project file (.zmx files, or folders of .zmx; the folder name is the maker and the file name is the part number; more than one allowed)
    #[arg(long = "lens-catalog", required = true)]
    pub lens_catalog: Vec<PathBuf>,
    /// Swap search settings (JSON: slots, vars, merit, iters, state, etc.)
    #[arg(long)]
    pub spec: PathBuf,
    /// State (takes precedence over state in the settings)
    #[arg(long)]
    pub state: Option<String>,
    /// JSONL to write each combination's result to as it goes (the batch run format). Continues if a file of the same search exists
    #[arg(long, short)]
    pub out: Option<PathBuf>,
    /// Rewrite the --out file instead of continuing from it
    #[arg(long = "no-resume")]
    pub no_resume: bool,
    /// Number of combinations to output (in order of merit). 0 for all
    #[arg(long, default_value_t = 20)]
    pub top: usize,
    /// Output JSON
    #[arg(long)]
    pub json: bool,
    /// Write the result (json, csv or npz)
    #[arg(long)]
    pub save: Option<PathBuf>,
}

/// プロジェクトファイル（系ファイルは状態を持たないので .zmx と同じに扱う）
fn is_project(p: &std::path::Path) -> bool {
    lensbench_project::open::FileKind::of(p) == lensbench_project::open::FileKind::Project
}

/// 状態ごとの系と、その名前
fn load_states(a: &ZoomArgs) -> Result<(Vec<System>, Vec<String>), crate::error::AppError> {
    let names: Option<Vec<String>> = a.states.as_ref().map(|s| {
        s.split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    });
    if is_project(&a.file) {
        let txt = lensbench_core::vfs::read_to_string(&a.file)
            .map_err(crate::error::AppError::io_at(&a.file))?;
        let p = Project::from_json(&txt)?;
        let names = match names {
            Some(n) => n,
            None => p.state_names().iter().map(|s| s.to_string()).collect(),
        };
        if names.is_empty() {
            return Err("the project has no states (states)".into());
        }
        let cat = LensCatalog::load_registered(&a.lens_catalog)?;
        let look = |v: &str, q: &str| cat.zmx(v, q);
        let mut v = Vec::new();
        for n in &names {
            let (s, warn) = lensbench_project::load_path_report(
                &a.file,
                &a.glass_dir,
                Some(&look),
                Some(n),
                None,
            )?;
            for w in warn {
                eprintln!(
                    "{}",
                    lensbench_core::i18n::warning(format!("state {n}: {w}"))
                );
            }
            v.push(s);
        }
        Ok((v, names))
    } else if lensbench_project::open::FileKind::of(&a.file)
        == lensbench_project::open::FileKind::SystemFile
    {
        Err("a system file has no configurations or states (give a .zmx or project file)".into())
    } else {
        let cfgs: Vec<usize> = match &names {
            Some(n) => n
                .iter()
                .map(|t| {
                    t.parse().map_err(|_| {
                        lensbench_core::Error::from(format!("configuration number {t} of --states"))
                    })
                })
                .collect::<lensbench_core::Result<_>>()?,
            None => Vec::new(),
        };
        let opt = zmx::LoadOptions {
            catalog_dirs: a.glass_dir.clone(),
            ..Default::default()
        };
        let v = zmx::load_configs(&a.file, &opt, &cfgs)?;
        let names = if cfgs.is_empty() {
            (1..=v.len()).map(|k| k.to_string()).collect()
        } else {
            cfgs.iter().map(|k| k.to_string()).collect()
        };
        Ok((v, names))
    }
}

pub fn zoom(a: &ZoomArgs) -> Result<Value, crate::error::AppError> {
    let (cs, names) = load_states(a)?;
    let o = ZoomParams {
        vars: a.vars.iter().map(|v| VarSpec::from(v.as_str())).collect(),
        efl: a.efl.clone().map(Targets::Text),
        pmag: a.pmag.clone().map(Targets::Text),
        steps: a.steps,
        merit: a.merit,
        reference: a.reference,
        rings: a.rings,
        iters: a.iters,
        tol: a.tol,
        method: a.method,
        qr: a.dls.qr,
        gtol: a.dls.gtol,
        active_set: a.dls.active_set,
        stopped_as_vignetted: a.dls.stopped_as_vignetted,
        ..ZoomParams::default()
    };
    let z = crate::progress::with(a.json, || {
        lensbench_engine::zoom::zoom_systems(&cs, &names, &o)
    })?;
    Ok(serde_json::to_value(z).map_err(|e| e.to_string())?)
}

pub fn swap(a: &SwapArgs) -> Result<Value, crate::error::AppError> {
    let txt = lensbench_core::vfs::read_to_string(&a.file)
        .map_err(crate::error::AppError::io_at(&a.file))?;
    let p = Project::from_json(&txt)?;
    let st = lensbench_core::vfs::read_to_string(&a.spec)
        .map_err(crate::error::AppError::io_at(&a.spec))?;
    let mut spec = SwapSpec::from_json(&st)?;
    if a.state.is_some() {
        spec.state = a.state.clone();
    }
    let cat = LensCatalog::load_registered(&a.lens_catalog)?;
    let env = Env {
        step_files: None,
        base: a.file.parent().map(|p| p.to_path_buf()).unwrap_or_default(),
        glass_dirs: a.glass_dir.clone(),
        catalog: None,
    };
    let s = SwapSearch::new(&p, &env, &cat, spec)?;
    let n = s.count();
    let t0 = web_time::Instant::now();
    let mut r = crate::progress::with(a.json, || s.run_to(a.out.as_deref(), !a.no_resume, None))?;
    if !a.json && a.out.is_some() {
        eprintln!();
    }
    let failed = r.iter().filter(|x| x.error.is_some()).count();
    if a.top > 0 {
        r.truncate(a.top);
    }
    Ok(json!({
        "count": n,
        "failed": failed,
        "seconds": t0.elapsed().as_secs_f64(),
        "slots": s.picks.iter().map(|v| v.len()).collect::<Vec<_>>(),
        "results": r,
    }))
}
