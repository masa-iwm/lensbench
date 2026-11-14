//! `lensbench zoom`（ズーム軌跡）と `lensbench swap`（カタログレンズの差し替え探索）

use std::path::PathBuf;

use clap::Args;
use lensbench_core::catalog::LensCatalog;
use lensbench_core::{zmx, System};
use lensbench_design::swap::{SwapSearch, SwapSpec};
use lensbench_project::{Env, Project};
use serde_json::{json, Value};

#[derive(Args)]
pub struct ZoomArgs {
    /// .zmx（多重構成の各構成が状態）か、プロジェクトファイル（.json、states が状態）
    pub file: PathBuf,
    #[arg(long = "glass-dir")]
    pub glass_dir: Vec<PathBuf>,
    #[arg(long = "lens-catalog")]
    pub lens_catalog: Vec<PathBuf>,
    /// 状態（プロジェクトなら状態の名前、.zmx なら構成の番号）をカンマで並べる。省くとすべて
    #[arg(long)]
    pub states: Option<String>,
    /// 状態ごとに解くパラメータ（"G2.shift"・"7.t"、範囲は "G2.shift=-5:20"）。複数可
    #[arg(long = "var", required = true)]
    pub vars: Vec<String>,
    /// 状態ごとの焦点距離の目標（カンマで、状態の数だけ。"-" は目標なし）
    #[arg(long, conflicts_with = "pmag", allow_hyphen_values = true)]
    pub efl: Option<String>,
    /// 状態ごとの近軸倍率の目標（有限物体。書き方は --efl と同じ）
    #[arg(long, allow_hyphen_values = true)]
    pub pmag: Option<String>,
    /// 隣り合う状態の間にとる点の数
    #[arg(long, default_value_t = 0)]
    pub steps: usize,
    /// 像の評価関数: spot か wavefront
    #[arg(long, default_value = "spot")]
    pub merit: String,
    /// 基準: centroid か chief
    #[arg(long = "reference", default_value = "centroid")]
    pub reference: String,
    #[arg(long, default_value_t = 3)]
    pub rings: usize,
    /// 1 点の反復の上限
    #[arg(long, default_value_t = 50)]
    pub iters: usize,
    /// 目標との差の許し（目標に対する比）
    #[arg(long, default_value_t = 1e-8)]
    pub tol: f64,
    #[arg(long)]
    pub json: bool,
    /// 結果を書き出す（json・csv・npz。csv・npz は軌跡の表）
    #[arg(long)]
    pub save: Option<PathBuf>,
}

#[derive(Args)]
pub struct SwapArgs {
    /// プロジェクトファイル（.json）
    pub file: PathBuf,
    #[arg(long = "glass-dir")]
    pub glass_dir: Vec<PathBuf>,
    #[arg(long = "lens-catalog", required = true)]
    pub lens_catalog: Vec<PathBuf>,
    /// 差し替え探索の設定（JSON: slots・vars・merit・iters・state など）
    #[arg(long)]
    pub spec: PathBuf,
    /// 状態（設定の state より優先）
    #[arg(long)]
    pub state: Option<String>,
    /// 組ごとの結果を逐次書き出す JSONL（一括実行の形）。同じ探索のファイルがあれば続きから回す
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// --out のファイルがあっても続きからにせず書き直す
    #[arg(long = "no-resume")]
    pub no_resume: bool,
    /// 出力する組の数（評価関数の小さい順）。0 ならすべて
    #[arg(long, default_value_t = 20)]
    pub top: usize,
    #[arg(long)]
    pub json: bool,
    /// 結果を書き出す（json・csv・npz）
    #[arg(long)]
    pub save: Option<PathBuf>,
}

/// プロジェクトファイル（系ファイルは状態を持たないので .zmx と同じに扱う）
fn is_project(p: &std::path::Path) -> bool {
    lensbench_project::open::FileKind::of(p) == lensbench_project::open::FileKind::Project
}

/// 状態ごとの系と、その名前
fn load_states(a: &ZoomArgs) -> Result<(Vec<System>, Vec<String>), String> {
    let names: Option<Vec<String>> = a.states.as_ref().map(|s| {
        s.split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    });
    if is_project(&a.file) {
        let txt = lensbench_core::vfs::read_to_string(&a.file)
            .map_err(|e| format!("{}: {e}", a.file.display()))?;
        let p = Project::from_json(&txt)?;
        let names = match names {
            Some(n) => n,
            None => p.state_names().iter().map(|s| s.to_string()).collect(),
        };
        if names.is_empty() {
            return Err("プロジェクトに状態（states）がない".into());
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
                eprintln!("注意: 状態 {n}: {w}");
            }
            v.push(s);
        }
        Ok((v, names))
    } else if lensbench_project::open::FileKind::of(&a.file)
        == lensbench_project::open::FileKind::SystemFile
    {
        Err("系ファイルは構成・状態を持たない（.zmx かプロジェクトファイルを渡す）".into())
    } else {
        let cfgs: Vec<usize> = match &names {
            Some(n) => n
                .iter()
                .map(|t| t.parse().map_err(|_| format!("--states の構成の番号 {t}")))
                .collect::<Result<_, String>>()?,
            None => Vec::new(),
        };
        let opt = zmx::LoadOptions {
            catalog_dirs: a.glass_dir.clone(),
            ..Default::default()
        };
        let v = zmx::load_configs(&a.file, &opt, &cfgs).map_err(|e| e.to_string())?;
        let names = if cfgs.is_empty() {
            (1..=v.len()).map(|k| k.to_string()).collect()
        } else {
            cfgs.iter().map(|k| k.to_string()).collect()
        };
        Ok((v, names))
    }
}

pub fn zoom(a: &ZoomArgs) -> Result<Value, String> {
    let (cs, names) = load_states(a)?;
    let o = lensbench_engine::zoom::ZoomOpts {
        vars: a.vars.clone(),
        efl: a.efl.clone(),
        pmag: a.pmag.clone(),
        steps: a.steps,
        merit: a.merit.clone(),
        reference: a.reference.clone(),
        rings: a.rings,
        iters: a.iters,
        tol: a.tol,
    };
    lensbench_engine::zoom::zoom_systems(&cs, &names, &o)
}

pub fn swap(a: &SwapArgs) -> Result<Value, String> {
    let txt = lensbench_core::vfs::read_to_string(&a.file)
        .map_err(|e| format!("{}: {e}", a.file.display()))?;
    let p = Project::from_json(&txt)?;
    let st = lensbench_core::vfs::read_to_string(&a.spec)
        .map_err(|e| format!("{}: {e}", a.spec.display()))?;
    let mut spec = SwapSpec::from_json(&st)?;
    if a.state.is_some() {
        spec.state = a.state.clone();
    }
    let cat = LensCatalog::load_registered(&a.lens_catalog)?;
    let env = Env {
        base: a.file.parent().map(|p| p.to_path_buf()).unwrap_or_default(),
        glass_dirs: a.glass_dir.clone(),
        catalog: None,
    };
    let s = SwapSearch::new(&p, &env, &cat, spec)?;
    let n = s.count();
    let t0 = web_time::Instant::now();
    let show = |pr: lensbench_core::runs::Progress| {
        if !a.json {
            eprint!(
                "\r組 {}/{}（残り {}）  ",
                pr.done,
                pr.total,
                pr.eta.map_or("?".into(), |e| format!("{e:.0} 秒"))
            );
        }
    };
    let mut r = s.run_to(a.out.as_deref(), !a.no_resume, None, Some(&show))?;
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
