//! figures の試験（figures.rs から分けた）
use super::*;

#[test]
fn params_and_freqs() {
    let mut m = Map::new();
    figure_params("wavefront", &mut m, None, true);
    assert_eq!(m["map"], json!(true));
    let mut m = Map::new();
    figure_params("wavefront", &mut m, None, false);
    assert!(m.is_empty());
    figure_params("mtf-focus", &mut m, None, false);
    assert_eq!(m["freqs"], json!([10.0, 20.0, 40.0]));
    let mut m = Map::new();
    m.insert("freqs".into(), json!([5]));
    figure_params("mtf-field", &mut m, Some(300.0), true);
    assert_eq!(m["freqs"], json!([5]));
    assert_eq!(default_freqs(Some(325.4)), vec![33.0, 81.0, 160.0]);
    assert_eq!(default_freqs(None), vec![10.0, 20.0, 40.0]);
    assert!(has_figure("interference") && !has_figure("info"));
    let mut m = Map::new();
    figure_params("interference", &mut m, None, true);
    assert_eq!(m["grid"], json!(true));
}

#[test]
fn wavefront_and_fringes() {
    let item = json!({
        "field": 1, "pv": 0.5, "rms": 0.1,
        "p": [-1.0, 0.0, 1.0],
        "map": [[null, 0.0, null], [0.0, 0.25, 0.0], [null, 0.5, null]],
    });
    let item: WavefrontRow = serde_json::from_value(item).unwrap();
    let g = wavefront_grid(&item).unwrap();
    // 行 0 は +y（map の最後の行）
    assert_eq!(g.v[1], 0.5);
    assert!(g.v[0].is_nan());
    assert_eq!(g.extent, [-1.5, 1.5, -1.5, 1.5]);
    assert_eq!(g.at(0.0, 1.0), Some(0.5));
    assert_eq!(g.range(), Some((0.0, 0.5)));
    let ig = interferogram(&g, &FringeCfg::default());
    // W = 0.25 波 → cos(π/2) = 0 → 0.5、W = 0.5 → 0
    assert!((ig.v[4] - 0.5).abs() < 1e-12);
    assert!(ig.v[1].abs() < 1e-12);
    assert!(ig.v[0].is_nan());
    let up = g.upsample(2);
    assert_eq!((up.w, up.h), (6, 6));
    assert!(up.v.iter().any(|x| x.is_finite()));
}

#[test]
fn pop_grid_rows_top_first() {
    let v = json!({ "x": [-1.0, 0.0, 1.0], "y": [-0.5, 0.5],
        "grid_irradiance": [1.0, 2.0, 3.0, 4.0, 5.0, 6.0] });
    let g = pop_grid(&v).unwrap();
    assert_eq!((g.w, g.h), (3, 2));
    assert_eq!(g.v, vec![4.0, 5.0, 6.0, 1.0, 2.0, 3.0]);
    assert_eq!(g.extent, [-1.5, 1.5, -1.0, 1.0]);
    assert!(pop_grid(&json!({ "x": [0.0], "y": [0.0] })).is_none());
}

#[test]
fn psf_grid() {
    let mut pts = Vec::new();
    for (iy, y) in [-1e-3, 0.0, 1e-3].iter().enumerate() {
        for (ix, x) in [-1e-3, 0.0, 1e-3].iter().enumerate() {
            pts.push(json!({ "field": 1, "x": x, "y": y, "i": (iy * 3 + ix) as f64 }));
        }
    }
    let v = json!({ "unit": "lens", "method": "fft", "psf": pts, "axes": [], "sampling": [],
                    "warnings": [] });
    let g = psf_grids(&serde_json::from_value(v).unwrap());
    assert_eq!(g.len(), 1);
    let g = &g[0].1;
    assert_eq!((g.w, g.h), (3, 3));
    // 行 0 は +y（iy = 2）
    assert_eq!(g.v[0], 6.0);
    assert!((g.extent[1] - 1.5).abs() < 1e-9);
}

#[test]
fn crop() {
    let mut v = vec![0.0; 100];
    v[5 * 10 + 5] = 1.0;
    let g = Grid {
        w: 10,
        h: 10,
        v,
        extent: [0.0, 10.0, 0.0, 10.0],
    };
    let c = crop_to_signal(&g, 1e-3);
    assert_eq!((c.w, c.h), (9, 9));
    // 値の位置は切り出しても同じ座標
    assert_eq!(c.at(5.5, 4.5), Some(1.0));
    assert_eq!(g.at(5.5, 4.5), Some(1.0));
}

#[test]
fn interference_grids() {
    // 2×2、j = 0 が下（y の小さい側）
    let v = json!({
        "mode": "detector", "quantity": "intensity", "nx": 2, "ny": 2,
        "top_left_pixel_center_mm": [-0.5, 0.5], "pixel_mm": 1.0,
        "grid": [1.0, 2.0, 3.0, null],
    });
    let g = interference_grid(&v).unwrap();
    assert_eq!(g.extent, [-1.0, 1.0, -1.0, 1.0]);
    assert_eq!(g.at(-0.5, 0.5), Some(3.0));
    assert_eq!(g.at(0.5, 0.5), None);
    assert_eq!(g.at(0.5, -0.5), Some(2.0));
    let p = json!({
        "mode": "pupil", "nx": 2, "ny": 2, "pupil_min": -0.5, "pupil_step": 1.0,
        "grid": [0.0, 0.0, 1.0, 1.0],
    });
    let g = interference_grid(&p).unwrap();
    assert_eq!(g.v, vec![1.0, 1.0, 0.0, 0.0]);
    assert_eq!(g.extent, [-1.0, 1.0, -1.0, 1.0]);
    assert!(interference_grid(&json!({ "nx": 2, "ny": 2 })).is_none());
}

#[test]
fn colors() {
    assert_eq!(cmap(Cmap::Gray, 1.0), Color32::from_gray(255));
    assert_eq!(
        cmap(Cmap::Viridis, 0.0),
        Color32::from_rgb(0x44, 0x01, 0x54)
    );
    assert_eq!(
        cmap(Cmap::CoolWarm, f64::NAN),
        Color32::from_rgb(0x3B, 0x4C, 0xC0)
    );
}
