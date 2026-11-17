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
        let mut z = Zip::new(ZIP_LIMITS);
        write_npz_meta(&mut z, w, &self.meta)?;
        for (n, c) in &self.cols {
            z.add(w, &format!("{n}.npy"), &[&npy(c, false)])?;
        }
        z.finish(w)
    }

    /// 同じ列の並びの表を後ろに足す（[`rays_each`] の塊をつなぐ）
    fn append(&mut self, other: Table) {
        if self.cols.is_empty() {
            self.cols = other.cols;
            return;
        }
        for ((_, a), (_, b)) in self.cols.iter_mut().zip(other.cols) {
            match (a, b) {
                (Col::F64(a), Col::F64(b)) => a.extend(b),
                (Col::I64(a), Col::I64(b)) => a.extend(b),
                (Col::Str(a), Col::Str(b)) => a.extend(b),
                _ => unreachable!("the chunks have the same columns"),
            }
        }
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

/// 表の外の値を npz に書く（スカラーは 0 次元、数の配列は 1 次元。ほかの値は書かない）
fn write_npz_meta<W: Write>(
    z: &mut Zip,
    w: &mut W,
    meta: &Map<String, Value>,
) -> std::io::Result<()> {
    for (k, v) in meta {
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
            z.add(w, &format!("{k}.npy"), &[&b])?;
        }
    }
    Ok(())
}

/// .npy の頭（版 1.0）。`shape` は `()`（0 次元）か `(n,)`
fn npy_header(descr: &str, shape: &str) -> Vec<u8> {
    let mut h = format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': {shape}, }}");
    // 魔法の 6 + 版 2 + 長さ 2 + 見出し + 改行 が 64 の倍数
    while (10 + h.len() + 1) % 64 != 0 {
        h.push(' ');
    }
    h.push('\n');
    let mut b = Vec::with_capacity(10 + h.len());
    b.extend_from_slice(b"\x93NUMPY\x01\x00");
    b.extend_from_slice(&(h.len() as u16).to_le_bytes());
    b.extend_from_slice(h.as_bytes());
    b
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
    let mut b = npy_header(&descr, &shape);
    b.extend_from_slice(&data);
    b
}

#[cfg(test)]
fn crc32(b: &[u8]) -> u32 {
    !crc32_update(!0, b)
}

/// CRC-32 を続けて計算する（始めは `!0`、終わりに `!` を取る）
fn crc32_update(mut c: u32, b: &[u8]) -> u32 {
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
    for &x in b {
        c = t[((c ^ x as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c
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

/// 無圧縮の zip（npz 用）を足すそばから書く（中身を全部は持たない）。4 GB・65,535 個を超える分は zip64 で書く
/// （numpy の np.load も読む）
struct Zip {
    lim: ZipLimits,
    /// 次の局所の頭の位置
    off: u64,
    central: Vec<u8>,
    n: usize,
}

impl Zip {
    fn new(lim: ZipLimits) -> Zip {
        Zip {
            lim,
            off: 0,
            central: Vec::new(),
            n: 0,
        }
    }

    /// 1 つ書く。中身は `parts` をつないだもの
    fn add<W: Write>(&mut self, w: &mut W, name: &str, parts: &[&[u8]]) -> std::io::Result<()> {
        let lim = self.lim;
        let off = self.off;
        let crc = !parts.iter().fold(!0u32, |c, p| crc32_update(c, p));
        let nb = name.as_bytes();
        let size: u64 = parts.iter().map(|p| p.len() as u64).sum();
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
        for p in parts {
            w.write_all(p)?;
        }
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
        let c = &mut self.central;
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
        self.off += h.len() as u64 + size;
        self.n += 1;
        Ok(())
    }

    /// 中央の目録と終わりの記録を書く
    fn finish<W: Write>(self, w: &mut W) -> std::io::Result<()> {
        let Zip {
            lim,
            off,
            central,
            n,
        } = self;
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

/// 光線の表の列（[`rays`]）。i は整数、f は実数
const RAY_COLS: [(&str, bool); 18] = [
    ("field", false),
    ("wave", false),
    ("ray", false),
    ("px", true),
    ("py", true),
    ("surface", false),
    ("x", true),
    ("y", true),
    ("z", true),
    ("l", true),
    ("m", true),
    ("n", true),
    ("gx", true),
    ("gy", true),
    ("gz", true),
    ("opl", true),
    ("err", false),
    ("vig", false),
];

/// 光線の面ごとの交点の表（1 行 = 1 光線 × 1 面）。
///
/// 列: field・wave（1 始まり）、ray（0 始まり）、px・py、surface、
/// x・y・z と l・m・n（その面の局所座標）、gx・gy・gz（大域座標）、opl（光線の開始点からの光路長 mm。
/// 無限遠の物体では面 1 の頂点を通る、光線に垂直な面から）、
/// err・vig（光線が止まった面・けられた面、0 はなし）。止まった後の面の値は NaN
pub fn rays(s: &System, o: &RayOptions) -> Result<Table> {
    let mut all = Table::default();
    let meta = rays_each(s, o, |t| {
        all.append(t);
        Ok(())
    })?;
    if all.cols.is_empty() {
        all = ray_chunk(Default::default(), Default::default(), Default::default());
    }
    all.meta = meta;
    Ok(all)
}

/// [`rays`] の表を、視野・波長の組ごとの塊（同じ列の [`Table`]、meta は空）で `f` に渡す。返すのは表の meta。
/// 全体を持たずに書き出すため（[`save_rays`]）
pub fn rays_each(
    s: &System,
    o: &RayOptions,
    mut f: impl FnMut(Table) -> Result<()>,
) -> Result<Map<String, Value>> {
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
    for &fi in &fields {
        for &w in &waves {
            let t = s.try_trace(FieldSel::Index(fi), &o.px, &o.py, Some(w), Some(last), true)?;
            let mut ints: [Vec<i64>; 6] = Default::default();
            let mut pxy: [Vec<f64>; 2] = Default::default();
            let mut v: [Vec<f64>; 10] = Default::default();
            for i in 0..o.px.len() {
                let r = &t.rays[i];
                for &j in &surfs {
                    // 無視する面は記録がない
                    let Some(q) = t.rec_at(j) else {
                        continue;
                    };
                    let e = q.entry(i);
                    ints[0].push(fi as i64 + 1);
                    ints[1].push(w as i64 + 1);
                    ints[2].push(i as i64);
                    pxy[0].push(o.px[i]);
                    pxy[1].push(o.py[i]);
                    ints[3].push(j as i64);
                    let vals = [
                        e.xyz[0], e.xyz[1], e.xyz[2], e.lmn[0], e.lmn[1], e.lmn[2], e.p[0], e.p[1],
                        e.p[2], e.opl,
                    ];
                    for k in 0..10 {
                        v[k].push(vals[k]);
                    }
                    ints[4].push(r.status.code() as i64);
                    ints[5].push(r.vig as i64);
                }
            }
            f(ray_chunk(ints, pxy, v))?;
        }
    }
    let mut meta = Map::new();
    meta.insert("name".into(), json!(s.name));
    meta.insert("unit".into(), json!("mm"));
    meta.insert("wavelengths_um".into(), json!(s.wavelengths));
    Ok(meta)
}

/// 光線の表の塊（整数の列 field・wave・ray・surface・err・vig、px・py、実数の列 x〜opl）
fn ray_chunk(ints: [Vec<i64>; 6], pxy: [Vec<f64>; 2], v: [Vec<f64>; 10]) -> Table {
    let [f, w, r, s, err, vig] = ints;
    let [px, py] = pxy;
    let mut cols = vec![
        ("field".to_string(), Col::I64(f)),
        ("wave".to_string(), Col::I64(w)),
        ("ray".to_string(), Col::I64(r)),
        ("px".to_string(), Col::F64(px)),
        ("py".to_string(), Col::F64(py)),
        ("surface".to_string(), Col::I64(s)),
    ];
    for ((n, _), c) in RAY_COLS[6..16].iter().zip(v) {
        cols.push((n.to_string(), Col::F64(c)));
    }
    cols.push(("err".into(), Col::I64(err)));
    cols.push(("vig".into(), Col::I64(vig)));
    Table {
        cols,
        meta: Map::new(),
    }
}

/// 書き出しの塊を溜めてから vfs の末尾に足す（1 回ごとにファイルを開かないため）
struct VfsAppend {
    path: std::path::PathBuf,
    buf: Vec<u8>,
}

impl VfsAppend {
    const CHUNK: usize = 8 << 20;
}

impl Write for VfsAppend {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(b);
        if self.buf.len() >= Self::CHUNK {
            self.flush()?;
        }
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            lensbench_core::vfs::append(&self.path, &self.buf)?;
            self.buf.clear();
        }
        Ok(())
    }
}

/// 一時の列のファイルを消す（途中の誤りでも残さない）
struct Parts(Vec<std::path::PathBuf>);

impl Drop for Parts {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = lensbench_core::vfs::remove_file(p);
        }
    }
}

/// [`rays`] の表を、全体をメモリに作らずにファイルへ書く。CSV は塊ごとに末尾へ、npz は列ごとの一時ファイル
/// （書き出し先の隣の `<名前>.part<k>`）に溜めて最後に 1 列ずつ zip に写す。JSON は列ごとの形なので [`rays`] で作ってから書く。
/// 中身は `rays(s, o)?.save(path)` と同じ。返すのは行の数
pub fn save_rays(s: &System, o: &RayOptions, path: &Path) -> Result<usize> {
    let e = path
        .extension()
        .and_then(|x| x.to_str())
        .map(|x| x.to_ascii_lowercase())
        .unwrap_or_default();
    let io = |r: std::io::Result<()>| r.map_err(lensbench_core::Error::io_at(path.display()));
    match e.as_str() {
        "csv" => {
            let h: Vec<String> = RAY_COLS.iter().map(|c| csv_quote(c.0)).collect();
            io(lensbench_core::vfs::write(
                path,
                format!("{}\n", h.join(",")),
            ))?;
            let mut rows = 0;
            let mut w = VfsAppend {
                path: path.to_path_buf(),
                buf: Vec::new(),
            };
            let r = rays_each(s, o, |t| {
                rows += t.rows();
                for i in 0..t.rows() {
                    let r: Vec<String> = t.cols.iter().map(|c| c.1.cell(i)).collect();
                    io(writeln!(w, "{}", r.join(",")))?;
                }
                Ok(())
            });
            let done = r.and_then(|_| io(w.flush()));
            if done.is_err() {
                let _ = lensbench_core::vfs::remove_file(path);
            }
            done.map(|_| rows)
        }
        "npz" => {
            let parts = Parts(
                (0..RAY_COLS.len())
                    .map(|k| {
                        let mut n = path.file_name().unwrap_or_default().to_os_string();
                        n.push(format!(".part{k}"));
                        path.with_file_name(n)
                    })
                    .collect(),
            );
            for p in &parts.0 {
                io(lensbench_core::vfs::write(p, b""))?;
            }
            let mut rows = 0;
            let meta = rays_each(s, o, |t| {
                rows += t.rows();
                for ((_, c), p) in t.cols.iter().zip(&parts.0) {
                    let b: Vec<u8> = match c {
                        Col::F64(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
                        Col::I64(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
                        Col::Str(_) => unreachable!("the ray table has no text columns"),
                    };
                    io(lensbench_core::vfs::append(p, b))?;
                }
                Ok(())
            })?;
            io(lensbench_core::vfs::write(path, b""))?;
            let mut w = VfsAppend {
                path: path.to_path_buf(),
                buf: Vec::new(),
            };
            let done = (|| -> std::io::Result<()> {
                let mut z = Zip::new(ZIP_LIMITS);
                write_npz_meta(&mut z, &mut w, &meta)?;
                for ((n, float), p) in RAY_COLS.iter().zip(&parts.0) {
                    let data = lensbench_core::vfs::read(p)?;
                    let descr = if *float { "<f8" } else { "<i8" };
                    let h = npy_header(descr, &format!("({rows},)"));
                    z.add(&mut w, &format!("{n}.npy"), &[&h, &data])?;
                }
                z.finish(&mut w)?;
                w.flush()
            })();
            if done.is_err() {
                let _ = lensbench_core::vfs::remove_file(path);
            }
            io(done).map(|_| rows)
        }
        _ => {
            let t = rays(s, o)?;
            t.save(path)?;
            Ok(t.rows())
        }
    }
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
        // 整数の列は取り出しと同じ式で判定する
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

    /// 上限を小さくして zip64 の欄（大きさ・位置・数）を書かせ、npz の読み手で読み直す
    #[test]
    fn npz_over_limits_is_zip64() {
        let cols = [
            ("a", Col::F64((0..40).map(f64::from).collect())),
            ("b", Col::I64(vec![1, 2, 3])),
            ("c", Col::F64(vec![0.5])),
        ];
        let write = |lim: ZipLimits| {
            let mut z = Zip::new(lim);
            let mut out = Vec::new();
            for (n, c) in &cols {
                z.add(&mut out, &format!("{n}.npy"), &[&npy(c, false)])
                    .unwrap();
            }
            z.finish(&mut out).unwrap();
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
