//! 最適化・公差・ズーム軌跡・差し替え探索（走っている間の進み具合と止め方も）

use super::*;
use lensbench_core::LockExt;
use serde::{Deserialize, Serialize};

// ------------------------------------------------------------------------------------------ 最適化

/// 最適化の apply で当てる変更（base は最適化を始めたときの系）
pub(super) struct PendingApply {
    base: Arc<System>,
    sys: System,
    spec: LoadSpec,
    change: Change,
}

/// 最適化の結果を当てる（プールの外で。回している間に系が変わっていたら当てず、結果の applied・notes を直す）
pub(super) fn apply_pending(sess: &Session, p: PendingApply, a: &mut Answer) {
    let _w = sess.request.write_ok();
    if sess
        .current_system()
        .is_ok_and(|now| Arc::ptr_eq(&now, &p.base))
    {
        store_changed(sess, p.sys, p.spec, p.change);
    } else {
        const NOTE: &str =
            "the system changed during the optimization, so the result was not applied to it";
        match a {
            Answer::Optimize(v) => {
                v.applied = false;
                v.notes.push(NOTE.into());
            }
            Answer::Json(v) => {
                v["applied"] = json!(false);
                if let Some(n) = v["notes"].as_array_mut() {
                    n.push(json!(NOTE));
                }
            }
            _ => {}
        }
    }
}

/// 走っている長い計算（最適化・公差・差し替え探索）の進み具合と中断の印（同じプロセスの GUI・serve の `progress` が読む）。
/// 計算は印 `token` と、ここへ書く受け手を文脈（[`lensbench_core::par::Ctx`]）に置いて回す（[`run_job`]）
#[derive(Default)]
pub(crate) struct Job {
    /// この計算だけを止める印（要求の印の子。要求の `cancel` でも止まる）
    token: lensbench_core::par::CancelToken,
    /// 反復ごとの評価関数の値（最適化。初期値を含む）
    history: Vec<f64>,
    /// 今の段の (段の名前, 終えた数, 全体の数)（公差・差し替え探索）
    count: Option<(&'static str, usize, usize)>,
}

/// 走っている長い計算の置き場（走っていなければ None）
pub(crate) type JobSlot = Arc<Mutex<Option<Job>>>;

/// f を、置き場 slot に job を置いた長い計算として回す: 文脈の中断の印を job の印（今の印の子）に、進み具合の受け手を
/// slot へ書く関数にする。終われば slot を空にする
fn run_job<R>(slot: &JobSlot, job: Job, f: impl FnOnce() -> R) -> R {
    use lensbench_core::par::{self, Event};
    let token = par::current_cancel().map_or_else(Default::default, |t| t.child());
    *slot.lock_ok() = Some(Job {
        token: token.clone(),
        ..job
    });
    let s = slot.clone();
    let p: par::ProgressFn = Arc::new(move |e| {
        if let Some(j) = s.lock_ok().as_mut() {
            match e {
                Event::Iter(i) => j.history.push(i.merit),
                Event::Count(c) => {
                    // 並列の作業は終わる順が前後するので、同じ段なら大きい方を残す
                    let d = match j.count {
                        Some((st, d, _)) if st == c.stage => d.max(c.done),
                        _ => c.done,
                    };
                    j.count = Some((c.stage, d, c.total));
                }
            }
        }
    });
    struct Clear<'a>(&'a JobSlot);
    impl Drop for Clear<'_> {
        fn drop(&mut self) {
            *self.0.lock_ok() = None;
        }
    }
    let _c = Clear(slot);
    par::with_cancel(Some(&token), || par::with_progress(Some(p), f))
}

/// 置き場 slot の計算を止める（走っていなければ何もしない）
fn stop_job(slot: &JobSlot) {
    if let Some(j) = slot.lock_ok().as_ref() {
        j.token.cancel();
    }
}

impl Session {
    /// 走っている最適化の、反復ごとの評価関数の値（走っていなければ None）。同じプロセスから呼ぶ（GUI の進み具合）
    pub fn optimize_progress(&self) -> Option<Vec<f64>> {
        self.opt_run.lock_ok().as_ref().map(|r| r.history.clone())
    }

    /// 走っている最適化を止める（次の反復の終わりで止まり、そこまでの結果を返す）。同じプロセスから呼ぶ
    pub fn optimize_cancel(&self) {
        stop_job(&self.opt_run);
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

/// 構成（.zmx の構成番号か、プロジェクトの状態の名前）の系。読み込んだ系と同じ構成なら読み込んだ系（set・edit の後）。
/// .zmx で多重構成の表を読めていれば読み込んだ系の表から（変更が入ったまま）。ほかは読んだときのファイル src から読み、
/// 読み込んだ後の変更を当てる
pub(super) fn config_system(
    b: &Value,
    cur: &Arc<System>,
    spec: &LoadSpec,
    src: &Source,
    (log, faces): (&[Change], &[Option<FaceId>]),
    notes: &mut Vec<String>,
) -> Result<System, RpcErr> {
    let mut sp = spec.clone();
    if is_project(&spec.file) {
        let name = b
            .as_str()
            .ok_or_else(|| RpcErr::params("configs must be a list of project state names"))?;
        if spec.state.as_deref() == Some(name) {
            return Ok((**cur).clone());
        }
        sp.state = Some(name.to_string());
    } else {
        let c = b
            .as_u64()
            .or_else(|| b.as_str().and_then(|x| x.parse().ok()))
            .ok_or_else(|| {
                RpcErr::params("configs must be a list of .zmx configuration numbers (1-based)")
            })? as usize;
        if c == active_cfg(cur, spec) {
            return Ok((**cur).clone());
        }
        if cur.configs.is_some() {
            return Ok(cur.config(c)?);
        }
        sp.cfg = c;
    }
    let (s, _, to) = crate::load::load_source_faces(src, &sp)?;
    let beam = if is_project(&spec.file) {
        format!("state {b}")
    } else {
        format!("configuration {b}")
    };
    Ok(replay(s, log, (faces, &to), &beam, notes)?)
}

/// 最適化。params は [`lensbench_design::dls_spec::OptimizeParams`]（CLI の optimize・Python の optimize と同じ設定の型）:
/// vars・merit・terms・reference・rings・iters・jac と、configs（構成の番号か状態の名前の並び。省けば読み込んだ系だけ。
/// 項・変数の構成の番号はこの並びの順）・apply（最適化した値を読み込んだ系に set する。読み込んだ系の構成に効く変数だけ）
///
/// 同じプロセスからは [`optimize_progress`] で進み具合を読み、[`optimize_cancel`] で止められる
pub(super) fn optimize(
    sess: &Session,
    mut op: lensbench_design::dls_spec::OptimizeParams,
) -> Result<views::OptimizeView, RpcErr> {
    use lensbench_optim as optim;
    let Current {
        sys: cur,
        spec,
        src,
        faces,
        log,
    } = sess.current_full()?;
    let mut notes = Vec::new();
    let (systems, cur_idx) = if op.configs.is_empty() {
        (vec![(*cur).clone()], Some(0))
    } else {
        let mut v = Vec::new();
        let mut cur_idx = None;
        for b in &op.configs {
            let same = if is_project(&spec.file) {
                spec.state.as_deref() == b.as_str()
            } else {
                b.as_u64()
                    .or_else(|| b.as_str().and_then(|x| x.parse().ok()))
                    == Some(active_cfg(&cur, &spec) as u64)
            };
            if same && cur_idx.is_none() {
                cur_idx = Some(v.len());
            }
            v.push(config_system(
                b,
                &cur,
                &spec,
                &src,
                (&log, &faces),
                &mut notes,
            )?);
        }
        (v, cur_idx)
    };
    for t in op.terms.iter_mut() {
        sess.files.check_term_paths(t)?;
    }
    let (prob, n_default) = op
        .problem(systems)
        .map_err(|e| RpcErr::params(e.to_string()))?;
    prob.check()?;
    let o = op.dls_opts();
    let m0 = prob.merit(&prob.x0()?)?;
    let job = Job {
        history: vec![m0],
        ..Default::default()
    };
    let r = run_job(&sess.opt_run, job, || optim::dls(&prob, &o))?;
    let names: Vec<String> = prob
        .vars
        .iter()
        .map(|v| prob.configs[0].param_name(v.param))
        .collect();
    let (tv0, pc0) = prob.term_values_contrib(&r.x0)?;
    let (tv, pc) = prob.term_values_contrib(&r.x)?;
    let rays = prob.ray_counts(&r.x)?;
    let stop = r.stop.as_str();
    // 読み込んだ系に当てられる変数（読み込んだ系の構成に効くもの）
    let applicable: Vec<bool> = prob
        .vars
        .iter()
        .map(|v| cur_idx.is_some_and(|c| v.configs.as_ref().is_none_or(|cs| cs.contains(&c))))
        .collect();
    let mut applied = false;
    if op.apply {
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
            // 当てるのは計算の後（dispatch がプールの外で場の鍵を取り、系が変わっていれば当てない）
            *sess.pending_apply.lock_ok() = Some(PendingApply {
                base: cur.clone(),
                sys: n,
                spec,
                change: Change::Set(rec),
            });
            applied = true;
        }
    }
    let terms = (0..prob.terms.len())
        .map(|i| views::TermValue {
            default: i < n_default,
            value0: tv0[i].value(),
            value: tv[i].value(),
            value_kind: tv[i].kind_name().to_string(),
            contrib0: pc0[i],
            contrib: pc[i],
            rays: rays[i].map(|c| views::RayCount {
                total: c.total,
                vignetted: c.vignetted,
                stopped: c.stopped,
            }),
        })
        .collect();
    let v = views::OptimizeView {
        names,
        x0: r.x0,
        x: r.x,
        applicable,
        applied,
        merit0: r.merit0,
        merit: r.merit,
        history: r.history,
        iterations: r.iterations,
        evals: r.evals,
        stop: stop.to_string(),
        terms,
        notes,
    };
    Ok(v)
}

// ------------------------------------------------------------------------------------ 公差

// 公差のモンテカルロの進み具合（同じプロセスの GUI が読む、[`tolerance_progress`]）
impl Session {
    /// 走っている公差のモンテカルロの (終えた評価, 評価の数)（感度の 2 × 公差の数 + 試行の数。走っていなければ None）
    pub fn tolerance_progress(&self) -> Option<(usize, usize)> {
        count_of(&self.tol_run)
    }

    /// 走っている公差のモンテカルロを止める（走っている試行は終わらせ、そこまでの結果を返す）。同じプロセスから呼ぶ
    pub fn tolerance_cancel(&self) {
        stop_job(&self.tol_run);
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

/// モンテカルロ公差。params は `lensbench tolerance --spec` の JSON（`lensbench_design::tolerance`）に加えて
/// configs（構成の番号か状態の名前の並び。optimize と同じ。省けば読み込んだ系だけ）。読み込んだ系（set・edit の後）を使う。
/// 同じプロセスからは [`tolerance_progress`] で進み具合を読み、[`tolerance_cancel`] で止められる
/// `tolerance` の params: 構成（状態の名前か構成の番号）の並びと、公差の設定（公差ファイルと同じ形の JSON）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TolReq {
    pub configs: Vec<Value>,
    pub spec: Value,
}

impl TolReq {
    pub(crate) fn parse(p: &Value) -> Result<TolReq, RpcErr> {
        let configs = p
            .get("configs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut spec = p.clone();
        if let Some(o) = spec.as_object_mut() {
            o.remove("configs");
        }
        lensbench_design::tolerance::read_spec(&spec).map_err(RpcErr::params)?;
        Ok(TolReq { configs, spec })
    }
}

pub(super) fn tolerance(
    sess: &Session,
    r: &TolReq,
) -> Result<lensbench_design::tolerance::ToleranceReport, RpcErr> {
    let Current {
        sys: cur,
        spec,
        src,
        faces,
        log,
    } = sess.current_full()?;
    let mut notes = Vec::new();
    let systems = if r.configs.is_empty() {
        vec![(*cur).clone()]
    } else {
        r.configs
            .iter()
            .map(|b| config_system(b, &cur, &spec, &src, (&log, &faces), &mut notes))
            .collect::<Result<Vec<_>, _>>()?
    };
    let (vendors, reversed) = crate::load::tolerance_vendors_in(&src);
    let mut v = r.spec.clone();
    if let Some(o) = v.as_object_mut() {
        if let Some(t) = o.get_mut("terms") {
            sess.files.check_term_paths(t)?;
        }
    }
    let ms = lensbench_design::tolerance::spec_from_json_with(&v, systems, &vendors, &reversed)
        .map_err(RpcErr::params)?;
    let oo = lensbench_design::tolerance::out_opts(&v).map_err(RpcErr::params)?;
    let job = Job {
        count: Some(("", 0, 0)),
        ..Default::default()
    };
    let r = run_job(&sess.tol_run, job, || {
        let run = || lensbench_design::montecarlo::run(&ms);
        match oo.threads {
            Some(t) => lensbench_core::with_threads(Some(t), run),
            None => run(),
        }
    })?;
    let mut out = lensbench_design::tolerance::report(&ms, &r, &oo);
    out.notes = Some(notes);
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
pub(super) fn zoom(
    sess: &Session,
    zp: &lensbench_design::dls_spec::ZoomParams,
) -> Result<views::ZoomView, RpcErr> {
    let Current {
        sys: cur,
        spec,
        src,
        faces,
        log,
    } = sess.current_full()?;
    let mut notes = Vec::new();
    let cfgs: Vec<Value> = if zp.configs.is_empty() {
        let c = configs(sess)?;
        match c["states"].as_array() {
            Some(st) if !st.is_empty() => st.clone(),
            _ => (1..=c["cfgs"].as_u64().unwrap_or(1))
                .map(|k| json!(k))
                .collect(),
        }
    } else {
        zp.configs.clone()
    };
    let systems = cfgs
        .iter()
        .map(|b| config_system(b, &cur, &spec, &src, (&log, &faces), &mut notes))
        .collect::<Result<Vec<_>, _>>()?;
    let names: Vec<String> = cfgs
        .iter()
        .map(|b| b.as_str().map_or_else(|| b.to_string(), String::from))
        .collect();
    let z = crate::zoom::zoom_systems(&systems, &names, zp)
        .map_err(|e| RpcErr::params(e.to_string()))?;
    let mut z = z;
    z.notes = notes;
    Ok(z)
}

// ------------------------------------------------------------------------------------ 差し替え探索

// 差し替え探索の進み具合（同じプロセスの GUI が読む、[`swap_progress`]）
impl Session {
    /// 走っている差し替え探索の (終えた組, 組の数)（走っていなければ None）。同じプロセスから呼ぶ
    pub fn swap_progress(&self) -> Option<(usize, usize)> {
        count_of(&self.swap_run)
    }

    /// 走っている差し替え探索を止める（走っている組は終わらせ、そこまでの結果を返す）。同じプロセスから呼ぶ
    pub fn swap_cancel(&self) {
        stop_job(&self.swap_run);
    }
}

/// 置き場の計算の (終えた数, 全体の数)（走っていなければ None）
fn count_of(slot: &JobSlot) -> Option<(usize, usize)> {
    slot.lock_ok()
        .as_ref()
        .map(|j| j.count.map_or((0, 0), |(_, d, n)| (d, n)))
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
/// `swap` の params: 探索の設定（`--spec` の JSON と同じ形）、別のプロジェクト `project`（省けば読み込んだもの）、
/// 返す組の数 `top`（50、0 は全部）、スレッド数、組の数だけ数える `count_only`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SwapReq {
    pub spec: Value,
    pub project: Option<Value>,
    pub top: Option<usize>,
    pub threads: Option<usize>,
    pub count_only: bool,
}

impl SwapReq {
    pub(crate) fn parse(p: &Value) -> Result<SwapReq, RpcErr> {
        // プロジェクトは処理するときに読む（要求のファイルを制限するときは、読む前に断る）
        let project = p.get("project").filter(|v| !v.is_null()).cloned();
        let mut spec = p.clone();
        if let Some(o) = spec.as_object_mut() {
            for k in ["project", "top", "threads", "count_only"] {
                o.remove(k);
            }
        }
        Ok(SwapReq {
            project,
            top: opt_usize(p, "top")?,
            threads: opt_usize(p, "threads")?,
            count_only: p["count_only"].as_bool() == Some(true),
            spec,
        })
    }
}

pub(super) fn swap(
    sess: &Session,
    r: &SwapReq,
) -> Result<lensbench_design::swap::SwapReply, RpcErr> {
    use lensbench_design::swap::{SwapReply, SwapSearch, SwapSpec};
    let super::Current { spec: ls, src, .. } = sess.current_full()?;
    let proj = match &r.project {
        // 中の zmx・カタログ・膜のパスを check_path で確かめられないので、要求のファイルを制限するときは断る
        Some(_) if !matches!(sess.files, Files::Any) => {
            return Err(RpcErr::app(
                "swap project can't be used with --allow-remote or --root (the paths inside can't be checked; run on the loaded file)",
            ))
        }
        Some(v) => serde_json::from_value::<lensbench_project::Project>(v.clone())
            .map_err(|e| RpcErr::params(format!("project: {e}")))?,
        // 読み込んだときのプロジェクト（読み直さない）
        None => src
            .project()
            .cloned()
            .ok_or_else(|| RpcErr::app("the loaded system is not a project"))?,
    };
    let mut sp = r.spec.clone();
    if let Some(o) = sp.as_object_mut() {
        let multi = o
            .get("states")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
        if !multi && !o.contains_key("state") && r.project.is_none() {
            if let Some(s) = &ls.state {
                o.insert("state".into(), json!(s));
            }
        }
    }
    let spec: SwapSpec = serde_json::from_value(sp)
        .map_err(|e| RpcErr::params(format!("swap search settings: {e}")))?;
    let cat = lensbench_core::catalog::LensCatalog::load_registered(&ls.lens_catalog)?;
    let env = lensbench_project::Env {
        step_files: None,
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
    let slots = s.slots();
    if r.count_only {
        let r = SwapReply {
            count: n,
            slots,
            ..Default::default()
        };
        return Ok(r);
    }
    let rows: Mutex<Vec<lensbench_design::swap::SwapResult>> = Mutex::new(Vec::new());
    let o = lensbench_runs::RunOpts {
        threads: r.threads,
        header: "{}".into(),
        stage: lensbench_design::swap::SwapStage::Main.name(),
        ..Default::default()
    };
    let t0 = web_time::Instant::now();
    let job = Job {
        count: Some((lensbench_design::swap::SwapStage::Main.name(), 0, n)),
        ..Default::default()
    };
    let (mut rows, screened, cancelled) = run_job(&sess.swap_run, job, || {
        if s.uses_beam() {
            // 所ごとのビーム探索: 評価したすべての所を選んだ組（screened はその数）
            let (v, _, stopped, why) = s.beam_run(o.threads);
            if let Some(e) = why {
                return Err(RpcErr::from(e));
            }
            let k = v.len();
            Ok((v, k, stopped))
        } else {
            // 粗いふるい（prescreen）があれば残った組だけ（ふるいの間も進み具合を出し、止められる）
            let (idx, stopped) = s.screen(&o)?;
            if let Some(j) = sess.swap_run.lock_ok().as_mut() {
                j.count = Some((o.stage, 0, idx.len()));
            }
            let sum = lensbench_runs::run(idx.len(), 0, &o, |j, _| {
                let v = s.eval(idx[j]);
                rows.lock_ok().push(v);
                Ok("{}".into())
            })?;
            let mut rows = std::mem::take(&mut *rows.lock_ok());
            lensbench_design::swap::sort_results(&mut rows);
            Ok((rows, idx.len(), sum.cancelled || stopped))
        }
    })?;
    let done = rows.len();
    let failed = rows.iter().filter(|x| x.error.is_some()).count();
    let top = r.top.unwrap_or(50);
    if top > 0 {
        rows.truncate(top);
    }
    Ok(SwapReply {
        count: n,
        screened: Some(screened),
        done: Some(done),
        failed: Some(failed),
        cancelled: Some(cancelled),
        seconds: Some(t0.elapsed().as_secs_f64()),
        slots,
        results: Some(rows),
    })
}
