//! OPD の厚さのソルブ（OPDZ）の OpticStudio 26.1 照合（tests/opd_solve）。
//!
//! 3 枚玉（絞り 5、EPD 10、視野 0・14・20°、F・d・C 線）の面 2・4・6・7 の厚さに OPD のソルブ（目標・瞳の帯を変えて
//! 7 系）を付けた .zmx を OpticStudio で開き、解いた系の像面の光線（zos-cs rays、3 視野 × 3 波長 × 瞳 29 点）を
//! 取った。lensbench で同じ .zmx を読む（ソルブを解き直す）と、解いた厚さで OPD が目標になる。
//!
//! - 絞りより後ろの面（o6・o7a〜c）: 光線が OpticStudio と一致する。
//! - 絞りより前の面（o2・o4・o4z）: OPD は厚さに対して単調でなく根が複数ある。lensbench は今の厚さに近い根を、
//!   OpticStudio は遠い根を取り、その解は軸上の光線で 8e-6〜6.5e-4 mm ずれる（収束が粗い）うえ、軸外の視野の光線を
//!   すべて面 2 のエラーにする。lensbench を OpticStudio の根の近くから解くと、軸上の光線がその粗さの中で合う
//!   （同じ条件の別の根であることの確認）。OpticStudio の根の選び方・粗さには合わせない
//! - ほかのソルブ（像面の焦点の MAZH）と組んだ場合も OpticStudio に合わせない（両方を同時に満たす。決定）
use lensbench_core::system::{FieldSel, System};
use lensbench_core::{analysis, zmx};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// (系, ソルブの面, OPD [波], 瞳の帯, OpticStudio の根の近くの厚さ（絞りより前の面だけ）)
const CASES: [(&str, usize, f64, f64, Option<f64>); 7] = [
    ("o2", 2, 1.0, 0.7, Some(30.5)),
    ("o4", 4, 0.5, 0.7, Some(-48.0)),
    ("o4z", 4, 0.0, 0.3, Some(-50.0)),
    ("o6", 6, 0.3, 0.7, None),
    ("o7a", 7, 0.25, 0.7, None),
    ("o7b", 7, -0.5, 1.0, None),
    ("o7c", 7, 0.0, 0.5, None),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests")
}

fn opts() -> zmx::LoadOptions {
    zmx::LoadOptions {
        catalog_dirs: vec![root().join("glass")],
        ..Default::default()
    }
}

/// .zmx を、面 k の厚さ（ソルブの初めの値）を t に書き換えて読む
fn load(path: &Path, k: usize, t: Option<f64>) -> System {
    let txt = std::fs::read_to_string(path).unwrap();
    let mut out = String::new();
    let mut cur = usize::MAX;
    for l in txt.lines() {
        if let Some(r) = l.strip_prefix("SURF ") {
            cur = r.trim().parse().unwrap_or(usize::MAX);
        }
        match t {
            Some(t) if cur == k && l.trim_start().starts_with("DISZ") => {
                out += &format!("  DISZ {t}\n")
            }
            _ => {
                out += l;
                out.push('\n');
            }
        }
    }
    let tmp = std::env::temp_dir().join(format!(
        "lb_opd_os_{}_{k}_{}.zmx",
        std::process::id(),
        path.file_stem().unwrap().to_string_lossy()
    ));
    std::fs::write(&tmp, out).unwrap();
    let s = zmx::load_with(&tmp, &opts()).unwrap();
    let _ = std::fs::remove_file(&tmp);
    s
}

/// 視野・波長ごとの OpticStudio の光線（視野, 波長, 光線の列）
type Traces = Vec<(usize, usize, Vec<Vec<f64>>)>;

/// OpticStudio の光線の (瞳の px, py, 視野・波長ごとの光線)
fn os_rays(name: &str) -> (Vec<f64>, Vec<f64>, Traces) {
    let f = root().join(format!("opd_solve/{name}.json"));
    let d: Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
    let num = |v: &Value| v.as_f64().unwrap();
    let pupil = d["pupil"].as_array().unwrap();
    let px = pupil.iter().map(|p| num(&p[0])).collect();
    let py = pupil.iter().map(|p| num(&p[1])).collect();
    let traces = d["traces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let rays = t["rays"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_array().unwrap().iter().map(num).collect())
                .collect();
            (
                t["field"].as_u64().unwrap() as usize - 1,
                t["wave"].as_u64().unwrap() as usize - 1,
                rays,
            )
        })
        .collect();
    (px, py, traces)
}

fn opd_at(s: &System, zone: f64) -> f64 {
    analysis::opd(s, FieldSel::Norm(0.0, 0.0), &[0.0], &[zone], None, true)
        .unwrap()
        .opd[0]
}

#[test]
fn opd_solves_match_opticstudio() {
    let (mut wp, mut wd, mut wo, mut wf) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let (mut n, mut nf) = (0, 0);
    for (name, k, target, zone, os_root) in CASES {
        let path = root().join(format!("opd_solve/{name}.zmx"));
        let s = load(&path, k, None);
        assert!(s.solves.iter().any(|q| q.surf == k), "{name}");
        wo = wo.max((opd_at(&s, zone) - target).abs());
        let (px, py, traces) = os_rays(name);
        let Some(t0) = os_root else {
            for (f, w, rays) in &traces {
                let tr = s
                    .try_trace(FieldSel::Index(*f), &px, &py, Some(*w), None, false)
                    .unwrap();
                for (i, v) in rays.iter().enumerate() {
                    let l = &tr.rays[i];
                    assert_eq!(
                        (l.err != 0, l.vig != 0),
                        (v[0] != 0.0, v[1] != 0.0),
                        "{name} F{f} W{w} {i}"
                    );
                    if v[0] != 0.0 || v[1] != 0.0 {
                        continue;
                    }
                    for c in 0..3 {
                        wp = wp.max((l.xyz[c] - v[2 + c]).abs());
                        wd = wd.max((tr.parity * l.lmn[c] - v[5 + c]).abs());
                    }
                    n += 1;
                }
            }
            continue;
        };
        // 絞りより前の面: OpticStudio の根の近くから解く
        assert!(k < s.stop(), "{name}");
        let far = load(&path, k, Some(t0));
        assert!((far.surfaces[k].t - s.surfaces[k].t).abs() > 10.0, "{name}");
        wo = wo.max((opd_at(&far, zone) - target).abs());
        for (f, w, rays) in &traces {
            let tr = far
                .try_trace(FieldSel::Index(*f), &px, &py, Some(*w), None, false)
                .unwrap();
            for (i, v) in rays.iter().enumerate() {
                if *f != 0 {
                    // OpticStudio は軸外の光線をすべて面 2 のエラーにする
                    assert_eq!(v[0], 2.0, "{name} F{f} W{w} {i}");
                    continue;
                }
                assert!(v[0] == 0.0 && v[1] == 0.0 && tr.ok(i), "{name} W{w} {i}");
                for c in 0..3 {
                    wf = wf.max((tr.rays[i].xyz[c] - v[2 + c]).abs());
                }
                nf += 1;
            }
        }
    }
    println!(
        "絞りより後ろ {n} 本: 位置 {wp:.2e} mm、方向 {wd:.2e}。絞りより前（OpticStudio の根） {nf} 本: 位置 {wf:.2e} mm。\
         OPD の残り {wo:.2e} 波"
    );
    assert!(n > 0 && nf > 0);
    assert!(wp < 1e-9 && wd < 1e-9 && wo < 1e-9, "{wp} {wd} {wo}");
    assert!(wf < 1e-3, "{wf}");
}
