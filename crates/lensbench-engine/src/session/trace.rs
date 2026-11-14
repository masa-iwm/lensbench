//! プラグイン（plugin）と光線の追跡（trace）

use super::*;

/// 解析・検出器のプラグインを 1 回呼ぶ。params: name・config（数・文字列・数の並び）・inputs（JSON の
/// 数の並び、入れ子で多次元。bin でもよい）。系の要るプラグインには読み込んだ系（set・edit の後）と膜を渡す。
/// 結果は bin（<f8・<c16・|b1）、数 1 つの出力は values にも入れる
pub(super) fn plugin(sess: &Session, p: &Value, bins: Vec<Bin>) -> Result<Reply, RpcErr> {
    use lensbench_core::plugin::{self as pl, Array};
    let name = p["name"]
        .as_str()
        .ok_or_else(|| RpcErr::params("plugin には name"))?;
    let mut config = pl::Config::new();
    if let Some(m) = p.get("config").filter(|v| !v.is_null()) {
        let m = m
            .as_object()
            .ok_or_else(|| RpcErr::params("config はオブジェクト"))?;
        for (k, v) in m {
            let x = match v {
                Value::Number(n) => pl::Value::Num(n.as_f64().unwrap_or(f64::NAN)),
                Value::String(t) => pl::Value::Str(t.clone()),
                Value::Bool(b) => pl::Value::Num(*b as u8 as f64),
                Value::Array(a) => pl::Value::Nums(
                    a.iter()
                        .map(|x| x.as_f64())
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| RpcErr::params(format!("config の {k} は数の並び")))?,
                ),
                _ => return Err(RpcErr::params(format!("config の {k} の値 {v}"))),
            };
            config.insert(k.clone(), x);
        }
    }
    let mut inputs = pl::Arrays::new();
    if let Some(m) = p.get("inputs").filter(|v| !v.is_null()) {
        let m = m
            .as_object()
            .ok_or_else(|| RpcErr::params("inputs はオブジェクト"))?;
        for (k, v) in m {
            let (mut shape, mut out) = (Vec::new(), Vec::new());
            nested(v, &mut shape, &mut out, 0)
                .map_err(|e| RpcErr::params(format!("inputs の {k}: {e}")))?;
            inputs.insert(k.clone(), Array::f64(shape, out)?);
        }
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
    Ok((json!({ "name": name, "values": values }), list))
}

/// 正規化瞳座標 (px, py) の光線を追う。結果の配列は最後の面の局所座標（global なら大域）
pub(super) fn trace(sess: &Session, p: &Value, bins: Vec<Bin>) -> Result<Reply, RpcErr> {
    let (s, _) = sess.current()?;
    let mut bins: HashMap<String, Bin> = bins.into_iter().map(|b| (b.name.clone(), b)).collect();
    let px = floats(p, &mut bins, "px")?.ok_or_else(|| RpcErr::params("trace には px"))?;
    let py = floats(p, &mut bins, "py")?.ok_or_else(|| RpcErr::params("trace には py"))?;
    if px.len() != py.len() {
        return Err(RpcErr::params(format!(
            "px と py の長さが違う（{} と {}）",
            px.len(),
            py.len()
        )));
    }
    let w = match opt_usize(p, "wave")? {
        None => s.primary,
        Some(i) if i >= 1 && i <= s.wavelengths.len() => i - 1,
        Some(i) => {
            return Err(RpcErr::params(format!(
                "波長 {i} はありません（1〜{}）",
                s.wavelengths.len()
            )))
        }
    };
    let (hx, hy) = match opt_usize(p, "field")? {
        Some(i) if i >= 1 && i <= s.fields.len() => s.hxhy(i - 1),
        Some(i) => {
            return Err(RpcErr::params(format!(
                "視野 {i} はありません（1〜{}）",
                s.fields.len()
            )))
        }
        None => (
            opt_f64(p, "hx")?.unwrap_or(0.0),
            opt_f64(p, "hy")?.unwrap_or(0.0),
        ),
    };
    let last = opt_usize(p, "last")?;
    if let Some(l) = last {
        if l == 0 || l > s.image() {
            return Err(RpcErr::params(format!("last {l} は 1〜{}", s.image())));
        }
    }
    let global = p.get("global").and_then(Value::as_bool).unwrap_or(false);
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
        err.push(r.err as i32);
        vig.push(r.vig as i32);
        if r.err == 0 && r.vig == 0 {
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
) -> Result<(), String> {
    match v {
        Value::Number(n) => {
            if depth != shape.len() {
                return Err("並びの深さがそろわない".into());
            }
            out.push(n.as_f64().ok_or("数")?);
            Ok(())
        }
        Value::Array(a) => {
            if depth == shape.len() {
                if !out.is_empty() {
                    return Err("並びの深さがそろわない".into());
                }
                shape.push(a.len());
            } else if shape[depth] != a.len() {
                return Err("並びの長さがそろわない".into());
            }
            for x in a {
                nested(x, shape, out, depth + 1)?;
            }
            Ok(())
        }
        _ => Err(format!("数の並びでない（{v}）")),
    }
}
