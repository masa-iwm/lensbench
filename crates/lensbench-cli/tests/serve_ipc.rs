//! serve の残り: 干渉の解析（fringe）・プラグイン・パイプ・共有メモリ。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use lensbench_core::coating::C64;
use serde_json::{json, Value};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// 2 つの構成の干渉計: f 50 の理想レンズで平行にした光をセンサで受ける。構成 1（試料）はレンズとセンサの間が
/// 0.3 mm 長い（多重構成の THIC）
const TWO_CFG: &str = "VERS 260327 42
MODE SEQ
UNIT MM X W X CM MR CPMM
OBNA 0.15 0
FTYP 0 0 1 1 0 0 0
YFLN 0
FWGN 1
WAVM 1 0.84 1
PWAV 1
SURF 0
  TYPE STANDARD
  CURV 0.0
  DISZ 50
SURF 1
  STOP
  TYPE PARAXIAL
  PARM 1 50
  PARM 2 1
  DISZ 100
SURF 2
  TYPE STANDARD
  CURV 0.0
  DISZ 0
MNUM 2 1
THIC 1 1 100.3 0 0 0 1 1 0
THIC 1 2 100.0 0 0 0 1 1 0
";

fn exe() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_lensbench"));
    c.current_dir(ROOT).env_remove("LENSBENCH_GLASSCAT");
    c
}

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lb-serve-ipc-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn cli_json(args: &[&str]) -> Value {
    let o = exe().args(args).arg("--json").output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    strip_notes(serde_json::from_slice(&o.stdout).unwrap())
}

/// 配列の値（dtype と生のバイト列）
#[derive(Clone, Debug, PartialEq)]
enum Arr {
    F64(Vec<f64>),
    C64(Vec<C64>),
    I32(Vec<i32>),
    Bool(Vec<bool>),
}

impl Arr {
    fn dtype(&self) -> &'static str {
        match self {
            Arr::F64(_) => "<f8",
            Arr::C64(_) => "<c16",
            Arr::I32(_) => "<i4",
            Arr::Bool(_) => "|b1",
        }
    }
    fn bytes(&self) -> Vec<u8> {
        let mut o = Vec::new();
        match self {
            Arr::F64(v) => v.iter().for_each(|x| o.extend(x.to_le_bytes())),
            Arr::C64(v) => v.iter().for_each(|z| {
                o.extend(z.re.to_le_bytes());
                o.extend(z.im.to_le_bytes());
            }),
            Arr::I32(v) => v.iter().for_each(|x| o.extend(x.to_le_bytes())),
            Arr::Bool(v) => v.iter().for_each(|&b| o.push(b as u8)),
        }
        o
    }
    fn decode(dtype: &str, b: &[u8]) -> Arr {
        match dtype {
            "<f8" => Arr::F64(
                b.chunks_exact(8)
                    .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
                    .collect(),
            ),
            "<c16" => Arr::C64(
                b.chunks_exact(16)
                    .map(|c| {
                        C64::new(
                            f64::from_le_bytes(c[..8].try_into().unwrap()),
                            f64::from_le_bytes(c[8..].try_into().unwrap()),
                        )
                    })
                    .collect(),
            ),
            "<i4" => Arr::I32(
                b.chunks_exact(4)
                    .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
                    .collect(),
            ),
            "|b1" => Arr::Bool(b.iter().map(|&x| x != 0).collect()),
            d => panic!("dtype {d}"),
        }
    }
    fn item(dtype: &str) -> usize {
        match dtype {
            "<f8" => 8,
            "<c16" => 16,
            "<i4" => 4,
            _ => 1,
        }
    }
}

/// 1 本の接続（TCP かパイプ）
struct Conn {
    r: BufReader<Box<dyn Read>>,
    w: Box<dyn Write>,
    id: u64,
}

type Named = Vec<(String, Vec<usize>, Arr)>;

impl Conn {
    /// 要求を送り (応答のヘッダ, ストリームで来た配列) を返す。shm の配列は呼ぶ側が読む
    fn send(
        &mut self,
        method: &str,
        params: Value,
        bins: &[(&str, Vec<usize>, Arr)],
        extra: Value,
    ) -> (Value, Named) {
        self.id += 1;
        let mut h = json!({ "jsonrpc": "2.0", "id": self.id, "method": method, "params": params });
        if let Value::Object(m) = extra {
            for (k, v) in m {
                h[k] = v;
            }
        }
        let mut payload = Vec::new();
        if !bins.is_empty() && h.get("bin").is_none() {
            h["bin"] = bins
                .iter()
                .map(|(n, s, a)| json!({ "name": n, "dtype": a.dtype(), "shape": s }))
                .collect();
            for (_, _, a) in bins {
                payload.extend(a.bytes());
            }
        }
        let mut out = serde_json::to_vec(&h).unwrap();
        out.push(b'\n');
        out.extend(payload);
        self.w.write_all(&out).unwrap();
        self.w.flush().unwrap();
        let mut line = String::new();
        self.r.read_line(&mut line).unwrap();
        let h: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(h["id"], json!(self.id), "{h}");
        let mut arrs = Vec::new();
        for b in h["bin"].as_array().cloned().unwrap_or_default() {
            if b.get("offset").is_some() {
                continue;
            }
            let shape: Vec<usize> = b["shape"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap() as usize)
                .collect();
            let dt = b["dtype"].as_str().unwrap();
            let mut buf = vec![0u8; shape.iter().product::<usize>() * Arr::item(dt)];
            self.r.read_exact(&mut buf).unwrap();
            arrs.push((
                b["name"].as_str().unwrap().to_string(),
                shape,
                Arr::decode(dt, &buf),
            ));
        }
        (h, arrs)
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let (h, _) = self.send(method, params, &[], json!({}));
        if let Some(e) = h.get("error") {
            panic!("{method}: {e}");
        }
        h["result"].clone()
    }
}

struct Server {
    child: Child,
    first: Value,
}

impl Server {
    fn start(args: &[&str]) -> Server {
        let mut child = exe()
            .args(["--threads", "1", "serve"])
            .args(args)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        Server {
            child,
            first: serde_json::from_str(&line).unwrap(),
        }
    }

    fn tcp(&self) -> Conn {
        let port = self.first["port"].as_u64().unwrap() as u16;
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_nodelay(true).unwrap();
        Conn {
            r: BufReader::new(Box::new(s.try_clone().unwrap())),
            w: Box::new(s),
            id: 0,
        }
    }

    fn pipe(&self) -> Conn {
        let p = self.first["pipe"].as_str().unwrap().to_string();
        #[cfg(unix)]
        {
            let s = std::os::unix::net::UnixStream::connect(&p).unwrap();
            Conn {
                r: BufReader::new(Box::new(s.try_clone().unwrap())),
                w: Box::new(s),
                id: 0,
            }
        }
        #[cfg(windows)]
        {
            // 前の接続の直後は次の口ができるまで少し待つ（ERROR_PIPE_BUSY）
            let t0 = std::time::Instant::now();
            let f = loop {
                match std::fs::OpenOptions::new().read(true).write(true).open(&p) {
                    Ok(f) => break f,
                    Err(_) if t0.elapsed().as_secs() < 10 => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(e) => panic!("{p}: {e}"),
                }
            };
            Conn {
                r: BufReader::new(Box::new(f.try_clone().unwrap())),
                w: Box::new(f),
                id: 0,
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn strip_notes(mut v: Value) -> Value {
    v.as_object_mut().unwrap().remove("notes");
    v
}

/// 完了条件: serve の fringe は CLI と同じ結果。set の変更は両方の光束に当て、構成で値が違うパラメータは当てない
#[test]
fn interference_like_cli() {
    let d = tmp_dir("fringe");
    let f = d.join("two_cfg.zmx");
    std::fs::write(&f, TWO_CFG).unwrap();
    let fs = f.to_str().unwrap();
    let sv = Server::start(&["--port", "0", fs]);
    let mut c = sv.tcp();

    let fr = ["--n", "16", "--beam1", "1", "--beam2", "2"];
    let mut args = vec!["fringe", fs];
    args.extend(fr);
    let want_f = cli_json(&args);
    let params = json!({ "cmd": "fringe", "n": 16, "beam1": "1", "beam2": "2" });
    let got_f = c.call("analyze", params.clone());
    assert_eq!(got_f["notes"], json!([]));
    assert_eq!(strip_notes(got_f), want_f);

    // grid: 格子も結果に入れる（GUI の図）。CLI の --grid と同じ
    let mut args = vec!["fringe", fs];
    args.extend(fr);
    args.push("--grid");
    let got_fg = strip_notes(c.call(
        "analyze",
        json!({ "cmd": "fringe", "n": 16, "beam1": "1", "beam2": "2", "grid": true }),
    ));
    assert_eq!(got_fg, cli_json(&args));
    assert_eq!(got_fg["grid"].as_array().unwrap().len(), 16 * 16);

    // 構成 1 だけの値（1.t）: 構成 2 には当てず、notes に書く
    c.call("set", json!({ "values": { "1.t": 100.5 } }));
    let got = c.call("analyze", params.clone());
    let notes = got["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].as_str().unwrap().contains("構成 2"));
    // 同じ値をファイルに書いた系の CLI と同じ（構成 1 の値だけ変わる）
    let f2 = d.join("two_cfg_changed.zmx");
    std::fs::write(&f2, TWO_CFG.replace("THIC 1 1 100.3", "THIC 1 1 100.5")).unwrap();
    let mut args = vec!["fringe", f2.to_str().unwrap()];
    args.extend(fr);
    assert_eq!(strip_notes(got), cli_json(&args));
    // 戻すと CLI と同じ
    c.call("set", json!({ "values": { "1.t": 100.3 } }));
    assert_eq!(strip_notes(c.call("analyze", params.clone())), want_f);
    // reload で変更の記録も消える
    c.call("set", json!({ "values": { "1.t": 100.5 } }));
    c.call("reload", json!({}));
    assert_eq!(strip_notes(c.call("analyze", params)), want_f);
    std::fs::remove_dir_all(&d).ok();
}

/// プラグイン: 系の要るもの（spot）は読み込んだ系で
#[test]
fn plugins_over_serve() {
    let sv = Server::start(&[
        "--port",
        "0",
        "tests/refs/A0/system.zmx",
        "--glass-dir",
        "tests/glass",
    ]);
    let mut c = sv.tcp();
    let list = c.call("plugins", json!({}));
    assert!(list["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "spot"));

    c.call("set", json!({ "values": { "3.t": 2.5 } }));
    let want =
        c.call("analyze", json!({ "cmd": "spot", "field": 2 }))["spot"][0]["rms_centroid"].clone();
    let got = c.call(
        "plugin",
        json!({ "name": "spot", "inputs": { "field": 2 } }),
    );
    assert_eq!(got["values"]["rms_centroid"], want);
}

/// パイプ（Windows は名前付きパイプ、ほかは Unix ドメインソケット）でも TCP と同じ書式
#[test]
fn pipe_transport() {
    let name = format!("lensbench-test-{}", std::process::id());
    #[cfg(unix)]
    let name = tmp_dir("pipe").join(name).to_str().unwrap().to_string();
    let sv = Server::start(&[
        "--pipe",
        &name,
        "tests/refs/A0/system.zmx",
        "--glass-dir",
        "tests/glass",
    ]);
    assert!(sv.first["pipe"]
        .as_str()
        .unwrap()
        .ends_with(name.rsplit(['/', '\\']).next().unwrap()));
    // 2 本つないで交互に使う
    let mut a = sv.pipe();
    let mut b = sv.pipe();
    assert_eq!(
        a.call("ping", json!({}))["version"],
        json!(lensbench_core::VERSION)
    );
    let v: Vec<f64> = (0..100_000).map(|k| k as f64 * 0.5).collect();
    let (_, back) = b.send(
        "echo",
        json!({}),
        &[("v", vec![v.len()], Arr::F64(v.clone()))],
        json!({}),
    );
    assert_eq!(back[0].2, Arr::F64(v));
    let s1 = a.call("analyze", json!({ "cmd": "spot", "field": 1 }));
    let s2 = b.call("analyze", json!({ "cmd": "spot", "field": 1 }));
    assert_eq!(s1, s2);
    // shutdown の応答は届いてから止まる
    a.call("shutdown", json!({}));
    let mut sv = sv;
    assert!(sv.child.wait().unwrap().success());
    #[cfg(unix)]
    std::fs::remove_dir_all(tmp_dir("pipe")).ok();
}

/// 共有メモリ（Linux は /dev/shm のファイル）: 配列を共有メモリから読み、応答も収まれば共有メモリに書く
#[cfg(target_os = "linux")]
#[test]
fn shared_memory_arrays() {
    use std::os::unix::fs::FileExt;
    let name = format!("lensbench-test-shm-{}", std::process::id());
    let path = PathBuf::from("/dev/shm").join(&name);
    let n = 2000;
    let px: Vec<f64> = (0..n)
        .map(|k| -1.0 + 2.0 * k as f64 / (n - 1) as f64)
        .collect();
    let py: Vec<f64> = px.iter().map(|x| 0.5 * x).collect();
    let fsz = 1 << 20;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    f.set_len(fsz).unwrap();
    f.write_all_at(&Arr::F64(px.clone()).bytes(), 0).unwrap();
    f.write_all_at(&Arr::F64(py.clone()).bytes(), 16384)
        .unwrap();

    let sv = Server::start(&[
        "--port",
        "0",
        "tests/refs/A0/system.zmx",
        "--glass-dir",
        "tests/glass",
    ]);
    let mut c = sv.tcp();
    let (want_h, want) = c.send(
        "trace",
        json!({ "field": 2 }),
        &[
            ("px", vec![n], Arr::F64(px.clone())),
            ("py", vec![n], Arr::F64(py.clone())),
        ],
        json!({}),
    );
    let (h, stream) = c.send(
        "trace",
        json!({ "field": 2 }),
        &[],
        json!({
            "shm": { "name": name, "out": 65536 },
            "bin": [
                { "name": "px", "dtype": "<f8", "shape": [n], "offset": 0 },
                { "name": "py", "dtype": "<f8", "shape": [n], "offset": 16384 },
            ],
        }),
    );
    assert!(stream.is_empty());
    assert_eq!(h["result"], want_h["result"]);
    let list = h["bin"].as_array().unwrap();
    assert_eq!(list.len(), want.len());
    for (b, (name, _, a)) in list.iter().zip(&want) {
        assert_eq!(b["name"], json!(name));
        let off = b["offset"].as_u64().unwrap();
        assert!(off >= 65536 && off % 64 == 0);
        let dt = b["dtype"].as_str().unwrap();
        let mut buf = vec![0u8; n * Arr::item(dt)];
        f.read_exact_at(&mut buf, off).unwrap();
        assert_eq!(&Arr::decode(dt, &buf), a, "{name}");
    }
    // 収まらない応答はストリームで返す
    let (h, stream) = c.send(
        "trace",
        json!({ "field": 2 }),
        &[],
        json!({
            "shm": { "name": name, "out": fsz - 1000 },
            "bin": [
                { "name": "px", "dtype": "<f8", "shape": [n], "offset": 0 },
                { "name": "py", "dtype": "<f8", "shape": [n], "offset": 16384 },
            ],
        }),
    );
    assert!(h["bin"][0].get("offset").is_none());
    assert_eq!(stream, want);
    // 範囲の外・名前の誤りはエラーで、接続は続く
    let (h, _) = c.send(
        "echo",
        json!({}),
        &[],
        json!({ "shm": { "name": name }, "bin": [{ "name": "x", "dtype": "<f8", "shape": [10], "offset": fsz - 8 }] }),
    );
    assert!(
        h["error"]["message"].as_str().unwrap().contains("範囲の外"),
        "{h}"
    );
    let (h, _) = c.send(
        "echo",
        json!({}),
        &[],
        json!({ "shm": { "name": "../etc/passwd" }, "bin": [{ "name": "x", "dtype": "<f8", "shape": [1], "offset": 0 }] }),
    );
    assert!(
        h["error"]["message"].as_str().unwrap().contains("名前"),
        "{h}"
    );
    assert_eq!(
        c.call("ping", json!({}))["version"],
        json!(lensbench_core::VERSION)
    );
    drop(f);
    std::fs::remove_file(&path).ok();
}

/// --allow-remote では共有メモリを使わない
#[cfg(target_os = "linux")]
#[test]
fn remote_does_not_use_shared_memory() {
    let sv = Server::start(&["--port", "0", "--allow-remote"]);
    let mut c = sv.tcp();
    let (h, _) = c.send(
        "echo",
        json!({}),
        &[],
        json!({ "shm": { "name": "x" }, "bin": [{ "name": "x", "dtype": "<f8", "shape": [1], "offset": 0 }] }),
    );
    assert!(
        h["error"]["message"]
            .as_str()
            .unwrap()
            .contains("allow-remote"),
        "{h}"
    );
}

/// --pipe に既にある普通のファイルを渡しても消さずに断る
#[cfg(unix)]
#[test]
fn pipe_keeps_regular_file() {
    let d = tmp_dir("pipe-file");
    let f = d.join("not-a-socket");
    std::fs::write(&f, "keep").unwrap();
    let o = exe()
        .args(["serve", "--pipe", f.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("ソケットでない"));
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "keep");
    std::fs::remove_dir_all(&d).ok();
}
