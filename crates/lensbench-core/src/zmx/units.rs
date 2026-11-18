//! mm 以外のレンズ単位の .zmx を mm に直す

use super::values::aperture_type;
use super::*;

/// mm 以外のレンズ単位（UNIT の CM・IN・METER）の .zmx を mm に直す。長さの値（曲率・厚さ・半直径・開口・偏心・
/// 偶数次非球面の係数・厚さのソルブの値・入射瞳径・高さの視野・多重構成の THIC・SDIA・CRVT など）を換算する。
/// 換算の決まっていない面タイプ・データ（標準・偶数次非球面・座標ブレーク・傾いた面・奇数次非球面以外、拡張データ、
/// パラメータのピックアップのオフセットなど）は未対応で止める
pub(super) fn to_mm(raw: &Raw, unit: &str) -> Result<Raw> {
    let f = match unit.to_ascii_uppercase().as_str() {
        "CM" => 10.0,
        "IN" => 25.4,
        "METER" | "M" => 1000.0,
        u => notimpl!("lens unit {u} (mm, cm, in, m)"),
    };
    // 光線エイミングの瞳のずらし（RAIM の 5〜7 番目、長さ。lensbench の書く RAIM 0 種類 1 1 0 0 0 … の 0 0 0）は換算を確かめていないので止める
    if let Some(a) = raw.sysl.get("RAIM").and_then(|v| v.first()) {
        if a.iter()
            .skip(4)
            .take(3)
            .any(|x| fv(x).unwrap_or(0.0) != 0.0)
        {
            notimpl!("ray aiming pupil shift in lens unit {unit}");
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
            parm.insert(
                i,
                v * parm_scale(&typ, i, f)
                    .map_err(|e| e.at(format!("surface {k}: lens unit {unit}")))?,
            );
        }
        sf.parm = parm;
        if !sf.xdat.is_empty() || sf.gdat.is_some() {
            notimpl!("surface {k}: extra data in lens unit {unit}");
        }
        if sf.ppar.values().any(|p| p.2 != 0.0) {
            notimpl!("surface {k}: parameter pickup offset in lens unit {unit}");
        }
        if let Some(p) = sf.pzup.as_mut() {
            p.2 *= f;
        }
        if let Some((kind, v)) = sf.tsolve.as_mut() {
            match kind {
                SolveKind::MarginalHeight { .. }
                | SolveKind::ChiefHeight { .. }
                | SolveKind::Position { .. }
                | SolveKind::Compensator { .. } => *v *= f,
                SolveKind::MarginalHeightZone { .. } => *v *= f,
                SolveKind::Edge { radius: rr, .. } => {
                    *v *= f;
                    *rr *= f;
                }
                _ => {}
            }
        }
        if let Some((kind, v)) = sf.csolve.as_mut() {
            // 傾き・F 値・面の関係は単位によらない。玉のパワー [1/長さ] だけ換算
            if matches!(kind, SolveKind::ElementPower { .. }) {
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
    // 構成ごとの絞りの種類（APER の単位が種類で変わる）
    let satp0 = aperture_type(raw)?;
    let satp_of = |c: i64| -> i64 {
        raw.mce
            .iter()
            .find(|m| m.0 == "SATP" && m.2 == c)
            .and_then(|m| fv(&m.3).ok())
            .map(|v| v as i64)
            .unwrap_or(satp0)
    };
    let typs: Vec<String> = raw
        .surfs
        .iter()
        .map(|s| s.typ.clone().unwrap_or_else(|| "STANDARD".into()))
        .collect();
    for (op, p1, c, v, rest) in r.mce.iter_mut() {
        let k = match op.as_str() {
            "THIC" | "SDIA" | "APMX" => f,
            "CRVT" => 1.0 / f,
            "XFIE" | "YFIE" if height_fields => f,
            // 入射瞳径・浮動の絞りの半径は長さ、F 値・NA・コーン角はそのまま
            "APER" => match satp_of(*c) {
                0 | 3 => f,
                _ => 1.0,
            },
            "PRAM" => {
                let (Some(typ), Some(i)) = (
                    usize::try_from(*p1).ok().and_then(|k| typs.get(k)),
                    rest.get(1).and_then(|x| x.parse::<i64>().ok()),
                ) else {
                    invalid!("multi-configuration PRAM {p1}: bad surface or PARM number");
                };
                parm_scale(typ, i, f)
                    .map_err(|e| e.at(format!("multi-configuration PRAM {p1}: lens unit {unit}")))?
            }
            "XFIE" | "YFIE" | "IGNR" | "STPS" | "WAVE" | "GLSS" | "MCOM" | "CWGT" | "CONN"
            | "SATP" | "TELE" | "MOFF" | "LTTL" => 1.0,
            // 扱わないオペランドは構成で変わらなければ使わない（build で確かめる）ので換算しない
            _ => 1.0,
        };
        if k != 1.0 {
            *v = format!("{:e}", fv(v)? * k);
        }
    }
    Ok(r)
}

/// 面タイプ typ の PARM i の、レンズ単位の長さ f [mm] 倍への換算の倍率
fn parm_scale(typ: &str, i: i64, f: f64) -> Result<f64> {
    Ok(match typ {
        "STANDARD" | "TILTSURF" => 1.0,
        // r^{2i} の係数: [長さ]^{1−2i}
        "EVENASPH" => f.powi(1 - 2 * i as i32),
        // r^i の係数: [長さ]^{1−i}
        "ODDASPHE" => f.powi(1 - i as i32),
        // 偏心 x・y（1・2）と Zs・Za・Zc（5〜7）は長さ、傾き（3・4）・角度（8）はそのまま
        "IRREGULA" => {
            if matches!(i, 1 | 2 | 5 | 6 | 7) {
                f
            } else {
                1.0
            }
        }
        // 偏心 x・y（1・2）は長さ、傾き（3〜5）・順序（6）はそのまま
        "COORDBRK" => {
            if i == 1 || i == 2 {
                f
            } else {
                1.0
            }
        }
        t => notimpl!("unit conversion of the parameters of surface type {t}"),
    })
}
