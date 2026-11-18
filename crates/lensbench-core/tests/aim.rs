//! 実光線エイミング: 本数が多いとき（格子の光線から初期値を当てはめ、ヤコビアンを固定して反復する）と
//! 少ないとき（光線ごとのニュートン法）で、絞りでの位置が同じ許容差で目標に合い、互いに一致する。

use std::path::PathBuf;

use lensbench_core::system::Aiming;
use lensbench_core::FieldSel;
use lensbench_formats::zmx::{self, LoadOptions};

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
        s.edit_raw(|p| {
            p.aiming = Aiming::Real;
            Ok(())
        })
        .unwrap();
        let st = s.stop();
        let (rx, ry) = s.stop_radius();
        let r = rx.abs().max(ry.abs()).max(1.0);
        for fi in 0..s.fields.len() {
            let f = FieldSel::Index(fi);
            let all = s.try_trace(f, &px, &py, None, Some(st), false).unwrap();
            let mut ok = 0;
            for c in (0..n).step_by(250) {
                let part = s
                    .try_trace(f, &px[c..c + 250], &py[c..c + 250], None, Some(st), false)
                    .unwrap();
                for (k, b) in part.rays.iter().enumerate() {
                    let a = &all.rays[c + k];
                    assert_eq!(
                        (a.status, a.vig),
                        (b.status, b.vig),
                        "{name} 視野 {fi} 光線 {}",
                        c + k
                    );
                    if a.status.surf().is_some() || a.vig != 0 {
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

/// 絞りの半径が 0.005 mm ほどの小さい系（A0 を 1/1000 に縮めた）でも、縁に狙った光線（瞳の単位円の上）が
/// 狙いの許容と開口の判定の余裕のずれでけられない（どちらも絞りの半径に対する比）
#[test]
fn rim_rays_pass_in_a_tiny_system() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let b = std::fs::read(root.join("tests/refs/A0/system.zmx")).unwrap();
    let u: Vec<u16> = b[2..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let t = String::from_utf16(&u).unwrap();
    let k = 1e-3;
    let num = |w: &str| -> String {
        match w.parse::<f64>() {
            Ok(v) => format!("{:e}", v),
            Err(_) => w.to_string(),
        }
    };
    let scaled: Vec<String> = t
        .split("\r\n")
        .map(|l| {
            let w: Vec<&str> = l.split(' ').collect();
            let key = l.trim_start();
            let f = if key.starts_with("CURV ") {
                1.0 / k
            } else if key.starts_with("DISZ ")
                || key.starts_with("DIAM ")
                || key.starts_with("MEMA ")
                || key.starts_with("ENPD ")
            {
                k
            } else {
                return l.to_string();
            };
            // 最初の数（キーワードの次）だけを縮める（INFINITY はそのまま）
            let mut w: Vec<String> = w.iter().map(|x| x.to_string()).collect();
            let i = w.iter().position(|x| !x.is_empty()).unwrap() + 1;
            if let Ok(v) = w[i].parse::<f64>() {
                w[i] = num(&(v * f).to_string());
            }
            w.join(" ")
        })
        .collect();
    let dir = std::env::temp_dir().join(format!("lb-aim-tiny-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("a.zmx");
    let mut out = vec![0xff, 0xfe];
    out.extend(
        scaled
            .join("\r\n")
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    std::fs::write(&f, out).unwrap();
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let mut s = zmx::load_with(&f, &opt).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    s.edit_raw(|p| {
        p.aiming = Aiming::Real;
        Ok(())
    })
    .unwrap();
    let (rx, ry) = s.stop_radius();
    assert!(rx.abs().max(ry.abs()) < 0.01, "{rx} {ry}");
    let n = 64;
    let px: Vec<f64> = (0..n)
        .map(|i| (i as f64 / n as f64 * std::f64::consts::TAU).cos())
        .collect();
    let py: Vec<f64> = (0..n)
        .map(|i| (i as f64 / n as f64 * std::f64::consts::TAU).sin())
        .collect();
    for fi in 0..s.fields.len() {
        let tr = s
            .try_trace(FieldSel::Index(fi), &px, &py, None, None, false)
            .unwrap();
        for (i, r) in tr.rays.iter().enumerate() {
            assert!(
                r.status.surf().is_none() && r.vig == 0,
                "視野 {fi} 光線 {i}: {:?} {}",
                r.status,
                r.vig
            );
        }
    }
}

/// 入射瞳の径だけを 1e-3〜1e-6 倍にした系（絞りがごく小さい）でも、狙いが丸めで止まらずに届く（許容の大きさに
/// 光線がたどる長さに対する丸めの下限がある）
#[test]
fn aiming_converges_with_a_tiny_stop() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["A1", "E1"] {
        let b = std::fs::read(root.join(format!("tests/refs/{name}/system.zmx"))).unwrap();
        let u: Vec<u16> = b[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let t = String::from_utf16(&u).unwrap();
        let at = t.find("\nENPD ").unwrap() + 1;
        let end = at + t[at..].find('\r').unwrap();
        let d: f64 = t[at + 5..end].trim().parse().unwrap();
        for k in [1e-3, 1e-6] {
            let t = format!("{}ENPD {:e}{}", &t[..at], d * k, &t[end..]);
            let dir =
                std::env::temp_dir().join(format!("lb-aim-stop-{name}-{k}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let f = dir.join("a.zmx");
            let mut out = vec![0xff, 0xfe];
            out.extend(t.encode_utf16().flat_map(u16::to_le_bytes));
            std::fs::write(&f, out).unwrap();
            let opt = LoadOptions {
                cfg: 1,
                catalog_dirs: vec![root.join("tests/glass")],
                ..Default::default()
            };
            let mut s = zmx::load_with(&f, &opt).unwrap();
            std::fs::remove_dir_all(&dir).ok();
            s.edit_raw(|p| {
                p.aiming = Aiming::Real;
                Ok(())
            })
            .unwrap();
            // 少ない光線（光線ごとのニュートン法）と多い光線（当てはめから反復する）の両方
            let n = 600;
            let mut px: Vec<f64> = (0..n)
                .map(|i| ((i * 7919) % 1000) as f64 / 1000.0 * 2.0 - 1.0)
                .collect();
            let mut py: Vec<f64> = (0..n)
                .map(|i| ((i * 104729) % 1000) as f64 / 1000.0 * 2.0 - 1.0)
                .collect();
            for (x, y) in px.iter_mut().zip(py.iter_mut()) {
                let r = x.hypot(*y);
                if r > 1.0 {
                    (*x, *y) = (*x / r, *y / r);
                }
            }
            let few = (
                vec![0.0, 1.0, -1.0, 0.0, 0.0, 0.5, 0.7],
                vec![0.0, 0.0, 0.0, 1.0, -1.0, 0.5, -0.7],
            );
            for (px, py) in [(&few.0, &few.1), (&px, &py)] {
                for fi in 0..s.fields.len() {
                    let tr = s
                        .try_trace(FieldSel::Index(fi), px, py, None, None, false)
                        .unwrap_or_else(|e| panic!("{name} ×{k} 視野 {fi}: {e}"));
                    for (i, r) in tr.rays.iter().enumerate() {
                        assert!(
                            r.status.surf().is_none(),
                            "{name} ×{k} 視野 {fi} 光線 {i}: {:?}",
                            r.status
                        );
                    }
                }
            }
        }
    }
}
