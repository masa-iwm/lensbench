//! 膜の道具のコマンド（coat-design・coat-model・coat-kinks・coat-compact）

use super::*;

pub(crate) fn coat_kinks(tables: &[PathBuf], only: &[String]) -> Result<Value, String> {
    use lensbench_design::coat_fit::detect_kinks;
    let mut keys = Vec::new();
    // 膜ごとに、KEY で見つかった折れ目をまとめる（5e-4 µm 以内は同じ）
    let mut per: Vec<(String, Vec<f64>)> = Vec::new();
    for t in tables {
        let txt =
            lensbench_core::vfs::read_to_string(t).map_err(|e| format!("{}: {e}", t.display()))?;
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

pub(crate) fn coat_model(
    tables: &[PathBuf],
    out: &std::path::Path,
    only: Option<&str>,
    kinks: Option<&str>,
    tol_psi: f64,
    note: Option<&str>,
) -> Result<Value, String> {
    let mut kinks: Vec<f64> = match kinks {
        None => Vec::new(),
        Some(k) => k
            .split(',')
            .map(|x| x.trim().parse::<f64>().map_err(|_| format!("--kinks: {x}")))
            .collect::<Result<_, _>>()?,
    };
    kinks.sort_by(f64::total_cmp);
    use lensbench_design::coat_fit::{fit_model, format_models, FitOptions};
    let mut all = Vec::new();
    for t in tables {
        let txt =
            lensbench_core::vfs::read_to_string(t).map_err(|e| format!("{}: {e}", t.display()))?;
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
    lensbench_core::vfs::write(out, format_models(&e))
        .map_err(|x| format!("{}: {x}", out.display()))?;
    Ok(json!({ "out": out.display().to_string(), "models": rep }))
}

pub(crate) fn coat_compact(
    model: &std::path::Path,
    out: &std::path::Path,
) -> Result<Value, String> {
    use lensbench_design::coat_fit::format_models_bin;
    let b = lensbench_core::vfs::read(model).map_err(|e| format!("{}: {e}", model.display()))?;
    let models = if b.starts_with(coating::MODEL_BIN_HEADER) {
        coating::parse_models_bin(&b)
    } else {
        coating::parse_models(&String::from_utf8_lossy(&b))
    }
    .map_err(|e| format!("{}: {e}", model.display()))?;
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
) -> Result<Vec<lensbench_design::coat_design::Target>, String> {
    use lensbench_design::coat_design::{Pol, Quantity, Target};
    let txt =
        lensbench_core::vfs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut lines = txt
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    let head: Vec<String> = lines
        .next()
        .ok_or_else(|| format!("{}: 空", p.display()))?
        .split(',')
        .map(|x| x.trim().to_ascii_lowercase())
        .collect();
    let col = |n: &str| head.iter().position(|h| h == n);
    let (wl, scale_wl) = match (col("wl_um"), col("wl_nm")) {
        (Some(i), _) => (i, 1.0),
        (None, Some(i)) => (i, 1e-3),
        _ => return Err(format!("{}: 列 wl_um か wl_nm がない", p.display())),
    };
    let (val, scale_v) = match (col("value"), col("value_pct")) {
        (Some(i), _) => (i, 1.0),
        (None, Some(i)) => (i, 0.01),
        _ => return Err(format!("{}: 列 value か value_pct がない", p.display())),
    };
    let (ang, pol, qty, w) = (col("angle_deg"), col("pol"), col("quantity"), col("weight"));
    let mut v = Vec::new();
    for (ln, l) in lines.enumerate() {
        let c: Vec<&str> = l.split(',').map(str::trim).collect();
        let num = |i: usize| -> Result<f64, String> {
            c.get(i)
                .and_then(|x| x.parse::<f64>().ok())
                .ok_or_else(|| format!("{}: {} 行目の数値", p.display(), ln + 2))
        };
        let pol = match pol.and_then(|i| c.get(i)).map(|x| x.to_ascii_lowercase()) {
            None => Pol::Avg,
            Some(x) if x == "avg" || x.is_empty() => Pol::Avg,
            Some(x) if x == "s" => Pol::S,
            Some(x) if x == "p" => Pol::P,
            Some(x) => return Err(format!("{}: 偏光 {x}（s・p・avg）", p.display())),
        };
        let q = match qty.and_then(|i| c.get(i)).map(|x| x.to_ascii_uppercase()) {
            None => Quantity::R,
            Some(x) if x == "R" || x.is_empty() => Quantity::R,
            Some(x) if x == "T" => Quantity::T,
            Some(x) => return Err(format!("{}: 量 {x}（R・T）", p.display())),
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

pub(crate) fn coat_design(a: &CoatDesignArgs) -> Result<Value, String> {
    use lensbench_design::coat_design::{fit_all, format_lbcs, pick, Material, Options};
    let mut ts = Vec::new();
    for p in a.targets {
        ts.extend(read_targets(p)?);
    }
    let file = match a.coating_file {
        Some(p) => coating::CoatingFile::load(p).map_err(|e| e.to_string())?,
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
    let mut lookup = |name: &str| -> Result<(Material, bool), String> {
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
        let g = glass(name)
            .ok_or_else(|| format!("材料 {name} が膜ファイルにもガラスカタログにもない"))?;
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
    let (lo, hi) = match a.layers.split_once("..") {
        Some((x, y)) => (
            x.trim()
                .parse::<usize>()
                .map_err(|_| format!("--layers {}", a.layers))?,
            y.trim()
                .parse::<usize>()
                .map_err(|_| format!("--layers {}", a.layers))?,
        ),
        None => {
            let n = a
                .layers
                .trim()
                .parse::<usize>()
                .map_err(|_| format!("--layers {}", a.layers))?;
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
    let all = fit_all(&ts, &*sub, &o).map_err(|e| e.to_string())?;
    let d = pick(&all, a.slack).ok_or("当てはめられない")?;
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
            .map_err(|e| format!("{}: {e}", out.display()))?;
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
