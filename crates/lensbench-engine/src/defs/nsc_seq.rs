//! 非順次の経路の表の行を順次の系に戻す（[`lensbench_project::nsc::seq`]）: 場面の系の主の経路、ゴーストの展開
//! （面 a・b）、プロジェクトに足す光束（BS の経路と玉の面での折り返し `turn` の並び）

use std::path::Path;

use lensbench_analysis::Cx;
use lensbench_core::Result;
use lensbench_project::nsc::seq;
use lensbench_project::raypath::EventKind;
use lensbench_project::scene::{ObjectKind, Scene};
use serde_json::{json, Value};

use super::nsc::SceneSystem;

/// 足して確かめる光束の名前（プロジェクトの光束・状態の名前と重ならないもの）
const PROBE: &str = "nsc-path";

/// 行の出来事の列 events を順次の系に戻せるか。`kind` は system（場面の系そのもの）・ghost（系の面 a・b のゴースト）・
/// beam（足す光束の path・turn。turn は 1 つならオブジェクト、2 つ以上は並び）・none（戻せない。`reason`）。out があればゴーストの系を .zmx に書く（`file`）
pub(super) fn sequential(
    cx: &Cx<'_>,
    sc: &Scene,
    systems: &[SceneSystem],
    keys: &[Vec<Option<Value>>],
    events: &[(usize, EventKind)],
    out: Option<&Path>,
    notes: &mut Vec<String>,
) -> Result<Value> {
    for (k, (label, s, _)) in systems.iter().enumerate() {
        if seq::is_system_path(sc, k, s, events)? {
            return Ok(json!({"kind": "system", "name": label}));
        }
    }
    let refl: Vec<usize> = events
        .iter()
        .filter(|e| e.1.reflects())
        .map(|e| e.0)
        .collect();
    let on_lens = |f: &usize| sc.objects[sc.faces[*f].object].kind == ObjectKind::Lens;
    if events
        .iter()
        .any(|e| matches!(e.1, EventKind::Scatter | EventKind::ScatterReflect))
    {
        return Ok(none("the path scatters"));
    }
    for (k, (label, s, _)) in systems.iter().enumerate() {
        if let Some((a, b)) = seq::ghost_of(sc, k, s, events)? {
            let mut v = json!({"kind": "ghost", "system": label, "a": a, "b": b});
            if let Some(dir) = out {
                lensbench_core::vfs::create_dir_all(dir)
                    .map_err(|x| lensbench_core::Error::Io(format!("{}: {x}", dir.display())))?;
                let name = if systems.len() > 1 {
                    format!("ghost_{}_{a}_{b}.zmx", k + 1)
                } else {
                    format!("ghost_{a}_{b}.zmx")
                };
                let f = dir.join(name);
                let g = lensbench_analysis::ghost::ghost_system(s, a, b)?;
                notes.extend(lensbench_formats::zmx_write::save(&f, &g.sys)?);
                v["file"] = json!(f.display().to_string());
            }
            return Ok(v);
        }
    }
    // 折り返しの光束: 玉の面（書き方のある面）での反射を順に turn に並べる
    let lens_refl: Vec<usize> = refl.iter().copied().filter(on_lens).collect();
    let keyed: Vec<Value> = lens_refl
        .iter()
        .filter_map(|&f| seq::face_key(sc, keys, f))
        .map(|mut k| {
            // turn の面は往路の番号で書く（向きは折り返すごとに入れ替わる）
            if let Some(o) = k.as_object_mut() {
                o.remove("back");
            }
            k
        })
        .collect();
    if let (Some(src), false) = (cx.source, keyed.is_empty() || keyed.len() < lens_refl.len()) {
        // pass は前の折り返しからの回数なので、場面の系の書き方の pass で合わなければ外して試す
        let bare: Vec<Value> = keyed
            .iter()
            .map(|k| {
                let mut k = k.clone();
                if let Some(o) = k.as_object_mut() {
                    o.remove("pass");
                }
                k
            })
            .collect();
        let mut tries = vec![keyed.clone()];
        if bare != keyed {
            tries.push(bare);
        }
        let mut paths: Vec<Option<String>> = vec![None];
        paths.extend(src.paths()?.into_iter().map(Some));
        for turns in tries {
            let turn = match &turns[..] {
                [t] => t.clone(),
                _ => json!(turns),
            };
            for path in &paths {
                let mut beam = json!({"name": PROBE, "turn": turn, "role": "stray"});
                if let Some(p) = path {
                    beam["path"] = json!(p);
                }
                let mut n = Vec::new();
                let r = src.load_extra_beam(&beam, &mut n);
                let Ok(Some((s, _))) = r else {
                    continue;
                };
                if seq::is_placed_path(sc, &s, events)? {
                    return Ok(json!({"kind": "beam", "path": path, "turn": turn}));
                }
            }
        }
    }
    Ok(none(
        "no sequential system (beam, beam splitter path or ghost) has this path",
    ))
}

fn none(reason: &str) -> Value {
    json!({"kind": "none", "reason": reason})
}
