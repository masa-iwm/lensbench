//! OpticStudio の順次モードの .zmx を読む。
//!
//! 順次の光線追跡に効くものだけを読み、対応していないもの（未知の面タイプや面の行、扱わない多重構成の
//! オペランドが構成で変わる、非順次、mm 以外の単位など）は `Error::NotImplemented` で止める。

use crate::error::{Error, Result};
use crate::glass::{self, Glass};
use crate::surface::{After, Aperture, SurfType, Surface, Tilt};
use crate::system::{Aiming, ApertureKind, Field, FieldNorm, FieldType, System};
use crate::{invalid, notimpl};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

const HARMLESS: &[&str] = &[
    "HIDE", "MIRR", "SLAB", "POPS", "SSID", "FIMP", "MEMA", "VPAR", "VDSZ", "VCRV", "VCON", "TOLE",
    "MAZH", "OEMA", "BLNK", "NAME",
];
const MCE_HANDLED: &[&str] = &[
    "THIC", "IGNR", "APMX", "SDIA", "STPS", "SATP", "APER", "TELE", "CRVT", "CONN", "WAVE", "GLSS",
    "XFIE", "YFIE", "MOFF", "LTTL",
];

#[derive(Debug, Clone, Default)]
pub struct RawSurf {
    pub n: i64,
    pub typ: Option<String>,
    pub coat: Option<String>,
    pub glas: Option<String>,
    pub model_glass: Option<(f64, f64)>,
    pub curv: Option<f64>,
    pub curv_solve: bool,
    pub coni: Option<f64>,
    pub disz: Option<f64>,
    pub diam: Option<f64>,
    pub comment: String,
    pub parm: BTreeMap<i64, f64>,
    pub ppar: BTreeMap<i64, (usize, f64, f64)>,
    pub pzup: Option<(usize, f64, f64)>,
    pub clap: Option<Vec<f64>>,
    pub sqap: Option<Vec<f64>>,
    pub flap: Option<Vec<f64>>,
    pub obsc: Option<Vec<f64>>,
    pub obdc: Option<Vec<f64>>,
    pub scbd: Vec<Vec<String>>,
    pub stop: bool,
    pub ignore: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Raw {
    pub surfs: Vec<RawSurf>,
    /// (オペランド, 番号, 構成, 値, 残り)
    pub mce: Vec<(String, i64, i64, String, Vec<String>)>,
    pub sysl: HashMap<String, Vec<Vec<String>>>,
}

fn fv(v: &str) -> Result<f64> {
    if v.eq_ignore_ascii_case("INFINITY") {
        return Ok(f64::INFINITY);
    }
    v.parse::<f64>()
        .map_err(|_| Error::Invalid(format!("数値ではない: {v}")))
}

fn mce_line(line: &str) -> Option<(String, i64, i64, String, Vec<String>)> {
    // ^([A-Z0-9]{4})\s+(-?\d+)\s+(\d+)\s+(\S+)(.*)$
    let b = line.as_bytes();
    if b.len() < 4
        || !b[..4]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return None;
    }
    let rest = &line[4..];
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let t: Vec<&str> = rest.split_whitespace().collect();
    if t.len() < 3 {
        return None;
    }
    let p1: i64 = t[0].parse().ok()?;
    if !t[1].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let cfg: i64 = t[1].parse().ok()?;
    Some((
        line[..4].to_string(),
        p1,
        cfg,
        t[2].to_string(),
        t[3..].iter().map(|s| s.to_string()).collect(),
    ))
}

pub fn parse_text(txt: &str) -> Result<Raw> {
    let mut raw = Raw::default();
    let mut cur: Option<usize> = None;
    let mut in_mce = false;
    for line in txt.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.is_empty() {
            continue;
        }
        let key = t[0];
        let rest = || t[1..].iter().map(|s| s.to_string()).collect::<Vec<_>>();
        if key == "SURF" {
            raw.surfs.push(RawSurf {
                n: t.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
                ..Default::default()
            });
            cur = Some(raw.surfs.len() - 1);
            continue;
        }
        if key == "MNUM" {
            in_mce = true;
            cur = None;
            raw.sysl.entry(key.into()).or_default().push(rest());
            continue;
        }
        if in_mce {
            if let Some(m) = mce_line(line) {
                raw.mce.push(m);
            }
            continue;
        }
        let Some(ci) = cur.filter(|_| line.starts_with("  ")) else {
            raw.sysl.entry(key.into()).or_default().push(rest());
            cur = None;
            continue;
        };
        let s = &mut raw.surfs[ci];
        let num = |i: usize| -> Result<f64> { fv(t.get(i).copied().unwrap_or("0")) };
        let nums = || -> Result<Vec<f64>> { t[1..].iter().map(|v| fv(v)).collect() };
        match key {
            "COMM" => s.comment = line.trim().get(5..).unwrap_or("").to_string(),
            "PARM" => {
                s.parm.insert(num(1)? as i64, num(2)?);
            }
            "PPAR" => {
                s.ppar
                    .insert(num(1)? as i64, (num(2)? as usize, num(3)?, num(4)?));
            }
            "PZUP" => s.pzup = Some((num(1)? as usize, num(2)?, num(3)?)),
            "TYPE" => s.typ = Some(t.get(1).unwrap_or(&"").to_string()),
            "COAT" => s.coat = Some(t.get(1).unwrap_or(&"").to_string()),
            "GLAS" => {
                s.glas = Some(t.get(1).unwrap_or(&"").to_string());
                if t.get(1) == Some(&"___BLANK") {
                    s.model_glass = Some((num(4)?, num(5)?));
                }
            }
            "CURV" => {
                s.curv = Some(num(1)?);
                if t.len() > 2 && t[2] != "0" && t[2] != "1" {
                    s.curv_solve = true;
                }
            }
            "CONI" => s.coni = Some(num(1)?),
            "DISZ" => s.disz = Some(num(1)?),
            "DIAM" => s.diam = Some(num(1)?),
            "CLAP" => s.clap = Some(nums()?),
            "SQAP" => s.sqap = Some(nums()?),
            "FLAP" => s.flap = Some(nums()?),
            "OBSC" => s.obsc = Some(nums()?),
            "OBDC" => s.obdc = Some(nums()?),
            "SCBD" => s.scbd.push(rest()),
            "STOP" => s.stop = true,
            "IGNR" => s.ignore = t.get(1).is_some_and(|v| *v != "0"),
            k if HARMLESS.contains(&k) => {}
            k => notimpl!("zmx の面 {}: 行 {} は未対応", s.n, k),
        }
    }
    Ok(raw)
}

fn sys_str<'a>(raw: &'a Raw, key: &str, i: usize) -> Option<&'a str> {
    raw.sysl.get(key)?.first()?.get(i).map(|s| s.as_str())
}
fn sys_f(raw: &Raw, key: &str, i: usize, default: f64) -> f64 {
    sys_str(raw, key, i)
        .and_then(|v| fv(v).ok())
        .unwrap_or(default)
}

/// 読込の追加指定。
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    pub cfg: usize,
    pub catalog_dirs: Vec<PathBuf>,
    /// 波長を置き換える [µm]
    pub wavelengths: Option<Vec<f64>>,
    /// ファイルの GCAT の代わりに使うガラス
    pub glasses: Option<HashMap<String, Glass>>,
}

pub fn load(path: &Path) -> Result<System> {
    load_with(
        path,
        &LoadOptions {
            cfg: 1,
            ..Default::default()
        },
    )
}

pub fn load_with(path: &Path, opt: &LoadOptions) -> Result<System> {
    let txt = glass::read_text(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    let raw = parse_text(&txt)?;
    let mut dirs = opt.catalog_dirs.clone();
    if let Some(p) = path.parent() {
        dirs.push(p.to_path_buf());
    }
    build(&raw, opt, &glass::catalog_dirs(&dirs), path.parent())
}

pub fn build(
    raw: &Raw,
    opt: &LoadOptions,
    dirs: &[PathBuf],
    base: Option<&Path>,
) -> Result<System> {
    let cfg = opt.cfg.max(1) as i64;
    let mode = sys_str(raw, "MODE", 0).unwrap_or("SEQ");
    if mode != "SEQ" {
        notimpl!("モード {mode}（順次のみ）");
    }
    let unit = sys_str(raw, "UNIT", 0).unwrap_or("MM");
    if !unit.eq_ignore_ascii_case("MM") {
        notimpl!("レンズ単位 {unit}（mm のみ）");
    }
    if raw.sysl.contains_key("AFOC") && sys_f(raw, "AFOC", 0, 0.0) != 0.0 {
        notimpl!("無焦点の像空間");
    }
    let ncfg = sys_f(raw, "MNUM", 0, 1.0) as i64;
    if !(1..=ncfg).contains(&cfg) {
        invalid!("構成 {cfg}（全 {ncfg}）");
    }
    // この構成の多重構成の値。扱わないオペランドは構成で変わってはいけない
    let mut ops: HashMap<(String, i64), String> = HashMap::new();
    let mut vals: HashMap<(String, i64, Vec<String>), HashSet<String>> = HashMap::new();
    for (op, p1, c, v, rest) in &raw.mce {
        vals.entry((
            op.clone(),
            *p1,
            rest.iter().skip(2).take(1).cloned().collect(),
        ))
        .or_default()
        .insert(v.clone());
        if *c == cfg {
            ops.insert((op.clone(), *p1), v.clone());
        }
    }
    for ((op, p1, _), vs) in &vals {
        if !MCE_HANDLED.contains(&op.as_str()) && vs.len() > 1 {
            notimpl!("多重構成のオペランド {op} {p1} が構成で変わる");
        }
    }
    let fop = |op: &str, p1: i64, default: f64| -> f64 {
        ops.get(&(op.to_string(), p1))
            .and_then(|v| fv(v).ok())
            .unwrap_or(default)
    };

    let temp = sys_f(raw, "ENVD", 0, 20.0);
    let pres = sys_f(raw, "ENVD", 1, 1.0);
    let adjust = sys_f(raw, "ENVD", 2, 0.0) != 0.0;
    // 波長
    let ftyp: Vec<i64> = raw
        .sysl
        .get("FTYP")
        .and_then(|v| v.first())
        .map(|v| v.iter().map(|x| fv(x).unwrap_or(0.0) as i64).collect())
        .unwrap_or_else(|| vec![0, 0, 1, 1]);
    let nw = *ftyp.get(3).unwrap_or(&1) as usize;
    let mut wav: HashMap<i64, (f64, f64)> = HashMap::new();
    for a in raw.sysl.get("WAVM").into_iter().flatten() {
        if a.len() >= 3 {
            wav.insert(fv(&a[0])? as i64, (fv(&a[1])?, fv(&a[2])?));
        }
    }
    let mut wl = Vec::new();
    let mut wwt = Vec::new();
    for i in 1..=nw as i64 {
        let (l, w) = *wav
            .get(&i)
            .ok_or_else(|| Error::Invalid(format!("波長 {i} がない")))?;
        wl.push(fop("WAVE", i, l));
        wwt.push(w);
    }
    if let Some(w) = &opt.wavelengths {
        wl = w.clone();
        wwt = vec![1.0; wl.len()];
    }
    let primary = (sys_f(raw, "PWAV", 0, 1.0) as usize).saturating_sub(1);
    if primary >= wl.len() {
        invalid!("主波長 {}（波長は {} 個）", primary + 1, wl.len());
    }
    // ガラス
    let glasses = match &opt.glasses {
        Some(g) => g.clone(),
        None => {
            let cats: Vec<String> = raw
                .sysl
                .get("GCAT")
                .and_then(|v| v.first())
                .cloned()
                .unwrap_or_else(|| vec!["SCHOTT".into()]);
            glass::load_catalogs(&cats, dirs).0
        }
    };
    let ns = raw.surfs.len();
    // 厚さ・パラメータとピックアップ
    let mut th: Vec<f64> = raw.surfs.iter().map(|s| s.disz.unwrap_or(0.0)).collect();
    for (k, t) in th.iter_mut().enumerate() {
        if let Some(v) = ops.get(&("THIC".to_string(), k as i64)) {
            *t = fv(v)?;
        }
    }
    let mut settled = false;
    for _ in 0..=ns {
        let mut changed = false;
        for k in 0..ns {
            if let Some((src, sc, off)) = raw.surfs[k].pzup {
                let v = sc * th[src] + off;
                if v != th[k] {
                    th[k] = v;
                    changed = true;
                }
            }
        }
        if !changed {
            settled = true;
            break;
        }
    }
    if !settled {
        invalid!("厚さのピックアップが収束しない（循環？）");
    }
    let mut parm: Vec<BTreeMap<i64, f64>> = raw.surfs.iter().map(|s| s.parm.clone()).collect();
    settled = false;
    for _ in 0..=ns {
        let mut changed = false;
        for k in 0..ns {
            for (&p, &(src, sc, off)) in &raw.surfs[k].ppar {
                let v = sc * parm[src].get(&p).copied().unwrap_or(0.0) + off;
                if parm[k].get(&p) != Some(&v) {
                    parm[k].insert(p, v);
                    changed = true;
                }
            }
        }
        if !changed {
            settled = true;
            break;
        }
    }
    if !settled {
        invalid!("パラメータのピックアップが収束しない（循環？）");
    }
    let stop_default = raw.surfs.iter().position(|s| s.stop).unwrap_or(1) as f64;
    let stop = fop("STPS", 0, stop_default) as usize;
    let mut sf: Vec<Surface> = Vec::with_capacity(ns);
    let mut nprev = vec![1.0; wl.len()];
    for (k, s) in raw.surfs.iter().enumerate() {
        let ki = k as i64;
        let name = s.typ.clone().unwrap_or_else(|| "STANDARD".into());
        let typ = match name.as_str() {
            "STANDARD" => SurfType::Standard,
            "EVENASPH" => SurfType::EvenAsph,
            "COORDBRK" => SurfType::CoordBrk,
            "JONESMAT" => SurfType::Jones,
            "PARAXIAL" => SurfType::Paraxial,
            _ => notimpl!("面 {k}: 面タイプ {name}"),
        };
        let mut mat = s.glas.clone().unwrap_or_default();
        let mut model = s.model_glass;
        if let Some(v) = ops.get(&("GLSS".to_string(), ki)) {
            mat = v.trim_matches('"').to_string();
            model = None;
        }
        if let Some((nd, vd)) = model {
            if vd != 0.0 {
                notimpl!("分散ありのモデルガラス（面 {k}）");
            }
            mat = format!("{nd:.2},{vd:.1}");
        }
        let ign = k > 0 && fop("IGNR", ki, if s.ignore { 1.0 } else { 0.0 }) != 0.0;
        let p = |i: i64| parm[k].get(&i).copied().unwrap_or(0.0);
        let mut d = Surface {
            typ,
            ignore: ign,
            t: th[k],
            mirror: mat.eq_ignore_ascii_case("MIRROR"),
            comment: s.comment.clone(),
            mat: if typ == SurfType::CoordBrk {
                "-".into()
            } else {
                mat.clone()
            },
            coat: s.coat.clone().unwrap_or_default(),
            stop: k == stop,
            semi: fop("SDIA", ki, s.diam.unwrap_or(0.0)),
            ..Default::default()
        };
        match typ {
            SurfType::Jones => {
                for i in 0..8 {
                    d.jones[i] = p(i as i64 + 1);
                }
            }
            SurfType::CoordBrk => {
                for i in 0..5 {
                    d.cb[i] = p(i as i64 + 1);
                }
                d.cb_order = p(6) as u8;
            }
            SurfType::Paraxial => {
                d.focal = p(1);
                d.opd_mode = p(2) as i32;
                // 0・1 は基準 D0〜D3 で OpticStudio と一致を確認済み。それ以外は未確認
                if !(0..=1).contains(&d.opd_mode) {
                    notimpl!("理想レンズ（面 {k}）の OPD モード {}", d.opd_mode);
                }
                if d.focal == 0.0 {
                    invalid!("理想レンズ（面 {k}）の焦点距離が 0");
                }
            }
            _ => {}
        }
        if matches!(
            typ,
            SurfType::Standard | SurfType::EvenAsph | SurfType::Jones
        ) {
            d.c = fop("CRVT", ki, s.curv.unwrap_or(0.0));
            d.k = fop("CONN", ki, s.coni.unwrap_or(0.0));
            if typ == SurfType::EvenAsph {
                d.asph = (1..=8).map(p).collect();
            } else if typ == SurfType::Standard && parm[k].values().any(|&v| v != 0.0) {
                notimpl!("面 {k}: 標準面にパラメータ");
            }
            if typ == SurfType::Jones && d.c != 0.0 {
                notimpl!("曲面の Jones 行列面 {k}");
            }
        }
        tilts(s, &mut d, k)?;
        aperture(s, &mut d, &ops);
        let n = if d.mirror || typ == SurfType::CoordBrk {
            nprev.clone()
        } else if mat.is_empty() || mat == "AIR" {
            vec![1.0; wl.len()]
        } else if let Some((nd, _)) = model {
            vec![nd; wl.len()]
        } else {
            let g = glasses
                .get(&mat)
                .ok_or_else(|| Error::Invalid(format!("ガラス {mat}（面 {k}）がカタログにない")))?;
            wl.iter().map(|&l| g.index(l, temp, pres, adjust)).collect()
        };
        d.n = n.clone();
        if !ign {
            nprev = n;
        }
        sf.push(d);
    }
    let satp = fop("SATP", 0, aperture_type(raw) as f64) as i64;
    let kind = match satp {
        0 => ApertureKind::Epd,
        1 => ApertureKind::FNum,
        2 => ApertureKind::Na,
        3 => ApertureKind::Float,
        4 => ApertureKind::Pwfn,
        5 => ApertureKind::Cone,
        _ => notimpl!("系の絞りタイプ {satp}"),
    };
    let aval = fop("APER", 0, aperture_value(raw, satp));
    let tele = fop("TELE", 0, *ftyp.get(1).unwrap_or(&0) as f64) != 0.0;
    let ft = match ftyp[0] {
        0 | 4 => FieldType::Angle, // 経緯儀: ファイルには x・y の角度で入っている
        1 => FieldType::ObjH,
        2 => FieldType::ParH,
        3 => FieldType::RealH,
        t => notimpl!("視野タイプ {t}"),
    };
    let nf = *ftyp.get(2).unwrap_or(&1) as usize;
    let col = |key: &str, default: f64| -> Vec<f64> {
        let mut v: Vec<f64> = raw
            .sysl
            .get(key)
            .and_then(|v| v.first())
            .map(|v| v.iter().map(|x| fv(x).unwrap_or(0.0)).collect())
            .unwrap_or_default();
        v.resize(nf.max(v.len()), default);
        v.truncate(nf);
        v
    };
    let (xs, ys, fw) = (col("XFLN", 0.0), col("YFLN", 0.0), col("FWGN", 1.0));
    let vd: Vec<Vec<f64>> = ["VDXN", "VDYN", "VCXN", "VCYN", "VANN"]
        .iter()
        .map(|k| col(k, 0.0))
        .collect();
    let fields: Vec<Field> = (0..nf)
        .map(|i| Field {
            x: fop("XFIE", i as i64 + 1, xs[i]),
            y: fop("YFIE", i as i64 + 1, ys[i]),
            w: fw[i],
            vdx: vd[0][i],
            vdy: vd[1][i],
            vcx: vd[2][i],
            vcy: vd[3][i],
            van: vd[4][i],
        })
        .collect();
    let norm = if ftyp.get(4) == Some(&1) {
        FieldNorm::Rect
    } else {
        FieldNorm::Radial
    };
    let aiming = match sys_f(raw, "RAIM", 1, 0.0) as i64 {
        0 => Aiming::Off,
        1 => Aiming::Paraxial,
        2 => Aiming::Real,
        r => notimpl!("光線エイミング {r}"),
    };
    let glrs = sys_f(raw, "GLRS", 0, 1.0) as usize;
    let ropd = sys_f(raw, "ROPD", 0, 2.0) as i64;
    let mut cofn = None;
    for a in raw.sysl.get("COFN").into_iter().flatten() {
        if a.first().map(|s| s.as_str()) == Some("QF") {
            cofn = a.get(1).map(|s| s.trim_matches('"').to_string());
        }
    }
    let coating_file = cofn.and_then(|c| {
        base.into_iter()
            .chain(dirs.iter().map(|d| d.as_path()))
            .map(|d| d.join(&c))
            .find(|p| p.exists())
    });
    let name = raw
        .sysl
        .get("NAME")
        .and_then(|v| v.first())
        .map(|v| v.join(" "))
        .unwrap_or_default();
    let mut s = System::new(
        sf,
        wl,
        wwt,
        primary,
        fields,
        ft,
        norm,
        (kind, aval),
        tele,
        aiming,
        glasses,
    )?;
    s.temperature = temp;
    s.pressure = pres;
    s.glrs = glrs;
    s.ropd = ropd;
    s.coating_file = coating_file;
    s.name = name;
    s.adjust_index = adjust;
    Ok(s)
}

fn aperture_type(raw: &Raw) -> i64 {
    if raw.sysl.contains_key("FLOA") {
        return 3;
    }
    if raw.sysl.contains_key("FNUM") {
        return if sys_f(raw, "FNUM", 1, 0.0) as i64 == 1 {
            4
        } else {
            1
        };
    }
    if raw.sysl.contains_key("OBNA") {
        return if sys_f(raw, "OBNA", 1, 0.0) as i64 == 1 {
            5
        } else {
            2
        };
    }
    0
}

fn aperture_value(raw: &Raw, satp: i64) -> f64 {
    match satp {
        0 => sys_f(raw, "ENPD", 0, 0.0),
        1 | 4 => sys_f(raw, "FNUM", 0, 0.0),
        2 | 5 => sys_f(raw, "OBNA", 0, 0.0),
        _ => 0.0,
    }
}

fn tilts(s: &RawSurf, d: &mut Surface, k: usize) -> Result<()> {
    for v in &s.scbd {
        let g = |i: usize| -> f64 { v.get(i).and_then(|x| fv(x).ok()).unwrap_or(0.0) };
        let (which, order, mode) = (g(0) as i64, g(1) as i64, g(2) as i64);
        let vals = [g(3), g(4), g(5), g(6), g(7)];
        let any = vals.iter().any(|&x| x != 0.0);
        if d.typ == SurfType::CoordBrk && any {
            notimpl!("面 {k}: 座標ブレークの面の傾き偏心");
        }
        if order != 0 && order != 1 {
            notimpl!("面 {k}: 傾き偏心の順序 {order}");
        }
        let t = Tilt {
            d: vals,
            order: order as u8,
        };
        match which {
            1 => {
                if any {
                    d.before = Some(t);
                }
            }
            2 => match mode {
                2 => {
                    if d.before.is_some() {
                        d.after = After::Reverse;
                    }
                }
                0 => {
                    if any {
                        d.after = After::Tilt(t);
                    }
                }
                _ => notimpl!("面 {k}: 面の後の傾き偏心のモード {mode}（ピックアップ）"),
            },
            _ => notimpl!("面 {k}: SCBD {which}"),
        }
    }
    Ok(())
}

fn aperture(s: &RawSurf, d: &mut Surface, ops: &HashMap<(String, i64), String>) {
    let dec = s.obdc.clone().unwrap_or_default();
    let (dx, dy) = (
        dec.first().copied().unwrap_or(0.0),
        dec.get(1).copied().unwrap_or(0.0),
    );
    let k = s.n;
    let g = |v: &Vec<f64>, i: usize| v.get(i).copied().unwrap_or(0.0);
    d.ap = if let Some(c) = &s.clap {
        let max = ops
            .get(&("APMX".to_string(), k))
            .and_then(|v| fv(v).ok())
            .unwrap_or(g(c, 1));
        Aperture::Circular {
            min: g(c, 0),
            max,
            dx,
            dy,
        }
    } else if let Some(c) = &s.sqap {
        Aperture::Rect {
            hx: g(c, 0),
            hy: g(c, 1),
            dx,
            dy,
        }
    } else if s.flap.is_some() {
        Aperture::Floating { semi: d.semi }
    } else if let Some(c) = &s.obsc {
        Aperture::Obsc {
            min: g(c, 0),
            max: g(c, 1),
            dx,
            dy,
        }
    } else {
        Aperture::None
    };
}
