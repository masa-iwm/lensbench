//! ゴーストの経路（2 回反射）の展開と、屈折面での反射の係数。

use lensbench_core::coating::{stack, Coatings, C64};
use lensbench_core::ghost::{self, ghost_system};
use lensbench_core::system::FieldSel;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use std::path::PathBuf;

fn load(name: &str) -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap()
}

/// 空気中の平行平板（n = 1.5、厚さ 5）。視野 0°・10°
fn plate(coat: &str) -> System {
    let t = format!(
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 4\nFTYP 0 0 2 1 0 0 0\n\
         XFLN 0 0\nYFLN 0 10\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\n\
         SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
         SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  COAT {coat}\n  DISZ 5\n  GLAS ___BLANK 1 0 1.5 0 0 0 0 0 0\n\
         SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  COAT {coat}\n  DISZ 20\n\
         SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    );
    let raw = zmx::parse_text(&t).unwrap();
    zmx::build(&raw, &Default::default(), &[], None).unwrap()
}

const X: [C64; 2] = [C64::new(1.0, 0.0), C64::new(0.0, 0.0)];
const Y: [C64; 2] = [C64::new(0.0, 0.0), C64::new(1.0, 0.0)];

/// 面の座標系（大域）が、写しの元の面と一致する
fn assert_frames_match(s: &System, tol: f64) {
    let f0 = s.frames();
    let refl = ghost::reflecting_surfaces(s);
    let pairs = ghost::pairs(s, 1, s.image());
    assert!(!pairs.is_empty());
    assert_eq!(pairs.len(), refl.len() * (refl.len() - 1) / 2);
    for (a, b) in pairs {
        let g = ghost_system(s, a, b).unwrap();
        let f1 = g.sys.frames();
        assert_eq!(g.origin[g.at.0], Some(a));
        assert_eq!(g.origin[g.at.1], Some(b));
        assert!(g.sys.surfaces[g.at.0].mirror && g.sys.surfaces[g.at.1].mirror);
        assert_eq!(g.sys.surfaces.iter().filter(|d| d.stop).count(), 1);
        for (k, o) in g.origin.iter().enumerate() {
            let Some(m) = *o else { continue };
            let (Some(p), Some(q)) = (f0.surf[m], f1.surf[k]) else {
                assert!(f0.surf[m].is_none() && f1.surf[k].is_none(), "{a}-{b} {k}");
                continue;
            };
            let mut d = 0.0f64;
            for i in 0..3 {
                d = d.max((p.o[i] - q.o[i]).abs());
                for j in 0..3 {
                    d = d.max((p.r[i][j] - q.r[i][j]).abs());
                }
            }
            assert!(
                d < tol,
                "{a}-{b}: 面 {k}（元 {m}）の座標系が {d:.1e} ずれる"
            );
        }
    }
}

#[test]
fn unfolded_surfaces_sit_where_the_originals_are() {
    // 共軸の系（厚さの符号を変えるだけ）
    assert_frames_match(&load("A0"), 1e-12);
    assert_frames_match(&load("F0"), 1e-12);
    // 鏡・傾き・偏心のある系（座標ブレークを挟む）
    assert_frames_match(&load("C0"), 1e-9);
}

#[test]
fn ghost_system_drops_units_of_the_original_numbering() {
    // 玉・群の範囲は元の面番号を指すので、展開した系には持ち込まない（持ち込むと違う面が動く）
    let s = load("A0");
    assert!(!s.units.is_empty(), "A0 は読込で玉が作られるはず");
    let g = ghost_system(&s, 3, 2).unwrap();
    assert!(g.sys.units.is_empty());
    assert!(g.sys.pickups.is_empty());
}

#[test]
fn coaxial_ghost_uses_negative_thicknesses() {
    let s = plate("");
    let g = ghost_system(&s, 2, 1).unwrap();
    let t: Vec<f64> = g.sys.surfaces.iter().map(|d| d.t).collect();
    // 0, 1, 2（反射）, 1（反射）, 2, 像面
    assert_eq!(
        g.origin,
        vec![Some(0), Some(1), Some(2), Some(1), Some(2), Some(3)]
    );
    assert_eq!(&t[1..], &[5.0, -5.0, 5.0, 20.0, 0.0]);
    assert_eq!(g.at, (2, 3));
    let p = ghost::paraxial(&g);
    // 平板のゴーストは平行光のまま
    assert!(p.marginal_u.abs() < 1e-15 && p.focus.is_infinite());
}

#[test]
fn plate_ghost_has_r_squared_intensity_and_the_extra_path() {
    let s = plate("");
    let g = ghost_system(&s, 2, 1).unwrap().sys;
    let c = Coatings::for_system(&s).unwrap();
    let n = 1.5;
    let r = (n - 1.0) / (n + 1.0);
    for hy in [0.0, 1.0] {
        let m = s
            .try_trace_pol(FieldSel::Norm(0.0, hy), &[0.0], &[0.0], None, &[X, Y], &c)
            .unwrap();
        let h = g
            .try_trace_pol(FieldSel::Norm(0.0, hy), &[0.0], &[0.0], None, &[X, Y], &c)
            .unwrap();
        let th0 = (10.0f64 * hy).to_radians();
        let th1 = (th0.sin() / n).asin();
        // 像面までの光路長の差は板の中の往復 2 n t / cosθ'（板の後ろの空気の光路は同じ）
        let (m0, h0) = (m.trace.rays[0], h.trace.rays[0]);
        let shift = 2.0 * 5.0 * th1.tan();
        let dl = 2.0 * n * 5.0 / th1.cos();
        assert!(
            ((h0.opl - m0.opl) - dl).abs() < 1e-12,
            "{hy}: {} {dl}",
            h0.opl - m0.opl
        );
        // 像面で光線は平行のまま、y で 2 t tanθ' ずれる
        assert!((h0.lmn[1] - m0.lmn[1]).abs() < 1e-15);
        assert!(
            ((h0.xyz[1] - m0.xyz[1]).abs() - shift).abs() < 1e-12,
            "{hy}"
        );
        // 強度の比は内側の 2 回の反射率の積（s・p で別）
        let rs_rp = if hy == 0.0 {
            (r * r, r * r)
        } else {
            let (c0, c1) = (th1.cos(), th0.cos());
            let rs = (n * c0 - c1) / (n * c0 + c1);
            let rp = (1.0 / c0 * n - 1.0 / c1) / (n / c0 + 1.0 / c1);
            (rs * rs, rp * rp)
        };
        let i = |e: [C64; 3]| e.iter().map(|z| z.norm_sqr()).sum::<f64>();
        // 入力 x は s（子午面が yz）、y は p
        let ix = i(h.e[0][0]) / i(m.e[0][0]);
        let iy = i(h.e[0][1]) / i(m.e[0][1]);
        assert!((ix - rs_rp.0 * rs_rp.0).abs() < 1e-15, "{hy}: {ix}");
        assert!((iy - rs_rp.1 * rs_rp.1).abs() < 1e-15, "{hy}: {iy}");
    }
}

#[test]
fn ideal_coating_reflects_one_minus_t() {
    let s = plate("I.99");
    let g = ghost_system(&s, 2, 1).unwrap().sys;
    let c = Coatings::for_system(&s).unwrap();
    let m = s
        .try_trace_pol(FieldSel::Norm(0.0, 1.0), &[0.3], &[0.2], None, &[X], &c)
        .unwrap();
    let h = g
        .try_trace_pol(FieldSel::Norm(0.0, 1.0), &[0.3], &[0.2], None, &[X], &c)
        .unwrap();
    let i = |e: [C64; 3]| e.iter().map(|z| z.norm_sqr()).sum::<f64>();
    let ratio = i(h.e[0][0]) / i(m.e[0][0]);
    assert!((ratio - 1e-4).abs() < 1e-16, "{ratio}");
}

#[test]
fn reflection_keys_and_coefficients() {
    use lensbench_core::coating::{media_keys, CoatingFile};
    use std::f64::consts::PI;
    let s = plate("AR2");
    let g = ghost_system(&s, 2, 1).unwrap();
    let (keys, _) = media_keys(&g.sys);
    let k: Vec<&str> = keys.iter().flatten().map(|k| k.as_str()).collect();
    assert_eq!(
        k,
        vec![
            "AR2|AIR|1.50,0.0",
            "AR2|1.50,0.0|AIR|R",
            "AR2|1.50,0.0|AIR|R",
            "AR2|1.50,0.0|AIR"
        ]
    );
    let txt = "MATE L\n0.4 1.38 0\n1.2 1.38 0\nMATE H\n0.4 2.1 0\n1.2 2.1 0\n\
               COAT AR2\nL 0.11 1\nH 0.06 1\n";
    let c = Coatings {
        file: CoatingFile::parse(txt).unwrap(),
        ..Default::default()
    };
    let wl = 0.55;
    let k = 2.0 * PI / wl;
    let layers = [(C64::new(1.38, 0.0), 0.11), (C64::new(2.1, 0.0), 0.06)];
    let rev = [layers[1], layers[0]];
    let one = C64::new(1.0, 0.0);
    // 空気から: 膜の側から入る。位相は鏡と同じ exp(2 i k n1 cosθ Σd)
    let f = c
        .prepare(&g.sys, "AR2|AIR|1.5,0.0|R", wl, 1.0, 1.5)
        .unwrap();
    // 硝材から: 基板の側から入る（層は逆）、位相なし
    let h = c
        .prepare(&g.sys, "AR2|1.5,0.0|AIR|R", wl, 1.5, 1.0)
        .unwrap();
    let ft = c.prepare(&g.sys, "AR2|AIR|1.5,0.0", wl, 1.0, 1.5).unwrap();
    let ht = c.prepare(&g.sys, "AR2|1.5,0.0|AIR", wl, 1.5, 1.0).unwrap();
    for deg in [0.0f64, 20.0, 40.0, 41.0, 60.0] {
        let th = deg.to_radians();
        let (rs, rp, _, _) = stack(1.0, &layers, C64::new(1.5, 0.0), wl, th);
        let e = C64::new(0.0, 2.0 * k * th.cos() * 0.17).exp();
        let v = f.eval(th).unwrap();
        assert!((v[0] - rs * e).norm() < 1e-14 && (v[1] - rp * e).norm() < 1e-14);
        // 損失のない膜: |r|² + |t|² = 1
        let t = ft.eval(th).unwrap();
        for p in 0..2 {
            assert!(
                (v[p].norm_sqr() + t[p].norm_sqr() - 1.0).abs() < 1e-13,
                "{deg}"
            );
        }
        let (rs, rp, _, _) = stack(1.5, &rev, one, wl, th);
        let v = h.eval(th).unwrap();
        assert!(
            (v[0] - rs).norm() < 1e-14 && (v[1] - rp).norm() < 1e-14,
            "{deg}"
        );
        if 1.5 * th.sin() < 1.0 {
            let t = ht.eval(th).unwrap();
            for p in 0..2 {
                assert!(
                    (v[p].norm_sqr() + t[p].norm_sqr() - 1.0).abs() < 1e-13,
                    "{deg}"
                );
            }
        } else {
            // 全反射
            for z in v {
                assert!((z.norm() - 1.0).abs() < 1e-13, "{deg}");
            }
        }
    }
    // 膜なし: フレネルの r（鏡の約束、p は反射の後で反転）
    let b = c.prepare(&g.sys, "-|AIR|1.5,0.0|R", wl, 1.0, 1.5).unwrap();
    let v = b.eval(0.0).unwrap();
    assert!((v[0].re + 0.2).abs() < 1e-15 && (v[1].re + 0.2).abs() < 1e-15);
    let v = b.eval_cos(0.5).unwrap();
    let (rs, rp, _, _) = stack(1.0, &[], C64::new(1.5, 0.0), wl, 0.5f64.acos());
    assert!((v[0] - rs).norm() < 1e-15 && (v[1] - rp).norm() < 1e-15);
}

#[test]
fn rejects_bad_pairs() {
    let s = plate("");
    assert!(ghost_system(&s, 1, 2).is_err());
    assert!(ghost_system(&s, 2, 2).is_err());
    assert!(ghost_system(&s, 4, 1).is_err());
}

#[test]
fn image_surface_reflects_first_only_when_listed_with_a_coating() {
    // 膜のない像面は一覧に入れない（反射しない）が、a に指定すれば経路は作れる（係数は 0）
    let s = plate("");
    assert!(!ghost::reflecting_surfaces(&s).contains(&3));
    let g = ghost_system(&s, 3, 1).unwrap().sys;
    let c = Coatings::for_system(&s).unwrap();
    let h = g
        .try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], None, &[X], &c)
        .unwrap();
    assert!(h.e[0][0].iter().all(|z| z.norm() == 0.0));
    // 像面に理想膜 I.96（反射率 4%）: 検出器の反射
    let mut s = plate("");
    s.surfaces[3].coat = "I.96".into();
    assert_eq!(ghost::reflecting_surfaces(&s), vec![1, 2, 3]);
    assert_eq!(ghost::pairs(&s, 1, 3), vec![(2, 1), (3, 1), (3, 2)]);
    let g = ghost_system(&s, 3, 2).unwrap().sys;
    let m = s
        .try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], None, &[X], &c)
        .unwrap();
    let h = g
        .try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], None, &[X], &c)
        .unwrap();
    let i = |e: [C64; 3]| e.iter().map(|z| z.norm_sqr()).sum::<f64>();
    // 像面で 4%、面 2 に空気側から戻って 4%（膜なし n = 1.5）、面 2 の透過はもう 1 回ぶん要らない
    // 経路: 1 透過 → 2 透過 → 像面 反射 → 2 反射（空気から）→ 像面: 比 = 0.04 · 0.04
    assert!((i(h.e[0][0]) / i(m.e[0][0]) - 0.04 * 0.04).abs() < 1e-15);
}

/// OpticStudio 2026 R1.01 の Analyze > Coatings > Reflection vs Angle（tests/ghost）: 膜 AR2（LBGHOST.DAT）を
/// 付けた n = 1.5 の平板の面 1（空気から）と面 2（硝材から、層は逆順、41.8° より先は全反射）の Rs・Rp・反射の位相。
/// OpticStudio の位相は膜の外側の面を基準にした r = (η0 B − C)/(η0 B + C) の偏角。lensbench は膜の側から入るとき
/// 基板の面（頂点）を基準にするので exp(2 i k n1 cosθ Σd) だけ違う（基板の側から入る面 2 は同じ）
#[test]
fn reflection_matches_opticstudio_coating_analysis() {
    use lensbench_core::coating::CoatingFile;
    use std::f64::consts::PI;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/ghost");
    let c = Coatings {
        file: CoatingFile::load(&root.join("LBGHOST.DAT")).unwrap(),
        ..Default::default()
    };
    let s = plate("AR2");
    let wl = 0.55;
    let k = 2.0 * PI / wl;
    let (mut wr, mut wp) = (0.0f64, 0.0f64);
    for (file, key, n1, n2, front) in [
        ("reflect_s1.tsv", "AR2|AIR|1.5,0.0|R", 1.0, 1.5, true),
        ("reflect_s2.tsv", "AR2|1.5,0.0|AIR|R", 1.5, 1.0, false),
    ] {
        let f = c.prepare(&s, key, wl, n1, n2).unwrap();
        let txt = std::fs::read_to_string(root.join(file)).unwrap();
        for line in txt.lines().skip(1) {
            let v: Vec<f64> = line.split('\t').map(|x| x.parse().unwrap()).collect();
            let th = v[0].to_radians();
            let mut r = f.eval(th).unwrap();
            if front {
                let e = C64::new(0.0, -2.0 * k * n1 * th.cos() * 0.17).exp();
                r = r.map(|z| z * e);
            }
            for p in 0..2 {
                wr = wr.max((r[p].norm_sqr() - v[1 + p]).abs());
                // 全反射の手前の 1 点などで 2 つの位相が ±180° で入れ替わる表記の差を除く
                let mut d = r[p].arg().to_degrees() - v[3 + p];
                d -= 360.0 * (d / 360.0).round();
                wp = wp.max(d.abs());
            }
        }
    }
    eprintln!("R {wr:.1e}, 位相 {wp:.1e} deg");
    // 表は R が 7 桁、位相が 0.0001° の丸め
    assert!(wr < 1e-7 && wp < 1e-4, "R {wr:.1e} 位相 {wp:.1e}");
}

/// OpticStudio 2026 R1.01 のゴーストのオペランド（tests/ghost/ghost_ops_*.json）: 2 回反射の経路の像面での実光線
/// GPRX・GPRY（主波長）と近軸の周辺光線の高さ GPSY（Hy = 0, Py = 1）。光線がけられた・追えないとき OpticStudio は
/// 0 を返す（GPRT = 0 の光線は比べない）。像面を先の反射面にした組も含む。
/// C0（面の傾き・偏心と鏡）は合わない組が多い: 先の反射面より後ろに絞りがある組では視野の光線が（近軸の入射瞳の位置が
/// 違い）、反射の間に傾いた面をはさむ組では軸上の光線も 0.5〜3 mm 違う。lensbench の展開した系は面の位置が元の系と
/// 一致する（`unfolded_surfaces_sit_where_the_originals_are`）。OpticStudio の保存するゴーストの系（GH004003）は
/// 2 回目の反射・透過で面 3 の傾き・偏心を落としていて、物理的な位置にない（合わせない、2026-11-08 決定）。C0 は合う組（軸上の光線）だけ確かめる
#[test]
fn ghost_rays_match_opticstudio_operands() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/ghost");
    for name in ["A0", "F0", "C0"] {
        let s = load(name);
        let txt = std::fs::read_to_string(root.join(format!("ghost_ops_{name}.json"))).unwrap();
        let d: serde_json::Value = serde_json::from_str(&txt).unwrap();
        let rays: Vec<Vec<f64>> = serde_json::from_value(d["rays_HxHyPxPy"].clone()).unwrap();
        let (mut worst, mut n, mut par) = (0.0f64, 0, 0.0f64);
        let mut off = Vec::new();
        for (key, v) in d["pairs"].as_object().unwrap() {
            let (a, b) = key.split_once('-').unwrap();
            let (a, b): (usize, usize) = (a.parse().unwrap(), b.parse().unwrap());
            let g = ghost_system(&s, a, b).unwrap();
            let col = |c: &str| -> Vec<f64> { serde_json::from_value(v[c].clone()).unwrap() };
            let (t, x, y, sy) = (col("GPRT"), col("GPRX"), col("GPRY"), col("GPSY"));
            par = par.max((ghost::paraxial(&g).marginal_y - sy[1]).abs());
            let (mut w, mut w_axis) = (0.0f64, 0.0f64);
            for (k, r) in rays.iter().enumerate() {
                if t[k] == 0.0 {
                    continue;
                }
                let tr = g
                    .sys
                    .try_trace(
                        FieldSel::Norm(r[0], r[1]),
                        &[r[2]],
                        &[r[3]],
                        None,
                        None,
                        false,
                    )
                    .unwrap();
                let o = tr.rays[0];
                let e = (o.xyz[0] - x[k]).abs().max((o.xyz[1] - y[k]).abs());
                w = w.max(e);
                if r[0] == 0.0 && r[1] == 0.0 {
                    w_axis = w_axis.max(e);
                }
                n += 1;
            }
            if name == "C0" {
                if w > 1e-9 {
                    off.push(key.clone());
                }
                // 傾いた面を 1 つだけはさむ組は軸上の光線が合う（6-4 は視野の光線も）
                if ["2-1", "3-2", "6-4"].contains(&key.as_str()) {
                    worst = worst.max(if key == "6-4" { w } else { w_axis });
                }
            } else {
                worst = worst.max(w);
            }
        }
        eprintln!("{name}: {n} 本 最大 {worst:.1e}、近軸 {par:.1e}、合わない組 {off:?}");
        assert!(worst < 1e-10, "{name}: {worst:.1e}");
        if name != "C0" {
            assert!(par < 1e-10, "{name}: 近軸 {par:.1e}");
        }
    }
}
