//! 並列の制御: 呼び出しごとのスレッド数と、計算の文脈（[`Ctx`]: 中断の印 [`CancelToken`]・まとめ追跡の切り替え）。
//!
//! 文脈は呼び出したスレッドに置き（[`with_cancel`]・[`with_batch`]）、このモジュールの並列は作業のスレッドへ引き継ぐ。
//! プロセスの大域には置かないので、同じプロセスで同時に回る別の計算（GUI の下書き・serve の別の接続）に響かない。
//!
//! 追跡は光線ごとに独立に計算し、結果は入力の順に並べる。総和・統計は並列にせず決まった順で足すので、
//! スレッド数によらず結果は同じビットになる（tests/determinism.rs）。
//!
//! 外側でプロセス並列にするときは `with_threads(Some(1), ..)` で中を逐次にする（スレッドを作らない）。

use crate::LockExt;
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
/// apart は [`with_threads_apart`] の別のプール
fn pool(n: usize, apart: bool) -> Result<Arc<rayon::ThreadPool>, rayon::ThreadPoolBuildError> {
    type Pools = HashMap<(usize, bool), Arc<rayon::ThreadPool>>;
    static POOLS: OnceLock<Mutex<Pools>> = OnceLock::new();
    let mut m = POOLS.get_or_init(Default::default).lock_ok();
    if let Some(p) = m.get(&(n, apart)) {
        return Ok(p.clone());
    }
    let p = Arc::new(rayon::ThreadPoolBuilder::new().num_threads(n).build()?);
    m.insert((n, apart), p.clone());
    Ok(p)
}

/// f を threads 本のスレッドで実行する。None は既定（外の指定を引き継ぐ: `with_threads(Some(1), ..)` の中なら逐次、
/// 別の with_threads のプールの中ならそのプール。どちらでもなければ [`set_default_threads`] の数があればその数、
/// なければ rayon の大域プールで RAYON_NUM_THREADS が効く）、
/// Some(1) は呼び出したスレッドで逐次、Some(0) は外の逐次の指定を外して既定に戻す。
/// Some(n)（n ≥ 2）のプールは n ごとに作って使い回し、解放しない。
pub fn with_threads<R: Send>(threads: Option<usize>, f: impl FnOnce() -> R + Send) -> R {
    threads_in(threads, false, f)
}

/// [`with_threads`] で、n ≥ 2 のときは同じ数でも別のプールで回す（長い計算を、同時に回る短い計算のプールの
/// 盗みに載せない。載ると短い計算が長い計算の終わりまで返らない）
pub fn with_threads_apart<R: Send>(threads: Option<usize>, f: impl FnOnce() -> R + Send) -> R {
    threads_in(threads, true, f)
}

fn threads_in<R: Send>(threads: Option<usize>, apart: bool, f: impl FnOnce() -> R + Send) -> R {
    match threads {
        // 既定は外の逐次の指定を打ち消さない（以前は with_threads_or_outer と分かれていた）
        None if sequential() => f(),
        // 既定の数（CLI・Python が起動時に決める）。別のプールの中ならそのプール
        None if rayon::current_thread_index().is_none() && default_threads().is_some() => {
            threads_in(default_threads(), apart, f)
        }
        None | Some(0) => with_flag(false, f),
        Some(1) => with_flag(true, f),
        // wasm はスレッドを足せない（使えるのは Web Worker で作った大域プールだけ）
        Some(_) if cfg!(target_arch = "wasm32") => with_flag(false, f),
        Some(n) => match pool(n, apart) {
            Ok(p) => {
                // プールのスレッドで回すので、中断の印を引き継ぐ
                let c = current();
                p.install(move || with_ctx(&c, f))
            }
            // スレッドを作れないときは逐次にする（結果は同じ）
            Err(_) => with_flag(true, f),
        },
    }
}

/// 既定のスレッド数（0 は無し: rayon の大域プール）
static DEFAULT_THREADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// `with_threads(None, ..)` の既定のスレッド数を決める（None・0 で rayon の大域プール）。フロントエンド
/// （CLI の `--threads`・Python の読込）が起動時に呼ぶ。core は環境変数を読まない
pub fn set_default_threads(n: Option<usize>) {
    DEFAULT_THREADS.store(n.unwrap_or(0), Ordering::Relaxed);
}

/// 既定のスレッド数（[`set_default_threads`]）
pub fn default_threads() -> Option<usize> {
    Some(DEFAULT_THREADS.load(Ordering::Relaxed)).filter(|&n| n > 0)
}

/// 既定のスレッド数を環境変数 LENSBENCH_THREADS（1 以上の整数。`lensbench run --threads` がスクリプトに渡す）
/// から決める。CLI の起動・Python の import で呼ぶ（`--threads`・`threads=` を省いたときの数）
pub fn default_threads_from_env() {
    let n = std::env::var("LENSBENCH_THREADS")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok());
    set_default_threads(n);
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
        let cx = current();
        a.par_chunks_mut(n)
            .enumerate()
            .for_each(|(i, c)| with_ctx(&cx, || f(i, c)));
    }
}

/// items のそれぞれに f をかけ、結果を入力の順に返す（逐次の指定の中では逐次）。
/// 要素ごとに独立に計算するので、スレッド数によらず結果は同じ
pub fn map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync + Send) -> Vec<R> {
    use rayon::prelude::*;
    if sequential() || items.len() < 2 {
        items.iter().map(f).collect()
    } else {
        items.par_iter().with_max_len(1).map(scoped(f)).collect()
    }
}

/// 計算の中断の印。計算（要求）ごとに作って [`with_cancel`] で渡し、別のスレッドから [`CancelToken::cancel`] で倒す。
/// 印は計算ごとなので、同じプロセスで同時に回っている別の計算は止めない
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// この印で回っている計算に、区切りのよい所でやめるよう頼む（[`check_cancel`] が `Error::Cancelled` を返す）
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// 中断を頼まれたか
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// 同じ印か（写しも同じ印）
    pub fn same(&self, o: &CancelToken) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

/// 計算の文脈（呼び出したスレッドに置き、このモジュールの並列が作業のスレッドへ引き継ぐ）
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    /// 中断の印（[`with_cancel`]）。None は中断できない
    pub cancel: Option<CancelToken>,
    /// まとめ追跡（`batch`）を使わない（[`with_batch`]。結果は同じビットなので切り分け・比較のためだけ）
    pub no_batch: bool,
}

thread_local! {
    /// 呼び出し中のスレッドの計算の文脈
    static CTX: std::cell::RefCell<Ctx> = std::cell::RefCell::new(Ctx::default());
}

/// f を文脈 c の下で回す（panic でも戻す）
pub fn with_ctx<R>(c: &Ctx, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Ctx>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(c) = self.0.take() {
                CTX.with(|x| *x.borrow_mut() = c);
            }
        }
    }
    let _g = Restore(Some(CTX.with(|x| x.replace(c.clone()))));
    f()
}

/// 呼び出し中のスレッドの文脈（rayon を直に使う所で、作業のスレッドへ [`with_ctx`] で引き継ぐ）
pub fn current() -> Ctx {
    CTX.with(|x| x.borrow().clone())
}

/// f を中断の印 t の下で回す（文脈のほかの項目は引き継ぐ）。この中の [`check_cancel`]・[`cancelled`] は t を見る
pub fn with_cancel<R>(t: Option<&CancelToken>, f: impl FnOnce() -> R) -> R {
    let c = Ctx {
        cancel: t.cloned(),
        ..current()
    };
    with_ctx(&c, f)
}

/// f をまとめ追跡あり（on）・なしで回す（文脈のほかの項目は引き継ぐ）
pub fn with_batch<R>(on: bool, f: impl FnOnce() -> R) -> R {
    let c = Ctx {
        no_batch: !on,
        ..current()
    };
    with_ctx(&c, f)
}

/// まとめ追跡を使ってよいか（[`with_batch`]）
pub fn batch_enabled() -> bool {
    CTX.with(|x| !x.borrow().no_batch)
}

/// f を、今の文脈を引き継ぐ関数にする（`par_iter().map(scoped(f))` のように rayon に渡す）
pub fn scoped<A, R>(f: impl Fn(A) -> R + Sync + Send) -> impl Fn(A) -> R + Sync + Send {
    let c = current();
    move |a| with_ctx(&c, || f(a))
}

/// 中断を頼まれているか（印の無い計算は頼まれない）
pub fn cancelled() -> bool {
    CTX.with(|x| {
        x.borrow()
            .cancel
            .as_ref()
            .is_some_and(CancelToken::is_cancelled)
    })
}

/// 中断を頼まれていれば `Error::Cancelled`。重い解析のループ（MTF の周波数・Huygens の像の点など）で呼ぶ
pub fn check_cancel() -> crate::Result<()> {
    if cancelled() {
        Err(crate::error::Error::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod cancel_tests {
    use super::*;

    /// 印は計算ごと: 倒した印の計算だけが止まり、並列の作業のスレッドにも引き継がれる
    #[test]
    fn token_is_per_computation_and_reaches_workers() {
        let a = CancelToken::new();
        let b = CancelToken::new();
        a.cancel();
        with_cancel(Some(&a), || {
            assert!(check_cancel().is_err());
            let v = with_threads(Some(3), || map(&[1, 2, 3, 4], |_| cancelled()));
            assert!(v.iter().all(|&x| x));
        });
        with_cancel(Some(&b), || {
            assert!(check_cancel().is_ok());
            let v = with_threads(Some(3), || map(&[1, 2, 3, 4], |_| cancelled()));
            assert!(v.iter().all(|&x| !x));
        });
        // 印の外は頼まれない
        assert!(!cancelled());
    }

    /// まとめ追跡の切り替えも文脈で、作業のスレッドに引き継ぎ、外では既定（使う）に戻る
    #[test]
    fn batch_switch_is_scoped() {
        assert!(batch_enabled());
        let v = with_batch(false, || {
            with_threads(Some(3), || map(&[1, 2, 3, 4], |_| batch_enabled()))
        });
        assert!(v.iter().all(|&x| !x));
        assert!(batch_enabled());
    }
}
