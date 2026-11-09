//! フィルタ・ダイクロイックの透過特性: プロジェクト・ユーザーカタログの `coatings` が TABLE 膜として
//! 偏光の追跡に効く

use lensbench_core::coating::{Coatings, C64};
use lensbench_core::system::FieldSel;
use lensbench_project::{expand_path, Env, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env(base: PathBuf) -> Env<'static> {
    Env {
        base,
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

/// 軸上の光線の強度透過率（x と y の直線偏光の平均）
fn transmission(s: &lensbench_core::System, w: usize) -> f64 {
    let coat = Coatings::for_system(s).unwrap();
    let one = C64::new(1.0, 0.0);
    let zero = C64::new(0.0, 0.0);
    let t = s
        .try_trace_pol_opts(
            FieldSel::Index(0),
            &[0.0],
            &[0.0],
            Some(w),
            &[[one, zero], [zero, one]],
            &coat,
            false,
        )
        .unwrap();
    let e = &t.e[0];
    (e[0]
        .iter()
        .chain(e[1].iter())
        .map(|c| c.norm_sqr())
        .sum::<f64>())
        / 2.0
}

fn project(coatings: &str, blocks: &str) -> Project {
    Project::from_json(&format!(
        r#"{{"wavelengths": [{{"um": 0.532}}, {{"um": 0.6}}, {{"um": 0.45}}],
        "aperture": {{"type": "epd", "value": 2}}, "fields": {{"points": [[0, 0]]}},
        "coatings": {coatings}, "blocks": {blocks}}}"#
    ))
    .unwrap()
}

/// 帯域フィルタの面: 帯域内（532 nm）は t、帯域外（600 nm）は 10^−od。裏面（膜なし）のフレネルは同じくらい
#[test]
fn bandpass_on_plate() {
    let p = project(
        r#"{"BP532": {"type": "bandpass", "center_nm": 532, "fwhm_nm": 10, "t": 0.9, "od": 3}}"#,
        r#"[{"type": "stop"}, {"type": "air", "t": 5},
           {"type": "lens", "source": {"surfaces": [{"t": 2, "glass": "N-BK7", "coat": "BP532"}, {}]}},
           {"type": "air", "t": 10}]"#,
    );
    let s = expand_path(&p, &env(root()), None).unwrap().0;
    assert!(s.coat_tables.contains_key("BP532"));
    let (tin, tout) = (transmission(&s, 0), transmission(&s, 1));
    // 裏面のフレネル（N-BK7 → 空気 ≈ 4 %）を除く
    let fres = |n: f64| 1.0 - ((n - 1.0) / (n + 1.0)).powi(2);
    let n = |w: usize| s.surfaces[2].n[w];
    assert!((tin / fres(n(0)) - 0.9).abs() < 1e-6, "{tin}");
    assert!((tout / fres(n(1)) - 1e-3).abs() < 1e-8, "{tout}");
    // 仕様の誤り（半値全幅が中心波長以上）は展開で誤り
    let p = project(
        r#"{"BP": {"type": "bandpass", "center_nm": 532, "fwhm_nm": 600}}"#,
        "[]",
    );
    assert!(expand_path(&p, &env(root()), None).is_err());
}

/// ユーザーカタログの coatings: 品の面の coat から使え、プロジェクトの同じ名前が優先
#[test]
fn catalog_coatings() {
    let dir = std::env::temp_dir().join(format!("lbfilt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("filters.json"),
        r#"{"version": 1, "coatings": {"ND1": {"type": "nd", "od": 1, "absorptive": true}},
          "items": {"NE10A": {"kind": "filter", "blocks": [{"type": "lens", "source": {"surfaces": [
            {"t": 1, "glass": "N-BK7", "coat": "ND1"}, {"semi": 5}]}}]}}}"#,
    )
    .unwrap();
    let blocks = r#"[{"type": "stop"}, {"type": "air", "t": 5},
        {"type": "user", "catalog": "filters.json", "item": "NE10A"}, {"type": "air", "t": 10}]"#;
    let s = expand_path(&project("{}", blocks), &env(dir.clone()), None)
        .unwrap()
        .0;
    let fres = |n: f64| 1.0 - ((n - 1.0) / (n + 1.0)).powi(2);
    let t = transmission(&s, 0);
    assert!((t / fres(s.surfaces[2].n[0]) - 0.1).abs() < 1e-9, "{t}");
    let s = expand_path(
        &project(
            r#"{"ND1": {"type": "nd", "od": 2, "absorptive": true}}"#,
            blocks,
        ),
        &env(dir.clone()),
        None,
    )
    .unwrap()
    .0;
    let t = transmission(&s, 0);
    assert!((t / fres(s.surfaces[2].n[0]) - 0.01).abs() < 1e-9, "{t}");
    std::fs::remove_dir_all(&dir).ok();
}

/// ダイクロイック BS（ロングパス 500 nm、45°）: 透過の経路は長い波長、反射の経路は短い波長が通る
#[test]
fn dichroic_splits_wavelengths() {
    let p = project(
        r#"{"DMLP500": {"type": "longpass", "edge_nm": 500, "width_nm": 10, "t": 0.95, "od": 3}}"#,
        r#"[{"type": "stop"}, {"type": "air", "t": 10},
           {"type": "splitter", "id": "BS", "form": "ideal", "coat": "DMLP500", "angle": [45, 0]},
           {"type": "air", "t": 20}]"#,
    );
    let t = expand_path(&p, &env(root()), Some("T")).unwrap().0;
    let r = expand_path(&p, &env(root()), Some("R")).unwrap().0;
    // 532 nm・600 nm は透過、450 nm は反射（45° でエッジは 500·√(1 − (sin45°/2)²) ≈ 468 nm へ）
    for w in [0, 1] {
        assert!(transmission(&t, w) > 0.9, "{w} {}", transmission(&t, w));
        assert!(transmission(&r, w) < 0.1, "{w} {}", transmission(&r, w));
    }
    assert!(transmission(&t, 2) < 0.01, "{}", transmission(&t, 2));
    assert!(transmission(&r, 2) > 0.95, "{}", transmission(&r, 2));
}

/// .zmx の書き出し: 系に書いた膜の表は隣の膜ファイル（名前_coat.dat）に書き、COFN はそれを指す。読み戻すと同じ透過
#[test]
fn zmx_export_keeps_coat_tables() {
    let p = project(
        r#"{"BP532": {"type": "bandpass", "center_nm": 532, "fwhm_nm": 10, "t": 0.9, "od": 3}}"#,
        r#"[{"type": "stop"}, {"type": "air", "t": 5},
           {"type": "lens", "source": {"surfaces": [{"t": 2, "glass": "N-BK7", "coat": "BP532"}, {}]}},
           {"type": "air", "t": 10}]"#,
    );
    let s = expand_path(&p, &env(root()), None).unwrap().0;
    let dir = std::env::temp_dir().join(format!("lb_coat_export_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let zp = dir.join("bp.zmx");
    lensbench_core::zmx_write::save(&zp, &s).unwrap();
    let dat = std::fs::read_to_string(dir.join("bp_coat.dat")).unwrap();
    assert!(dat.contains("TABLE BP532"));
    // OpticStudio は全部の角度で同じ波長の並びでないと表を 0 として読む（理想フィルタの表は角度ごとにエッジの
    // 近くの波長が違うので、書き出しで並べ直す）
    let mut per_angle: Vec<Vec<&str>> = Vec::new();
    for line in dat.lines() {
        if line.starts_with("ANGL") {
            per_angle.push(Vec::new());
        } else if let Some(w) = line.strip_prefix("WAVE ") {
            per_angle
                .last_mut()
                .unwrap()
                .push(w.split(' ').next().unwrap());
        }
    }
    assert!(per_angle.len() > 2);
    assert!(
        per_angle.iter().all(|v| *v == per_angle[0]),
        "角度ごとに波長が違う"
    );
    let txt = lensbench_core::glass::read_text(&zp).unwrap();
    assert!(txt.contains("COFN QF \"bp_coat.dat\""), "{txt}");
    let r = lensbench_core::zmx::load_with(
        &zp,
        &lensbench_core::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        r.coating_file.as_deref(),
        Some(dir.join("bp_coat.dat").as_path())
    );
    for w in 0..3 {
        let (a, b) = (transmission(&s, w), transmission(&r, w));
        assert!((a - b).abs() < 1e-12, "波長 {w}: {a} {b}");
    }
    // 表を使わない系は膜ファイルを書かない
    assert!(lensbench_core::zmx_write::coating_dat(&r)
        .unwrap()
        .is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// OpticStudio 26.1 との照合（tests/filter_os/filter_refs.json、zos-cs pol で作成）: tests/filter_os の 4 系
/// （ロングパス 45° の板・バンドパス + ND・ショートパス + ノッチ・curve + table）を lensbench が書き出した .zmx と
/// 膜ファイル（理想フィルタの表を含む）を OpticStudio で偏光追跡した像面の電場（入力 x・y、8 波長・3 視野・間引いた
/// 瞳の点）。OpticStudio の N-BK7 には内部透過率（ガラスの吸収、厚さ 7 mm ほどで 0.1〜0.2 %）があり、試験のガラス
/// （tests/glass）にはないので、強度は (系, 視野, 波長) ごとの比の中央値で割ってから比べ、中央値そのものは吸収の
/// 大きさ（0.3 % 以内）で見る。比のばらつき（光線ごとのガラスの中の長さの違い）は 5e-4 以内。電場は向き（大きさで割った複素の 3 成分）を比べる
#[test]
fn filters_match_opticstudio() {
    let dir = root().join("tests/filter_os");
    let refs: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("filter_refs.json")).unwrap())
            .unwrap();
    let f = |v: &serde_json::Value| v.as_f64().unwrap();
    let (mut wdir, mut wspread, mut wmed, mut n) = (0.0f64, 0.0f64, 0.0f64, 0usize);
    for (name, fields) in refs["cases"].as_object().unwrap() {
        let txt = std::fs::read_to_string(dir.join(format!("{name}.json"))).unwrap();
        let p = Project::from_json(&txt).unwrap();
        let path = (name == "lp45_T").then_some("T");
        let s = expand_path(&p, &env(dir.clone()), path).unwrap().0;
        let coat = Coatings::for_system(&s).unwrap();
        for fld in fields.as_array().unwrap() {
            // (波長) ごとの強度の比
            let mut ratios: Vec<Vec<f64>> = vec![Vec::new(); s.wavelengths.len()];
            for r in fld["rays"].as_array().unwrap() {
                assert_eq!(r["err"].as_u64(), Some(0), "{name} {r}");
                let w = r["wave"].as_u64().unwrap() as usize - 1;
                let inp = if r["in"].as_u64().unwrap() == 0 {
                    [C64::new(1.0, 0.0), C64::new(0.0, 0.0)]
                } else {
                    [C64::new(0.0, 0.0), C64::new(1.0, 0.0)]
                };
                let t = s
                    .try_trace_pol(
                        FieldSel::Norm(f(&fld["hx"]), f(&fld["hy"])),
                        &[f(&r["px"])],
                        &[f(&r["py"])],
                        Some(w),
                        &[inp],
                        &coat,
                    )
                    .unwrap();
                let e = t.e[0][0];
                let z: Vec<C64> = (0..3)
                    .map(|k| C64::new(f(&r["E"][k][0]), f(&r["E"][k][1])))
                    .collect();
                let (ie, iz): (f64, f64) = (
                    e.iter().map(|c| c.norm_sqr()).sum(),
                    z.iter().map(|c| c.norm_sqr()).sum(),
                );
                assert!(
                    (iz - f(&r["I"])).abs() <= 1e-9 * iz.max(1e-12),
                    "{name} {r}"
                );
                if iz < 1e-6 {
                    // 阻止域（OD 3〜4 の 2 面）は強度だけ絶対値で
                    assert!((ie - iz).abs() < 1e-8, "{name} 波長 {w}: {ie:e} {iz:e}");
                    continue;
                }
                ratios[w].push(ie / iz);
                for k in 0..3 {
                    wdir = wdir.max((e[k] / ie.sqrt() - z[k] / iz.sqrt()).norm());
                }
                n += 1;
            }
            for (w, v) in ratios.iter_mut().enumerate() {
                if v.is_empty() {
                    continue;
                }
                v.sort_by(f64::total_cmp);
                let med = v[v.len() / 2];
                wmed = wmed.max((med - 1.0).abs());
                for x in v.iter() {
                    let d = (x / med - 1.0).abs();
                    assert!(
                        d < 1e-3,
                        "{name} 視野 ({}, {}) 波長 {w}: 比 {x} 中央値 {med}",
                        fld["hx"],
                        fld["hy"]
                    );
                    wspread = wspread.max(d);
                }
            }
        }
    }
    eprintln!("{n} 本: 電場の向き {wdir:.1e}、強度の比の中央値からの差 {wspread:.1e}、中央値 − 1 {wmed:.1e}");
    assert!(n > 1000, "{n}");
    assert!(
        wdir < 1e-4 && wspread < 5e-4 && wmed < 3e-3,
        "{wdir:.1e} {wspread:.1e} {wmed:.1e}"
    );
}
