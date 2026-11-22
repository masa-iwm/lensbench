//! 公差の感度・逆感度・補償のしかたを OpticStudio の公差解析と比べる（tests/tolerance_os/A0: A0 に RSCE の評価関数と
//! 手で組んだ公差の表を入れた系と、OpticStudio 2026 R1.01 の結果 os.json）。
//!
//! 公差の表の写し方: TRAD → 面の曲率半径 `s.r`、TTHI s s → 厚さ `s.t`（後ろの面をずらす）、TEDX・TEDY・TETX・TETY → 玉の
//! 偏心・傾き（回転の中心は玉の前の頂点で OpticStudio の既定と同じ）、TSTX → 面の傾き `s.tx`、TIND → `s.nd`、
//! TABB → `s.vd`（どちらもモデルガラスの式で波長ごとの変化を決める。OpticStudio と同じ解釈）、COMP → 補償子 `8.t`。
//!
//! TABB だけは少し合わない: 式の差は同じでも、lensbench は波長ごとの変化を dn と disp の 2 つで表す（主波長で合わせ、
//! ほかの波長は最小二乗）ため。±0.5 では評価の差が公称の 1e-3 以下、±5 で 1e-2 以下。

use lensbench_core::System;
use lensbench_design::montecarlo::run;
use lensbench_design::tolerance::{out_opts, result_json, spec_from_json};
use lensbench_formats::zmx::{self, LoadOptions};
use serde_json::{json, Value};
use std::collections::HashMap;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn dir() -> std::path::PathBuf {
    root().join("tests/tolerance_os/A0")
}

fn load() -> System {
    zmx::load_with(
        &dir().join("system.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap()
}

fn os() -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir().join("os.json")).unwrap()).unwrap()
}

/// .zmx の評価関数（RSCE 3 行）を写した項
fn terms(s: &System) -> Vec<Value> {
    let mf = lensbench_formats::zmx::merit::merit_function(&dir().join("system.zmx")).unwrap();
    let z = lensbench_design::merit_zmx::terms_from_zmx(
        &mf,
        s.primary + 1,
        s.image(),
        &lensbench_design::merit_zmx::ZmxSys::of(s),
    );
    assert!(z.notes.is_empty(), "{:?}", z.notes);
    assert_eq!(z.terms.len(), 3);
    z.terms
}

/// 玉の範囲 (a, b) の名前
fn lens_name(s: &System, a: usize, b: usize) -> String {
    s.units
        .iter()
        .find(|u| u.auto && u.spans[0] == [a, b])
        .unwrap_or_else(|| panic!("no lens {a}-{b}"))
        .name
        .clone()
}

/// os.json の公差の行（type, ints, 範囲）を設定の tols の 1 つに
fn tol_item(s: &System, ty: &str, ints: &[usize], lo: f64, hi: f64) -> Value {
    let (a, b) = (ints[0], ints.get(1).copied().unwrap_or(0));
    let label = format!("{ty} {a} {b}");
    let (param, lo, hi) = match ty {
        "TRAD" => (format!("{a}.r"), lo, hi),
        "TTHI" => (format!("{a}.t"), lo, hi),
        "TSTX" => (format!("{a}.tx"), lo, hi),
        "TIND" => (format!("{a}.nd"), lo, hi),
        "TABB" => (format!("{a}.vd"), lo, hi),
        "TEDX" | "TEDY" | "TETX" | "TETY" => {
            let k = match ty {
                "TEDX" => "dx",
                "TEDY" => "dy",
                "TETX" => "tx",
                _ => "ty",
            };
            (format!("{}.{k}", lens_name(s, a, b)), lo, hi)
        }
        _ => panic!("{ty}"),
    };
    json!({"label": label, "param": param, "lo": lo, "hi": hi})
}

/// 実行 run の設定（逆感度・補償のしかた）
fn spec_for(s: &System, run: &Value) -> Value {
    let tols: Vec<Value> = run["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let ints: Vec<usize> = r["ints"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap() as usize)
                .collect();
            let b = r["before"].as_array().unwrap();
            tol_item(
                s,
                r["type"].as_str().unwrap(),
                &ints,
                b[0].as_f64().unwrap(),
                b[1].as_f64().unwrap(),
            )
        })
        .collect();
    let comp_mode = match run["comp"].as_str().unwrap() {
        "OptimizeAll_DLS" => "optimize",
        "ParaxialFocus" => "paraxial",
        "None" => "none",
        x => panic!("{x}"),
    };
    let mut v = json!({
        "preset": "none", "merit": "none", "terms": terms(s), "tols": tols,
        "compensators": ["8.t"], "comp_mode": comp_mode, "trials": 0,
        "comp_iters": 30,
    });
    match run["mode"].as_str().unwrap() {
        "InverseLimit" => v["inverse"] = json!({"limit": run["value"]}),
        "InverseIncrement" => v["inverse"] = json!({"increment": run["value"]}),
        _ => {}
    }
    v
}

/// 実行 name を lensbench で回し、（公称、RSS の見積り、公差の label → 感度の行）にする
fn lensbench_run(name: &str) -> (f64, Option<f64>, HashMap<String, Value>) {
    let s = load();
    let o = os();
    let run_v = &o["runs"][name];
    let v = spec_for(&s, run_v);
    let spec = spec_from_json(&v, vec![s], &HashMap::new()).unwrap();
    let r = run(&spec).unwrap();
    let out = result_json(&spec, &r, &out_opts(&json!({})).unwrap());
    let nominal = out["nominal"]["merit"].as_f64().unwrap();
    let m = out["sensitivity"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| (x["label"].as_str().unwrap().to_string(), x.clone()))
        .collect();
    (nominal, out["rss_estimate"].as_f64(), m)
}

/// 許す差
struct Tol {
    /// 評価の差 / 評価の変化（変化が公称の 1e-2 より小さい項目は公称の 1e-2 で割る）
    crit: f64,
    /// 締めた値の相対差
    value: f64,
    /// TABB の評価の差 / 公称
    glass_crit: f64,
    /// TABB の締めた値の相対差
    glass_value: f64,
    /// 補償子の値の差 [mm]
    comp: f64,
    /// TABB の補償子の値の差 [mm]
    glass_comp: f64,
}

const TOL: Tol = Tol {
    crit: 5e-4,
    value: 1e-5,
    glass_crit: 1e-3,
    glass_value: 1e-2,
    comp: 1e-4,
    glass_comp: 2e-3,
};

/// 公差を 10 倍にした系（TABB の Δν ±5 は OpticStudio の非線形の変化と差が出る）
const TOL_X: Tol = Tol {
    glass_crit: 1e-2,
    glass_value: 2e-2,
    glass_comp: 2e-2,
    ..TOL
};

/// 実行 name の公称・各公差の評価・締めた値・補償子を比べ、いちばん大きい差を返す（許す差を超えたら落とす）
fn compare(name: &str, tol: &Tol) {
    let o = os();
    let run_v = &o["runs"][name];
    let (nominal, rss, m) = lensbench_run(name);
    let n_os = run_v["nominal"].as_f64().unwrap();
    println!("{name}: nominal lensbench {nominal:.8} OS {n_os:.8}");
    assert!(
        (nominal - n_os).abs() <= 1e-7,
        "{name} nominal {nominal} {n_os}"
    );
    let mut by_ty: std::collections::BTreeMap<String, (f64, String, f64, String)> =
        Default::default();
    let mut worst_comp = 0.0f64;
    for r in run_v["rows"].as_array().unwrap() {
        let ints: Vec<u64> = r["ints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap())
            .collect();
        let ty = r["type"].as_str().unwrap();
        let label = format!("{ty} {} {}", ints[0], ints.get(1).copied().unwrap_or(0));
        let x = &m[&label];
        let glass = ty == "TABB";
        let sides = [("min", "lo", 0), ("max", "hi", 1)];
        for (side, k, ci) in sides {
            let os_v = r[side][0].as_f64().unwrap();
            let os_c = r[side][1].as_f64().unwrap();
            let lb_v = x[k].as_f64().unwrap();
            let lb_c = x[&format!("merit_{k}")].as_f64().unwrap();
            let dc = if glass {
                (lb_c - os_c).abs() / n_os
            } else {
                // OS は 8 桁で出すので 1e-8 の丸めを除く
                ((lb_c - os_c).abs() - 1e-8).max(0.0) / ((os_c - n_os).abs().max(1e-2 * n_os))
            };
            let dv = (lb_v - os_v).abs() / os_v.abs().max(1e-12);
            let vmsg = format!("{label} {side}: lensbench {lb_v:.8} OS {os_v:.8}");
            // 補償子（OS は 6 桁で出す。評価の底が平らなので、値は評価ほど合わない）
            if let Some(c) = r["comp"].as_array() {
                let os_k = c[ci].as_f64().unwrap();
                let lb_k = x[&format!("comp_{k}")][0].as_f64().unwrap();
                let ct = if glass { tol.glass_comp } else { tol.comp };
                assert!(
                    (lb_k - os_k).abs() <= ct,
                    "{name} {label} {side} comp: lensbench {lb_k:.6} OS {os_k:.6}"
                );
                if !glass {
                    worst_comp = worst_comp.max((lb_k - os_k).abs());
                }
            } else {
                assert!(x[&format!("comp_{k}")].is_null(), "{name} {label}: comp");
            }
            let (ct, vt) = if glass {
                (tol.glass_crit, tol.glass_value)
            } else {
                (tol.crit, tol.value)
            };
            assert!(
                dc <= ct,
                "{name} {label} {side}: criterion lensbench {lb_c:.8} OS {os_c:.8} ({dc:.3e})"
            );
            assert!(dv <= vt, "{name} {vmsg} ({dv:.3e})");
            let e = by_ty
                .entry(ty.to_string())
                .or_insert((0.0, String::new(), 0.0, String::new()));
            if dc > e.0 {
                e.0 = dc;
                e.1 = format!("{label} {side}: lensbench {lb_c:.8} OS {os_c:.8}");
            }
            if dv > e.2 {
                e.2 = dv;
                e.3 = vmsg;
            }
        }
    }
    // RSS の見積り: OpticStudio の Estimated change は √Σ (lo 側の変化² + hi 側の変化²)/2（出力と 8 桁で一致）
    let os_rss: f64 = run_v["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let a = r["min"][1].as_f64().unwrap() - n_os;
            let b = r["max"][1].as_f64().unwrap() - n_os;
            (a * a + b * b) / 2.0
        })
        .sum::<f64>()
        .sqrt();
    let lb_rss = rss.unwrap() - nominal;
    println!("  RSS change: lensbench {lb_rss:.8} OS {os_rss:.8}");
    assert!(
        (lb_rss - os_rss).abs() <= tol.glass_crit * os_rss,
        "{name} RSS {lb_rss} {os_rss}"
    );
    for (t, e) in &by_ty {
        println!(
            "  {t}: crit {:.3e} ({}) | value {:.3e} ({})",
            e.0, e.1, e.2, e.3
        );
    }
    println!("  comp: {worst_comp:.2e}");
}

/// 感度（補償子 DLS・補償しない）、逆感度（Limit・Increment、DLS・近軸の焦点）。x は公差を 10 倍にした系
#[test]
fn matches_opticstudio() {
    for n in [
        "R1", "R2", "R3", "R4", "R5", "R1x", "R2x", "R3x", "R4x", "R5x",
    ] {
        compare(n, if n.ends_with('x') { &TOL_X } else { &TOL });
    }
}
