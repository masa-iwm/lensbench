//! 記録する面を選んだ追跡（RecSel::Surfs）が、全面の記録の該当する面とビット単位で一致する
//! （まとめ追跡あり・なし、スレッド数 1・4）。

use std::path::PathBuf;

use lensbench_core::par::with_batch;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{FieldSel, RecSel, System};

fn load(name: &str) -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap()
}

fn bits(v: &[[f64; 3]]) -> Vec<u64> {
    v.iter().flatten().map(|x| x.to_bits()).collect()
}

#[test]
fn selected_surfaces_match_the_full_record() {
    let mut px = Vec::new();
    let mut py = Vec::new();
    for i in 0..41 {
        for j in 0..37 {
            px.push(-1.1 + 2.2 * i as f64 / 40.0);
            py.push(-1.1 + 2.2 * j as f64 / 36.0);
        }
    }
    for name in ["A0", "A2", "C0", "D0"] {
        let s = load(name);
        let f = FieldSel::Index(1.min(s.fields.len() - 1));
        let img = s.image();
        let sel = [img, 2, 3, img + 5];
        for on in [true, false] {
            for th in [1, 4] {
                with_batch(on, || {
                    lensbench_core::with_threads(Some(th), || {
                        let all = s.try_trace(f, &px, &py, None, None, true).unwrap();
                        let part = s
                            .try_trace_rec(f, &px, &py, None, None, RecSel::Surfs(&sel))
                            .unwrap();
                        let surfs: Vec<usize> = part.rec.iter().map(|r| r.surf).collect();
                        assert_eq!(surfs, vec![2, 3, img], "{name}");
                        for r in &part.rec {
                            let a = all.rec_at(r.surf).unwrap();
                            assert_eq!(bits(&a.xyz), bits(&r.xyz), "{name} {}", r.surf);
                            assert_eq!(bits(&a.lmn), bits(&r.lmn), "{name} {}", r.surf);
                            assert_eq!(bits(&a.p), bits(&r.p), "{name} {}", r.surf);
                            let (oa, or): (Vec<u64>, Vec<u64>) = (
                                a.opl.iter().map(|x| x.to_bits()).collect(),
                                r.opl.iter().map(|x| x.to_bits()).collect(),
                            );
                            assert_eq!(oa, or, "{name} {}", r.surf);
                            assert_eq!(a.parity, r.parity);
                        }
                        for i in 0..px.len() {
                            assert_eq!(all.rays[i].opl.to_bits(), part.rays[i].opl.to_bits());
                        }
                        // dOPL/dp は全面の記録が要る
                        let p = [s.param("2.t").unwrap()];
                        assert!(s.opl_grad(&part, s.primary, &p).is_err());
                    })
                });
            }
        }
    }
}
