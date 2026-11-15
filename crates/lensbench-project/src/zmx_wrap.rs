//! .zmx の系を、.zmx を 1 つの zmx ブロックとして取り込むプロジェクトに書き換える（GUI が .zmx を組合せとして
//! 開くため）。作り直した系が元の系と同じかは [`same_system`] で確かめる

use super::*;
use serde_json::json;

/// [`zmx_project`] の zmx ブロックの id（GLRS をブロックの面で書くとき）
pub const ZMX_ID: &str = "zmx";

/// 系 s（`path` の .zmx を構成 1 で読んだもの）の設定と、`path` をまるごと取り込む zmx ブロック 1 つのプロジェクト。
/// プロジェクトで書けない設定（多重構成・条件で定める視野）があれば誤り
pub fn zmx_project(s: &System, path: &str) -> Result<Value, String> {
    if s.configs.is_some() {
        return Err("a .zmx with two or more configurations".into());
    }
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
    let cb = |j: usize| s.surfaces[j].typ == SurfType::CoordBrk;
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
    // 書いた形を読めるか（知らないキー・型の違いがないか）
    Project::from_json(&p.to_string())?;
    Ok(p)
}

/// 2 つの系が同じか。名前・読込の注意・ガラスの表（屈折率は面ごとに比べる）・玉の表と、物体面・像面の半直径
/// （描画にだけ使う）は見ない。自動の半直径は光線から求めた値なので 1e-9 mm まで。違えば最初のいくつかの違い
pub fn same_system(a: &System, b: &System) -> Result<(), String> {
    let va = comparable(a)?;
    let vb = comparable(b)?;
    let mut out = Vec::new();
    diff(&va, &vb, &mut String::new(), &mut out);
    if out.is_empty() {
        Ok(())
    } else {
        out.truncate(5);
        Err(out.join("; "))
    }
}

fn comparable(s: &System) -> Result<Value, String> {
    let mut v: Value =
        serde_json::from_str(&sysfile::to_string(s, &[])?).map_err(|e| e.to_string())?;
    let mut v = v["system"].take();
    let m = v.as_object_mut().ok_or("system is not an object")?;
    for k in ["name", "import_notes", "glasses", "units"] {
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
