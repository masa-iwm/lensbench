//! 実光線エイミングで絞りに届かない光線がある格子の追跡時間（見直し KRN-07）。
//! `cargo run --profile ci -p lensbench-core --example aim_unreachable -- [瞳座標の倍率]`
use lensbench_core::system::FieldSel;
use lensbench_core::zmx;
use std::time::Instant;

fn main() {
    let k: f64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4.0);
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = zmx::LoadOptions {
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let s = zmx::load_with(&root.join("tests/refs/A2/system.zmx"), &opt).unwrap();
    let n = 45;
    let (mut px, mut py) = (Vec::new(), Vec::new());
    for i in 0..n {
        for j in 0..n {
            px.push(k * (2.0 * i as f64 / (n - 1) as f64 - 1.0));
            py.push(k * (2.0 * j as f64 / (n - 1) as f64 - 1.0));
        }
    }
    for f in 0..s.fields.len() {
        let t0 = Instant::now();
        let t = s
            .try_trace(FieldSel::Index(f), &px, &py, None, None, false)
            .unwrap();
        let bad = (0..px.len()).filter(|&i| !t.traced(i)).count();
        println!(
            "視野 {} : {:.3} s、届かない光線 {bad} / {}",
            f + 1,
            t0.elapsed().as_secs_f64(),
            px.len()
        );
    }
}
