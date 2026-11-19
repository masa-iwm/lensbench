//! 評価関数の項の JSON 表現。CLI の `optimize --term`・Python の `optimize(terms=)` で使う。
//!
//! 1 つの項は {"type": 種類, ...} のオブジェクト。共通のキー: `weight`（既定 1）、`config`（構成の番号、1 始まり、既定 1）。
//! 視野 `field` は番号（1 始まり）か正規化視野 [hx, hy]、波長 `wave` は番号（1 始まり）か null、面 `surf` は面の番号
//! （物体 0）。目標 `target` の既定は 0（MTF・結合効率は 1）。目標が 1 つの値の項（mtf の tan・sag、paraxial・ray・centroid・opd・
//! zernike・coupling）は `target` の代わりに `min`（値 ≥ min のときは残差 0）か `max`（値 ≤ max のときは残差 0）を書ける。
//!
//! | type | キー |
//! |---|---|
//! | `spot`・`wavefront` | field、wave（null は多色）、rings（3）、reference（"centroid"・"chief"） |
//! | `mtf` | field、wave、freq [cycles/mm]、dir（"tan"・"sag"・"both"）、n（瞳の標本数）、target |
//! | `paraxial` | q（efl・pmag・enpp・epd・expp・expd・wfno・pwfn・isfn・pimh・totr・obsn・isna）、wave（efl・pimh だけ、null は主波長）、target |
//! | `centroid` | field、wave（null は多色）、rings（3）、q（x・y）、target [レンズ単位]。像面の光束の重心（スポットの項と同じ光線・重み） |
//! | `ray` | field、wave（null は主波長）、px・py（正規化瞳座標、0）、surf（像面）、q（x・y・z・l・m・n、主波長の主光線からのずれ trax・tray・trar、光路長 opth）、target |
//! | `opd` | field、wave、px・py、target [波] |
//! | `surface` | surf、q（curv・conic・sag・sagx・sagy・thick・diam・mech_diam・gx・gy・gz）、x・y（sag・thick の点 [レンズ単位]、0）、target |
//! | `cardinal` | q（efl・f_object・f_image・focal_*・principal_*・anti_principal_*・nodal_*・anti_nodal_*、* は object・image）、from・to（面の範囲、面 1 から像面の手前の面まで）、wave、target [レンズ単位] |
//! | `fnum` | field、wave、dir（"tan"（既定）・"sag"）、target。実光線の F ナンバー 1/(n′(sin θ₊ + sin θ₋))（主光線と瞳の縁の 2 本の角度） |
//! | `seidel` | q（spha・coma・asti・fcur・dist）、surf（0 は和、0）、wave、target [波] |
//! | `distortion` | wave、absolute（true で実像高 − 近軸像高 [レンズ単位]、false で %）、target。最大の視野の主光線 |
//! | `longitudinal` | wave、wave2（差をとる波長）、zone（0〜1、0 は近軸）、target [レンズ単位]。軸上の光線が光軸を横切る点の像面からの距離 |
//! | `lateral_color` | wave、wave2、target [レンズ単位]。最大の視野の近軸主光線の像面での高さの差（wave2 − wave） |
//! | `semi` | surf、to（surf）、min か max。面ごとに半直径と比べ、はみ出しを足す（MNSD・MXSD） |
//! | `zernike` | field、wave、term（1 始まり）、kind（"fringe"・"standard"）、n（格子、64）、target [波] |
//! | `coupling` | wave、na（送り・受けとも）か na_source・na_receiver、source・receiver [x, y]、grid（101）、na_max、clip_stop、target |
//! | `zplm` | macro（番号 nn なら ZPLnn.ZPL、か名前）か file（.ZPL のパス）、data（OPTRETURN の番号、0）、hx・hy・px・py（PVHX…、0）、macro_dirs（探すフォルダの並び）、target |
//! | `plugin` | name（登録の名前）、args（設定 {"名前": 値}）、inputs（入力 {"名前": 数か数の並び}）か field（inputs の field の略）、output（出力の名前）、index（その要素だけ）、target（数か出力と同じ数の並び、0） |
//!
//! `zplm` のマクロを探す順: file、macro_dirs、系に読み込んだマクロ（.zmx のソルブのもの）、LENSBENCH_MACROS、今のフォルダ。
//! `"default_dirs": false` なら最後の 2 つ（既定の場所）を探さない（serve が --root・--allow-remote のときに付ける）。
//!
//! 種類ごとのキーは [`crate::term_spec`] の型で読み、知らないキーは誤り。スキーマは `docs/schema/terms.schema.json`
//! （正式な綴りだけ。読みが受ける大文字の `type` はスキーマには無い）。

use lensbench_analysis::analysis::ZernKind;
use lensbench_analysis::fiber::FiberOpts;
use lensbench_analysis::sweep::RmsRef;
use lensbench_core::seidel::DistComp;
use lensbench_core::{FieldSel, System};
use lensbench_optim::zplm::ZplmTerm;
use lensbench_optim::{
    CalcArg, CalcOp, CardQ, DistMode, Each, EnergyMethod, EnergyQ, Medium, MtfDir, MtfKind, ParaxQ,
    Quad, RayQ, SeidelQ, Side, SpotAxis, SurfQ, Term, TermKind,
};
use serde_json::Value;

use crate::term_spec::{
    AxisName, CalcArgSpec, CalcName, DirName, EachName, FieldRef, MacroRef, MtfKindName, Nums,
    QuadName, RefName, RmsSpec, TermSpec, Xy, ZernName, ZplmSpec,
};

/// 項の並び（配列か 1 つのオブジェクト）
pub fn terms_from_json(v: &Value, configs: &[System]) -> lensbench_core::Result<Vec<Term>> {
    match v {
        Value::Array(a) => a
            .iter()
            .enumerate()
            .map(|(i, t)| {
                term_from_json(t, configs)
                    .map_err(|e| e.map_message(|e| format!("term {}: {e}", i + 1)))
            })
            .collect(),
        _ => Ok(vec![term_from_json(v, configs)?]),
    }
}

fn field(f: FieldRef, s: &System) -> lensbench_core::Result<FieldSel> {
    match f {
        FieldRef::Norm([x, y]) => Ok(FieldSel::Norm(x, y)),
        FieldRef::Index(i) if i >= 1 && i <= s.fields.len() => Ok(FieldSel::Index(i - 1)),
        FieldRef::Index(i) => Err(format!("no field {i} (1 to {})", s.fields.len()).into()),
    }
}

fn wave(w: Option<usize>, s: &System) -> lensbench_core::Result<Option<usize>> {
    Ok(lensbench_core::waves::index_opt(w, s.wavelengths.len()).map_err(String::from)?)
}

fn reference(r: Option<RefName>) -> RmsRef {
    match r.unwrap_or_default() {
        RefName::Centroid => RmsRef::Centroid,
        RefName::Chief => RmsRef::Chief,
    }
}

fn quad(r: &RmsSpec) -> lensbench_core::Result<Quad> {
    let q = r.quadrature.unwrap_or_default();
    let pos = |k: &str, v: usize| -> lensbench_core::Result<usize> {
        if v == 0 {
            return Err(format!("{k} must be 1 or more").into());
        }
        Ok(v)
    };
    if r.arms.is_some() && !matches!(q, QuadName::Opticstudio) {
        return Err("arms goes with quadrature opticstudio".to_string().into());
    }
    if r.n.is_some() && !matches!(q, QuadName::Grid) {
        return Err("n goes with quadrature grid".to_string().into());
    }
    Ok(match (q, r.arms, r.n) {
        (QuadName::Lensbench, ..) => Quad::Lensbench,
        (QuadName::Opticstudio, None, _) => Quad::OpticStudio,
        (QuadName::Opticstudio, Some(a), _) => Quad::Arms(pos("arms", a)?),
        (QuadName::Grid, _, Some(n)) => Quad::Grid(pos("n", n)?),
        (QuadName::Grid, _, None) => return Err("quadrature grid needs n".into()),
    })
}

/// calc の項（引数の数と、引数の項が 1 つの値の項かを確かめる）
fn calc_kind(c: crate::term_spec::CalcSpec, s: &System) -> lensbench_core::Result<TermKind> {
    let k = TermKind::Calc {
        op: match c.op {
            CalcName::Sum => CalcOp::Sum,
            CalcName::Diff => CalcOp::Diff,
            CalcName::Prod => CalcOp::Prod,
            CalcName::Div => CalcOp::Div,
            CalcName::Abs => CalcOp::Abs,
            CalcName::Sqrt => CalcOp::Sqrt,
            CalcName::Reci => CalcOp::Reci,
            CalcName::Max => CalcOp::Max,
            CalcName::Min => CalcOp::Min,
            CalcName::Qsum => CalcOp::Qsum,
            CalcName::Sin => CalcOp::Sin,
            CalcName::Cos => CalcOp::Cos,
            CalcName::Tan => CalcOp::Tan,
            CalcName::Asin => CalcOp::Asin,
            CalcName::Acos => CalcOp::Acos,
            CalcName::Atan => CalcOp::Atan,
        },
        args: c
            .args
            .into_iter()
            .map(|a| match a {
                CalcArgSpec::Num(x) => Ok(CalcArg::Const(x)),
                CalcArgSpec::Term(t) => Ok(CalcArg::Term(Box::new(kind_of(*t, s)?))),
            })
            .collect::<lensbench_core::Result<_>>()?,
        target: c.target.unwrap_or(0.0),
    };
    if let TermKind::Calc { op, args, .. } = &k {
        let (lo, hi) = op.arity();
        if args.len() < lo || args.len() > hi {
            let (op, n) = (op.name(), args.len());
            if lo == hi {
                return Err(format!("the {op} calc term takes {lo} arguments ({n})").into());
            }
            return Err(format!("the {op} calc term takes {lo} or more arguments ({n})").into());
        }
        if args
            .iter()
            .any(|a| matches!(a, CalcArg::Term(t) if t.target().is_none()))
        {
            return Err("an argument of a calc term must be a term with a single value".into());
        }
    }
    Ok(k)
}

fn num(k: &str, v: &Value) -> lensbench_core::Result<f64> {
    Ok(v.as_f64()
        .ok_or_else(|| format!("{k} must be a number ({v})"))?)
}

/// 1 つの項。共通のキー（weight・config・min・max）を外し、残りを [`TermSpec`] で読む
pub fn term_from_json(v: &Value, configs: &[System]) -> lensbench_core::Result<Term> {
    let mut m = v
        .as_object()
        .ok_or_else(|| format!("a term is an object {{\"type\": ...}} ({v})"))?
        .clone();
    m.retain(|_, x| !x.is_null());
    // 片側の条件: "min"（値 ≥ min）・"max"（値 ≤ max）は target の代わりに書く
    let mut side = Side::Eq;
    for (k, sd) in [("min", Side::Min), ("max", Side::Max)] {
        if let Some(b) = m.remove(k) {
            if side != Side::Eq || m.contains_key("target") {
                return Err("exactly one of target, min and max".into());
            }
            m.insert("target".into(), b);
            side = sd;
        }
    }
    let weight = m.remove("weight").map_or(Ok(1.0), |x| num("weight", &x))?;
    let config = match m.remove("config").map(|c| c.as_u64().ok_or(c)) {
        None => 0,
        Some(Ok(c)) if c >= 1 && c as usize <= configs.len() => c as usize - 1,
        Some(Ok(c)) => return Err(format!("no configuration {c} (1 to {})", configs.len()).into()),
        Some(Err(c)) => return Err(format!("config must be an integer 1 or more ({c})").into()),
    };
    if let Some(Value::String(t)) = m.get_mut("type") {
        *t = t.to_ascii_lowercase();
    }
    let ty = m
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    if ty.is_empty() {
        return Err("no type".into());
    }
    const TYPES: [&str; 25] = [
        "spot",
        "wavefront",
        "mtf",
        "paraxial",
        "ray",
        "centroid",
        "opd",
        "zernike",
        "coupling",
        "thickness",
        "surface",
        "volume",
        "cardinal",
        "fnum",
        "seidel",
        "distortion",
        "energy",
        "strehl",
        "illum",
        "longitudinal",
        "lateral_color",
        "semi",
        "zplm",
        "plugin",
        "calc",
    ];
    if !TYPES.contains(&ty.as_str()) {
        return Err(format!("type {ty} (one of {})", TYPES.join(", ")).into());
    }
    if side != Side::Eq && ["spot", "wavefront", "zplm", "plugin"].contains(&ty.as_str()) {
        return Err(format!(
            "{ty} terms can't use min or max (only terms with a single target value)"
        )
        .into());
    }
    let spec: TermSpec =
        serde_json::from_value(Value::Object(m)).map_err(|e| format!("{ty} term: {e}"))?;
    let s = configs.get(config).ok_or("no configurations")?;
    let kind = kind_of(spec, s)?;
    let single = matches!(
        kind,
        TermKind::Mtf {
            dir: MtfDir::Tan | MtfDir::Sag | MtfDir::Avg,
            ..
        } | TermKind::Paraxial { .. }
            | TermKind::Ray { .. }
            | TermKind::RayPath { .. }
            | TermKind::ParaxRay { .. }
            | TermKind::Centroid { .. }
            | TermKind::Opd { .. }
            | TermKind::Zernike { .. }
            | TermKind::Coupling { .. }
            | TermKind::Thickness { .. }
            | TermKind::Surface { .. }
            | TermKind::Volume { .. }
            | TermKind::Cardinal { .. }
            | TermKind::FNum { .. }
            | TermKind::Seidel { .. }
            | TermKind::Distortion { .. }
            | TermKind::Energy { .. }
            | TermKind::Strehl { .. }
            | TermKind::Illum { .. }
            | TermKind::Longitudinal { .. }
            | TermKind::LateralColor { .. }
            | TermKind::SurfEach { .. }
            | TermKind::Calc { .. }
    );
    // 面ごとに比べる厚みの項は min・max の向きで残差を決める
    let mut kind = kind;
    if let TermKind::Thickness { each: Some(e), .. } = &mut kind {
        if side == Side::Eq {
            return Err("a thickness term with each needs min or max".into());
        }
        e.upper = side == Side::Max;
    }
    if let TermKind::SurfEach { upper, .. } = &mut kind {
        if side == Side::Eq {
            return Err("a semi term or a surface term with to needs min or max".into());
        }
        *upper = side == Side::Max;
    }
    // 面ごとの入射角・射出角は min・max の向きで比べる
    if let TermKind::RayPath {
        q: RayQ::Aoi | RayQ::Aoe,
        upper,
        ..
    } = &mut kind
    {
        if side == Side::Eq {
            return Err("a ray term with to and aoi or aoe needs min or max".into());
        }
        *upper = Some(side == Side::Max);
    }
    if side != Side::Eq && !single {
        return Err(format!(
            "{ty} terms can't use min or max (only terms with a single target value)"
        )
        .into());
    }
    Ok(Term {
        side,
        config,
        weight,
        kind,
    })
}

/// 型にした項を core の項にする（番号の確かめ・既定値・マクロとプラグインの準備）
fn kind_of(spec: TermSpec, s: &System) -> lensbench_core::Result<TermKind> {
    let user = |t: std::sync::Arc<dyn lensbench_optim::UserTerm>| {
        TermKind::User(lensbench_optim::UserTermRef(t))
    };
    Ok(match spec {
        TermSpec::Spot(r) => TermKind::Spot {
            field: field(r.field, s)?,
            wave: wave(r.wave, s)?,
            rings: r.rings.unwrap_or(3),
            reference: reference(r.reference),
            quad: quad(&r)?,
            axis: match r.axis.unwrap_or_default() {
                AxisName::Xy => SpotAxis::Both,
                AxisName::X => SpotAxis::X,
                AxisName::Y => SpotAxis::Y,
            },
        },
        TermSpec::Wavefront(r) => {
            if r.axis.is_some() {
                return Err("axis goes with spot".to_string().into());
            }
            TermKind::Wavefront {
                field: field(r.field, s)?,
                wave: wave(r.wave, s)?,
                rings: r.rings.unwrap_or(3),
                reference: reference(r.reference),
                quad: quad(&r)?,
            }
        }
        TermSpec::Mtf(t) => TermKind::Mtf {
            field: field(t.field, s)?,
            wave: wave(t.wave, s)?,
            n: t.n,
            freq: t.freq,
            dir: match t.dir.unwrap_or_default() {
                DirName::Tan => MtfDir::Tan,
                DirName::Sag => MtfDir::Sag,
                DirName::Both => MtfDir::Both,
                DirName::Avg => MtfDir::Avg,
            },
            kind: match (t.kind.unwrap_or_default(), t.diff_limit) {
                (MtfKindName::Geometric, d) => MtfKind::Geometric {
                    diff_limit: d.unwrap_or(true),
                },
                (_, Some(_)) => {
                    return Err("diff_limit of an mtf term goes with kind geometric"
                        .to_string()
                        .into())
                }
                (MtfKindName::Diffraction, None) => MtfKind::Diffraction,
                (MtfKindName::Square, None) => MtfKind::SquareWave,
                (MtfKindName::Huygens, None) => MtfKind::Huygens,
            },
            target: t.target.unwrap_or(1.0),
        },
        TermSpec::Paraxial(t) => TermKind::Paraxial {
            q: ParaxQ::parse(&t.q)?,
            wave: wave(t.wave, s)?,
            target: t
                .target
                .ok_or("paraxial term has no target (or min and max)")?,
        },
        TermSpec::Ray(t) if t.paraxial == Some(true) => {
            if t.to.is_some() {
                return Err("to does not go with paraxial".to_string().into());
            }
            TermKind::ParaxRay {
                field: field(t.field, s)?,
                wave: wave(t.wave, s)?,
                px: t.px.unwrap_or(0.0),
                py: t.py.unwrap_or(0.0),
                surf: t.surf.unwrap_or(s.image()),
                q: RayQ::parse(&t.q)?,
                target: t.target.unwrap_or(0.0),
            }
        }
        TermSpec::Ray(t) => match t.to {
            Some(to) => TermKind::RayPath {
                field: field(t.field, s)?,
                wave: wave(t.wave, s)?,
                px: t.px.unwrap_or(0.0),
                py: t.py.unwrap_or(0.0),
                surf: t.surf.ok_or("to goes with surf")?,
                to,
                q: RayQ::parse(&t.q)?,
                upper: None,
                target: t.target.unwrap_or(0.0),
            },
            None => TermKind::Ray {
                field: field(t.field, s)?,
                wave: wave(t.wave, s)?,
                px: t.px.unwrap_or(0.0),
                py: t.py.unwrap_or(0.0),
                surf: t.surf,
                q: RayQ::parse(&t.q)?,
                target: t.target.unwrap_or(0.0),
            },
        },
        TermSpec::Centroid(t) => TermKind::Centroid {
            field: field(t.field, s)?,
            wave: wave(t.wave, s)?,
            rings: t.rings.unwrap_or(3),
            y: matches!(t.q, Xy::Y),
            target: t.target.unwrap_or(0.0),
        },
        TermSpec::Opd(t) => TermKind::Opd {
            field: field(t.field, s)?,
            wave: wave(t.wave, s)?,
            px: t.px.unwrap_or(0.0),
            py: t.py.unwrap_or(0.0),
            target: t.target.unwrap_or(0.0),
        },
        TermSpec::Zernike(t) => TermKind::Zernike {
            field: field(t.field, s)?,
            wave: wave(t.wave, s)?,
            kind: match t.kind.unwrap_or_default() {
                ZernName::Fringe => ZernKind::Fringe,
                ZernName::Standard => ZernKind::Standard,
            },
            term: t.term,
            fit: t.fit,
            n: t.n.unwrap_or(64),
            target: t.target.unwrap_or(0.0),
        },
        TermSpec::Coupling(t) => {
            let d = FiberOpts::default();
            let opts = FiberOpts {
                na_source: t.na_source.or(t.na).unwrap_or(d.na_source),
                na_receiver: t.na_receiver.or(t.na).unwrap_or(d.na_receiver),
                source: t.source.unwrap_or(d.source),
                receiver: t.receiver.unwrap_or(d.receiver),
                grid: t.grid.unwrap_or(d.grid),
                na_max: t.na_max,
                clip_stop: t.clip_stop.unwrap_or(d.clip_stop),
            };
            TermKind::Coupling {
                wave: wave(t.wave, s)?,
                opts,
                target: t.target.unwrap_or(1.0),
            }
        }
        TermSpec::Thickness(t) => {
            let zone = t.zone.unwrap_or(1.0);
            if t.zone.is_some() && t.edge != Some(true) {
                return Err("zone goes with edge".to_string().into());
            }
            if !(zone > 0.0 && zone <= 1.0) {
                return Err(format!("zone {zone} (more than 0, up to 1)").into());
            }
            TermKind::Thickness {
                surf: t.surf,
                to: t.to.unwrap_or(t.surf),
                target: t.target.unwrap_or(0.0),
                edge: (t.edge == Some(true)).then_some(zone),
                each: t.each.map(|e| Each {
                    medium: match e {
                        EachName::All => Medium::All,
                        EachName::Air => Medium::Air,
                        EachName::Glass => Medium::Glass,
                    },
                    upper: false,
                }),
            }
        }
        TermSpec::Surface(t) => {
            let mut q = SurfQ::parse(&t.q)?;
            if (t.x.is_some() || t.y.is_some())
                && !matches!(q, SurfQ::Sag | SurfQ::Thick | SurfQ::Total)
            {
                return Err("x and y go with sag, thick and total".to_string().into());
            }
            match (q, t.param) {
                (SurfQ::Parm(_), Some(n)) => q = SurfQ::Parm(n),
                (SurfQ::Parm(_), None) => {
                    return Err("q param needs param (the parameter number)"
                        .to_string()
                        .into())
                }
                (_, Some(_)) => return Err("param goes with q param".to_string().into()),
                _ => {}
            }
            if let Some(to) = t.to {
                if t.wave.is_some() || t.x.is_some() || t.y.is_some() {
                    return Err("a surface term with to has no wave, x or y"
                        .to_string()
                        .into());
                }
                return Ok(TermKind::SurfEach {
                    q,
                    surf: t.surf,
                    to,
                    upper: false,
                    target: t.target.ok_or("surface term with to has no min or max")?,
                });
            }
            if q == SurfQ::Semi {
                return Err("q semi goes with to (or use the semi term)"
                    .to_string()
                    .into());
            }
            TermKind::Surface {
                surf: t.surf,
                q,
                x: t.x.unwrap_or(0.0),
                y: t.y.unwrap_or(0.0),
                wave: wave(t.wave, s)?,
                target: t
                    .target
                    .ok_or("surface term has no target (or min and max)")?,
            }
        }
        TermSpec::Volume(t) => TermKind::Volume {
            surf: t.surf,
            to: t.to.unwrap_or(t.surf),
            mass: t.mass.unwrap_or(false),
            clear: t.clear.unwrap_or(false),
            target: t
                .target
                .ok_or("volume term has no target (or min and max)")?,
        },
        TermSpec::Cardinal(t) => {
            let last = (1..s.image())
                .rev()
                .find(|&j| !s.surfaces[j].ignore)
                .unwrap_or(1);
            TermKind::Cardinal {
                from: t.from.unwrap_or(1),
                to: t.to.unwrap_or(last),
                q: CardQ::parse(&t.q)?,
                wave: wave(t.wave, s)?,
                target: t
                    .target
                    .ok_or("cardinal term has no target (or min and max)")?,
            }
        }
        TermSpec::Fnum(t) => TermKind::FNum {
            field: field(t.field, s)?,
            wave: wave(t.wave, s)?,
            sag: match t.dir {
                None | Some(DirName::Tan) => false,
                Some(DirName::Sag) => true,
                Some(DirName::Both | DirName::Avg) => {
                    return Err("dir of an fnum term is tan or sag".to_string().into())
                }
            },
            target: t.target.ok_or("fnum term has no target (or min and max)")?,
        },
        TermSpec::Seidel(t) => TermKind::Seidel {
            surf: t.surf.unwrap_or(0),
            q: SeidelQ::parse(&t.q)?,
            wave: wave(t.wave, s)?,
            target: t
                .target
                .ok_or("seidel term has no target (or min and max)")?,
        },
        TermSpec::Energy(t) => {
            use lensbench_analysis::energy::{EnergyCenter, EnergyShape};
            let airy = t.airy.unwrap_or(false);
            let method = match t.method.as_deref().unwrap_or("geometric") {
                "geometric" => EnergyMethod::Geometric { airy },
                m @ ("fft" | "huygens") => {
                    if airy {
                        return Err("airy of an energy term goes with method geometric"
                            .to_string()
                            .into());
                    }
                    if m == "fft" {
                        EnergyMethod::Fft
                    } else {
                        EnergyMethod::Huygens
                    }
                }
                m => return Err(format!("energy method {m} (geometric, fft or huygens)").into()),
            };
            let shape = match t.shape.as_deref().unwrap_or("circle") {
                "circle" => EnergyShape::Circle,
                "x" => EnergyShape::X,
                "y" => EnergyShape::Y,
                "square" => EnergyShape::Square,
                v => return Err(format!("energy shape {v} (circle, x, y or square)").into()),
            };
            let center = match t.center.as_deref().unwrap_or("chief") {
                "chief" => EnergyCenter::Chief,
                "centroid" => EnergyCenter::Centroid,
                "vertex" => EnergyCenter::Vertex,
                "middle" => EnergyCenter::Middle,
                v => {
                    return Err(
                        format!("energy center {v} (chief, centroid, vertex or middle)").into(),
                    )
                }
            };
            let q = match (t.size_at, t.fraction_at) {
                (Some(f), None) => EnergyQ::Size(f),
                (None, Some(r)) => EnergyQ::Fraction(r),
                _ => {
                    return Err("an energy term needs one of size_at and fraction_at"
                        .to_string()
                        .into())
                }
            };
            TermKind::Energy {
                field: field(t.field, s)?,
                wave: wave(t.wave, s)?,
                n: t.n,
                method,
                shape,
                center,
                q,
                target: t
                    .target
                    .ok_or("energy term has no target (or min and max)")?,
            }
        }
        TermSpec::Strehl(t) => TermKind::Strehl {
            field: field(t.field, s)?,
            wave: wave(t.wave, s)?,
            n: t.n,
            target: t
                .target
                .ok_or("strehl term has no target (or min and max)")?,
        },
        TermSpec::Illum(t) => TermKind::Illum {
            field: field(t.field, s)?,
            wave: wave(t.wave, s)?,
            n: t.n,
            relative: match t.q.as_str() {
                "relative" => true,
                "fno" => false,
                q => return Err(format!("illum q {q} (relative or fno)").into()),
            },
            target: t
                .target
                .ok_or("illum term has no target (or min and max)")?,
        },
        TermSpec::Distortion(t) => TermKind::Distortion {
            wave: wave(t.wave, s)?,
            absolute: t.absolute.unwrap_or(false),
            field: t.field.map(|f| field(f, s)).transpose()?,
            reference: t.reference.map(|f| field(f, s)).transpose()?,
            abcd: t.abcd,
            component: match t.component.as_deref().unwrap_or("radial") {
                "radial" => DistComp::Radial,
                "x" => DistComp::X,
                "y" => DistComp::Y,
                c => return Err(format!("distortion component {c} (radial, x, y)").into()),
            },
            mode: match (t.peak.unwrap_or(false), t.ftheta.unwrap_or(false)) {
                _ if t.coef.is_some()
                    && (t.peak == Some(true) || t.ftheta == Some(true) || t.abcd.is_some()) =>
                {
                    return Err("a distortion coefficient term has no peak, ftheta or abcd".into())
                }
                _ if t.coef.is_some() => match t.coef.as_deref().unwrap_or("") {
                    "a" => DistMode::Coef(0),
                    "b" => DistMode::Coef(1),
                    "c" => DistMode::Coef(2),
                    "d" => DistMode::Coef(3),
                    c => return Err(format!("distortion coefficient {c} (a, b, c, d)").into()),
                },
                (_, _) if t.abcd.is_some() && (t.peak == Some(true) || t.ftheta == Some(true)) => {
                    return Err("a distortion term with abcd has no peak or ftheta".into())
                }
                (false, false) => DistMode::At,
                (true, false) if t.reference.is_none() => DistMode::Max,
                (false, true) if t.field.is_none() && t.reference.is_none() => DistMode::FTheta,
                (true, false) => {
                    return Err("a peak distortion term is against the axis (no reference)".into())
                }
                _ => {
                    return Err("an f-theta distortion term has no field, reference or peak".into())
                }
            },
            target: t
                .target
                .ok_or("distortion term has no target (or min and max)")?,
        },
        TermSpec::Longitudinal(t) => TermKind::Longitudinal {
            wave: wave(t.wave, s)?,
            wave2: wave(t.wave2, s)?,
            zone: t.zone.unwrap_or(0.0),
            target: t
                .target
                .ok_or("longitudinal term has no target (or min and max)")?,
        },
        TermSpec::LateralColor(t) => TermKind::LateralColor {
            wave: wave(t.wave, s)?,
            wave2: wave(Some(t.wave2), s)?.unwrap_or(s.primary),
            target: t
                .target
                .ok_or("lateral_color term has no target (or min and max)")?,
        },
        TermSpec::Semi(t) => TermKind::SurfEach {
            q: SurfQ::Semi,
            surf: t.surf,
            to: t.to.unwrap_or(t.surf),
            upper: false,
            target: t.target.ok_or("semi term has no min or max")?,
        },
        TermSpec::Calc(c) => calc_kind(c, s)?,
        TermSpec::Zplm(t) => {
            let m = zplm_macro(&t, s)?;
            let pv = [t.hx, t.hy, t.px, t.py].map(|x| x.unwrap_or(0.0));
            let data = t.data.unwrap_or(0);
            let z = ZplmTerm::new(&m, data, pv, t.target.unwrap_or(0.0))?;
            user(std::sync::Arc::new(z))
        }
        TermSpec::Plugin(t) => {
            if t.name.is_empty() {
                return Err("no name (the plugin name)".into());
            }
            let args = crate::plugin_term::config_of(t.args.as_ref())?;
            let mut inputs = crate::plugin_term::arrays_of(t.inputs.as_ref())?;
            if let Some(f) = &t.field {
                if inputs.contains_key("field") {
                    return Err("only one of field and the field in inputs".into());
                }
                inputs.insert("field".into(), crate::plugin_term::array_of("field", f)?);
            }
            if t.output.is_empty() {
                return Err("no output (the output name)".into());
            }
            let target = match t.target {
                None => vec![],
                Some(Nums::One(x)) => vec![x],
                Some(Nums::Many(v)) => v,
            };
            let p = crate::plugin_term::PluginTerm::new(
                &t.name, &args, inputs, &t.output, t.index, target, s,
            )?;
            user(std::sync::Arc::new(p))
        }
    })
}

/// zplm の項のマクロ（macro か file）を探して読む
fn zplm_macro(
    t: &ZplmSpec,
    s: &System,
) -> lensbench_core::Result<lensbench_core::zpl_solve::Macro> {
    use lensbench_core::zpl_solve::{find_macro, macro_dirs, Macro};
    use std::path::{Path, PathBuf};
    if let Some(f) = &t.file {
        if t.macro_.is_some() {
            return Err("only one of macro and file".into());
        }
        let p = Path::new(f);
        let src =
            lensbench_core::vfs::read(p).map_err(lensbench_core::Error::io_at(f.to_string()))?;
        let name = p
            .file_stem()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut m = Macro::from_source(&name, &String::from_utf8_lossy(&src))?;
        m.path = p.to_path_buf();
        return Ok(m);
    }
    let name = match &t.macro_ {
        None => return Err("no macro (number or name) or file".into()),
        Some(MacroRef::Name(n)) => n.clone(),
        Some(MacroRef::No(n)) if *n <= 99 => format!("ZPL{n:02}"),
        Some(MacroRef::No(n)) => {
            return Err(format!("macro must be a number 0 to 99 or a name ({n})").into())
        }
    };
    let extra: Vec<PathBuf> = t.macro_dirs.iter().flatten().map(PathBuf::from).collect();
    if !extra.is_empty() {
        if let Ok(m) = find_macro(&name, &extra) {
            return Ok(m);
        }
    }
    if let Some(m) = s.macros.iter().find(|m| m.name.eq_ignore_ascii_case(&name)) {
        return Ok(m.clone());
    }
    if !t.default_dirs.unwrap_or(true) {
        return find_macro(&name, &extra);
    }
    let mut dirs = macro_dirs(&extra, None);
    dirs.push(PathBuf::from("."));
    find_macro(&name, &dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys() -> System {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        lensbench_formats::zmx::load_with(
            &root.join("tests/refs/A0/system.zmx"),
            &lensbench_formats::zmx::LoadOptions {
                cfg: 1,
                catalog_dirs: vec![root.join("tests/glass")],
                ..Default::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn parse_terms() {
        let s = sys();
        let v: Value = serde_json::from_str(
            r#"[
            {"type": "ray", "field": 1, "py": 1, "q": "y", "target": 0.1, "weight": 2},
            {"type": "paraxial", "q": "EFFL", "target": 50},
            {"type": "opd", "field": [0, 0.5], "wave": 1, "px": 0.5},
            {"type": "zernike", "field": 1, "term": 9},
            {"type": "spot", "field": 1, "wave": null, "reference": "chief"},
            {"type": "mtf", "field": 1, "freq": 30, "dir": "tan"}
        ]"#,
        )
        .unwrap();
        let t = terms_from_json(&v, std::slice::from_ref(&s)).unwrap();
        assert_eq!(t.len(), 6);
        assert_eq!(t[0].weight, 2.0);
        assert_eq!(
            t[0].kind,
            TermKind::Ray {
                field: FieldSel::Index(0),
                wave: None,
                px: 0.0,
                py: 1.0,
                surf: None,
                q: RayQ::Y,
                target: 0.1
            }
        );
        assert_eq!(
            t[1].kind,
            TermKind::Paraxial {
                q: ParaxQ::Efl,
                wave: None,
                target: 50.0
            }
        );
        assert!(matches!(
            t[2].kind,
            TermKind::Opd {
                field: FieldSel::Norm(_, _),
                wave: Some(0),
                ..
            }
        ));
        assert!(matches!(t[5].kind, TermKind::Mtf { target, .. } if target == 1.0));
        // 評価関数の Wizard の求積（腕・格子）と、スポットの向き
        let w = serde_json::json!({"type": "spot", "field": 1, "quadrature": "opticstudio", "arms": 6, "axis": "y"});
        assert!(matches!(
            term_from_json(&w, std::slice::from_ref(&s)).unwrap().kind,
            TermKind::Spot {
                quad: Quad::Arms(6),
                axis: SpotAxis::Y,
                ..
            }
        ));
        let w = serde_json::json!({"type": "wavefront", "field": 1, "quadrature": "grid", "n": 4});
        assert!(matches!(
            term_from_json(&w, std::slice::from_ref(&s)).unwrap().kind,
            TermKind::Wavefront {
                quad: Quad::Grid(4),
                ..
            }
        ));
        let mut p = lensbench_optim::Problem::single(s.clone());
        p.terms = t;
        p.check().unwrap();
        // 間違い
        for bad in [
            r#"{"type": "ray", "field": 1, "q": "y", "taget": 1}"#,
            r#"{"type": "ray", "field": 9, "q": "y"}"#,
            r#"{"type": "paraxial", "q": "foo", "target": 1}"#,
            r#"{"type": "nope"}"#,
            r#"{"field": 1}"#,
            r#"{"type": "spot", "field": 1, "arms": 6}"#,
            r#"{"type": "spot", "field": 1, "quadrature": "opticstudio", "arms": 0}"#,
            r#"{"type": "spot", "field": 1, "quadrature": "grid"}"#,
            r#"{"type": "spot", "field": 1, "n": 4}"#,
            r#"{"type": "wavefront", "field": 1, "axis": "x"}"#,
            r#"{"type": "calc", "op": "diff", "args": [1]}"#,
            r#"{"type": "calc", "op": "abs", "args": [{"type": "spot", "field": 1}]}"#,
            r#"{"type": "calc", "op": "sum", "args": []}"#,
            r#"{"type": "calc", "op": "sum", "args": [{"type": "paraxial", "q": "efl", "target": 0, "weight": 2}]}"#,
            r#"{"type": "thickness", "surf": 1, "to": 3, "each": "air", "target": 1}"#,
            r#"{"type": "thickness", "surf": 1, "zone": 0.5, "target": 1}"#,
            r#"{"type": "thickness", "surf": 1, "edge": true, "zone": 0, "target": 1}"#,
            r#"{"type": "ray", "field": 1, "to": 3, "q": "opth"}"#,
            r#"{"type": "ray", "field": 1, "surf": 1, "to": 3, "paraxial": true, "q": "y"}"#,
            r#"{"type": "ray", "field": 1, "surf": 1, "to": 3, "q": "aoi", "target": 10}"#,
            r#"{"type": "cardinal", "q": "focal", "target": 0}"#,
            r#"{"type": "cardinal", "q": "efl"}"#,
            r#"{"type": "fnum", "field": 1, "dir": "both", "target": 3}"#,
            r#"{"type": "fnum", "field": 1}"#,
            r#"{"type": "seidel", "q": "petz", "target": 0}"#,
            r#"{"type": "distortion"}"#,
            r#"{"type": "longitudinal", "zone": 0.5}"#,
            r#"{"type": "lateral_color", "target": 0}"#,
            r#"{"type": "semi", "surf": 1, "target": 5}"#,
        ] {
            let v: Value = serde_json::from_str(bad).unwrap();
            assert!(
                term_from_json(&v, std::slice::from_ref(&s)).is_err(),
                "{bad}"
            );
        }
    }

    /// ミラーのある系の近軸の量（OpticStudio で確かめていない）は項の誤り
    #[test]
    fn mirror_terms_rejected() {
        let raw = lensbench_formats::zmx::parse_text(
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 1 1 0 0 0\n\
YFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV -0.01\n  DISZ -50\n  GLAS MIRROR 0 0 1.5 40 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        )
        .unwrap();
        let s = lensbench_formats::zmx::build(&raw, &Default::default(), &[], None).unwrap();
        let check = |v: Value| {
            let mut p = lensbench_optim::Problem::single(s.clone());
            p.terms = terms_from_json(&serde_json::json!([v]), std::slice::from_ref(&s)).unwrap();
            p.check()
        };
        for v in [
            serde_json::json!({"type": "paraxial", "q": "petc", "target": 0}),
            serde_json::json!({"type": "paraxial", "q": "petz", "target": 0}),
            serde_json::json!({"type": "paraxial", "q": "linv", "target": 0}),
            serde_json::json!({"type": "surface", "surf": 1, "q": "power", "target": 0}),
            serde_json::json!({"type": "surface", "surf": 2, "q": "ynip", "target": 0}),
        ] {
            assert!(check(v.clone()).is_err(), "{v}");
        }
        check(serde_json::json!({"type": "paraxial", "q": "efl", "target": 0})).unwrap();
    }

    #[test]
    fn zplm_term() {
        let s = sys();
        let d = std::env::temp_dir().join(format!("lb_zplm_{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&d).unwrap();
        lensbench_core::vfs::write(d.join("ZPL07.ZPL"), "OPTRETURN 2, THIC(1) + PVHY()\n").unwrap();
        lensbench_core::vfs::write(d.join("lbEfl.zpl"), "OPTRETURN 0, CURV(1)\n").unwrap();
        let dirs = d.to_string_lossy().into_owned();
        let v = serde_json::json!([
            {"type": "zplm", "macro": 7, "data": 2, "hy": 0.5, "target": 1.0, "macro_dirs": [dirs], "weight": 3},
            {"type": "zplm", "file": d.join("lbEfl.zpl").to_string_lossy()}
        ]);
        let t = terms_from_json(&v, std::slice::from_ref(&s)).unwrap();
        assert_eq!(t[0].weight, 3.0);
        let mut p = lensbench_optim::Problem::single(s.clone());
        p.terms = t;
        p.check().unwrap();
        let r = p.residuals(&[]).unwrap();
        let want = [3f64.sqrt() * (s.surfaces[1].t + 0.5 - 1.0), s.surfaces[1].c];
        assert!((r[0] - want[0]).abs() < 1e-14 && r[1] == want[1], "{r:?}");
        // 間違い
        for bad in [
            serde_json::json!({"type": "zplm"}),
            serde_json::json!({"type": "zplm", "macro": 100}),
            serde_json::json!({"type": "zplm", "macro": "NOPE_NOT_THERE"}),
            serde_json::json!({"type": "zplm", "macro": 7, "field": 1, "macro_dirs": [dirs]}),
            serde_json::json!({"type": "zplm", "macro": 7, "default_dirs": false}),
        ] {
            assert!(
                term_from_json(&bad, std::slice::from_ref(&s)).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn plugin_term() {
        let s = sys();
        let v = serde_json::json!([
            {"type": "plugin", "name": "rms", "field": 1, "output": "wfe", "args": {"rings": 4}},
            {"type": "plugin", "name": "rms", "inputs": {"field": [0.0, 1.0]}, "output": "wfe", "target": 1e-3, "weight": 4},
            {"type": "plugin", "name": "zernike", "field": 1, "output": "coef", "args": {"terms": 9}, "index": 3}
        ]);
        let t = terms_from_json(&v, std::slice::from_ref(&s)).unwrap();
        let mut p = lensbench_optim::Problem::single(s.clone());
        p.terms = t;
        p.check().unwrap();
        let r = p.residuals(&[]).unwrap();
        assert_eq!(r.len(), 3);
        let reg = crate::plugins::builtin();
        let ctx_sys = std::sync::Arc::new(s.clone());
        let ctx = lensbench_core::plugin::Context::with_system(&ctx_sys);
        let f1 = lensbench_core::plugin::Arrays::from([(
            "field".into(),
            lensbench_core::plugin::Array::scalar(1.0),
        )]);
        let cfg = |k: &str, x: f64| {
            lensbench_core::plugin::Config::from([(
                k.to_string(),
                lensbench_core::plugin::Value::Num(x),
            )])
        };
        let wfe = reg
            .prepare("rms", &cfg("rings", 4.0))
            .unwrap()
            .run(&ctx, &f1)
            .unwrap()["wfe"]
            .as_f64()
            .unwrap()[0];
        assert_eq!(r[0], wfe);
        let z = reg
            .prepare("zernike", &cfg("terms", 9.0))
            .unwrap()
            .run(&ctx, &f1)
            .unwrap()["coef"]
            .as_f64()
            .unwrap()[3];
        assert_eq!(r[2], z);
        let fn1 = lensbench_core::plugin::Arrays::from([(
            "field".into(),
            lensbench_core::plugin::Array::f64(vec![2], vec![0.0, 1.0]).unwrap(),
        )]);
        let a = reg
            .prepare("rms", &lensbench_core::plugin::Config::new())
            .unwrap()
            .run(&ctx, &fn1)
            .unwrap()["wfe"]
            .as_f64()
            .unwrap()[0];
        assert!((r[1] - 2.0 * (a - 1e-3)).abs() < 1e-15);
        // 間違い
        for bad in [
            serde_json::json!({"type": "plugin", "name": "nope", "field": 1, "output": "x"}),
            serde_json::json!({"type": "plugin", "name": "rms", "field": 1}),
            serde_json::json!({"type": "plugin", "name": "rms", "field": 1, "output": "nope"}),
            serde_json::json!({"type": "plugin", "name": "rms", "output": "wfe"}),
            serde_json::json!({"type": "plugin", "name": "rms", "field": 1, "output": "wfe", "target": [1, 2]}),
            serde_json::json!({"type": "plugin", "name": "rms", "field": 1, "inputs": {"field": 1}, "output": "wfe"}),
        ] {
            assert!(
                term_from_json(&bad, std::slice::from_ref(&s)).is_err(),
                "{bad}"
            );
        }
    }
}
