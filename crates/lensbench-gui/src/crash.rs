//! panic の記録（GAR-19）。リリースの exe はコンソールが無いので、panic の文・場所・バックトレース・版を設定の
//! フォルダの `crash.log` に足し、画面のスレッドならメッセージの窓も出す。計算のスレッドの panic（serve が
//! `catch_unwind` で受ける）は、文と場所を [`lensbench_cli::serve::note_panic`] に渡して「内部エラー: …」に載せる。
//! Web 版はブラウザのコンソール（console.error）に出す。

use std::panic::PanicHookInfo;

/// panic の文と場所（`文（file:line）`）
fn text(info: &PanicHookInfo<'_>) -> String {
    let msg = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into());
    match info.location() {
        Some(l) => format!("{msg}（{}:{}）", l.file(), l.line()),
        None => msg,
    }
}

/// 起動時に一度呼ぶ。`dir` は crash.log を書くフォルダ（設定の隣。None なら書かない）
#[cfg(not(target_arch = "wasm32"))]
pub fn install(dir: Option<std::path::PathBuf>) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let t = text(info);
        lensbench_cli::serve::note_panic(&t);
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("?").to_string();
        let bt = std::backtrace::Backtrace::force_capture();
        let when = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = format!(
            "=== lensbench {} panic（スレッド {name}、UNIX 時刻 {when}）\n{t}\n{bt}\n",
            env!("CARGO_PKG_VERSION")
        );
        if let Some(d) = &dir {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(d.join("crash.log"))
            {
                let _ = f.write_all(entry.as_bytes());
            }
        }
        // 画面のスレッドが落ちると窓が消えるだけなので、理由を窓で出す（計算のスレッドは serve がエラーで返す）
        if name == "main" {
            let where_ = dir
                .as_ref()
                .map(|d| d.join("crash.log").display().to_string())
                .unwrap_or_default();
            crate::files::error_box(
                "lensbench",
                &format!("内部エラーで終了します / Internal error:\n{t}\n\n{where_}"),
            );
        }
        prev(info);
    }));
}

/// Web 版: panic をブラウザのコンソールに出す（場所つき）
#[cfg(target_arch = "wasm32")]
pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        let t = text(info);
        lensbench_cli::serve::note_panic(&t);
        web_sys::console::error_1(&format!("lensbench panic: {t}").into());
    }));
}
