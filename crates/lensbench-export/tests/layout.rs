//! レイアウトの書き出しの確かめ: 場面の中身、各形式の構造、立体が閉じていて体積が正しいこと。

use std::collections::HashMap;

use lensbench_core::math::*;
use lensbench_core::System;
use lensbench_export::d2::{self, Options2D};
use lensbench_export::mesh::{self, Options3D};
use lensbench_export::scene::{self, Options, SolidKind};
use lensbench_export::step;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn zmx(name: &str) -> System {
    let opt = lensbench_formats::zmx::LoadOptions {
        catalog_dirs: vec![format!("{ROOT}/tests/glass").into()],
        ..Default::default()
    };
    lensbench_formats::zmx::load_with(
        format!("{ROOT}/tests/refs/{name}/system.zmx").as_ref(),
        &opt,
    )
    .unwrap()
}

fn project(name: &str) -> System {
    project_with(name, |t| t)
}

/// 見本のプロジェクトの文を `edit` で書き換えてから組み立てる（相対パスは見本のフォルダから引く）
fn project_with(name: &str, edit: impl FnOnce(String) -> String) -> System {
    let path = format!("{ROOT}/examples/projects/{name}.json");
    let txt = edit(std::fs::read_to_string(&path).unwrap());
    lensbench_project::Project::from_json(&txt)
        .and_then(|q| {
            lensbench_project::expand(
                &q,
                &lensbench_project::Env::for_file(
                    path.as_ref(),
                    &[format!("{ROOT}/tests/glass").into()],
                    None,
                ),
                &lensbench_project::ExpandOpts::default(),
            )
            .map_err(Into::into)
        })
        .map(|x| x.system)
        .unwrap()
}

#[test]
fn triplet_scene() {
    let s = zmx("A0");
    let sc = scene::build(&s, &Options::default()).unwrap();
    assert_eq!(sc.solids.len(), 3);
    assert!(sc
        .solids
        .iter()
        .all(|x| x.kind == SolidKind::Glass && x.coaxial()));
    assert_eq!(sc.plates.len(), 1);
    assert_eq!(sc.rays.len(), s.fields.len() * 7);
    // 光線は像面まで届き、最後の点は像面の上
    let img = s.frames().surf[s.image()].unwrap();
    for r in &sc.rays {
        let p = *r.points.last().unwrap();
        let z = dot(sub(p, img.o), col(&img.r, 2));
        assert!(z.abs() < 1e-9, "{z}");
    }
    // 波長をすべて
    let o = Options {
        waves: Some((0..s.wavelengths.len()).collect()),
        fields: Some(vec![0]),
        ..Default::default()
    };
    let sc = scene::build(&s, &o).unwrap();
    assert_eq!(sc.rays.len(), s.wavelengths.len() * 7);
}

#[test]
fn double_pass_lens_is_drawn_once() {
    let s = project("P4_double_pass");
    let sc = scene::build(&s, &Options::default()).unwrap();
    let k: Vec<SolidKind> = sc.solids.iter().map(|x| x.kind).collect();
    assert_eq!(k, vec![SolidKind::Glass, SolidKind::Mirror]);
}

/// 網の三角形を位置で結び、各辺がちょうど 2 つの三角形に向きを逆にして使われているかと、体積
fn closed_volume(m: &mesh::Mesh) -> f64 {
    let key = |p: V3| p.map(|v| v.to_bits());
    let mut id: HashMap<[u64; 3], usize> = HashMap::new();
    let idx: Vec<usize> = m
        .pos
        .iter()
        .map(|&p| {
            let n = id.len();
            *id.entry(key(p)).or_insert(n)
        })
        .collect();
    let mut edges: HashMap<(usize, usize), i32> = HashMap::new();
    let mut vol = 0.0;
    for t in &m.tris {
        let v = t.map(|i| idx[i as usize]);
        for k in 0..3 {
            let (a, b) = (v[k], v[(k + 1) % 3]);
            *edges.entry((a.min(b), a.max(b))).or_insert(0) += if a < b { 1 } else { -1 };
        }
        let [a, b, c] = t.map(|i| m.pos[i as usize]);
        vol += dot(a, cross(b, c)) / 6.0;
    }
    assert!(
        edges.values().all(|&c| c == 0),
        "{}: 閉じていないか向きがそろわない",
        m.name
    );
    vol
}

fn sag(r: f64, c: f64, k: f64, a: &[f64]) -> f64 {
    let mut z = c * r * r / (1.0 + (1.0 - (1.0 + k) * c * c * r * r).sqrt());
    for (i, ai) in a.iter().enumerate() {
        z += ai * r.powi(2 * (i as i32 + 1));
    }
    z
}

/// 非球面の玉（E0 の面 1–2）の体積を、断面の積分で
fn e0_volume(semi1: f64, semi2: f64, rim: f64) -> f64 {
    let n = 200_000;
    let mut v = 0.0;
    for i in 0..n {
        let r = (i as f64 + 0.5) / n as f64 * rim;
        let zf = sag(r.min(semi1), 1.0 / 30.0, -0.6, &[0.0, -1e-6, 3e-9, -2e-12]);
        let zb = 8.0 + sag(r.min(semi2), -1.0 / 120.0, 0.0, &[0.0, 4e-6]);
        v += 2.0 * std::f64::consts::PI * r * (zb - zf) * rim / n as f64;
    }
    v
}

#[test]
fn meshes_are_closed_with_the_right_volume() {
    let s = zmx("E0");
    let sc = scene::build(&s, &Options::default()).unwrap();
    let sd = &sc.solids[0];
    let exact = e0_volume(sd.front.semi, sd.back.semi, sd.rim);
    let o = Options3D {
        segments: 256,
        rings: 64,
        ..Default::default()
    };
    let ms = mesh::meshes(&sc, &o, None);
    let v = closed_volume(&ms[0]);
    // 円を内接多角形にした分だけ小さい
    assert!(v > 0.0 && (v / exact - 1.0).abs() < 3e-4, "{v} {exact}");
    // 傾き偏心のある系・折り返しの鏡も閉じている
    for sys in [zmx("C0"), project("P3_fold"), project("P4_double_pass")] {
        let sc = scene::build(&sys, &Options::default()).unwrap();
        for m in mesh::meshes(&sc, &Options3D::default(), None) {
            if matches!(m.kind, mesh::MeshKind::Glass | mesh::MeshKind::Mirror) {
                assert!(closed_volume(&m) > 0.0, "{}", m.name);
            }
        }
    }
}

/// チップゾーンの分だけ面の形が外へ延び、固定した機械半径が外径になる
#[test]
fn chip_zone_and_mech_semi_shape_the_solid() {
    let mut s = zmx("E0");
    let base = scene::build(&s, &Options::default()).unwrap().solids[0].clone();
    s.edit(|p| {
        p.surfaces[1].chip_zone = 0.5;
        p.surfaces[2].mech_semi = Some(base.rim + 2.0);
        Ok(())
    })
    .unwrap();
    let sc = scene::build(&s, &Options::default()).unwrap();
    let sd = &sc.solids[0];
    assert_eq!(sd.front.semi, base.front.semi + 0.5);
    assert_eq!(sd.back.semi, base.back.semi);
    assert_eq!(sd.rim, base.rim + 2.0);
    let o = Options3D {
        segments: 256,
        rings: 64,
        ..Default::default()
    };
    let v = closed_volume(&mesh::meshes(&sc, &o, None)[0]);
    let exact = e0_volume(sd.front.semi, sd.back.semi, sd.rim);
    assert!((v / exact - 1.0).abs() < 3e-4, "{v} {exact}");
}

#[test]
fn files_have_the_expected_structure() {
    let s = zmx("A0");
    let sc = scene::build(&s, &Options::default()).unwrap();
    let o2 = Options2D {
        width: 800,
        ..Default::default()
    };
    let o3 = Options3D::default();
    // SVG: 立体 3 つの多角形と光線の折れ線
    let mut b = Vec::new();
    d2::write_svg(&mut b, &sc, &o2).unwrap();
    let t = String::from_utf8(b).unwrap();
    assert!(t.starts_with("<?xml") && t.trim_end().ends_with("</svg>"));
    assert_eq!(t.matches("<polygon").count(), 3);
    assert_eq!(t.matches("<polyline").count(), 1 + sc.rays.len());
    // PNG: 指定の幅で読める
    let mut b = Vec::new();
    d2::write_png(&mut b, &sc, &o2).unwrap();
    let dec = png::Decoder::new(std::io::Cursor::new(b));
    let r = dec.read_info().unwrap();
    assert_eq!(r.info().width, 800);
    // DXF: 図形ごとに POLYLINE
    let mut b = Vec::new();
    d2::write_dxf(&mut b, &sc, &o2).unwrap();
    let t = String::from_utf8(b).unwrap();
    assert_eq!(t.matches("\nPOLYLINE\n").count(), 3 + 1 + sc.rays.len());
    assert!(t.trim_end().ends_with("EOF"));
    // STL: 50 バイト × 三角形
    let mut b = Vec::new();
    mesh::write_stl(&mut b, &sc, &o3).unwrap();
    let n = u32::from_le_bytes(b[80..84].try_into().unwrap()) as usize;
    assert_eq!(b.len(), 84 + 50 * n);
    // glb: ヘッダの長さと JSON
    let mut b = Vec::new();
    mesh::write_gltf(&mut b, &sc, &o3, true).unwrap();
    assert_eq!(&b[0..4], b"glTF");
    assert_eq!(
        u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize,
        b.len()
    );
    let jl = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
    let j: serde_json::Value = serde_json::from_slice(&b[20..20 + jl]).unwrap();
    // 立体 3・像面 1・光線（視野ごと）
    assert_eq!(
        j["meshes"].as_array().unwrap().len(),
        3 + 1 + s.fields.len()
    );
    // gltf: バッファを埋め込む
    let mut b = Vec::new();
    mesh::write_gltf(&mut b, &sc, &o3, false).unwrap();
    let j: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert!(j["buffers"][0]["uri"]
        .as_str()
        .unwrap()
        .starts_with("data:application/octet-stream;base64,"));
    // OBJ: 光線は線
    let mut b = Vec::new();
    mesh::write_obj(&mut b, &sc, &o3).unwrap();
    let t = String::from_utf8(b).unwrap();
    assert_eq!(
        t.lines().filter(|l| l.starts_with("o ")).count(),
        3 + 1 + s.fields.len()
    );
    assert!(t.lines().any(|l| l.starts_with("l ")));
}

/// STEP の DATA の行がすべて `#n=...;` で、参照する番号がすべてあるか
fn check_step(t: &str) -> HashMap<String, usize> {
    assert!(t.starts_with("ISO-10303-21;") && t.trim_end().ends_with("END-ISO-10303-21;"));
    let data = &t[t.find("DATA;").unwrap() + 5..t.rfind("ENDSEC;").unwrap()];
    let mut ids = std::collections::HashSet::new();
    let mut kinds: HashMap<String, usize> = HashMap::new();
    for l in data.lines().filter(|l| !l.trim().is_empty()) {
        assert!(l.starts_with('#') && l.ends_with(';'), "{l}");
        let (id, body) = l.split_once('=').unwrap();
        ids.insert(id[1..].parse::<usize>().unwrap());
        let k: String = body.chars().take_while(|c| *c != '(').collect();
        *kinds.entry(k).or_insert(0) += 1;
    }
    for l in data.lines() {
        let body = l.split_once('=').map(|x| x.1).unwrap_or("");
        // 文字列（'…'）の中の # は参照でない
        let mut quoted = false;
        for (i, c) in body.char_indices() {
            if c == '\'' {
                quoted = !quoted;
            }
            if c == '#' && !quoted {
                let n: String = body[i + 1..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                assert!(
                    ids.contains(&n.parse::<usize>().unwrap()),
                    "#{n} がない: {l}"
                );
            }
        }
    }
    kinds
}

#[test]
fn step_has_solids_and_rays() {
    for (s, solids) in [(zmx("A0"), 3), (zmx("C0"), 4), (zmx("E0"), 1)] {
        let sc = scene::build(&s, &Options::default()).unwrap();
        let mut b = Vec::new();
        let rep = step::write_step(&mut b, &sc, &Default::default()).unwrap();
        assert_eq!(rep.solids, solids);
        assert_eq!(rep.rays, sc.rays.len());
        let k = check_step(&String::from_utf8(b).unwrap());
        assert_eq!(k["MANIFOLD_SOLID_BREP"], solids);
        assert_eq!(k["POLYLINE"], sc.rays.len());
    }
}

/// 書いた STEP を読み直し、非球面の回転面（SURFACE_OF_REVOLUTION の断面の B スプライン）の点がサグと
/// 許容差の中で合う。読みは lensbench-step（品の STEP の読み手と同じ）
#[test]
fn step_revolved_asphere_follows_the_sag() {
    let s = zmx("E0");
    let sc = scene::build(&s, &Options::default()).unwrap();
    let f = &sc.solids[0].front;
    assert!(!f.is_spherical());
    for tol in [1e-6, 1e-9] {
        let o = step::OptionsStep {
            profile_tol: tol,
            ..Default::default()
        };
        let mut b = Vec::new();
        step::write_step(&mut b, &sc, &o).unwrap();
        let t = String::from_utf8(b).unwrap();
        let k = check_step(&t);
        assert!(
            !k.contains_key("RATIONAL_B_SPLINE_SURFACE")
                && !t.contains("RATIONAL_B_SPLINE_SURFACE")
        );
        let file = lensbench_step::StepFile::parse(&t).unwrap();
        let one = |id: u64| -> &[lensbench_step::Arg] { &file.parts(id).unwrap()[0].1 };
        let nums = |a: &lensbench_step::Arg| -> Vec<f64> {
            a.list().unwrap().iter().map(|x| x.num().unwrap()).collect()
        };
        let point = |id: u64| -> V3 {
            let v = nums(&one(id)[1]);
            [v[0], v[1], v[2]]
        };
        // 面 1（頂点が f の頂点）の回転面
        let revs: Vec<&[lensbench_step::Arg]> = file
            .iter()
            .filter(|(_, p)| p[0].0 == "SURFACE_OF_REVOLUTION")
            .map(|(id, _)| one(id))
            .filter(|rv| {
                let axis = one(rv[2].r().unwrap());
                norm(sub(point(axis[1].r().unwrap()), f.frame.o)) < 1e-9
            })
            .collect();
        assert_eq!(revs.len(), 1);
        let rv = revs[0];
        let (curve, axis) = (one(rv[1].r().unwrap()), one(rv[2].r().unwrap()));
        let o0 = point(axis[1].r().unwrap());
        let az = nums(&one(axis[2].r().unwrap())[1]);
        let az = [az[0], az[1], az[2]];
        let ctrl3: Vec<V3> = curve[2]
            .list()
            .unwrap()
            .iter()
            .map(|x| point(x.r().unwrap()))
            .collect();
        // 断面の面の中の向き（軸に垂直）: 端の制御点から
        let last = sub(*ctrl3.last().unwrap(), o0);
        let xr = normalize(sub(last, scale(az, dot(last, az))));
        let ctrl: Vec<[f64; 2]> = ctrl3
            .iter()
            .map(|p| {
                let d = sub(*p, o0);
                // 制御点は軸を含む面の中
                assert!(norm(sub(d, add(scale(xr, dot(d, xr)), scale(az, dot(d, az))))) < 1e-9);
                [dot(d, xr), dot(d, az)]
            })
            .collect();
        let mults = nums(&curve[6]);
        let kv = nums(&curve[7]);
        let knots: Vec<f64> = kv
            .iter()
            .zip(&mults)
            .flat_map(|(k, m)| std::iter::repeat_n(*k, *m as usize))
            .collect();
        assert_eq!(knots.len(), ctrl.len() + 4);
        let mut worst: f64 = 0.0;
        for i in 0..=4000 {
            let p = step::bspline_eval(&ctrl, &knots, i as f64 / 4000.0);
            worst = worst.max((p[1] - f.sag(p[0].clamp(0.0, f.semi))).abs());
        }
        // 区間の中点で確かめた許容差（間の点でも同じ桁）
        assert!(worst < 2.0 * tol, "tol {tol}: {worst}");
        assert!((ctrl.last().unwrap()[0] - f.semi).abs() < 1e-12);
    }
}

/// 円柱レンズ（トロイダル面）: 立体は閉じて体積が合い、STEP も書ける
#[test]
fn cylinder_lens_mesh_and_step() {
    let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE TOROIDAL\n  CURV 0.05\n  PARM 1 0\n  XDAT 1 0 0 0 1 0 0 \"\"\n  XDAT 2 100 0 0 1 0 0 \"\"\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 30\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let raw = lensbench_formats::zmx::parse_text(txt).unwrap();
    let s = lensbench_formats::zmx::build(&raw, &Default::default(), &[], None).unwrap();
    let sc = scene::build(&s, &Options::default()).unwrap();
    let sd = &sc.solids[0];
    assert!(sd.front.toroidal && !sd.back.toroidal);
    assert_eq!((sd.front.semi, sd.back.semi), (6.0, 6.0));
    // 面の上の点: x には曲がらない
    let p = sd.front.point(3.0, 4.0);
    let z = 0.05 * 16.0 / (1.0 + (1.0 - 0.0025 * 16.0f64).sqrt());
    assert!((dot(sub(p, sd.front.frame.o), sd.front.axis()) - z).abs() < 1e-12);
    // 体積: 円の中で厚さ 4 − z(y) を積分
    let n = 4000;
    let mut exact = 0.0;
    for i in 0..n {
        let y = -6.0 + 12.0 * (i as f64 + 0.5) / n as f64;
        let zy = 0.05 * y * y / (1.0 + (1.0 - 0.0025 * y * y).sqrt());
        exact += 2.0 * (36.0 - y * y).sqrt() * (4.0 - zy) * 12.0 / n as f64;
    }
    let o = Options3D {
        segments: 256,
        rings: 64,
        ..Default::default()
    };
    let v = closed_volume(&mesh::meshes(&sc, &o, None)[0]);
    assert!((v / exact - 1.0).abs() < 3e-4, "{v} {exact}");
    // STEP: 面は B スプライン曲面、縁は B スプライン曲線
    let mut b = Vec::new();
    let rep = step::write_step(&mut b, &sc, &Default::default()).unwrap();
    assert_eq!(rep.solids, 1);
    let k = check_step(&String::from_utf8(b).unwrap());
    assert_eq!(k["MANIFOLD_SOLID_BREP"], 1);
    assert_eq!(k["ADVANCED_FACE"], 3);
    assert_eq!(k["B_SPLINE_SURFACE_WITH_KNOTS"], 3);
}

const TOR_HEAD: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";
const TOR_X: &str = "  XDAT 1 0 0 0 1 0 0 \"\"\n  XDAT 2 100 0 0 1 0 0 \"\"\n";

/// トロイダル面の立体の STEP: 傾いた円柱面と大きい外径（輪あり）、回転半径のある両面トロイダル。
/// 立体の妥当性（BRepCheck）と体積は OpenCASCADE で別に確かめた（2026-11-07、体積はメッシュの閉じた体積と 1e-4 以内）
#[test]
fn toroidal_step_solids() {
    let cases = [
        format!(
            "{TOR_HEAD}SURF 1\n  TYPE COORDBRK\n  PARM 1 0.5\n  PARM 3 3\n  PARM 5 30\n  DISZ 0\n\
SURF 2\n  STOP\n  TYPE TOROIDAL\n  CURV -0.04\n  CONI -1.5\n  PARM 1 0\n  PARM 2 1e-4\n{TOR_X}  DISZ 5\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n  DIAM 5 1 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV -0.02\n  DISZ 30\n  DIAM 7 1 0 0 1 \"\"\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
        ),
        format!(
            "{TOR_HEAD}SURF 1\n  STOP\n  TYPE TOROIDAL\n  CURV 0.03\n  PARM 1 40\n{TOR_X}  DISZ 6\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n  DIAM 8 1 0 0 1 \"\"\n\
SURF 2\n  TYPE TOROIDAL\n  CURV 0.02\n  PARM 1 -60\n{TOR_X}  DISZ 30\n  DIAM 8 1 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
        ),
    ];
    for (i, txt) in cases.iter().enumerate() {
        let raw = lensbench_formats::zmx::parse_text(txt).unwrap();
        let s = lensbench_formats::zmx::build(&raw, &Default::default(), &[], None).unwrap();
        let sc = scene::build(&s, &Options::default()).unwrap();
        assert_eq!(sc.solids.len(), 1);
        let mut b = Vec::new();
        step::write_step(&mut b, &sc, &Default::default()).unwrap();
        let t = String::from_utf8(b).unwrap();
        let k = check_step(&t);
        assert_eq!(k["MANIFOLD_SOLID_BREP"], 1, "{i}");
        // 輪のある面は 1 つ多い
        assert_eq!(k["ADVANCED_FACE"], if i == 0 { 4 } else { 3 }, "{i}");
        let o = Options3D {
            segments: 512,
            rings: 128,
            ..Default::default()
        };
        let v = closed_volume(&mesh::meshes(&sc, &o, None)[0]);
        println!("場合 {i}: メッシュの体積 {v}");
    }
}

/// Fresnel・Cylinder Fresnel・Binary 2の描き方: Fresnel は平面、Cylinder Fresnel は基板（y だけ）、
/// Binary 2 は偶数次非球面の形。立体は閉じ、STEP も書ける
#[test]
fn fresnel_and_binary_faces() {
    for name in ["F1", "C2", "B2"] {
        let opt = lensbench_formats::zmx::LoadOptions {
            catalog_dirs: vec![format!("{ROOT}/tests/glass").into()],
            ..Default::default()
        };
        let s = lensbench_formats::zmx::load_with(
            format!("{ROOT}/tests/surftypes/{name}/system.zmx").as_ref(),
            &opt,
        )
        .unwrap();
        let sc = scene::build(&s, &Options::default()).unwrap();
        let sd = sc.solids.last().unwrap();
        let b = &sd.back;
        let d = &s.surfaces[2];
        let (x, y) = (0.6 * b.semi, 0.5 * b.semi);
        let z = dot(sub(b.point(x, y), b.frame.o), b.axis());
        let want = match name {
            "F1" => 0.0,
            "C2" => lensbench_core::trace::sag(d.c, d.k, d.asph(), y * y).0,
            _ => lensbench_core::trace::sag(d.c, d.k, d.asph(), x * x + y * y).0,
        };
        assert!((z - want).abs() < 1e-12, "{name}: {z} {want}");
        assert_eq!(b.toroidal, name == "C2", "{name}");
        let v = closed_volume(&mesh::meshes(&sc, &Options3D::default(), None)[0]);
        assert!(v > 0.0, "{name}");
        let mut out = Vec::new();
        let rep = step::write_step(&mut out, &sc, &Default::default()).unwrap();
        assert_eq!(rep.solids, sc.solids.len(), "{name}");
    }
}

/// 重ね描き: BS の経路を重ねると共通の玉は 1 つ、ずらすと構成ごとに描く。光線の色分けは構成ごと
#[test]
fn overlay_splitter_paths() {
    let txt =
        std::fs::read_to_string(format!("{ROOT}/examples/projects/P16_twyman_green.json")).unwrap();
    let p = lensbench_project::Project::from_json(&txt).unwrap();
    let env = lensbench_project::Env {
        step_files: None,
        base: format!("{ROOT}/examples/projects").into(),
        glass_dirs: vec![format!("{ROOT}/tests/glass").into()],
        catalog: None,
    };
    let ps = lensbench_project::paths(&p, &env).unwrap();
    let scenes = |off: V3| {
        let parts: Vec<(String, scene::Scene)> = ps
            .iter()
            .map(|q| {
                let s = lensbench_project::expand(
                    &p,
                    &env,
                    &lensbench_project::ExpandOpts {
                        path: Some(q),
                        ..Default::default()
                    },
                )
                .map(|x| (x.system, x.notes))
                .unwrap()
                .0;
                (q.clone(), scene::build(&s, &Options::default()).unwrap())
            })
            .collect();
        let total: usize = parts.iter().map(|x| x.1.solids.len()).sum();
        let nrays: usize = parts.iter().map(|x| x.1.rays.len()).sum();
        (scene::overlay(parts, off).unwrap(), total, nrays)
    };
    let (sc, total, nrays) = scenes([0.0; 3]);
    assert_eq!(sc.configs, ps);
    assert_eq!(sc.rays.len(), nrays);
    // 被検レンズ・キューブの入口側・鏡は経路をまたいで 1 つ
    assert!(sc.solids.len() < total, "{} {total}", sc.solids.len());
    let mut keys: Vec<Vec<i64>> = sc.solids.iter().map(|x| x.key()).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), sc.solids.len());
    assert!(sc.solids[0].name.starts_with("TT "));
    assert_eq!(sc.rays.iter().map(|r| r.cfg).max(), Some(ps.len() - 1));
    // ずらすと全部の立体を描き、構成 k の光線は k 倍ずれる
    let (sc2, total2, _) = scenes([0.0, 0.0, 100.0]);
    assert_eq!(sc2.solids.len(), total2);
    for (a, b) in sc.rays.iter().zip(&sc2.rays) {
        let d = sub(b.points[0], a.points[0]);
        assert!((d[2] - 100.0 * a.cfg as f64).abs() < 1e-9 && d[0].abs() + d[1].abs() < 1e-9);
    }
    // 2D の光線の色は構成ごと
    let o = Options2D {
        color: d2::ColorBy::Config,
        ..Default::default()
    };
    let sh = d2::shapes(&sc, &o);
    let rays: Vec<_> = sh.iter().filter(|x| x.layer == d2::Layer::Rays).collect();
    assert!(rays.iter().any(|x| x.name.starts_with("RR ")));
    assert_ne!(rays.first().unwrap().stroke, rays.last().unwrap().stroke);
}

/// BS のキューブ（一辺 20 mm）は直角プリズム 2 つ: 2D の外形は三角形、3D は閉じて体積が一辺³/2、STEP は平面だけの立体。
/// 反射側へ進む経路、分ける面を逆向き（−45°）にしたものでも同じ
#[test]
fn splitter_cube_is_two_prisms() {
    let txt =
        std::fs::read_to_string(format!("{ROOT}/examples/projects/P16_twyman_green.json")).unwrap();
    for (flip, path) in [(false, "TR"), (false, "RT"), (true, "RT")] {
        let mut v: serde_json::Value = serde_json::from_str(&txt).unwrap();
        if flip {
            v["blocks"][2]["angle"] = serde_json::json!([-45.0, 0.0]);
        }
        let p = lensbench_project::Project::from_json(&v.to_string()).unwrap();
        let env = lensbench_project::Env {
            step_files: None,
            base: format!("{ROOT}/examples/projects").into(),
            glass_dirs: vec![format!("{ROOT}/tests/glass").into()],
            catalog: None,
        };
        let s = lensbench_project::expand(
            &p,
            &env,
            &lensbench_project::ExpandOpts {
                path: Some(path),
                ..Default::default()
            },
        )
        .map(|x| (x.system, x.notes))
        .unwrap()
        .0;
        let sc = scene::build(&s, &Options::default()).unwrap();
        let prisms: Vec<_> = sc.solids.iter().filter(|x| x.poly.is_some()).collect();
        assert!(prisms.len() >= 2, "{path} {flip}");
        for sd in &prisms {
            let p = sd.poly.as_ref().unwrap();
            assert_eq!(p.pts.len(), 6, "{}", sd.name);
            assert_eq!(p.faces.len(), 5, "{}", sd.name);
            let outline = scene::solid_section(sd, 64);
            assert_eq!(outline.len(), 3, "{} {outline:?}", sd.name);
            let o = Options3D::default();
            let one = scene::Scene {
                solids: vec![(*sd).clone()],
                plates: vec![],
                rays: vec![],
                ..sc.clone()
            };
            let vol = closed_volume(&mesh::meshes(&one, &o, None)[0]);
            assert!((vol.abs() - 4000.0).abs() < 1e-6, "{} {vol}", sd.name);
        }
        let mut buf = Vec::new();
        step::write_step(&mut buf, &sc, &step::OptionsStep::default()).unwrap();
        let t = String::from_utf8(buf).unwrap();
        assert!(t.contains("LINE("), "{path}");
    }
}

/// 反射側の口だけを通る BS のキューブ: 通る半分（入る面〜分ける面〜反射側の出る面）は同じ直角プリズムなので、
/// もう 1 つは分ける面で折り返した透過側の半分にする。2 つ合わせて一辺の正方形（体積は一辺³）
#[test]
fn splitter_reflect_only_draws_whole_cube() {
    let v = serde_json::json!({
        "name": "BS reflect only",
        "wavelengths": [{"um": 0.6328}],
        "aperture": {"type": "na", "value": 0.02},
        "fields": {"type": "object_height", "points": [[0, 0]]},
        "object_distance": 100,
        "blocks": [
            {"type": "stop", "semi": 3},
            {"type": "air", "t": 20},
            {"type": "splitter", "id": "BS", "form": "cube", "size": 20, "glass": "N-BK7",
             "ratio": 0.5, "arm": "r", "r": [{"type": "air", "t": 30}]}
        ]
    });
    for flip in [false, true] {
        let mut v = v.clone();
        if flip {
            v["blocks"][2]["angle"] = serde_json::json!([-45.0, 0.0]);
        }
        let p = lensbench_project::Project::from_json(&v.to_string()).unwrap();
        let env = lensbench_project::Env {
            step_files: None,
            base: format!("{ROOT}/examples/projects").into(),
            glass_dirs: vec![format!("{ROOT}/tests/glass").into()],
            catalog: None,
        };
        let s = lensbench_project::expand(
            &p,
            &env,
            &lensbench_project::ExpandOpts {
                path: None,
                ..Default::default()
            },
        )
        .map(|x| (x.system, x.notes))
        .unwrap()
        .0;
        let sc = scene::build(&s, &Options::default()).unwrap();
        let prisms: Vec<_> = sc.solids.iter().filter(|x| x.poly.is_some()).collect();
        assert_eq!(prisms.len(), 2, "{flip}");
        // 2 つの直角プリズムは重ならない: 頂点の重心が違い、合わせた頂点は立方体の 8 角
        let center = |sd: &scene::Solid| {
            let p = sd.poly.as_ref().unwrap();
            let n = p.pts.len() as f64;
            p.pts
                .iter()
                .fold([0.0; 3], |a, q| add(a, *q))
                .map(|x| x / n)
        };
        let (c0, c1) = (center(prisms[0]), center(prisms[1]));
        assert!(norm(sub(c0, c1)) > 1.0, "{flip} {c0:?} {c1:?}");
        let mut all: Vec<V3> = Vec::new();
        for sd in &prisms {
            for &q in &sd.poly.as_ref().unwrap().pts {
                if !all.iter().any(|&r| norm(sub(q, r)) < 1e-9) {
                    all.push(q);
                }
            }
        }
        assert_eq!(all.len(), 8, "{flip}");
        let o = Options3D::default();
        let mut total = 0.0;
        for sd in &prisms {
            let one = scene::Scene {
                solids: vec![(*sd).clone()],
                plates: vec![],
                rays: vec![],
                ..sc.clone()
            };
            let vol = closed_volume(&mesh::meshes(&one, &o, None)[0]);
            assert!((vol.abs() - 4000.0).abs() < 1e-6, "{} {vol}", sd.name);
            total += vol.abs();
        }
        assert!((total - 8000.0).abs() < 1e-6);
        // 2D の断面も三角形 2 つ
        for sd in &prisms {
            assert_eq!(scene::solid_section(sd, 64).len(), 3);
        }
        let mut buf = Vec::new();
        step::write_step(&mut buf, &sc, &step::OptionsStep::default()).unwrap();
        assert_eq!(
            String::from_utf8(buf)
                .unwrap()
                .matches("MANIFOLD_SOLID_BREP")
                .count(),
            sc.solids.len()
        );
    }
}

/// 機構部品: 場面の輪、閉じた網の体積、STEP の輪の立体（辺は 2 回、逆向きに使う）、2D の MECH
#[test]
fn mechanics_are_drawn() {
    let s = project("P19_tube_cage");
    let sc = scene::build(&s, &Options::default()).unwrap();
    assert_eq!(sc.mechs.len(), 2);
    assert_eq!(sc.mechs[0].rings, vec![[0.0, 30.0, 12.7, 15.25]]);
    // ケージ板は L1 の後面（絞りから 25）から 25 mm、厚さ 8
    let z = sc.mechs[1].frame.o[2];
    assert!((z - 50.0).abs() < 1e-9, "{z}");
    let o = Options3D::default();
    let ms = mesh::meshes(&sc, &o, None);
    let n = o.segments as f64;
    let area = |r: f64| 0.5 * n * r * r * (2.0 * std::f64::consts::PI / n).sin();
    for (m, sh) in ms
        .iter()
        .filter(|m| m.kind == mesh::MeshKind::Mech)
        .zip(&sc.mechs)
    {
        let want: f64 = sh
            .rings
            .iter()
            .map(|q| (area(q[3]) - area(q[2])) * (q[1] - q[0]))
            .sum();
        let v = closed_volume(m);
        assert!((v - want).abs() < 1e-9 * want, "{}: {v} {want}", m.name);
    }
    let mut b = Vec::new();
    let rep = step::write_step(&mut b, &sc, &Default::default()).unwrap();
    assert_eq!(rep.mechs, 2);
    let t = String::from_utf8(b).unwrap();
    let k = check_step(&t);
    assert_eq!(k["MANIFOLD_SOLID_BREP"], rep.solids + 2);
    // 輪の立体の辺: 各辺を 2 回、逆の向きで使う
    let data = &t[t.find("DATA;").unwrap() + 5..t.rfind("ENDSEC;").unwrap()];
    let mut uses: HashMap<String, (i32, i32)> = HashMap::new();
    for l in data.lines() {
        if let Some(i) = l.find("ORIENTED_EDGE('',*,*,#") {
            let rest = &l[i + 22..];
            let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let e = uses.entry(id).or_insert((0, 0));
            e.0 += 1;
            e.1 += if rest.contains(".T.") { 1 } else { -1 };
        }
    }
    assert!(uses.values().all(|&(n, s)| n == 2 && s == 0), "{uses:?}");
    let v = d2::shapes(&sc, &Options2D::default());
    assert_eq!(v.iter().filter(|x| x.layer == d2::Layer::Mech).count(), 4);
    let mut svg = Vec::new();
    d2::write_dxf(&mut svg, &sc, &Options2D::default()).unwrap();
    assert!(String::from_utf8(svg).unwrap().contains("MECH"));
    // 部品の外径が箱に入る
    let (lo, hi) = sc.bounds();
    assert!(lo[1] <= -20.5 && hi[1] >= 20.5);
}

/// ガラスの立体・鏡の名前は、面を含むプロジェクトの要素の id（id の無い要素は L1 … のまま）
#[test]
fn solids_are_named_after_project_elements() {
    let txt = r#"{"wavelengths": [{"um": 0.5876}], "aperture": {"type": "epd", "value": 5},
        "blocks": [{"type": "stop", "semi": 3}, {"type": "air", "t": 5},
        {"type": "lens", "id": "front", "source": {"surfaces": [{"radius": 30, "t": 3, "glass": "N-BK7", "semi": 6}, {"radius": -30, "semi": 6}]}},
        {"type": "air", "t": 5},
        {"type": "lens", "source": {"surfaces": [{"radius": 40, "t": 3, "glass": "N-BK7", "semi": 6}, {"radius": 0, "semi": 6}]}},
        {"type": "air", "t": 20}, {"type": "mirror", "id": "fold", "semi": 8}, {"type": "air", "t": 10}]}"#;
    let q = lensbench_project::Project::from_json(txt).unwrap();
    let env = lensbench_project::Env::in_dir(
        std::path::Path::new(ROOT),
        &[format!("{ROOT}/tests/glass").into()],
        None,
    );
    let s = lensbench_project::expand(&q, &env, &Default::default())
        .unwrap()
        .system;
    let sc = scene::build(&s, &Options::default()).unwrap();
    let names: Vec<&str> = sc.solids.iter().map(|x| x.name.as_str()).collect();
    assert!(names[0].starts_with("front S"), "{names:?}");
    assert!(names[1].starts_with("L2 S"), "{names:?}");
    assert!(names[2].starts_with("fold S"), "{names:?}");
}

/// 折れ線を閉じた多角形（点の (z, y) へ投影）が自分自身と交差しないか（隣り合う辺どうしは見ない）
fn simple_polygon(pts: &[V3]) -> bool {
    let p: Vec<(f64, f64)> = pts.iter().map(|q| (q[2], q[1])).collect();
    let n = p.len();
    let cr = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    for i in 0..n {
        let (a, b) = (p[i], p[(i + 1) % n]);
        for j in i + 2..n {
            if (j + 1) % n == i {
                continue;
            }
            let (c, d) = (p[j], p[(j + 1) % n]);
            let (d1, d2) = (cr(a, b, c), cr(a, b, d));
            let (d3, d4) = (cr(c, d, a), cr(c, d, b));
            if d1 * d2 < -1e-12 && d3 * d4 < -1e-12 {
                return false;
            }
        }
    }
    true
}

/// ダブルパスの系の展開: 鏡で折り返した後（鏡映で左手系になった座標）の玉も、断面が交差しない玉の形になり、
/// 往きの玉を鏡の面で鏡映した形になる
#[test]
fn unfolded_return_lens_keeps_its_shape() {
    // P4 は球面の鏡、その鏡を傾けた平面鏡にしたもの（P12 と同じ形、P12 はカタログの玉なのでここでは組めない）、
    // 折り曲げの鏡の後ろに玉がある P3
    let tilted = |t: String| t.replace("\"radius\": -80,", "\"tilt\": [0.5, -0.3],");
    let systems = [
        ("P4_double_pass", project("P4_double_pass")),
        ("P4 tilted flat", project_with("P4_double_pass", tilted)),
        ("P3_fold", project("P3_fold")),
    ];
    for (name, s) in systems {
        let opt = Options {
            unfold: true,
            ..Default::default()
        };
        let sc = scene::build(&s, &opt).unwrap();
        let glass: Vec<_> = sc
            .solids
            .iter()
            .filter(|x| x.kind == SolidKind::Glass)
            .collect();
        // ダブルパスは往きと戻りの 2 つ（展開では戻りの玉が鏡の後ろに別の場所に描かれる）
        let want = if name.starts_with("P4") { 2 } else { 1 };
        assert!(glass.len() >= want, "{name}: {} glass solids", glass.len());
        for sd in &glass {
            assert!(
                simple_polygon(&scene::solid_section(sd, 24)),
                "{name}: {} crosses itself",
                sd.name
            );
            assert!(
                simple_polygon(&scene::solid_section_xz(sd, 24)),
                "{name}: {} (xz) crosses itself",
                sd.name
            );
            // 前の面の縁 y = +r と、後ろの面の y = +r の点は同じ側（軸から同じ向き）
            let r = sd.rim;
            let (fp, bp) = (sd.front.point(0.0, r), sd.back_point(0.0, r));
            let fy = col(&sd.front.frame.r, 1);
            let side = |p: V3| dot(sub(p, sd.front.frame.o), fy);
            assert!(side(fp) > 0.0 && side(bp) > 0.0, "{name}: {}", sd.name);
        }
    }
}

/// 機構部品に切られる光線: 部品で切る設定（既定）なら切られる点で終わり、切なら最後まで描いて切られる点から
/// 後ろを薄く（P19 の 20°）。0° は切られない。2D の書き出しは薄い部分を別の線にする
#[test]
fn rays_clipped_by_mechanics_stop_or_turn_faint() {
    let on = project("P19_tube_cage");
    let off = project_with("P19_tube_cage", |t| {
        t.replacen("\"fields\"", "\"mech_clip\": false, \"fields\"", 1)
    });
    assert!(on.mech_clip && !off.mech_clip);
    let opt = Options {
        fields: Some(vec![0, 2]),
        ..Options::default()
    };
    let a = scene::build(&on, &opt).unwrap();
    let b = scene::build(&off, &opt).unwrap();
    assert_eq!(a.rays.len(), b.rays.len());
    // 部品の穴の最小の半径（チューブ 11.5・ケージ板 11）
    let rmin = on
        .mechanics
        .iter()
        .flat_map(|m| m.bores.iter().map(|b| b.radius))
        .fold(f64::MAX, f64::min);
    let mut cut = 0;
    for (x, y) in a.rays.iter().zip(&b.rays) {
        if x.field == 0 {
            assert_eq!((x.faint, y.faint), (None, None));
            assert_eq!(x.points, y.points);
            continue;
        }
        let k = y.faint.expect("20° は部品に切られる");
        assert_eq!(
            x.faint,
            Some(x.points.len() - 1),
            "入なら切られる点で終わる"
        );
        assert!(y.points.len() > k + 1, "切なら後ろも描く");
        assert_eq!(x.points[..], y.points[..=k]);
        // 切られる点は軸から穴の半径より外
        let p = y.points[k];
        assert!(p[0].hypot(p[1]) >= rmin - 1e-9, "{p:?}");
        cut += 1;
    }
    assert!(cut > 0);
    let thin = |sc: &scene::Scene| {
        d2::shapes(sc, &Options2D::default())
            .iter()
            .filter(|s| s.layer == d2::Layer::Rays)
            .count()
    };
    assert_eq!(thin(&b), thin(&a) + cut);
}
