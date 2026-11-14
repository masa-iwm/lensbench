//! 最適化・公差・ズーム軌跡・差し替え探索（走っている間の進み具合と止め方も）

use super::*;

// ------------------------------------------------------------------------------------------ 最適化

/// 最適化の進み具合（同じプロセスの GUI が読む、[`optimize_progress`]）
pub(super) struct OptRun {
    /// 反復ごとの評価関数の値（初期値を含む）
    history: Vec<f64>,
}

impl Session {
    /// 走っている最適化の、反復ごとの評価関数の値（走っていなければ None）。同じプロセスから呼ぶ（GUI の進み具合）
    pub fn optimize_progress(&self) -> Option<Vec<f64>> {
        self.opt_run
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|r| r.history.clone())
    }

    /// 走っている最適化を止める（次の反復の終わりで止まり、そこまでの結果を返す）。同じプロセスから呼ぶ
    pub fn optimize_cancel(&self) {
        self.opt_cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// [`global`] の場の [`Session::optimize_progress`]
pub fn optimize_progress() -> Option<Vec<f64>> {
    global().optimize_progress()
}

/// [`global`] の場の [`Session::optimize_cancel`]
pub fn optimize_cancel() {
    global().optimize_cancel()
}

/// 構成（.zmx の構成番号か、プロジェクトの状態の名前）の系。読み込んだ系と同じ構成なら読み込んだ系（set・edit の後）、
/// ほかはファイルから読み、読み込んだ後の変更を当てる
pub(super) fn config_system(
    sess: &Session,
    b: &Value,
    cur: &Arc<System>,
    spec: &LoadSpec,
    notes: &mut Vec<String>,
) -> Result<System, RpcErr> {
    let mut sp = spec.clone();
    if is_project(&spec.file) {
        let name = b
            .as_str()
            .ok_or_else(|| RpcErr::params("configs はプロジェクトの状態の名前の並び"))?;
        if spec.state.as_deref() == Some(name) {
            return Ok((**cur).clone());
        }
        sp.state = Some(name.to_string());
    } else {
        let c = b
            .as_u64()
            .or_else(|| b.as_str().and_then(|x| x.parse().ok()))
            .ok_or_else(|| RpcErr::params("configs は .zmx の構成番号（1 始まり）の並び"))?
            as usize;
        if c == spec.cfg {
            return Ok((**cur).clone());
        }
        sp.cfg = c;
    }
    let s = load_file(&sp)?;
    let beam = if is_project(&spec.file) {
        format!("状態 {b}")
    } else {
        format!("構成 {b}")
    };
    Ok(replay(s, &sess.current_log(), &beam, notes)?)
}

/// 最適化。params:
/// - vars: 変数の並び。各要素は "7.t" か {"name": "7.t", "lo": 下限, "hi": 上限, "configs": [構成の番号（1 始まり）]}
/// - merit: 既定の評価関数 "spot"・"wavefront"・null（なし）。rings（3）・reference（"centroid"）
/// - terms: 足す項（`lensbench_project::merit` の JSON）
/// - configs: 構成（.zmx の構成番号か状態の名前）の並び。省けば読み込んだ系だけ。項・変数の構成の番号はこの並びの順
/// - iters（50）・jac（"auto"・"fd"・"forward"）
/// - apply: true なら最適化した値を読み込んだ系に set する（読み込んだ系の構成に効く変数だけ）
///
/// 同じプロセスからは [`optimize_progress`] で進み具合を読み、[`optimize_cancel`] で止められる
pub(super) fn optimize(sess: &Session, p: &Value) -> Result<Value, RpcErr> {
    use lensbench_core::optim;
    let (cur, spec) = sess.current()?;
    let mut notes = Vec::new();
    let (systems, cur_idx) = match p.get("configs").and_then(Value::as_array) {
        Some(bs) if !bs.is_empty() => {
            let mut v = Vec::new();
            let mut cur_idx = None;
            for b in bs {
                let same = if is_project(&spec.file) {
                    spec.state.as_deref() == b.as_str()
                } else {
                    b.as_u64()
                        .or_else(|| b.as_str().and_then(|x| x.parse().ok()))
                        == Some(spec.cfg as u64)
                };
                if same && cur_idx.is_none() {
                    cur_idx = Some(v.len());
                }
                v.push(config_system(sess, b, &cur, &spec, &mut notes)?);
            }
            (v, cur_idx)
        }
        _ => (vec![(*cur).clone()], Some(0)),
    };
    let mut prob = optim::Problem::new(systems);
    let vars = p["vars"]
        .as_array()
        .ok_or_else(|| RpcErr::params("optimize には vars（変数の並び）"))?;
    for v in vars {
        let (name, lo, hi, cfgs) = match v {
            Value::String(n) => (n.as_str(), None, None, None),
            Value::Object(_) => (
                v["name"]
                    .as_str()
                    .ok_or_else(|| RpcErr::params("vars の要素に name"))?,
                opt_f64(v, "lo")?,
                opt_f64(v, "hi")?,
                match v.get("configs") {
                    None | Some(Value::Null) => None,
                    Some(c) => Some(
                        c.as_array()
                            .ok_or_else(|| RpcErr::params("vars の configs は番号の並び"))?
                            .iter()
                            .map(|x| match x.as_u64() {
                                Some(k) if k >= 1 && (k as usize) <= prob.configs.len() => {
                                    Ok(k as usize - 1)
                                }
                                _ => Err(RpcErr::params(format!(
                                    "vars の configs {x}（1〜{}）",
                                    prob.configs.len()
                                ))),
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                },
            ),
            _ => return Err(RpcErr::params("vars の要素は \"7.t\" か {\"name\": ...}")),
        };
        let first = cfgs.as_ref().and_then(|c| c.first().copied()).unwrap_or(0);
        let param = prob.configs[first].param(name)?;
        let mut var = optim::Var::new(param)
            .bounds(lo.unwrap_or(f64::NEG_INFINITY), hi.unwrap_or(f64::INFINITY));
        if let Some(c) = cfgs {
            var = var.in_configs(c);
        }
        prob.vars.push(var);
    }
    let reference = match p["reference"].as_str().unwrap_or("centroid") {
        "centroid" => lensbench_core::sweep::RmsRef::Centroid,
        "chief" => lensbench_core::sweep::RmsRef::Chief,
        r => {
            return Err(RpcErr::params(format!(
                "reference は centroid か chief（{r}）"
            )))
        }
    };
    let rings = opt_usize(p, "rings")?.unwrap_or(3);
    match p.get("merit").map(|m| m.as_str()) {
        None | Some(Some("spot")) => prob.add_default(optim::DefaultMerit::Spot, rings, reference),
        Some(Some("wavefront")) => {
            prob.add_default(optim::DefaultMerit::Wavefront, rings, reference)
        }
        Some(None) | Some(Some("none")) => {}
        Some(Some(m)) => {
            return Err(RpcErr::params(format!(
                "merit は spot・wavefront・none（{m}）"
            )))
        }
    }
    let n_default = prob.terms.len();
    if let Some(t) = p.get("terms").filter(|t| !t.is_null()) {
        let mut t = t.clone();
        sess.files.check_term_paths(&mut t)?;
        let ts =
            lensbench_project::merit::terms_from_json(&t, &prob.configs).map_err(RpcErr::params)?;
        prob.terms.extend(ts);
    }
    let j = p["jac"].as_str().unwrap_or("auto");
    prob.jac = optim::JacMode::parse(j).ok_or_else(|| {
        RpcErr::params(format!("jac は {} のどれか（{j}）", optim::JacMode::NAMES))
    })?;
    prob.check()?;
    let o = optim::DlsOpts {
        max_iter: opt_usize(p, "iters")?.unwrap_or(50),
        ..Default::default()
    };
    let m0 = prob.merit(&prob.x0()?)?;
    sess.opt_cancel
        .store(false, std::sync::atomic::Ordering::SeqCst);
    *sess.opt_run.lock().unwrap_or_else(|e| e.into_inner()) = Some(OptRun { history: vec![m0] });
    let r = optim::dls_with(&prob, &o, |pr| {
        if let Some(run) = sess
            .opt_run
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            run.history.push(pr.merit);
        }
        !sess.opt_cancel.load(std::sync::atomic::Ordering::SeqCst)
    });
    *sess.opt_run.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let r = r?;
    let names: Vec<String> = prob
        .vars
        .iter()
        .map(|v| prob.configs[0].param_name(v.param))
        .collect();
    let tv0 = prob.term_values(&r.x0)?;
    let tv = prob.term_values(&r.x)?;
    let stop = r.stop.as_str();
    // 読み込んだ系に当てられる変数（読み込んだ系の構成に効くもの）
    let applicable: Vec<bool> = prob
        .vars
        .iter()
        .map(|v| cur_idx.is_some_and(|c| v.configs.as_ref().map_or(true, |cs| cs.contains(&c))))
        .collect();
    let mut applied = false;
    if p["apply"].as_bool() == Some(true) {
        let ch: Vec<(lensbench_core::param::Param, f64)> = prob
            .vars
            .iter()
            .zip(&r.x)
            .zip(&applicable)
            .filter(|(_, &a)| a)
            .map(|((v, &x), _)| (v.param, x))
            .collect();
        if !ch.is_empty() {
            let n = cur.with_params(&ch)?;
            let rec = ch
                .iter()
                .map(|(q, v)| Ok((n.param_name(*q), cur.get_param(*q)?, *v)))
                .collect::<Result<Vec<_>, lensbench_core::Error>>()?;
            store_changed(sess, n, spec, Change::Set(rec));
            applied = true;
        }
    }
    let terms: Vec<Value> = (0..prob.terms.len())
        .map(|i| json!({ "default": i < n_default, "value0": tv0[i], "value": tv[i] }))
        .collect();
    Ok(json!({
        "names": names,
        "x0": r.x0,
        "x": r.x,
        "applicable": applicable,
        "applied": applied,
        "merit0": r.merit0,
        "merit": r.merit,
        "history": r.history,
        "iterations": r.iterations,
        "evals": r.evals,
        "stop": stop,
        "terms": terms,
        "notes": notes,
    }))
}

// ------------------------------------------------------------------------------------ 公差

// 公差のモンテカルロの進み具合（同じプロセスの GUI が読む、[`tolerance_progress`]）
impl Session {
    /// 走っている公差のモンテカルロの (終えた評価, 評価の数)（感度の 2 × 公差の数 + 試行の数。走っていなければ None）
    pub fn tolerance_progress(&self) -> Option<(usize, usize)> {
        *self.tol_run.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 走っている公差のモンテカルロを止める（走っている試行は終わらせ、そこまでの結果を返す）。同じプロセスから呼ぶ
    pub fn tolerance_cancel(&self) {
        self.tol_cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// [`global`] の場の [`Session::tolerance_progress`]
pub fn tolerance_progress() -> Option<(usize, usize)> {
    global().tolerance_progress()
}

/// [`global`] の場の [`Session::tolerance_cancel`]
pub fn tolerance_cancel() {
    global().tolerance_cancel()
}

/// モンテカルロ公差。params は `lensbench tolerance --spec` の JSON（`lensbench_project::tolerance`）に加えて
/// configs（構成の番号か状態の名前の並び。optimize と同じ。省けば読み込んだ系だけ）。読み込んだ系（set・edit の後）を使う。
/// 同じプロセスからは [`tolerance_progress`] で進み具合を読み、[`tolerance_cancel`] で止められる
pub(super) fn tolerance(sess: &Session, p: &Value) -> Result<Value, RpcErr> {
    let (cur, spec) = sess.current()?;
    let mut notes = Vec::new();
    let systems = match p.get("configs").and_then(Value::as_array) {
        Some(bs) if !bs.is_empty() => bs
            .iter()
            .map(|b| config_system(sess, b, &cur, &spec, &mut notes))
            .collect::<Result<Vec<_>, _>>()?,
        _ => vec![(*cur).clone()],
    };
    let (vendors, reversed) = if is_project(&spec.file) {
        lensbench_core::vfs::read_to_string(&spec.file)
            .ok()
            .and_then(|t| lensbench_project::Project::from_json(&t).ok())
            .map(|p| {
                (
                    lensbench_project::tolerance::catalog_vendors(&p),
                    lensbench_project::tolerance::reversed_lenses(&p),
                )
            })
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let mut v = p.clone();
    if let Some(o) = v.as_object_mut() {
        o.remove("configs");
        if let Some(t) = o.get_mut("terms") {
            sess.files.check_term_paths(t)?;
        }
    }
    let ms = lensbench_project::tolerance::spec_from_json_with(&v, systems, &vendors, &reversed)
        .map_err(RpcErr::params)?;
    let oo = lensbench_project::tolerance::out_opts(&v).map_err(RpcErr::params)?;
    sess.tol_cancel
        .store(false, std::sync::atomic::Ordering::SeqCst);
    *sess.tol_run.lock().unwrap_or_else(|e| e.into_inner()) = Some((0, 0));
    let run = || {
        lensbench_core::tolerance::run_with(&ms, |k, n| {
            let mut g = sess.tol_run.lock().unwrap_or_else(|e| e.into_inner());
            // 並列の試行は終わる順が前後するので、大きい方を残す
            let k = g.map_or(k, |(d, _)| d.max(k));
            *g = Some((k, n));
            !sess.tol_cancel.load(std::sync::atomic::Ordering::SeqCst)
        })
    };
    let r = match oo.threads {
        Some(t) => lensbench_core::with_threads(Some(t), run),
        None => run(),
    };
    *sess.tol_run.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let r = r?;
    let mut out = lensbench_project::tolerance::result_json(&ms, &r, &oo);
    out["notes"] = json!(notes);
    Ok(out)
}

// ------------------------------------------------------------------------------------ ズーム軌跡

/// ズーム軌跡（CLI の `zoom`）。params:
/// - configs: 状態（プロジェクトなら状態の名前、.zmx なら構成の番号）の並び。省けばすべて
/// - vars: 状態ごとに解くパラメータ。"G2.shift"・"G2.shift=-5:20" か {"name", "lo", "hi"}
/// - efl か pmag: 状態ごとの目標の並び（数か null。状態の数だけ）。省けば目標なし
/// - steps（0）・merit（"spot"）・reference（"centroid"）・rings（3）・iters（50）・tol（1e-8）
///
/// 読み込んだ後の値の変更（set・edit）は各状態の系にも当てる
pub(super) fn zoom(sess: &Session, p: &Value) -> Result<Value, RpcErr> {
    let (cur, spec) = sess.current()?;
    let mut notes = Vec::new();
    let cfgs: Vec<Value> = match p.get("configs").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a.clone(),
        _ => {
            let c = configs(sess)?;
            match c["states"].as_array() {
                Some(st) if !st.is_empty() => st.clone(),
                _ => (1..=c["cfgs"].as_u64().unwrap_or(1))
                    .map(|k| json!(k))
                    .collect(),
            }
        }
    };
    let systems = cfgs
        .iter()
        .map(|b| config_system(sess, b, &cur, &spec, &mut notes))
        .collect::<Result<Vec<_>, _>>()?;
    let names: Vec<String> = cfgs
        .iter()
        .map(|b| b.as_str().map_or_else(|| b.to_string(), String::from))
        .collect();
    let vars = p
        .get("vars")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| RpcErr::params("vars（解くパラメータの並び）がない"))?
        .iter()
        .map(|v| match v {
            Value::String(s) => Ok(s.clone()),
            Value::Object(o) => {
                let name = o
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcErr::params(format!("vars の {v} に name がない")))?;
                let b = |k: &str| o.get(k).and_then(Value::as_f64).map(|x| x.to_string());
                Ok(match (b("lo"), b("hi")) {
                    (None, None) => name.to_string(),
                    (lo, hi) => format!(
                        "{name}={}:{}",
                        lo.unwrap_or_default(),
                        hi.unwrap_or_default()
                    ),
                })
            }
            _ => Err(RpcErr::params(format!(
                "vars の {v} は文字列か {{name, lo, hi}}"
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let targets = |k: &str| -> Result<Option<String>, RpcErr> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(Value::Array(a)) => a
                .iter()
                .map(|x| match x {
                    Value::Null => Ok("-".to_string()),
                    Value::Number(n) => Ok(n.to_string()),
                    _ => Err(RpcErr::params(format!("{k} の {x} は数か null"))),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|v| Some(v.join(","))),
            Some(x) => Err(RpcErr::params(format!("{k} は数（か null）の並び（{x}）"))),
        }
    };
    let (efl, pmag) = (targets("efl")?, targets("pmag")?);
    if efl.is_some() && pmag.is_some() {
        return Err(RpcErr::params("efl と pmag は一方だけ"));
    }
    let str_of = |k: &str, d: &str| p.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
    let a = crate::zoom::ZoomOpts {
        vars,
        efl,
        pmag,
        steps: opt_usize(p, "steps")?.unwrap_or(0),
        merit: str_of("merit", "spot"),
        reference: str_of("reference", "centroid"),
        rings: opt_usize(p, "rings")?.unwrap_or(3),
        iters: opt_usize(p, "iters")?.unwrap_or(50),
        tol: p.get("tol").and_then(Value::as_f64).unwrap_or(1e-8),
    };
    let mut out = crate::zoom::zoom_systems(&systems, &names, &a).map_err(RpcErr::params)?;
    out["notes"] = json!(notes);
    Ok(out)
}

// ------------------------------------------------------------------------------------ 差し替え探索

// 差し替え探索の進み具合（同じプロセスの GUI が読む、[`swap_progress`]）
impl Session {
    /// 走っている差し替え探索の (終えた組, 組の数)（走っていなければ None）。同じプロセスから呼ぶ
    pub fn swap_progress(&self) -> Option<(usize, usize)> {
        *self.swap_run.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 走っている差し替え探索を止める（走っている組は終わらせ、そこまでの結果を返す）。同じプロセスから呼ぶ
    pub fn swap_cancel(&self) {
        self.swap_cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// [`global`] の場の [`Session::swap_progress`]
pub fn swap_progress() -> Option<(usize, usize)> {
    global().swap_progress()
}

/// [`global`] の場の [`Session::swap_cancel`]
pub fn swap_cancel() {
    global().swap_cancel()
}

/// カタログレンズの差し替え探索（GUI）。params は `lensbench swap --spec` の JSON（slots・vars・merit・
/// reference・rings・iters・efl・state・max_count）に加えて:
/// - project: プロジェクトの JSON（省けば読み込んだファイル。読み込んだ後の変更は使わない）
/// - top: 返す組の数（50。0 なら全部）
/// - threads: スレッド数
/// - count_only: true なら回さず、組の数と所ごとの候補だけ返す
///
/// 読み込んだ系のガラス・レンズカタログのフォルダを使う。同じプロセスからは [`swap_progress`] で進み具合を読み、
/// [`swap_cancel`] で止められる
pub(super) fn swap(sess: &Session, p: &Value) -> Result<Value, RpcErr> {
    use lensbench_project::swap::{SwapSearch, SwapSpec};
    let (_, ls) = sess.current()?;
    let proj = match p.get("project").filter(|v| !v.is_null()) {
        // 中の zmx・カタログ・膜のパスを check_path で確かめられないので、要求のファイルを制限するときは断る
        Some(_) if !matches!(sess.files, Files::Any) => {
            return Err(RpcErr::app(
                "swap の project は --allow-remote・--root では使えない（中のパスを確かめられない。読み込んだファイルで回す）",
            ))
        }
        Some(v) => serde_json::from_value::<lensbench_project::Project>(v.clone())
            .map_err(|e| RpcErr::params(format!("project: {e}")))?,
        None => {
            let txt = lensbench_core::vfs::read_to_string(&ls.file)
                .map_err(|e| RpcErr::app(format!("{}: {e}", ls.file.display())))?;
            lensbench_project::Project::from_json(&txt)
                .map_err(|e| RpcErr::app(format!("読み込んだ系がプロジェクトでない（{e}）")))?
        }
    };
    let mut sp = p.clone();
    if let Some(o) = sp.as_object_mut() {
        for k in ["project", "top", "threads", "count_only"] {
            o.remove(k);
        }
        let multi = o
            .get("states")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
        if !multi && !o.contains_key("state") && p.get("project").is_none() {
            if let Some(s) = &ls.state {
                o.insert("state".into(), json!(s));
            }
        }
    }
    let spec: SwapSpec = serde_json::from_value(sp)
        .map_err(|e| RpcErr::params(format!("差し替え探索の設定: {e}")))?;
    let cat = lensbench_core::catalog::LensCatalog::load_registered(&ls.lens_catalog)?;
    let env = lensbench_project::Env {
        base: ls
            .file
            .parent()
            .map(|x| x.to_path_buf())
            .unwrap_or_default(),
        glass_dirs: ls.glass_dir.clone(),
        catalog: None,
    };
    let s = SwapSearch::new(&proj, &env, &cat, spec)?;
    let n = s.count();
    let slots: Vec<Value> = s
        .picks
        .iter()
        .map(|v| {
            json!(v
                .iter()
                .map(|k| json!({
                    "vendor": k.lens.vendor, "part": k.lens.part, "reversed": k.reversed,
                    "efl": k.lens.efl, "diameter": k.lens.diameter,
                }))
                .collect::<Vec<_>>())
        })
        .collect();
    if p["count_only"].as_bool() == Some(true) {
        return Ok(json!({ "count": n, "slots": slots }));
    }
    let rows: Mutex<Vec<Value>> = Mutex::new(Vec::new());
    sess.swap_cancel
        .store(false, std::sync::atomic::Ordering::SeqCst);
    *sess.swap_run.lock().unwrap_or_else(|e| e.into_inner()) = Some((0, n));
    let show = |pr: lensbench_core::runs::Progress| {
        *sess.swap_run.lock().unwrap_or_else(|e| e.into_inner()) = Some((pr.done, pr.total));
    };
    let o = lensbench_core::runs::RunOpts {
        threads: opt_usize(p, "threads")?,
        progress: Some(&show),
        cancel: Some(&sess.swap_cancel),
        header: "{}".into(),
        ..Default::default()
    };
    let t0 = web_time::Instant::now();
    // 粗いふるい（prescreen）があれば残った組だけ
    let idx = s.screened(opt_usize(p, "threads")?);
    *sess.swap_run.lock().unwrap_or_else(|e| e.into_inner()) = Some((0, idx.len()));
    let r = lensbench_core::runs::run(idx.len(), 0, &o, |j, _| {
        let v = s.eval(idx[j]);
        rows.lock().unwrap_or_else(|e| e.into_inner()).push(v);
        Ok("{}".into())
    });
    *sess.swap_run.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let sum = r?;
    let mut rows = rows.into_inner().unwrap_or_else(|e| e.into_inner());
    lensbench_project::swap::sort_results(&mut rows);
    let done = rows.len();
    let failed = rows.iter().filter(|x| x.get("error").is_some()).count();
    let top = opt_usize(p, "top")?.unwrap_or(50);
    if top > 0 {
        rows.truncate(top);
    }
    Ok(json!({
        "count": n,
        "screened": idx.len(),
        "done": done,
        "failed": failed,
        "cancelled": sum.cancelled,
        "seconds": t0.elapsed().as_secs_f64(),
        "slots": slots,
        "results": rows,
    }))
}
