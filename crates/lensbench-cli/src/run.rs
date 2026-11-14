//! `lensbench run script.py [引数]`: Python のスクリプトを画面なしで実行する。
//!
//! exe に Python は埋め込まない。外の Python を見つけ、ガラス・膜の場所・スレッド数・出力先を環境変数で渡して
//! 起動するだけの薄い入口（スクリプトは `import lensbench` で lensbench-py を使う）。Python がなくても
//! lensbench の他のコマンドは動く。
//!
//! - 渡す環境変数: LENSBENCH_GLASSCAT・LENSBENCH_COATINGS（OS のパス区切り）、LENSBENCH_THREADS、
//!   LENSBENCH_OUT、LENSBENCH_LOG、LENSBENCH_RUN=1。Python 側は `lensbench.run_settings()` で読める
//! - スクリプトの標準出力・標準エラーはそのまま画面に出し、--log があれば経過時間つきでファイルにも書く
//! - 終了コードはスクリプトのもの。Python を起動できなければ 127
//!
//! GUI からも同じスクリプトを呼べるよう、本体は [`run_script`]（行ごとの受け取り口つき）にしてある。

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Instant;

use clap::Args;
use serde_json::Value;

#[derive(Args, Clone, Debug, Default)]
pub struct RunArgs {
    /// 設定ファイル（JSON: python・glass_dirs・coatings・threads・out・log・env）。相対パスはこのファイルから。引数で与えたものが優先する
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// 使う Python（省くと LENSBENCH_PYTHON、仮想環境（VIRTUAL_ENV）、PATH の python3・python・py の順）
    #[arg(long)]
    pub python: Option<String>,
    /// ガラスカタログを探すフォルダ（複数可。今の LENSBENCH_GLASSCAT の前に足す）
    #[arg(long = "glass-dir")]
    pub glass_dir: Vec<PathBuf>,
    /// 膜の実測表・モデルのファイル（複数可。System.load の coatings= を省いたときに使う）
    #[arg(long)]
    pub coatings: Vec<PathBuf>,
    /// 出力先のフォルダ（無ければ作る。スクリプトは run_settings()["out"] で受け取る）
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// ログのファイル（省くと --out があれば <out>/run.log、なければ書かない）
    #[arg(long)]
    pub log: Option<PathBuf>,
    /// ログのファイルを書かない
    #[arg(long = "no-log")]
    pub no_log: bool,
    /// 実行する Python スクリプト
    pub script: PathBuf,
    /// スクリプトに渡す引数（sys.argv[1:]）
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

/// 1 行の出どころ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    /// スクリプトの標準出力
    Stdout,
    /// スクリプトの標準エラー
    Stderr,
    /// lensbench run 自身の知らせ（開始・終了）
    Runner,
}

/// 解決した設定
#[derive(Clone, Debug, Default)]
pub struct Resolved {
    pub python: Vec<String>,
    pub glass_dirs: Vec<PathBuf>,
    pub coatings: Vec<PathBuf>,
    pub threads: Option<usize>,
    pub out: Option<PathBuf>,
    pub log: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

/// 設定ファイル（JSON）を読む
fn read_config(path: &Path) -> Result<Resolved, String> {
    let txt = lensbench_core::vfs::read_to_string(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let v: Value = serde_json::from_str(&txt).map_err(|e| format!("{}: {e}", path.display()))?;
    let obj = v
        .as_object()
        .ok_or_else(|| format!("{}: JSON のオブジェクトでない", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    let rel = |s: &str| -> PathBuf {
        let p = PathBuf::from(s);
        if p.is_absolute() {
            p
        } else {
            base.join(p)
        }
    };
    let strs = |k: &str| -> Result<Vec<String>, String> {
        match obj.get(k) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::String(s)) => Ok(vec![s.clone()]),
            Some(Value::Array(a)) => a
                .iter()
                .map(|x| {
                    x.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| format!("設定 {k} は文字列の並び"))
                })
                .collect(),
            Some(_) => Err(format!("設定 {k} は文字列か文字列の並び")),
        }
    };
    let mut r = Resolved::default();
    for k in obj.keys() {
        if ![
            "python",
            "glass_dirs",
            "coatings",
            "threads",
            "out",
            "log",
            "env",
        ]
        .contains(&k.as_str())
        {
            return Err(format!("{}: 知らない設定 {k}", path.display()));
        }
    }
    // python は文字列（"py -3" のように引数つきも可）か並び。パスに見えるものだけ設定ファイルから解く
    r.python = match obj.get("python") {
        Some(Value::String(s)) => split_command(s),
        _ => strs("python")?,
    };
    if let Some(first) = r.python.first_mut() {
        if first.contains('/') || first.contains('\\') {
            *first = rel(first).display().to_string();
        }
    }
    r.glass_dirs = strs("glass_dirs")?.iter().map(|s| rel(s)).collect();
    r.coatings = strs("coatings")?.iter().map(|s| rel(s)).collect();
    r.threads = match obj.get("threads") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .filter(|&n| n > 0)
                .ok_or("設定 threads は 1 以上の整数")? as usize,
        ),
    };
    r.out = strs("out")?.first().map(|s| rel(s));
    r.log = strs("log")?.first().map(|s| rel(s));
    if let Some(e) = obj.get("env") {
        let e = e.as_object().ok_or("設定 env は {名前: 値}")?;
        for (k, v) in e {
            let v = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => return Err(format!("設定 env.{k} は文字列か数")),
            };
            r.env.push((k.clone(), v));
        }
    }
    Ok(r)
}

/// 引数と設定ファイルを合わせる（引数が優先、並びは引数 → 設定ファイルの順に足す）
pub fn resolve(a: &RunArgs, threads: Option<usize>) -> Result<Resolved, String> {
    let mut r = match &a.config {
        Some(p) => read_config(p)?,
        None => Resolved::default(),
    };
    if let Some(p) = &a.python {
        r.python = split_command(p);
    }
    r.glass_dirs = a.glass_dir.iter().cloned().chain(r.glass_dirs).collect();
    r.coatings = a.coatings.iter().cloned().chain(r.coatings).collect();
    if threads.is_some() {
        r.threads = threads;
    }
    if a.out.is_some() {
        r.out = a.out.clone();
    }
    if a.log.is_some() {
        r.log = a.log.clone();
    }
    if r.log.is_none() {
        r.log = r.out.as_ref().map(|o| o.join("run.log"));
    }
    if a.no_log {
        r.log = None;
    }
    // スクリプトが作業フォルダを変えても同じ場所を指すよう、絶対パスにして渡す
    let abs = |p: PathBuf| std::path::absolute(&p).unwrap_or(p);
    r.out = r.out.map(abs);
    r.log = r.log.map(abs);
    r.glass_dirs = r.glass_dirs.into_iter().map(abs).collect();
    r.coatings = r.coatings.into_iter().map(abs).collect();
    Ok(r)
}

/// Python の指定（`--python`・`LENSBENCH_PYTHON`・設定の文字列）をコマンドと引数に分ける。
/// 全体が在るファイルならそのまま 1 つのパス（`C:\Program Files\Python312\python.exe`）。
/// ほかは空白で分け、"…"・'…' の中の空白は分けない（`"C:\Program Files\py.exe" -3`、`py -3`）
pub fn split_command(s: &str) -> Vec<String> {
    let t = s.trim();
    if t.is_empty() {
        return Vec::new();
    }
    if std::path::Path::new(t).is_file() {
        return vec![t.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for ch in t.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                any = true;
            }
            None if ch.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            None => cur.push(ch),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// 試す Python の候補（先頭から順に起動を試す）
fn python_candidates(r: &Resolved) -> Vec<Vec<String>> {
    if !r.python.is_empty() {
        return vec![r.python.clone()];
    }
    let mut v = Vec::new();
    if let Ok(p) = std::env::var("LENSBENCH_PYTHON") {
        if !p.trim().is_empty() {
            return vec![split_command(&p)];
        }
    }
    if let Some(venv) = std::env::var_os("VIRTUAL_ENV") {
        let venv = PathBuf::from(venv);
        let exe = if cfg!(windows) {
            venv.join("Scripts").join("python.exe")
        } else {
            venv.join("bin").join("python")
        };
        if exe.exists() {
            v.push(vec![exe.display().to_string()]);
        }
    }
    if cfg!(windows) {
        v.push(vec!["python".into()]);
        v.push(vec!["py".into(), "-3".into()]);
    } else {
        v.push(vec!["python3".into()]);
        v.push(vec!["python".into()]);
    }
    v
}

fn join_paths(
    new: &[PathBuf],
    old: Option<std::ffi::OsString>,
) -> Result<std::ffi::OsString, String> {
    let mut all: Vec<PathBuf> = new.to_vec();
    if let Some(o) = old {
        all.extend(std::env::split_paths(&o));
    }
    std::env::join_paths(all).map_err(|e| format!("パスを並べられない: {e}"))
}

/// Python を起動できないときの終了コード（シェルの「コマンドがない」と同じ）
pub const NO_PYTHON: i32 = 127;

/// スクリプトを実行し、終了コードを返す（Python を起動できなければ [`NO_PYTHON`]）。
/// on_line は行ごとに（出どころ・経過秒・行）で呼ばれる。設定・スクリプトの誤りは Err
pub fn run_script(
    a: &RunArgs,
    r: &Resolved,
    mut on_line: impl FnMut(Stream, f64, &str),
) -> Result<i32, String> {
    if !a.script.is_file() {
        return Err(format!("スクリプトがない: {}", a.script.display()));
    }
    if let Some(o) = &r.out {
        lensbench_core::vfs::create_dir_all(o).map_err(|e| format!("{}: {e}", o.display()))?;
    }
    let t0 = Instant::now();
    let mut child = None;
    let mut tried = Vec::new();
    for cand in python_candidates(r) {
        let mut c = Command::new(&cand[0]);
        c.args(&cand[1..])
            .arg(&a.script)
            .args(&a.args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PYTHONUNBUFFERED", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .env("LENSBENCH_RUN", "1");
        if !r.glass_dirs.is_empty() {
            c.env(
                "LENSBENCH_GLASSCAT",
                join_paths(&r.glass_dirs, std::env::var_os("LENSBENCH_GLASSCAT"))?,
            );
        }
        if !r.coatings.is_empty() {
            c.env(
                "LENSBENCH_COATINGS",
                join_paths(&r.coatings, std::env::var_os("LENSBENCH_COATINGS"))?,
            );
        }
        if let Some(n) = r.threads {
            c.env("LENSBENCH_THREADS", n.to_string());
            c.env("RAYON_NUM_THREADS", n.to_string());
        }
        if let Some(o) = &r.out {
            c.env("LENSBENCH_OUT", o);
        }
        if let Some(l) = &r.log {
            c.env("LENSBENCH_LOG", l);
        }
        for (k, v) in &r.env {
            c.env(k, v);
        }
        match c.spawn() {
            Ok(ch) => {
                child = Some((ch, cand.join(" ")));
                break;
            }
            Err(e) => tried.push(format!("{}（{e}）", cand.join(" "))),
        }
    }
    let Some((mut child, py)) = child else {
        on_line(
            Stream::Runner,
            0.0,
            &format!(
                "lensbench run: Python を起動できない: {}。--python か環境変数 LENSBENCH_PYTHON で指定する",
                tried.join("、")
            ),
        );
        return Ok(NO_PYTHON);
    };
    on_line(
        Stream::Runner,
        0.0,
        &format!(
            "lensbench run: {}{}（Python: {py}{}{}）",
            a.script.display(),
            a.args.iter().map(|x| format!(" {x}")).collect::<String>(),
            r.threads
                .map(|n| format!("、スレッド {n}"))
                .unwrap_or_default(),
            r.out
                .as_ref()
                .map(|o| format!("、出力 {}", o.display()))
                .unwrap_or_default(),
        ),
    );
    let (tx, rx) = mpsc::channel::<(Stream, String)>();
    let pump = |src: Box<dyn Read + Send>, s: Stream, tx: mpsc::Sender<(Stream, String)>| {
        std::thread::spawn(move || {
            let mut rd = BufReader::new(src);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match rd.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buf);
                        let line = line.trim_end_matches(['\n', '\r']).to_string();
                        if tx.send((s, line)).is_err() {
                            break;
                        }
                    }
                }
            }
        })
    };
    let h1 = pump(
        Box::new(child.stdout.take().expect("piped")),
        Stream::Stdout,
        tx.clone(),
    );
    let h2 = pump(
        Box::new(child.stderr.take().expect("piped")),
        Stream::Stderr,
        tx,
    );
    let mut missing_module = false;
    for (s, line) in rx {
        if s == Stream::Stderr && line.contains("No module named 'lensbench'") {
            missing_module = true;
        }
        on_line(s, t0.elapsed().as_secs_f64(), &line);
    }
    let _ = h1.join();
    let _ = h2.join();
    let status = child
        .wait()
        .map_err(|e| format!("スクリプトの終わりを待てない: {e}"))?;
    let code = exit_code(status);
    if missing_module {
        on_line(
            Stream::Runner,
            t0.elapsed().as_secs_f64(),
            &format!("lensbench run: この Python（{py}）に lensbench が入っていない。pip install で wheel を入れるか、--python で入っている Python を指す"),
        );
    }
    on_line(
        Stream::Runner,
        t0.elapsed().as_secs_f64(),
        &format!(
            "lensbench run: 終了コード {code}（{:.2} s）",
            t0.elapsed().as_secs_f64()
        ),
    );
    Ok(code)
}

/// 終了コード。シグナルで止まった（unix）なら 128 + シグナルの番号、ほかに値がなければ 1
fn exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(c) = status.code() {
        return c;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        128 + status.signal().unwrap_or(0)
    }
    #[cfg(not(unix))]
    {
        1
    }
}

/// コマンドの入口: 画面とログのファイルに出し、終了コードを返す
pub fn run(a: &RunArgs, threads: Option<usize>) -> Result<i32, String> {
    let r = resolve(a, threads)?;
    let mut log = match &r.log {
        Some(p) => {
            if let Some(d) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
                lensbench_core::vfs::create_dir_all(d)
                    .map_err(|e| format!("{}: {e}", d.display()))?;
            }
            Some(std::io::BufWriter::new(
                std::fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()))?,
            ))
        }
        None => None,
    };
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let code = run_script(a, &r, |s, t, line| {
        match s {
            Stream::Stdout => {
                let _ = writeln!(stdout.lock(), "{line}");
            }
            Stream::Stderr | Stream::Runner => {
                let _ = writeln!(stderr.lock(), "{line}");
            }
        }
        if let Some(f) = log.as_mut() {
            let tag = match s {
                Stream::Stdout => "out",
                Stream::Stderr => "err",
                Stream::Runner => "run",
            };
            let _ = writeln!(f, "[{t:9.3}] {tag} {line}");
            let _ = f.flush();
        }
    })?;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::split_command;

    #[test]
    fn python_command_with_spaces() {
        assert_eq!(split_command("py -3"), ["py", "-3"]);
        assert_eq!(
            split_command(r#""C:\Program Files\Python312\python.exe" -X utf8"#),
            [r"C:\Program Files\Python312\python.exe", "-X", "utf8"]
        );
        assert_eq!(split_command("  python3  "), ["python3"]);
        assert!(split_command("  ").is_empty());
        // 全体が在るファイルなら空白を含んでも 1 つのパス
        let dir = std::env::temp_dir().join(format!("lb run {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("my python");
        std::fs::write(&exe, "").unwrap();
        let p = exe.display().to_string();
        assert_eq!(split_command(&p), std::slice::from_ref(&p));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
