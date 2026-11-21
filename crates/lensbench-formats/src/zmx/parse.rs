//! .zmx の文を行ごとに読んで [`Raw`] にする（面の行・系の行・多重構成の行）

use super::*;

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

/// 多重構成の行の値の後の欄: 種類（0 固定・1 変数・2 以上は構成のピックアップで元の構成 + 1）・番号 2・番号 3・
/// 元の構成・元の行・倍率・オフセット（OpticStudio 26.1 で確認。同じ演算子の別の行からも拾える）。表のピックアップは
/// 同じ行からだけなので、別の行からのものと、種類と元の構成が合わないもの（熱のピックアップは元の構成 1 で種類 5 と
/// 書かれる）は固定（種類 0）にして値を使い、知らせの文を返す
fn config_pickup_columns(rest: &mut [String], row: usize) -> Option<String> {
    let num = |i: usize| rest.get(i).and_then(|x| x.parse::<i64>().ok());
    let (t, from, src) = (num(0)?, num(3), num(4));
    if t < 2 {
        return None;
    }
    let why = if from != Some(t - 1) {
        format!("the solve of kind {t} (a thermal pickup or another one) was read as its value")
    } else if src != i64::try_from(row).ok() {
        format!(
            "the configuration pickup from row {} was read as its value",
            src.unwrap_or(0)
        )
    } else {
        return None;
    };
    rest[0] = "0".into();
    Some(why)
}

pub fn parse_text(txt: &str) -> Result<Raw> {
    let mut raw = Raw::default();
    let mut cur: Option<usize> = None;
    let mut in_mce = false;
    // 多重構成の表の前の文の構成（構成の番号が増えなければ次の行）
    let mut last_cfg = i64::MAX;
    // 面の後の字下げしない行は評価関数（OpticStudio は面の後に評価関数・公差（TOL）・多重構成（MNUM）の順で書く）
    let mut in_merit = false;
    // 古い .zmx（Edmund の一部の品など）は面の行を字下げしない。字下げした行が 1 つも無いときは、
    // SURF の後の行を全て面の行として読み、知らない行は飛ばす（小文字の stop は絞り）
    let legacy = !txt.lines().any(|l| l.starts_with("  "));
    for (no, line) in txt.lines().enumerate() {
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
            in_merit = false;
            cur = None;
            raw.sysl.entry(key.into()).or_default().push(rest());
            continue;
        }
        if key == "TOL" {
            in_merit = false;
        }
        if in_merit {
            raw.merit
                .push(merit::MeritRow::parse(raw.merit.len() + 1, no + 1, line));
            continue;
        }
        if in_mce {
            // 多重構成の表の空の行（BLNK）は光線追跡に効かない。番号・値の有無に依らず飛ばし、行の番号だけ数える
            if key == "BLNK" {
                let cfg = mce_line(line).map_or(i64::MAX, |m| m.2);
                if cfg == i64::MAX || cfg <= last_cfg {
                    raw.mce_rows.push((key.to_string(), 0, 0));
                }
                last_cfg = cfg;
                continue;
            }
            let Some(mut m) = mce_line(line) else {
                invalid!("can't read the zmx multi-configuration line: {line}");
            };
            // FVAN と FTAN は同じ量（OpticStudio 26.1 で確認）。同じ名前にして、表と構成を選んで読む経路のどちらでも
            // 後の行を勝たせる
            if m.0 == "FVAN" {
                m.0 = "FTAN".into();
            }
            // PAR1〜PAR8 は PRAM の PARM 1〜8 の古い名前（OpticStudio 26.1 は PRAM にして書く）
            if let Some(n) = m.0.strip_prefix("PAR").and_then(|n| n.parse::<u8>().ok()) {
                if (1..=8).contains(&n) {
                    m.0 = "PRAM".into();
                    m.4.resize(m.4.len().max(2), "0".into());
                    m.4[1] = n.to_string();
                }
            }
            if m.2 <= last_cfg {
                let parm = if m.0 == "PRAM" || m.0.starts_with("PSP") {
                    m.4.get(1).and_then(|x| x.parse::<i64>().ok()).unwrap_or(-1)
                } else {
                    0
                };
                raw.mce_rows.push((m.0.clone(), m.1, parm));
            }
            last_cfg = m.2;
            if let Some(why) = config_pickup_columns(&mut m.4, raw.mce_rows.len()) {
                raw.notes.push(format!(
                    "multi-configuration row {} configuration {}: {why}",
                    raw.mce_rows.len(),
                    m.2
                ));
            }
            if m.0 == "IGNM" {
                // 面の範囲の無視（番号 1 から 2 番目の番号の面まで）は面ごとの IGNR にする（同じ面の IGNR とは後の行が勝つ）
                let end =
                    m.4.get(1)
                        .and_then(|x| x.parse::<i64>().ok())
                        .unwrap_or(m.1)
                        .min(m.1.saturating_add(100_000));
                for k in m.1..=end {
                    raw.mce
                        .push(("IGNR".into(), k, m.2, m.3.clone(), m.4.clone()));
                }
                continue;
            }
            raw.mce.push(m);
            continue;
        }
        let Some(ci) = cur.filter(|_| legacy || line.starts_with("  ")) else {
            if cur.is_some() && key != "TOL" {
                in_merit = true;
                cur = None;
                raw.merit
                    .push(merit::MeritRow::parse(raw.merit.len() + 1, no + 1, line));
                continue;
            }
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
            // PKUP 元の面 倍率 列（円錐係数はオフセットなし）。列は [`Kind::from_column`]（OpticStudio 2026 R1.01 で実測）
            // 元の面が負（OpticStudio 2026 R1 の座標ブレークの主光線のソルブがこう保存される）は、保存された値で固定する
            // （PARM は解いた値。付属のサンプルで OpticStudio と光線が一致）
            "PPAR" if num(2)? < 0.0 => {}
            "PPAR" => {
                let i = num(1)? as i64;
                let from = pick_col(s.n, "PPAR", num(5)?, Kind::Parm(i.clamp(0, 255) as u8))?;
                s.ppar
                    .insert(i, (surf_no(num(2)?)?, num(3)?, num(4)?, from));
            }
            "PZUP" => {
                let from = pick_col(s.n, "PZUP", num(4)?, Kind::Thick)?;
                s.pzup = Some((surf_no(num(1)?)?, num(2)?, num(3)?, from));
            }
            // 厚さのソルブ（OpticStudio 2026 R1.01 で実測）。OPDZ OPD 帯
            "MAZH" => {
                let z = num(2)?;
                let k = if z == 0.0 {
                    SolveKind::MarginalHeight { height: 0.0 }
                } else {
                    SolveKind::MarginalHeightZone {
                        height: 0.0,
                        zone: z,
                    }
                };
                s.tsolve = Some((k, num(1)?));
            }
            "CHZH" => s.tsolve = Some((SolveKind::ChiefHeight { height: 0.0 }, num(1)?)),
            "TPUP" => s.tsolve = Some((SolveKind::PupilPosition, 0.0)),
            "EDGE" => {
                s.tsolve = Some((
                    SolveKind::Edge {
                        thickness: 0.0,
                        radius: num(2)?,
                    },
                    num(1)?,
                ))
            }
            "TOLE" => {
                s.tsolve = Some((
                    SolveKind::Position {
                        other: surf_no(num(1)?)?,
                        length: 0.0,
                    },
                    num(2)?,
                ))
            }
            "TCOM" => {
                s.tsolve = Some((
                    SolveKind::Compensator {
                        other: surf_no(num(1)?)?,
                        sum: 0.0,
                    },
                    num(2)?,
                ))
            }
            "TCOC" => {
                s.tsolve = Some((
                    SolveKind::CenterOfCurvature {
                        other: surf_no(num(1)?)?,
                    },
                    0.0,
                ))
            }
            "OPDZ" => {
                s.tsolve = Some((
                    SolveKind::Opd {
                        opd: 0.0,
                        zone: num(2)?,
                    },
                    num(1)?,
                ))
            }
            "PKUP" => {
                let from = pick_col(s.n, "PKUP", num(3)?, Kind::Conic)?;
                s.pkup = Some((surf_no(num(1)?)?, num(2)?, 0.0, from));
            }
            "XDAT" => {
                s.xdat.insert(num(1)? as i64, num(2)?);
            }
            "GDAT" => s.gdat = Some(nums()?),
            "GARR" => {
                let v = nums()?;
                if v.len() < 5 {
                    invalid!("surface {}: too few GARR values", s.n);
                }
                let g = |i: usize| v.get(i).copied().unwrap_or(0.0);
                s.garr.push((
                    count_in(v[0], "GARR number", 0, MAX_GRID)?,
                    [g(1), g(2), g(3), g(4), g(5)],
                ));
            }
            "FIMP" => s.fimp = Some(line.trim().get(5..).unwrap_or("").trim().to_string()),
            "TYPE" => s.typ = Some(t.get(1).unwrap_or(&"").to_string()),
            "COAT" => s.coat = Some(t.get(1).unwrap_or(&"").to_string()),
            // GLAS 名前 ソルブ 元の面 nd vd ΔPgF …。ソルブ 1 はモデルガラス（名前 ___BLANK）、2 は材料のピックアップ
            // （Ghost Focus Generator が保存するゴーストの系の戻りの面など。名前と nd・vd は保存時の値で、使わない）
            "GLAS" => {
                s.glas = Some(t.get(1).unwrap_or(&"").to_string());
                // ソルブ 4 は Offset（欄 10 が Δnd、欄 11 が Δvd、OpticStudio 2026 R1.01 で実測）。ずれの式を確かめていないので止める
                if t.get(2) == Some(&"4") && (num(10)? != 0.0 || num(11)? != 0.0) {
                    let (surf, dnd, dvd) = (s.n, num(10)?, num(11)?);
                    notimpl!("zmx surface {surf}: material Offset solve (Δnd {dnd}, Δvd {dvd})");
                }
                if t.get(2) == Some(&"2") {
                    s.glas_pickup = Some(surf_no(num(3)?)?);
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
                        let from = pick_col(s.n, "CURV", num(5)?, Kind::Curv)?;
                        s.curv_pickup = Some((surf_no(p1)?, num(4)?, from));
                        None
                    }
                    // 曲率のソルブ（OpticStudio 2026 R1.01 で実測）
                    "2" => Some((SolveKind::MarginalAngle { angle: 0.0 }, p1)),
                    "3" => Some((SolveKind::ChiefAngle { angle: 0.0 }, p1)),
                    "5" => Some((SolveKind::MarginalNormal, 0.0)),
                    "6" => Some((SolveKind::ChiefNormal, 0.0)),
                    "7" => Some((SolveKind::Aplanatic, 0.0)),
                    "8" => Some((SolveKind::ElementPower { power: 0.0 }, p1)),
                    "9" => Some((
                        SolveKind::ConcentricSurface {
                            other: surf_no(p1)?,
                        },
                        0.0,
                    )),
                    "10" => Some((
                        SolveKind::ConcentricRadius {
                            other: surf_no(p1)?,
                        },
                        0.0,
                    )),
                    "11" => Some((SolveKind::FNumber { fnum: 0.0 }, p1)),
                    // ZPL マクロのソルブ（最後の欄の "…" がマクロの名前）。マクロは build で探す
                    "12" => {
                        s.cmacro = Some(macro_name(s.n, line)?);
                        None
                    }
                    c => notimpl!("zmx surface {}: curvature solve {c}", s.n),
                };
            }
            "CONI" => s.coni = Some(num(1)?),
            // ZPL マクロのソルブ（OpticStudio 26.1.1 で実測）
            "TZPL" => s.tmacro = Some(macro_name(s.n, line)?),
            "KZPL" => s.kmacro = Some(macro_name(s.n, line)?),
            "PZPL" => {
                let n = num(1)?;
                if !(0.0..=255.0).contains(&n) || n.fract() != 0.0 {
                    invalid!("zmx surface {}: PZPL parameter number {n}", s.n);
                }
                s.pmacros.push((n as u8, macro_name(s.n, line)?));
            }
            "DISZ" => s.disz = Some(num(1)?),
            "TCED" => s.tced = Some(num(1)?),
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
                    let from = pick_col(s.n, "DIAM", num(4)?, Kind::Semi)?;
                    s.diam_pickup = Some((surf_no(num(3)?)?, num(5)?, from));
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
                    invalid!("zmx surface {}: spider arm width {w}, {n} arms", s.n);
                }
                s.spid = Some(v);
            }
            "OBDC" => s.obdc = Some(nums()?),
            "SCBD" => s.scbd.push(rest()),
            "STOP" => s.stop = true,
            // 面の無視（Ignore This Surface）: OpticStudio は HIDE の 6 番目の値を 1 にして書く（IGNR は多重構成の
            // 演算子の名前で、面の行にはない）
            "HIDE" => s.ignore = t.get(6).is_some_and(|v| *v != "0"),
            // MEMA 機械半径 固定(1)・自動(0)・ピックアップ(2)、OEMA チップゾーン（OpticStudio 26.1 で実測）。ピックアップは
            // DIAM と同じ形 MEMA 値 2 元の面 列 倍率 "マクロ"（列は 0 だけ、オフセットなし。OpticStudio 2026 R1.01 で実測）
            // 機械半径は追跡に効かない（縁の厚み・体積・図だけ）ので、知らないソルブ・0 でない列は止めずに自動にして注意を出す
            "MEMA" => match t.get(2).copied().unwrap_or("0") {
                "0" => {}
                "1" => s.mema = Some(num(1)?),
                "2" if num(4)? == 0.0 => {
                    s.mema = Some(num(1)?);
                    s.mema_pickup = Some((surf_no(num(3)?)?, num(5)?));
                }
                "2" => {
                    let (n, col) = (s.n, num(4)?);
                    raw.notes.push(format!(
                        "zmx surface {n}: the mechanical semi-diameter picks up column {col}, which is not supported; it was read as automatic"
                    ));
                }
                m => {
                    let n = s.n;
                    raw.notes.push(format!(
                        "zmx surface {n}: mechanical semi-diameter solve {m} is not supported; it was read as automatic"
                    ));
                }
            },
            "OEMA" => s.oema = num(1)?,
            k if HARMLESS.contains(&k) => {}
            k if legacy => raw.notes.push(format!(
                "zmx surface {}: line {k} was skipped (an old .zmx without indentation)",
                s.n
            )),
            // 非順次の物体（非順次モード・混合モードの系）: 光線に効くので飛ばせない
            "NSOH" => notimpl!(
                "zmx surface {}: non-sequential objects (NSOH) are not supported (a non-sequential or mixed-mode system)",
                s.n
            ),
            k => notimpl!("zmx surface {}: line {} is not supported", s.n, k),
        }
    }
    raw.check_counts()?;
    Ok(raw)
}

/// マクロのソルブの行の名前（最初の " から最後の " までの中身）
fn macro_name(n: i64, line: &str) -> Result<String> {
    match (line.find('"'), line.rfind('"')) {
        (Some(a), Some(b)) if b > a + 1 => Ok(line[a + 1..b].to_string()),
        _ => invalid!("zmx surface {n}: no macro solve name"),
    }
}

/// 面の番号の欄
fn surf_no(v: f64) -> Result<usize> {
    if v < 0.0 || v.fract() != 0.0 {
        invalid!("surface number {v}");
    }
    Ok(v as usize)
}

/// ピックアップの列のコードから元の量（同じ列なら None）
fn pick_col(n: i64, key: &str, col: f64, dst: Kind) -> Result<Option<Kind>> {
    match Kind::from_column(col as i64, dst).filter(|_| col.fract() == 0.0) {
        Some(f) => Ok(f),
        None => notimpl!("zmx surface {n}: pickup column {col} of {key}"),
    }
}
