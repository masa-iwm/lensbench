//! 拡張のサブコマンド: この CLI を組み込んだ exe（非公開版など）が足す解析。
//!
//! 足したサブコマンドは、同梱のものと同じく CLI・`serve` の `analyze`・`watch`・GUI の解析（GUI 側の登録は
//! `lensbench_gui::ext`）で使える。exe の入口で [`register`] してから [`crate::main`] を呼ぶ。
//!
//! - 引数は clap の [`clap::Command`] で決める（[`Command::command`]）。系を読む解析は [`Common`]（と視野・波長の
//!   [`Sel`]）を `#[command(flatten)]` で入れると、serve の `analyze` が読み込んだ系の読み方（ガラス・構成・状態など）を
//!   当て、`--save` を書き出す。
//! - 実行は [`Args::run`]。serve の中では [`Ctx::serve`] で読み込んだ系の読み方と、読み込んだ後の変更（set・edit）を
//!   別に読んだ系に当てる口（[`Served::replay`]）を使える。
//! - GUI の引数の欄（`arg_spec::analysis_args`）は clap の定義から作る。文字列で受けて値が決まっている引数は
//!   [`Command::choices`] に書く。

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use lensbench_core::System;
use serde_json::Value;

pub use crate::{Common, Sel};

/// 解析を 1 回実行する引数（[`Command::parse`] が作る）
pub trait Args: Send {
    /// 系を読む解析の共通の引数（serve の analyze が系の読み方を当て、`--save` を書き出す）。
    /// [`Args::common_mut`] と同じものを返す
    fn common(&self) -> Option<&Common> {
        None
    }
    /// [`Args::common`] の書き換えられる方
    fn common_mut(&mut self) -> Option<&mut Common> {
        None
    }
    /// 解析が読み書きするファイルと、書き出しか（serve で場所を確かめる）。`Common` の `--save` も含める
    /// （既定は `--save` だけ）
    fn paths_mut(&mut self) -> Vec<(&mut PathBuf, bool)> {
        self.common_mut()
            .and_then(|c| c.save.as_mut())
            .map(|p| vec![(p, true)])
            .unwrap_or_default()
    }
    /// 実行して (JSON で出すか, 結果) を返す
    fn run(self: Box<Self>, ctx: &Ctx) -> Result<(bool, Value), String>;
}

/// 拡張のサブコマンド
#[derive(Clone, Copy)]
pub struct Command {
    /// サブコマンドの名前（同梱のものと同じ名前は使えない）
    pub name: &'static str,
    /// 引数の定義（名前は [`Command::name`] に置き換える）
    pub command: fn() -> clap::Command,
    /// 解釈した引数から実行する引数を作る
    pub parse: fn(&clap::ArgMatches) -> Result<Box<dyn Args>, String>,
    /// 文字列で受けて値が決まっている引数（引数の名前（- は _）, 値の並び）。GUI の選択の欄にする
    pub choices: &'static [(&'static str, &'static [&'static str])],
}

static COMMANDS: RwLock<Vec<Command>> = RwLock::new(Vec::new());

/// サブコマンドを足す（同じ名前は置き換える）。同梱のサブコマンドと同じ名前は誤り
pub fn register(c: Command) -> Result<(), String> {
    if crate::builtin_command_names().iter().any(|n| n == c.name) {
        return Err(format!("サブコマンド {} は同梱のものと同じ名前", c.name));
    }
    let mut v = COMMANDS.write().unwrap_or_else(|e| e.into_inner());
    v.retain(|x| x.name != c.name);
    v.push(c);
    Ok(())
}

/// 名前のサブコマンド
pub fn get(name: &str) -> Option<Command> {
    list().into_iter().find(|c| c.name == name)
}

/// 足したサブコマンド（足した順）
pub fn list() -> Vec<Command> {
    COMMANDS.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 実行の場（CLI か serve か）
#[derive(Default)]
pub struct Ctx<'a> {
    /// serve の analyze から呼んだとき、読み込んだ系
    pub serve: Option<&'a dyn Served>,
}

/// serve で読み込んだ系（[`Ctx::serve`]）
pub trait Served {
    /// load で選んだ系の読み方（ファイル・ガラス・カタログ・構成・状態・経路）
    fn common(&self) -> Common;
    /// 解析が読み書きするファイルの場所を確かめる（serve の --files の設定）
    fn check_path(&self, p: &Path, write: bool) -> Result<PathBuf, String>;
    /// 別に読んだ系（別の構成・状態など）に、読み込んだ後の変更（set・edit）を当てる。当てなかった変更は notes に書く。
    /// label は説明に付ける名前
    fn replay(&self, s: System, label: &str, notes: &mut Vec<String>) -> Result<System, String>;
}

/// 系を読む（serve で読み込んであればその系、なければファイルから。読み込みの注意は標準エラーへ）。
/// `--save` の JSON の provenance に刻む hash もここで覚える
pub fn load(c: &Common) -> Result<System, String> {
    crate::load(c)
}

/// ファイルから系を読む（serve の読み込んだ系は使わない。別の構成・状態の系を読むとき）
pub fn load_file(c: &Common) -> Result<System, String> {
    crate::load_file(c)
}

/// 視野の番号（0 始まり）の並び（`--field` を省けば全視野）
pub fn fields(s: &System, sel: &Sel) -> Result<Vec<usize>, String> {
    crate::fields(s, sel)
}

/// 波長の番号（0 始まり、`--wave` を省けば None = 多色・主波長）
pub fn wave(s: &System, sel: &Sel) -> Result<Option<usize>, String> {
    crate::wave(s, sel)
}

/// プロジェクトファイル（.json、系ファイルを除く）を読み、状態（`--state`）を当てたもの。.zmx・系ファイルなら None
pub fn project(c: &Common) -> Result<Option<lensbench_project::Project>, String> {
    if !c
        .file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        || lensbench_project::sysfile::is_system_file(&c.file)
    {
        return Ok(None);
    }
    let t = lensbench_core::vfs::read_to_string(&c.file)
        .map_err(|x| format!("{}: {x}", c.file.display()))?;
    let mut p = lensbench_project::Project::from_json(&t)?;
    if let Some(st) = &c.state {
        p = p.with_state(st)?;
    }
    Ok(Some(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, FromArgMatches, Parser};
    use serde_json::json;

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    /// 試しの拡張: 系を読み、視野の数と引数を返す
    #[derive(Parser)]
    struct Toy {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// 試しの選択
        #[arg(long, default_value = "a")]
        mode: String,
        /// 設定のオブジェクト（serve の analyze で JSON を渡したとき）
        #[arg(long, hide = true)]
        opt_json: Option<String>,
    }

    impl Args for Toy {
        fn common(&self) -> Option<&Common> {
            Some(&self.c)
        }
        fn common_mut(&mut self) -> Option<&mut Common> {
            Some(&mut self.c)
        }
        fn run(self: Box<Self>, ctx: &Ctx) -> Result<(bool, Value), String> {
            let s = load(&self.c)?;
            let f = fields(&s, &self.s)?;
            let label = ctx.serve.map(|x| {
                let mut notes = Vec::new();
                let s2 = x.replay(load_file(&x.common()).unwrap(), "別の系", &mut notes);
                (s2.unwrap().surfaces.len(), notes.len())
            });
            Ok((
                self.c.json,
                json!({
                    "fields": f, "mode": self.mode, "served": label,
                    "opt": self.opt_json.map(|t| serde_json::from_str::<Value>(&t).unwrap()),
                }),
            ))
        }
    }

    fn register_toy() {
        register(Command {
            name: "toy-ext",
            command: Toy::command,
            parse: |m| {
                Ok(Box::new(
                    Toy::from_arg_matches(m).map_err(|e| e.to_string())?,
                ))
            },
            choices: &[("mode", &["a", "b"])],
        })
        .unwrap();
    }

    #[test]
    fn ext_command_runs_in_cli_serve_and_arg_spec() {
        register_toy();
        let zmx = format!("{ROOT}/tests/refs/A0/system.zmx");
        let glass = format!("{ROOT}/tests/glass");
        // CLI
        let cli = crate::parse_cli([
            "lensbench",
            "--threads",
            "2",
            "toy-ext",
            &zmx,
            "--glass-dir",
            &glass,
            "--field",
            "2",
            "--mode",
            "b",
        ])
        .unwrap();
        assert_eq!(cli.threads, Some(2));
        let (_, v) = crate::execute(cli.cmd).unwrap();
        assert_eq!(v["fields"], json!([1]));
        assert_eq!(v["mode"], "b");
        assert_eq!(v["served"], Value::Null);
        // 引数の誤りは clap の誤り
        assert!(crate::parse_cli(["lensbench", "toy-ext", &zmx, "--nope"]).is_err());
        // GUI の引数の欄: choices の値が選択になり、系の読み方の引数は出ない
        let a = crate::arg_spec::analysis_args("toy-ext").unwrap();
        let m = a.iter().find(|x| x["name"] == "mode").unwrap();
        assert_eq!(m["type"], "choice");
        assert_eq!(m["choices"], json!(["a", "b"]));
        assert!(a
            .iter()
            .all(|x| x["name"] != "glass_dir" && x["name"] != "opt_json"));
        // serve の analyze: 読み込んだ系の読み方と、読み込んだ後の変更を当てる口
        crate::serve::call("load", &json!({ "file": zmx, "glass_dir": [glass] }), None).unwrap();
        let r = crate::serve::call(
            "analyze",
            &json!({ "cmd": "toy-ext", "field": 1, "opt": { "x": [1, 2] } }),
            None,
        )
        .unwrap();
        assert_eq!(r["fields"], json!([0]));
        assert_eq!(r["opt"], json!({ "x": [1, 2] }));
        assert!(r["served"][0].as_u64().unwrap() > 3);
        // 同梱の名前は足せない
        assert!(register(Command {
            name: "spot",
            command: Toy::command,
            parse: |_| Err("x".into()),
            choices: &[],
        })
        .is_err());
    }
}
