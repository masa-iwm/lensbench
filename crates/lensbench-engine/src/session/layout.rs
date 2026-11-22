//! レイアウト図（layout・layout_export・configs）

use super::*;
use crate::layout::{Draw, Pick, RaySel, View};
use serde::{Deserialize, Serialize};

/// `layout` の params（光線の選び方・見る面・重ねる構成）。`layout_export` は [`LayoutExportReq`]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutReq {
    /// 見る面（yz・xz・3d）
    pub plane: View,
    /// 視野（None で全視野）
    pub fields: Option<Pick>,
    /// 波長（None で主波長）
    pub waves: Option<Pick>,
    /// 瞳の上の並べ方の名前（読めることは確かめてある）
    pub fan: Option<String>,
    pub rays: Option<usize>,
    pub first: Option<usize>,
    pub last: Option<usize>,
    /// 折り返しを展開して描く
    pub unfold: bool,
    /// 光線の色分け（field・wave・cfg、小文字）
    pub color: Option<String>,
    /// 機構部品を取った STEP の詳しい形で描く
    pub mech_detail: bool,
    /// 重ねる構成（.zmx の多重構成・プロジェクトの状態・BS の経路。カンマ区切りか "all"）
    pub cfgs: Option<String>,
    pub states: Option<String>,
    pub beam_paths: Option<String>,
    /// 重ねる光束（名前のカンマ区切りか "all"）
    pub beams: Option<String>,
    /// 構成 k をずらす量 [dx, dy, dz]（空ならずらさない）
    pub offset: Vec<f64>,
}

/// `layout_export` の params: 書き出すファイル（形式は拡張子）と、`layout` と同じ欄と書き出しの細かさ
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutExportReq {
    pub file: String,
    pub layout: LayoutReq,
    pub width: Option<usize>,
    pub segments: Option<usize>,
    pub rings: Option<usize>,
    pub tube: Option<f64>,
}

impl LayoutReq {
    /// serve の params（誤りの文は前と同じ）
    pub(crate) fn parse(p: &Value) -> Result<LayoutReq, RpcErr> {
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
        let plane = match p.get("plane") {
            None | Some(Value::Null) => View::YZ,
            Some(Value::String(x)) => x.parse().map_err(RpcErr::params)?,
            Some(_) => return Err(RpcErr::params("plane must be \"yz\", \"xz\" or \"3d\"")),
        };
        let fan = match p.get("fan").and_then(Value::as_str) {
            Some(f) => {
                f.parse::<lensbench_export::scene::Fan>()
                    .map_err(RpcErr::params)?;
                Some(f.to_string())
            }
            None => None,
        };
        let color = match p.get("color") {
            None | Some(Value::Null) => None,
            Some(Value::String(c)) if c.is_empty() => None,
            Some(Value::String(c)) => {
                c.parse::<lensbench_export::d2::ColorBy>()
                    .map_err(RpcErr::params)?;
                Some(c.to_ascii_lowercase())
            }
            Some(_) => return Err(RpcErr::params("color must be field, wave or cfg")),
        };
        // 構成の選び方（文字列か並び。"all" は全部）。BS の経路は load の beam_path に合わせて beam_paths
        let one = |k: &'static str| -> Result<Option<String>, RpcErr> {
            match p.get(k) {
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
        let offset: Vec<f64> = match p.get("offset") {
            None | Some(Value::Null) => Vec::new(),
            Some(v) => {
                let a: Vec<f64> = v
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default();
                crate::layout::offset(&a)?;
                a
            }
        };
        Ok(LayoutReq {
            plane,
            fields: pick("fields")?,
            waves: pick("waves")?,
            fan,
            rays: opt_usize(p, "rays")?,
            first: opt_usize(p, "first")?,
            last: opt_usize(p, "last")?,
            unfold,
            color,
            mech_detail: p["mech_detail"] == true,
            cfgs: one("cfgs")?,
            states: one("states")?,
            beam_paths: one("beam_paths")?,
            beams: one("beams")?,
            offset,
        })
    }

    /// 光線の色分け
    fn color_by(&self) -> Result<Option<lensbench_export::d2::ColorBy>, RpcErr> {
        self.color
            .as_deref()
            .map(|c| c.parse().map_err(RpcErr::params))
            .transpose()
    }

    /// 重ねる構成
    fn overlay(&self) -> crate::load::Overlay {
        crate::load::Overlay {
            cfgs: self.cfgs.clone(),
            states: self.states.clone(),
            paths: self.beam_paths.clone(),
            beams: self.beams.clone(),
        }
    }

    /// 描く光線の選び方
    fn ray_sel(&self) -> Result<RaySel, RpcErr> {
        Ok(RaySel {
            fields: self.fields.clone(),
            waves: self.waves.clone(),
            fan: match &self.fan {
                Some(f) => Some(f.parse().map_err(RpcErr::params)?),
                None => None,
            },
            rays: self.rays,
            first: self.first,
            last: self.last,
            unfold: self.unfold,
        })
    }

    fn scene_opts(&self, s: &System) -> Result<lensbench_export::scene::Options, RpcErr> {
        Ok(self.ray_sel()?.options(s, self.plane)?)
    }

    /// 機構部品の詳しい形の読み手（mech_detail のとき）
    fn details(&self) -> Option<lensbench_export::detail::Details> {
        self.mech_detail
            .then(lensbench_cad::detail::details)
            .flatten()
    }
}

impl LayoutExportReq {
    pub(crate) fn parse(p: &Value) -> Result<LayoutExportReq, RpcErr> {
        let file = p["file"]
            .as_str()
            .ok_or_else(|| RpcErr::params("layout_export needs file"))?
            .to_string();
        Ok(LayoutExportReq {
            file,
            layout: LayoutReq::parse(p)?,
            width: opt_usize(p, "width")?,
            segments: opt_usize(p, "segments")?,
            rings: opt_usize(p, "rings")?,
            tube: opt_f64(p, "tube")?,
        })
    }
}

/// 断面図（YZ）の図形と、玉・群・面の位置（GUI のレイアウト図）。座標は [z, y] mm（大域座標）
///
/// `cfgs`・`states`・`paths`（並びか "all"）を与えると、その構成（.zmx の多重構成・プロジェクトの状態・BS の経路）を
/// 重ねる。構成 k は `offset` [dx, dy, dz] の k 倍ずらす。.zmx の多重構成は読み込んだ系の表から（読み込んだ後の変更
/// set・edit が入る）。プロジェクトの別の状態・経路は読んだときのファイルから展開し、読み込んだ後の変更を当てる
/// （構成で値の違うパラメータの set は当てずに `notes` に書く）。`surfaces`・`units` は読み込んだ系のもの
pub(super) fn layout(sess: &Session, r: &LayoutReq) -> Result<Value, RpcErr> {
    let c = sess.current_full()?;
    let mut v = layout_with(&c.sys, &c.spec, r, Some(&c))?;
    // ブロックごとの面の範囲（読んだときのもの。面を足す・消す edit の後は空。GUI の組合せの図）
    if let Some(b) = sess.served().as_ref().map(|x| x.blocks.clone()) {
        v["blocks"] = json!(*b);
    }
    Ok(v)
}

/// [`layout`]。loaded は読み込んだ系（None は s をファイルから読んだばかりの系として、重ねる構成もファイルから）
pub(super) fn layout_with(
    s: &System,
    spec: &LoadSpec,
    r: &LayoutReq,
    loaded: Option<&Current>,
) -> Result<Value, RpcErr> {
    let (sc, notes) = layout_scene(s, spec, r, loaded)?;
    let mut v = view_json(s, &sc, r)?;
    if !notes.is_empty() {
        v["notes"] = json!(notes);
    }
    // 光線の色分け（field・wave・cfg）。省略なら書かない（GUI は重ね描きなら構成ごと、ほかは視野ごと）
    if let Some(c) = &r.color {
        v["color"] = json!(c);
    }
    Ok(v)
}

/// layout_export: 2D（svg・png・dxf）は plane の面（"3d" なら yz）、3D（gltf・glb・obj・stl・step）は 3D の場面
/// （unfold は無視）
pub(super) fn layout_export(sess: &Session, r: &LayoutExportReq) -> Result<Value, RpcErr> {
    let file = r.file.as_str();
    let path = sess.files.check_path(Path::new(file), true)?;
    let f = lensbench_export::LayoutFormat::from_path(&path).ok_or_else(|| {
        RpcErr::params(format!(
            "{file}: the extension must be svg, png, dxf, gltf, glb, obj, stl, step or stp"
        ))
    })?;
    let mut q = r.layout.clone();
    if f.is_2d() {
        if q.plane == View::D3 {
            q.plane = View::YZ;
        }
    } else {
        q.plane = View::D3;
        q.unfold = false;
    }
    let c = sess.current_full()?;
    let (s, spec) = (&c.sys, &c.spec);
    let (sc, mut notes) = layout_scene(s, spec, &q, Some(&c))?;
    // 色分けの既定は CLI の layout と同じ（重ね描きなら構成ごと、ほかは視野ごと）
    let draw = Draw {
        plane: if f.is_2d() {
            q.plane.plane()
        } else {
            Plane::YZ
        },
        color: r.layout.color_by()?,
        width: r.width.map(|w| w.min(u32::MAX as usize) as u32),
        segments: r.segments,
        rings: r.rings,
        tube: r.tube,
        mech_detail: r.layout.details(),
    };
    let (o2, o3) = draw.options(r.layout.overlay().any())?;
    let w = lensbench_export::write_layout(&path, &sc, &o2, &o3).map_err(RpcErr::app)?;
    notes.extend(w.notes);
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
    s: &System,
    spec: &LoadSpec,
    r: &LayoutReq,
    loaded: Option<&Current>,
) -> Result<(lensbench_export::scene::Scene, Vec<String>), RpcErr> {
    let sel = r.overlay();
    if !sel.any() {
        return Ok((
            lensbench_export::scene::build(s, &r.scene_opts(s)?)?,
            Vec::new(),
        ));
    }
    // ファイルから展開した構成には、読み込んだ後の変更（set・edit）を当てる
    let (systems, mut notes) =
        crate::load::overlay_in(spec, &sel, loaded.map(|c| (&*c.src, &*c.sys)))?;
    let log = loaded.map_or(&[][..], |c| &c.log[..]);
    let from = loaded.map_or(&[][..], |c| &c.faces[..]);
    let systems = systems
        .into_iter()
        .map(|o| {
            if o.edited {
                return Ok((o.name, o.sys));
            }
            let label = if o.name.is_empty() {
                "configuration".to_string()
            } else {
                o.name.clone()
            };
            replay(o.sys, log, (from, &o.faces), &label, &mut notes).map(|sy| (o.name, sy))
        })
        .collect::<lensbench_core::Result<Vec<_>>>()
        .map_err(RpcErr::app)?;
    let offset = if r.offset.is_empty() {
        [0.0; 3]
    } else {
        crate::layout::offset(&r.offset)?
    };
    use lensbench_export::scene;
    let mut opt = r.scene_opts(s)?;
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

/// 重ねて描ける構成: .zmx は構成の数、プロジェクトは状態の名前と BS の経路（今の状態で）
pub(super) fn configs(sess: &Session) -> Result<Value, RpcErr> {
    let Current { sys, spec, src, .. } = sess.current_full()?;
    let kind = src.kind();
    if kind != lensbench_project::open::FileKind::Project {
        let n = if kind == lensbench_project::open::FileKind::SystemFile {
            1
        } else if let Some(t) = &sys.configs {
            // 多重構成の表を読めていれば表の構成の数（読み込んだ後に set_config_count で変えたものも）
            t.n
        } else {
            // 読み込んだときのファイルの中身から（読み直さない）
            src.configurations()?
        };
        let cfg = active_cfg(&sys, &spec);
        return Ok(json!({ "cfgs": n, "cfg": cfg, "states": [], "beams": [], "paths": [] }));
    }
    let paths = src.paths(&spec.open_spec())?;
    Ok(json!({
        "cfgs": 1,
        "states": src.states(),
        "beams": src.beams(),
        "state": spec.state,
        "paths": paths,
        "path": spec.path,
    }))
}

/// 見る面に合わせて、場面を断面図（yz・xz）か 3D の網（"3d"）の JSON に
fn view_json(
    s: &System,
    sc: &lensbench_export::scene::Scene,
    r: &LayoutReq,
) -> Result<Value, RpcErr> {
    if r.plane == View::D3 {
        return Ok(scene3d_json(s, sc, r.mech_detail));
    }
    let mut v = scene_json(s, sc, r.plane.plane());
    if r.unfold {
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
                "mirror": d.is_mirror(),
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
