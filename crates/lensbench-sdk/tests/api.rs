//! 約束の型の一覧（API.txt）が src/lib.rs の再輸出と合う。約束を変える PR では差分に必ず出る。
//! 直すときは `LB_UPDATE_API=1 cargo test -p lensbench-sdk --test api` で書き直す

use std::path::Path;

/// lib.rs の `pub use` を「モジュール: 名前」の行にする（cfg の付いたモジュールも含める）
fn exports(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut module: Vec<(String, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut stmt: Option<String> = None;
    for line in src.lines() {
        let t = line.trim();
        if t.starts_with("//") {
            continue;
        }
        if let Some(s) = stmt.as_mut() {
            s.push(' ');
            s.push_str(t);
            if t.ends_with(';') {
                let s = stmt.take().unwrap_or_default();
                push(&mut out, &module, &s);
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("pub mod ") {
            if let Some(name) = rest.strip_suffix(" {") {
                module.push((name.to_string(), depth));
            }
        }
        if t.starts_with("pub use ") {
            if t.ends_with(';') {
                push(&mut out, &module, t);
            } else {
                stmt = Some(t.to_string());
            }
            continue;
        }
        depth += t.matches('{').count();
        depth -= t.matches('}').count().min(depth);
        while module.last().is_some_and(|(_, d)| *d >= depth) {
            module.pop();
        }
    }
    out.sort();
    out
}

fn push(out: &mut Vec<String>, module: &[(String, usize)], stmt: &str) {
    let m = module
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join("::");
    let m = if m.is_empty() {
        "(root)".to_string()
    } else {
        m
    };
    let body = stmt
        .trim_start_matches("pub use ")
        .trim_end_matches(';')
        .trim();
    let (prefix, items) = match body.find('{') {
        Some(i) => (body[..i].trim(), body[i + 1..].trim_end_matches('}')),
        None => match body.rfind("::") {
            Some(i) => (&body[..i + 2], &body[i + 2..]),
            None => ("", body),
        },
    };
    for it in items.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (path, shown) = it.split_once(" as ").unwrap_or((it, it));
        out.push(format!("{m}: {shown} = {prefix}{path}"));
    }
}

#[test]
fn api_list_matches_the_reexports() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
    let got = exports(&src).join("\n") + "\n";
    let path = dir.join("API.txt");
    if std::env::var_os("LB_UPDATE_API").is_some() {
        std::fs::write(&path, &got).unwrap();
    }
    // Windows の checkout では改行が CRLF になる（autocrlf）ので LF に揃えて比べる
    let want = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert_eq!(
        want, got,
        "API.txt differs from the re-exports in src/lib.rs. If the change is intended, run \
         LB_UPDATE_API=1 cargo test -p lensbench-sdk --test api and note it in docs/changes.md"
    );
}
