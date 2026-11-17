//! プロジェクトの .json（ブロックで書いた系）を組合せにする（文書モデルの 1 段目、GAR-02・GUX-11）。
//! 組合せから作り直したプロジェクトが元と同じ（[`canon`] で比べる）ときだけ組合せにする。違えば誤りを返し、
//! 呼ぶ側は今までどおり系として開く。.zmx は zmx ブロック 1 つのプロジェクトにして、作り直した系が元の系と
//! 同じときだけ組合せにする（文書モデルの 2 段目）

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::combo::{
    ApShape, Combo, ElemKind, GroupDef, MechPick, Pick, SurfEdit, SurfKind, SurfRow, ZGroup,
    SURF_MORE,
};
use crate::sysset::{Aiming, ApKind, FieldKind, FieldPt, FieldSolve, SolveCond, WavePt};
use lensbench_core::system::ApertureKind;
use lensbench_project::{FieldSolveSpec, FieldsSpec, Project};

type R<T> = Result<T, String>;

impl Combo {
    /// .zmx（構成 1 つだけ）を、まるごと取り込む要素 1 つの組合せにする。プロジェクトで書けない設定（多重構成・
    /// 像面の傾きなど）があり、作り直した系が元と違えば誤り
    pub fn from_zmx(f: &Path, glass_dirs: &[PathBuf], lens_catalogs: &[PathBuf]) -> R<Combo> {
        // 組合せの系は一時のフォルダに書いて読むので、パスは作業フォルダから絶対にしておく
        let f = &absolute(f);
        let o = lensbench_project::open::OpenSpec {
            glass_dirs,
            lens_catalogs,
            cfg: 1,
            ..Default::default()
        };
        let s = lensbench_project::open::open(f, &o)
            .map_err(|e| e.to_string())?
            .system;
        // ブロックのパスはファイルの名前だけ（プロジェクトの相対パスはフォルダ base から）
        let name = f.file_name().ok_or("no file name")?.to_string_lossy();
        let p = lensbench_project::zmx_project(&s, &name)?;
        let base = f.parent().unwrap_or(Path::new(""));
        // 書ける .zmx は要素に分ける（変更は中身へ）。書けなければ .zmx をまるごと取り込む
        if let Some(c) = crate::combo_split::split(&s, &p, base, glass_dirs) {
            return Ok(c);
        }
        let t = lensbench_project::Project::from_json(&p.to_string())
            .and_then(|q| {
                lensbench_project::expand(
                    &q,
                    &lensbench_project::Env::in_dir(base, glass_dirs, None),
                    &Default::default(),
                )
            })
            .map(|x| x.system)?;
        lensbench_project::same_system(&s, &t)?;
        Combo::from_project(&p, base)
    }

    /// プロジェクトの JSON（`gui` は見ない）を組合せにする。相対パスは base から。組合せで同じ系を書けなければ誤り
    pub fn from_project(v: &Value, base: &Path) -> R<Combo> {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.remove("gui");
        }
        absolutize(&mut v, base);
        move_state_off(&mut v);
        // ふつうの読込と同じ検査（知らないキー・検出器）。黙って落とさない
        let p = Project::from_json(&v.to_string())?;
        let mut c = Combo {
            name: p.name.clone(),
            glass_catalogs: p.glass_catalogs.clone(),
            coatings: p.coatings.clone(),
            path: p.path.clone().unwrap_or_default(),
            ..Combo::default()
        };
        settings(&mut c, &v)?;
        let blocks = v["blocks"].as_array().cloned().unwrap_or_default();
        let mut rest = &blocks[..];
        // 前の絞り（id の無い絞りと空気。EPD なら径が開口の値と同じとき）
        if let [s, a, tail @ ..] = rest {
            let semi = s["semi"].as_f64().unwrap_or(0.0);
            let lead = s["type"] == "stop"
                && a["type"] == "air"
                && no_id(s)
                && no_id(a)
                && (c.aperture != ApKind::Epd || c.epd == 2.0 * semi);
            if lead {
                c.epd = 2.0 * semi;
                c.stop_gap = a["t"].as_f64().unwrap_or(0.0);
                rest = tail;
            }
        }
        // 先頭の空気（前の絞りが無い系。往路を戻る鏡は復路でも通るので、物体距離には移さない）
        if let Some((a, tail)) = rest
            .split_first()
            .filter(|(a, _)| a["type"] == "air" && no_id(a))
        {
            c.lead_air = Some(f(&a["t"], 0.0));
            rest = tail;
        }
        c.picks = list(rest, false, &mut c.groups)?;
        refs_unique(&c, &v)?;
        states(&mut c, &p)?;
        mechanics(&mut c, &v)?;
        // 作り直したプロジェクトが元と同じか
        let a = canon(&v)?;
        let b = canon(&c.project())?;
        if a != b {
            return Err(first_diff(&a, &b, ""));
        }
        Ok(c)
    }
}

fn no_id(b: &Value) -> bool {
    b["id"].as_str().is_none_or(str::is_empty)
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn f(v: &Value, d: f64) -> f64 {
    v.as_f64().unwrap_or(d)
}

fn arr<const N: usize>(v: &Value) -> [f64; N] {
    std::array::from_fn(|i| f(&v[i], 0.0))
}

/// ブロック・機構部品の相対パスを base からの絶対パスにする（組合せは作業用のフォルダで系を作るため）
fn absolutize(v: &mut Value, base: &Path) {
    let abs = |x: &mut Value| {
        if let Some(p) = x.as_str() {
            if Path::new(p).is_relative() && !p.starts_with(['%', '$']) && !p.is_empty() {
                *x = json!(base.join(p).display().to_string());
            }
        }
    };
    fn walk(bs: &mut Value, abs: &dyn Fn(&mut Value)) {
        for b in bs.as_array_mut().into_iter().flatten() {
            match b["type"].as_str() {
                Some("lens") => {
                    if b["source"].get("zmx").is_some() {
                        abs(&mut b["source"]["zmx"]);
                    }
                }
                Some("zmx") => abs(&mut b["path"]),
                Some("user") => abs(&mut b["catalog"]),
                Some("group") => walk(&mut b["blocks"], abs),
                Some("splitter") => {
                    for k in ["t", "r", "d"] {
                        if b.get(k).is_some() {
                            walk(&mut b[k], abs);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    walk(&mut v["blocks"], &abs);
    for m in v
        .get_mut("mechanics")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if m.get("catalog").is_some() {
            abs(&mut m["catalog"]);
        }
    }
}

/// 状態で OFF にするときの残し方（`off`）を、ブロックの `off` へ移す。基本で ON のブロックで、OFF にする状態が
/// 全部同じ残し方を書いているときだけ（系は変わらない。組合せは残し方を要素に持つ）
fn move_state_off(v: &mut Value) {
    let Some(sts) = v["states"].as_array().cloned() else {
        return;
    };
    let ids: BTreeSet<String> = sts
        .iter()
        .filter_map(|s| s["set"].as_object())
        .flat_map(|m| {
            m.iter()
                .filter(|(_, x)| x.get("off").is_some())
                .map(|(k, _)| k.clone())
        })
        .collect();
    for id in ids {
        let offs: Vec<Option<&Value>> = sts
            .iter()
            .filter_map(|s| s["set"].get(&id))
            .filter(|x| x["on"] == false)
            .map(|x| x.get("off"))
            .collect();
        let Some(Some(mode)) = offs.first().cloned() else {
            continue;
        };
        let mode = mode.clone();
        if offs.iter().any(|o| *o != Some(&mode)) {
            continue;
        }
        let Some(b) = find_block(&mut v["blocks"], &id) else {
            continue;
        };
        if b["on"] == false || b.get("off").is_some_and(|o| *o != mode) {
            continue;
        }
        b["off"] = mode;
        for st in v["states"].as_array_mut().into_iter().flatten() {
            if let Some(x) = st["set"].get_mut(&id).and_then(Value::as_object_mut) {
                x.remove("off");
            }
        }
    }
}

fn find_block<'a>(bs: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    fn has(b: &Value, id: &str) -> bool {
        b["id"] == id
            || ["blocks", "t", "r", "d"].iter().any(|k| {
                b[*k]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| has(x, id)))
            })
    }
    let b = bs.as_array_mut()?.iter_mut().find(|b| has(b, id))?;
    if b["id"] == id {
        return Some(b);
    }
    let k = ["blocks", "t", "r", "d"].into_iter().find(|k| {
        b[*k]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| has(x, id)))
    })?;
    find_block(&mut b[k], id)
}

/// 系の値（波長・開口・視野・物体距離など）を v から c へ
fn settings(c: &mut Combo, v: &Value) -> R<()> {
    let e = |x: serde_json::Error| x.to_string();
    let waves: Vec<lensbench_project::Wave> =
        serde_json::from_value(v["wavelengths"].clone()).map_err(e)?;
    c.waves = waves
        .iter()
        .map(|w| WavePt {
            um: w.um,
            w: w.weight,
        })
        .collect();
    c.primary = v["primary"].as_u64().unwrap_or(1) as usize;
    let ap: lensbench_project::ApertureSpec =
        serde_json::from_value(v["aperture"].clone()).map_err(e)?;
    c.aperture = match ap.kind {
        ApertureKind::Epd => ApKind::Epd,
        ApertureKind::FNum => ApKind::Fnum,
        ApertureKind::Na => ApKind::Na,
        ApertureKind::Float => ApKind::Float,
        ApertureKind::Pwfn => ApKind::Pwfn,
        ApertureKind::Cone => ApKind::Cone,
    };
    match c.aperture {
        ApKind::Epd => c.epd = ap.value,
        ApKind::Float => {}
        _ => c.ap_value = ap.value,
    }
    let fs: FieldsSpec = match v.get("fields") {
        Some(x) => serde_json::from_value(x.clone()).map_err(e)?,
        None => FieldsSpec::default(),
    };
    c.field_kind = match fs.kind {
        lensbench_project::FieldKind::Angle => FieldKind::Angle,
        lensbench_project::FieldKind::ObjectHeight => FieldKind::ObjectHeight,
        lensbench_project::FieldKind::ParaxialImageHeight => FieldKind::ParaxialImageHeight,
        lensbench_project::FieldKind::RealImageHeight => FieldKind::RealImageHeight,
        lensbench_project::FieldKind::Solve => FieldKind::Solve,
    };
    c.fields = fs
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| FieldPt {
            vig: fs.vignetting.get(i).copied().unwrap_or_default(),
            ..FieldPt::xyw(p[0], p[1], fs.weights.get(i).copied().unwrap_or(1.0))
        })
        .collect();
    c.field_solve = match fs.solve {
        Some(FieldSolveSpec::Position { surface }) => FieldSolve {
            cond: SolveCond::Position,
            s1: surface,
            s2: String::new(),
        },
        Some(FieldSolveSpec::DirSum { s1, s2 }) => FieldSolve {
            cond: SolveCond::DirSum,
            s1,
            s2,
        },
        Some(FieldSolveSpec::DirDiff { s1, s2 }) => FieldSolve {
            cond: SolveCond::DirDiff,
            s1,
            s2,
        },
        None => FieldSolve::default(),
    };
    c.object_distance = v["object_distance"].as_f64();
    c.telecentric = v["telecentric"].as_bool().unwrap_or(false);
    c.aiming = match v["aiming"].as_str() {
        Some("paraxial") => Aiming::Paraxial,
        Some("real") => Aiming::Real,
        None | Some("off") => Aiming::Off,
        Some(a) => return Err(format!("aiming {a}")),
    };
    c.afocal = v["afocal"].as_str().map(str::to_string);
    c.floating_clip = v["floating_clip"].as_bool().unwrap_or(true);
    c.extra = crate::combo::SysExtra::read(v)?;
    Ok(())
}

/// 並びの要素と、その後の空気（t, id）
type Item = (Pick, Option<(f64, String)>);

/// ブロックの並びを要素の並びにする（群は要素の group、分岐のある BS は口の先）。branch は BS の口の先の並び
fn list(bs: &[Value], branch: bool, groups: &mut Vec<GroupDef>) -> R<Vec<Pick>> {
    let items = items(bs, groups, "")?;
    let n = items.len();
    let mut out = Vec::new();
    for (k, (mut p, air)) in items.into_iter().enumerate() {
        // 空気を持たない要素: 分岐のある BS（並びの最後）と、口の先の往路を戻る鏡
        let bare = p.branched || (branch && p.kind == ElemKind::Mirror && p.mirror().back);
        match (bare, air) {
            (true, None) => {
                if p.branched && k + 1 != n {
                    return Err("blocks after a branched splitter".into());
                }
            }
            (false, Some((t, id))) => {
                p.air = t;
                p.air_id = id;
            }
            // 最上位の最後の要素の後に空気が無い: 空気 0 と同じ（[`canon`]）
            (false, None) if !branch && k + 1 == n => p.air = 0.0,
            _ => return Err(format!("air after {}", p.id)),
        }
        out.push(p);
    }
    Ok(out)
}

/// ブロックの並びを（要素, 後の空気）の並びに。群の中は group を付けて平らにする
fn items(bs: &[Value], groups: &mut Vec<GroupDef>, group: &str) -> R<Vec<Item>> {
    let mut out: Vec<Item> = Vec::new();
    for b in bs {
        match b["type"].as_str() {
            Some("air") => {
                let last = out.last_mut().ok_or("air before the first element")?;
                if last.1.is_some() {
                    return Err("two airs in a row".into());
                }
                last.1 = Some((f(&b["t"], 0.0), s(&b["id"])));
            }
            Some("group") => {
                let id = s(&b["id"]);
                let moved = b["decenter"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| f(x, 0.0) != 0.0))
                    || b["tilt"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|x| f(x, 0.0) != 0.0));
                if !group.is_empty() || id.is_empty() || moved || b["on"] == false {
                    return Err(format!("group {id}"));
                }
                if groups.iter().any(|g| g.name == id) {
                    return Err(format!("group {id} twice"));
                }
                let inner = items(
                    b["blocks"].as_array().map_or(&[][..], |a| &a[..]),
                    groups,
                    &id,
                )?;
                if inner.is_empty() || inner.last().is_some_and(|x| x.1.is_some()) {
                    return Err(format!("group {id} ends with air"));
                }
                groups.push(GroupDef {
                    name: id,
                    role: s(&b["role"]),
                    shift: f(&b["shift"], 0.0),
                    ..GroupDef::default()
                });
                out.extend(inner);
            }
            Some(t) if !lensbench_project::BUILTIN_BLOCKS.contains(&t) => {
                return Err(format!("block {t:?}"));
            }
            _ => {
                let mut p = elem_or_raw(b, groups)?;
                p.group = group.to_string();
                out.push((p, None));
            }
        }
    }
    Ok(out)
}

/// 置き方（ON/OFF・偏心・傾き）
fn place(b: &Value, p: &mut Pick) {
    p.on = b["on"].as_bool().unwrap_or(true);
    p.off_air = b["off"] == "air";
    // 開口・遮蔽の偏心は開口の dx・dy（ブロックの置き方は書かない）
    if p.kind != ElemKind::Aperture {
        p.decenter = arr(&b["decenter"]);
        p.tilt = arr(&b["tilt"]);
    }
}

fn semi(v: &Value) -> (f64, bool) {
    (f(v, 0.0), v == "auto")
}

/// 分岐のある BS のブロック（口の先 t・r・d を持つ）
pub(crate) fn is_branched_bs(b: &Value) -> bool {
    b["type"] == "splitter" && ["t", "r", "d"].iter().any(|k| b.get(*k).is_some())
}

/// 要素のブロック 1 つ。組合せの欄で同じブロックを書けなければ、ブロックをそのまま持つ要素に
fn elem_or_raw(b: &Value, groups: &mut Vec<GroupDef>) -> R<Pick> {
    // 面の上書き・操作は要素の edits・ops に（ブロックの中身と分けて持つ）
    let mut b = b.clone();
    let (es, os) = match b.as_object_mut() {
        Some(o) => (o.remove("surface_edits"), o.remove("surface_ops")),
        None => (None, None),
    };
    let mut p = elem_or_raw_inner(&b, groups)?;
    for e in es.as_ref().and_then(Value::as_array).into_iter().flatten() {
        let surface = e["surface"].as_u64().unwrap_or(0) as usize;
        for (k, x) in e["fields"].as_object().into_iter().flatten() {
            p.edits.push(SurfEdit {
                surface,
                field: k.clone(),
                value: x.clone(),
            });
        }
    }
    if let Some(x) = os {
        p.ops = serde_json::from_value(x).map_err(|e| e.to_string())?;
    }
    Ok(p)
}

fn elem_or_raw_inner(b: &Value, groups: &mut Vec<GroupDef>) -> R<Pick> {
    let same = |p: &Pick| -> bool {
        let one = |v: Value| -> Option<Value> {
            let mut v = v;
            v.as_object_mut()?.remove("id");
            let x: lensbench_project::Block = serde_json::from_value(v).ok()?;
            let mut x = serde_json::to_value(x).ok()?;
            norm(&mut x);
            Some(x)
        };
        let a = one(b.clone());
        a.is_some() && a == one(p.block(""))
    };
    match elem(b, groups) {
        // 分岐した BS は口の先を並びで持つので、ブロック 1 つでは比べない（全体で比べる）
        Ok(p) if p.kind == ElemKind::Splitter || same(&p) => Ok(p),
        // 分岐のある BS はそのまま持てない（後に要素を置けない。口の先は並びで持つ）
        r if is_branched_bs(b) => r,
        _ => {
            let mut p = Pick::raw(b);
            p.id = s(&b["id"]);
            Ok(p)
        }
    }
}

/// 要素のブロック 1 つ
fn elem(b: &Value, groups: &mut Vec<GroupDef>) -> R<Pick> {
    let src = &b["source"];
    let mut p = match b["type"].as_str() {
        Some("lens") if src.get("vendor").is_some() => {
            let mut p = Pick::new(ElemKind::Catalog);
            p.vendor = s(&src["vendor"]);
            p.part = s(&src["part"]);
            p
        }
        Some("lens") if src.get("zmx").is_some() => {
            let mut p = Pick::new(ElemKind::Zmx);
            p.path = s(&src["zmx"]);
            p.zmx_mut().lens = src["lens"].as_u64().unwrap_or(1) as usize;
            p.zmx_mut().config = src["config"].as_u64().unwrap_or(1) as usize;
            p
        }
        Some("lens") => lens(src["surfaces"].as_array().map_or(&[][..], |a| &a[..]))?,
        Some("user") => {
            let mut p = Pick::new(ElemKind::Catalog);
            p.path = s(&b["catalog"]);
            p.part = s(&b["item"]);
            p
        }
        Some("mirror") => {
            if b.get("aperture").is_some() {
                return Err("mirror aperture".into());
            }
            let mut p = Pick::new(ElemKind::Mirror);
            p.mirror_mut().radius = f(&b["radius"], 0.0);
            p.mirror_mut().conic = f(&b["conic"], 0.0);
            let t: [f64; 2] = arr(&b["tilt"]);
            p.tilt = [t[0], t[1], 0.0];
            p.semi = f(&b["semi"], 0.0);
            p.coat = s(&b["coat"]);
            p.mirror_mut().back = b["back"] == true;
            p.on = b["on"].as_bool().unwrap_or(true);
            p.id = s(&b["id"]);
            return Ok(p);
        }
        Some("stop") => {
            let mut p = Pick::new(ElemKind::Stop);
            p.semi = f(&b["semi"], 0.0);
            p.id = s(&b["id"]);
            return Ok(p);
        }
        Some("surfaces") if aperture_of(b).is_some() => aperture_of(b).unwrap_or_default(),
        Some("surfaces") => {
            let mut p = Pick::new(ElemKind::Surfaces);
            p.surfs = b["surfaces"]
                .as_array()
                .into_iter()
                .flatten()
                .map(row)
                .collect();
            p
        }
        Some("zmx") => {
            let mut p = Pick::new(ElemKind::ZmxRange);
            p.path = s(&b["path"]);
            p.zmx_mut().range = [
                b["from"].as_u64().unwrap_or(0) as usize,
                b["to"].as_u64().unwrap_or(0) as usize,
            ];
            p.zmx_mut().config = b["config"].as_u64().unwrap_or(1) as usize;
            p.stop_front = b["stop"].as_bool().unwrap_or(true);
            for g in b["groups"].as_array().into_iter().flatten() {
                // 基本の移動・置き方は組合せの欄に無い（比べると違うので組合せにしない）
                let (id, name) = (s(&g["id"]), s(&g["name"]));
                p.zmx_mut().zgroups.push(ZGroup {
                    from: g["from"].as_u64().unwrap_or(0) as usize,
                    to: g["to"].as_u64().unwrap_or(0) as usize,
                    id: if id == name.trim() { String::new() } else { id },
                    name,
                    role: s(&g["role"]),
                    ..ZGroup::default()
                });
            }
            p
        }
        Some("splitter") => {
            let mut p = Pick::new(ElemKind::Splitter);
            let mut extra = b.clone();
            if let Some(o) = extra.as_object_mut() {
                for k in ["t", "r", "d", "on", "off", "decenter", "tilt"] {
                    o.remove(k);
                }
            }
            p.extra = extra;
            p.absorb_extra();
            for (k, key) in ["t", "r", "d"].into_iter().enumerate() {
                let Some(a) = b[key].as_array() else { continue };
                p.branched = true;
                let (air, rest) = match a.split_first() {
                    Some((x, rest)) if x["type"] == "air" && no_id(x) => (f(&x["t"], 0.0), rest),
                    _ => return Err(format!("splitter {key} starts without air")),
                };
                p.ports[k].air = air;
                p.ports[k].picks = list(rest, true, groups)?;
            }
            p
        }
        t => return Err(format!("block {t:?}")),
    };
    if matches!(
        p.kind,
        ElemKind::Catalog | ElemKind::Zmx | ElemKind::Lens | ElemKind::Plate
    ) {
        p.reversed = b["reversed"] == true;
        p.stop_front = b["stop"] == true;
    }
    place(b, &mut p);
    p.id = s(&b["id"]);
    Ok(p)
}

/// 面を直接書いた玉（2 面で半径・ガラスがそろえば欄の玉、ほかは面の表）
fn lens(ss: &[Value]) -> R<Pick> {
    if ss.len() < 2 {
        return Err("lens with less than 2 surfaces".into());
    }
    let mut p = Pick::new(ElemKind::Lens);
    let simple = ss.len() == 2
        && semi(&ss[0]["semi"]) == semi(&ss[1]["semi"])
        && f(&ss[0]["conic"], 0.0) == 0.0
        && f(&ss[1]["conic"], 0.0) == 0.0
        && ss[0]["glass"].is_string()
        && ss.iter().all(|x| row(x).more.is_empty());
    if simple {
        p.r1 = f(&ss[0]["radius"], 0.0);
        p.r2 = f(&ss[1]["radius"], 0.0);
        p.t = f(&ss[0]["t"], 0.0);
        p.glass = s(&ss[0]["glass"]);
        (p.semi, p.semi_auto) = semi(&ss[0]["semi"]);
        if p.r1 == 0.0 && p.r2 == 0.0 {
            p.kind = ElemKind::Plate;
        }
    } else {
        p.surfs = ss.iter().map(row).collect();
    }
    Ok(p)
}

/// 面 1 つ（欄の無い SurfSpec の値は more に持つ。ほかの書けない項目は落ちる。比べると違うので組合せにしない）
fn row(x: &Value) -> SurfRow {
    let mut r = SurfRow {
        radius: f(&x["radius"], 0.0),
        conic: f(&x["conic"], 0.0),
        t: f(&x["t"], 0.0),
        glass: s(&x["glass"]),
        ..SurfRow::default()
    };
    (r.semi, r.semi_auto) = semi(&x["semi"]);
    for k in SURF_MORE {
        match x.get(*k) {
            None | Some(Value::Null) => {}
            Some(Value::String(t)) if t.is_empty() => {}
            Some(Value::Array(a)) if a.is_empty() => {}
            Some(v) => {
                r.more.insert(k.to_string(), v.clone());
            }
        }
    }
    if let Some(j) = x.get("jones") {
        r.kind = SurfKind::Jones;
        r.jones = arr(j);
    } else if let Some(c) = x.get("coordbrk") {
        r.kind = SurfKind::Coordbrk;
        r.cb = arr(c);
        r.order = x["order"].as_u64().unwrap_or(0) as u8;
    } else if let Some(pol) = x.get("polarizer") {
        r.kind = SurfKind::Polarizer;
        r.axis_deg = f(&pol["axis_deg"], 0.0);
        r.transmittance = f(&pol["transmittance"], 1.0);
        r.extinction_ratio = f(&pol["extinction_ratio"], 0.0);
    } else if let Some(wp) = x.get("waveplate") {
        r.kind = SurfKind::Waveplate;
        r.retardance = f(&wp["retardance"], 0.25);
        r.wavelength_um = f(&wp["wavelength_um"], 0.6328);
        r.axis_deg = f(&wp["axis_deg"], 0.0);
        r.wp_order = wp["order"].as_u64().unwrap_or(0) as u32;
        r.transmittance = f(&wp["transmittance"], 1.0);
        if let Some(pl) = wp["plates"].as_array() {
            r.wp_material = s(&pl[0]["material"]);
            r.wp_thickness = f(&pl[0]["thickness_mm"], 0.0);
            if let Some(b) = pl.get(1) {
                r.wp_material2 = s(&b["material"]);
                r.wp_thickness2 = f(&b["thickness_mm"], 0.0);
            }
        } else {
            r.wp_material = s(&wp["material"]);
            r.wp_thickness = f(&wp["thickness_mm"], 0.0);
        }
    }
    r
}

/// 状態・面の上書き・面の操作・機構部品が指す id は、要素 1 つだけのもの（重なった id は組合せが付け直すので、
/// 指す先が 1 つ目だけになる。元の読込は誤り）
fn refs_unique(c: &Combo, v: &Value) -> R<()> {
    let mut seen = BTreeSet::new();
    let dup: BTreeSet<String> = c
        .all()
        .iter()
        .filter_map(|l| c.get(l))
        .filter(|p| !p.id.is_empty() && !seen.insert(p.id.clone()))
        .map(|p| p.id.clone())
        .collect();
    if dup.is_empty() {
        return Ok(());
    }
    let mut used: Vec<String> = Vec::new();
    for st in v["states"].as_array().into_iter().flatten() {
        used.extend(
            st["set"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, _)| k.clone()),
        );
    }
    fn blocks_in(x: &Value, out: &mut Vec<String>) {
        match x {
            Value::Object(o) => {
                if let Some(b) = o.get("block").and_then(Value::as_str) {
                    out.push(b.to_string());
                }
                o.values().for_each(|y| blocks_in(y, out));
            }
            Value::Array(a) => a.iter().for_each(|y| blocks_in(y, out)),
            _ => {}
        }
    }
    // ブロックの surface_ops のほかのブロックの面
    blocks_in(&v["blocks"], &mut used);
    for m in v["mechanics"].as_array().into_iter().flatten() {
        blocks_in(&m["at"], &mut used);
        used.extend(m["holds"].as_array().into_iter().flatten().map(s));
    }
    match used.into_iter().find(|u| dup.contains(u)) {
        Some(id) => Err(format!("two or more blocks with id {id}")),
        None => Ok(()),
    }
}

/// 要素（分岐の先も）を id で
fn find<'a>(ps: &'a mut [Pick], id: &str) -> Option<&'a mut Pick> {
    for p in ps {
        if p.id == id {
            return Some(p);
        }
        for b in p.ports.iter_mut() {
            if let Some(x) = find(&mut b.picks, id) {
                return Some(x);
            }
        }
    }
    None
}

/// .zmx の取り込みの中の群を id で
fn find_zgroup<'a>(ps: &'a mut [Pick], id: &str) -> Option<&'a mut ZGroup> {
    for p in ps {
        if let Some(i) = p.zmx().zgroups.iter().position(|g| g.key() == id) {
            return p.zmx_mut().zgroups.get_mut(i);
        }
        for b in p.ports.iter_mut() {
            if let Some(x) = find_zgroup(&mut b.picks, id) {
                return Some(x);
            }
        }
    }
    None
}

/// 後の空気の id で要素を
fn find_air<'a>(ps: &'a mut [Pick], id: &str) -> Option<&'a mut Pick> {
    for p in ps {
        if p.air_id == id {
            return Some(p);
        }
        for b in p.ports.iter_mut() {
            if let Some(x) = find_air(&mut b.picks, id) {
                return Some(x);
            }
        }
    }
    None
}

/// 状態（要素の ON/OFF・置き方、空気、群の移動、系の値、BS の経路）
fn states(c: &mut Combo, p: &Project) -> R<()> {
    let base = c.clone();
    let base_v = c.project();
    for st in &p.states {
        let n = st.name.clone();
        c.states.push(n.clone());
        let mut merged = base_v.clone();
        for (k, x) in &st.project {
            match k.as_str() {
                "path" => {
                    c.state_path.insert(n.clone(), s(x));
                }
                "wavelengths" | "primary" | "aperture" | "fields" | "object_distance"
                | "telecentric" | "aiming" | "afocal" => {
                    if x.is_null() {
                        merged.as_object_mut().map(|o| o.remove(k));
                    } else {
                        merged[k] = x.clone();
                    }
                }
                _ => return Err(format!("state project {k}")),
            }
        }
        let mut t = base.clone();
        settings(&mut t, &merged)?;
        if t.snap() != base.snap() {
            c.state_sys.insert(n.clone(), t.snap());
        }
        for (id, set) in &st.set {
            if let Some(g) = c.groups.iter_mut().find(|g| g.name == *id) {
                for (k, x) in set {
                    match k.as_str() {
                        "shift" => {
                            g.state_shift.insert(n.clone(), f(x, 0.0));
                        }
                        _ => return Err(format!("state {n} {id} {k}")),
                    }
                }
                continue;
            }
            if let Some(g) = find_zgroup(&mut c.picks, id) {
                for (k, x) in set {
                    match k.as_str() {
                        "shift" => {
                            g.state_shift.insert(n.clone(), f(x, 0.0));
                        }
                        "decenter" | "tilt" => {
                            let e = g.state_place.entry(n.clone()).or_default();
                            if k == "decenter" {
                                e.decenter = arr(x);
                            } else {
                                e.tilt = arr(x);
                            }
                        }
                        _ => return Err(format!("state {n} {id} {k}")),
                    }
                }
                continue;
            }
            if let Some(q) = find_air(&mut c.picks, id) {
                for (k, x) in set {
                    match k.as_str() {
                        "t" => {
                            q.state_air.insert(n.clone(), f(x, 0.0));
                        }
                        _ => return Err(format!("state {n} {id} {k}")),
                    }
                }
                continue;
            }
            let q = find(&mut c.picks, id).ok_or_else(|| format!("state {n}: no {id}"))?;
            for (k, x) in set {
                match k.as_str() {
                    "on" => {
                        q.state_on.insert(n.clone(), x == true);
                    }
                    "decenter" | "tilt" => {
                        let (dc, tl) = (q.decenter, q.tilt);
                        let e =
                            q.state_place
                                .entry(n.clone())
                                .or_insert(crate::combo::StatePlace {
                                    decenter: dc,
                                    tilt: tl,
                                });
                        if k == "decenter" {
                            e.decenter = arr(x);
                        } else {
                            e.tilt = arr(x);
                        }
                    }
                    _ => return Err(format!("state {n} {id} {k}")),
                }
            }
        }
    }
    Ok(())
}

fn mechanics(c: &mut Combo, v: &Value) -> R<()> {
    for m in v["mechanics"].as_array().into_iter().flatten() {
        if m.get("mech").is_some() {
            return Err("inline mech".into());
        }
        let at = &m["at"];
        if at.get("block").is_none() && at.get("surface").is_some_and(|x| x != 1) {
            return Err("mechanics at a system surface".into());
        }
        c.mechanics.push(MechPick {
            name: s(&m["name"]),
            catalog: s(&m["catalog"]),
            item: s(&m["item"]),
            at: s(&at["block"]),
            face: at["surface"].as_u64().unwrap_or(0) as usize,
            offset: arr(&m["offset"]),
            tilt: arr(&m["tilt"]),
            holds: m["holds"].as_array().into_iter().flatten().map(s).collect(),
            seat: m["seat"] == true,
            after: s(&m["after"]),
            reversed: m["reversed"] == true,
        });
    }
    Ok(())
}

/// 比べる形: 型（[`Project`]）を通して既定を埋め、意味の同じ書き方をそろえる。要素のブロックの id は比べない（元の id を
/// 要素に残すので同じになる。元で重なった id は組合せが付け直す）。状態・面の上書きなどが指す id は残る
fn canon(v: &Value) -> R<Value> {
    let mut v = v.clone();
    if let Some(o) = v.as_object_mut() {
        o.remove("gui");
    }
    fn strip_ids(bs: &mut Value) {
        for b in bs.as_array_mut().into_iter().flatten() {
            if !matches!(b["type"].as_str(), Some("air" | "group")) {
                b.as_object_mut().map(|o| o.remove("id"));
            }
            for k in ["blocks", "t", "r", "d"] {
                if b.get(k).is_some_and(Value::is_array) {
                    strip_ids(&mut b[k]);
                }
            }
        }
    }
    if let Some(bs) = v.get_mut("blocks") {
        strip_ids(bs);
    }
    // 最後の空気 0（id 無し）は書かないのと同じ（最後の面の厚さに 0 を足す）
    if let Some(bs) = v.get_mut("blocks").and_then(Value::as_array_mut) {
        if bs
            .last()
            .is_some_and(|b| b["type"] == "air" && no_id(b) && f(&b["t"], 1.0) == 0.0)
        {
            bs.pop();
        }
    }
    let p: Project = serde_json::from_value(v).map_err(|e| e.to_string())?;
    let mut v = serde_json::to_value(&p).map_err(|e| e.to_string())?;
    // 視野の重みが全部 1 なら書かないのと同じ
    if v["fields"]["weights"]
        .as_array()
        .is_some_and(|w| w.iter().all(|x| x.as_f64() == Some(1.0)))
    {
        v["fields"]["weights"] = json!([]);
    }
    // ケラレ係数は書かない視野が 0、全部 0 なら書かないのと同じ
    let n = v["fields"]["points"].as_array().map_or(0, Vec::len);
    if let Some(vg) = v["fields"]["vignetting"].as_array_mut() {
        vg.resize(n.max(vg.len()), json!([0.0, 0.0, 0.0, 0.0, 0.0]));
        if vg
            .iter()
            .flat_map(|r| r.as_array().into_iter().flatten())
            .all(|x| x.as_f64() == Some(0.0))
        {
            vg.clear();
        }
    }
    // ブロックの面の上書きは 1 項目ずつ、面・項目の順に（同じ項目の順は残す）
    fn one_by_one(bs: &mut Value) {
        for b in bs.as_array_mut().into_iter().flatten() {
            if let Some(es) = b["surface_edits"].as_array() {
                let mut one: Vec<Value> = es
                    .iter()
                    .flat_map(|e| {
                        e["fields"].as_object().into_iter().flatten().map(
                            |(k, x)| json!({ "surface": e["surface"], "fields": { k.clone(): x } }),
                        )
                    })
                    .collect();
                one.sort_by_key(|e| {
                    (
                        e["surface"].as_u64(),
                        e["fields"]
                            .as_object()
                            .and_then(|m| m.keys().next().cloned()),
                    )
                });
                b["surface_edits"] = Value::Array(one);
            }
            for k in ["blocks", "t", "r", "d"] {
                if b.get(k).is_some_and(Value::is_array) {
                    one_by_one(&mut b[k]);
                }
            }
        }
    }
    if let Some(bs) = v.get_mut("blocks") {
        one_by_one(bs);
    }
    norm(&mut v);
    // 状態の系の値で基本と同じものは書かないのと同じ
    let top = v.clone();
    for st in v["states"].as_array_mut().into_iter().flatten() {
        if let Some(m) = st.get_mut("project").and_then(Value::as_object_mut) {
            m.retain(|k, x| top.get(k).unwrap_or(&Value::Null) != x);
        }
    }
    Ok(v)
}

/// 数は全部 f64、曲率半径 0 は平面（null）、群の名前（展開に使わない）は外す
fn norm(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(x) = n.as_f64().and_then(serde_json::Number::from_f64) {
                *n = x;
            }
        }
        Value::Array(a) => a.iter_mut().for_each(norm),
        Value::Object(o) => {
            if o.get("radius").and_then(Value::as_f64) == Some(0.0) {
                o.insert("radius".into(), Value::Null);
            }
            if o.get("type").and_then(Value::as_str) == Some("group") {
                o.remove("name");
            }
            o.values_mut().for_each(norm);
        }
        _ => {}
    }
}

/// 最初に違う所（誤りの文）
pub(crate) fn first_diff(a: &Value, b: &Value, at: &str) -> String {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                let (p, q) = (
                    x.get(k).unwrap_or(&Value::Null),
                    y.get(k).unwrap_or(&Value::Null),
                );
                if p != q {
                    return first_diff(p, q, &format!("{at}.{k}"));
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                if p != q {
                    return first_diff(p, q, &format!("{at}[{i}]"));
                }
            }
        }
        _ => {}
    }
    format!("not the same at {at}: {a} / {b}")
}

#[cfg(test)]
#[path = "combo_from_tests.rs"]
mod tests;

/// 作業フォルダからの相対パスを絶対パスにする（Web 版などで作業フォルダが無ければそのまま）
pub(crate) fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    std::env::current_dir().map_or_else(|_| p.to_path_buf(), |d| d.join(p))
}

/// 開口・遮蔽の要素が書くブロック（厚さ 0 の面 1 つに開口だけ、[`Pick::block`]）なら、その要素
fn aperture_of(b: &Value) -> Option<Pick> {
    let [sf] = b["surfaces"].as_array()?.as_slice() else {
        return None;
    };
    let o = sf.as_object()?;
    if o.keys()
        .any(|k| !matches!(k.as_str(), "t" | "semi" | "aperture"))
        || f(&sf["t"], 0.0) != 0.0
    {
        return None;
    }
    let a = &sf["aperture"];
    let mut p = Pick::new(ElemKind::Aperture);
    p.decenter = [f(&a["dx"], 0.0), f(&a["dy"], 0.0)];
    let off = p.decenter[0].hypot(p.decenter[1]);
    match a["type"].as_str()? {
        "circular" | "obscuration" => {
            p.ap_mut().shape = if a["type"] == "circular" {
                ApShape::Circle
            } else {
                ApShape::Obscuration
            };
            p.ap_mut().min = f(&a["min"], 0.0);
            p.semi = f(&a["max"], 0.0);
        }
        "rect" | "ellipse" => {
            p.ap_mut().shape = if a["type"] == "rect" {
                ApShape::Rect
            } else {
                ApShape::Ellipse
            };
            p.ap_mut().half = [f(&a["hx"], 0.0), f(&a["hy"], 0.0)];
        }
        "spider" => {
            p.ap_mut().shape = ApShape::Spider;
            p.ap_mut().spider_width = f(&a["width"], 0.0);
            p.ap_mut().spider_arms = a["arms"].as_u64().unwrap_or(1) as u32;
            p.semi = f(&sf["semi"], 0.0) - off;
        }
        _ => return None,
    }
    // 書いたブロックと同じでなければ（手で書いた面など）面の並びのまま
    let mut q = p.clone();
    q.id = s(&b["id"]);
    (q.block(&q.id.clone())["surfaces"] == b["surfaces"]).then_some(p)
}
