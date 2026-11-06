//! dOPL/dp を全パラメータ・全系で書き出す（実装の新旧比較用）: `grad_dump [--glass-dir D] [zmx ...]`
use std::path::PathBuf;

use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::FieldSel;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut dirs = vec![];
    let mut files = vec![];
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < a.len() {
        if a[i] == "--glass-dir" {
            dirs.push(PathBuf::from(&a[i + 1]));
            i += 2;
        } else {
            files.push(PathBuf::from(&a[i]));
            i += 1;
        }
    }
    dirs.push(root.join("tests/glass"));
    if files.is_empty() {
        let mut v: Vec<_> = std::fs::read_dir(root.join("tests/refs"))
            .unwrap()
            .map(|e| e.unwrap().path().join("system.zmx"))
            .collect();
        v.sort();
        files = v;
    }
    let (px, py): (Vec<f64>, Vec<f64>) = (0..400)
        .map(|k| {
            let t = k as f64 * 2.399963;
            let r = ((k as f64 + 0.5) / 400.0).sqrt();
            (r * t.cos(), r * t.sin())
        })
        .unzip();
    for f in files {
        let opt = LoadOptions {
            cfg: 1,
            catalog_dirs: dirs.clone(),
            ..Default::default()
        };
        let s = zmx::load_with(&f, &opt).unwrap();
        let mut ps = vec![];
        for j in 0..=s.image() {
            for k in [
                "c", "k", "a4", "t", "dx", "dy", "tx", "ty", "tz", "dn", "disp",
            ] {
                if let Ok(p) = s.param(&format!("{j}.{k}")) {
                    if s.trace_grad(FieldSel::Index(0), &[0.0], &[0.0], None, None, &[p])
                        .is_ok()
                    {
                        ps.push(p);
                    }
                }
            }
        }
        for k in ["objx", "objy"] {
            if let Ok(p) = s.param(k) {
                if s.trace_grad(FieldSel::Index(0), &[0.0], &[0.0], None, None, &[p])
                    .is_ok()
                {
                    ps.push(p);
                }
            }
        }
        for fi in 0..s.fields.len() {
            match s.trace_grad(FieldSel::Index(fi), &px, &py, None, None, &ps) {
                Ok((_, g)) => {
                    let v: Vec<String> = g.iter().map(|x| format!("{x:e}")).collect();
                    let names: Vec<String> = ps.iter().map(|p| p.name()).collect();
                    eprintln!("{} {}", f.display(), names.join(","));
                    println!("{} {fi} {} {}", f.display(), ps.len(), v.join(" "));
                }
                Err(e) => println!("{} {fi} err {e}", f.display()),
            }
        }
    }
}
