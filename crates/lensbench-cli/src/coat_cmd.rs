//! 膜の道具のコマンド（coat-design・coat-model・coat-kinks・coat-compact）

use super::*;

pub(crate) fn coat_kinks(
    tables: &[PathBuf],
    only: &[String],
    min_score: f64,
) -> Result<Value, crate::error::AppError> {
    use lensbench_design::coat_fit::detect_kinks_with;
    let mut keys = Vec::new();
    // 膜ごとに、KEY で見つかった折れ目をまとめる（5e-4 µm 以内は同じ）
    let mut per: Vec<(String, Vec<f64>)> = Vec::new();
    for t in tables {
        let txt = lensbench_core::vfs::read_to_string(t)
            .map_err(lensbench_core::Error::io_at(t.display()))?;
        for (k, m) in coating::parse_measured(&txt).map_err(|e| e.at(t.display()))? {
            let air = k.split('|').nth(1) == Some("AIR");
            if (only.is_empty() && !air) || (!only.is_empty() && !only.contains(&k)) {
                continue;
            }
            let ks = detect_kinks_with(&m, min_score);
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

// `coatings` の引数（main.rs を大きくしないようここに置く）
#[derive(clap::Args)]
pub(crate) struct CoatingsArgs {
    /// A .zmx file (every multi-configuration), a project file (.json, every state) or a system file
    pub(crate) file: PathBuf,
    /// Folders to search for glass catalogs (more than one allowed)
    #[arg(long = "glass-dir")]
    pub(crate) glass_dir: Vec<PathBuf>,
    /// Lens catalogs for the catalog lenses ("vendor", "part") of a project file (.zmx files, or folders of .zmx; the folder name is the maker and the file name is the part number; more than one allowed)
    #[arg(long = "lens-catalog")]
    pub(crate) lens_catalog: Vec<PathBuf>,
    /// Measured coating tables and models (more than one allowed; same as --coatings of ghost and the analyses)
    #[arg(long)]
    pub(crate) coatings: Vec<PathBuf>,
    /// Output JSON
    #[arg(long)]
    pub(crate) json: bool,
}

// `coat-model` の引数（main.rs を大きくしないようここに置く）
#[derive(clap::Args)]
pub(crate) struct CoatModelArgs {
    /// Measured tables (LBCT format, more than one allowed; for the same KEY the later table is used)
    tables: Vec<PathBuf>,
    /// LBCM file to write
    #[arg(long, short)]
    out: PathBuf,
    /// Coatings to make (comma-separated; all in the tables by default)
    #[arg(long)]
    coat: Option<String>,
    /// Kinks along wavelength [µm] (comma-separated): nodes of the dispersion tables of the coating materials. Interpolation doesn't cross these points
    #[arg(long)]
    kinks: Option<String>,
    /// Upper limit of the ψ interpolation error that sets BMAX [rad]
    #[arg(long, default_value_t = 1e-4)]
    tol_psi: f64,
    /// Note to put in the model (NOTE; shown by `lensbench coatings`; e.g. a mark for an approximate model made from a short measurement)
    #[arg(long)]
    note: Option<String>,
    /// Measured tables to check the model against (LBCT format, more than one allowed; points measured apart from the model points). Reports the largest difference and the wavelengths to add
    #[arg(long)]
    check: Vec<PathBuf>,
    /// Difference of the coefficients above which --check proposes wavelengths to add
    #[arg(long, default_value_t = 1e-4)]
    check_tol: f64,
    /// Output JSON
    #[arg(long)]
    pub(crate) json: bool,
}

pub(crate) fn coat_model(a: &CoatModelArgs) -> lensbench_core::Result<Value> {
    let (tables, out, only, tol_psi, check_tol) =
        (&a.tables, &a.out, a.coat.as_deref(), a.tol_psi, a.check_tol);
    let (kinks, note) = (a.kinks.as_deref(), a.note.as_deref());
    // 照合用の表（モデルを作った点とは別に測った表）
    let mut checks: Vec<(String, coating::Measured)> = Vec::new();
    for t in &a.check {
        let txt = lensbench_core::vfs::read_to_string(t)
            .map_err(lensbench_core::Error::io_at(t.display()))?;
        checks.extend(coating::parse_measured(&txt).map_err(|e| e.at(t.display()))?);
    }
    let mut kinks: Vec<f64> = match kinks {
        None => Vec::new(),
        Some(k) => k
            .split(',')
            .map(|x| x.trim().parse::<f64>().map_err(|_| format!("--kinks: {x}")))
            .collect::<Result<_, _>>()?,
    };
    kinks.sort_by(f64::total_cmp);
    use lensbench_core::coating::format_models;
    use lensbench_design::coat_fit::{fit_model, FitOptions};
    let mut all = Vec::new();
    for t in tables {
        let txt = lensbench_core::vfs::read_to_string(t)
            .map_err(lensbench_core::Error::io_at(t.display()))?;
        for (k, m) in coating::parse_measured(&txt).map_err(|e| e.at(t.display()))? {
            // 同じ KEY は後の表で置き換える（渡す順で結果が変わらないように、混ぜない）
            if let Some(i) = all.iter().position(|(a, _): &(String, _)| *a == k) {
                eprintln!(
                    "{}",
                    lensbench_core::i18n::tr(&format!(
                        "warning: {k}: replaced with the table in {}",
                        t.display()
                    ))
                );
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
                eprintln!(
                    "{}",
                    lensbench_core::i18n::tr(&format!("warning: {c}: skipped ({e})"))
                );
                rep.push(json!({ "coat": c, "skipped": e.to_string() }));
                continue;
            }
        };
        let mut r = r;
        r.model.kinks = kinks.clone();
        r.model.note = note.unwrap_or("").trim().to_string();
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
            eprintln!("{}", lensbench_core::i18n::tr(&format!("warning: {c}: not measured at the kink wavelengths {unmeasured:?} um (extrapolated near them)")));
        }
        let coarse = r.wave_gap > r.wave_gap_needed;
        if coarse {
            let (gap, need) = (r.wave_gap, r.wave_gap_needed);
            eprintln!("{}", lensbench_core::i18n::tr(&format!(
                "warning: {c}: the wavelength grid is coarse (wavenumber step {gap:.4} > guide {need:.4} [1/um]); coefficients shift between wavelengths"
            )));
        }
        let chk = (!checks.is_empty()).then(|| check_model(c, &r.model, &checks, check_tol));
        rep.push(json!({
            "coat": c,
            "check": chk,
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
        return Err("no coating model could be made".into());
    }
    let e: Vec<(&str, &coating::CoatModel)> = models.iter().map(|(c, m)| (c.as_str(), m)).collect();
    lensbench_core::vfs::write(out, format_models(&e))
        .map_err(|x| format!("{}: {x}", out.display()))?;
    Ok(json!({ "out": out.display().to_string(), "models": rep }))
}

/// 屈折率だけの媒質（`AIR`・`n,k`）の屈折率。硝材の名前などは None
fn model_medium_n(m: &str) -> Option<f64> {
    if m == "AIR" {
        return Some(1.0);
    }
    m.split_once(',')?.0.trim().parse().ok()
}

/// モデル（OpticStudio 互換の位相）を、作った点とは別に測った表と比べる（`coat-model --check`）。
/// 透過と鏡の係数の差 |Δ| の最大（s・p の大きい方）と、その点。差が tol を超えた点（透過と鏡、硝材側の鏡も）を含む
/// モデルの波長の区間の中点（波数で）を `refine_wl_um` に返す（そこで測り足すと補間がよくなる）。
/// 屈折率だけの媒質の KEY だけを見る。後ろの媒質の全反射の近く（n2² − β² < 1e-3）とモデルの範囲の外（notimpl）は数えない
fn check_model(
    coat: &str,
    model: &coating::CoatModel,
    tabs: &[(String, coating::Measured)],
    tol: f64,
) -> Value {
    #[derive(Default)]
    struct Worst {
        d: f64,
        at: Value,
    }
    let (mut wt, mut wm) = (Worst::default(), Worst::default());
    let (mut points, mut skipped) = (0usize, 0usize);
    let mut bad: Vec<f64> = Vec::new();
    for (key, m) in tabs {
        let mut it = key.split('|');
        let (Some(c), Some(m1), Some(m2)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        if !c.eq_ignore_ascii_case(coat) || key.contains('@') {
            continue;
        }
        let mirror = m2 == "MIRROR";
        let Some(n1) = model_medium_n(m1) else {
            continue;
        };
        let n2 = if mirror {
            0.0
        } else {
            match model_medium_n(m2) {
                Some(n) => n,
                None => continue,
            }
        };
        for (iw, &wl) in m.wavelengths.iter().enumerate() {
            let Ok(f) = model.iface(key, wl, n1, n2, coating::PhaseConvention::OpticStudio) else {
                skipped += m.angles_deg.len();
                continue;
            };
            for (ia, &ang) in m.angles_deg.iter().enumerate() {
                let v = m.data[iw][ia];
                let b = n1 * ang.to_radians().sin();
                if !v[0].re.is_finite() || (!mirror && n2 * n2 - b * b < 1e-3) {
                    continue;
                }
                let Ok(p) = f.eval(ang.to_radians()) else {
                    skipped += 1;
                    continue;
                };
                let d = (p[0] - v[0]).norm().max((p[1] - v[1]).norm());
                points += 1;
                let w = if mirror { &mut wm } else { &mut wt };
                if d > w.d {
                    *w = Worst {
                        d,
                        at: json!({ "key": key, "wl_um": wl, "angle_deg": ang, "beta": b }),
                    };
                }
                if d > tol {
                    bad.push(wl);
                }
            }
        }
    }
    // 差の大きい点を含むモデルの波長の区間の中点（波数で）。モデルの波長ちょうどの点・範囲の外は足さない
    let ws = &model.wavelengths;
    let mut refine: Vec<f64> = Vec::new();
    for wl in bad {
        let i = ws.partition_point(|&x| x < wl);
        if i == 0 || i >= ws.len() || (ws[i] - wl).abs() < 1e-9 {
            continue;
        }
        let mid = 2.0 / (1.0 / ws[i - 1] + 1.0 / ws[i]);
        let mid = (mid * 1e9).round() / 1e9;
        if !refine.iter().any(|&x| (x - mid).abs() < 1e-9) {
            refine.push(mid);
        }
    }
    refine.sort_by(f64::total_cmp);
    json!({
        "tol": tol,
        "transmit_max": wt.d,
        "transmit_at": wt.at,
        "mirror_max": wm.d,
        "mirror_at": wm.at,
        "points": points,
        "notimpl": skipped,
        "refine_wl_um": refine,
    })
}

pub(crate) fn coat_compact(
    model: &std::path::Path,
    out: &std::path::Path,
) -> lensbench_core::Result<Value> {
    use lensbench_core::coating::format_models_bin;
    let b =
        lensbench_core::vfs::read(model).map_err(lensbench_core::Error::io_at(model.display()))?;
    let models = if coating::is_models_bin(&b) {
        coating::parse_models_bin(&b)
    } else {
        coating::parse_models(&String::from_utf8_lossy(&b))
    }
    .map_err(|e| e.at(model.display()))?;
    let rep: Vec<Value> = models
        .iter()
        .map(|(name, m)| {
            json!({
                "coat": name,
                "wavelengths": m.wavelengths.len(),
                "betas": m.betas.len(),
                "mirror_wavelengths": m.mirror.as_ref().map_or(0, |x| x.wavelengths.len()),
                "mirror_betas": m.mirror.as_ref().map_or(0, |x| x.betas.len()),
            })
        })
        .collect();
    let e: Vec<(&str, &coating::CoatModel)> = models.iter().map(|(c, m)| (c.as_str(), m)).collect();
    let bytes = format_models_bin(&e).map_err(|x| x.to_string())?;
    lensbench_core::vfs::write(out, &bytes).map_err(|x| format!("{}: {x}", out.display()))?;
    Ok(
        json!({ "out": out.display().to_string(), "bytes_in": b.len(), "bytes": bytes.len(), "models": rep }),
    )
}

/// 曲線の CSV を読む（[`Cmd::CoatDesign`] の列）
pub(crate) fn read_targets(
    p: &std::path::Path,
) -> lensbench_core::Result<Vec<lensbench_design::coat_design::Target>> {
    use lensbench_design::coat_design::{Pol, Quantity, Target};
    let txt = lensbench_core::vfs::read_to_string(p)
        .map_err(lensbench_core::Error::io_at(p.display()))?;
    let mut lines = txt
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    let head: Vec<String> = lines
        .next()
        .ok_or_else(|| format!("{}: empty", p.display()))?
        .split(',')
        .map(|x| x.trim().to_ascii_lowercase())
        .collect();
    let col = |n: &str| head.iter().position(|h| h == n);
    let (wl, scale_wl) = match (col("wl_um"), col("wl_nm")) {
        (Some(i), _) => (i, 1.0),
        (None, Some(i)) => (i, 1e-3),
        _ => return Err(format!("{}: no column wl_um or wl_nm", p.display()).into()),
    };
    let (val, scale_v) = match (col("value"), col("value_pct")) {
        (Some(i), _) => (i, 1.0),
        (None, Some(i)) => (i, 0.01),
        _ => return Err(format!("{}: no column value or value_pct", p.display()).into()),
    };
    let (ang, pol, qty, w) = (col("angle_deg"), col("pol"), col("quantity"), col("weight"));
    let mut v = Vec::new();
    for (ln, l) in lines.enumerate() {
        let c: Vec<&str> = l.split(',').map(str::trim).collect();
        let num = |i: usize| -> lensbench_core::Result<f64> {
            Ok(c.get(i)
                .and_then(|x| x.parse::<f64>().ok())
                .ok_or_else(|| format!("{}: number on line {}", p.display(), ln + 2))?)
        };
        let pol = match pol.and_then(|i| c.get(i)).map(|x| x.to_ascii_lowercase()) {
            None => Pol::Avg,
            Some(x) if x == "avg" || x.is_empty() => Pol::Avg,
            Some(x) if x == "s" => Pol::S,
            Some(x) if x == "p" => Pol::P,
            Some(x) => return Err(format!("{}: polarization {x} (s, p, avg)", p.display()).into()),
        };
        let q = match qty.and_then(|i| c.get(i)).map(|x| x.to_ascii_uppercase()) {
            None => Quantity::R,
            Some(x) if x == "R" || x.is_empty() => Quantity::R,
            Some(x) if x == "T" => Quantity::T,
            Some(x) => return Err(format!("{}: quantity {x} (R, T)", p.display()).into()),
        };
        v.push(Target {
            wl: num(wl)? * scale_wl,
            angle_deg: match ang {
                Some(i) => num(i)?,
                None => 0.0,
            },
            pol,
            q,
            value: num(val)? * scale_v,
            weight: match w {
                Some(i) => num(i)?,
                None => 1.0,
            },
        });
    }
    Ok(v)
}

pub(crate) fn coat_design(a: &CoatDesignArgs) -> Result<Value, crate::error::AppError> {
    use lensbench_design::coat_design::{fit_all, format_lbcs, pick, Material, Options};
    let mut ts = Vec::new();
    for p in a.targets {
        ts.extend(read_targets(p)?);
    }
    let file = match a.coating_file {
        Some(p) => coating::CoatingFile::load(p)?,
        None => coating::CoatingFile::default(),
    };
    // ガラスカタログは必要になったときに全部読む
    let mut glasses: Option<std::collections::HashMap<String, lensbench_core::glass::Glass>> = None;
    let mut glass = |name: &str| -> Option<lensbench_core::glass::Glass> {
        let g = glasses.get_or_insert_with(|| {
            let mut m = std::collections::HashMap::new();
            for d in lensbench_core::glass::catalog_dirs(a.glass_dir) {
                for f in lensbench_core::vfs::read_dir(&d).unwrap_or_default() {
                    if f.extension().is_some_and(|e| e.eq_ignore_ascii_case("agf")) {
                        if let Ok(x) = lensbench_core::glass::load_agf(&f) {
                            for (k, v) in x {
                                m.entry(k).or_insert(v);
                            }
                        }
                    }
                }
            }
            m
        });
        g.get(name).cloned()
    };
    // 材料: 膜ファイルの MATE、なければガラス（0.2〜2.5 µm を 5 nm 刻みの表にして .lbcs に MATE も書く）
    let mut mats = Vec::new();
    let mut with_mate = Vec::new();
    let mut lookup = |name: &str| -> lensbench_core::Result<(Material, bool)> {
        if let Some(r) = file.mate.get(name) {
            let mut rows = r.clone();
            rows.sort_by(|x, y| x[0].total_cmp(&y[0]));
            return Ok((
                Material {
                    name: name.into(),
                    rows,
                },
                false,
            ));
        }
        let g = glass(name).ok_or_else(|| {
            format!("material {name} is neither in the coating file nor in the glass catalogs")
        })?;
        let rows = (0..=460)
            .map(|i| {
                let wl = 0.2 + 0.005 * i as f64;
                [wl, g.index(wl, 20.0, 1.0, false), 0.0]
            })
            .collect();
        Ok((
            Material {
                name: name.into(),
                rows,
            },
            true,
        ))
    };
    for m in a.materials {
        let (mat, from_glass) = lookup(m)?;
        if from_glass {
            with_mate.push(mats.len());
        }
        mats.push(mat);
    }
    let sub: Box<dyn Fn(f64) -> coating::C64> = match a.substrate.parse::<f64>() {
        Ok(n) => Box::new(move |_| coating::C64::new(n, 0.0)),
        Err(_) => {
            let (m, _) = lookup(a.substrate)?;
            Box::new(move |wl| m.index(wl))
        }
    };
    let bad_layers =
        |l: &str| format!("--layers {l}: expected a number of layers N or a range N..M");
    let (lo, hi) = match a.layers.split_once("..") {
        Some((x, y)) => (
            x.trim()
                .parse::<usize>()
                .map_err(|_| bad_layers(a.layers))?,
            y.trim()
                .parse::<usize>()
                .map_err(|_| bad_layers(a.layers))?,
        ),
        None => {
            let n = a
                .layers
                .trim()
                .parse::<usize>()
                .map_err(|_| bad_layers(a.layers))?;
            (n, n)
        }
    };
    let o = Options {
        materials: mats.clone(),
        layers: (lo, hi),
        starts: a.starts,
        d_max: a.d_max,
        ..Default::default()
    };
    let all = fit_all(&ts, &*sub, &o)?;
    let d = pick(&all, a.slack).ok_or("can't be fitted")?;
    let lay = |d: &lensbench_design::coat_design::Design| -> Vec<Value> {
        d.layers
            .iter()
            .map(|&(k, t)| json!({ "material": mats[k].name, "d_um": t }))
            .collect()
    };
    let mut written = None;
    if let Some(out) = a.out {
        let note = format!(
            "近似の膜: lensbench coat-design で分光の曲線 {} 点に合わせた（RMS {:.3e}、最大 {:.3e}）。\n\
             曲線の外（位相・別の角度・硝材の側から）は元の膜と違いうる",
            ts.len(),
            d.rms,
            d.max_err
        );
        let txt = format_lbcs(a.name, d, &mats, &with_mate, &note);
        lensbench_core::vfs::write(out, txt.as_bytes())
            .map_err(lensbench_core::Error::io_at(out.display()))?;
        written = Some(out.display().to_string());
    }
    Ok(json!({
        "name": a.name,
        "points": ts.len(),
        "layers": lay(d),
        "rms": d.rms,
        "max_err": d.max_err,
        "by_layers": all.iter().map(|x| json!({ "layers": x.layers.len(), "rms": x.rms, "max_err": x.max_err })).collect::<Vec<_>>(),
        "out": written,
    }))
}

/// プロジェクト（`glass_catalogs`、書かなければ既定の SCHOTT）・系ファイル（`system.catalogs` の名前）のガラスのカタログ
fn json_catalogs(file: &std::path::Path) -> Vec<String> {
    let Ok(v) =
        std::fs::read(file).map(|b| serde_json::from_slice::<Value>(&b).unwrap_or_default())
    else {
        return Vec::new();
    };
    let names = |a: &Value, key: Option<&str>| -> Vec<String> {
        a.as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| key.map_or(x, |k| &x[k]).as_str().map(str::to_string))
            .collect()
    };
    if v["system"].is_object() {
        names(&v["system"]["catalogs"], Some("name"))
    } else if v["glass_catalogs"].is_array() {
        names(&v["glass_catalogs"], None)
    } else {
        vec!["SCHOTT".to_string()]
    }
}

/// 面の膜の一覧（coatings）。.zmx は多重構成ごと、プロジェクトの .json は状態ごと、系ファイルは 1 つ
pub(crate) fn coatings(a: &CoatingsArgs) -> Result<Value, AppError> {
    let (file, glass_dir, lens_catalog, tables) =
        (a.file.as_path(), &a.glass_dir, &a.lens_catalog, &a.coatings);
    use lensbench_project::open::FileKind;
    let e = AppError::from;
    // 構成（番号、プロジェクトの状態の名前）ごとの系
    let mut systems: Vec<(usize, Option<String>, lensbench_core::System)> = Vec::new();
    let (coating_file, catalogs) = if FileKind::of(file) == FileKind::Zmx {
        let txt = glass::read_text(file).map_err(AppError::io_at(file))?;
        let raw = zmx::parse_text(&txt).map_err(e)?;
        for cfg in 1..=raw.configurations() {
            let opt = zmx::LoadOptions {
                cfg,
                catalog_dirs: glass_dir.to_vec(),
                ..Default::default()
            };
            let s = zmx::load_with(file, &opt).map_err(|x| x.at(format!("configuration {cfg}")))?;
            systems.push((cfg, None, s));
        }
        (raw.coating_file_name(), raw.catalogs())
    } else {
        let spec = |state: Option<String>| lensbench_engine::SysSpec {
            file: file.to_path_buf(),
            glass_dir: glass_dir.to_vec(),
            lens_catalog: lens_catalog.to_vec(),
            cfg: 1,
            state,
            ..Default::default()
        };
        let states = lensbench_engine::load::state_names(&spec(None));
        if states.is_empty() {
            systems.push((
                1,
                None,
                lensbench_engine::load::load_file_noted(&spec(None)).map_err(e)?,
            ));
        } else {
            for (k, st) in states.into_iter().enumerate() {
                let s = lensbench_engine::load::load_file_noted(&spec(Some(st.clone())))
                    .map_err(|x| x.at(format!("state {st}")))?;
                systems.push((k + 1, Some(st), s));
            }
        }
        let name = systems
            .first()
            .and_then(|x| x.2.coating_file.as_ref())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        (name, json_catalogs(file))
    };
    let mut cfgs = Vec::new();
    // 実測表が要る境界 → (コンフィグ, 波長)
    let mut need: Vec<(String, Vec<usize>, Vec<f64>)> = Vec::new();
    let mut found = None;
    // 膜ファイルのフォルダから膜の名前で自動で読んだもの（膜, パス）
    let mut auto: Vec<AutoCoat> = Vec::new();
    // 登録した膜の一覧から入れたもの（膜, 一覧）と注意
    let mut registered: Vec<(String, String)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (cfg, state, s) in &systems {
        let cfg = *cfg;
        found = s.coating_file.clone();
        let CoatInterfaces {
            list: v,
            needs: keys,
            auto: au,
            registered: re,
            notes: no,
        } = coat_interfaces(s, tables)?;
        for r in re {
            if !registered.contains(&r) {
                registered.push(r);
            }
        }
        for n in no {
            if !notes.contains(&n) {
                notes.push(n);
            }
        }
        for a in au {
            if !auto.iter().any(|b| b.0 == a.0 && b.1 == a.1) {
                auto.push(a);
            }
        }
        for k in keys {
            let i = match need.iter().position(|n| n.0 == k) {
                Some(i) => i,
                None => {
                    need.push((k, Vec::new(), Vec::new()));
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
        let mut c = json!({ "cfg": cfg, "wavelengths_um": s.wavelengths, "interfaces": v });
        if let Some(st) = state {
            c["state"] = json!(st);
        }
        cfgs.push(c);
    }
    Ok(json!({
        "coating_file": coating_file,
        "coating_file_path": found.map(|p| p.display().to_string()),
        "catalogs": catalogs,
        "auto_coatings": auto.iter().map(|(c, p, n)| json!({ "coat": c, "path": p, "note": n })).collect::<Vec<_>>(),
        "registered_coatings": registered.iter().map(|(c, k)| json!({ "coat": c, "catalog": k })).collect::<Vec<_>>(),
        "notes": notes,
        "configurations": cfgs,
        "needs_table": need.iter().map(|(k, c, w)| json!({ "key": k, "cfgs": c, "wavelengths_um": w })).collect::<Vec<_>>(),
    }))
}
