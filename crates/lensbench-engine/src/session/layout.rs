//! レイアウト図（layout・layout_export・configs）

use super::*;

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
        Some(_) => Err(RpcErr::params("color は field・wave・cfg のどれか")),
    }
}

/// layout_export: {"file": 書き出すファイル, layout と同じ引数}。2D（svg・png・dxf）は plane の面
/// （"3d" なら yz）、3D（gltf・glb・obj・stl・step）は 3D の場面（unfold は無視）
pub(super) fn layout_export(sess: &Session, p: &Value) -> Result<Value, RpcErr> {
    let file = p["file"]
        .as_str()
        .ok_or_else(|| RpcErr::params("layout_export には file"))?;
    let path = sess.files.check_path(Path::new(file), true)?;
    let f = lensbench_export::LayoutFormat::from_path(&path).ok_or_else(|| {
        RpcErr::params(format!(
            "{file}: 拡張子は svg・png・dxf・gltf・glb・obj・stl・step・stp のどれか"
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
    let color = match color_of(p)? {
        Some(c) => c,
        None if config_sel(p)?.any() => lensbench_export::d2::ColorBy::Config,
        None => lensbench_export::d2::ColorBy::Field,
    };
    let d2 = lensbench_export::d2::Options2D::default();
    let d3 = lensbench_export::mesh::Options3D::default();
    let segments = opt_usize(p, "segments")?
        .unwrap_or(d2.segments)
        .clamp(4, 1024);
    let o2 = lensbench_export::d2::Options2D {
        plane: if f.is_2d() { plane_of(&q)? } else { Plane::YZ },
        color,
        width: opt_usize(p, "width")?
            .map(|w| w.clamp(100, 20000) as u32)
            .unwrap_or(d2.width),
        segments,
    };
    let tube = match opt_f64(p, "tube")? {
        Some(t) if !(t > 0.0 && t.is_finite()) => {
            return Err(RpcErr::params("tube は正の数 [mm]"));
        }
        t => t,
    };
    let o3 = lensbench_export::mesh::Options3D {
        segments,
        rings: opt_usize(p, "rings")?.unwrap_or(d3.rings).clamp(1, 256),
        tube,
        color,
        mech_detail: p["mech_detail"] == true,
    };
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
    let systems = crate::load::overlay_systems(&spec, &sel)?
        .into_iter()
        .map(|(name, sy)| {
            let label = if name.is_empty() {
                "構成".to_string()
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
            if a.len() != 3 {
                return Err(RpcErr::params("offset は [dx, dy, dz]"));
            }
            [a[0], a[1], a[2]]
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
    let one = |k: &str| -> Result<Option<String>, RpcErr> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(x)) => Ok(Some(x.clone())),
            Some(Value::Number(x)) => Ok(Some(x.to_string())),
            Some(Value::Array(a)) if !a.is_empty() => a
                .iter()
                .map(|x| match x {
                    Value::String(t) => Ok(t.clone()),
                    Value::Number(n) => Ok(n.to_string()),
                    _ => Err(RpcErr::params(format!("{k} は文字列か番号の並び"))),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|v| Some(v.join(","))),
            _ => Err(RpcErr::params(format!(
                "{k} は文字列か番号の並び（\"all\" で全部）"
            ))),
        }
    };
    Ok(crate::load::Overlay {
        cfgs: one("cfgs")?,
        states: one("states")?,
        paths: one("paths")?,
    })
}

/// 重ねて描ける構成: .zmx は構成の数、プロジェクトは状態の名前と BS の経路（今の状態で）
pub(super) fn configs(sess: &Session) -> Result<Value, RpcErr> {
    let (_, spec) = sess.current()?;
    let is_json = spec
        .file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"));
    if lensbench_project::sysfile::is_system_file(&spec.file) || !is_json {
        let n = if is_json {
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
    let txt = lensbench_core::vfs::read_to_string(&spec.file)
        .map_err(|x| RpcErr::app(format!("{}: {x}", spec.file.display())))?;
    let p = lensbench_project::Project::from_json(&txt)?;
    let ps = match &spec.state {
        Some(n) => p.with_state(n)?,
        None => p.clone(),
    };
    let cat = lensbench_core::catalog::LensCatalog::load(
        &spec.lens_catalog,
        &[&lensbench_core::catalog::ZmxFileSource],
    )?;
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let env = lensbench_project::Env {
        base: spec
            .file
            .parent()
            .map(|x| x.to_path_buf())
            .unwrap_or_default(),
        glass_dirs: spec.glass_dir.clone(),
        catalog: Some(&look),
    };
    let paths = lensbench_project::paths(&ps, &env)?;
    Ok(json!({
        "cfgs": 1,
        "states": p.state_names(),
        "state": spec.state,
        "paths": paths,
        "path": spec.path,
    }))
}

fn scene_opts(s: &System, p: &Value) -> Result<lensbench_export::scene::Options, RpcErr> {
    use lensbench_export::scene;
    // 光線の扇の既定は見る面の向き（xz 面なら x の扇）
    // 3D は光束の形が分かる円周（CLI の 3D の書き出しと同じ）
    let d3 = is_3d(p);
    let fan: scene::Fan = match p.get("fan").and_then(Value::as_str) {
        Some(f) => f.parse().map_err(RpcErr::params)?,
        None if d3 => scene::Fan::Ring,
        None if plane_of(p)? == Plane::XZ => scene::Fan::X,
        None => scene::Fan::Y,
    };
    let unfold = match p.get("unfold") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(RpcErr::params("unfold は true か false")),
    };
    if unfold && d3 {
        return Err(RpcErr::params("unfold は 2D（plane が yz・xz）だけ"));
    }
    let n0 = if fan == scene::Fan::Ring { 12 } else { 7 };
    Ok(scene::Options {
        fields: opt_index_list(p, "field", s.fields.len())?,
        waves: opt_index_list(p, "wave", s.wavelengths.len())?,
        fan,
        rays: opt_usize(p, "rays")?.unwrap_or(n0).clamp(1, 200),
        first: opt_usize(p, "first")?,
        last: opt_usize(p, "last")?,
        unfold,
    })
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
    p["plane"]
        .as_str()
        .is_some_and(|x| x.eq_ignore_ascii_case("3d"))
}

/// layout の見る面（"yz"（既定）か "xz"。"3d" は [`is_3d`]）
fn plane_of(p: &Value) -> Result<Plane, RpcErr> {
    match p.get("plane") {
        None | Some(Value::Null) => Ok(Plane::YZ),
        Some(Value::String(x)) => x.parse().map_err(RpcErr::params),
        Some(_) => Err(RpcErr::params("plane は \"yz\"・\"xz\"・\"3d\"")),
    }
}

/// 場面を GUI の 3D 表示の JSON に: 立体・面・光線の網（座標は大域 [x, y, z] mm を平らに並べたもの）。
/// 立体と面は `surfaces`（面の番号）を持ち、組合せの要素の選択に使う
fn scene3d_json(s: &System, sc: &lensbench_export::scene::Scene, mech_detail: bool) -> Value {
    use lensbench_export::mesh::{self, MeshKind};
    use lensbench_export::scene;
    let o = mesh::Options3D {
        segments: 32,
        rings: 6,
        mech_detail,
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
