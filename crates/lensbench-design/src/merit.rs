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
//! | `paraxial` | q（efl・pmag・enpp・epd・expp・expd・wfno・pwfn・isfn・pimh・totr・obna・isna）、target |
//! | `centroid` | field、wave（null は多色）、rings（3）、q（x・y）、target [レンズ単位]。像面の光束の重心（スポットの項と同じ光線・重み） |
//! | `ray` | field、wave（null は主波長）、px・py（正規化瞳座標、0）、surf（像面）、q（x・y・z・l・m・n）、target |
//! | `opd` | field、wave、px・py、target [波] |
//! | `zernike` | field、wave、term（1 始まり）、kind（"fringe"・"standard"）、n（格子、64）、target [波] |
//! | `coupling` | wave、na（送り・受けとも）か na_source・na_receiver、source・receiver [x, y]、grid（101）、na_max、clip_stop、target |
//! | `zplm` | macro（番号 nn なら ZPLnn.ZPL、か名前）か file（.ZPL のパス）、data（OPTRETURN の番号、0）、hx・hy・px・py（PVHX…、0）、macro_dirs（探すフォルダの並び）、target |
//! | `plugin` | name（登録の名前）、args（設定 {"名前": 値}）、inputs（入力 {"名前": 数か数の並び}）か field（inputs の field の略）、output（出力の名前）、index（その要素だけ）、target（数か出力と同じ数の並び、0） |
//!
//! `zplm` のマクロを探す順: file、macro_dirs、系に読み込んだマクロ（.zmx のソルブのもの）、LENSBENCH_MACROS、今のフォルダ。
//! `"default_dirs": false` なら最後の 2 つ（既定の場所）を探さない（serve が --root・--allow-remote のときに付ける）。
//!
//! 種類ごとのキーは [`crate::term_spec`] の型で読み、知らないキーは誤り。スキーマは `docs/schema/terms.schema.json`
//! （正式な綴りだけ。読みが受ける別名 `wfe`・dir の `t`・`s`・大文字の `type` はスキーマには無い）。

use lensbench_core::analysis::ZernKind;
use lensbench_core::fiber::FiberOpts;
use lensbench_core::sweep::RmsRef;
use lensbench_core::{FieldSel, System};
use lensbench_optim::zplm::ZplmTerm;
use lensbench_optim::{MtfDir, ParaxQ, RayQ, Side, Term, TermKind};
use serde_json::Value;

use crate::term_spec::{
    DirName, FieldRef, MacroRef, Nums, RefName, TermSpec, Xy, ZernName, ZplmSpec,
};

/// 項の並び（配列か 1 つのオブジェクト）
pub fn terms_from_json(v: &Value, configs: &[System]) -> Result<Vec<Term>, String> {
    match v {
        Value::Array(a) => a
            .iter()
            .enumerate()
            .map(|(i, t)| term_from_json(t, configs).map_err(|e| format!("term {}: {e}", i + 1)))
            .collect(),
        _ => Ok(vec![term_from_json(v, configs)?]),
    }
}

fn field(f: FieldRef, s: &System) -> Result<FieldSel, String> {
    match f {
        FieldRef::Norm([x, y]) => Ok(FieldSel::Norm(x, y)),
        FieldRef::Index(i) if i >= 1 && i <= s.fields.len() => Ok(FieldSel::Index(i - 1)),
        FieldRef::Index(i) => Err(format!("no field {i} (1 to {})", s.fields.len())),
    }
}

fn wave(w: Option<usize>, s: &System) -> Result<Option<usize>, String> {
    lensbench_core::waves::index_opt(w, s.wavelengths.len()).map_err(String::from)
}

fn reference(r: Option<RefName>) -> RmsRef {
    match r.unwrap_or_default() {
        RefName::Centroid => RmsRef::Centroid,
        RefName::Chief => RmsRef::Chief,
    }
}

fn num(k: &str, v: &Value) -> Result<f64, String> {
    v.as_f64()
        .ok_or_else(|| format!("{k} must be a number ({v})"))
}

/// 1 つの項。共通のキー（weight・config・min・max）を外し、残りを [`TermSpec`] で読む
pub fn term_from_json(v: &Value, configs: &[System]) -> Result<Term, String> {
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
        Some(Ok(c)) => return Err(format!("no configuration {c} (1 to {})", configs.len())),
        Some(Err(c)) => return Err(format!("config must be an integer 1 or more ({c})")),
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
    const TYPES: [&str; 12] = [
        "spot",
        "wavefront",
        "wfe",
        "mtf",
        "paraxial",
        "ray",
        "centroid",
        "opd",
        "zernike",
        "coupling",
        "zplm",
        "plugin",
    ];
    if !TYPES.contains(&ty.as_str()) {
        return Err(format!("type {ty}（{}）", TYPES.join("・")));
    }
    if side != Side::Eq && ["spot", "wavefront", "wfe", "zplm", "plugin"].contains(&ty.as_str()) {
        return Err(format!(
            "{ty} terms can't use min or max (only terms with a single target value)"
        ));
    }
    let spec: TermSpec =
        serde_json::from_value(Value::Object(m)).map_err(|e| format!("{ty} term: {e}"))?;
    let s = configs.get(config).ok_or("no configurations")?;
    let kind = kind_of(spec, s)?;
    let single = matches!(
        kind,
        TermKind::Mtf {
            dir: MtfDir::Tan | MtfDir::Sag,
            ..
        } | TermKind::Paraxial { .. }
            | TermKind::Ray { .. }
            | TermKind::Centroid { .. }
            | TermKind::Opd { .. }
            | TermKind::Zernike { .. }
            | TermKind::Coupling { .. }
    );
    if side != Side::Eq && !single {
        return Err(format!(
            "{ty} terms can't use min or max (only terms with a single target value)"
        ));
    }
    Ok(Term {
        side,
        config,
        weight,
        kind,
    })
}

/// 型にした項を core の項にする（番号の確かめ・既定値・マクロとプラグインの準備）
fn kind_of(spec: TermSpec, s: &System) -> Result<TermKind, String> {
    let user = |t: std::sync::Arc<dyn lensbench_optim::UserTerm>| {
        TermKind::User(lensbench_optim::UserTermRef(t))
    };
    Ok(match spec {
        TermSpec::Spot(r) => TermKind::Spot {
            field: field(r.field, s)?,
            wave: wave(r.wave, s)?,
            rings: r.rings.unwrap_or(3),
            reference: reference(r.reference),
        },
        TermSpec::Wavefront(r) => TermKind::Wavefront {
            field: field(r.field, s)?,
            wave: wave(r.wave, s)?,
            rings: r.rings.unwrap_or(3),
            reference: reference(r.reference),
        },
        TermSpec::Mtf(t) => TermKind::Mtf {
            field: field(t.field, s)?,
            wave: wave(t.wave, s)?,
            n: t.n,
            freq: t.freq,
            dir: match t.dir.unwrap_or_default() {
                DirName::Tan => MtfDir::Tan,
                DirName::Sag => MtfDir::Sag,
                DirName::Both => MtfDir::Both,
            },
            target: t.target.unwrap_or(1.0),
        },
        TermSpec::Paraxial(t) => TermKind::Paraxial {
            q: ParaxQ::parse(&t.q).map_err(|e| e.to_string())?,
            target: t
                .target
                .ok_or("paraxial term has no target (or min and max)")?,
        },
        TermSpec::Ray(t) => TermKind::Ray {
            field: field(t.field, s)?,
            wave: wave(t.wave, s)?,
            px: t.px.unwrap_or(0.0),
            py: t.py.unwrap_or(0.0),
            surf: t.surf,
            q: RayQ::parse(&t.q).map_err(|e| e.to_string())?,
            target: t.target.unwrap_or(0.0),
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
        TermSpec::Zplm(t) => {
            let m = zplm_macro(&t, s)?;
            let pv = [t.hx, t.hy, t.px, t.py].map(|x| x.unwrap_or(0.0));
            let data = t.data.unwrap_or(0);
            let z =
                ZplmTerm::new(&m, data, pv, t.target.unwrap_or(0.0)).map_err(|e| e.to_string())?;
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
            )
            .map_err(|e| e.to_string())?;
            user(std::sync::Arc::new(p))
        }
    })
}

/// zplm の項のマクロ（macro か file）を探して読む
fn zplm_macro(t: &ZplmSpec, s: &System) -> Result<lensbench_core::zpl_solve::Macro, String> {
    use lensbench_core::zpl_solve::{find_macro, macro_dirs, Macro};
    use std::path::{Path, PathBuf};
    if let Some(f) = &t.file {
        if t.macro_.is_some() {
            return Err("only one of macro and file".into());
        }
        let p = Path::new(f);
        let src = lensbench_core::vfs::read(p).map_err(|e| format!("{f}: {e}"))?;
        let name = p
            .file_stem()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut m =
            Macro::from_source(&name, &String::from_utf8_lossy(&src)).map_err(|e| e.to_string())?;
        m.path = p.to_path_buf();
        return Ok(m);
    }
    let name = match &t.macro_ {
        None => return Err("no macro (number or name) or file".into()),
        Some(MacroRef::Name(n)) => n.clone(),
        Some(MacroRef::No(n)) if *n <= 99 => format!("ZPL{n:02}"),
        Some(MacroRef::No(n)) => {
            return Err(format!("macro must be a number 0 to 99 or a name ({n})"))
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
        return find_macro(&name, &extra).map_err(|e| e.to_string());
    }
    let mut dirs = macro_dirs(&extra, None);
    dirs.push(PathBuf::from("."));
    find_macro(&name, &dirs).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys() -> System {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        lensbench_core::zmx::load_with(
            &root.join("tests/refs/A0/system.zmx"),
            &lensbench_core::zmx::LoadOptions {
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
        ] {
            let v: Value = serde_json::from_str(bad).unwrap();
            assert!(
                term_from_json(&v, std::slice::from_ref(&s)).is_err(),
                "{bad}"
            );
        }
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
