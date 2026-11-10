//! 品ページを開くアプリ内のブラウザ（Windows の WebView2、公開 #70）。GUI とは別のプロセス
//! （この exe を `--webview` で起こしたもの）で動かし、ページの文字・落としたファイルを 1 行 1 つの JSON で
//! 標準出力に書く（GUI の [`Child`] が読む）。
//!
//! 開くのはユーザーが選んだ品の 1 ページだけ。ページの文字はページの中で読むだけで、裏でほかのページを
//! 取りに行かない。ボットの判定・captcha はそのままユーザーに見せる（回避しない）。

use std::path::PathBuf;

/// ページの文字を GUI へ送るスクリプト（読み込み時と、表が後から描かれるページのため 2 秒ごと。変わったときだけ）
const SCRIPT: &str = r#"(function () {
  var last = '';
  function send() {
    try {
      var t = document.body ? document.body.innerText : '';
      if (t && t !== last) {
        last = t;
        window.ipc.postMessage(JSON.stringify({ url: location.href, text: t.slice(0, 4000000) }));
      }
    } catch (e) {}
  }
  window.addEventListener('load', send);
  setInterval(send, 2000);
})();"#;

/// 子のプロセスの入口: `--webview URL --download-dir DIR [--title T] [--hidden]`。
/// `--hidden` は窓を見せずに開き、標準入力に `show` が来たら見せる（行を選んだときの裏の読み取り）
pub fn run(args: &[String]) -> std::process::ExitCode {
    use std::io::Write;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::window::WindowBuilder;
    use wry::WebViewBuilder;
    let get = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let Some(url) =
        get("--webview").filter(|u| u.starts_with("https://") || u.starts_with("http://"))
    else {
        eprintln!("--webview には http(s) の URL");
        return std::process::ExitCode::FAILURE;
    };
    let dl = get("--download-dir")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let title = get("--title").unwrap_or_else(|| "lensbench".into());
    let out = |v: serde_json::Value| {
        let mut o = std::io::stdout().lock();
        let _ = writeln!(o, "{v}");
        let _ = o.flush();
    };
    let hidden = args.iter().any(|a| a == "--hidden");
    let event_loop = EventLoopBuilder::<()>::with_user_event().build();
    // 標準入力の `show` で窓を見せる（親が閉じたら終わる）
    let proxy = event_loop.create_proxy();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if line.trim() == "show" && proxy.send_event(()).is_err() {
                break;
            }
        }
    });
    let window = match WindowBuilder::new()
        .with_title(title)
        .with_visible(!hidden)
        .with_inner_size(tao::dpi::LogicalSize::new(1100.0, 850.0))
        .build(&event_loop)
    {
        Ok(w) => w,
        Err(e) => {
            eprintln!("窓を作れない: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let dl2 = dl.clone();
    let webview = WebViewBuilder::new()
        .with_url(&url)
        .with_initialization_script(SCRIPT)
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(req.body()) {
                out(serde_json::json!({ "type": "page", "url": v["url"], "text": v["text"] }));
            }
        })
        .with_download_started_handler(move |_url: String, path: &mut PathBuf| {
            // 落とすファイルは決めたフォルダへ（名前はブラウザの決めたもの）
            let name = path
                .file_name()
                .map(|n| n.to_os_string())
                .unwrap_or_else(|| "download".into());
            let _ = std::fs::create_dir_all(&dl2);
            *path = dl2.join(name);
            true
        })
        .with_download_completed_handler(move |url: String, path: Option<PathBuf>, ok: bool| {
            out(serde_json::json!({
                "type": "download", "url": url, "ok": ok,
                "path": path.map(|p| p.display().to_string()),
            }));
        })
        .build(&window);
    let webview = match webview {
        Ok(w) => w,
        Err(e) => {
            eprintln!("WebView を作れない（WebView2 Runtime が要る）: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    event_loop.run(move |event, _, flow| {
        let _ = &webview;
        *flow = ControlFlow::Wait;
        if let Event::UserEvent(()) = event {
            window.set_visible(true);
            window.set_focus();
            return;
        }
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *flow = ControlFlow::Exit;
        }
    })
}

/// 子のプロセスから届いたもの
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// ページの文字（読み込み・変化のたび）
    Page { url: String, text: String },
    /// 落としたファイル
    Download { path: Option<PathBuf>, ok: bool },
    /// 窓が閉じた（プロセスが終わった）
    Closed,
}

/// 開いている品ページの窓（子のプロセス）。落とすと窓も閉じる
pub struct Child {
    proc: std::process::Child,
    rx: std::sync::mpsc::Receiver<Msg>,
}

impl Child {
    /// この exe を `--webview` で起こす
    pub fn spawn(
        url: &str,
        title: &str,
        download_dir: &std::path::Path,
        hidden: bool,
    ) -> Result<Child, String> {
        use std::io::BufRead;
        use std::process::{Command, Stdio};
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut proc = Command::new(exe)
            .arg("--webview")
            .arg(url)
            .arg("--download-dir")
            .arg(download_dir)
            .arg("--title")
            .arg(title)
            .args(hidden.then_some("--hidden"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let out = proc.stdout.take().ok_or("標準出力が無い")?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
                let m = match v["type"].as_str() {
                    Some("page") => Msg::Page {
                        url: s("url"),
                        text: s("text"),
                    },
                    Some("download") => Msg::Download {
                        path: v["path"].as_str().map(PathBuf::from),
                        ok: v["ok"] == true,
                    },
                    _ => continue,
                };
                if tx.send(m).is_err() {
                    return;
                }
            }
            let _ = tx.send(Msg::Closed);
        });
        Ok(Child { proc, rx })
    }

    /// 見せずに開いた窓を見せる
    pub fn show(&mut self) {
        use std::io::Write;
        if let Some(i) = self.proc.stdin.as_mut() {
            let _ = writeln!(i, "show");
            let _ = i.flush();
        }
    }

    /// 届いたものを全部
    pub fn poll(&mut self) -> Vec<Msg> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.proc.kill();
        let _ = self.proc.wait();
    }
}
