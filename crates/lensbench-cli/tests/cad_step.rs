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
    std::fs::remove_dir_all(&dir).ok();
}
