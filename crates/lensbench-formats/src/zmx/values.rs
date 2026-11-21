//! .zmx の系の値: 構成の多重構成の値・波長・ガラス・開口・視野・そのほかの系の設定

use super::*;
use std::sync::Arc;

/// 構成 cfg の多重構成の値（オペランドと 1 番目の番号 → 値）。扱わないオペランドは構成で変わってはいけない
pub(super) fn config_ops(raw: &Raw, cfg: i64) -> Result<Ops> {
    let mut ops: Ops = HashMap::new();
    // 2 番目の番号（PRAM の PARM の番号など、多重構成の表の config_table と同じ位置）も鍵にする
    let key = |op: &String, p1: i64, rest: &[String]| (op.clone(), p1, rest.get(1).cloned());
    let mut vals: HashMap<(String, i64, Option<String>), HashSet<String>> = HashMap::new();
    for (op, p1, c, v, rest) in &raw.mce {
        // 変わるかは、数なら数で（1 と 1.0 は同じ）、数でなければ書いた文字で見る（表の config_table と同じ）
        let norm = fv(v).map_or_else(|_| v.clone(), |x| format!("{x:?}"));
        vals.entry(key(op, *p1, rest)).or_default().insert(norm);
        if *c == cfg {
            // PSP は面と PARM の番号で決まるので、鍵の名前に PARM の番号を付ける（configs::apply_solve_values）
            let name = match rest.get(1) {
                Some(n) if op.starts_with("PSP") => format!("{op}:{n}"),
                _ => op.clone(),
            };
            ops.insert((name, *p1), v.clone());
        }
    }
    // 誤りはファイルの順で最初のオペランド（毎回同じものを出す）
    for (op, p1, _, _, rest) in &raw.mce {
        if !MCE_HANDLED.contains(&op.as_str()) && vals[&key(op, *p1, rest)].len() > 1 {
            notimpl!("multi-configuration operand {op} {p1} changes between configurations");
        }
    }
    Ok(ops)
}

/// 波長と FTYP の欄
pub(super) struct Waves {
    /// FTYP の値（視野の種類・テレセントリック・視野の数・波長の数・視野の正規化・…・アフォーカル）
    pub ftyp: Vec<i64>,
    pub wl: Vec<f64>,
    pub wwt: Vec<f64>,
    pub primary: usize,
}

pub(super) fn wavelengths(raw: &Raw, ops: &Ops, opt: &LoadOptions) -> Result<Waves> {
    let ftyp: Vec<i64> = raw
        .sysl
        .get("FTYP")
        .and_then(|v| v.first())
        .map(|v| {
            v.iter()
                .map(|x| fv(x).map(|x| x as i64))
                .collect::<Result<Vec<i64>>>()
        })
        .transpose()?
        .unwrap_or_else(|| vec![0, 0, 1, 1]);
    // 数の範囲は parse_text で確かめている
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
            .ok_or_else(|| Error::Invalid(format!("no wavelength {i}")))?;
        wl.push(fop(ops, "WAVE", i, l));
        wwt.push(fop(ops, "WLWT", i, w));
    }
    if let Some(w) = &opt.wavelengths {
        wl = w.clone();
        wwt = vec![1.0; wl.len()];
    }
    let primary = count_in(
        fop(ops, "PRWV", 0, sys_f(raw, "PWAV", 0, 1.0)?),
        "zmx primary wavelength PWAV",
        1,
        MAX_WAVES,
    )? - 1;
    if primary >= wl.len() {
        let (pw, nw) = (primary + 1, wl.len());
        invalid!("primary wavelength {pw} (there are {nw} wavelengths)");
    }
    Ok(Waves {
        ftyp,
        wl,
        wwt,
        primary,
    })
}

/// 系のガラス
pub(super) struct Glasses {
    /// GCAT のカタログ（無ければ SCHOTT）
    pub cats: Vec<String>,
    pub map: Arc<HashMap<String, Glass>>,
    /// ガラスが無いときの誤りに添える文（見つからない・読めなかったカタログと、渡し方の案内）
    pub why: String,
}

pub(super) fn glasses(raw: &Raw, opt: &LoadOptions, dirs: &[PathBuf]) -> Glasses {
    let cats: Vec<String> = raw
        .sysl
        .get("GCAT")
        .and_then(|v| v.first())
        .cloned()
        .unwrap_or_else(|| vec!["SCHOTT".into()]);
    let (map, cat_missing): (Arc<HashMap<String, Glass>>, _) = match &opt.glasses {
        Some(g) => (Arc::new(g.clone()), Vec::new()),
        None => glass::load_catalogs_shared(&cats, dirs),
    };
    let why = glass::missing_note(&cat_missing) + glass::DIRS_HINT; // 「（見つからない…）」と渡し方の案内
    Glasses { cats, map, why }
}

/// 系の開口の種類と値。OpticStudio に合わせて、媒質の境の面に浮動開口を足す（sf を直す）
pub(super) fn aperture_kind(
    raw: &Raw,
    ops: &Ops,
    sf: &mut [Surface],
) -> Result<(ApertureKind, f64)> {
    let satp = fop(ops, "SATP", 0, aperture_type(raw)? as f64) as i64;
    let Some(kind) = ApertureKind::from_os_code(satp) else {
        notimpl!("system aperture type {satp}");
    };
    // OpticStudio 2026 R1 は読込で、半直径を固定した面のうち、前後で媒質が変わる面（ガラスと空気・別のガラスの境）で開口のない
    // 面に浮動開口（FLAP）を足してける（空気どうし・同じガラスどうしの面には足さない。MEMA の行があっても足す）。座標ブレークは
    // 半直径・材料を読込で捨てる（自動の 0、後ろの媒質は前のまま）ので足さない（2026-11-21 に 2026 R1.01 で確かめた）。
    // 鏡は前の媒質がガラスなら足す（裏面鏡。実測していないので前からの規則のまま）。絞りの大きさで決める開口（Float By Stop
    // Size）の絞りは瞳そのものなので除く
    let edge = lensbench_core::surface::medium_edges(sf);
    for k in 1..sf.len().min(raw.surfs.len()).saturating_sub(1) {
        let r = &raw.surfs[k];
        let cb = sf[k].typ() == SurfType::CoordBrk;
        let float_stop = kind == ApertureKind::Float && r.stop;
        if r.diam_fixed && edge[k] && !cb && !float_stop && sf[k].ap == Aperture::None {
            sf[k].ap = Aperture::Floating { semi: sf[k].semi };
        }
    }
    Ok((kind, fop(ops, "APER", 0, aperture_value(raw, satp)?)))
}

/// 視野
pub(super) struct Fields {
    pub tele: bool,
    pub typ: FieldType,
    pub norm: FieldNorm,
    pub list: Vec<Field>,
}

pub(super) fn fields(raw: &Raw, ops: &Ops, ftyp: &[i64]) -> Result<Fields> {
    let tele = fop(ops, "TELE", 0, *ftyp.get(1).unwrap_or(&0) as f64) != 0.0;
    let typ = match fop(ops, "FLTP", 0, ftyp[0] as f64) as i64 {
        0 | 4 => FieldType::Angle, // 経緯儀: ファイルには x・y の角度で入っている
        1 => FieldType::ObjH,
        2 => FieldType::ParH,
        3 => FieldType::RealH,
        t => notimpl!("field type {t}"),
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
    let list: Vec<Field> = (0..nf)
        .map(|i| Field {
            x: fop(ops, "XFIE", i as i64 + 1, xs[i]),
            y: fop(ops, "YFIE", i as i64 + 1, ys[i]),
            w: fop(ops, "FLWT", i as i64 + 1, fw[i]),
            vdx: fop(ops, "FVDX", i as i64 + 1, vd[0][i]),
            vdy: fop(ops, "FVDY", i as i64 + 1, vd[1][i]),
            vcx: fop(ops, "FVCX", i as i64 + 1, vd[2][i]),
            vcy: fop(ops, "FVCY", i as i64 + 1, vd[3][i]),
            // FVAN は読むときに FTAN にしている
            van: fop(ops, "FTAN", i as i64 + 1, vd[4][i]),
        })
        .collect();
    let norm = if ftyp.get(4) == Some(&1) {
        FieldNorm::Rect
    } else {
        FieldNorm::Radial
    };
    Ok(Fields {
        tele,
        typ,
        norm,
        list,
    })
}

/// そのほかの系の設定
pub(super) struct Settings {
    pub aiming: Aiming,
    pub glrs: usize,
    pub ropd: i64,
    pub coating_file: Option<PathBuf>,
    pub name: String,
}

pub(super) fn settings(raw: &Raw, dirs: &[PathBuf], base: Option<&Path>) -> Result<Settings> {
    let aiming = match sys_f(raw, "RAIM", 1, 0.0)? as i64 {
        0 => Aiming::Off,
        1 => Aiming::Paraxial,
        2 => Aiming::Real,
        r => notimpl!("ray aiming {r}"),
    };
    let glrs = count_in(
        sys_f(raw, "GLRS", 0, 1.0)?,
        "zmx GLRS",
        0,
        raw.surfs.len().max(1),
    )?;
    let ropd = sys_f(raw, "ROPD", 0, 2.0)? as i64;
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
            .find(|p| lensbench_core::vfs::exists(p))
    });
    let name = raw
        .sysl
        .get("NAME")
        .and_then(|v| v.first())
        .map(|v| v.join(" "))
        .unwrap_or_default();
    Ok(Settings {
        aiming,
        glrs,
        ropd,
        coating_file,
        name,
    })
}

pub(super) fn aperture_type(raw: &Raw) -> Result<i64> {
    if raw.sysl.contains_key("FLOA") {
        return Ok(3);
    }
    if raw.sysl.contains_key("FNUM") {
        return Ok(if sys_f(raw, "FNUM", 1, 0.0)? as i64 == 1 {
            4
        } else {
            1
        });
    }
    if raw.sysl.contains_key("OBNA") {
        return Ok(if sys_f(raw, "OBNA", 1, 0.0)? as i64 == 1 {
            5
        } else {
            2
        });
    }
    Ok(0)
}

fn aperture_value(raw: &Raw, satp: i64) -> Result<f64> {
    Ok(match satp {
        0 => sys_f(raw, "ENPD", 0, 0.0)?,
        1 | 4 => sys_f(raw, "FNUM", 0, 0.0)?,
        2 | 5 => sys_f(raw, "OBNA", 0, 0.0)?,
        _ => 0.0,
    })
}
