//! serve・watch: 外のプロセスから系を開き、間隔を変えて再解析を繰り返す。1 回の往復の時間も測る
//! （`cargo test --release -p lensbench-cli --test it serve:: -- --nocapture` で表示）。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use lensbench_core::system::Launch;
use lensbench_core::{analysis, zmx, zmx_write, FieldSel, System};
use serde_json::{json, Value};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const ZMX: &str = "tests/refs/A0/system.zmx";

fn exe() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_lensbench"));
    c.current_dir(ROOT).env_remove("LENSBENCH_GLASSCAT");
    c
}

fn core_system(path: &Path) -> System {
    let opt = zmx::LoadOptions {
        catalog_dirs: vec![Path::new(ROOT).join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(path, &opt).unwrap()
}

struct Server {
    child: Child,
    r: BufReader<TcpStream>,
    w: TcpStream,
    id: u64,
}

impl Server {
    fn start(args: &[&str]) -> Server {
        let mut child = exe()
            .args(["--threads", "1", "serve", "--port", "0"])
            .args(args)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        let port = v["port"].as_u64().unwrap() as u16;
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_nodelay(true).unwrap();
        Server {
            child,
            r: BufReader::with_capacity(1 << 16, s.try_clone().unwrap()),
            w: s,
            id: 0,
        }
    }

    /// 要求を送り (ヘッダ, 配列) を返す。配列は f64 は f64、i32 は f64 に直して返す
    fn call_bin(
        &mut self,
        method: &str,
        params: Value,
        bins: &[(&str, &[f64])],
    ) -> (Value, Vec<(String, Vec<f64>)>) {
        self.id += 1;
        let mut h = json!({ "jsonrpc": "2.0", "id": self.id, "method": method, "params": params });
        if !bins.is_empty() {
            h["bin"] = Value::Array(
                bins.iter()
                    .map(|(n, v)| json!({ "name": n, "dtype": "<f8", "shape": [v.len()] }))
                    .collect(),
            );
        }
        let mut out = serde_json::to_vec(&h).unwrap();
        out.push(b'\n');
        for (_, v) in bins {
            for x in *v {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        self.w.write_all(&out).unwrap();
        let mut line = String::new();
        self.r.read_line(&mut line).unwrap();
        let h: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(h["id"], json!(self.id));
        let mut arrs = Vec::new();
        if let Some(list) = h["bin"].as_array() {
            for b in list {
                let n = b["shape"][0].as_u64().unwrap() as usize;
                let v = match b["dtype"].as_str().unwrap() {
                    "<f8" => {
                        let mut buf = vec![0u8; n * 8];
                        self.r.read_exact(&mut buf).unwrap();
                        buf.chunks_exact(8)
                            .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
                            .collect()
                    }
                    "<i4" => {
                        let mut buf = vec![0u8; n * 4];
                        self.r.read_exact(&mut buf).unwrap();
                        buf.chunks_exact(4)
                            .map(|c| i32::from_le_bytes(c.try_into().unwrap()) as f64)
                            .collect()
                    }
                    d => panic!("dtype {d}"),
                };
                arrs.push((b["name"].as_str().unwrap().to_string(), v));
            }
        }
        (h, arrs)
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let (h, _) = self.call_bin(method, params, &[]);
        if let Some(e) = h.get("error") {
            panic!("{method}: {e}");
        }
        h["result"].clone()
    }

    fn error(&mut self, method: &str, params: Value) -> String {
        let (h, _) = self.call_bin(method, params, &[]);
        h["error"]["message"]
            .as_str()
            .expect("エラーになるはず")
            .to_string()
    }

    fn stop(mut self) {
        self.call("shutdown", json!({}));
        let st = self.child.wait().unwrap();
        assert!(st.success());
    }
}

impl Drop for Server {
    /// 失敗したテストでもサーバーを残さない
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-14 * (1.0 + a.abs().max(b.abs()))
}

fn ms(t: Instant, n: usize) -> f64 {
    t.elapsed().as_secs_f64() * 1e3 / n as f64
}

/// 完了条件: 外部プロセスから系を開き、間隔を変えて再解析を繰り返す。結果は同じ値で読み直した系と一致する
#[test]
fn loop_thickness() {
    let mut sv = Server::start(&[]);
    // 系がないうちは解析できない
    assert!(sv
        .error("analyze", json!({ "cmd": "spot" }))
        .contains("load"));
    let sum = sv.call("load", json!({ "file": ZMX, "glass_dir": ["tests/glass"] }));
    assert_eq!(sum["image_surface"], 9);
    let base = core_system(&Path::new(ROOT).join(ZMX));
    let p = base.param("3.t").unwrap();
    let t0 = sv.call("get", json!({ "param": "3.t" }))["value"]
        .as_f64()
        .unwrap();
    assert_eq!(t0, base.get_param(p).unwrap());

    let t = Instant::now();
    let n = 25;
    for k in 0..n {
        let v = t0 + 0.1 * (k as f64 - 12.0);
        sv.call("set", json!({ "values": { "3.t": v } }));
        assert_eq!(
            sv.call("get", json!({ "params": ["3.t"] }))["values"][0],
            json!(v)
        );
        let r = sv.call("analyze", json!({ "cmd": "spot", "field": 3 }));
        let got = r["spot"][0]["rms_centroid"].as_f64().unwrap();
        let s = base.with_params(&[(p, v)]).unwrap();
        let want = analysis::spot(&s, FieldSel::Index(2), None, 6)
            .unwrap()
            .rms_centroid;
        assert!(close(got, want), "t = {v}: {got} {want}");
    }
    eprintln!("set + get + spot の 1 往復: {:.3} ms", ms(t, n));

    // 解析の引数は CLI と同じ
    let r = sv.call(
        "analyze",
        json!({ "cmd": "wavefront", "field": 2, "wave": 2, "n": 32 }),
    );
    let wf = analysis::wavefront(
        &base.with_params(&[(p, t0 + 1.2)]).unwrap(),
        FieldSel::Index(1),
        Some(1),
        32,
        false,
    )
    .unwrap();
    assert!(close(r["wavefront"][0]["rms"].as_f64().unwrap(), wf.rms));
    assert!(r["wavefront"][0]["map"].is_null());
    // map: 波面の格子（GUI の波面図）。瞳の外は null
    let r = sv.call(
        "analyze",
        json!({ "cmd": "wavefront", "field": 2, "wave": 2, "n": 32, "map": true }),
    );
    let m = r["wavefront"][0]["map"].as_array().unwrap();
    assert_eq!(m.len(), 32);
    assert_eq!(r["wavefront"][0]["p"].as_array().unwrap().len(), 32);
    for (iy, row) in m.iter().enumerate() {
        for (ix, x) in row.as_array().unwrap().iter().enumerate() {
            match x.as_f64() {
                Some(v) => assert!(close(v, wf.w[iy][ix])),
                None => assert!(wf.w[iy][ix].is_nan()),
            }
        }
    }
    assert!(sv
        .error("analyze", json!({ "cmd": "spot", "cfg": 2 }))
        .contains("load"));
    assert!(sv
        .error("analyze", json!({ "cmd": "catalog-list" }))
        .contains("解析ではない"));
    // 解析の値の誤りは文だけ（「不正: 」を付けない・重ねない）
    let e = sv.error("analyze", json!({ "cmd": "mech-check" }));
    assert!(e.starts_with("機構部品がない"), "{e}");
    let e = sv.error("analyze", json!({ "cmd": "fringe", "beam1": "9" }));
    assert!(!e.contains("不正"), "{e}");
    assert!(!sv
        .error("set", json!({ "values": { "99.t": 1.0 } }))
        .is_empty());
    assert!(sv.error("nothing", json!({})).contains("nothing"));

    // reload はファイルの値に戻す
    sv.call("reload", json!({}));
    assert_eq!(
        sv.call("get", json!({ "param": "3.t" }))["value"]
            .as_f64()
            .unwrap(),
        t0
    );
    sv.stop();
}

/// undo・redo は set・edit を 1 段ずつ戻す・やり直す。同じパラメータの set が続けば 1 段、load で空になる
#[test]
fn undo_redo_history() {
    let mut sv = Server::start(&[ZMX, "--glass-dir", "tests/glass"]);
    let get = |sv: &mut Server, p: &str| {
        sv.call("get", json!({ "param": p }))["value"]
            .as_f64()
            .unwrap()
    };
    let (t3, t2) = (get(&mut sv, "3.t"), get(&mut sv, "2.t"));
    assert!(sv.error("undo", json!({})).contains("元に戻す"));
    // 同じ名前の set が続けば 1 段にまとめる
    sv.call("set", json!({ "values": { "3.t": t3 + 1.0 } }));
    let r = sv.call("set", json!({ "values": { "3.t": t3 + 2.0 } }));
    assert_eq!((r["undo"].as_u64(), r["redo"].as_u64()), (Some(1), Some(0)));
    let r = sv.call(
        "edit",
        json!({ "ops": [{ "op": "set_surface", "surface": 2, "fields": { "t": t2 + 0.5 } }] }),
    );
    assert_eq!(r["undo"].as_u64(), Some(2));
    // 段の一覧（新しい順）: edit の操作と set の名前
    let h = sv.call("history", json!({}));
    assert_eq!(
        h["undo_list"][0]["edit"][0],
        json!({ "op": "set_surface", "surface": 2, "fields": ["t"] })
    );
    assert_eq!(h["undo_list"][1], json!({ "set": ["3.t"] }));
    // steps でまとめて戻す・やり直す
    let r = sv.call("undo", json!({ "steps": 2 }));
    assert_eq!((r["undo"].as_u64(), r["redo"].as_u64()), (Some(0), Some(2)));
    assert_eq!(r["redo_list"][0], json!({ "set": ["3.t"] }));
    assert_eq!(get(&mut sv, "3.t"), t3);
    sv.call("redo", json!({ "steps": 2 }));
    assert_eq!(get(&mut sv, "2.t"), t2 + 0.5);
    let r = sv.call("undo", json!({}));
    assert_eq!((r["undo"].as_u64(), r["redo"].as_u64()), (Some(1), Some(1)));
    assert_eq!(get(&mut sv, "2.t"), t2);
    assert_eq!(get(&mut sv, "3.t"), t3 + 2.0);
    sv.call("undo", json!({}));
    assert_eq!(get(&mut sv, "3.t"), t3);
    assert!(sv.error("undo", json!({})).contains("元に戻す"));
    sv.call("redo", json!({}));
    sv.call("redo", json!({}));
    assert_eq!(
        (get(&mut sv, "3.t"), get(&mut sv, "2.t")),
        (t3 + 2.0, t2 + 0.5)
    );
    assert!(sv.error("redo", json!({})).contains("やり直す"));
    // 戻した後に変えるとやり直す段は消える
    sv.call("undo", json!({}));
    let r = sv.call("set", json!({ "values": { "2.t": t2 + 0.25 } }));
    assert_eq!((r["undo"].as_u64(), r["redo"].as_u64()), (Some(2), Some(0)));
    sv.call("reload", json!({}));
    let h = sv.call("history", json!({}));
    assert_eq!((h["undo"].as_u64(), h["redo"].as_u64()), (Some(0), Some(0)));

    // history: "push" の開く・読み直すは 1 段（戻すと前の系・変えた値・前の段に戻る）、"keep" は段を変えない
    sv.call("set", json!({ "values": { "3.t": t3 + 1.0 } }));
    let r = sv.call(
        "load",
        json!({ "file": "tests/refs/A1/system.zmx", "glass_dir": ["tests/glass"], "history": "push" }),
    );
    assert_eq!(r["undo"].as_u64(), Some(2));
    assert_eq!(r["undo_list"][0], json!({ "open": "system.zmx" }));
    let r = sv.call("undo", json!({}));
    assert!(r["loaded"]["file"]
        .as_str()
        .unwrap()
        .ends_with("A0/system.zmx"));
    assert_eq!(get(&mut sv, "3.t"), t3 + 1.0);
    let r = sv.call("redo", json!({}));
    assert!(r["loaded"]["file"].as_str().unwrap().contains("A1"));
    sv.call("undo", json!({}));
    let r = sv.call("reload", json!({ "history": "push" }));
    assert_eq!(r["undo_list"][0], json!({ "reload": "system.zmx" }));
    assert_eq!(get(&mut sv, "3.t"), t3);
    sv.call("undo", json!({}));
    assert_eq!(get(&mut sv, "3.t"), t3 + 1.0);
    let r = sv.call(
        "load",
        json!({ "file": ZMX, "glass_dir": ["tests/glass"], "history": "keep" }),
    );
    assert_eq!((r["undo"].as_u64(), r["redo"].as_u64()), (Some(1), Some(1)));
    assert!(sv
        .error("load", json!({ "file": ZMX, "history": "x" }))
        .contains("history"));
    // 無いファイルは誤りの種類 io（XCT-06）
    let (h, _) = sv.call_bin("load", json!({ "file": "nosuch.zmx" }), &[]);
    assert_eq!(h["error"]["data"]["kind"], "io", "{h}");
    sv.stop();
}

/// trace は配列をバイナリで受け渡し、コアの追跡とビット一致する。往復の時間を測る
#[test]
fn trace_binary_and_timing() {
    let mut sv = Server::start(&[ZMX, "--glass-dir", "tests/glass"]);
    let s = core_system(&Path::new(ROOT).join(ZMX));
    let n = 100_000;
    let (px, py): (Vec<f64>, Vec<f64>) = (0..n)
        .map(|i| {
            let a = i as f64 * 2.399963;
            let r = ((i as f64 + 0.5) / n as f64).sqrt();
            (r * a.cos(), r * a.sin())
        })
        .unzip();
    sv.call(
        "set",
        json!({ "values": { "5.t": s.get_param(s.param("5.t").unwrap()).unwrap() + 0.05 } }),
    );
    let s = s
        .with_params(&[(
            s.param("5.t").unwrap(),
            s.get_param(s.param("5.t").unwrap()).unwrap() + 0.05,
        )])
        .unwrap();
    let t = Instant::now();
    let (h, arr) = sv.call_bin(
        "trace",
        json!({ "field": 4, "wave": 1 }),
        &[("px", &px), ("py", &py)],
    );
    let t_trace = ms(t, 1);
    assert_eq!(h["result"]["rays"], json!(n));
    let (hx, hy) = s.hxhy(3);
    let launch = Launch::Pupil {
        spec: s.object_spec(hx, hy).unwrap(),
        signed: false,
        px: &px,
        py: &py,
        vig: Some(s.vig_factors(hx, hy).unwrap()),
        hxy: Some((hx, hy)),
    };
    let tr = s.trace_launch(&launch, 0, None, false);
    let get = |k: &str| &arr.iter().find(|(n, _)| n == k).unwrap().1;
    for (i, r) in tr.rays.iter().enumerate() {
        for (k, want) in [
            ("x", r.xyz[0]),
            ("y", r.xyz[1]),
            ("l", r.lmn[0]),
            ("n", r.lmn[2]),
            ("opl", r.opl),
        ] {
            let g = get(k)[i];
            assert!(
                g.to_bits() == want.to_bits() || (g.is_nan() && want.is_nan()),
                "{k}[{i}] {g} {want}"
            );
        }
        assert_eq!(get("vig")[i], r.vig as f64);
        assert_eq!(get("err")[i], r.err as f64);
    }
    assert_eq!(
        h["result"]["ok"],
        json!(tr.rays.iter().filter(|r| r.err == 0 && r.vig == 0).count())
    );
    // JSON の配列でも渡せる
    let (h, arr) = sv.call_bin(
        "trace",
        json!({ "hy": 0.5, "px": [0.0, 0.3], "py": [0.0, 0.3], "global": true }),
        &[],
    );
    assert_eq!(h["result"]["frame"], "global");
    assert_eq!(arr[0].1.len(), 2);

    // 往復の時間
    let k = 200;
    let t = Instant::now();
    for _ in 0..k {
        sv.call("ping", json!({}));
    }
    let t_ping = ms(t, k);
    let big: Vec<f64> = (0..500_000).map(|i| i as f64).collect(); // 4 MB
    let k = 20;
    let t = Instant::now();
    for _ in 0..k {
        let (_, a) = sv.call_bin("echo", json!({}), &[("a", &big)]);
        assert_eq!(a[0].1.len(), big.len());
    }
    let t_echo = ms(t, k);
    let t = Instant::now();
    let k = 50;
    for _ in 0..k {
        sv.call("analyze", json!({ "cmd": "spot", "field": 1 }));
    }
    let t_spot = ms(t, k);
    eprintln!(
        "往復: ping {t_ping:.3} ms, echo 4 MB {t_echo:.2} ms, spot {t_spot:.3} ms, trace {n} 本（入 1.6 MB・出 7.2 MB）{t_trace:.2} ms"
    );
    sv.stop();
}

fn wait_for(p: &Path, f: impl Fn(&Value) -> bool) -> Value {
    let t = Instant::now();
    loop {
        if let Ok(s) = std::fs::read_to_string(p) {
            if let Ok(v) = serde_json::from_str::<Value>(&s) {
                if f(&v) {
                    return v;
                }
            }
        }
        assert!(
            t.elapsed() < Duration::from_secs(60),
            "{} を待ちきれない",
            p.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// watch: 系のファイルが書き換わるたびに再解析して結果を書く
#[test]
fn watch_file() {
    let dir: PathBuf = std::env::temp_dir().join(format!("lensbench-watch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let zmx_path = dir.join("sys.zmx");
    let base = core_system(&Path::new(ROOT).join(ZMX));
    zmx_write::save(&zmx_path, &base).unwrap();
    let out = dir.join("out.json");
    let status = dir.join("status.json");
    let g = Path::new(ROOT).join("tests/glass");
    let mut child = exe()
        .args(["watch", "--interval", "0.05", "--max-runs", "2", "--status"])
        .arg(&status)
        .args(["--", "spot"])
        .arg(&zmx_path)
        .arg("--glass-dir")
        .arg(&g)
        .args(["--field", "3", "--save"])
        .arg(&out)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for(&status, |v| v["run"] == 1);
    let first: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    // 別アプリが系を書き換える
    let p = base.param("3.t").unwrap();
    let changed = base
        .with_params(&[(p, base.get_param(p).unwrap() + 0.5)])
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    zmx_write::save(&zmx_path, &changed).unwrap();
    let st = wait_for(&status, |v| v["run"] == 2);
    assert_eq!(st["ok"], true, "{st}");
    assert!(child.wait().unwrap().success());
    let mut so = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut so)
        .unwrap();
    assert_eq!(so.lines().count(), 2, "{so}");
    let second: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let want = |s: &System| {
        analysis::spot(s, FieldSel::Index(2), None, 6)
            .unwrap()
            .rms_centroid
    };
    let rms = |v: &Value| v["spot"][0]["rms_centroid"].as_f64().unwrap();
    let reread = core_system(&zmx_path);
    assert!(close(rms(&first), want(&base)));
    assert!(close(rms(&second), want(&reread)));
    assert!((rms(&first) - rms(&second)).abs() > 1e-6);
    std::fs::remove_dir_all(&dir).ok();
}

/// 要求をヘッダだけ送り、応答のヘッダを読む（配列のバイト列は送らない）
fn send_head(sv: &mut Server, h: Value) -> Value {
    let mut out = serde_json::to_vec(&h).unwrap();
    out.push(b'\n');
    sv.w.write_all(&out).unwrap();
    let mut line = String::new();
    sv.r.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

/// 配列の大きさは上限を確かめてから確保する。超えたら断って接続を閉じる
#[test]
fn request_size_is_limited() {
    let mut sv = Server::start(&["--max-request-bytes", "1000"]);
    // 上限の内なら通る
    let small = vec![1.0; 100];
    let (h, a) = sv.call_bin("echo", json!({}), &[("a", &small)]);
    assert!(h.get("error").is_none(), "{h}");
    assert_eq!(a[0].1, small);
    let h = send_head(
        &mut sv,
        json!({ "jsonrpc": "2.0", "id": 9, "method": "echo",
                "bin": [{ "name": "a", "dtype": "<f8", "shape": [1000] }] }),
    );
    assert!(
        h["error"]["message"]
            .as_str()
            .unwrap()
            .contains("大きすぎる"),
        "{h}"
    );
    let mut rest = String::new();
    assert_eq!(sv.r.read_line(&mut rest).unwrap(), 0, "接続は閉じる");
    // 積があふれる形も断る（新しい接続で）
    let mut sv = Server::start(&[]);
    let h = send_head(
        &mut sv,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "echo",
                "bin": [{ "name": "a", "dtype": "<f8", "shape": [1u64 << 40, 1u64 << 40] }] }),
    );
    assert!(
        h["error"]["message"]
            .as_str()
            .unwrap()
            .contains("大きすぎる"),
        "{h}"
    );
}

/// --allow-remote では要求でファイルを読み書きしない（起動時に与えた系は読める）
#[test]
fn remote_does_not_touch_files() {
    let mut sv = Server::start(&["--allow-remote", ZMX, "--glass-dir", "tests/glass"]);
    let deny = |m: String| assert!(m.contains("--allow-remote"), "{m}");
    deny(sv.error("load", json!({ "file": ZMX, "glass_dir": ["tests/glass"] })));
    deny(sv.error("write", json!({ "path": "x.zmx" })));
    deny(sv.error("analyze", json!({ "cmd": "spot", "save": "out.json" })));
    deny(sv.error("layout_export", json!({ "file": "a.svg" })));
    // 差し替え探索のプロジェクトは中のパスを確かめられないので受けない
    deny(sv.error(
        "swap",
        json!({ "slots": [{"block": "A"}], "project": {"blocks": [{"type": "lens", "id": "A", "source": {"zmx": "/etc/x.zmx"}}]} }),
    ));
    assert!(!Path::new(ROOT).join("x.zmx").exists());
    // ファイルを使わない解析と、起動時の読み方での reload はできる
    sv.call("analyze", json!({ "cmd": "spot", "field": 1 }));
    sv.call("reload", json!({}));
    sv.stop();
}

/// --root の下だけ読み書きする（相対パスは --root から）
#[test]
fn root_limits_files() {
    let dir = std::env::temp_dir().join(format!("lensbench-root-{}", std::process::id()));
    let root = dir.join("root");
    std::fs::create_dir_all(root.join("glass")).unwrap();
    std::fs::copy(Path::new(ROOT).join(ZMX), root.join("sys.zmx")).unwrap();
    for g in ["OHARA.AGF", "SCHOTT.AGF"] {
        std::fs::copy(
            Path::new(ROOT).join("tests/glass").join(g),
            root.join("glass").join(g),
        )
        .unwrap();
    }
    let r = root.display().to_string();
    let mut sv = Server::start(&["--allow-remote", "--root", &r]);
    sv.call("load", json!({ "file": "sys.zmx", "glass_dir": ["glass"] }));
    // 外のパス、または無いファイル・フォルダ（OS のエラー）
    let out = |m: String| assert!(m.contains("--root の外") || m.contains("os error"), "{m}");
    let outside = |m: String| assert!(m.contains("--root の外"), "{m}");
    // 外のファイル・フォルダは読まない
    outside(sv.error(
        "load",
        json!({ "file": Path::new(ROOT).join(ZMX).display().to_string() }),
    ));
    outside(sv.error("load", json!({ "file": "sys.zmx", "glass_dir": [Path::new(ROOT).join("tests/glass").display().to_string()] })));
    // 無いファイルは読めない（外か内かを明かさない）
    out(sv.error("load", json!({ "file": "../root/../../x.zmx" })));
    // 書き出しも下だけ
    sv.call("write", json!({ "path": "copy.zmx" }));
    assert!(root.join("copy.zmx").exists());
    outside(sv.error("write", json!({ "path": "../escape.zmx" })));
    outside(sv.error(
        "write",
        json!({ "path": dir.join("escape2.zmx").display().to_string() }),
    ));
    assert!(!dir.join("escape.zmx").exists() && !dir.join("escape2.zmx").exists());
    out(sv.error("write", json!({ "path": "nofolder/x.zmx" })));
    sv.call("analyze", json!({ "cmd": "spot", "save": "spot.json" }));
    assert!(root.join("spot.json").exists());
    outside(sv.error("analyze", json!({ "cmd": "spot", "save": "../spot.json" })));
    // 評価関数の zplm のマクロも下だけ。既定の場所（LENSBENCH_MACROS・今のフォルダ）は探さない
    std::fs::write(root.join("ZPL01.ZPL"), "OPTRETURN 0, 1\n").unwrap();
    std::fs::write(dir.join("ZPL02.ZPL"), "OPTRETURN 0, 1\n").unwrap();
    let term = |t: Value| json!({ "vars": [], "merit": "none", "terms": [t], "iters": 0 });
    sv.call(
        "optimize",
        term(json!({"type": "zplm", "file": "ZPL01.ZPL"})),
    );
    sv.call(
        "optimize",
        term(json!({"type": "zplm", "macro": 1, "macro_dirs": ["."]})),
    );
    let zp = dir.join("ZPL02.ZPL").display().to_string();
    outside(sv.error("optimize", term(json!({"type": "zplm", "file": zp}))));
    let d = dir.display().to_string();
    outside(sv.error(
        "optimize",
        term(json!({"type": "zplm", "macro": 2, "macro_dirs": [d]})),
    ));
    let e = sv.error("optimize", term(json!({"type": "zplm", "macro": 1})));
    assert!(e.contains("見つからない"), "{e}");
    let tol = json!({ "trials": 1, "merit": "none", "tols": [{"param": "1.t", "pm": 0.01}],
        "terms": [{"type": "zplm", "file": zp}] });
    outside(sv.error("tolerance", tol));
    // optimize・tolerance・layout は analyze では受けない（専用のメソッドで）
    for c in ["optimize", "tolerance", "layout"] {
        let e = sv.error("analyze", json!({ "cmd": c }));
        assert!(e.contains("解析ではない"), "{e}");
    }
    sv.stop();
    std::fs::remove_dir_all(&dir).ok();
}

/// 系の編集と系ファイル: edit で面を足して save し、読み直した系の解析が同じ
#[test]
fn edit_and_save() {
    let dir = std::env::temp_dir().join(format!("lb_serve_edit_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut sv = Server::start(&[]);
    sv.call("load", json!({ "file": ZMX, "glass_dir": ["tests/glass"] }));
    let n0 = sv.call("surface", json!({}))["surfaces"]
        .as_array()
        .unwrap()
        .len();
    let r = sv.call(
        "edit",
        json!({ "ops": [
            {"op": "insert_surface", "at": 1, "surface": {"t": 0.0, "comment": "D"}},
            {"op": "set_surface", "surface": "D", "fields": {"semi": 4.0}},
        ]}),
    );
    assert_eq!(r["surfaces"].as_u64().unwrap() as usize, n0 + 1);
    let f = sv.call("surface", json!({ "surface": "D" }));
    assert_eq!(f["surface"], json!(1));
    assert_eq!(f["fields"]["semi"], json!(4.0));
    // 失敗する操作の列は何も変えない
    let e = sv.error(
        "edit",
        json!({ "ops": [{"op": "delete_surface", "surface": "D"}, {"op": "nope"}] }),
    );
    assert!(e.contains("操作 2"), "{e}");
    assert_eq!(
        sv.call("surface", json!({ "surface": "D" }))["surface"],
        json!(1)
    );
    let spot = |sv: &mut Server| sv.call("analyze", json!({ "cmd": "spot", "field": 2 }));
    let a = spot(&mut sv);
    let p = dir.join("edited.json");
    let w = sv.call("save", json!({ "path": p.to_str().unwrap() }));
    assert_eq!(w["system_hash"], r["system_hash"]);
    sv.call("load", json!({ "file": p.to_str().unwrap() }));
    let b = spot(&mut sv);
    assert_eq!(a, b);
    // 選んだ範囲（面 2〜3）だけの書き出し: 物体面・像面と 2 面
    let curv = |sv: &mut Server, j: usize| {
        sv.call("surface", json!({ "surface": j }))["fields"]["c"].clone()
    };
    let (c2, c3) = (curv(&mut sv, 2), curv(&mut sv, 3));
    let q = dir.join("range.json");
    let w = sv.call(
        "save_range",
        json!({ "path": q.to_str().unwrap(), "from": 2, "to": 3 }),
    );
    assert_eq!(w["surfaces"], json!(4));
    sv.call("load", json!({ "file": q.to_str().unwrap() }));
    assert_eq!((curv(&mut sv, 1), curv(&mut sv, 2)), (c2, c3));
    assert_eq!(
        sv.call("surface", json!({}))["surfaces"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert!(sv
        .error(
            "save_range",
            json!({ "path": q.to_str().unwrap(), "from": 0, "to": 1 })
        )
        .contains("書き出す面"));
    sv.stop();
    std::fs::remove_dir_all(&dir).unwrap();
}

/// layout: 玉の断面・光線・面の頂点・玉の範囲（GUI のレイアウト図）
#[test]
fn layout_of_combo() {
    let mut sv = Server::start(&[]);
    sv.call(
        "load",
        json!({
            "file": Path::new(ROOT).join("examples/projects/P8_catalog_two.json").display().to_string(),
            "glass_dir": [Path::new(ROOT).join("tests/glass").display().to_string()],
            "lens_catalog": [Path::new(ROOT).join("examples/lenses/LB").display().to_string()],
        }),
    );
    let v = sv.call("layout", json!({ "rays": 5 }));
    let solids = v["solids"].as_array().unwrap();
    assert!(solids.len() >= 2, "{v}");
    assert!(solids
        .iter()
        .all(|s| s["pts"].as_array().unwrap().len() > 4));
    assert!(v["plates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["kind"] == "image"));
    let surfaces = v["surfaces"].as_array().unwrap();
    assert_eq!(surfaces.iter().filter(|s| s["stop"] == true).count(), 1);
    // 視野 4 × 光線 5（主波長）
    assert_eq!(v["rays"].as_array().unwrap().len(), 20);
    // 玉の範囲は面の番号
    for u in v["units"].as_array().unwrap() {
        for sp in u["spans"].as_array().unwrap() {
            let (a, b) = (
                sp[0].as_u64().unwrap() as usize,
                sp[1].as_u64().unwrap() as usize,
            );
            assert!(a <= b && b < surfaces.len(), "{u}");
        }
    }
    // 範囲外の視野はエラー
    assert!(sv.error("layout", json!({ "field": 99 })).contains("field"));
    sv.stop();
}

/// layout_export: 拡張子の形式で書き出す（GUI のレイアウトの書き出し）。2D・3D・STEP、重ね描き
#[test]
fn layout_export_writes_formats() {
    let dir = std::env::temp_dir().join(format!("lensbench-lexp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut sv = Server::start(&[]);
    sv.call(
        "load",
        json!({
            "file": Path::new(ROOT).join("examples/projects/P16_twyman_green.json").display().to_string(),
            "glass_dir": [Path::new(ROOT).join("tests/glass").display().to_string()],
        }),
    );
    for (name, extra, fmt) in [
        ("a.svg", json!({ "plane": "3d" }), "svg"),
        ("a.png", json!({ "unfold": true }), "png"),
        ("a.glb", json!({ "unfold": true }), "glb"),
        (
            "a.step",
            json!({ "paths": "all", "offset": [0, 0, 0] }),
            "step",
        ),
    ] {
        let f = dir.join(name);
        let mut p = extra.clone();
        p["file"] = json!(f.display().to_string());
        let v = sv.call("layout_export", p);
        assert_eq!(v["format"], fmt, "{v}");
        assert!(v["solids"].as_u64().unwrap() >= 2, "{v}");
        assert!(std::fs::metadata(&f).unwrap().len() > 100, "{name}");
    }
    let svg = std::fs::read_to_string(dir.join("a.svg")).unwrap();
    assert!(
        svg.starts_with("<?xml") || svg.starts_with("<svg"),
        "{}",
        &svg[..40]
    );
    assert!(std::fs::read_to_string(dir.join("a.step"))
        .unwrap()
        .contains("ISO-10303-21"));
    assert!(sv
        .error(
            "layout_export",
            json!({ "file": dir.join("a.txt").display().to_string() })
        )
        .contains("拡張子"));
    assert!(sv.error("layout_export", json!({})).contains("file"));
    sv.stop();
    std::fs::remove_dir_all(&dir).unwrap();
}

/// preview_layout: 読み込んだ系を変えずに、load＋layout と同じ図を返す（GUI の下書き）
#[test]
fn preview_layout_matches_load_and_layout() {
    let load = json!({
        "file": Path::new(ROOT).join("examples/projects/P8_catalog_two.json").display().to_string(),
        "glass_dir": [Path::new(ROOT).join("tests/glass").display().to_string()],
        "lens_catalog": [Path::new(ROOT).join("examples/lenses/LB").display().to_string()],
    });
    let pre = lensbench_engine::preview_layout(&load, &json!({ "rays": 5 })).unwrap();
    lensbench_engine::call("load", &load, None).unwrap();
    let v = lensbench_engine::call("layout", &json!({ "rays": 5 }), None).unwrap();
    assert_eq!(pre, v);
    assert!(lensbench_engine::preview_layout(&json!({}), &json!({})).is_err());
}

/// layout の見る面: xz 面は [z, x]。回転対称な玉の断面は yz 面と同じで、光線の扇の既定は x
#[test]
fn layout_plane_xz() {
    let load = json!({
        "file": Path::new(ROOT).join("examples/projects/P8_catalog_two.json").display().to_string(),
        "glass_dir": [Path::new(ROOT).join("tests/glass").display().to_string()],
        "lens_catalog": [Path::new(ROOT).join("examples/lenses/LB").display().to_string()],
    });
    let yz = lensbench_engine::preview_layout(&load, &json!({ "rays": 5 })).unwrap();
    let xz = lensbench_engine::preview_layout(&load, &json!({ "rays": 5, "plane": "xz" })).unwrap();
    assert_eq!(yz["plane"], "yz");
    assert_eq!(xz["plane"], "xz");
    let close = |a: &Value, b: &Value| {
        let (a, b) = (a.as_array().unwrap(), b.as_array().unwrap());
        a.len() == b.len()
            && a.iter().zip(b).all(|(p, q)| {
                (0..2).all(|i| (p[i].as_f64().unwrap() - q[i].as_f64().unwrap()).abs() < 1e-9)
            })
    };
    for (a, b) in yz["solids"]
        .as_array()
        .unwrap()
        .iter()
        .zip(xz["solids"].as_array().unwrap())
    {
        assert!(close(&a["pts"], &b["pts"]), "{a} {b}");
    }
    assert_eq!(yz["surfaces"], xz["surfaces"]);
    // 視野 1〜3 は y だけ（主光線は xz 面では縦 0）、視野 4 は x 1.5° で縦が x に出る
    let rays = xz["rays"].as_array().unwrap();
    assert_eq!(rays.len(), yz["rays"].as_array().unwrap().len());
    let end_x = |r: &Value| {
        r["pts"].as_array().unwrap().last().unwrap()[1]
            .as_f64()
            .unwrap()
    };
    for r in rays.iter().filter(|r| r["px"] == 0.0 && r["py"] == 0.0) {
        if r["field"] == 4 {
            assert!(end_x(r) > 0.1, "{r}");
        } else {
            assert!(end_x(r).abs() < 1e-9, "{r}");
        }
    }
    assert!(lensbench_engine::preview_layout(&load, &json!({ "plane": "xy" })).is_err());
}

/// layout の重ね描き: configs で BS の経路を知り、paths で重ねる。ずらすと構成 k の光線は k 倍ずれる
#[test]
fn layout_overlay_paths() {
    let load = json!({
        "file": Path::new(ROOT).join("examples/projects/P16_twyman_green.json").display().to_string(),
        "glass_dir": [Path::new(ROOT).join("tests/glass").display().to_string()],
    });
    let mut sv = Server::start(&[]);
    sv.call("load", load);
    let c = sv.call("configs", json!({}));
    let paths: Vec<String> = c["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert!(paths.len() >= 2, "{c}");
    let one = sv.call("layout", json!({ "rays": 3 }));
    assert_eq!(one["configs"], json!([]));
    let v = sv.call("layout", json!({ "rays": 3, "paths": "all" }));
    assert_eq!(v["configs"], json!(paths));
    let rays = v["rays"].as_array().unwrap();
    let ncfg = rays.iter().map(|r| r["cfg"].as_u64().unwrap()).max();
    assert_eq!(ncfg, Some(paths.len() as u64 - 1));
    // 並びで 2 つ選び、z に 50 mm ずらす
    let two = json!([paths[0], paths[1]]);
    let a = sv.call("layout", json!({ "rays": 3, "paths": two }));
    let b = sv.call(
        "layout",
        json!({ "rays": 3, "paths": two, "offset": [0, 0, 50] }),
    );
    let ra = a["rays"].as_array().unwrap();
    let rb = b["rays"].as_array().unwrap();
    assert_eq!(ra.len(), rb.len());
    for (x, y) in ra.iter().zip(rb) {
        let k = x["cfg"].as_f64().unwrap();
        let dz = y["pts"][0][0].as_f64().unwrap() - x["pts"][0][0].as_f64().unwrap();
        assert!((dz - 50.0 * k).abs() < 1e-9, "{dz} {k}");
    }
    // プロジェクトに .zmx の構成は無い・offset は 3 つ
    assert!(sv
        .error("layout", json!({ "cfgs": "all" }))
        .contains("cfgs"));
    assert!(sv
        .error("layout", json!({ "paths": "all", "offset": [1, 2] }))
        .contains("offset"));
    // 読み込んだ後の set は、ファイルから読み直した経路にも当てる
    let before = sv.call("layout", json!({ "rays": 3, "paths": [paths[0].clone()] }));
    let t = sv.call("get", json!({ "param": "1.t" }))["value"]
        .as_f64()
        .unwrap();
    sv.call("set", json!({ "values": { "1.t": t + 3.0 } }));
    let after = sv.call("layout", json!({ "rays": 3, "paths": [paths[0].clone()] }));
    assert_ne!(before["rays"], after["rays"]);
    assert!(after.get("notes").is_none(), "{}", after["notes"]);
    sv.stop();
}

/// preview_layout でも構成を重ねられる（GUI の組合せの図）
#[test]
fn preview_layout_overlay() {
    let load = json!({
        "file": Path::new(ROOT).join("examples/projects/P16_twyman_green.json").display().to_string(),
        "glass_dir": [Path::new(ROOT).join("tests/glass").display().to_string()],
    });
    let v = lensbench_engine::preview_layout(&load, &json!({ "rays": 3, "paths": "all" })).unwrap();
    assert!(
        v["configs"].as_array().unwrap().len() >= 2,
        "{}",
        v["configs"]
    );
}

/// layout の展開: 折り曲げの系で、展開した図では光線が光軸の向きへだけ進み（z が減らない）、
/// 立体・面の数は同じ。3D（plane "3d"）は立体と面の網（面の番号つき）と 3 次元の光線
#[test]
fn layout_unfold_and_3d() {
    let load = json!({
        "file": Path::new(ROOT).join("examples/projects/P3_fold.json").display().to_string(),
        "glass_dir": [Path::new(ROOT).join("tests/glass").display().to_string()],
    });
    let call = |o: Value| lensbench_engine::preview_layout(&load, &o);
    let yz = call(json!({ "rays": 5 })).unwrap();
    let un = call(json!({ "rays": 5, "unfold": true })).unwrap();
    assert_eq!(un["unfold"], true);
    assert_eq!(
        yz["solids"].as_array().unwrap().len(),
        un["solids"].as_array().unwrap().len()
    );
    let forward = |v: &Value| {
        v["rays"].as_array().unwrap().iter().all(|r| {
            let z: Vec<f64> = r["pts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p[0].as_f64().unwrap())
                .collect();
            z.windows(2).all(|w| w[1] >= w[0] - 1e-9)
        })
    };
    assert!(forward(&un));
    // 縦の広がり: 折れた図では光路が縦へ伸びる、展開した図では光束の太さだけ
    let span = |v: &Value| {
        let y: Vec<f64> = v["rays"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| {
                r["pts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p[1].as_f64().unwrap())
            })
            .collect();
        y.iter().cloned().fold(f64::MIN, f64::max) - y.iter().cloned().fold(f64::MAX, f64::min)
    };
    assert!(span(&yz) > 3.0 * span(&un), "{} {}", span(&yz), span(&un));
    // 面の頂点も展開した位置（光軸の上、z は増えていく）
    let vz: Vec<[f64; 2]> = un["surfaces"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| Some([s["vertex"][0].as_f64()?, s["vertex"][1].as_f64()?]))
        .collect();
    assert!(vz.iter().all(|v| v[1].abs() < 1e-9), "{vz:?}");
    assert!(vz.windows(2).all(|w| w[1][0] >= w[0][0] - 1e-9), "{vz:?}");

    let d3 = call(json!({ "plane": "3d" })).unwrap();
    assert_eq!(d3["plane"], "3d");
    let ms = d3["meshes"].as_array().unwrap();
    assert!(ms.iter().any(|m| m["kind"] == "mirror"));
    assert!(ms.iter().any(|m| m["kind"] == "image"));
    for m in ms {
        let np = m["pos"].as_array().unwrap().len();
        assert_eq!(np % 3, 0);
        assert!(m["tris"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| (i.as_u64().unwrap() as usize) < np / 3));
        assert_eq!(m["surfaces"].as_array().unwrap().len(), 2);
    }
    let r = d3["rays"].as_array().unwrap();
    assert!(!r.is_empty() && r.iter().all(|r| r["pts"][0].as_array().unwrap().len() == 3));
    assert!(d3["surfaces"][1]["vertex"].as_array().unwrap().len() == 3);
    // xz 面でも展開できる（縦は x）
    let ux = call(json!({ "plane": "xz", "unfold": true })).unwrap();
    assert_eq!(
        (ux["plane"].as_str(), ux["unfold"].as_bool()),
        (Some("xz"), Some(true))
    );
    assert!(forward(&ux));
    assert!(call(json!({ "plane": "3d", "unfold": true })).is_err());
    assert!(call(json!({ "unfold": 1 })).is_err());
}

/// layout: 機構部品の断面（2D の四角形）と 3D の網（kind "mech"）も返す
#[test]
fn layout_has_mechanics() {
    let load = json!({
        "file": Path::new(ROOT).join("examples/projects/P19_tube_cage.json").display().to_string(),
        "glass_dir": [Path::new(ROOT).join("tests/glass").display().to_string()],
    });
    let call = |o: Value| lensbench_engine::preview_layout(&load, &o).unwrap();
    let v = call(json!({ "rays": 3 }));
    let ms = v["mechs"].as_array().unwrap();
    let names: Vec<&str> = ms.iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["tube", "cage"]);
    for m in ms {
        let q = m["quads"].as_array().unwrap();
        assert!(!q.is_empty());
        assert!(q.iter().all(|q| q.as_array().unwrap().len() == 4));
    }
    let d3 = call(json!({ "plane": "3d" }));
    let n = d3["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["kind"] == "mech")
        .count();
    assert_eq!(n, 2);
}

/// analyze の coatings: 読み込んだ系（プロジェクトの膜の定義も）の膜の境界
#[test]
fn coatings_of_loaded_project() {
    let mut sv = Server::start(&[]);
    sv.call(
        "load",
        json!({
            "file": Path::new(ROOT).join("tests/filter_os/bp_nd.json").display().to_string(),
            "glass_dir": [Path::new(ROOT).join("tests/glass").display().to_string()],
        }),
    );
    let r = sv.call("analyze", json!({ "cmd": "coatings" }));
    let ifs = r["interfaces"].as_array().unwrap();
    let kind = |c: &str| {
        ifs.iter()
            .find(|x| x["key"].as_str().unwrap().starts_with(&format!("{c}|")))
            .map(|x| x["kind"].as_str().unwrap().to_string())
    };
    assert_eq!(kind("BP550").as_deref(), Some("table"), "{r}");
    assert_eq!(kind("ND05").as_deref(), Some("table"), "{r}");
    assert_eq!(r["needs_table"], json!([]));
    assert!(sv
        .error("analyze", json!({ "cmd": "coatings", "field": 1 }))
        .contains("field"));
    sv.stop();
}

#[test]
fn zoom_of_loaded_project() {
    let mut sv = Server::start(&[]);
    sv.call(
        "load",
        json!({
            "file": Path::new(ROOT).join("examples/projects/P17_zoom_three_groups.json").display().to_string(),
            "glass_dir": [Path::new(ROOT).join("tests/glass").display().to_string()],
        }),
    );
    let p = json!({
        "vars": ["G2.shift", { "name": "G3.shift", "lo": -10.0, "hi": 30.0 }],
        "efl": [140.0, 160.0, 175.0],
        "steps": 1,
    });
    let r = sv.call("zoom", p.clone());
    let path = r["path"].as_array().unwrap();
    // 3 状態と間の 2 点
    assert_eq!(path.len(), 5, "{r}");
    assert_eq!(r["vars"], json!(["G2.shift", "G3.shift"]));
    for (k, e) in [(0, 140.0), (2, 160.0), (4, 175.0)] {
        assert!((path[k]["efl"].as_f64().unwrap() - e).abs() < 1e-5, "{r}");
    }
    assert_eq!(path[4]["state"], json!("tele"));
    // CLI と同じ値
    let cli: Value = {
        let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
            .current_dir(ROOT)
            .args([
                "zoom",
                "examples/projects/P17_zoom_three_groups.json",
                "--glass-dir",
                "tests/glass",
                "--var",
                "G2.shift",
                "--var",
                "G3.shift=-10:30",
                "--efl",
                "140,160,175",
                "--steps",
                "1",
                "--json",
            ])
            .output()
            .unwrap();
        serde_json::from_slice(&o.stdout).unwrap()
    };
    assert_eq!(cli["path"], r["path"]);
    // 状態を選ぶ・目標の数が違えば誤り
    let mut q = p.clone();
    q["configs"] = json!(["wide", "tele"]);
    assert!(sv.error("zoom", q.clone()).contains("目標"));
    q["efl"] = json!([140.0, null]);
    assert_eq!(sv.call("zoom", q)["path"].as_array().unwrap().len(), 3);
    assert!(sv.error("zoom", json!({ "vars": [] })).contains("vars"));
    sv.stop();
}

/// 登録簿の解析の analyze: 前の受け方（数の文字列・カンマ区切り・ハイフンの鍵・json）も受け、設定の誤りは要求の誤り（−32602）。
/// save の provenance は今の系（set の後なら編集後）の hash
#[test]
fn analyze_registered_params_and_save_hash() {
    let mut sv = Server::start(&[]);
    sv.call("load", json!({ "file": ZMX, "glass_dir": ["tests/glass"] }));
    let a = sv.call(
        "analyze",
        json!({ "cmd": "mtf", "field": 2, "freqs": [10, 20] }),
    );
    let b = sv.call(
        "analyze",
        json!({ "cmd": "mtf", "field": "2", "freqs": "10,20", "json": true }),
    );
    assert_eq!(a, b);
    let c = sv.call(
        "analyze",
        json!({ "cmd": "mtf", "field": 2, "max-freq": 20, "freq-steps": 1 }),
    );
    assert_eq!(a["mtf"][0]["freqs"].as_array().unwrap().len(), 2);
    assert_eq!(c["mtf"][0]["freqs"], json!([0.0, 20.0]));
    for bad in [
        json!({ "cmd": "spot", "nope": 1 }),
        json!({ "cmd": "spot", "rings": "x" }),
    ] {
        let (h, _) = sv.call_bin("analyze", bad, &[]);
        assert_eq!(h["error"]["code"], json!(-32602), "{h}");
    }
    let dir = std::env::temp_dir().join(format!("lensbench-hash-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("s.json");
    let hash = |sv: &mut Server| {
        sv.call(
            "analyze",
            json!({ "cmd": "spot", "field": 1, "save": p.to_str().unwrap() }),
        );
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        v["provenance"]["system_hash"].clone()
    };
    let h0 = hash(&mut sv);
    assert!(h0.is_string(), "{h0}");
    let t = sv.call("get", json!({ "param": "3.t" }))["value"]
        .as_f64()
        .unwrap();
    sv.call("set", json!({ "values": { "3.t": t + 0.1 } }));
    let h1 = hash(&mut sv);
    assert!(h1.is_string() && h1 != h0, "{h0} {h1}");
    std::fs::remove_dir_all(&dir).ok();
}
