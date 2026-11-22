//! プラグイン（plugin）と光線の追跡（trace）

use super::*;
use serde::{Deserialize, Serialize};

/// プラグインの設定の値（数・文字列・数の並び。真偽は 0・1）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PluginValue {
    Num(f64),
    Str(String),
    Nums(Vec<f64>),
}

/// プラグインの入力の配列（形と、平らな並び）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginInput {
    pub name: String,
    pub shape: Vec<usize>,
    pub data: Vec<f64>,
}

/// `plugin` の params: プラグインの名前・設定・入力（JSON の数の並びは入れ子で多次元。bin の入力は別に足す）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginReq {
    pub name: String,
    pub config: Vec<(String, PluginValue)>,
    pub inputs: Vec<PluginInput>,
}

impl PluginReq {
    pub(crate) fn parse(p: &Value) -> Result<PluginReq, RpcErr> {
        let name = p["name"]
            .as_str()
            .ok_or_else(|| RpcErr::params("plugin needs name"))?
            .to_string();
        let mut config = Vec::new();
        if let Some(m) = p.get("config").filter(|v| !v.is_null()) {
            let m = m
                .as_object()
                .ok_or_else(|| RpcErr::params("config must be an object"))?;
            for (k, v) in m {
                let x = match v {
                    Value::Number(n) => PluginValue::Num(n.as_f64().unwrap_or(f64::NAN)),
                    Value::String(t) => PluginValue::Str(t.clone()),
                    Value::Bool(b) => PluginValue::Num(*b as u8 as f64),
                    Value::Array(a) => PluginValue::Nums(
                        a.iter()
                            .map(|x| x.as_f64())
                            .collect::<Option<Vec<_>>>()
                            .ok_or_else(|| {
                                RpcErr::params(format!("config {k} must be a list of numbers"))
                            })?,
                    ),
                    _ => return Err(RpcErr::params(format!("config {k} value {v}"))),
                };
                config.push((k.clone(), x));
            }
        }
        let mut inputs = Vec::new();
        if let Some(m) = p.get("inputs").filter(|v| !v.is_null()) {
            let m = m
                .as_object()
                .ok_or_else(|| RpcErr::params("inputs must be an object"))?;
            for (k, v) in m {
                let (mut shape, mut data) = (Vec::new(), Vec::new());
                nested(v, &mut shape, &mut data, 0)
                    .map_err(|e| RpcErr::params(format!("inputs {k}: {e}")))?;
                inputs.push(PluginInput {
                    name: k.clone(),
                    shape,
                    data,
                });
            }
        }
        Ok(PluginReq {
            name,
            config,
            inputs,
        })
    }
}

/// 解析・検出器のプラグインを 1 回呼ぶ。系の要るプラグインには読み込んだ系（set・edit の後）と膜を渡す。
/// 結果は bin（<f8・<c16・|b1）、数 1 つの出力は values にも入れる。膜を読んだときは膜の注意を warnings に入れる
pub(super) fn plugin(sess: &Session, r: &PluginReq, bins: Vec<Bin>) -> Result<Reply, RpcErr> {
    use lensbench_core::plugin::{self as pl, Array};
    let name = r.name.as_str();
    let mut config = pl::Config::new();
    for (k, v) in &r.config {
        let x = match v {
            PluginValue::Num(x) => pl::Value::Num(*x),
            PluginValue::Str(t) => pl::Value::Str(t.clone()),
            PluginValue::Nums(a) => pl::Value::Nums(a.clone()),
        };
        config.insert(k.clone(), x);
    }
    let mut inputs = pl::Arrays::new();
    for x in &r.inputs {
        inputs.insert(x.name.clone(), Array::f64(x.shape.clone(), x.data.clone())?);
    }
    for b in bins {
        let data = match b.data {
            Data::F64(v) => pl::Data::F64(v),
            Data::I32(v) => pl::Data::F64(v.into_iter().map(f64::from).collect()),
            Data::U8(v) => pl::Data::F64(v.into_iter().map(f64::from).collect()),
            Data::C64(v) => pl::Data::C64(v),
            Data::Bool(v) => pl::Data::Bool(v),
        };
        inputs.insert(b.name, Array::new(b.shape, data)?);
    }
    let reg = crate::load::builtin_plugins();
    let plug = reg.get(name)?;
    let inst = plug.prepare(&config)?;
    let sys = sess.served().as_ref().map(|x| x.sys.clone());
    let coat = match &sys {
        Some(s) if plug.spec().needs_system => {
            Some(lensbench_core::coating::Coatings::for_system(s)?)
        }
        _ => None,
    };
    let ctx = pl::Context {
        system: sys.as_ref(),
        coatings: coat.as_ref(),
    };
    let out = inst.run(&ctx, &inputs)?;
    let mut values = Map::new();
    let mut list = Vec::with_capacity(out.len());
    for (k, a) in out {
        if let pl::Data::F64(v) = &a.data {
            if v.len() == 1 && a.shape.iter().all(|&d| d == 1) {
                values.insert(k.clone(), json!(v[0]));
            }
        }
        let data = match a.data {
            pl::Data::F64(v) => Data::F64(v),
            pl::Data::C64(v) => Data::C64(v),
            pl::Data::Bool(v) => Data::Bool(v),
        };
        list.push(Bin {
            name: k,
            shape: a.shape,
            data,
        });
    }
    let mut reply = json!({ "name": name, "values": values });
    // 系の膜の注意（同じ名前の膜のどれを使ったかなど、Coatings::notes）。GUI は coating で始まる注意を結果の上に出す
    if let Some(c) = coat.as_ref().filter(|c| !c.notes.is_empty()) {
        reply["warnings"] = json!(c.notes);
    }
    Ok((reply, list))
}

/// `trace` の params: 正規化瞳座標の並び px・py（bin でもよい）、波長 wave、視野 field（番号）か hx・hy、
/// 追う最後の面 last、大域座標で返すか global
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TraceReq {
    pub px: Option<Vec<f64>>,
    pub py: Option<Vec<f64>>,
    pub wave: Option<usize>,
    pub field: Option<usize>,
    pub hx: Option<f64>,
    pub hy: Option<f64>,
    pub last: Option<usize>,
    pub global: bool,
}

impl TraceReq {
    pub(crate) fn parse(p: &Value) -> Result<TraceReq, RpcErr> {
        let list = |k: &str| -> Result<Option<Vec<f64>>, RpcErr> {
            match p.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Array(a)) => a
                    .iter()
                    .map(|x| {
                        x.as_f64()
                            .ok_or_else(|| RpcErr::params(format!("{k} must be a list of numbers")))
                    })
                    .collect::<Result<_, _>>()
                    .map(Some),
                _ => Err(RpcErr::params(format!("{k} must be a list of numbers"))),
            }
        };
        Ok(TraceReq {
            px: list("px")?,
            py: list("py")?,
            wave: opt_usize(p, "wave")?,
            field: opt_usize(p, "field")?,
            hx: opt_f64(p, "hx")?,
            hy: opt_f64(p, "hy")?,
            last: opt_usize(p, "last")?,
            global: p.get("global").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// 正規化瞳座標 (px, py) の光線を追う。結果の配列は最後の面の局所座標（global なら大域）
pub(super) fn trace(sess: &Session, r: &TraceReq, bins: Vec<Bin>) -> Result<Reply, RpcErr> {
    let (s, _) = sess.current()?;
    let mut bins: HashMap<String, Bin> = bins.into_iter().map(|b| (b.name.clone(), b)).collect();
    // bin で来た並びは params より先
    let mut floats = |k: &str, v: &Option<Vec<f64>>| -> Result<Option<Vec<f64>>, RpcErr> {
        match bins.remove(k) {
            Some(b) => match b.data {
                Data::F64(v) => Ok(Some(v)),
                _ => Err(RpcErr::params(format!("{k} must be <f8"))),
            },
            None => Ok(v.clone()),
        }
    };
    let px = floats("px", &r.px)?.ok_or_else(|| RpcErr::params("trace needs px"))?;
    let py = floats("py", &r.py)?.ok_or_else(|| RpcErr::params("trace needs py"))?;
    if px.len() != py.len() {
        return Err(RpcErr::params(format!(
            "px and py have different lengths ({} and {})",
            px.len(),
            py.len()
        )));
    }
    let w = match r.wave {
        None => s.primary,
        Some(i) => lensbench_core::waves::index(i, s.wavelengths.len())
            .map_err(|e| RpcErr::params(String::from(e)))?,
    };
    let (hx, hy) = match r.field {
        Some(i) if i >= 1 && i <= s.fields.len() => s.hxhy(i - 1),
        Some(i) => {
            return Err(RpcErr::params(format!(
                "no field {i} (1 to {})",
                s.fields.len()
            )))
        }
        None => (r.hx.unwrap_or(0.0), r.hy.unwrap_or(0.0)),
    };
    let last = r.last;
    if let Some(l) = last {
        if l == 0 || l > s.image() {
            return Err(RpcErr::params(format!(
                "last {l} must be 1 to {}",
                s.image()
            )));
        }
    }
    let global = r.global;
    let launch = Launch::Pupil {
        spec: s.object_spec(hx, hy)?,
        signed: false,
        px: &px,
        py: &py,
        vig: Some(s.vig_factors(hx, hy)?),
        hxy: Some((hx, hy)),
    };
    let t = s.trace_launch(&launch, w, last, false);
    let n = t.rays.len();
    let mut c: [Vec<f64>; 7] = std::array::from_fn(|_| Vec::with_capacity(n));
    let mut err = Vec::with_capacity(n);
    let mut vig = Vec::with_capacity(n);
    let mut ok = 0;
    for r in &t.rays {
        let (a, b) = if global { (r.p, r.k) } else { (r.xyz, r.lmn) };
        for i in 0..3 {
            c[i].push(a[i]);
            c[3 + i].push(b[i]);
        }
        c[6].push(r.opl);
        err.push(r.status.code() as i32);
        vig.push(r.vig as i32);
        if r.status.is_ok() && r.vig == 0 {
            ok += 1;
        }
    }
    let [x, y, z, l, m, nn, opl] = c;
    let out = vec![
        Bin::f64("x", x),
        Bin::f64("y", y),
        Bin::f64("z", z),
        Bin::f64("l", l),
        Bin::f64("m", m),
        Bin::f64("n", nn),
        Bin::f64("opl", opl),
        Bin::i32("err", err),
        Bin::i32("vig", vig),
    ];
    Ok((
        json!({
            "rays": n, "ok": ok, "hx": hx, "hy": hy, "wave": w + 1, "last": t.last,
            "frame": if global { "global" } else { "local" },
        }),
        out,
    ))
}

/// JSON の数の並び（入れ子で多次元）を形と平らな並びに
fn nested(
    v: &Value,
    shape: &mut Vec<usize>,
    out: &mut Vec<f64>,
    depth: usize,
) -> lensbench_core::Result<()> {
    match v {
        Value::Number(n) => {
            if depth != shape.len() {
                return Err("the list depths don't match".into());
            }
            out.push(n.as_f64().ok_or("number")?);
            Ok(())
        }
        Value::Array(a) => {
            if depth == shape.len() {
                if !out.is_empty() {
                    return Err("the list depths don't match".into());
                }
                shape.push(a.len());
            } else if shape[depth] != a.len() {
                return Err("the list lengths don't match".into());
            }
            for x in a {
                nested(x, shape, out, depth + 1)?;
            }
            Ok(())
        }
        _ => Err(format!("not a list of numbers ({v})").into()),
    }
}
