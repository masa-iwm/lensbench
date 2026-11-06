//! 実光線エイミング: 本数が多いとき（格子の光線から初期値を当てはめ、ヤコビアンを固定して反復する）と
//! 少ないとき（光線ごとのニュートン法）で、絞りでの位置が同じ許容差で目標に合い、互いに一致する。

use std::path::PathBuf;

use lensbench_core::system::Aiming;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::FieldSel;

#[test]
fn many_rays_agree_with_few_rays() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    // 2000 本（当てはめを使う）を 250 本ずつ（使わない）と比べる。瞳の外の光線も含む
    let n = 2000;
    let px: Vec<f64> = (0..n)
        .map(|i| ((i * 7919) % 1000) as f64 / 1000.0 * 2.4 - 1.2)
        .collect();
    let py: Vec<f64> = (0..n)
        .map(|i| ((i * 104729) % 1000) as f64 / 1000.0 * 2.4 - 1.2)
        .collect();
    for name in ["A0", "A2", "B4", "C0", "E1"] {
        let opt = LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let mut s =
            zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap();
        s.aiming = Aiming::Real;
        let st = s.stop();
        let (rx, ry) = s.stop_radius();
        let r = rx.abs().max(ry.abs()).max(1.0);
        for fi in 0..s.fields.len() {
            let f = FieldSel::Index(fi);
            let all = s.trace(f, &px, &py, None, Some(st), false);
            let mut ok = 0;
            for c in (0..n).step_by(250) {
                let part = s.trace(f, &px[c..c + 250], &py[c..c + 250], None, Some(st), false);
                for (k, b) in part.rays.iter().enumerate() {
                    let a = &all.rays[c + k];
                    assert_eq!(
                        (a.err, a.vig),
                        (b.err, b.vig),
                        "{name} 視野 {fi} 光線 {}",
                        c + k
                    );
                    if a.err != 0 || a.vig != 0 {
                        continue;
                    }
                    ok += 1;
                    let i = c + k;
                    for (x, t) in [(a.xyz[0], px[i] * rx), (a.xyz[1], py[i] * ry)] {
                        assert!(
                            (x - t).abs() <= 1e-12 * r,
                            "{name} 視野 {fi} 光線 {i}: {x} {t}"
                        );
                    }
                    for j in 0..3 {
                        assert!(
                            (a.xyz[j] - b.xyz[j]).abs() <= 2e-12 * r,
                            "{name} 視野 {fi} 光線 {i}: {:?} {:?}",
                            a.xyz,
                            b.xyz
                        );
                    }
                }
            }
            assert!(ok > n / 2, "{name} 視野 {fi}: 通った光線が少ない ({ok})");
        }
    }
}
