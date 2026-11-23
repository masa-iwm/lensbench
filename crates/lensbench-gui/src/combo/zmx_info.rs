//! 要素の .zmx から読む情報（図を描くたびに呼ぶので覚えておく）

/// .zmx の絞りの面と像面の番号（読めなければ None）。ファイルの更新時刻ごとに覚えておく（図を描くたびに呼ぶので）。
/// 更新時刻の読み出し（stat）も 1 秒に 1 回まで（1 フレームに何十回も呼ばれ、ネットワークの上では遅い）
pub(crate) fn zmx_stop_info(path: &str) -> Option<(usize, usize)> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    /// 更新時刻、確かめた時刻、値
    type Memo = HashMap<
        String,
        (
            Option<std::time::SystemTime>,
            web_time::Instant,
            Option<(usize, usize)>,
        ),
    >;
    static MEMO: OnceLock<Mutex<Memo>> = OnceLock::new();
    if path.is_empty() {
        return None;
    }
    let memo = MEMO.get_or_init(Default::default);
    if let Some((_, at, v)) = memo.lock().ok()?.get(path) {
        if at.elapsed() < std::time::Duration::from_secs(1) {
            return *v;
        }
    }
    let mtime = lensbench_core::vfs::stamp(path).ok().and_then(|s| s.0);
    let now = web_time::Instant::now();
    if let Some((t, at, v)) = memo.lock().ok()?.get_mut(path) {
        if *t == mtime {
            *at = now;
            return *v;
        }
    }
    let v = lensbench_core::glass::read_text(std::path::Path::new(path))
        .ok()
        .and_then(|t| lensbench_formats::zmx::parse_text(&t).ok())
        .and_then(|raw| {
            let stop = raw.surfs.iter().position(|x| x.stop)?;
            Some((stop, raw.surfs.len().checked_sub(1)?))
        });
    memo.lock().ok()?.insert(path.to_string(), (mtime, now, v));
    v
}
