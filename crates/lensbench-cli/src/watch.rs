//! `lensbench watch`: 系のファイルの更新を見張り、変わるたびに解析を実行して結果を書き出す。
//!
//! ファイルを介した簡易のループ（別アプリが系のファイルを書く → lensbench が再解析して結果を書く → 別アプリが読む）。
//! 見張りは一定の間隔で更新時刻と大きさを見る（ファイルシステムの通知は使わない）。書きかけを読まないよう、
//! 変わった後に 1 間隔待って変化が止まってから解析する。
//!
//! - 結果（--save）は一時ファイルに書いてから名前を変えるので、読む側が書きかけを見ることはない
//! - 見張るのは、系のファイル・--also のほか、解析の中で読んだファイル（プロジェクトが参照する .zmx・カタログ・ガラス・
//!   マクロなど。回すたびに足す。フォルダは見ない）。CLI-30
//! - 1 回ごとに 1 行の JSON を標準出力に出す（--status があれば同じ内容をそのファイルにも置き換えで書く）。
//!   `run` は 1 からの通し番号で、読む側は番号が進んだら結果を読めばよい

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use clap::Args;
use serde_json::{json, Value};

use super::{execute, print_out, Cli, Cmd};

#[derive(Args)]
pub struct WatchArgs {
    /// 見張る間隔 [s]
    #[arg(long, default_value_t = 0.5)]
    interval: f64,
    /// この回数だけ解析したら終える（省くと止めるまで続ける）
    #[arg(long = "max-runs")]
    max_runs: Option<usize>,
    /// 系のファイルと解析の中で読んだファイルのほかに見張るファイル（複数可）
    #[arg(long)]
    also: Vec<PathBuf>,
    /// 1 回ごとの状態（run・ok・error・elapsed_ms）を書くファイル（JSON）
    #[arg(long)]
    status: Option<PathBuf>,
    /// 結果の全体を標準出力に出す（既定は 1 行の状態だけ。--save がなければ全体を出す）
    #[arg(long)]
    print: bool,
    /// 実行する解析（-- の後に CLI と同じ書き方。例: -- spot sys.zmx --save out.json）
    #[arg(last = true, required = true, num_args = 1..)]
    cmd: Vec<String>,
}

type Stamp = Vec<Option<(SystemTime, u64)>>;

fn stamp(files: &[PathBuf]) -> Stamp {
    files
        .iter()
        .map(|f| {
            std::fs::metadata(f)
                .ok()
                .map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()))
        })
        .collect()
}

fn parse(cmd: &[String]) -> Result<Cli, String> {
    let argv = std::iter::once("lensbench".to_string()).chain(cmd.iter().cloned());
    crate::parse_cli(argv).map_err(|e| e.render().to_string())
}

/// 書きかけを見せないよう、一時ファイル（同じフォルダ・同じ拡張子）に書いてから置き換える
fn tmp_path(p: &Path) -> PathBuf {
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    p.with_file_name(format!(".~{}-{name}", std::process::id()))
}

fn write_atomic(p: &Path, s: &str) -> Result<(), String> {
    let t = tmp_path(p);
    lensbench_core::vfs::write(&t, s).map_err(|e| format!("{}: {e}", t.display()))?;
    std::fs::rename(&t, p).map_err(|e| format!("{}: {e}", p.display()))
}

/// 系・プロジェクトのファイルと --save の欄（見張る・一時ファイルに書かせる）
fn file_and_save(c: &mut Cmd) -> Option<(&Path, &mut Option<PathBuf>)> {
    match c {
        Cmd::Zoom(a) => Some((&a.file, &mut a.save)),
        Cmd::Swap(a) => Some((&a.file, &mut a.save)),
        c => c.common_mut().map(|c| (c.file.as_path(), &mut c.save)),
    }
}

/// 1 回解析する。返すのは (結果, 書き出したファイル) と、解析の中で読んだファイル
#[allow(clippy::type_complexity)]
fn run_once(cmd: &[String]) -> (Result<(bool, Value, Option<PathBuf>), String>, Vec<PathBuf>) {
    let (r, read) = lensbench_core::vfs::recording(|| {
        let mut cli = parse(cmd)?;
        let save = match file_and_save(&mut cli.cmd) {
            Some((_, s)) => {
                let save = s.clone();
                if let Some(p) = &save {
                    *s = Some(tmp_path(p));
                }
                save
            }
            None => None,
        };
        let (json_out, out) = execute(cli.cmd)?;
        if let Some(p) = &save {
            std::fs::rename(tmp_path(p), p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        Ok((json_out, out, save))
    });
    // 書き出しと同じフォルダを見張ると書くたびに回り直すので、フォルダは除く
    let files = read.into_iter().filter(|p| p.is_file()).collect();
    (r, files)
}

pub fn watch(a: WatchArgs) -> Result<(), String> {
    if a.interval.is_nan() || a.interval <= 0.0 {
        return Err("--interval は正".into());
    }
    // 引数の誤りは最初に知らせる
    let mut cli = parse(&a.cmd)?;
    let mut files: Vec<PathBuf> = file_and_save(&mut cli.cmd)
        .map(|(f, _)| f.to_path_buf())
        .into_iter()
        .collect();
    files.extend(a.also.iter().cloned());
    let dt = Duration::from_secs_f64(a.interval);
    let mut last = stamp(&files);
    let mut n = 0usize;
    loop {
        n += 1;
        let t0 = Instant::now();
        let (r, read) = run_once(&a.cmd);
        let before = files.len();
        for f in read {
            if !files.contains(&f) {
                files.push(f);
            }
        }
        // 新しく見張るファイルは今の状態から
        last.extend(stamp(&files[before..]));
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        let st = match &r {
            Ok((_, _, w)) => json!({
                "run": n, "ok": true, "elapsed_ms": ms,
                "written": w.as_ref().map(|p| p.display().to_string()),
            }),
            Err(e) => json!({ "run": n, "ok": false, "error": e, "elapsed_ms": ms }),
        };
        if let Some(p) = &a.status {
            // 読む側が開いていて置き換えられないこともある（Windows）。見張りは続ける
            if let Err(e) = write_atomic(p, &(serde_json::to_string_pretty(&st).unwrap() + "\n")) {
                eprintln!("lensbench watch: {e}");
            }
        }
        match r {
            Ok((json_out, out, w)) if a.print || w.is_none() => {
                println!("{st}");
                print_out(json_out, &out);
            }
            _ => println!("{st}"),
        }
        use std::io::Write;
        std::io::stdout().flush().ok();
        if a.max_runs.is_some_and(|m| n >= m) {
            return Ok(());
        }
        // 変わるまで待ち、変化が止まるまでもう少し待つ
        loop {
            std::thread::sleep(dt);
            let now = stamp(&files);
            if now != last {
                let mut cur = now;
                loop {
                    std::thread::sleep(dt);
                    let again = stamp(&files);
                    if again == cur {
                        break;
                    }
                    cur = again;
                }
                last = cur;
                break;
            }
        }
    }
}
