//! 品の STEP を 1 件だけ取ってキャッシュする（公開 #120）: 手元の HTTP の口から、lensbench が書き出した角の
//! ケージ板の STEP を返し、1 回目は取りに行き、2 回目はキャッシュを使うこと・簡易外形と比べること・
//! STEP でない中身（HTML）は誤りで品ページを示すことを確かめる

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// 道（/plate.step・/page.html）ごとに中身を返す口。受けた要求の数を数える
fn server(step: Vec<u8>) -> (String, Arc<AtomicUsize>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let n = Arc::new(AtomicUsize::new(0));
    let n2 = n.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            n2.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 4096];
            let k = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..k]).to_string();
            let body: Vec<u8> = if req.starts_with("GET /plate.step") {
                step.clone()
            } else {
                b"<!DOCTYPE html><html><body>sign in</body></html>".to_vec()
            };
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(&body);
        }
    });
    (base, n)
}

/// 角のケージ板（一辺 40.64・長さ 8.89・内径 26.416）だけを置いた系を STEP に書き出す
fn plate_step(dir: &std::path::Path) -> Vec<u8> {
    let p = dir.join("p.json");
    std::fs::write(
        &p,
        json!({
            "wavelengths": [{ "um": 0.5876 }],
            "aperture": { "type": "epd", "value": 10 },
            "blocks": [{ "type": "stop" }, { "type": "air", "t": 20 }],
            "mechanics": [{ "name": "plate", "at": { "surface": 1 },
                "mech": { "bores": [{ "z0": 0, "z1": 8.89, "diameter": 26.416 }],
                          "length": 8.89, "outer_diameter": 40.64, "outline": "square" } }],
        })
        .to_string(),
    )
    .unwrap();
    let step = dir.join("p.step");
    let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
        .args(["layout", p.to_str().unwrap(), "--glass-dir", "tests/glass"])
        .args(["--rays", "0", "--out"])
        .arg(&step)
        .current_dir(root())
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    std::fs::read(&step).unwrap()
}

#[test]
fn step_is_fetched_once_and_cached() {
    let dir = std::env::temp_dir().join(format!("lensbench-cad-step-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (base, hits) = server(plate_step(&dir));
    let mech = |outer: f64| {
        json!({ "bores": [{ "z0": 0, "z1": 8.89, "diameter": 26.416 }],
                "length": 8.89, "outer_diameter": outer, "outline": "square" })
    };
    let cat = dir.join("cat.json");
    std::fs::write(
        &cat,
        json!({ "items": {
            "CP-A": { "mech": mech(40.64), "url": "https://example.com/CP-A",
                      "cad": { "step": format!("{base}/plate.step"), "pdf": format!("{base}/a.pdf") } },
            // 簡易外形が STEP と違う品（外径 30・丸）
            "CP-B": { "mech": { "bores": [{ "z0": 0, "z1": 8.89, "diameter": 26.416 }],
                                "length": 8.89, "outer_diameter": 30.0 },
                      "cad": { "step": format!("{base}/plate.step?b") } },
            // STEP でない中身（ログインのページ）
            "CP-C": { "mech": mech(40.64), "url": "https://example.com/CP-C",
                      "cad": { "step": format!("{base}/page.html") } },
            "CP-D": { "mech": mech(40.64) },
        } })
        .to_string(),
    )
    .unwrap();
    let cache = dir.join("cache");
    let call = |item: &str| -> Result<Value, String> {
        lensbench_cli::serve::call(
            "cad_step",
            &json!({ "catalog": cat.to_str().unwrap(), "item": item, "cache": cache.to_str().unwrap() }),
            None,
        )
        .map_err(|e| e.message)
    };
    // 1 回目は取りに行き、2 回目はキャッシュ
    let a = call("CP-A").unwrap();
    assert_eq!(a["cached"], false, "{a}");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(a["notes"], json!([]), "{a}");
    assert_eq!(a["measured"]["frame"]["outline"], "square", "{a}");
    let path = std::path::PathBuf::from(a["path"].as_str().unwrap());
    assert!(path.starts_with(&cache) && path.is_file());
    let a2 = call("CP-A").unwrap();
    assert_eq!(a2["cached"], true);
    assert_eq!(a2["path"], a["path"]);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "キャッシュがあれば取りに行かない"
    );
    // 簡易外形との食い違い
    let b = call("CP-B").unwrap();
    let notes: Vec<&str> = b["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(notes.len(), 2, "{b}");
    assert!(
        notes[0].starts_with("外径: カタログ 30 mm、STEP 40.64 mm"),
        "{notes:?}"
    );
    assert!(
        notes[1].starts_with("外形: カタログ 丸、STEP 角"),
        "{notes:?}"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    // STEP でなければ誤りで、品ページを示す。キャッシュに残さない
    let c = call("CP-C").unwrap_err();
    assert!(
        c.contains("STEP（ISO-10303-21）でない") && c.contains("https://example.com/CP-C"),
        "{c}"
    );
    let n_files = std::fs::read_dir(&cache).unwrap().count();
    assert_eq!(n_files, 2, "取れた 2 品だけ");
    // URL の無い品
    assert!(call("CP-D").unwrap_err().contains("cad.step"));
    // CLI
    let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
        .args([
            "cad-step",
            "--catalog",
            cat.to_str().unwrap(),
            "--item",
            "CP-A",
            "--json",
        ])
        .arg("--cache")
        .arg(&cache)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["cached"], true, "{v}");
    detail_is_drawn(&dir, &cat, &cache);
    std::fs::remove_dir_all(&dir).ok();
}

/// 詳しい形（#121）: キャッシュに STEP のある品を置き、layout --mech-detail で STL・STEP に取った STEP の形を書く
fn detail_is_drawn(dir: &std::path::Path, cat: &std::path::Path, cache: &std::path::Path) {
    let p = dir.join("placed.json");
    std::fs::write(
        &p,
        json!({
            "wavelengths": [{ "um": 0.5876 }],
            "aperture": { "type": "epd", "value": 10 },
            "blocks": [{ "type": "stop" }, { "type": "air", "t": 20 }],
            "mechanics": [{ "name": "a", "catalog": cat.to_str().unwrap(), "item": "CP-A",
                            "at": { "surface": 1 }, "offset": [0, 0, 5] }],
        })
        .to_string(),
    )
    .unwrap();
    let layout = |out: &str, detail: bool| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_lensbench"));
        c.args([
            "layout",
            p.to_str().unwrap(),
            "--glass-dir",
            "tests/glass",
            "--rays",
            "0",
        ])
        .arg("--out")
        .arg(dir.join(out))
        .env("LENSBENCH_CAD_CACHE", cache)
        .current_dir(root());
        if detail {
            c.arg("--mech-detail");
        }
        let o = c.output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        std::fs::read(dir.join(out)).unwrap()
    };
    // STL: 三角形の数が簡易外形と違い、z は 5〜13.89
    let stl_tris = |b: &[u8]| u32::from_le_bytes(b[80..84].try_into().unwrap());
    let simple = layout("s.stl", false);
    let det = layout("d.stl", true);
    assert_ne!(stl_tris(&simple), stl_tris(&det));
    let n = stl_tris(&det) as usize;
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for t in 0..n {
        let o = 84 + 50 * t + 12;
        for v in 0..3 {
            let z = f32::from_le_bytes(b4(&det[o + 12 * v + 8..]));
            // 絞りの面（z 0）と像面（z 20）の円板を除く
            if z.abs() > 1e-4 && (z - 20.0).abs() > 1e-4 {
                lo = lo.min(z);
                hi = hi.max(z);
            }
        }
    }
    assert!((lo - 5.0).abs() < 1e-3, "{lo}");
    assert!((hi - 13.89).abs() < 1e-3, "{hi}");
    // STEP: 取った STEP の立体を写し、測り直すと品の寸法
    let st = String::from_utf8(layout("d.step", true)).unwrap();
    assert!(
        st.contains("MANIFOLD_SOLID_BREP('a detail 1'"),
        "写した立体"
    );
    let s = lensbench_project::catalog_fill::read_step(&st).unwrap();
    let f = s.frame.clone().expect("枠");
    assert_eq!((f.length, f.outer_diameter), (8.89, 40.64), "{s:?}");
    assert_eq!(f.outline, lensbench_project::MechOutline::Square);
    // 詳しい形を選ばなければ簡易外形
    let st0 = String::from_utf8(layout("s.step", false)).unwrap();
    assert!(!st0.contains("detail") && st0.contains("MANIFOLD_SOLID_BREP('a 1'"));
}

fn b4(b: &[u8]) -> [u8; 4] {
    [b[0], b[1], b[2], b[3]]
}

/// 共有の置き場の口（/cad/<鍵> の GET・HEAD・PUT）。Access のサービストークンの見出しが無ければ 302（ログインへ）。
/// 受けた要求（方法と道）を覚える
type Db = Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>>;

fn store_server() -> (String, Db, Arc<std::sync::Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/cad", l.local_addr().unwrap());
    let db: Db = Default::default();
    let log: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let (db2, log2) = (db.clone(), log.clone());
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            // 見出しと本文（Content-Length の分）を読む
            let mut buf = Vec::new();
            let mut tmp = [0u8; 65536];
            let (head, mut body) = loop {
                let k = s.read(&mut tmp).unwrap_or(0);
                if k == 0 {
                    break (String::from_utf8_lossy(&buf).to_string(), Vec::new());
                }
                buf.extend_from_slice(&tmp[..k]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break (
                        String::from_utf8_lossy(&buf[..i]).to_string(),
                        buf[i + 4..].to_vec(),
                    );
                }
            };
            let lower = head.to_ascii_lowercase();
            let len: usize = lower
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            while body.len() < len {
                let k = s.read(&mut tmp).unwrap_or(0);
                if k == 0 {
                    break;
                }
                body.extend_from_slice(&tmp[..k]);
            }
            let mut it = head.split_whitespace();
            let (m, path) = (
                it.next().unwrap_or("").to_string(),
                it.next().unwrap_or("").to_string(),
            );
            log2.lock().unwrap().push(format!("{m} {path}"));
            let key = path.trim_start_matches("/cad/").to_string();
            let ok_token = lower.contains("cf-access-client-id: tid")
                && lower.contains("cf-access-client-secret: tsecret");
            let (st, out): (&str, Vec<u8>) = if !ok_token {
                ("302 Found\r\nLocation: https://login.example/", Vec::new())
            } else {
                match m.as_str() {
                    "PUT" => {
                        db2.lock().unwrap().insert(key, body);
                        ("201 Created", Vec::new())
                    }
                    "GET" | "HEAD" => match db2.lock().unwrap().get(&key) {
                        Some(b) if m == "GET" => ("200 OK", b.clone()),
                        Some(_) => ("200 OK", Vec::new()),
                        None => ("404 Not Found", Vec::new()),
                    },
                    _ => ("405 Method Not Allowed", Vec::new()),
                }
            };
            let _ = write!(
                s,
                "HTTP/1.1 {st}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                out.len()
            );
            let _ = s.write_all(&out);
        }
    });
    (base, db, log)
}

#[test]
fn store_is_checked_first_and_vendor_steps_are_uploaded() {
    let dir = std::env::temp_dir().join(format!("lensbench-cad-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let step = plate_step(&dir);
    let (vendor, hits) = server(step.clone());
    let (store_url, db, log) = store_server();
    let mech = json!({ "bores": [{ "z0": 0, "z1": 8.89, "diameter": 26.416 }],
                       "length": 8.89, "outer_diameter": 40.64, "outline": "square" });
    let cat = dir.join("cat.json");
    std::fs::write(
        &cat,
        json!({ "items": {
            "CP-A": { "mech": mech, "cad": { "step": format!("{vendor}/plate.step") } },
            // URL の無い品（置き場にだけある）
            "CP-N": { "mech": mech },
        } })
        .to_string(),
    )
    .unwrap();
    let run = |args: &[&str], cache: &std::path::Path, secret: &str| {
        let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
            .args(args)
            .arg("--json")
            .env("LENSBENCH_CAD_CACHE", cache)
            .env("LENSBENCH_CAD_STORE", &store_url)
            .env("LENSBENCH_CAD_STORE_ID", "tid")
            .env("LENSBENCH_CAD_STORE_SECRET", secret)
            .output()
            .unwrap();
        (
            o.status.success(),
            serde_json::from_slice::<Value>(&o.stdout).unwrap_or(Value::Null),
            String::from_utf8_lossy(&o.stderr).to_string(),
        )
    };
    let c = cat.to_str().unwrap();
    // 1 台目: 置き場に無いので配布元から取り、置き場に上げる
    let (ok, v, e) = run(
        &["cad-step", "--catalog", c, "--item", "CP-A"],
        &dir.join("c1"),
        "tsecret",
    );
    assert!(ok, "{e}");
    assert_eq!(v["source"], "vendor_uploaded", "{v}");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let key = std::path::Path::new(v["path"].as_str().unwrap())
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_eq!(db.lock().unwrap().get(&key), Some(&step));
    // 2 台目（別のキャッシュ）: 置き場から取り、配布元へは行かない
    let (ok, v, e) = run(
        &["cad-step", "--catalog", c, "--item", "CP-A"],
        &dir.join("c2"),
        "tsecret",
    );
    assert!(ok, "{e}");
    assert_eq!(v["source"], "store", "{v}");
    assert_eq!(v["notes"], json!([]), "{v}");
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "置き場にあれば配布元へ行かない"
    );
    // トークンが違えば（Access のログインへの 302）、配布元から取り、置き場の誤りを添える
    let (ok, v, e) = run(
        &["cad-step", "--catalog", c, "--item", "CP-A"],
        &dir.join("c3"),
        "bad",
    );
    assert!(ok, "{e}");
    assert_eq!(v["source"], "vendor", "{v}");
    assert!(
        v["store_notes"][0]
            .as_str()
            .unwrap()
            .contains("サービストークン"),
        "{v}"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    // URL の無い品: 置き場に無ければ誤り。cad-upload --dir でファイル名から当てて上げると取れる
    let (ok, _, e) = run(
        &["cad-step", "--catalog", c, "--item", "CP-N"],
        &dir.join("c4"),
        "tsecret",
    );
    assert!(!ok && e.contains("置き場にもない"), "{e}");
    let up = dir.join("vendor-files");
    std::fs::create_dir_all(&up).unwrap();
    std::fs::write(up.join("CP-N_ttn0001-e0w.step"), &step).unwrap();
    std::fs::write(up.join("unknown.step"), &step).unwrap();
    let u = up.to_str().unwrap();
    let n_put = |log: &Vec<String>| log.iter().filter(|l| l.starts_with("PUT")).count();
    let put0 = n_put(&log.lock().unwrap());
    let (ok, v, e) = run(
        &["cad-upload", "--catalog", c, "--dir", u, "--dry-run"],
        &dir.join("c4"),
        "tsecret",
    );
    assert!(ok, "{e}");
    assert_eq!(v["files"][0]["status"], "dry_run", "{v}");
    assert_eq!(v["unmatched"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(n_put(&log.lock().unwrap()), put0, "dry-run は上げない");
    let (ok, v, e) = run(
        &["cad-upload", "--catalog", c, "--dir", u],
        &dir.join("c4"),
        "tsecret",
    );
    assert!(ok, "{e}");
    assert_eq!(
        (v["uploaded"].as_u64(), v["files"][0]["item"].as_str()),
        (Some(1), Some("CP-N")),
        "{v}"
    );
    let (ok, v, e) = run(
        &["cad-step", "--catalog", c, "--item", "CP-N"],
        &dir.join("c4"),
        "tsecret",
    );
    assert!(ok, "{e}");
    assert_eq!(v["source"], "store", "{v}");
    // キャッシュをそのまま上げる: 置き場にある鍵は上げない
    let (ok, v, e) = run(
        &["cad-upload", "--cache", dir.join("c1").to_str().unwrap()],
        &dir.join("c1"),
        "tsecret",
    );
    assert!(ok, "{e}");
    assert_eq!(
        (v["exists"].as_u64(), v["uploaded"].as_u64()),
        (Some(1), Some(0)),
        "{v}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 膜のモデル: coat-upload で目録を作って上げ、フォルダから外した膜を置き場から取って使う（系の膜の自動の読み）。
/// coat-fetch でフォルダに戻す
#[test]
fn coating_models_come_from_the_store_when_missing() {
    use lensbench_core::coating::Coatings;
    let dir = std::env::temp_dir().join(format!("lensbench-coat-store-{}", std::process::id()));
    let coat = dir.join("coating");
    std::fs::create_dir_all(&coat).unwrap();
    let (store_url, db, _log) = store_server();
    // 膜 ZEC_R のモデル（どの角度・波長でも素通し）
    let mut lbcm = String::from("LBCM 2\nCOAT ZEC_R\nD 0.0\nBMAX 1.3\n");
    for w in [0.4, 0.6, 0.8] {
        lbcm += &format!("WAVE {w}\n");
        for i in 0..=13 {
            let b = 0.1 * i as f64;
            lbcm += &format!("{b:?} 1 0 0 0 0 0 1 0 0 1 0 0 0 0 0 1 0 0\n");
        }
    }
    std::fs::write(coat.join("ZEC_R.lbcb"), &lbcm).unwrap();
    std::fs::write(coat.join("COATING.DAT"), "MATE N15\n0.5 1.5 0\n").unwrap();
    let run = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
            .args(args)
            .arg("--json")
            .env("LENSBENCH_CAD_CACHE", dir.join("cache"))
            .env("LENSBENCH_CAD_STORE", &store_url)
            .env("LENSBENCH_CAD_STORE_ID", "tid")
            .env("LENSBENCH_CAD_STORE_SECRET", "tsecret")
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        serde_json::from_slice::<Value>(&o.stdout).unwrap()
    };
    let v = run(&["coat-upload", coat.to_str().unwrap()]);
    assert_eq!(v["uploaded"], 1, "{v}");
    let idx: Value =
        serde_json::from_slice(&std::fs::read(coat.join("lbcb-remote.json")).unwrap()).unwrap();
    let h = idx["files"]["ZEC_R"].as_str().unwrap().to_string();
    assert_eq!(h, lensbench_cli::coat_remote::hash_hex(lbcm.as_bytes()));
    assert!(db
        .lock()
        .unwrap()
        .contains_key(&format!("coat/ZEC_R-{h}.lbcb")));
    assert_eq!(run(&["coat-upload", coat.to_str().unwrap()])["exists"], 1);
    // 配る版と同じく .lbcb を外す。系の膜は置き場から取って読む
    std::fs::remove_file(coat.join("ZEC_R.lbcb")).unwrap();
    let opt = lensbench_core::zmx::LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    let mut s =
        lensbench_core::zmx::load_with(&root().join("tests/refs/A0/system.zmx"), &opt).unwrap();
    let j = (1..s.surfaces.len())
        .find(|&j| !s.surfaces[j].mat.is_empty())
        .unwrap();
    s.surfaces[j].coat = "zec_r".into();
    s.coating_file = Some(coat.join("COATING.DAT"));
    lensbench_project::catalog_fill::set_cad_cache_dir(dir.join("cache"));
    lensbench_cli::cad::set_store(Some(lensbench_cli::cad::Store {
        url: store_url.clone(),
        id: "tid".into(),
        secret: "tsecret".into(),
    }));
    // 口が無いうちは読めない
    assert!(Coatings::for_system(&s).unwrap().auto.is_empty());
    lensbench_cli::coat_remote::install();
    let c = Coatings::for_system(&s).unwrap();
    assert_eq!(c.auto.len(), 1, "{:?}", c.auto);
    assert!(
        c.auto[0].1.starts_with(dir.join("cache").join("coat")),
        "{:?}",
        c.auto
    );
    assert!(c.models.contains_key("zec_r"));
    // 中身が目録と違えば使わない（キャッシュも作らない）
    db.lock()
        .unwrap()
        .insert(format!("coat/ZEC_R-{h}.lbcb"), b"LBCM 2\nCOAT X\n".to_vec());
    std::fs::remove_dir_all(dir.join("cache")).unwrap();
    let e = lensbench_cli::coat_remote::fetch(lensbench_cli::cad::store().as_ref(), "ZEC_R", &h)
        .unwrap_err();
    assert!(e.contains("目録と違う"), "{e}");
    db.lock()
        .unwrap()
        .insert(format!("coat/ZEC_R-{h}.lbcb"), lbcm.clone().into_bytes());
    // coat-fetch でフォルダに戻す
    let v = run(&["coat-fetch", coat.to_str().unwrap()]);
    assert_eq!(v["fetched"], 1, "{v}");
    assert_eq!(
        std::fs::read_to_string(coat.join("ZEC_R.lbcb")).unwrap(),
        lbcm
    );
    std::fs::remove_dir_all(&dir).ok();
}
