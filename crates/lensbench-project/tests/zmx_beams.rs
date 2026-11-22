//! 経路ごとの多重構成の .zmx（`zmx --beam-paths`、構成ごとに違う面を IGNR で無視）を `zmx_beams` で BS と
//! 往路を戻るミラーのプロジェクトに読み戻すと、光束の系が元の経路の系と光線で一致する

use lensbench_core::system::{FieldSel, Frame};
use lensbench_core::System;
use lensbench_project::{expand, Env, ExpandOpts, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env() -> Env<'static> {
    Env {
        step_files: None,
        base: root().join("tests/refs"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

fn sys(p: &Project, state: Option<&str>, path: Option<&str>) -> System {
    expand(
        p,
        &env(),
        &ExpandOpts {
            state,
            path,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("{state:?} {path:?}: {e}"))
    .system
}

/// 大域座標の点（point）か向きを、面の座標系 f の局所座標に
fn local(f: &Frame, v: [f64; 3], point: bool) -> [f64; 3] {
    let d = if point {
        [v[0] - f.o[0], v[1] - f.o[1], v[2] - f.o[2]]
    } else {
        v
    };
    let mut out = [0.0; 3];
    for (i, o) in out.iter_mut().enumerate() {
        *o = (0..3).map(|k| f.r[k][i] * d[k]).sum();
    }
    out
}

const PX: [f64; 5] = [0.0, 0.7, -0.5, 0.3, 0.0];
const PY: [f64; 5] = [0.0, 0.2, 0.6, -0.9, -1.0];

#[test]
fn beam_path_configs_read_back_as_beams() {
    // 透過の腕に傾けた面の玉、反射の腕に偏心した面
    let mut tilted = lensbench_core::surface::Surface {
        c: 1.0 / 50.0,
        t: 3.0,
        mat: lensbench_core::material::Material::Glass("N-BK7".into()),
        ..Default::default()
    };
    tilted.semi = 8.0;
    tilted.ap = lensbench_core::surface::Aperture::Circular {
        min: 0.0,
        max: 8.0,
        dx: 0.0,
        dy: 0.0,
    };
    tilted.before = Some(lensbench_core::surface::Tilt {
        d: [0.2, -0.1, 1.5, -0.7, 0.0],
        order: 0,
    });
    tilted.after = lensbench_core::surface::After::Reverse;
    let row =
        serde_json::to_string(&lensbench_project::surface_spec(&tilted, false).unwrap()).unwrap();
    let txt = &r#"{ "glass_catalogs": ["SCHOTT"],
      "wavelengths": [{"um": 0.4861327}, {"um": 0.5875618}], "primary": 2,
      "aperture": {"type": "na", "value": 0.05},
      "fields": {"type": "object_height", "points": [[0, 0], [0, 1.5]]},
      "object_distance": 50, "blocks": [{"type": "stop", "semi": 3}, {"type": "air", "t": 10},
      {"type": "splitter", "id": "BS", "form": "cube", "size": 12, "glass": "N-BK7",
        "t": [{"type": "air", "t": 10},
              {"type": "lens", "id": "L", "source": {"surfaces": [
                ROW,
                {"radius": -80, "semi": 8}]}},
              {"type": "air", "t": 20}, {"type": "mirror", "back": true, "semi": 12}],
        "r": [{"type": "air", "t": 33}, {"type": "mirror", "back": true, "radius": -200, "semi": 12}],
        "d": [{"type": "air", "t": 25}, {"type": "lens", "source": {"surfaces": [{"radius": 40, "t": 4, "glass": "N-BK7", "semi": 8}, {"radius": -60, "semi": 8}]}}, {"type": "air", "t": 50}]}] }"#
        .replace("ROW", &row);
    let p = Project::from_json(txt).unwrap();
    let paths = ["TR", "RT"];
    let orig: Vec<System> = paths.iter().map(|q| sys(&p, None, Some(q))).collect();
    let zmx = lensbench_formats::zmx_write::write_configs(&orig).unwrap();
    let dir = std::env::temp_dir().join(format!("lb-zmx-beams-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("paths.zmx");
    std::fs::write(&f, zmx).unwrap();
    let cfgs: Vec<System> = (1..=2)
        .map(|k| {
            lensbench_formats::zmx::load_with(
                &f,
                &lensbench_formats::zmx::LoadOptions {
                    cfg: k,
                    catalog_dirs: vec![root().join("tests/glass")],
                    ..Default::default()
                },
            )
            .unwrap()
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    // 戻りの面（構成 1 の腕のミラーの後の、曲がった面）の曲率・厚みを変えた .zmx は誤り
    let mi = (1..cfgs[0].surfaces.len())
        .find(|&j| !cfgs[0].surfaces[j].ignore && cfgs[0].surfaces[j].is_mirror())
        .unwrap();
    let back = (mi + 1..cfgs[0].surfaces.len())
        .find(|&j| !cfgs[0].surfaces[j].ignore && cfgs[0].surfaces[j].c != 0.0)
        .unwrap();
    let mut bent = cfgs.clone();
    for c in &mut bent {
        c.edit(|m| {
            m.surfaces[back].c *= 1.01;
            Ok(())
        })
        .unwrap();
    }
    let mut thick = cfgs.clone();
    thick[0]
        .edit(|m| {
            m.surfaces[back].t += 0.1;
            Ok(())
        })
        .unwrap();
    for (what, x) in [("curvature", bent), ("thickness", thick)] {
        let e = lensbench_project::zmx_beams(&x, "paths.zmx", &env())
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("configuration 1:") && e.contains("must retrace the arm"),
            "{what}: {e}"
        );
    }
    let v = lensbench_project::zmx_beams(&cfgs, "paths.zmx", &env()).unwrap();
    let q = Project::from_json(&v.to_string()).unwrap();
    // 腕は往路を戻るミラー（戻りの面は書かない）
    let arms = &v["blocks"].as_array().unwrap().last().unwrap();
    for key in ["t", "r"] {
        let last = arms[key].as_array().unwrap().last().unwrap();
        assert_eq!(
            (last["type"].as_str(), last["back"].as_bool()),
            (Some("mirror"), Some(true))
        );
    }
    for (k, a) in orig.iter().enumerate() {
        let mut passed = 0;
        let b = sys(&q, Some(&format!("C{}", k + 1)), None);
        let image = |x: &System| x.frames().surf.last().cloned().flatten().unwrap();
        let (fa, fb) = (image(a), image(&b));
        for f in 0..a.fields.len() {
            for w in 0..a.wavelengths.len() {
                let ta = a
                    .try_trace(FieldSel::Index(f), &PX, &PY, Some(w), None, false)
                    .unwrap();
                let tb = b
                    .try_trace(FieldSel::Index(f), &PX, &PY, Some(w), None, false)
                    .unwrap();
                for (ra, rb) in ta.rays.iter().zip(&tb.rays) {
                    assert_eq!(
                        (ra.status, ra.vig == 0),
                        (rb.status, rb.vig == 0),
                        "{}",
                        paths[k]
                    );
                    passed += usize::from(ra.vig == 0 && ra.status.is_ok());
                    let (pa, ka) = (local(&fa, ra.p, true), local(&fa, ra.k, false));
                    let (pb, kb) = (local(&fb, rb.p, true), local(&fb, rb.k, false));
                    for c in 0..3 {
                        assert!((pa[c] - pb[c]).abs() < 1e-9, "{} {pa:?} {pb:?}", paths[k]);
                        assert!((ka[c] - kb[c]).abs() < 1e-12, "{} {ka:?} {kb:?}", paths[k]);
                    }
                    assert!(
                        (ra.opl - rb.opl).abs() < 1e-9,
                        "{} {} {}",
                        paths[k],
                        ra.opl,
                        rb.opl
                    );
                }
            }
        }
        assert!(passed > 10, "{}: {passed}", paths[k]);
    }
}
