//! 玉モード: 基準の系を要素に分けて組み直し、.zmx に書き出して読み戻すと、元と同じ光線になる
//! （tests/refs、LB_REFS_DIR で差し替え）。

use lensbench_core::elements::{self, Kind};
use lensbench_core::system::FieldSel;
use lensbench_core::zmx::{self, LoadOptions};

use crate::common::{glass_dirs, refs_dir};

#[test]
fn split_and_assemble_trace_the_same_rays() {
    let dir = refs_dir();
    let glass = glass_dirs();
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    names.sort();
    let mut n = 0;
    for d in names {
        let f = d.join("system.zmx");
        if !f.exists() {
            continue;
        }
        let opt = LoadOptions {
            cfg: 1,
            catalog_dirs: glass.clone(),
            ..Default::default()
        };
        let s = zmx::load_with(&f, &opt).unwrap();
        let e = match elements::split(&s) {
            Ok(e) => e,
            Err(x) => {
                eprintln!("{}: 分けない（{x}）", d.display());
                continue;
            }
        };
        let summary: Vec<String> = e
            .iter()
            .map(|x| match x.kind {
                Kind::Lens => format!("L{}", x.surfaces.len()),
                Kind::Mirror => "M".into(),
                Kind::CoordBreak => "CB".into(),
                Kind::Surface => "S".into(),
                Kind::Air => "|".into(),
            })
            .collect();
        eprintln!(
            "{}: {}",
            d.file_name().unwrap().to_string_lossy(),
            summary.join(" ")
        );
        let a0 = elements::assemble(&s, &e).unwrap();
        // .zmx に書き出して読み戻した系で比べる（書き出しの確認も兼ねる）
        let txt = lensbench_core::zmx_write::write(&a0).unwrap();
        let raw = zmx::parse_text(&txt).unwrap();
        let a = zmx::build(
            &raw,
            &opt,
            &lensbench_core::glass::catalog_dirs(&glass),
            None,
        )
        .unwrap();
        assert_eq!(a.surfaces.len(), s.surfaces.len());
        let (px, py) = (vec![0.0, 0.7, -0.5, 0.3], vec![0.0, 0.2, 0.6, -0.9]);
        for fi in 0..s.fields.len() {
            for w in 0..s.wavelengths.len() {
                let t0 = s
                    .try_trace(FieldSel::Index(fi), &px, &py, Some(w), None, false)
                    .unwrap();
                let t1 = a
                    .try_trace(FieldSel::Index(fi), &px, &py, Some(w), None, false)
                    .unwrap();
                for i in 0..px.len() {
                    let (r0, r1) = (&t0.rays[i], &t1.rays[i]);
                    assert_eq!((r0.err, r0.vig), (r1.err, r1.vig));
                    if r0.err == 0 {
                        assert_eq!(r0.xyz, r1.xyz);
                        assert_eq!(r0.opl, r1.opl);
                    }
                }
            }
        }
        n += 1;
    }
    assert!(n > 0);
}
