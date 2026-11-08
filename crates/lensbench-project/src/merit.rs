//! 評価関数の項の JSON 表現。CLI の `optimize --term`・Python の `optimize(terms=)` で使う。
//!
//! 1 つの項は {"type": 種類, ...} のオブジェクト。共通のキー: `weight`（既定 1）、`config`（構成の番号、1 始まり、既定 1）。
//! 視野 `field` は番号（1 始まり）か正規化視野 [hx, hy]、波長 `wave` は番号（1 始まり）か null、面 `surf` は面の番号
//! （物体 0）。目標 `target` の既定は 0（MTF・結合効率は 1）。目標が 1 つの値の項（mtf の tan・sag、paraxial・ray・opd・
//! zernike・coupling）は `target` の代わりに `min`（値 ≥ min のときは残差 0）か `max`（値 ≤ max のときは残差 0）を書ける。
//!
//! | type | キー |
//! |---|---|
//! | `spot`・`wavefront` | field、wave（null は多色）、rings（3）、reference（"centroid"・"chief"） |
//! | `mtf` | field、wave、freq [cycles/mm]、dir（"tan"・"sag"・"both"）、n（瞳の標本数）、target |
//! | `paraxial` | q（efl・pmag・enpp・epd・expp・expd・wfno・pwfn・isfn・pimh・totr・obna・isna）、target |
//! | `ray` | field、wave（null は主波長）、px・py（正規化瞳座標、0）、surf（像面）、q（x・y・z・l・m・n）、target |
//! | `opd` | field、wave、px・py、target [波] |
//! | `zernike` | field、wave、term（1 始まり）、kind（"fringe"・"standard"）、n（格子、64）、target [波] |
//! | `coupling` | wave、na（送り・受けとも）か na_source・na_receiver、source・receiver [x, y]、grid（101）、na_max、clip_stop、target |
//! | `zplm` | macro（番号 nn なら ZPLnn.ZPL、か名前）か file（.ZPL のパス）、data（OPTRETURN の番号、0）、hx・hy・px・py（PVHX…、0）、macro_dirs（探すフォルダの並び）、target |
//! | `plugin` | name（登録の名前）、args（設定 {"名前": 値}）、inputs（入力 {"名前": 数か数の並び}）か field（inputs の field の略）、output（出力の名前）、index（その要素だけ）、target（数か出力と同じ数の並び、0） |
//!
//! `zplm` のマクロを探す順: file、macro_dirs、系に読み込んだマクロ（.zmx のソルブのもの）、LENSBENCH_MACROS、今のフォルダ。
//! `"default_dirs": false` なら最後の 2 つ（既定の場所）を探さない（serve が --root・--allow-remote のときに付ける）。

use lensbench_core::analysis::ZernKind;
use lensbench_core::fiber::FiberOpts;
use lensbench_core::optim::{MtfDir, ParaxQ, RayQ, Side, Term, TermKind};
use lensbench_core::sweep::RmsRef;
use lensbench_core::{FieldSel, System};
use serde_json::{Map, Value};

/// 項の並び（配列か 1 つのオブジェクト）
pub fn terms_from_json(v: &Value, configs: &[System]) -> Result<Vec<Term>, String> {
    match v {
        Value::Array(a) => a
            .iter()
            .enumerate()
            .map(|(i, t)| term_from_json(t, configs).map_err(|e| format!("項 {}: {e}", i + 1)))
            .collect(),
        _ => Ok(vec![term_from_json(v, configs)?]),
    }
}

struct Obj<'a> {
    m: &'a Map<String, Value>,
    used: Vec<&'static str>,
}

impl<'a> Obj<'a> {
    fn get(&mut self, k: &'static str) -> Option<&'a Value> {
        self.used.push(k);
        self.m.get(k).filter(|v| !v.is_null())
    }
    fn f64_or(&mut self, k: &'static str, d: f64) -> Result<f64, String> {
        match self.get(k) {
            None => Ok(d),
            Some(v) => v.as_f64().ok_or_else(|| format!("{k} は数（{v}）")),
        }
    }
    fn f64_req(&mut self, k: &'static str) -> Result<f64, String> {
        match self.get(k) {
            None => Err(format!("{k} がない")),
            Some(v) => v.as_f64().ok_or_else(|| format!("{k} は数（{v}）")),
        }
    }
    fn usize_opt(&mut self, k: &'static str) -> Result<Option<usize>, String> {
        match self.get(k) {
            None => Ok(None),
            Some(v) => v
                .as_u64()
                .map(|x| Some(x as usize))
                .ok_or_else(|| format!("{k} は 0 以上の整数（{v}）")),
        }
    }
    fn str_or(&mut self, k: &'static str, d: &'static str) -> Result<String, String> {
        match self.get(k) {
            None => Ok(d.to_string()),
            Some(v) => v
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| format!("{k} は文字列（{v}）")),
        }
    }
    fn pair(&mut self, k: &'static str) -> Result<Option<[f64; 2]>, String> {
        match self.get(k) {
            None => Ok(None),
            Some(v) => {
                let a = v.as_array().filter(|a| a.len() == 2);
                let p = a.and_then(|a| Some([a[0].as_f64()?, a[1].as_f64()?]));
                p.map(Some).ok_or_else(|| format!("{k} は [x, y]（{v}）"))
            }
        }
    }
    fn bool_or(&mut self, k: &'static str, d: bool) -> Result<bool, String> {
        match self.get(k) {
            None => Ok(d),
            Some(v) => v
                .as_bool()
                .ok_or_else(|| format!("{k} は true・false（{v}）")),
        }
    }
    fn field(&mut self, s: &System) -> Result<FieldSel, String> {
        match self.get("field") {
            None => Err("field がない（番号か [hx, hy]）".into()),
            Some(Value::Array(_)) => {
                let [x, y] = self.pair("field")?.unwrap();
                Ok(FieldSel::Norm(x, y))
            }
            Some(v) => {
                let i = v
                    .as_u64()
                    .ok_or_else(|| format!("field は番号か [hx, hy]（{v}）"))?
                    as usize;
                if i == 0 || i > s.fields.len() {
                    return Err(format!("視野 {i} はない（1〜{}）", s.fields.len()));
                }
                Ok(FieldSel::Index(i - 1))
            }
        }
    }
    fn wave(&mut self, s: &System) -> Result<Option<usize>, String> {
        match self.usize_opt("wave")? {
            None => Ok(None),
            Some(w) if w >= 1 && w <= s.wavelengths.len() => Ok(Some(w - 1)),
            Some(w) => Err(format!("波長 {w} はない（1〜{}）", s.wavelengths.len())),
        }
    }
    fn reference(&mut self) -> Result<RmsRef, String> {
        match self
            .str_or("reference", "centroid")?
            .to_ascii_lowercase()
            .as_str()
        {
            "centroid" => Ok(RmsRef::Centroid),
            "chief" => Ok(RmsRef::Chief),
            r => Err(format!("reference は centroid か chief（{r}）")),
        }
    }
}

/// 1 つの項
pub fn term_from_json(v: &Value, configs: &[System]) -> Result<Term, String> {
    let m0 = v
        .as_object()
        .ok_or_else(|| format!("項は {{\"type\": ...}} のオブジェクト（{v}）"))?;
    // 片側の条件: "min"（値 ≥ min）・"max"（値 ≤ max）は target の代わりに書く
    let mut m1 = m0.clone();
    let mut side = Side::Eq;
    for (k, sd) in [("min", Side::Min), ("max", Side::Max)] {
        if let Some(b) = m1.remove(k).filter(|b| !b.is_null()) {
            if side != Side::Eq || m0.get("target").is_some_and(|t| !t.is_null()) {
                return Err("target・min・max はどれか 1 つ".into());
            }
            m1.insert("target".into(), b);
            side = sd;
        }
    }
    let m = &m1;
    let mut o = Obj {
        m,
        used: Vec::new(),
    };
    let ty = o.str_or("type", "")?.to_ascii_lowercase();
    let weight = o.f64_or("weight", 1.0)?;
    let config = match o.usize_opt("config")? {
        None => 0,
        Some(c) if c >= 1 && c <= configs.len() => c - 1,
        Some(c) => return Err(format!("構成 {c} はない（1〜{}）", configs.len())),
    };
    let s = configs.get(config).ok_or("構成がない")?;
    let kind = match ty.as_str() {
        "spot" | "wavefront" | "wfe" => {
            let field = o.field(s)?;
            let wave = o.wave(s)?;
            let rings = o.usize_opt("rings")?.unwrap_or(3);
            let reference = o.reference()?;
            if ty == "spot" {
                TermKind::Spot {
                    field,
                    wave,
                    rings,
                    reference,
                }
            } else {
                TermKind::Wavefront {
                    field,
                    wave,
                    rings,
                    reference,
                }
            }
        }
        "mtf" => TermKind::Mtf {
            field: o.field(s)?,
            wave: o.wave(s)?,
            n: o.usize_opt("n")?,
            freq: o.f64_req("freq")?,
            dir: match o.str_or("dir", "both")?.to_ascii_lowercase().as_str() {
                "tan" | "t" => MtfDir::Tan,
                "sag" | "s" => MtfDir::Sag,
                "both" => MtfDir::Both,
                d => return Err(format!("dir は tan・sag・both（{d}）")),
            },
            target: o.f64_or("target", 1.0)?,
        },
        "paraxial" => TermKind::Paraxial {
            q: ParaxQ::parse(&o.str_or("q", "")?).map_err(|e| e.to_string())?,
            target: o.f64_req("target")?,
        },
        "ray" => TermKind::Ray {
            field: o.field(s)?,
            wave: o.wave(s)?,
            px: o.f64_or("px", 0.0)?,
            py: o.f64_or("py", 0.0)?,
            surf: o.usize_opt("surf")?,
            q: RayQ::parse(&o.str_or("q", "")?).map_err(|e| e.to_string())?,
            target: o.f64_or("target", 0.0)?,
        },
        "opd" => TermKind::Opd {
            field: o.field(s)?,
            wave: o.wave(s)?,
            px: o.f64_or("px", 0.0)?,
            py: o.f64_or("py", 0.0)?,
            target: o.f64_or("target", 0.0)?,
        },
        "zernike" => TermKind::Zernike {
            field: o.field(s)?,
            wave: o.wave(s)?,
            kind: match o.str_or("kind", "fringe")?.to_ascii_lowercase().as_str() {
                "fringe" => ZernKind::Fringe,
                "standard" => ZernKind::Standard,
                k => return Err(format!("kind は fringe か standard（{k}）")),
            },
            term: o.usize_opt("term")?.ok_or("term がない")?,
            n: o.usize_opt("n")?.unwrap_or(64),
            target: o.f64_or("target", 0.0)?,
        },
        "coupling" => {
            let d = FiberOpts::default();
            let na = o.f64_or("na", f64::NAN)?;
            let pick = |x: f64, d: f64| if x.is_nan() { d } else { x };
            let na_s = o.f64_or("na_source", f64::NAN)?;
            let na_r = o.f64_or("na_receiver", f64::NAN)?;
            let opts = FiberOpts {
                na_source: pick(na_s, pick(na, d.na_source)),
                na_receiver: pick(na_r, pick(na, d.na_receiver)),
                source: o.pair("source")?.unwrap_or(d.source),
                receiver: o.pair("receiver")?.unwrap_or(d.receiver),
                grid: o.usize_opt("grid")?.unwrap_or(d.grid),
                na_max: o.get("na_max").and_then(|v| v.as_f64()),
                clip_stop: o.bool_or("clip_stop", d.clip_stop)?,
            };
            TermKind::Coupling {
                wave: o.wave(s)?,
                opts,
                target: o.f64_or("target", 1.0)?,
            }
        }
        "zplm" => {
            let m = zplm_macro(&mut o, s)?;
            let data = o.usize_opt("data")?.unwrap_or(0);
            let pv = [
                o.f64_or("hx", 0.0)?,
                o.f64_or("hy", 0.0)?,
                o.f64_or("px", 0.0)?,
                o.f64_or("py", 0.0)?,
            ];
            let t = lensbench_core::zplm::ZplmTerm::new(&m, data, pv, o.f64_or("target", 0.0)?)
                .map_err(|e| e.to_string())?;
            TermKind::User(lensbench_core::optim::UserTermRef(std::sync::Arc::new(t)))
        }
        "plugin" => {
            let name = o.str_or("name", "")?;
            if name.is_empty() {
                return Err("name（プラグインの名前）がない".into());
            }
            let args = crate::plugin_term::config_of(o.get("args"))?;
            let mut inputs = crate::plugin_term::arrays_of(o.get("inputs"))?;
            if let Some(f) = o.get("field") {
                if inputs.contains_key("field") {
                    return Err("field と inputs の field はどちらか一方".into());
                }
                inputs.insert("field".into(), crate::plugin_term::array_of("field", f)?);
            }
            let output = o.str_or("output", "")?;
            if output.is_empty() {
                return Err("output（出力の名前）がない".into());
            }
            let index = o.usize_opt("index")?;
            let target = match o.get("target") {
                None => vec![],
                Some(Value::Array(a)) => a
                    .iter()
                    .map(|x| {
                        x.as_f64()
                            .ok_or_else(|| format!("target は数か数の並び（{x}）"))
                    })
                    .collect::<Result<_, _>>()?,
                Some(v) => vec![v
                    .as_f64()
                    .ok_or_else(|| format!("target は数か数の並び（{v}）"))?],
            };
            let t = crate::plugin_term::PluginTerm::new(
                &name, &args, inputs, &output, index, target, s,
            )
            .map_err(|e| e.to_string())?;
            TermKind::User(lensbench_core::optim::UserTermRef(std::sync::Arc::new(t)))
        }
        "" => return Err("type がない".into()),
        t => {
            const TYPES: &str =
                "spot・wavefront・mtf・paraxial・ray・opd・zernike・coupling・zplm・plugin";
            return Err(format!("type {t}（{TYPES}）"));
        }
    };
    if let Some(k) = m.keys().find(|k| !o.used.contains(&k.as_str())) {
        return Err(format!("{ty} の項に使わないキー {k}"));
    }
    if side != Side::Eq
        && !matches!(
            kind,
            TermKind::Mtf {
                dir: MtfDir::Tan | MtfDir::Sag,
                ..
            } | TermKind::Paraxial { .. }
                | TermKind::Ray { .. }
                | TermKind::Opd { .. }
                | TermKind::Zernike { .. }
                | TermKind::Coupling { .. }
        )
    {
        return Err(format!(
            "{ty} の項に min・max は使えない（目標が 1 つの値の項だけ）"
        ));
    }
    Ok(Term {
        side,
        config,
        weight,
        kind,
    })
}

/// zplm の項のマクロ（macro か file）を探して読む
fn zplm_macro(o: &mut Obj, s: &System) -> Result<lensbench_core::zpl_solve::Macro, String> {
    use lensbench_core::zpl_solve::{find_macro, macro_dirs, Macro};
    use std::path::{Path, PathBuf};
    let defaults = o.bool_or("default_dirs", true)?;
    if let Some(f) = o.get("file") {
        let f = f.as_str().ok_or_else(|| format!("file は文字列（{f}）"))?;
        let p = Path::new(f);
        let src = lensbench_core::vfs::read(p).map_err(|e| format!("{f}: {e}"))?;
        let name = p
            .file_stem()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut m =
            Macro::from_source(&name, &String::from_utf8_lossy(&src)).map_err(|e| e.to_string())?;
        m.path = p.to_path_buf();
        if o.get("macro").is_some() {
            return Err("macro と file はどちらか一方".into());
        }
        return Ok(m);
    }
    let name = match o.get("macro") {
        None => return Err("macro（番号か名前）か file がない".into()),
        Some(Value::String(n)) => n.clone(),
        Some(v) => match v.as_u64() {
            Some(n) if n <= 99 => format!("ZPL{n:02}"),
            _ => return Err(format!("macro は 0〜99 の番号か名前（{v}）")),
        },
    };
    let mut extra: Vec<PathBuf> = vec![];
    if let Some(d) = o.get("macro_dirs") {
        let a = d
            .as_array()
            .ok_or_else(|| format!("macro_dirs はフォルダの並び（{d}）"))?;
        for x in a {
            extra.push(PathBuf::from(
                x.as_str()
                    .ok_or_else(|| format!("macro_dirs の要素は文字列（{x}）"))?,
            ));
        }
    }
    if !extra.is_empty() {
        if let Ok(m) = find_macro(&name, &extra) {
            return Ok(m);
        }
    }
    if let Some(m) = s.macros.iter().find(|m| m.name.eq_ignore_ascii_case(&name)) {
        return Ok(m.clone());
    }
    if !defaults {
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
        let mut p = lensbench_core::optim::Problem::single(s.clone());
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
        let mut p = lensbench_core::optim::Problem::single(s.clone());
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
        let mut p = lensbench_core::optim::Problem::single(s.clone());
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
