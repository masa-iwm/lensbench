//! lensbench のコマンドライン。視野・波長の番号は OpticStudio と同じく 1 始まり。

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use lensbench_core::analysis::{self, ZernKind};
use lensbench_core::coating::{self, Coatings};
use lensbench_core::{glass, report, zmx, FieldSel, System};
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
    /// 面の膜: 全コンフィグの境界（膜|前の媒質|後の媒質）と扱い。実測表が要る境界（暗号化された膜など）も示す
    Coatings {
        /// .zmx ファイル
        file: PathBuf,
        /// ガラスカタログを探すフォルダ（複数可）
        #[arg(long = "glass-dir")]
        glass_dir: Vec<PathBuf>,
        /// 膜の実測表（LBCT 形式、複数可）
        #[arg(long)]
        table: Vec<PathBuf>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 実測表（LBCT）から、波長方向の折れ目（膜の材料の分散表の節）を探す。
    /// 波長の格子は膜の厚さから決めた格子の 4 倍くらい細かくして測っておく
    CoatKinks {
        /// 実測表（LBCT 形式、複数可）
        tables: Vec<PathBuf>,
        /// 調べる KEY（複数回指定できる。既定は空気から入る透過と鏡の KEY。硝材側は高い β の列で残差が大きく、検出が鈍い）
        #[arg(long)]
        key: Vec<String>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 暗号化された膜のモデル（LBCM 2）を、モデルガラスの組で測った実測表（LBCT）から作る
    CoatModel {
        /// 実測表（LBCT 形式、複数可。同じ KEY は後の表のものを使う）
        tables: Vec<PathBuf>,
        /// 書き出す LBCM ファイル
        #[arg(long)]
        out: PathBuf,
        /// 作る膜（カンマ区切り、既定は表にある全部）
        #[arg(long)]
        coat: Option<String>,
        /// 波長方向の折れ目 [µm]（カンマ区切り）。膜の材料の分散表の節。補間はこの点をまたがない
        #[arg(long)]
        kinks: Option<String>,
        /// BMAX を決める ψ の補間誤差の上限 [rad]
        #[arg(long, default_value_t = 1e-4)]
        tol_psi: f64,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
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
        Cmd::Coatings {
            file,
            glass_dir,
            table,
            json,
        } => (json, coatings(&file, &glass_dir, &table)?),
        Cmd::CoatKinks { tables, key, json } => (json, coat_kinks(&tables, &key)?),
        Cmd::CoatModel {
            tables,
            out,
            coat,
            kinks,
            tol_psi,
            json,
        } => (
            json,
            coat_model(&tables, &out, coat.as_deref(), kinks.as_deref(), tol_psi)?,
        ),
    };
    if json_out {
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        print_text(&out, 0);
    }
    Ok(())
}

fn coatings(
    file: &std::path::Path,
    glass_dir: &[PathBuf],
    tables: &[PathBuf],
) -> Result<Value, String> {
    let e = |e: lensbench_core::Error| e.to_string();
    let txt = glass::read_text(file).map_err(|x| format!("{}: {x}", file.display()))?;
    let raw = zmx::parse_text(&txt).map_err(e)?;
    let mut cfgs = Vec::new();
    // 実測表が要る境界 → (コンフィグ, 波長)
    let mut need: Vec<(String, Vec<usize>, Vec<f64>)> = Vec::new();
    let mut found = None;
    for cfg in 1..=raw.configurations() {
        let opt = zmx::LoadOptions {
            cfg,
            catalog_dirs: glass_dir.to_vec(),
            ..Default::default()
        };
        let s = zmx::load_with(file, &opt).map_err(|x| format!("構成 {cfg}: {x}"))?;
        let mut c = Coatings::for_system(&s).map_err(e)?;
        for t in tables {
            c.load_file(t).map_err(e)?;
        }
        found = s.coating_file.clone();
        let (keys, _) = coating::media_keys(&s);
        let mut v = Vec::new();
        for (j, k) in keys.iter().enumerate() {
            let Some(k) = k else { continue };
            let kind = c.kind(k);
            v.push(json!({ "surface": j, "key": k, "kind": kind.name() }));
            if kind.needs_table() {
                let i = match need.iter().position(|n| n.0 == *k) {
                    Some(i) => i,
                    None => {
                        need.push((k.clone(), Vec::new(), Vec::new()));
                        need.len() - 1
                    }
                };
                let n = &mut need[i];
                if !n.1.contains(&cfg) {
                    n.1.push(cfg);
                }
                for &w in &s.wavelengths {
                    if !n.2.contains(&w) {
                        n.2.push(w);
                    }
                }
            }
        }
        cfgs.push(json!({ "cfg": cfg, "wavelengths_um": s.wavelengths, "interfaces": v }));
    }
    Ok(json!({
        "coating_file": raw.coating_file_name(),
        "coating_file_path": found.map(|p| p.display().to_string()),
        "catalogs": raw.catalogs(),
        "configurations": cfgs,
        "needs_table": need.iter().map(|(k, c, w)| json!({ "key": k, "cfgs": c, "wavelengths_um": w })).collect::<Vec<_>>(),
    }))
}

fn coat_kinks(tables: &[PathBuf], only: &[String]) -> Result<Value, String> {
    use lensbench_core::coat_fit::detect_kinks;
    let mut keys = Vec::new();
    // 膜ごとに、KEY で見つかった折れ目をまとめる（5e-4 µm 以内は同じ）
    let mut per: Vec<(String, Vec<f64>)> = Vec::new();
    for t in tables {
        let txt = std::fs::read_to_string(t).map_err(|e| format!("{}: {e}", t.display()))?;
        for (k, m) in coating::parse_measured(&txt).map_err(|e| format!("{}: {e}", t.display()))? {
            let air = k.split('|').nth(1) == Some("AIR");
            if (only.is_empty() && !air) || (!only.is_empty() && !only.contains(&k)) {
                continue;
            }
            let ks = detect_kinks(&m);
            let coat = k.split('|').next().unwrap_or("").to_string();
            let i = match per.iter().position(|(c, _)| *c == coat) {
                Some(i) => i,
                None => {
                    per.push((coat.clone(), Vec::new()));
                    per.len() - 1
                }
            };
            for kk in &ks {
                if !per[i].1.iter().any(|&w| (w - kk.wl).abs() < 5e-4) {
                    per[i].1.push(kk.wl);
                }
            }
            keys.push(json!({
                "key": k,
                "wavelengths": m.wavelengths.len(),
                "kinks": ks.iter().map(|k| json!({"wl_um": k.wl, "smooth": k.smooth, "kinked": k.kinked})).collect::<Vec<_>>(),
            }));
        }
    }
    let coats: Vec<Value> = per
        .into_iter()
        .map(|(c, mut w)| {
            w.sort_by(f64::total_cmp);
            // --kinks にそのまま渡せる形（0.1 nm に丸める）
            let arg: Vec<String> = w.iter().map(|x| format!("{:.4}", x)).collect();
            json!({ "coat": c, "kinks_um": w, "kinks_arg": arg.join(",") })
        })
        .collect();
    Ok(json!({ "coats": coats, "keys": keys }))
}

fn coat_model(
    tables: &[PathBuf],
    out: &std::path::Path,
    only: Option<&str>,
    kinks: Option<&str>,
    tol_psi: f64,
) -> Result<Value, String> {
    let mut kinks: Vec<f64> = match kinks {
        None => Vec::new(),
        Some(k) => k
            .split(',')
            .map(|x| x.trim().parse::<f64>().map_err(|_| format!("--kinks: {x}")))
            .collect::<Result<_, _>>()?,
    };
    kinks.sort_by(f64::total_cmp);
    use lensbench_core::coat_fit::{fit_model, format_models, FitOptions};
    let mut all = Vec::new();
    for t in tables {
        let txt = std::fs::read_to_string(t).map_err(|e| format!("{}: {e}", t.display()))?;
        for (k, m) in coating::parse_measured(&txt).map_err(|e| format!("{}: {e}", t.display()))? {
            // 同じ KEY は後の表で置き換える（渡す順で結果が変わらないように、混ぜない）
            if let Some(i) = all.iter().position(|(a, _): &(String, _)| *a == k) {
                eprintln!("warning: {k}: {} の表で置き換えた", t.display());
                all.remove(i);
            }
            all.push((k, m));
        }
    }
    let mut coats: Vec<String> = Vec::new();
    for (k, _) in &all {
        let c = k.split('|').next().unwrap_or("").to_string();
        if !coats.contains(&c) {
            coats.push(c);
        }
    }
    if let Some(o) = only {
        let want: Vec<&str> = o.split(',').collect();
        coats.retain(|c| want.contains(&c.as_str()));
    }
    let opt = FitOptions {
        tol_psi,
        ..Default::default()
    };
    let mut models = Vec::new();
    let mut rep = Vec::new();
    for c in &coats {
        let t: Vec<(&str, &coating::Measured)> = all
            .iter()
            .filter(|(k, _)| k.split('|').next() == Some(c.as_str()))
            .map(|(k, m)| (k.as_str(), m))
            .collect();
        // モデルガラスの組がない膜などは飛ばして、ほかの膜は作る
        let r = match fit_model(&t, &opt) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("warning: {c}: 飛ばした（{e}）");
                rep.push(json!({ "coat": c, "skipped": e.to_string() }));
                continue;
            }
        };
        let mut r = r;
        r.model.kinks = kinks.clone();
        // 折れ目の波長で測っていないと、そこをはさむ区間の端は外挿になる
        let unmeasured: Vec<f64> = kinks
            .iter()
            .copied()
            .filter(|&k| {
                let w = &r.model.wavelengths;
                !w.is_empty()
                    && (w[0] < k && k < w[w.len() - 1])
                    && !w.iter().any(|&x| (x - k).abs() < 1e-9)
            })
            .collect();
        if !unmeasured.is_empty() {
            eprintln!("warning: {c}: 折れ目 {unmeasured:?} um の波長で測っていない（その付近は外挿になる）");
        }
        let coarse = r.wave_gap > r.wave_gap_needed;
        if coarse {
            eprintln!(
                "warning: {c}: 波長の格子が粗い（波数の間隔 {:.4} > 目安 {:.4} [1/um]）。波長の間で係数がずれる",
                r.wave_gap, r.wave_gap_needed
            );
        }
        rep.push(json!({
            "coat": c,
            "d_um": r.model.d,
            "bmax": r.model.bmax,
            "mirror_d_um": r.model.mirror.as_ref().map(|m| m.d),
            "wavelengths": r.model.wavelengths.len(),
            "betas": r.model.betas.len(),
            "points": r.points,
            "residual_max": r.residual_max,
            "wave_gap": r.wave_gap,
            "wave_gap_needed": r.wave_gap_needed,
            "coarse": coarse,
            "kinks_unmeasured": unmeasured,
        }));
        models.push((c.clone(), r.model));
    }
    if models.is_empty() {
        return Err("作れた膜のモデルがない".into());
    }
    let e: Vec<(&str, &coating::CoatModel)> = models.iter().map(|(c, m)| (c.as_str(), m)).collect();
    std::fs::write(out, format_models(&e)).map_err(|x| format!("{}: {x}", out.display()))?;
    Ok(json!({ "out": out.display().to_string(), "models": rep }))
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
