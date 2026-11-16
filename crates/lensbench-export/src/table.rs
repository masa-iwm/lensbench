//! 数値の表（列の並び）と、その CSV・JSON・npz（NumPy）への書き出し。
//!
//! - [`rays`] は光線の面ごとの交点・方向・光路長を 1 行 1 交点の表にする。
//! - [`from_json`] は解析の結果（JSON）を表にする（配列の値は `名前_1`, `名前_2`, … の列に展開）。
//! - npz は無圧縮の zip に `列名.npy` を並べたもの（`numpy.load` で読める）。

use std::io::Write;
use std::path::Path;

use lensbench_core::{Error, FieldSel, Result, System};
use serde_json::{json, Map, Value};

/// 列の値
#[derive(Debug, Clone, PartialEq)]
pub enum Col {
    F64(Vec<f64>),
    I64(Vec<i64>),
    Str(Vec<String>),
}

impl Col {
    pub fn len(&self) -> usize {
        match self {
            Col::F64(v) => v.len(),
            Col::I64(v) => v.len(),
            Col::Str(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn cell(&self, i: usize) -> String {
        match self {
            Col::F64(v) => fmt_f64(v[i]),
            Col::I64(v) => v[i].to_string(),
            Col::Str(v) => csv_quote(&v[i]),
        }
    }
    fn json(&self) -> Value {
        match self {
            // JSON に NaN はないので null にする
            Col::F64(v) => Value::Array(
                v.iter()
                    .map(|&x| if x.is_finite() { json!(x) } else { Value::Null })
                    .collect(),
            ),
            Col::I64(v) => json!(v),
            Col::Str(v) => json!(v),
        }
    }
}

/// 列の並び（どの列も同じ長さ）
#[derive(Debug, Clone, Default)]
pub struct Table {
    pub cols: Vec<(String, Col)>,
    /// 表の外の値（単位など）。JSON と npz（スカラーは 0 次元、数の配列は 1 次元の配列）にだけ書く
    pub meta: Map<String, Value>,
}

impl Table {
    pub fn rows(&self) -> usize {
        self.cols.first().map(|c| c.1.len()).unwrap_or(0)
    }

    pub fn get(&self, name: &str) -> Option<&Col> {
        self.cols.iter().find(|c| c.0 == name).map(|c| &c.1)
    }

    pub fn write_csv<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        let h: Vec<String> = self.cols.iter().map(|c| csv_quote(&c.0)).collect();
        writeln!(w, "{}", h.join(","))?;
        for i in 0..self.rows() {
            let r: Vec<String> = self.cols.iter().map(|c| c.1.cell(i)).collect();
            writeln!(w, "{}", r.join(","))?;
        }
        Ok(())
    }

    /// JSON: {"meta"..., "columns": {名前: [値…]}}（列ごと）
    pub fn to_json(&self) -> Value {
        let mut o = self.meta.clone();
        let mut c = Map::new();
        for (n, v) in &self.cols {
            c.insert(n.clone(), v.json());
        }
        o.insert("rows".into(), json!(self.rows()));
        o.insert("columns".into(), Value::Object(c));
        Value::Object(o)
    }

    pub fn write_npz<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        let mut z = Zip::default();
        for (k, v) in &self.meta {
            let c = match v {
                Value::Number(n) => Some(match n.as_i64() {
                    Some(i) => npy(&Col::I64(vec![i]), true),
                    None => npy(&Col::F64(vec![n.as_f64().unwrap_or(f64::NAN)]), true),
                }),
                Value::String(s) => Some(npy(&Col::Str(vec![s.clone()]), true)),
                Value::Array(a) if !a.is_empty() && a.iter().all(Value::is_number) => Some(npy(
                    &Col::F64(a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect()),
                    false,
                )),
                _ => None,
            };
            if let Some(b) = c {
                z.add(&format!("{k}.npy"), b);
            }
        }
        for (n, c) in &self.cols {
            z.add(&format!("{n}.npy"), npy(c, false));
        }
        z.finish(w)
    }

    /// 拡張子（csv・json・npz）で決めて書き出す
    pub fn save(&self, path: &Path) -> lensbench_core::Result<()> {
        let e = path
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.to_ascii_lowercase())
            .unwrap_or_default();
        let mut w: Vec<u8> = Vec::new();
        let r = match e.as_str() {
            "csv" => self.write_csv(&mut w),
            "json" => serde_json::to_writer_pretty(&mut w, &self.to_json())
                .map_err(std::io::Error::other)
                .and_then(|_| writeln!(w)),
            "npz" => self.write_npz(&mut w),
            _ => {
                return Err(
                    format!("{}: the extension must be csv, json or npz", path.display()).into(),
                );
            }
        };
        r.and_then(|_| lensbench_core::vfs::write(path, w))
            .map_err(lensbench_core::Error::io_at(path.display()))
    }
}

fn fmt_f64(v: f64) -> String {
    if v.is_nan() {
        "nan".into()
    } else if v.is_infinite() {
        if v > 0.0 { "inf" } else { "-inf" }.into()
    } else {
        // 往復で同じ値に戻る最短の表記
        format!("{v:?}")
    }
}

fn csv_quote(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// .npy（版 1.0、リトルエンディアン）。scalar なら 0 次元
fn npy(c: &Col, scalar: bool) -> Vec<u8> {
    let (descr, data): (String, Vec<u8>) = match c {
        Col::F64(v) => (
            "<f8".into(),
            v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        ),
        Col::I64(v) => (
            "<i8".into(),
            v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        ),
        Col::Str(v) => {
            let n = v
                .iter()
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
                .max(1);
            let mut d = Vec::with_capacity(v.len() * n * 4);
            for s in v {
                let mut k = 0;
                for ch in s.chars() {
                    d.extend_from_slice(&(ch as u32).to_le_bytes());
                    k += 1;
                }
                d.extend(std::iter::repeat_n(0u8, (n - k) * 4));
            }
            (format!("<U{n}"), d)
        }
    };
    let shape = if scalar {
        "()".to_string()
    } else {
        format!("({},)", c.len())
    };
    let mut h = format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': {shape}, }}");
    // 魔法の 6 + 版 2 + 長さ 2 + 見出し + 改行 が 64 の倍数
    while (10 + h.len() + 1) % 64 != 0 {
        h.push(' ');
    }
    h.push('\n');
    let mut b = Vec::with_capacity(10 + h.len() + data.len());
    b.extend_from_slice(b"\x93NUMPY\x01\x00");
    b.extend_from_slice(&(h.len() as u16).to_le_bytes());
    b.extend_from_slice(h.as_bytes());
    b.extend_from_slice(&data);
    b
}

fn crc32(b: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *e = c;
        }
        t
    });
    let mut c = !0u32;
    for &x in b {
        c = t[((c ^ x as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

/// 無圧縮の zip（npz 用）。4 GB・65,535 個を超える分は zip64 で書く（PRJ-27。numpy の np.load も読む）
#[derive(Default)]
struct Zip {
    files: Vec<(String, Vec<u8>)>,
}

/// zip の 32 ビット・16 ビットの欄の上限（これ以上は zip64 の欄に書き、こちらは 0xFFFFFFFF・0xFFFF）
#[derive(Clone, Copy)]
struct ZipLimits {
    u32: u64,
    u16: usize,
}

const ZIP_LIMITS: ZipLimits = ZipLimits {
    u32: u32::MAX as u64,
    u16: u16::MAX as usize,
};

impl Zip {
    fn add(&mut self, name: &str, data: Vec<u8>) {
        self.files.push((name.to_string(), data));
    }

    fn finish<W: Write>(self, w: &mut W) -> std::io::Result<()> {
        self.finish_with(w, ZIP_LIMITS)
    }

    fn finish_with<W: Write>(self, w: &mut W, lim: ZipLimits) -> std::io::Result<()> {
        let mut off: u64 = 0;
        let mut central = Vec::new();
        let n = self.files.len();
        for (name, data) in &self.files {
            let crc = crc32(data);
            let nb = name.as_bytes();
            let size = data.len() as u64;
            let big = size >= lim.u32;
            let far = off >= lim.u32;
            // 版: zip64 の欄を持つなら 4.5
            let ver: u16 = if big || far { 45 } else { 20 };
            let size32 = if big { u32::MAX } else { size as u32 };
            // 局所の頭: 大きければ大きさは zip64 の追加の欄（元の大きさ・圧縮後の大きさ）
            let mut lx = Vec::new();
            if big {
                lx.extend_from_slice(&1u16.to_le_bytes());
                lx.extend_from_slice(&16u16.to_le_bytes());
                lx.extend_from_slice(&size.to_le_bytes());
                lx.extend_from_slice(&size.to_le_bytes());
            }
            let mut h = Vec::with_capacity(30 + nb.len() + lx.len());
            h.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            h.extend_from_slice(&ver.to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes()); // フラグ
            h.extend_from_slice(&0u16.to_le_bytes()); // 無圧縮
            h.extend_from_slice(&0u16.to_le_bytes()); // 時刻
            h.extend_from_slice(&0x0021u16.to_le_bytes()); // 日付 1980-01-01
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&size32.to_le_bytes());
            h.extend_from_slice(&size32.to_le_bytes());
            h.extend_from_slice(&(nb.len() as u16).to_le_bytes());
            h.extend_from_slice(&(lx.len() as u16).to_le_bytes());
            h.extend_from_slice(nb);
            h.extend_from_slice(&lx);
            w.write_all(&h)?;
            w.write_all(data)?;
            // 中央の頭: zip64 の追加の欄は、溢れた欄だけを元の大きさ・圧縮後の大きさ・頭の位置の順に
            let mut cx = Vec::new();
            if big {
                cx.extend_from_slice(&size.to_le_bytes());
                cx.extend_from_slice(&size.to_le_bytes());
            }
            if far {
                cx.extend_from_slice(&off.to_le_bytes());
            }
            if !cx.is_empty() {
                let body = std::mem::take(&mut cx);
                cx.extend_from_slice(&1u16.to_le_bytes());
                cx.extend_from_slice(&(body.len() as u16).to_le_bytes());
                cx.extend_from_slice(&body);
            }
            let mut c = Vec::with_capacity(46 + nb.len() + cx.len());
            c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            c.extend_from_slice(&ver.to_le_bytes());
            c.extend_from_slice(&ver.to_le_bytes());
            c.extend_from_slice(&0u16.to_le_bytes());
            c.extend_from_slice(&0u16.to_le_bytes());
            c.extend_from_slice(&0u16.to_le_bytes());
            c.extend_from_slice(&0x0021u16.to_le_bytes());
            c.extend_from_slice(&crc.to_le_bytes());
            c.extend_from_slice(&size32.to_le_bytes());
            c.extend_from_slice(&size32.to_le_bytes());
            c.extend_from_slice(&(nb.len() as u16).to_le_bytes());
            c.extend_from_slice(&(cx.len() as u16).to_le_bytes());
            c.extend_from_slice(&[0u8; 6]); // 注釈・ディスク番号・内部属性
            c.extend_from_slice(&0u32.to_le_bytes()); // 外部属性
            c.extend_from_slice(&(if far { u32::MAX } else { off as u32 }).to_le_bytes());
            c.extend_from_slice(nb);
            c.extend_from_slice(&cx);
            central.extend(c);
            off += (h.len() + data.len()) as u64;
        }
        w.write_all(&central)?;
        let clen = central.len() as u64;
        // 終わりの記録: 数・中央の大きさ・位置のどれかが溢れれば、zip64 の終わりの記録とその在り処を先に書く
        let z64 = n >= lim.u16 || clen >= lim.u32 || off >= lim.u32;
        if z64 {
            let mut e = Vec::with_capacity(56 + 20);
            e.extend_from_slice(&0x0606_4b50u32.to_le_bytes());
            e.extend_from_slice(&44u64.to_le_bytes()); // この後の大きさ
            e.extend_from_slice(&45u16.to_le_bytes());
            e.extend_from_slice(&45u16.to_le_bytes());
            e.extend_from_slice(&[0u8; 8]); // ディスク番号
            e.extend_from_slice(&(n as u64).to_le_bytes());
            e.extend_from_slice(&(n as u64).to_le_bytes());
            e.extend_from_slice(&clen.to_le_bytes());
            e.extend_from_slice(&off.to_le_bytes());
            e.extend_from_slice(&0x0706_4b50u32.to_le_bytes());
            e.extend_from_slice(&0u32.to_le_bytes());
            e.extend_from_slice(&(off + clen).to_le_bytes());
            e.extend_from_slice(&1u32.to_le_bytes()); // ディスクの数
            w.write_all(&e)?;
        }
        let n16 = if n >= lim.u16 { u16::MAX } else { n as u16 };
        let c32 = |x: u64| if x >= lim.u32 { u32::MAX } else { x as u32 };
        let mut e = Vec::with_capacity(22);
        e.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        e.extend_from_slice(&[0u8; 4]);
        e.extend_from_slice(&n16.to_le_bytes());
        e.extend_from_slice(&n16.to_le_bytes());
        e.extend_from_slice(&c32(clen).to_le_bytes());
        e.extend_from_slice(&c32(off).to_le_bytes());
        e.extend_from_slice(&0u16.to_le_bytes());
        w.write_all(&e)
    }
}

/// 光線の表の作り方
#[derive(Debug, Clone)]
pub struct RayOptions {
    /// 視野・波長（0 始まり）。None で全視野・主波長
    pub fields: Option<Vec<usize>>,
    pub waves: Option<Vec<usize>>,
    /// 瞳座標
    pub px: Vec<f64>,
    pub py: Vec<f64>,
    /// 書く面（None で物体面を除く全部、像面まで）
    pub surfaces: Option<Vec<usize>>,
}

/// 光線の面ごとの交点の表（1 行 = 1 光線 × 1 面）。
///
/// 列: field・wave（1 始まり）、ray（0 始まり）、px・py、surface、
/// x・y・z と l・m・n（その面の局所座標）、gx・gy・gz（大域座標）、opl（光線の開始点からの光路長 mm。
/// 無限遠の物体では面 1 の頂点を通る、光線に垂直な面から）、
/// err・vig（光線が止まった面・けられた面、0 はなし）。止まった後の面の値は NaN
pub fn rays(s: &System, o: &RayOptions) -> Result<Table> {
    let nf = s.fields.len();
    let nw = s.wavelengths.len();
    let fields = o.fields.clone().unwrap_or_else(|| (0..nf).collect());
    let waves = o.waves.clone().unwrap_or_else(|| vec![s.primary]);
    if let Some(&i) = fields.iter().find(|&&i| i >= nf) {
        return Err(Error::Invalid(format!("no field {} (1 to {nf})", i + 1)));
    }
    for &i in &waves {
        lensbench_core::waves::index(i + 1, nw)?;
    }
    let img = s.image();
    let surfs: Vec<usize> = match &o.surfaces {
        None => (1..=img).collect(),
        Some(v) => v.clone(),
    };
    if let Some(&j) = surfs.iter().find(|&&j| j == 0 || j > img) {
        return Err(Error::Invalid(format!(
            "surface {j} can't be written (1 to {img})"
        )));
    }
    let last = *surfs.iter().max().unwrap_or(&img);
    let mut c_f = Vec::new();
    let mut c_w = Vec::new();
    let mut c_r = Vec::new();
    let mut c_px = Vec::new();
    let mut c_py = Vec::new();
    let mut c_s = Vec::new();
    let mut v: [Vec<f64>; 10] = Default::default();
    let mut c_err = Vec::new();
    let mut c_vig = Vec::new();
    for &fi in &fields {
        for &w in &waves {
            let t = s.try_trace(FieldSel::Index(fi), &o.px, &o.py, Some(w), Some(last), true)?;
            for i in 0..o.px.len() {
                let r = &t.rays[i];
                for &j in &surfs {
                    // 無視する面は記録がない
                    let Some(q) = t.rec_at(j) else {
                        continue;
                    };
                    let e = q.entry(i);
                    c_f.push(fi as i64 + 1);
                    c_w.push(w as i64 + 1);
                    c_r.push(i as i64);
                    c_px.push(o.px[i]);
                    c_py.push(o.py[i]);
                    c_s.push(j as i64);
                    let vals = [
                        e.xyz[0], e.xyz[1], e.xyz[2], e.lmn[0], e.lmn[1], e.lmn[2], e.p[0], e.p[1],
                        e.p[2], e.opl,
                    ];
                    for k in 0..10 {
                        v[k].push(vals[k]);
                    }
                    c_err.push(r.status.code() as i64);
                    c_vig.push(r.vig as i64);
                }
            }
        }
    }
    let names = ["x", "y", "z", "l", "m", "n", "gx", "gy", "gz", "opl"];
    let mut cols = vec![
        ("field".to_string(), Col::I64(c_f)),
        ("wave".to_string(), Col::I64(c_w)),
        ("ray".to_string(), Col::I64(c_r)),
        ("px".to_string(), Col::F64(c_px)),
        ("py".to_string(), Col::F64(c_py)),
        ("surface".to_string(), Col::I64(c_s)),
    ];
    for (k, n) in names.iter().enumerate() {
        cols.push((n.to_string(), Col::F64(std::mem::take(&mut v[k]))));
    }
    cols.push(("err".into(), Col::I64(c_err)));
    cols.push(("vig".into(), Col::I64(c_vig)));
    let mut meta = Map::new();
    meta.insert("name".into(), json!(s.name));
    meta.insert("unit".into(), json!("mm"));
    meta.insert("wavelengths_um".into(), json!(s.wavelengths));
    Ok(Table { cols, meta })
}

/// 解析の結果（JSON）を表にする。最初の「オブジェクトの配列」の値を行に、なければ全体を 1 行にする。
/// 配列の値は `名前_1`, `名前_2`, …（入れ子は `名前_1_2`）の列に展開する。表の外のスカラーは meta へ
pub fn from_json(v: &Value) -> Table {
    let mut meta = Map::new();
    let mut records: Option<&Vec<Value>> = None;
    if let Value::Object(m) = v {
        for (k, x) in m {
            match x {
                Value::Array(a)
                    if records.is_none() && !a.is_empty() && a.iter().all(Value::is_object) =>
                {
                    records = Some(a)
                }
                Value::Number(_) | Value::String(_) | Value::Bool(_) => {
                    meta.insert(k.clone(), x.clone());
                }
                _ => {}
            }
        }
    }
    let one = [v.clone()];
    let recs: &[Value] = match records {
        Some(a) => a,
        None => {
            meta.clear();
            &one
        }
    };
    // 行ごとに平らにした (列名, 値)
    let flat: Vec<Vec<(String, Value)>> = recs
        .iter()
        .map(|r| {
            let mut out = Vec::new();
            if let Value::Object(m) = r {
                for (k, x) in m {
                    flatten(k, x, &mut out);
                }
            }
            out
        })
        .collect();
    let mut names: Vec<String> = Vec::new();
    for r in &flat {
        for (k, _) in r {
            if !names.contains(k) {
                names.push(k.clone());
            }
        }
    }
    let mut cols = Vec::new();
    for n in names {
        let vals: Vec<Option<&Value>> = flat
            .iter()
            .map(|r| r.iter().find(|(k, _)| *k == n).map(|x| &x.1))
            .collect();
        // 整数の列は取り出しと同じ式で判定する（XCT-21）
        let ints: Option<Vec<i64>> = vals.iter().map(|x| x.and_then(Value::as_i64)).collect();
        let all_num = vals
            .iter()
            .all(|x| matches!(x, None | Some(Value::Number(_)) | Some(Value::Null)));
        let col = if let Some(ints) = ints {
            Col::I64(ints)
        } else if all_num {
            Col::F64(
                vals.iter()
                    .map(|x| x.and_then(Value::as_f64).unwrap_or(f64::NAN))
                    .collect(),
            )
        } else {
            Col::Str(
                vals.iter()
                    .map(|x| match x {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Null) | None => String::new(),
                        Some(o) => o.to_string(),
                    })
                    .collect(),
            )
        };
        cols.push((n, col));
    }
    Table { cols, meta }
}

fn flatten(k: &str, x: &Value, out: &mut Vec<(String, Value)>) {
    match x {
        Value::Array(a) => {
            for (i, y) in a.iter().enumerate() {
                flatten(&format!("{k}_{}", i + 1), y, out);
            }
        }
        Value::Object(m) => {
            for (kk, y) in m {
                flatten(&format!("{k}_{kk}"), y, out);
            }
        }
        _ => out.push((k.to_string(), x.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 上限を小さくして zip64 の欄（大きさ・位置・数）を書かせ、npz の読み手で読み直す（PRJ-27）
    #[test]
    fn npz_over_limits_is_zip64() {
        let cols = [
            ("a", Col::F64((0..40).map(f64::from).collect())),
            ("b", Col::I64(vec![1, 2, 3])),
            ("c", Col::F64(vec![0.5])),
        ];
        let write = |lim: ZipLimits| {
            let mut z = Zip::default();
            for (n, c) in &cols {
                z.add(&format!("{n}.npy"), npy(c, false));
            }
            let mut out = Vec::new();
            z.finish_with(&mut out, lim).unwrap();
            out
        };
        let small = write(ZIP_LIMITS);
        let big = write(ZipLimits { u32: 200, u16: 2 });
        assert!(big.windows(4).any(|w| w == 0x0606_4b50u32.to_le_bytes()));
        assert!(!small.windows(4).any(|w| w == 0x0606_4b50u32.to_le_bytes()));
        for b in [small, big] {
            let mut r = ndarray_npy::NpzReader::new(std::io::Cursor::new(b)).unwrap();
            let a: ndarray::Array1<f64> = r.by_name("a.npy").unwrap();
            assert_eq!(a.len(), 40);
            assert_eq!(a[39], 39.0);
            let b: ndarray::Array1<i64> = r.by_name("b.npy").unwrap();
            assert_eq!(b.to_vec(), [1, 2, 3]);
            let c: ndarray::Array1<f64> = r.by_name("c.npy").unwrap();
            assert_eq!(c.to_vec(), [0.5]);
        }
    }

    #[test]
    fn crc32_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn npy_header_is_aligned() {
        let b = npy(&Col::F64(vec![1.0, 2.0]), false);
        let hl = u16::from_le_bytes([b[8], b[9]]) as usize;
        assert_eq!((10 + hl) % 64, 0);
        assert_eq!(b.len(), 10 + hl + 16);
        assert!(std::str::from_utf8(&b[10..10 + hl])
            .unwrap()
            .contains("'shape': (2,)"));
    }

    #[test]
    fn json_records_are_flattened() {
        let v = json!({"unit": "waves", "zernike": [
            {"field": 1, "coef": [0.1, 0.2], "pv": 0.5},
            {"field": 2, "coef": [0.3, 0.4], "pv": 0.25}]});
        let t = from_json(&v);
        let names: Vec<&str> = t.cols.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, ["field", "coef_1", "coef_2", "pv"]);
        assert_eq!(t.get("field"), Some(&Col::I64(vec![1, 2])));
        assert_eq!(t.get("coef_2"), Some(&Col::F64(vec![0.2, 0.4])));
        assert_eq!(t.meta["unit"], "waves");
        let mut b = Vec::new();
        t.write_csv(&mut b).unwrap();
        assert_eq!(
            String::from_utf8(b).unwrap(),
            "field,coef_1,coef_2,pv\n1,0.1,0.2,0.5\n2,0.3,0.4,0.25\n"
        );
    }
}
