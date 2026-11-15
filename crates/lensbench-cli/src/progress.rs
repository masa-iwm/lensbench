//! 長い計算（最適化・公差・差し替え探索・runs）の進み具合を標準エラーに出す受け手（IOO-26・CLI-10）。
//! 計算は進み具合を文脈（[`lensbench_core::par::with_progress`]）へ知らせるだけで、出し方は呼んだ側が決める

use lensbench_core::par::{self, Event};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// 標準エラーへ出す受け手（json なら何も出さないので None）。
/// 反復は 1 回ごとに 1 行、段の無い数の作業は全体の 5% ごとに「k / n」の行、段のある作業は同じ行を書き換える
pub fn stderr(json: bool) -> Option<par::ProgressFn> {
    if json {
        return None;
    }
    let last = Arc::new(AtomicUsize::new(0));
    Some(Arc::new(move |e: &Event| match e {
        Event::Iter(i) => eprintln!(
            "{}",
            lensbench_core::i18n::tr(&format!("iteration {:>3}: merit {:.9e}", i.iter, i.merit))
        ),
        Event::Count(c) if c.stage.is_empty() => {
            let step = c.total.div_ceil(20).max(1);
            // 並列の作業は終わる順が前後するので、出した数より大きいときだけ
            if (c.done % step == 0 || c.done == c.total)
                && last.fetch_max(c.done, Ordering::Relaxed) < c.done
            {
                eprintln!("{} / {}", c.done, c.total);
            }
        }
        Event::Count(c) => {
            let what = match c.stage {
                "screen" => "screening",
                "main" => "combinations",
                s => s,
            };
            let eta = c.eta.map_or("?".into(), |e| format!("{e:.0}"));
            let m = format!("{what} {}/{} ({} s left)", c.done, c.total, eta);
            eprint!("\r{}  ", lensbench_core::i18n::tr(&m));
            let _ = std::io::stderr().flush();
        }
    }))
}

/// f を、標準エラーへ出す受け手（[`stderr`]）の下で回す
pub fn with<R>(json: bool, f: impl FnOnce() -> R) -> R {
    par::with_progress(stderr(json), f)
}
