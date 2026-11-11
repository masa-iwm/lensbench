//! OpticStudio の順次モードの .zmx を読む。
//!
//! 順次の光線追跡に効くものだけを読み、対応していないもの（未知の面タイプや面の行、扱わない多重構成の
//! オペランドが構成で変わる、非順次、mm 以外の単位など）は `Error::NotImplemented` で止める。

use crate::error::{Error, Result};
use crate::glass::{self, Glass};
use crate::param::{surf_parm, PickKind, Pickup, SolveKind, ThickSolve};
use crate::surface::{After, Aperture, SurfType, Surface, Tilt};
use crate::system::{Aiming, ApertureKind, Field, FieldNorm, FieldType, System};
use crate::{invalid, notimpl};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

const HARMLESS: &[&str] = &[
    "HIDE", "MIRR", "SLAB", "POPS", "SSID", "FIMP", "MEMA", "VPAR", "VDSZ", "VCRV", "VCON", "OEMA",
    "BLNK", "NAME", // 縁の描き方（光線追跡に効かない）
    "EDGE", // 面の色（表示用）・面ごとの熱膨張係数（熱の解析だけに使う）
    "SCOL", "TCED",
];
const MCE_HANDLED: &[&str] = &[
    "THIC", "IGNR", "APMX", "SDIA", "STPS", "SATP", "APER", "TELE", "CRVT", "CONN", "WAVE", "GLSS",
    "XFIE", "YFIE", "MOFF", "LTTL", "PRAM",
    // 光線追跡に効かない: 構成の名前（MCOM）・最適化での構成の重み（CWGT）
    "MCOM", "CWGT",
];

#[derive(Debug, Clone, Default)]
pub struct RawSurf {
    pub n: i64,
    pub typ: Option<String>,
    pub coat: Option<String>,
    pub glas: Option<String>,
    /// モデルガラス（nd, vd, ΔPgF）
    pub model_glass: Option<(f64, f64, f64)>,
    /// 材料のピックアップの元の面（GLAS 名前 2 元の面 …。名前・モデルの値は使わない）
    pub glas_pickup: Option<usize>,
    pub curv: Option<f64>,
    /// 曲率のピックアップ（元の面, 倍率, 元の列。None は同じ列）
    pub curv_pickup: Option<(usize, f64, Option<PickKind>)>,
    pub coni: Option<f64>,
    pub disz: Option<f64>,
    pub diam: Option<f64>,
    /// DIAM の 2 番目の欄（0 自動、1 固定、2 ピックアップ、4 マクロ）
    pub diam_mode: Option<u8>,
    /// 半直径を固定（DIAM の 2 番目が 1）
    pub diam_fixed: bool,
    /// 機械的な半直径の行（MEMA）がある（OpticStudio が書いた .zmx）
    pub mema: bool,
    pub comment: String,
    pub parm: BTreeMap<i64, f64>,
    /// ピックアップ（元の面, 倍率, オフセット, 元の列。None は同じ列）
    pub ppar: BTreeMap<i64, (usize, f64, f64, Option<PickKind>)>,
    pub pzup: Option<(usize, f64, f64, Option<PickKind>)>,
    /// 厚さのソルブ（種類, 値）。DISZ の次の行（MAZH・CHZH・EDGE・TOLE・TCOM・TCOC・TPUP）
    pub tsolve: Option<(SolveKind, f64)>,
    /// 曲率のソルブ（種類, 値）。CURV の 3 番目の欄（2・3・5〜11）
    pub csolve: Option<(SolveKind, f64)>,
    /// 曲率の ZPL マクロのソルブのマクロの名前（CURV c 12 … "名前"）
    pub cmacro: Option<String>,
    /// 厚さの ZPL マクロのソルブのマクロの名前（TZPL "名前"）
    pub tmacro: Option<String>,
    /// 円錐係数の ZPL マクロのソルブ（KZPL "名前"、OpticStudio 26.1.1 で実測）
    pub kmacro: Option<String>,
    /// 半直径の ZPL マクロのソルブ（DIAM 値 4 0 0 1 "名前"、OpticStudio 26.1.1 で実測）
    pub smacro: Option<String>,
    /// パラメータの ZPL マクロのソルブ（番号, 名前）。PZPL 番号 "名前"
    pub pmacros: Vec<(u8, String)>,
    /// 円錐係数のピックアップ（元の面, 倍率, オフセット（常に 0）, 元の列）
    pub pkup: Option<(usize, f64, f64, Option<PickKind>)>,
    /// 半直径のピックアップ（元の面, 倍率, 元の列）
    pub diam_pickup: Option<(usize, f64, Option<PickKind>)>,
    /// 拡張データ（番号 → 値）
    pub xdat: BTreeMap<i64, f64>,
    /// Grid Sag の格子の頭（GDAT nx ny dx dy xdec ydec unit …）と点（GARR 番号 z zx zy zxy nodata）、.DAT の名前（FIMP）
    pub gdat: Option<Vec<f64>>,
    pub garr: Vec<(usize, [f64; 5])>,
    pub fimp: Option<String>,
    pub clap: Option<Vec<f64>>,
    pub sqap: Option<Vec<f64>>,
    pub elap: Option<Vec<f64>>,
    pub flap: Option<Vec<f64>>,
    pub obsc: Option<Vec<f64>>,
    /// スパイダー（SPID 腕の幅 腕の本数 開口のピックアップの元の面）
    pub spid: Option<Vec<f64>>,
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
    // 古い .zmx（Edmund の一部の品など）は面の行を字下げしない。字下げした行が 1 つも無いときは、
    // SURF の後の行を全て面の行として読み、知らない行は飛ばす（小文字の stop は絞り）
    let legacy = !txt.lines().any(|l| l.starts_with("  "));
    for line in txt.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.is_empty() {
            continue;
        }
        let key = if legacy && cur.is_some() && t[0] == "stop" {
            "STOP"
        } else {
            t[0]
        };
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
        let Some(ci) = cur.filter(|_| legacy || line.starts_with("  ")) else {
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
            // ピックアップ: PPAR 番号 元の面 倍率 オフセット 列、PZUP 元の面 倍率 オフセット 列、
            // PKUP 元の面 倍率 列（円錐係数はオフセットなし）。列は [`PickKind::from_column`]（OpticStudio 2026 R1.01 で実測）
            // 元の面が負（OpticStudio 2026 R1 の座標ブレークの主光線のソルブがこう保存される）は、保存された値で固定する
            // （PARM は解いた値。付属のサンプルで OpticStudio と光線が一致）
            "PPAR" if num(2)? < 0.0 => {}
            "PPAR" => {
                let i = num(1)? as i64;
                let from = pick_col(s.n, "PPAR", num(5)?, PickKind::Parm(i.clamp(0, 255) as u8))?;
                s.ppar.insert(i, (num(2)? as usize, num(3)?, num(4)?, from));
            }
            "PZUP" => {
                let from = pick_col(s.n, "PZUP", num(4)?, PickKind::Thick)?;
                s.pzup = Some((num(1)? as usize, num(2)?, num(3)?, from));
            }
            // 厚さのソルブ（OpticStudio 2026 R1.01 で実測）。OPDZ OPD 帯
            "MAZH" => {
                let z = num(2)?;
                let k = if z == 0.0 {
                    SolveKind::MarginalHeight
                } else {
                    SolveKind::MarginalHeightZone(z)
                };
                s.tsolve = Some((k, num(1)?));
            }
            "CHZH" => s.tsolve = Some((SolveKind::ChiefHeight, num(1)?)),
            "TPUP" => s.tsolve = Some((SolveKind::PupilPosition, 0.0)),
            "EDGE" => s.tsolve = Some((SolveKind::Edge(num(2)?), num(1)?)),
            "TOLE" => s.tsolve = Some((SolveKind::Position(surf_no(num(1)?)?), num(2)?)),
            "TCOM" => s.tsolve = Some((SolveKind::Compensator(surf_no(num(1)?)?), num(2)?)),
            "TCOC" => s.tsolve = Some((SolveKind::CenterOfCurvature(surf_no(num(1)?)?), 0.0)),
            "OPDZ" => s.tsolve = Some((SolveKind::Opd(num(2)?), num(1)?)),
            "PKUP" => {
                let from = pick_col(s.n, "PKUP", num(3)?, PickKind::Conic)?;
                s.pkup = Some((num(1)? as usize, num(2)?, 0.0, from));
            }
            "XDAT" => {
                s.xdat.insert(num(1)? as i64, num(2)?);
            }
            "GDAT" => s.gdat = Some(nums()?),
            "GARR" => {
                let v = nums()?;
                if v.len() < 5 {
                    invalid!("面 {}: GARR の値が足りない", s.n);
                }
                let g = |i: usize| v.get(i).copied().unwrap_or(0.0);
                s.garr.push((v[0] as usize, [g(1), g(2), g(3), g(4), g(5)]));
            }
            "FIMP" => s.fimp = Some(line.trim().get(5..).unwrap_or("").trim().to_string()),
            "TYPE" => s.typ = Some(t.get(1).unwrap_or(&"").to_string()),
            "COAT" => s.coat = Some(t.get(1).unwrap_or(&"").to_string()),
            // GLAS 名前 ソルブ 元の面 nd vd ΔPgF …。ソルブ 1 はモデルガラス（名前 ___BLANK）、2 は材料のピックアップ
            // （Ghost Focus Generator が保存するゴーストの系の戻りの面など。名前と nd・vd は保存時の値で、使わない）
            "GLAS" => {
                s.glas = Some(t.get(1).unwrap_or(&"").to_string());
                // ソルブ 4 は Offset（#10 Δnd、#11 Δvd、OpticStudio 2026 R1.01 で実測）。ずれの式を確かめていないので止める
                if t.get(2) == Some(&"4") && (num(10)? != 0.0 || num(11)? != 0.0) {
                    notimpl!(
                        "zmx の面 {}: 材料の Offset ソルブ（Δnd {}、Δvd {}）",
                        s.n,
                        num(10)?,
                        num(11)?
                    );
                }
                if t.get(2) == Some(&"2") {
                    s.glas_pickup = Some(num(3)? as usize);
                } else if t.get(1) == Some(&"___BLANK") {
                    s.model_glass = Some((num(4)?, num(5)?, num(6)?));
                }
            }
            // CURV 曲率 ソルブ ...。ソルブ 4 はピックアップ: CURV 曲率 4 元の面 倍率 列（同じ列なら倍率は曲率にかかり、
            // オフセットはない。[`Pickup`]）。ほかのソルブは保存された値を使う
            "CURV" => {
                s.curv = Some(num(1)?);
                let p1 = num(3)?;
                s.csolve = match t.get(2).copied().unwrap_or("0") {
                    "0" | "1" => None,
                    "4" => {
                        let from = pick_col(s.n, "CURV", num(5)?, PickKind::Curv)?;
                        s.curv_pickup = Some((p1 as usize, num(4)?, from));
                        None
                    }
                    // 曲率のソルブ（OpticStudio 2026 R1.01 で実測）
                    "2" => Some((SolveKind::MarginalAngle, p1)),
                    "3" => Some((SolveKind::ChiefAngle, p1)),
                    "5" => Some((SolveKind::MarginalNormal, 0.0)),
                    "6" => Some((SolveKind::ChiefNormal, 0.0)),
                    "7" => Some((SolveKind::Aplanatic, 0.0)),
                    "8" => Some((SolveKind::ElementPower, p1)),
                    "9" => Some((SolveKind::ConcentricSurface(surf_no(p1)?), 0.0)),
                    "10" => Some((SolveKind::ConcentricRadius(surf_no(p1)?), 0.0)),
                    "11" => Some((SolveKind::FNumber, p1)),
                    // ZPL マクロのソルブ（最後の欄の "…" がマクロの名前）。マクロは build で探す
                    "12" => {
                        s.cmacro = Some(macro_name(s.n, line)?);
                        None
                    }
                    c => notimpl!("zmx の面 {}: 曲率のソルブ {c}", s.n),
                };
            }
            "CONI" => s.coni = Some(num(1)?),
            // ZPL マクロのソルブ（OpticStudio 26.1.1 で実測）
            "TZPL" => s.tmacro = Some(macro_name(s.n, line)?),
            "KZPL" => s.kmacro = Some(macro_name(s.n, line)?),
            "PZPL" => {
                let n = num(1)?;
                if !(0.0..=255.0).contains(&n) || n.fract() != 0.0 {
                    invalid!("zmx の面 {}: PZPL のパラメータの番号 {n}", s.n);
                }
                s.pmacros.push((n as u8, macro_name(s.n, line)?));
            }
            "DISZ" => s.disz = Some(num(1)?),
            // DIAM 半直径 ソルブ …（ソルブ 0 は自動、1 は固定）。ソルブ 2 はピックアップ: DIAM 値 2 元の面 列 倍率
            // （オフセットなし、OpticStudio 2026 R1.01 で実測）
            "DIAM" => {
                s.diam = Some(num(1)?);
                s.diam_mode = t.get(2).and_then(|v| v.parse().ok());
                s.diam_fixed = t.get(2) == Some(&"1");
                if t.get(2) == Some(&"4") {
                    s.smacro = Some(macro_name(s.n, line)?);
                }
                if t.get(2) == Some(&"2") {
                    let from = pick_col(s.n, "DIAM", num(4)?, PickKind::Semi)?;
                    s.diam_pickup = Some((num(3)? as usize, num(5)?, from));
                }
            }
            "CLAP" => s.clap = Some(nums()?),
            "SQAP" => s.sqap = Some(nums()?),
            "ELAP" => s.elap = Some(nums()?),
            "FLAP" => s.flap = Some(nums()?),
            "OBSC" => s.obsc = Some(nums()?),
            // SPID 腕の幅（全幅） 腕の本数 開口のピックアップの元の面（OpticStudio 2026 R1.01 で実測）。ピックアップした面
            // にも元の面の値と偏心（OBDC）が書かれるので、その値を使う
            "SPID" => {
                let v = nums()?;
                let (w, n) = (
                    v.first().copied().unwrap_or(0.0),
                    v.get(1).copied().unwrap_or(0.0),
                );
                if !(w >= 0.0) || !(n >= 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64) {
                    invalid!("zmx の面 {}: スパイダーの腕の幅 {w}・本数 {n}", s.n);
                }
                s.spid = Some(v);
            }
            "OBDC" => s.obdc = Some(nums()?),
            "SCBD" => s.scbd.push(rest()),
            "STOP" => s.stop = true,
            // 面の無視（Ignore This Surface）: OpticStudio は HIDE の 6 番目の値を 1 にして書く（IGNR は多重構成の
            // 演算子の名前で、面の行にはない。以前の lensbench が書いた "IGNR 1" も読めるように残す）
            "HIDE" => s.ignore = s.ignore || t.get(6).is_some_and(|v| *v != "0"),
            "IGNR" => s.ignore = t.get(1).is_some_and(|v| *v != "0"),
            "MEMA" => s.mema = true,
            k if HARMLESS.contains(&k) => {}
            _ if legacy => {}
            k => notimpl!("zmx の面 {}: 行 {} は未対応", s.n, k),
        }
    }
    Ok(raw)
}

/// mm 以外のレンズ単位（UNIT の CM・IN・METER）の .zmx を mm に直す。長さの値（曲率・厚さ・半直径・開口・偏心・
/// 偶数次非球面の係数・厚さのソルブの値・入射瞳径・高さの視野・多重構成の THIC・SDIA・CRVT など）を換算する。
/// 換算の決まっていない面タイプ・データ（標準・偶数次非球面・座標ブレーク・傾いた面・奇数次非球面以外、拡張データ、
/// パラメータのピックアップのオフセットなど）は未対応で止める
fn to_mm(raw: &Raw, unit: &str) -> Result<Raw> {
    let f = match unit.to_ascii_uppercase().as_str() {
        "CM" => 10.0,
        "IN" => 25.4,
        "METER" | "M" => 1000.0,
        u => notimpl!("レンズ単位 {u}（mm・cm・in・m）"),
    };
    // 光線エイミングの瞳のずらし（RAIM の 5〜7 番目、長さ。lensbench の書く RAIM 0 種類 1 1 0 0 0 … の 0 0 0）は換算を確かめていないので止める
    if let Some(a) = raw.sysl.get("RAIM").and_then(|v| v.first()) {
        if a.iter()
            .skip(4)
            .take(3)
            .any(|x| fv(x).unwrap_or(0.0) != 0.0)
        {
            notimpl!("レンズ単位 {unit} の光線エイミングの瞳のずらし");
        }
    }
    let mut r = raw.clone();
    let len = |v: &mut f64| *v *= f;
    for (k, sf) in r.surfs.iter_mut().enumerate() {
        let typ = sf.typ.clone().unwrap_or_else(|| "STANDARD".into());
        if let Some(c) = sf.curv.as_mut() {
            *c /= f;
        }
        if let Some(d) = sf.disz.as_mut() {
            if d.is_finite() {
                len(d);
            }
        }
        if let Some(d) = sf.diam.as_mut() {
            len(d);
        }
        // PARM: 面タイプごとの単位
        let mut parm = BTreeMap::new();
        for (&i, &v) in &sf.parm {
            let w = match typ.as_str() {
                "STANDARD" | "TILTSURF" => v,
                // r^{2i} の係数: [長さ]^{1−2i}
                "EVENASPH" => v * f.powi(1 - 2 * i as i32),
                // r^i の係数: [長さ]^{1−i}
                "ODDASPHE" => v * f.powi(1 - i as i32),
                // 偏心 x・y（1・2）は長さ、傾き（3〜5）・順序（6）はそのまま
                // 偏心 x・y（1・2）と Zs・Za・Zc（5〜7）は長さ、傾き（3・4）・角度（8）はそのまま
                "IRREGULA" => {
                    if matches!(i, 1 | 2 | 5 | 6 | 7) {
                        v * f
                    } else {
                        v
                    }
                }
                "COORDBRK" => {
                    if i == 1 || i == 2 {
                        v * f
                    } else {
                        v
                    }
                }
                t => notimpl!("面 {k}: レンズ単位 {unit} の面タイプ {t}"),
            };
            parm.insert(i, w);
        }
        sf.parm = parm;
        if !sf.xdat.is_empty() || sf.gdat.is_some() {
            notimpl!("面 {k}: レンズ単位 {unit} の拡張データ");
        }
        if sf.ppar.values().any(|p| p.2 != 0.0) {
            notimpl!("面 {k}: レンズ単位 {unit} のパラメータのピックアップのオフセット");
        }
        if let Some(p) = sf.pzup.as_mut() {
            p.2 *= f;
        }
        if let Some((kind, v)) = sf.tsolve.as_mut() {
            match kind {
                SolveKind::MarginalHeight
                | SolveKind::ChiefHeight
                | SolveKind::Position(_)
                | SolveKind::Compensator(_) => *v *= f,
                SolveKind::MarginalHeightZone(_) => *v *= f,
                SolveKind::Edge(rr) => {
                    *v *= f;
                    *rr *= f;
                }
                _ => {}
            }
        }
        if let Some((kind, v)) = sf.csolve.as_mut() {
            // 傾き・F 値・面の関係は単位によらない。玉のパワー [1/長さ] だけ換算
            if matches!(kind, SolveKind::ElementPower) {
                *v /= f;
            }
        }
        for a in [
            &mut sf.clap,
            &mut sf.sqap,
            &mut sf.elap,
            &mut sf.flap,
            &mut sf.obsc,
            &mut sf.obdc,
        ]
        .into_iter()
        .flatten()
        {
            a.iter_mut().for_each(|v| *v *= f);
        }
        // スパイダーは腕の幅だけ長さ
        if let Some(w) = sf.spid.as_mut().and_then(|v| v.first_mut()) {
            *w *= f;
        }
        // 面の前後の傾き偏心（SCBD 前後 順序 種類 偏心x 偏心y 傾きx 傾きy 傾きz）: 偏心だけ換算
        for row in sf.scbd.iter_mut() {
            for j in [3usize, 4] {
                if let Some(x) = row.get_mut(j) {
                    let v = fv(x)? * f;
                    *x = format!("{v:e}");
                }
            }
        }
    }
    // 系の値: 入射瞳径、高さの視野
    if let Some(v) = r.sysl.get_mut("ENPD").and_then(|v| v.first_mut()) {
        if let Some(x) = v.first_mut() {
            *x = format!("{:e}", fv(x)? * f);
        }
    }
    // FTYP の 1 番目が視野の種類（1 物体高・2 近軸像高・3 実像高）
    let ftyp0 = r
        .sysl
        .get("FTYP")
        .and_then(|v| v.first())
        .and_then(|v| v.first())
        .map(|x| fv(x).unwrap_or(0.0) as i64)
        .unwrap_or(0);
    let height_fields = matches!(ftyp0, 1..=3);
    if height_fields {
        for key in ["XFLN", "YFLN"] {
            if let Some(v) = r.sysl.get_mut(key).and_then(|v| v.first_mut()) {
                for x in v.iter_mut() {
                    *x = format!("{:e}", fv(x)? * f);
                }
            }
        }
    }
    for (op, _, _, v, _) in r.mce.iter_mut() {
        let k = match op.as_str() {
            "THIC" | "SDIA" => f,
            "CRVT" => 1.0 / f,
            "XFIE" | "YFIE" if height_fields => f,
            "XFIE" | "YFIE" | "IGNR" | "STPS" | "WAVE" | "GLSS" | "MCOM" | "CWGT" | "CONN"
            | "SATP" | "TELE" => 1.0,
            o => notimpl!("レンズ単位 {unit} の多重構成のオペランド {o}"),
        };
        if k != 1.0 {
            *v = format!("{:e}", fv(v)? * k);
        }
    }
    Ok(r)
}

/// マクロのソルブの行の名前（最初の " から最後の " までの中身）
fn macro_name(n: i64, line: &str) -> Result<String> {
    match (line.find('"'), line.rfind('"')) {
        (Some(a), Some(b)) if b > a + 1 => Ok(line[a + 1..b].to_string()),
        _ => invalid!("zmx の面 {n}: マクロのソルブの名前がない"),
    }
}

/// 面の番号の欄
fn surf_no(v: f64) -> Result<usize> {
    if v < 0.0 || v.fract() != 0.0 {
        invalid!("面の番号 {v}");
    }
    Ok(v as usize)
}

/// ピックアップの列のコードから元の量（同じ列なら None）
fn pick_col(n: i64, key: &str, col: f64, dst: PickKind) -> Result<Option<PickKind>> {
    match PickKind::from_column(col as i64, dst).filter(|_| col.fract() == 0.0) {
        Some(f) => Ok(f),
        None => notimpl!("zmx の面 {n}: {key} のピックアップの列 {col}"),
    }
}

fn sys_str<'a>(raw: &'a Raw, key: &str, i: usize) -> Option<&'a str> {
    raw.sysl.get(key)?.first()?.get(i).map(|s| s.as_str())
}
fn sys_f(raw: &Raw, key: &str, i: usize, default: f64) -> f64 {
    sys_str(raw, key, i)
        .and_then(|v| fv(v).ok())
        .unwrap_or(default)
}

impl Raw {
    /// 自動の半直径を入れる前の lensbench が書いた .zmx か: VERS が lensbench の書く値（`VERS 260327 42 20120530 20120530`）で、
    /// どの面にも MEMA の行がない。その頃の書き出しは固定の半直径も `DIAM 値 0`（自動）で書いていたので、半直径は
    /// ファイルの値で固定として読む（[`LoadOptions::fixed_semi`] と同じ）。OpticStudio が保存した .zmx には全部の面に MEMA が
    /// あり、今の lensbench の書き出しも MEMA を書くので、どちらもこれに当たらない
    pub fn legacy_lensbench(&self) -> bool {
        let vers = self.sysl.get("VERS").and_then(|v| v.first());
        vers.is_some_and(|v| {
            v.iter()
                .map(|x| x.as_str())
                .eq(["260327", "42", "20120530", "20120530"])
        }) && !self.surfs.iter().any(|r| r.mema)
    }

    /// マルチコンフィグの数（MNUM、なければ 1）
    pub fn configurations(&self) -> usize {
        (sys_f(self, "MNUM", 0, 1.0) as usize).max(1)
    }

    /// ガラスカタログの名前（GCAT）
    pub fn catalogs(&self) -> Vec<String> {
        self.sysl
            .get("GCAT")
            .and_then(|v| v.first())
            .cloned()
            .unwrap_or_default()
    }

    /// Gradium の分布のファイルの名前（COFN の .GRD の項目、なければ PROFILE.GRD）
    pub fn grd_file_name(&self) -> String {
        self.sysl
            .get("COFN")
            .into_iter()
            .flatten()
            .flatten()
            .map(|v| v.trim_matches('"'))
            .find(|v| v.to_uppercase().ends_with(".GRD"))
            .unwrap_or("PROFILE.GRD")
            .to_string()
    }

    /// 膜ファイルの名前（COFN の最初の項目。OpticStudio の Coatings フォルダから探す名前）
    pub fn coating_file_name(&self) -> Option<String> {
        let rows = self.sysl.get("COFN")?;
        let pick = |a: &Vec<String>| {
            let a: Vec<&String> = a.iter().filter(|v| v.as_str() != "QF").collect();
            a.first().map(|v| v.trim_matches('"').to_string())
        };
        rows.iter()
            .find(|a| a.first().map(|v| v.as_str()) == Some("QF"))
            .and_then(pick)
            .or_else(|| rows.first().and_then(pick))
    }
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
    /// ZPL Macro Solve のマクロ（.ZPL）を探すフォルダ（.zmx のフォルダ・その隣の Macros・環境変数
    /// LENSBENCH_MACROS の前に探す。[`crate::zpl_solve::macro_dirs`]）
    pub macro_dirs: Vec<PathBuf>,
    /// 半直径をファイルの値で固定する（カタログの玉: 径は決まっている。自動の面も光線から求めない）
    pub fixed_semi: bool,
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

/// 多重構成の .zmx から構成 cfgs（1 始まり、空なら全部）の系をまとめて作る（ファイルは 1 回だけ読む）。
/// opt.cfg は使わない。結果は構成ごとに `load_with` で読んだものと同じ
pub fn load_configs(path: &Path, opt: &LoadOptions, cfgs: &[usize]) -> Result<Vec<System>> {
    let txt = glass::read_text(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    let raw = parse_text(&txt)?;
    let mut dirs = opt.catalog_dirs.clone();
    if let Some(p) = path.parent() {
        dirs.push(p.to_path_buf());
    }
    let dirs = glass::catalog_dirs(&dirs);
    let all: Vec<usize>;
    let cfgs = if cfgs.is_empty() {
        all = (1..=raw.configurations()).collect();
        &all[..]
    } else {
        cfgs
    };
    cfgs.iter()
        .map(|&c| {
            if c == 0 {
                invalid!("構成は 1 始まり（0）");
            }
            let o = LoadOptions {
                cfg: c,
                ..opt.clone()
            };
            build(&raw, &o, &dirs, path.parent())
        })
        .collect()
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
    let scaled;
    let raw = if unit.eq_ignore_ascii_case("MM") {
        raw
    } else {
        scaled = to_mm(raw, unit)?;
        &scaled
    };
    if raw.sysl.contains_key("AFOC") && sys_f(raw, "AFOC", 0, 0.0) != 0.0 {
        notimpl!("無焦点の像空間");
    }
    let ncfg = sys_f(raw, "MNUM", 0, 1.0) as i64;
    // 以前の lensbench の書き出し: 半直径は固定として読む（[`Raw::legacy_lensbench`]）
    let legacy = raw.legacy_lensbench();
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
        // 古い .zmx は重みを省く（WAVM 1 0.5876）。省いた重みは 1
        if a.len() >= 2 {
            let w = a.get(2).map(|x| fv(x)).transpose()?.unwrap_or(1.0);
            wav.insert(fv(&a[0])? as i64, (fv(&a[1])?, w));
        }
    }
    // 古い書式（WAVM が無く、WAVL に使う波長を並べ、WWGT にその重み）
    let mut nw = nw;
    if wav.is_empty() {
        if let Some(l) = raw.sysl.get("WAVL").and_then(|v| v.first()) {
            let w = raw.sysl.get("WWGT").and_then(|v| v.first());
            for (i, x) in l.iter().enumerate() {
                let wt = w
                    .and_then(|w| w.get(i))
                    .and_then(|x| fv(x).ok())
                    .unwrap_or(1.0);
                wav.insert(i as i64 + 1, (fv(x)?, wt));
            }
            if !raw.sysl.contains_key("FTYP") {
                nw = l.len().max(1);
            }
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
    let glasses: std::sync::Arc<HashMap<String, Glass>> = match &opt.glasses {
        Some(g) => std::sync::Arc::new(g.clone()),
        None => {
            let cats: Vec<String> = raw
                .sysl
                .get("GCAT")
                .and_then(|v| v.first())
                .cloned()
                .unwrap_or_else(|| vec!["SCHOTT".into()]);
            glass::load_catalogs_shared(&cats, dirs).0
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
    for (k, r) in raw.surfs.iter().enumerate() {
        for src in [
            r.pzup.map(|p| p.0),
            r.pkup.map(|p| p.0),
            r.curv_pickup.map(|p| p.0),
            r.diam_pickup.map(|p| p.0),
        ]
        .into_iter()
        .flatten()
        .chain(r.ppar.values().map(|p| p.0))
        {
            if src >= ns {
                invalid!("面 {k}: ピックアップ元の面 {src} がない");
            }
        }
        if r.tsolve.is_some() || r.tmacro.is_some() {
            if k == 0 || k + 1 >= ns {
                notimpl!("面 {k}: 物体面・像面の厚さのソルブ");
            }
            if ops.contains_key(&("THIC".to_string(), k as i64)) {
                notimpl!("面 {k}: 厚さのソルブと多重構成の THIC");
            }
            if r.pzup.is_some() {
                invalid!("面 {k}: 厚さにソルブとピックアップの両方");
            }
        }
        if r.csolve.is_some() || r.cmacro.is_some() {
            if k == 0 || k + 1 >= ns {
                notimpl!("面 {k}: 物体面・像面の曲率のソルブ");
            }
            if ops.contains_key(&("CRVT".to_string(), k as i64)) {
                notimpl!("面 {k}: 曲率のソルブと多重構成の CRVT");
            }
        }
    }
    thickness_pickups(raw, &mut th)?;
    let mut parm: Vec<BTreeMap<i64, f64>> = raw.surfs.iter().map(|s| s.parm.clone()).collect();
    // 多重構成の PRAM（面の PARM の値）: `PRAM 面 構成 値 0 PARM の番号 …`
    for (op, p1, c, v, rest) in &raw.mce {
        if op == "PRAM" && *c == cfg {
            let (Some(k), Some(p)) = (
                usize::try_from(*p1).ok().filter(|&k| k < ns),
                rest.get(1).and_then(|x| x.parse::<i64>().ok()),
            ) else {
                invalid!("多重構成の PRAM {p1}: 面か PARM の番号が不正");
            };
            parm[k].insert(p, fv(v)?);
        }
    }
    let mut settled = false;
    for _ in 0..=ns {
        let mut changed = false;
        for k in 0..ns {
            for (&p, &(src, sc, off, from)) in &raw.surfs[k].ppar {
                // 別の列からのものは系を作ってから解決する（[`System::resolve_pickups`]）
                if from.is_some() {
                    continue;
                }
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
    let mats = materials(raw, &ops)?;
    let stop_default = raw.surfs.iter().position(|s| s.stop).unwrap_or(1) as f64;
    let stop = fop("STPS", 0, stop_default) as usize;
    // GRIN の材料（PROFILE.GRD・SGRIN.DAT・GRADIENT_9.DAT）。GRIN の面があるときだけ読む
    let grin_data = if raw
        .surfs
        .iter()
        .any(|r| matches!(r.typ.as_deref(), Some("GRINSUR8") | Some("GRINSUR9")))
    {
        Some(crate::grin::GrinData::load(dirs, &raw.grd_file_name())?)
    } else {
        None
    };
    let mut sf: Vec<Surface> = Vec::with_capacity(ns);
    // 面の後の傾き偏心のピックアップ（元の面, 反転）
    let mut after_picks: Vec<Option<(usize, bool)>> = Vec::with_capacity(ns);
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
            "TOROIDAL" => SurfType::Toroidal,
            "SZERNSAG" => SurfType::ZernikeStd,
            "FZERNSAG" => SurfType::ZernikeFringe,
            "GRID_SAG" => SurfType::GridSag,
            "FRESNELS" => SurfType::Fresnel,
            "CFRESNEL" => SurfType::CylFresnel,
            "BINARY_2" => SurfType::Binary2,
            "GRINSUR8" => SurfType::Gradium,
            "GRINSUR9" => SurfType::Gradient9,
            "ODDASPHE" => SurfType::OddAsph,
            "TILTSURF" => SurfType::TiltSurf,
            "IRREGULA" => SurfType::Irregular,
            // 複屈折材の板の入口・出口は形は標準面（偏光の位相は Surface::bire）
            "BIRE__IN" | "BIRE_OUT" => SurfType::Standard,
            _ => notimpl!("面 {k}: 面タイプ {name}"),
        };
        let (mut mat, model) = mats[k].clone();
        if let Some((nd, vd, dp)) = model {
            mat = if vd == 0.0 {
                format!("{nd:.2},{vd:.1}")
            } else if dp == 0.0 {
                // 分散あり: 書き出しで値を戻せるように丸めない
                format!("{nd},{vd}")
            } else {
                format!("{nd},{vd},{dp}")
            };
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
            // 自動（DIAM … 0、欄がなければ自動）。多重構成の SDIA で値を決める面は固定
            semi_auto: !opt.fixed_semi
                && !legacy
                && s.diam_mode.unwrap_or(0) == 0
                && !ops.contains_key(&("SDIA".to_string(), ki)),
            ..Default::default()
        };
        if name == "BIRE__IN" {
            let mode = p(1);
            // Mode 1・3 は異常光の道（屈折が違う）で未対応
            if !(mode == 0.0 || mode == 2.0) {
                notimpl!("面 {k}: Birefringent In の Mode {mode}（0 と 2 だけ）");
            }
            if s.xdat.get(&1).copied().unwrap_or(0.0) != 0.0 {
                notimpl!("面 {k}: Birefringent In の Gradient Mode");
            }
            let axis = [p(2), p(3), p(4)];
            if !(axis.iter().map(|v| v * v).sum::<f64>() > 0.0) {
                invalid!("面 {k}: Birefringent In の結晶軸が 0");
            }
            d.bire = Some(crate::surface::Bire::In {
                mode: mode as u8,
                axis,
                ne: Vec::new(),
            });
        } else if name == "BIRE_OUT" {
            d.bire = Some(crate::surface::Bire::Out);
        }
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
            SurfType::ZernikeStd | SurfType::ZernikeFringe | SurfType::GridSag => {
                freeform_surface(s, &mut d, typ, &p, k)?;
            }
            SurfType::Binary2 => {
                // PARM 0 は回折次数。XDAT 1 は項の数、2 は正規化半径、k+2 は ρ^{2k} の係数 [rad]
                let nt = s.xdat.get(&1).copied().unwrap_or(0.0);
                if nt < 0.0 || nt.fract() != 0.0 || nt > 240.0 {
                    invalid!("面 {k}: Binary 2 の項の数 {nt}");
                }
                let norm = s.xdat.get(&2).copied().unwrap_or(1.0);
                if norm <= 0.0 {
                    invalid!("面 {k}: Binary 2 の正規化半径 {norm}");
                }
                let coef = (0..nt as i64)
                    .map(|j| s.xdat.get(&(j + 3)).copied().unwrap_or(0.0))
                    .collect();
                d.bin2 = Some(std::sync::Arc::new(crate::surface::Binary2 {
                    order: p(0),
                    norm,
                    coef,
                }));
            }
            SurfType::Gradium | SurfType::Gradient9 => {
                if mat.eq_ignore_ascii_case("MIRROR") {
                    notimpl!("面 {k}: GRIN の面の鏡");
                }
                let gd = grin_data.as_ref().expect("GRIN のデータは読込済み");
                let g = if typ == SurfType::Gradium {
                    // PARM 0 素材の厚さ（分布のファイルの値）、1 Δt、2 Δz、3 頂点の基準の屈折率（表示用）、
                    // 4〜7 dx dy tx ty、8 端の扱い
                    let cap = p(8);
                    if !(0.0..=3.0).contains(&cap) || cap.fract() != 0.0 {
                        invalid!("面 {k}: Gradium の端の扱い {cap}");
                    }
                    gd.gradium(&mat, p(2), [p(4), p(5), p(6), p(7)], cap as u8, p(1))?
                } else {
                    gd.grad9(&mat, [p(2), p(3)], p(1))?
                };
                d.grin = Some(std::sync::Arc::new(g));
            }
            SurfType::CylFresnel => {
                // XDAT 1 は項の数、2・3 は屈折の面の曲率・円錐係数、k+3 は y^{2k} の係数
                let nt = s.xdat.get(&1).copied().unwrap_or(0.0);
                if nt < 0.0 || nt.fract() != 0.0 || nt > 240.0 {
                    invalid!("面 {k}: Cylinder Fresnel の項の数 {nt}");
                }
                let coef = (0..nt as i64)
                    .map(|j| s.xdat.get(&(j + 4)).copied().unwrap_or(0.0))
                    .collect();
                d.cylf = Some(std::sync::Arc::new(crate::surface::CylFresnel {
                    c: s.xdat.get(&2).copied().unwrap_or(0.0),
                    k: s.xdat.get(&3).copied().unwrap_or(0.0),
                    coef,
                }));
            }
            _ => {}
        }
        if matches!(
            typ,
            SurfType::Standard
                | SurfType::Fresnel
                | SurfType::CylFresnel
                | SurfType::Binary2
                | SurfType::EvenAsph
                | SurfType::Jones
                | SurfType::Toroidal
                | SurfType::ZernikeStd
                | SurfType::ZernikeFringe
                | SurfType::GridSag
                | SurfType::Gradium
                | SurfType::Gradient9
                | SurfType::OddAsph
                | SurfType::TiltSurf
                | SurfType::Irregular
        ) {
            d.c = fop("CRVT", ki, s.curv.unwrap_or(0.0));
            d.k = fop("CONN", ki, s.coni.unwrap_or(0.0));
            if matches!(
                typ,
                SurfType::EvenAsph | SurfType::Fresnel | SurfType::CylFresnel | SurfType::Binary2
            ) || crate::freeform::is_freeform(typ)
            {
                d.asph = (1..=8).map(p).collect();
            } else if typ == SurfType::Toroidal {
                // パラメータ 1 は回転半径（0 は無限 = 円柱面）、2〜8 は y^2〜y^14 の係数
                let r = p(1);
                d.cx = if r == 0.0 { 0.0 } else { 1.0 / r };
                d.asph = (2..=8).map(p).collect();
                let nz = s.xdat.get(&1).copied().unwrap_or(0.0);
                if nz != 0.0 {
                    notimpl!("面 {k}: トロイダル面の Zernike の項（{nz} 項）");
                }
            } else if typ == SurfType::Standard
                && d.bire.is_none()
                && parm[k].values().any(|&v| v != 0.0)
            {
                notimpl!("面 {k}: 標準面にパラメータ");
            }
            if typ == SurfType::Jones && d.c != 0.0 {
                notimpl!("曲面の Jones 行列面 {k}");
            }
        }
        after_picks.push(tilts(s, &mut d, k)?);
        aperture(s, &mut d, &ops);
        let n = if d.mirror || typ == SurfType::CoordBrk {
            nprev.clone()
        } else if let Some(g) = &d.grin {
            // GRIN: 頂点の屈折率
            wl.iter()
                .map(|&l| {
                    g.vertex_index(l).ok_or_else(|| {
                        Error::Invalid(format!("面 {k}: GRIN の頂点が分布の範囲の外"))
                    })
                })
                .collect::<Result<Vec<f64>>>()?
        } else if mat.is_empty() || mat == "AIR" {
            vec![1.0; wl.len()]
        } else if let Some((nd, vd, dp)) = model {
            if vd == 0.0 {
                vec![nd; wl.len()]
            } else {
                wl.iter()
                    .map(|&l| glass::model_index(nd, vd, dp, l))
                    .collect()
            }
        } else {
            let g = glasses
                .get(&mat)
                .ok_or_else(|| Error::Invalid(format!("ガラス {mat}（面 {k}）がカタログにない")))?;
            wl.iter().map(|&l| g.index(l, temp, pres, adjust)).collect()
        };
        d.n = n.clone();
        if let Some(crate::surface::Bire::In { ne, .. }) = d.bire.as_mut() {
            let en = crate::surface::Bire::e_name(&mat);
            let g = glasses.get(&en).ok_or_else(|| {
                Error::Invalid(format!(
                    "複屈折材 {mat} の異常光のガラス {en}（面 {k}）がカタログにない"
                ))
            })?;
            *ne = wl.iter().map(|&l| g.index(l, temp, pres, adjust)).collect();
        }
        if !ign {
            nprev = n;
        }
        sf.push(d);
    }
    // 面の後の傾き偏心のピックアップ: 元の面の前の値と順序（反転は符号と順序が逆）。ファイルの SCBD 2 の値は使わない
    for k in 0..ns {
        if let Some((src, rev)) = after_picks[k] {
            sf[k].after = match sf[src].before {
                Some(b) if rev => After::Tilt(Tilt {
                    d: b.d.map(|v| -v),
                    order: 1 - b.order.min(1),
                }),
                Some(b) => After::Tilt(b),
                None => After::None,
            };
        }
    }
    // 円錐係数のピックアップ（元の面は後ろにあってもよい）
    settled = false;
    for _ in 0..=ns {
        let mut changed = false;
        for k in 0..ns {
            if let Some((src, sc, off, None)) = raw.surfs[k].pkup {
                if src >= ns {
                    invalid!("面 {k}: 円錐係数のピックアップ元の面 {src} がない");
                }
                let v = sc * sf[src].k + off;
                if v != sf[k].k {
                    sf[k].k = v;
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
        invalid!("円錐係数のピックアップが収束しない（循環？）");
    }
    // ピックアップのつながりを系に残す（パラメータを変えたときに解決し直す）。
    // 元か先の面タイプにそのパラメータがなければ、読込の値だけにする
    let mut pickups: Vec<Pickup> = Vec::new();
    for (k, r) in raw.surfs.iter().enumerate() {
        let mut add =
            |src: usize, what: PickKind, scale: f64, offset: f64, from: Option<PickKind>| {
                pickups.push(Pickup {
                    from,
                    ..Pickup::new(k, src, what, scale, offset)
                })
            };
        if let Some((src, sc, off, from)) = r.pzup {
            add(src, PickKind::Thick, sc, off, from);
        }
        if let Some((src, sc, off, from)) = r.pkup {
            add(src, PickKind::Conic, sc, off, from);
        }
        if let Some((src, sc, from)) = r.curv_pickup {
            add(src, PickKind::Curv, sc, 0.0, from);
        }
        if let Some((src, sc, from)) = r.diam_pickup {
            add(src, PickKind::Semi, sc, 0.0, from);
        }
        if let Some(src) = r.glas_pickup {
            // 材料は読込で元の面のものにした。屈折率のずれ（dn・disp）も元の面に追従させる
            add(src, PickKind::Dn, 1.0, 0.0, None);
            add(src, PickKind::Disp, 1.0, 0.0, None);
        }
        for (&p, &(src, sc, off, from)) in &r.ppar {
            let Ok(i) = u8::try_from(p) else { continue };
            let has = |j: usize, w: PickKind| match w {
                PickKind::Parm(i) => surf_parm(&sf[j], i).is_some(),
                _ => true,
            };
            if src < ns && has(k, PickKind::Parm(i)) && has(src, from.unwrap_or(PickKind::Parm(i)))
            {
                add(src, PickKind::Parm(i), sc, off, from);
            }
        }
        if let Some((src, rev)) = after_picks[k] {
            // 面の後の傾き偏心 = 元の面の前の傾き偏心（反転は値の符号と順序が逆）
            for i in 0..5u8 {
                pickups.push(Pickup {
                    from: Some(PickKind::Before(i)),
                    ..Pickup::new(
                        k,
                        src,
                        PickKind::After(i),
                        if rev { -1.0 } else { 1.0 },
                        0.0,
                    )
                });
            }
        }
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
    // OpticStudio 2026 R1 は読込で、半直径を固定した玉の面（前か後ろがガラス）で開口のない面に浮動開口（FLAP）を足してける
    // （空気どうしの面には足さない）。MEMA の行があっても足す（2026-11-09 に FLAP の行を消したファイルで確かめた。
    // 以前は MEMA のない .zmx だけとしていた）。同じにする。絞りの大きさで決める開口（Float By Stop Size）の絞りは瞳そのものなので除く
    {
        let glass = |m: &Option<String>| {
            m.as_deref()
                .is_some_and(|g| !g.is_empty() && !g.eq_ignore_ascii_case("MIRROR"))
        };
        for k in 1..raw.surfs.len().saturating_sub(1) {
            let r = &raw.surfs[k];
            let lens = glass(&r.glas)
                || glass(&raw.surfs[k - 1].glas)
                || r.model_glass.is_some()
                || raw.surfs[k - 1].model_glass.is_some();
            let float_stop = kind == ApertureKind::Float && r.stop;
            if r.diam_fixed && lens && !float_stop && sf[k].ap == Aperture::None {
                sf[k].ap = Aperture::Floating { semi: sf[k].semi };
            }
        }
    }
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
            .find(|p| crate::vfs::exists(p))
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
    // Afocal Image Space は FTYP の 7 番目（OpticStudio 26.1 で確認。保存すると HYPR も 1 になるが、読むときは FTYP だけが効く）
    s.afocal = ftyp.get(6) == Some(&1);
    // UNIT の値の 6 番目（MM X W X CM の次）が Afocal Mode Units（UR・MR・RR・AS・AM・DG、OpticStudio 26.1 で確認）。無ければ mrad
    // （知らない記号はアフォーカルのときだけ誤り）
    if let Some(u) = sys_str(raw, "UNIT", 5) {
        match crate::system::AfocalUnit::from_code(u) {
            Some(a) => s.afocal_unit = a,
            None if s.afocal => invalid!("UNIT の Afocal Mode Units {u}"),
            None => {}
        }
    }
    s.coating_file = coating_file;
    s.name = name;
    s.adjust_index = adjust;
    s.pickups = pickups;
    if !s.pickups.is_empty() {
        // OpticStudio は読込でピックアップを解き直す（保存された値が古くても元の面に合わせる）
        s.resolve_pickups()?;
        s.changed()?;
    }
    s.solves = raw
        .surfs
        .iter()
        .enumerate()
        .flat_map(|(k, r)| {
            [r.csolve, r.tsolve]
                .into_iter()
                .flatten()
                .map(move |(kind, h)| ThickSolve {
                    surf: k,
                    height: h,
                    kind,
                })
        })
        .collect();
    attach_macros(&mut s, raw, opt, base);
    // OpticStudio は読込でソルブを解き直す（保存した DISZ が古くても同じ系になる）
    let is_macro = |k: &SolveKind| matches!(k, SolveKind::MacroCurv(_) | SolveKind::MacroThick(_));
    if s.solves.iter().any(|q| is_macro(&q.kind)) || !s.macro_parms.is_empty() {
        let before = s.clone();
        if let Err(e) = s.solve_thickness() {
            // マクロのソルブが収まらない（自分の値を読むなど）・解き直しの途中でエラーになる: マクロのソルブを外す
            s = before;
            for q in s.solves.iter().filter(|q| is_macro(&q.kind)) {
                let name = match q.kind {
                    SolveKind::MacroCurv(i) | SolveKind::MacroThick(i) => {
                        s.macros[i as usize].name.clone()
                    }
                    _ => unreachable!(),
                };
                s.macro_notes.push(format!(
                    "面 {} のマクロのソルブ {name} を外して、保存された値で固定した: 解き直せない（{e}）",
                    q.surf
                ));
            }
            s.solves.retain(|q| !is_macro(&q.kind));
            for q in &s.macro_parms {
                let name = s.macros[q.id as usize].name.clone();
                s.macro_notes.push(format!(
                    "面 {} の{}のマクロのソルブ {name} を外して、保存された値で固定した: 解き直せない（{e}）",
                    q.surf,
                    q.col.label(q.parm)
                ));
            }
            s.macro_parms.clear();
        }
    }
    s.solve_thickness()?;
    // 自動の半直径を光線から求める（OpticStudio は読込で求め直す）
    s.update_semi();
    // 玉モードの玉（lens1, lens2, …）を玉・群のパラメータで使えるように
    s.units = s.lens_units();
    Ok(s)
}

/// ZPL Macro Solveのマクロを探して系に付ける。マクロが見つからない・読めない・対応していない命令を使う・
/// 今の系で値が出ないときは、ソルブを付けずに保存された値で固定し、理由を [`System::macro_notes`] に残す
fn attach_macros(s: &mut System, raw: &Raw, opt: &LoadOptions, base: Option<&Path>) {
    let dirs = crate::zpl_solve::macro_dirs(&opt.macro_dirs, base);
    for (k, r) in raw.surfs.iter().enumerate() {
        use crate::zpl_solve::MacroCol;
        // 曲率・厚さはソルブ（ThickSolve）、ほかの列は MacroParm
        enum T {
            Curv,
            Thick,
            Col(MacroCol, u8),
        }
        let mut todo: Vec<(T, &String)> = Vec::new();
        todo.extend(r.cmacro.iter().map(|n| (T::Curv, n)));
        todo.extend(r.tmacro.iter().map(|n| (T::Thick, n)));
        todo.extend(r.kmacro.iter().map(|n| (T::Col(MacroCol::Conic, 0), n)));
        todo.extend(r.smacro.iter().map(|n| (T::Col(MacroCol::Semi, 0), n)));
        todo.extend(
            r.pmacros
                .iter()
                .map(|(p, n)| (T::Col(MacroCol::Parm, *p), n)),
        );
        for (what, name) in todo {
            let col = match &what {
                T::Curv => "曲率".to_string(),
                T::Thick => "厚さ".to_string(),
                T::Col(c, p) => c.label(*p),
            };
            let got = crate::zpl_solve::find_macro(name, &dirs).and_then(|m| {
                if let T::Col(c, p) = &what {
                    if crate::param::pick_get(&s.surfaces[k], c.pick(*p)).is_none() {
                        notimpl!("面タイプ {:?} の {col}", s.surfaces[k].typ);
                    }
                }
                s.run_macro(&m, k)?;
                Ok(m)
            });
            match got {
                Ok(m) => {
                    let id = match s
                        .macros
                        .iter()
                        .position(|x| x.name == m.name && x.source == m.source)
                    {
                        Some(i) => i,
                        None => {
                            s.macros.push(m);
                            s.macros.len() - 1
                        }
                    } as u16;
                    match what {
                        T::Curv | T::Thick => s.solves.push(ThickSolve {
                            surf: k,
                            height: 0.0,
                            kind: if matches!(what, T::Curv) {
                                SolveKind::MacroCurv(id)
                            } else {
                                SolveKind::MacroThick(id)
                            },
                        }),
                        T::Col(col, parm) => s.macro_parms.push(crate::zpl_solve::MacroParm {
                            surf: k,
                            parm,
                            id,
                            col,
                        }),
                    }
                }
                Err(e) => s.macro_notes.push(format!(
                    "面 {k} の{col}のマクロのソルブ {name} を外して、保存された値で固定した: {e}"
                )),
            }
        }
    }
}

/// 面ごとの材料（名前, モデルガラス）。多重構成の GLSS を当て、材料のピックアップ（GLAS … 2 元の面）は元の面の
/// 材料にする（連なっていても、元の面が後ろにあってもよい）
type Material = (String, Option<(f64, f64, f64)>);

fn materials(raw: &Raw, ops: &HashMap<(String, i64), String>) -> Result<Vec<Material>> {
    let ns = raw.surfs.len();
    let own = |k: usize| -> Material {
        let s = &raw.surfs[k];
        match ops.get(&("GLSS".to_string(), k as i64)) {
            Some(v) => (v.trim_matches('"').to_string(), None),
            None => (s.glas.clone().unwrap_or_default(), s.model_glass),
        }
    };
    (0..ns)
        .map(|k| {
            let mut j = k;
            for _ in 0..=ns {
                // 多重構成で材料を与えた面はピックアップより多重構成が勝つ
                let picked = raw.surfs[j]
                    .glas_pickup
                    .filter(|_| !ops.contains_key(&("GLSS".to_string(), j as i64)));
                match picked {
                    None => return Ok(own(j)),
                    Some(src) if src >= ns => {
                        invalid!("面 {j}: 材料のピックアップ元の面 {src} がない")
                    }
                    Some(src) => j = src,
                }
            }
            invalid!("面 {k}: 材料のピックアップが循環している")
        })
        .collect()
}

/// 厚さのピックアップ（PZUP）を値にする
fn thickness_pickups(raw: &Raw, th: &mut [f64]) -> Result<()> {
    for _ in 0..=th.len() {
        let mut changed = false;
        for (k, r) in raw.surfs.iter().enumerate() {
            if let Some((src, sc, off, None)) = r.pzup {
                let v = sc * th.get(src).copied().unwrap_or(0.0) + off;
                if v != th[k] {
                    th[k] = v;
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(());
        }
    }
    invalid!("厚さのピックアップが収束しない（循環？）")
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

/// 面の傾き偏心。面の後のピックアップなら（元の面, 反転）を返す（値は系を作ってから元の面の前の値で解決する）
fn tilts(s: &RawSurf, d: &mut Surface, k: usize) -> Result<Option<(usize, bool)>> {
    let mut pick = None;
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
                // 1 = この面の前と同じ、2·(この面 − 元の面) + 1 は元の面の前の値、+2 は反転（OpticStudio 2026 R1.01
                // で実測。元の面の後の値は使わない。後ろの面からは取れない）
                m if m >= 1 => {
                    let off = ((m - 1) / 2) as usize;
                    if off > k {
                        invalid!("面 {k}: 面の後の傾き偏心のピックアップ元の面（モード {m}）");
                    }
                    pick = Some((k - off, m % 2 == 0));
                }
                _ => notimpl!("面 {k}: 面の後の傾き偏心のモード {mode}"),
            },
            _ => notimpl!("面 {k}: SCBD {which}"),
        }
    }
    Ok(pick)
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
    } else if let Some(c) = &s.elap {
        Aperture::Ellipse {
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
    } else if let Some(c) = &s.spid {
        Aperture::Spider {
            width: g(c, 0),
            arms: g(c, 1) as u32,
            dx,
            dy,
        }
    } else {
        Aperture::None
    };
}

/// Zernike Sag・Grid Sag の面の読込（OpticStudio 26.1 の書き方、非公開側の照合データで確認）:
/// PARM 0 は外挿（Zernike、1 = する）か補間（Grid、0 = 双三次・1 = 双一次）、PARM 1〜8 は r^2〜r^16、
/// PARM 9・10 は Zernike の項の偏心、XDAT 1 は項の数、XDAT 2 は正規化半径、XDAT k+2 は項 k の係数。
/// Grid Sag は GDAT（nx ny dx dy xdec ydec unit …）と GARR（.DAT の行の順: +y の行から、行の中は −x から）
fn freeform_surface(
    s: &RawSurf,
    d: &mut Surface,
    typ: SurfType,
    p: &dyn Fn(i64) -> f64,
    k: usize,
) -> Result<()> {
    use crate::freeform::{GridSag, ZernSag};
    use std::sync::Arc;
    let fringe = typ == SurfType::ZernikeFringe;
    let nterm = s.xdat.get(&1).copied().unwrap_or(0.0);
    if nterm < 0.0 || nterm.fract() != 0.0 || nterm as usize > ZernSag::max_terms(fringe) {
        invalid!("面 {k}: Zernike の項の数 {nterm}");
    }
    let nterm = nterm as usize;
    if nterm > 0 {
        let norm = s.xdat.get(&2).copied().unwrap_or(100.0);
        if !(norm > 0.0) {
            invalid!("面 {k}: Zernike の正規化半径 {norm}");
        }
        let coef = (1..=nterm)
            .map(|j| s.xdat.get(&(j as i64 + 2)).copied().unwrap_or(0.0))
            .collect();
        // Grid Sag には外挿の列がない（常に外挿する）
        let extrapolate = typ == SurfType::GridSag || s.parm.get(&0).copied().unwrap_or(1.0) != 0.0;
        let mut z = ZernSag::new(fringe, norm, coef, extrapolate);
        z.dx = p(9);
        z.dy = p(10);
        d.zern = Some(Arc::new(z));
    } else if typ != SurfType::GridSag {
        d.zern = Some(Arc::new(ZernSag::new(fringe, 100.0, Vec::new(), true)));
    }
    if typ == SurfType::GridSag {
        let Some(h) = &s.gdat else {
            invalid!("面 {k}: Grid Sag の格子（GDAT）がない");
        };
        let g = |i: usize| h.get(i).copied().unwrap_or(0.0);
        let (nx, ny) = (g(0) as usize, g(1) as usize);
        if nx < 2 || ny < 2 || !(g(2) > 0.0 && g(3) > 0.0) {
            invalid!("面 {k}: Grid Sag の格子 {nx}×{ny}、間隔 {} {}", g(2), g(3));
        }
        if g(6) != 0.0 {
            notimpl!("面 {k}: Grid Sag の格子の単位 {}（mm 以外）", g(6));
        }
        let mut z = vec![[0.0; 4]; nx * ny];
        let mut nodata = vec![false; nx * ny];
        let mut seen = vec![false; nx * ny];
        for &(i, v) in &s.garr {
            if i >= nx * ny {
                invalid!("面 {k}: GARR の番号 {i}（格子は {nx}×{ny}）");
            }
            let kk = (ny - 1 - i / nx) * nx + i % nx;
            z[kk] = [v[0], v[1], v[2], v[3]];
            nodata[kk] = v[4] != 0.0;
            seen[kk] = true;
        }
        if seen.iter().any(|&b| !b) {
            invalid!(
                "面 {k}: GARR の点が足りない（{} / {}）",
                s.garr.len(),
                nx * ny
            );
        }
        let interp = p(0);
        if interp != 0.0 && interp != 1.0 {
            notimpl!("面 {k}: Grid Sag の補間 {interp}");
        }
        d.grid = Some(Arc::new(GridSag {
            nx,
            ny,
            dx: g(2),
            dy: g(3),
            xdec: g(4),
            ydec: g(5),
            z,
            nodata,
            linear: interp == 1.0,
        }));
        d.grid_file = s.fimp.clone().unwrap_or_default();
    } else {
        let z = d.zern.as_mut().map(Arc::make_mut).unwrap();
        z.dx = p(9);
        z.dy = p(10);
    }
    Ok(())
}
