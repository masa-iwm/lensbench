//! worker の試験（worker.rs から分けた）
use super::*;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

static STARTED: AtomicBool = AtomicBool::new(false);
static LOADED: AtomicBool = AtomicBool::new(false);

/// 中断を頼まれるまで回る解析（"spin"）と、すぐ終わる要求
fn fake(c: &Call, _t: Option<usize>) -> Result<(Answer, Arrays), CallErr> {
    let m = c.method();
    if m == "spin" {
        STARTED.store(true, Ordering::SeqCst);
        let t0 = Instant::now();
        while !lensbench_core::par::cancelled() {
            if t0.elapsed() > Duration::from_secs(2) {
                return Ok((Value::from("timeout").into(), Vec::new()));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        return Err("cancelled".into());
    }
    // 時間のかかる系の読み込みと、それが終わっていたかを返す要求
    if m == "slowload" {
        std::thread::sleep(Duration::from_millis(100));
        LOADED.store(true, Ordering::SeqCst);
    }
    if m == "loaded" {
        return Ok((
            Value::from(LOADED.load(Ordering::SeqCst)).into(),
            Vec::new(),
        ));
    }
    Ok((Value::from(m).into(), Vec::new()))
}

fn wait_all(w: &mut Worker, n: usize) -> Vec<Done> {
    let t0 = Instant::now();
    let mut out = Vec::new();
    while out.len() < n && t0.elapsed() < Duration::from_secs(10) {
        match w.poll() {
            Some(d) => out.push(d),
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    out
}

fn wait_started() {
    let t0 = Instant::now();
    while !STARTED.load(Ordering::SeqCst) && t0.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(1));
    }
    STARTED.store(false, Ordering::SeqCst);
}

// 順番待ちと世代は Worker で 1 つなので、1 つのテストの中で順に確かめる
#[test]
fn refresh_calls_come_back_to_their_tags() {
    for c in Tag::refresh_calls() {
        assert_eq!(Tag::of_refresh(c.method()).method(), Some(c.method()));
    }
    assert_eq!(Tag::UserCatalog(CatOp::Edit).method(), Some("catalog_edit"));
}

#[test]
fn changing_the_system_cancels_and_drops_analyses_but_not_other_jobs() {
    let mut w = Worker::with_call(Some(1), || {}, true, Kind::Test(fake));
    // 回っている解析は中断され、待っていた解析は回らず、どちらも捨てた扱い。系の変更は回る
    w.send_batch(Tag::Analyze(1), vec![Call::json("spin", Value::Null)]);
    w.send_batch(Tag::Analyze(2), vec![Call::json("info", Value::Null)]);
    wait_started();
    w.send_batch(Tag::Set, vec![Call::json("set", Value::Null)]);
    w.send_batch(Tag::Analyze(3), vec![Call::json("info", Value::Null)]);
    let d = wait_all(&mut w, 4);
    let got: Vec<(Tag, bool)> = d.iter().map(|d| (d.tag.clone(), d.skipped)).collect();
    assert_eq!(
        got,
        vec![
            (Tag::Analyze(1), true),
            (Tag::Analyze(2), true),
            (Tag::Set, false),
            (Tag::Analyze(3), false),
        ]
    );
    assert!(!w.busy());
    // 解析でない長い要求（最適化など）は系の変更で止めない（中の解析も止めない）。
    // 別のスレッドで回るので、その間の系の変更・解析は先に終わる
    w.send_batch(Tag::Optimize, vec![Call::json("spin", Value::Null)]);
    wait_started();
    assert!(w.busy() && !w.busy_short() && !w.busy_core());
    w.send_batch(Tag::Set, vec![Call::json("set", Value::Null)]);
    w.send_batch(Tag::Analyze(4), vec![Call::json("info", Value::Null)]);
    let d = wait_all(&mut w, 3);
    let tags: Vec<Tag> = d.iter().map(|d| d.tag.clone()).collect();
    assert_eq!(tags, vec![Tag::Set, Tag::Analyze(4), Tag::Optimize]);
    assert!(!d[2].skipped);
    assert_eq!(d[2].first().2, Ok(&Value::from("timeout")));
    assert!(!w.busy());
    // 系の読み込みの直後に出した長い要求は、読み込みが終わってから回る（前の系で計算しない）
    w.send_batch(Tag::Load, vec![Call::json("slowload", Value::Null)]);
    w.send_batch(Tag::Zoom, vec![Call::json("loaded", Value::Null)]);
    assert!(w.busy_core() && w.busy());
    let d = wait_all(&mut w, 2);
    let tags: Vec<Tag> = d.iter().map(|d| d.tag.clone()).collect();
    assert_eq!(tags, vec![Tag::Load, Tag::Zoom]);
    assert_eq!(d[1].first().2, Ok(&Value::from(true)));
    assert!(!w.busy());
}
