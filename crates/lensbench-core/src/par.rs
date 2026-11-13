//! 並列の制御: 呼び出しごとのスレッド数。
//!
//! 追跡は光線ごとに独立に計算し、結果は入力の順に並べる。総和・統計は並列にせず決まった順で足すので、
//! スレッド数によらず結果は同じビットになる（tests/determinism.rs）。
//!
//! 外側でプロセス並列にするときは `with_threads(Some(1), ..)` で中を逐次にする（スレッドを作らない）。

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

thread_local! {
    static SEQUENTIAL: Cell<bool> = const { Cell::new(false) };
}

/// スレッドを作れない（スレッドなしの wasm）。rayon に触らず常に逐次にする
const NO_THREADS: bool = cfg!(all(target_arch = "wasm32", not(target_feature = "atomics")));

/// スレッドありの wasm: rayon の大域プールを Web Worker で作り終えたか（[`set_pool_ready`]）。
/// 作る前（画面のスレッド・下書きの Worker など）は逐次にする（std のスレッドは作れない）
#[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
static POOL_READY: AtomicBool = AtomicBool::new(false);

/// スレッドありの wasm: rayon の大域プールを作り終えた（wasm-bindgen-rayon の initThreadPool の後に呼ぶ）
#[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
pub fn set_pool_ready() {
    POOL_READY.store(true, Ordering::Relaxed);
}

fn no_threads() -> bool {
    #[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
    if !POOL_READY.load(Ordering::Relaxed) {
        return true;
    }
    NO_THREADS
}

/// 呼び出し中のスレッドで逐次に計算するか（`with_threads(Some(1), ..)` の中、またはスレッドなしの wasm）
pub fn sequential() -> bool {
    no_threads() || SEQUENTIAL.with(|c| c.get())
}

/// スレッド数ごとのプール（作るのが重いので使い回す。系の状態は持たない）。
/// 一度作ったプールとそのスレッドはプロセスの終わりまで解放しない（使うスレッド数の種類だけ残る）
fn pool(n: usize) -> Result<Arc<rayon::ThreadPool>, rayon::ThreadPoolBuildError> {
    static POOLS: OnceLock<Mutex<HashMap<usize, Arc<rayon::ThreadPool>>>> = OnceLock::new();
    let mut m = POOLS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(p) = m.get(&n) {
        return Ok(p.clone());
    }
    let p = Arc::new(rayon::ThreadPoolBuilder::new().num_threads(n).build()?);
    m.insert(n, p.clone());
    Ok(p)
}

/// f を threads 本のスレッドで実行する。None は既定（外の指定を引き継ぐ: `with_threads(Some(1), ..)` の中なら逐次、
/// 別の with_threads のプールの中ならそのプール。どちらでもなければ環境変数 LENSBENCH_THREADS があればその数、
/// なければ rayon の大域プールで RAYON_NUM_THREADS が効く）、
/// Some(1) は呼び出したスレッドで逐次、Some(0) は外の逐次の指定を外して既定に戻す。
/// Some(n)（n ≥ 2）のプールは n ごとに作って使い回し、解放しない。
pub fn with_threads<R: Send>(threads: Option<usize>, f: impl FnOnce() -> R + Send) -> R {
    match threads {
        // 既定は外の逐次の指定を打ち消さない（以前は with_threads_or_outer と分かれていた）
        None if sequential() => f(),
        // 既定の数は環境変数 LENSBENCH_THREADS（`lensbench run --threads`）。別のプールの中ならそのプール
        None if rayon::current_thread_index().is_none() && env_threads().is_some() => {
            with_threads(env_threads(), f)
        }
        None | Some(0) => with_flag(false, f),
        Some(1) => with_flag(true, f),
        // wasm はスレッドを足せない（使えるのは Web Worker で作った大域プールだけ）
        Some(_) if cfg!(target_arch = "wasm32") => with_flag(false, f),
        Some(n) => match pool(n) {
            Ok(p) => p.install(f),
            // スレッドを作れないときは逐次にする（結果は同じ）
            Err(_) => with_flag(true, f),
        },
    }
}

/// 環境変数 LENSBENCH_THREADS（1 以上の整数）。最初に読んだ値を使い続ける
pub fn env_threads() -> Option<usize> {
    static T: OnceLock<Option<usize>> = OnceLock::new();
    *T.get_or_init(|| {
        std::env::var("LENSBENCH_THREADS")
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|&n| n > 0)
    })
}

/// 呼び出したスレッドの逐次の指定を f の間だけ変える（panic でも戻す）
fn with_flag<R>(seq: bool, f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            SEQUENTIAL.with(|c| c.set(self.0));
        }
    }
    let _g = Restore(SEQUENTIAL.with(|c| c.replace(seq)));
    f()
}

/// いま f が動くプールのスレッド数（逐次の指定の中では 1）
pub fn current_threads() -> usize {
    if sequential() {
        1
    } else {
        rayon::current_num_threads()
    }
}

/// a を n 個ずつに分けた塊のそれぞれに f(番号, 塊) をかける（逐次の指定の中では逐次）。
/// 塊ごとに独立に計算するので、スレッド数によらず結果は同じ
pub fn for_each_chunk_mut<T: Send>(
    a: &mut [T],
    n: usize,
    f: impl Fn(usize, &mut [T]) + Sync + Send,
) {
    use rayon::prelude::*;
    if sequential() {
        a.chunks_mut(n).enumerate().for_each(|(i, c)| f(i, c));
    } else {
        a.par_chunks_mut(n).enumerate().for_each(|(i, c)| f(i, c));
    }
}

/// items のそれぞれに f をかけ、結果を入力の順に返す（逐次の指定の中では逐次）。
/// 要素ごとに独立に計算するので、スレッド数によらず結果は同じ
pub fn map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync + Send) -> Vec<R> {
    use rayon::prelude::*;
    if sequential() || items.len() < 2 {
        items.iter().map(f).collect()
    } else {
        items.par_iter().with_max_len(1).map(f).collect()
    }
}

/// 計算の中断の印（プロセスで 1 つ）。GUI が系を変えるときに立て、次の要求の前に下ろす
static CANCEL: AtomicBool = AtomicBool::new(false);

/// 回っている解析に、区切りのよい所でやめるよう頼む（[`check_cancel`] が `Error::Cancelled` を返す）。
/// 印はプロセスで 1 つなので、同じプロセスで同時に回っている別の解析もやめる
pub fn request_cancel() {
    CANCEL.store(true, Ordering::Relaxed);
}

/// 中断の印を下ろす（次の計算を始める前に）
pub fn clear_cancel() {
    CANCEL.store(false, Ordering::Relaxed);
}

/// 中断を頼まれているか
pub fn cancelled() -> bool {
    CANCEL.load(Ordering::Relaxed)
}

/// 中断を頼まれていれば `Error::Cancelled`。重い解析のループ（MTF の周波数・Huygens の像の点など）で呼ぶ
pub fn check_cancel() -> crate::Result<()> {
    if cancelled() {
        Err(crate::error::Error::Cancelled)
    } else {
        Ok(())
    }
}
