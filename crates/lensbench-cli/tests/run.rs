//! `lensbench run`: 外の Python でスクリプトを実行し、設定を環境変数で渡す。
//! Python が見つからない環境では、起動できる場合のテストは飛ばす（起動できない場合の終了コードは確かめる）

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| {
        Command::new(p)
            .arg("-c")
            .arg("import sys; sys.exit(0 if sys.version_info >= (3, 8) else 1)")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lensbench-run-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn lb(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lensbench"))
        .current_dir(dir)
        .args(args)
        .env_remove("LENSBENCH_GLASSCAT")
        .env_remove("LENSBENCH_COATINGS")
        .env_remove("LENSBENCH_THREADS")
        .env_remove("LENSBENCH_PYTHON")
        .env_remove("VIRTUAL_ENV")
        .output()
        .unwrap()
}

const SCRIPT: &str = r#"
import json, os, sys
keys = ["LENSBENCH_RUN", "LENSBENCH_THREADS", "LENSBENCH_OUT", "LENSBENCH_LOG",
        "LENSBENCH_GLASSCAT", "LENSBENCH_COATINGS", "EXTRA"]
print(json.dumps({"args": sys.argv[1:], **{k: os.environ.get(k) for k in keys}}))
print("lensbench:progress 1/2 half", file=sys.stderr)
out = os.environ.get("LENSBENCH_OUT")
if out:
    open(os.path.join(out, "done.txt"), "w").write("ok")
sys.exit(int(sys.argv[1]) if len(sys.argv) > 1 and sys.argv[1].isdigit() else 0)
"#;

fn env_of(o: &Output) -> serde_json::Value {
    let s = String::from_utf8_lossy(&o.stdout);
    let line = s.lines().find(|l| l.starts_with('{')).expect("JSON の行");
    serde_json::from_str(line).unwrap()
}

#[test]
fn passes_settings_and_returns_the_exit_code() {
    let Some(py) = python() else { return };
    let d = tmp("pass");
    std::fs::write(d.join("s.py"), SCRIPT).unwrap();
    let o = lb(
        &d,
        &[
            "run",
            "--python",
            py,
            "--threads",
            "3",
            "--out",
            "res",
            "--glass-dir",
            "g1",
            "--coatings",
            "c.lbcm",
            "s.py",
            "7",
            "--flag",
            "-x",
        ],
    );
    assert_eq!(o.status.code(), Some(7), "{o:?}");
    let v = env_of(&o);
    assert_eq!(v["args"], serde_json::json!(["7", "--flag", "-x"]));
    assert_eq!(v["LENSBENCH_RUN"], "1");
    assert_eq!(v["LENSBENCH_THREADS"], "3");
    // パスは絶対パスにして渡す
    let out = PathBuf::from(v["LENSBENCH_OUT"].as_str().unwrap());
    assert!(out.is_absolute() && out.ends_with("res"));
    assert!(PathBuf::from(v["LENSBENCH_GLASSCAT"].as_str().unwrap()).ends_with("g1"));
    assert!(PathBuf::from(v["LENSBENCH_COATINGS"].as_str().unwrap()).ends_with("c.lbcm"));
    assert_eq!(
        std::fs::read_to_string(d.join("res/done.txt")).unwrap(),
        "ok"
    );
    // ログは既定で <out>/run.log。標準出力・標準エラー・開始と終了を経過時間つきで残す
    let log = std::fs::read_to_string(d.join("res/run.log")).unwrap();
    assert!(log.contains("] out {\"args\""), "{log}");
    assert!(log.contains("] err lensbench:progress 1/2 half"), "{log}");
    assert!(log.contains("終了コード 7"), "{log}");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("lensbench:progress 1/2 half"), "{err}");
}

#[test]
fn reads_a_config_file_and_arguments_take_precedence() {
    let Some(py) = python() else { return };
    let d = tmp("config");
    std::fs::create_dir_all(d.join("conf")).unwrap();
    std::fs::write(d.join("s.py"), SCRIPT).unwrap();
    std::fs::write(
        d.join("conf/run.json"),
        format!(
            r#"{{"python": "{py}", "threads": 2, "out": "o", "glass_dirs": ["gl"],
                "env": {{"EXTRA": "x"}}}}"#
        ),
    )
    .unwrap();
    let o = lb(
        &d,
        &["run", "--config", "conf/run.json", "--no-log", "s.py"],
    );
    assert!(o.status.success(), "{o:?}");
    let v = env_of(&o);
    assert_eq!(v["LENSBENCH_THREADS"], "2");
    assert_eq!(v["EXTRA"], "x");
    // 相対パスは設定ファイルのフォルダから
    assert!(PathBuf::from(v["LENSBENCH_OUT"].as_str().unwrap()).ends_with("conf/o"));
    assert!(PathBuf::from(v["LENSBENCH_GLASSCAT"].as_str().unwrap()).ends_with("conf/gl"));
    assert!(v["LENSBENCH_LOG"].is_null());
    assert!(!d.join("conf/o/run.log").exists());
    // 引数が優先する
    let o = lb(
        &d,
        &[
            "run",
            "--config",
            "conf/run.json",
            "--threads",
            "5",
            "--log",
            "l.txt",
            "s.py",
        ],
    );
    assert!(o.status.success(), "{o:?}");
    assert_eq!(env_of(&o)["LENSBENCH_THREADS"], "5");
    assert!(d.join("l.txt").exists());
    // 知らない設定は止める
    std::fs::write(d.join("conf/bad.json"), r#"{"thread": 2}"#).unwrap();
    let o = lb(&d, &["run", "--config", "conf/bad.json", "s.py"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("知らない設定 thread"));
}

#[test]
fn missing_python_or_script_fails_with_a_clear_code() {
    let d = tmp("missing");
    std::fs::write(d.join("s.py"), "print(1)").unwrap();
    let o = lb(&d, &["run", "--python", "no-such-python-lensbench", "s.py"]);
    assert_eq!(o.status.code(), Some(127));
    assert!(String::from_utf8_lossy(&o.stderr).contains("Python を起動できない"));
    let o = lb(&d, &["run", "no-such-script.py"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("スクリプトがない"));
}
