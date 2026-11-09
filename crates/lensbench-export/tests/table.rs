//! 追跡結果の表: 値が追跡と一致し、npz が NumPy の形式で読めること。

use lensbench_core::{FieldSel, System};
use lensbench_export::table::{self, Col, RayOptions};
use ndarray::Array1;
use ndarray_npy::NpzReader;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn zmx(name: &str) -> System {
    let opt = lensbench_core::zmx::LoadOptions {
        catalog_dirs: vec![format!("{ROOT}/tests/glass").into()],
        ..Default::default()
    };
    lensbench_core::zmx::load_with(
        format!("{ROOT}/tests/refs/{name}/system.zmx").as_ref(),
        &opt,
    )
    .unwrap()
}

fn f64s(t: &table::Table, n: &str) -> Vec<f64> {
    match t.get(n) {
        Some(Col::F64(v)) => v.clone(),
        c => panic!("{n}: {c:?}"),
    }
}

fn i64s(t: &table::Table, n: &str) -> Vec<i64> {
    match t.get(n) {
        Some(Col::I64(v)) => v.clone(),
        c => panic!("{n}: {c:?}"),
    }
}

#[test]
fn ray_table_matches_the_trace() {
    let s = zmx("A0");
    let (px, py) = (vec![0.0, 0.3, -0.7], vec![1.0, -0.2, 0.5]);
    let img = s.image();
    let o = RayOptions {
        fields: Some(vec![2]),
        waves: Some(vec![0, 2]),
        px: px.clone(),
        py: py.clone(),
        surfaces: Some(vec![1, img]),
    };
    let t = table::rays(&s, &o).unwrap();
    assert_eq!(t.rows(), 2 * 3 * 2);
    let (x, l, opl) = (f64s(&t, "x"), f64s(&t, "l"), f64s(&t, "opl"));
    let (surf, wave) = (i64s(&t, "surface"), i64s(&t, "wave"));
    for (k, w) in [0usize, 2].into_iter().enumerate() {
        let tr = s.trace(FieldSel::Index(2), &px, &py, Some(w), None, false);
        for i in 0..3 {
            // 像面の行
            let r = k * 6 + i * 2 + 1;
            assert_eq!(surf[r], img as i64);
            assert_eq!(wave[r], w as i64 + 1);
            assert!((x[r] - tr.rays[i].xyz[0]).abs() < 1e-12);
            assert!((l[r] - tr.rays[i].lmn[0]).abs() < 1e-12);
            assert!((opl[r] - tr.rays[i].opl).abs() < 1e-9);
        }
    }
}

#[test]
fn npz_is_readable() {
    let s = zmx("A0");
    let (px, py) = lensbench_export::scene::pupil(lensbench_export::scene::Fan::Grid, 3);
    let o = RayOptions {
        fields: None,
        waves: None,
        px,
        py,
        surfaces: None,
    };
    let t = table::rays(&s, &o).unwrap();
    let mut b = Vec::new();
    t.write_npz(&mut b).unwrap();
    let mut z = NpzReader::new(std::io::Cursor::new(b)).unwrap();
    let gz: Array1<f64> = z.by_name("gz.npy").unwrap();
    assert_eq!(gz.to_vec(), f64s(&t, "gz"));
    let sf: Array1<i64> = z.by_name("surface.npy").unwrap();
    assert_eq!(sf.to_vec(), i64s(&t, "surface"));
    let wl: Array1<f64> = z.by_name("wavelengths_um.npy").unwrap();
    assert_eq!(wl.to_vec(), s.wavelengths);
}
