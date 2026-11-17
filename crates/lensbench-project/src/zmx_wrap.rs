//! .zmx の系を、.zmx を 1 つの zmx ブロックとして取り込むプロジェクトに書き換える（GUI が .zmx を組合せとして
//! 開くため）。作り直した系が元の系と同じかは [`same_system`] で確かめる

use super::*;
use serde_json::json;

/// [`zmx_project`] の zmx ブロックの id（GLRS をブロックの面で書くとき）
pub const ZMX_ID: &str = "zmx";

/// .zmx（`path`）の構成ごとの系 cfgs（cfgs[k - 1] が構成 k）の設定と、`path` をまるごと取り込む zmx ブロック 1 つの
/// プロジェクト。系 1 つで多重構成の表があれば、表から構成ごとの系を作る。構成が 2 つ以上なら、本体は構成 1 で、
/// 構成ごとに状態「Config k」を足す（zmx ブロックの config と、構成で変わる系の値）。
/// プロジェクトで書けない設定（条件で定める視野）があれば誤り
pub fn zmx_project(cfgs: &[System], path: &str) -> lensbench_core::Result<Value> {
    let from_table: Vec<System>;
    let cfgs = match cfgs {
        [] => return Err("no system".into()),
        [s] => match s.configs().map(|c| c.n) {
            Some(n) => {
                from_table = (1..=n)
                    .map(|k| s.config(k))
                    .collect::<lensbench_core::Result<_>>()?;
                &from_table[..]
            }
            None => {
                let p = settings(s, path)?;
                Project::from_json(&p.to_string())?;
                return Ok(p);
            }
        },
        _ => cfgs,
    };
    let mut p = settings(&cfgs[0], path)?;
    p["blocks"][0]["id"] = json!(ZMX_ID);
    let mut states = Vec::new();
    for (i, s) in cfgs.iter().enumerate() {
        let q = settings(s, path)?;
        let (Some(a), Some(b)) = (p.as_object(), q.as_object()) else {
            unreachable!("settings write an object");
        };
        let mut project = serde_json::Map::new();
        for (key, v) in b {
            if key != "blocks" && a.get(key) != Some(v) {
                project.insert(key.clone(), v.clone());
            }
        }
        // 構成 1 にあって構成 k で省いた値は既定に戻す
        for key in a.keys().filter(|key| !b.contains_key(*key)) {
            let d = match key.as_str() {
                "telecentric" => json!(false),
                "floating_clip" => json!(true),
                "environment" => {
                    json!({ "temperature_c": 20.0, "pressure_atm": 1.0, "adjust_index": false })
                }
                _ => Value::Null,
            };
            project.insert(key.clone(), d);
        }
        let k = i + 1;
        states.push(json!({
            "name": format!("Config {k}"),
            "set": { ZMX_ID: { "config": k } },
            "project": project,
        }));
    }
    p["states"] = json!(states);
    // 書いた形を読めるか、状態を当てられるか
    let pj = Project::from_json(&p.to_string())?;
    for st in pj.state_names() {
        pj.with_state(st)?;
    }
    Ok(p)
}

/// 系 s の設定と zmx ブロック 1 つのプロジェクト（多重構成の表は見ない）
fn settings(s: &System, path: &str) -> lensbench_core::Result<Value> {
    let ap = match s.aperture.0 {
        ApertureKind::Epd => "epd",
        ApertureKind::FNum => "fnum",
        ApertureKind::Na => "na",
        ApertureKind::Float => "float",
        ApertureKind::Pwfn => "pwfn",
        ApertureKind::Cone => "cone",
    };
    let ft = match s.field_type {
        FieldType::Angle => "angle",
        FieldType::ObjH => "object_height",
        FieldType::ParH => "paraxial_image_height",
        FieldType::RealH => "real_image_height",
        FieldType::Solve(_) => return Err("a field defined by a condition".into()),
    };
    let mut fields = json!({
        "type": ft,
        "points": s.fields.iter().map(|f| [f.x, f.y]).collect::<Vec<_>>(),
        "weights": s.fields.iter().map(|f| f.w).collect::<Vec<_>>(),
    });
    if s.field_norm == FieldNorm::Rect {
        fields["norm"] = json!("rect");
    }
    let vig: Vec<[f64; 5]> = s
        .fields
        .iter()
        .map(|f| [f.vdx, f.vdy, f.vcx, f.vcy, f.van])
        .collect();
    if vig.iter().flatten().any(|&x| x != 0.0) {
        fields["vignetting"] = json!(vig);
    }
    let mut p = json!({
        "name": s.name,
        "wavelengths": s.wavelengths.iter().zip(&s.weights)
            .map(|(um, w)| json!({ "um": um, "weight": w }))
            .collect::<Vec<_>>(),
        "primary": s.primary + 1,
        "aperture": { "type": ap, "value": s.aperture.1 },
        "fields": fields,
        "aiming": match s.aiming {
            Aiming::Off => "off",
            Aiming::Paraxial => "paraxial",
            Aiming::Real => "real",
        },
        "blocks": [{ "type": "zmx", "path": path }],
    });
    let t0 = s.surfaces.first().map_or(f64::INFINITY, |o| o.t);
    if t0.is_finite() {
        p["object_distance"] = json!(t0);
    }
    if s.telecentric {
        p["telecentric"] = json!(true);
    }
    if s.afocal {
        p["afocal"] = json!(s.afocal_unit.code());
    }
    if !s.floating_clip {
        p["floating_clip"] = json!(false);
    }
    // GLRS はブロックの面で書く（前に要素を足しても同じ面を指す）。ブロックの面は座標ブレークを数えないので、
    // 座標ブレーク・物体面・像面は系の面の番号のまま
    let image = s.surfaces.len().saturating_sub(1);
    let cb = |j: usize| s.surfaces[j].is_coord_break();
    if (1..image).contains(&s.glrs) && !cb(s.glrs) {
        p["blocks"][0]["id"] = json!(ZMX_ID);
        let k = (1..s.glrs).filter(|&j| !cb(j)).count();
        p["glrs"] = json!({ "block": ZMX_ID, "surface": k });
    } else if s.glrs != 1 {
        p["glrs"] = json!(s.glrs);
    }
    if s.temperature != 20.0 || s.pressure != 1.0 || s.adjust_index {
        p["environment"] = json!({
            "temperature_c": s.temperature,
            "pressure_atm": s.pressure,
            "adjust_index": s.adjust_index,
        });
    }
    // 像面の形・傾き（平面で傾きなしなら書かない。半直径は描画にだけ使うので書かない）
    if let Some(img) = s.surfaces.last().filter(|_| image > 0) {
        let mut r = crate::sysfile_v2::write::surface_row(img, false)?;
        r.semi = None;
        r.stop = false;
        if r != crate::sysfile_v2::Row::default() {
            p["image"] = serde_json::to_value(&r).map_err(|e| e.to_string())?;
        }
    }
    Ok(p)
}

/// 2 つの系が同じか。名前・読込の注意・ガラスの表（屈折率は面ごとに比べる）・玉の表と、物体面・像面の半直径
/// （描画にだけ使う）は見ない。自動の半直径は光線から求めた値なので 1e-9 mm まで。違えば最初のいくつかの違い
pub fn same_system(a: &System, b: &System) -> lensbench_core::Result<()> {
    let va = comparable(a)?;
    let vb = comparable(b)?;
    let mut out = Vec::new();
    diff(&va, &vb, &mut String::new(), &mut out);
    if out.is_empty() {
        Ok(())
    } else {
        out.truncate(5);
        Err(lensbench_core::Error::Invalid(out.join("; ")))
    }
}

fn comparable(s: &System) -> lensbench_core::Result<Value> {
    let mut v = sysfile::internal_value(s)?;
    let m = v.as_object_mut().ok_or("system is not an object")?;
    for k in ["name", "import_notes", "glasses", "units", "configs"] {
        m.remove(k);
    }
    if let Some(Value::Array(sf)) = m.get_mut("surfaces") {
        let last = sf.len().saturating_sub(1);
        for (i, x) in sf.iter_mut().enumerate() {
            let Some(o) = x.as_object_mut() else { continue };
            if i == 0 || i == last {
                o.remove("semi");
                o.remove("semi_auto");
            } else if o.get("semi_auto") == Some(&Value::Bool(true)) {
                if let Some(r) = o.get("semi").and_then(Value::as_f64) {
                    o.insert("semi".into(), json!((r * 1e9).round() / 1e9));
                }
            }
        }
    }
    Ok(v)
}

fn diff(a: &Value, b: &Value, path: &mut String, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                let n = path.len();
                path.push('/');
                path.push_str(k);
                match (x.get(k), y.get(k)) {
                    (Some(u), Some(w)) => diff(u, w, path, out),
                    _ => out.push(format!("{path} on one side only")),
                }
                path.truncate(n);
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (u, w)) in x.iter().zip(y).enumerate() {
                let n = path.len();
                path.push_str(&format!("/{i}"));
                diff(u, w, path, out);
                path.truncate(n);
            }
        }
        _ if a != b => out.push(format!("{path}: {a} vs {b}")),
        _ => {}
    }
}
