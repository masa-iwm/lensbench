//! レイアウト図（layout・layout_export・configs）

use super::*;
use crate::layout::{Draw, Pick, RaySel, View};

/// 断面図（YZ）の図形と、玉・群・面の位置（GUI のレイアウト図）。座標は [z, y] mm（大域座標）
///
/// `cfgs`・`states`・`paths`（並びか "all"）を与えると、その構成（.zmx の多重構成・プロジェクトの状態・BS の経路）を
/// ファイルから読み直して重ねる。構成 k は `offset` [dx, dy, dz] の k 倍ずらす。読み込んだ後の変更（set・edit）は
/// 読み直した構成にも当てる（構成で値の違うパラメータの set は当てずに `notes` に書く）。
/// `surfaces`・`units` は読み込んだ系のもの
pub(super) fn layout(sess: &Session, p: &Value) -> Result<Value, RpcErr> {
    let (s, spec) = sess.current()?;
    layout_with(sess, &s, &spec, p)
}

pub(super) fn layout_with(
    sess: &Session,
    s: &System,
    spec: &LoadSpec,
    p: &Value,
) -> Result<Value, RpcErr> {
    let (sc, notes) = layout_scene(sess, s, spec, p)?;
    let mut v = view_json(s, &sc, p)?;
    if !notes.is_empty() {
        v["notes"] = json!(notes);
    }
    // 光線の色分け（field・wave・cfg）。省略なら書かない（GUI は重ね描きなら構成ごと、ほかは視野ごと）
    if color_of(p)?.is_some() {
        v["color"] = json!(p["color"].as_str().unwrap_or_default().to_ascii_lowercase());
    }
    Ok(v)
}

/// layout・layout_export の color（field・wave・cfg、省略で None）
fn color_of(p: &Value) -> Result<Option<lensbench_export::d2::ColorBy>, RpcErr> {
    match p.get("color") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(c)) if c.is_empty() => Ok(None),
        Some(Value::String(c)) => c.parse().map(Some).map_err(RpcErr::params),
        Some(_) => Err(RpcErr::params("color must be field, wave or cfg")),
    }
}

/// layout_export: {"file": 書き出すファイル, layout と同じ引数}。2D（svg・png・dxf）は plane の面
/// （"3d" なら yz）、3D（gltf・glb・obj・stl・step）は 3D の場面（unfold は無視）
pub(super) fn layout_export(sess: &Session, p: &Value) -> Result<Value, RpcErr> {
    let file = p["file"]
        .as_str()
        .ok_or_else(|| RpcErr::params("layout_export needs file"))?;
    let path = sess.files.check_path(Path::new(file), true)?;
    let f = lensbench_export::LayoutFormat::from_path(&path).ok_or_else(|| {
        RpcErr::params(format!(
            "{file}: the extension must be svg, png, dxf, gltf, glb, obj, stl, step or stp"
        ))
    })?;
    let mut q = p.clone();
    if let Some(m) = q.as_object_mut() {
        if f.is_2d() {
            if is_3d(p) {
                m.insert("plane".into(), json!("yz"));
            }
        } else {
            m.insert("plane".into(), json!("3d"));
            m.remove("unfold");
        }
    }
    let (s, spec) = sess.current()?;
    let (sc, notes) = layout_scene(sess, &s, &spec, &q)?;
    // 色分けの既定は CLI の layout と同じ（重ね描きなら構成ごと、ほかは視野ごと）
    let draw = Draw {
        plane: if f.is_2d() { plane_of(&q)? } else { Plane::YZ },
        color: color_of(p)?,
        width: opt_usize(p, "width")?.map(|w| w.min(u32::MAX as usize) as u32),
        segments: opt_usize(p, "segments")?,
        rings: opt_usize(p, "rings")?,
        tube: opt_f64(p, "tube")?,
        mech_detail: mech_detail(p),
    };
    let (o2, o3) = draw.options(config_sel(p)?.any())?;
    lensbench_export::write_layout(&path, &sc, &o2, &o3).map_err(RpcErr::app)?;
    Ok(json!({
        "written": path.display().to_string(),
        "format": format!("{f:?}").to_lowercase(),
        "solids": sc.solids.len(),
        "mechanics": sc.mechs.len(),
        "rays": sc.rays.len(),
        "notes": notes,
    }))
}

/// layout・layout_export の場面（構成を選べば重ねる）と、読み直した構成に変更を当てられなかった説明
pub(super) fn layout_scene(
    sess: &Session,
    s: &System,
    spec: &LoadSpec,
    p: &Value,
) -> Result<(lensbench_export::scene::Scene, Vec<String>), RpcErr> {
    let sel = config_sel(p)?;
    if !sel.any() {
        return Ok((
            lensbench_export::scene::build(s, &scene_opts(s, p)?)?,
            Vec::new(),
        ));
    }
    // ファイルから読み直した構成にも、読み込んだ後の変更（set・edit）を当てる
    let log = sess.current_log();
    let mut notes = Vec::new();
    let systems = crate::load::overlay_systems(spec, &sel)?
        .into_iter()
        .map(|(name, sy)| {
            let label = if name.is_empty() {
                "configuration".to_string()
            } else {
                name.clone()
            };
            replay(sy, &log, &label, &mut notes).map(|sy| (name, sy))
        })
        .collect::<Result<Vec<_>, String>>()
        .map_err(RpcErr::app)?;
    let offset = match p.get("offset") {
        None | Some(Value::Null) => [0.0; 3],
        Some(v) => {
            let a: Vec<f64> = v
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default();
            crate::layout::offset(&a)?
        }
    };
    use lensbench_export::scene;
    let mut opt = scene_opts(s, p)?;
    // 描き始める面をそろえる（物体面から描けない構成があれば全部面 1 から。CLI の layout と同じ）
    if opt.first.is_none() && systems.iter().any(|x| scene::auto_first(&x.1) != 0) {
        opt.first = Some(1);
    }
    let mut parts = Vec::with_capacity(systems.len());
    for (name, sy) in &systems {
        let sc = scene::build(sy, &opt).map_err(|x| RpcErr::app(format!("{name}: {x}")))?;
        parts.push((name.clone(), sc));
    }
    Ok((scene::overlay(parts, offset)?, notes))
}

/// layout の構成の選び方（文字列か並び。"all" は全部）
fn config_sel(p: &Value) -> Result<crate::load::Overlay, RpcErr> {
    let one = |k: &'static str| -> Result<Option<String>, RpcErr> {
        // BS の経路は load の beam_path に合わせて beam_paths（旧名 paths は誤り）
        let v = if k == "beam_paths" {
            Some(renamed(p, "beam_paths", "paths")?)
        } else {
            p.get(k)
        };
        match v {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(x)) => Ok(Some(x.clone())),
            Some(Value::Number(x)) => Ok(Some(x.to_string())),
            Some(Value::Array(a)) if !a.is_empty() => a
                .iter()
                .map(|x| match x {
                    Value::String(t) => Ok(t.clone()),
                    Value::Number(n) => Ok(n.to_string()),
                    _ => Err(RpcErr::params(format!(
                        "{k} must be a string or a list of numbers"
                    ))),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|v| Some(v.join(","))),
            _ => Err(RpcErr::params(format!(
                "{k} must be a string or a list of numbers (\"all\" for all)"
            ))),
        }
    };
    Ok(crate::load::Overlay {
        cfgs: one("cfgs")?,
        states: one("states")?,
        paths: one("beam_paths")?,
    })
}

/// 重ねて描ける構成: .zmx は構成の数、プロジェクトは状態の名前と BS の経路（今の状態で）
pub(super) fn configs(sess: &Session) -> Result<Value, RpcErr> {
    let (_, spec) = sess.current()?;
    let kind = lensbench_project::open::FileKind::of(&spec.file);
    if kind != lensbench_project::open::FileKind::Project {
        let n = if kind == lensbench_project::open::FileKind::SystemFile {
            1
        } else {
            let txt = lensbench_core::glass::read_text(&spec.file)
                .map_err(|x| RpcErr::app(format!("{}: {x}", spec.file.display())))?;
            lensbench_core::zmx::parse_text(&txt)
                .map_err(|x| RpcErr::app(x.to_string()))?
                .configurations()
        };
        return Ok(json!({ "cfgs": n, "cfg": spec.cfg, "states": [], "paths": [] }));
    }
    let src = lensbench_project::open::Source::read(&spec.file)?;
    let paths = src.paths(&spec.open_spec())?;
    Ok(json!({
        "cfgs": 1,
        "states": src.states(),
        "state": spec.state,
        "paths": paths,
        "path": spec.path,
    }))
}

fn scene_opts(s: &System, p: &Value) -> Result<lensbench_export::scene::Options, RpcErr> {
    Ok(ray_sel(p)?.options(s, view_of(p)?)?)
}

/// layout の光線の選び方（field・wave は番号か並びか "all"）
fn ray_sel(p: &Value) -> Result<RaySel, RpcErr> {
    let pick = |k: &str| -> Result<Option<Pick>, RpcErr> {
        let bad = || RpcErr::params(format!("{k} must be numbers from 1 or \"all\""));
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(x)) => x.parse().map(Some).map_err(|_| bad()),
            Some(x) => {
                let a = x.as_array().cloned().unwrap_or_else(|| vec![x.clone()]);
                a.iter()
                    .map(|v| v.as_u64().map(|i| i as usize).ok_or_else(bad))
                    .collect::<Result<_, _>>()
                    .map(|v| Some(Pick::List(v)))
            }
        }
    };
    let unfold = match p.get("unfold") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(RpcErr::params("unfold must be true or false")),
    };
    Ok(RaySel {
        fields: pick("field")?,
        waves: pick("wave")?,
        fan: match p.get("fan").and_then(Value::as_str) {
            Some(f) => Some(f.parse().map_err(RpcErr::params)?),
            None => None,
        },
        rays: opt_usize(p, "rays")?,
        first: opt_usize(p, "first")?,
        last: opt_usize(p, "last")?,
        unfold,
    })
}

/// layout の見る面（plane: "yz"（既定）・"xz"・"3d"）
fn view_of(p: &Value) -> Result<View, RpcErr> {
    match p.get("plane") {
        None | Some(Value::Null) => Ok(View::YZ),
        Some(Value::String(x)) => x.parse().map_err(RpcErr::params),
        Some(_) => Err(RpcErr::params("plane must be \"yz\", \"xz\" or \"3d\"")),
    }
}

/// 見る面に合わせて、場面を断面図（yz・xz）か 3D の網（"3d"）の JSON に
fn view_json(s: &System, sc: &lensbench_export::scene::Scene, p: &Value) -> Result<Value, RpcErr> {
    if is_3d(p) {
        return Ok(scene3d_json(s, sc, p["mech_detail"] == true));
    }
    let mut v = scene_json(s, sc, plane_of(p)?);
    if p["unfold"] == true {
        // 面の頂点も展開した位置に（寸法線・群の位置）
        let maps = lensbench_export::scene::unfold_maps(s);
        let fr = s.frames();
        let xz = v["plane"] == "xz";
        if let Some(a) = v["surfaces"].as_array_mut() {
            for (j, x) in a.iter_mut().enumerate() {
                if let (Some(f), Some(m)) = (fr.surf[j], maps.get(j)) {
                    let q = m.0.point(f.o);
                    x["vertex"] = json!(if xz { [q[2], q[0]] } else { [q[2], q[1]] });
                }
            }
        }
        v["unfold"] = json!(true);
    }
    Ok(v)
}

/// layout の 3D 表示か（plane が "3d"）
fn is_3d(p: &Value) -> bool {
    matches!(view_of(p), Ok(View::D3))
}

/// layout の断面図の面（"3d" なら yz）
fn plane_of(p: &Value) -> Result<Plane, RpcErr> {
    Ok(view_of(p)?.plane())
}

/// 要求の `mech_detail`（真なら機構部品を取った STEP の詳しい形で描く読み手、PRJ-21）
fn mech_detail(p: &Value) -> Option<lensbench_export::detail::Details> {
    (p["mech_detail"] == true)
        .then(lensbench_cad::detail::details)
        .flatten()
}

/// 場面を GUI の 3D 表示の JSON に: 立体・面・光線の網（座標は大域 [x, y, z] mm を平らに並べたもの）。
/// 立体と面は `surfaces`（面の番号）を持ち、組合せの要素の選択に使う
fn scene3d_json(s: &System, sc: &lensbench_export::scene::Scene, mech_detail: bool) -> Value {
    use lensbench_export::mesh::{self, MeshKind};
    use lensbench_export::scene;
    let o = mesh::Options3D {
        segments: 32,
        rings: 6,
        mech_detail: mech_detail.then(lensbench_cad::detail::details).flatten(),
        ..Default::default()
    };
    // 立体と面だけ網に（光線は折れ線で別に）
    let only = scene::Scene {
        name: sc.name.clone(),
        solids: sc.solids.clone(),
        plates: sc.plates.clone(),
        mechs: sc.mechs.clone(),
        rays: Vec::new(),
        fields: sc.fields,
        waves: sc.waves.clone(),
        configs: sc.configs.clone(),
    };
    let ms = mesh::meshes(&only, &o, None);
    let ns = sc.solids.len();
    let np = sc.plates.len();
    // 小数 4 桁（0.1 µm）に丸めて JSON を小さく
    let r4 = |v: f64| (v * 1e4).round() / 1e4;
    let meshes: Vec<Value> = ms
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let (surfs, kind) = if i < ns {
                let k = if m.kind == MeshKind::Mirror {
                    "mirror"
                } else {
                    "glass"
                };
                ([sc.solids[i].front.surf, sc.solids[i].back.surf], k)
            } else if i >= ns + np {
                // 機構部品: 面の番号なし
                ([0, 0], "mech")
            } else {
                let pl = &sc.plates[i - ns];
                let k = match pl.kind {
                    scene::PlateKind::Image => "image",
                    scene::PlateKind::Object => "object",
                    scene::PlateKind::Paraxial => "paraxial",
                };
                ([pl.face.surf, pl.face.surf], k)
            };
            json!({
                "name": m.name,
                "kind": kind,
                "surfaces": surfs,
                "pos": m.pos.iter().flat_map(|p| p.map(r4)).collect::<Vec<_>>(),
                "tris": m.tris.iter().flatten().collect::<Vec<_>>(),
            })
        })
        .collect();
    let rays: Vec<Value> = sc
        .rays
        .iter()
        .filter(|r| r.points.len() >= 2)
        .map(|r| {
            json!({
                "field": r.field + 1, "wave": r.wave + 1, "cfg": r.cfg, "px": r.px, "py": r.py,
                "pts": r.points.iter().map(|q| q.map(r4)).collect::<Vec<_>>(),
            })
        })
        .collect();
    let fr = s.frames();
    let surfaces: Vec<Value> = (0..s.surfaces.len())
        .map(|j| json!({ "vertex": fr.surf[j].map(|f| f.o), "stop": j == s.stop() }))
        .collect();
    let units: Vec<Value> = s
        .units
        .iter()
        .map(|u| json!({ "name": u.name, "spans": u.spans }))
        .collect();
    json!({
        "meshes": meshes, "rays": rays, "surfaces": surfaces, "units": units, "configs": sc.configs,
        "plane": "3d",
    })
}

/// 場面を GUI の断面図の JSON に（座標は [z, y]、xz 面なら [z, x]）
fn scene_json(s: &System, sc: &lensbench_export::scene::Scene, plane: Plane) -> Value {
    use lensbench_export::scene;
    let n = 48;
    let xz = plane == Plane::XZ;
    let pr = |q: [f64; 3]| if xz { [q[2], q[0]] } else { [q[2], q[1]] };
    let solid_pts = |sd: &scene::Solid| {
        if xz {
            scene::solid_section_xz(sd, n)
        } else {
            scene::solid_section(sd, n)
        }
    };
    let face_pts = |f: &scene::Face| {
        if xz {
            scene::face_section_xz(f, n)
        } else {
            scene::face_section(f, n)
        }
    };
    let solids: Vec<Value> = sc
        .solids
        .iter()
        .map(|sd| {
            json!({
                "name": sd.name,
                "kind": match sd.kind { scene::SolidKind::Glass => "glass", scene::SolidKind::Mirror => "mirror" },
                "material": sd.material,
                "surfaces": [sd.front.surf, sd.back.surf],
                "pts": solid_pts(sd).into_iter().map(pr).collect::<Vec<_>>(),
            })
        })
        .collect();
    let plates: Vec<Value> = sc
        .plates
        .iter()
        .map(|pl| {
            json!({
                "name": pl.name,
                "kind": match pl.kind {
                    scene::PlateKind::Image => "image",
                    scene::PlateKind::Object => "object",
                    scene::PlateKind::Paraxial => "paraxial",
                },
                "surface": pl.face.surf,
                "pts": face_pts(&pl.face).into_iter().map(pr).collect::<Vec<_>>(),
            })
        })
        .collect();
    let rays: Vec<Value> = sc
        .rays
        .iter()
        .filter(|r| r.points.len() >= 2)
        .map(|r| {
            json!({
                "field": r.field + 1, "wave": r.wave + 1, "cfg": r.cfg, "px": r.px, "py": r.py, "vig": r.vig, "err": r.err,
                "pts": r.points.iter().map(|&q| pr(q)).collect::<Vec<_>>(),
            })
        })
        .collect();
    // 機構部品: 輪の壁の断面の四角
    let mechs: Vec<Value> = sc
        .mechs
        .iter()
        .map(|m| {
            json!({
                "name": m.name,
                "quads": m.section(xz).into_iter().map(|q| q.into_iter().map(pr).collect::<Vec<_>>()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let fr = s.frames();
    // 面の頂点（無視する面・無限遠の物体面は null）と半径
    let surfaces: Vec<Value> = (0..s.surfaces.len())
        .map(|j| {
            let d = &s.surfaces[j];
            json!({
                "vertex": fr.surf[j].map(|f| pr(f.o)),
                "stop": j == s.stop(),
                "mirror": d.mirror,
                "semi": d.semi,
                "t": d.t,
            })
        })
        .collect();
    let units: Vec<Value> = s
        .units
        .iter()
        .map(|u| json!({ "name": u.name, "spans": u.spans, "motion": u.motion, "shift": u.shift, "auto": u.auto }))
        .collect();
    json!({
        "solids": solids, "plates": plates, "mechs": mechs, "rays": rays, "surfaces": surfaces, "units": units,
        "configs": sc.configs, "plane": if xz { "xz" } else { "yz" },
    })
}
