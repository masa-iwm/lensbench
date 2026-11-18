//! `save_project`: 読み込んだプロジェクトに、読み込んだ後の面の変更をブロックの面の上書き・操作として書き戻して
//! 保存する（[`lensbench_project::writeback`]）

use super::*;
use lensbench_project::edit_ops::EditOp;
use lensbench_project::edit_spec::{Num, Ref};

/// set のパラメータ名（`3.t` など）を、面の値の編集に。面の値で書けないもの（玉・群・系のパラメータ、名前の面）は
/// SetParam のまま（書き戻せない注意になる）
fn set_op(name: &str, value: f64) -> EditOp {
    let plain = || EditOp::SetParam {
        param: name.to_string(),
        value: Num(value),
    };
    let Some((t, k)) = name.trim().rsplit_once('.') else {
        return plain();
    };
    let t = t.strip_prefix("surf:").unwrap_or(t);
    use lensbench_core::param::Kind;
    let key = match Kind::parse(k) {
        Ok(Kind::Curv) => "c",
        Ok(Kind::Radius) => "r",
        Ok(Kind::Conic) => "k",
        Ok(Kind::Thick) => "t",
        _ => return plain(),
    };
    let Ok(j) = t.trim().parse::<usize>() else {
        return plain();
    };
    let mut fields = serde_json::Map::new();
    fields.insert(key.into(), json!(value));
    EditOp::SetSurface {
        surface: Ref::Index(j),
        fields,
    }
}

/// 読み込んだプロジェクトに ops を書き戻したもの（書き戻す前のもの、notes、読み込みの指定）。ops が None なら
/// 読み込んだ後の変更（set・edit の記録）
fn written(
    sess: &Session,
    ops: Option<Vec<EditOp>>,
) -> Result<
    (
        lensbench_project::Project,
        lensbench_project::Project,
        Vec<String>,
        LoadSpec,
    ),
    RpcErr,
> {
    let Current {
        sys,
        src,
        faces,
        log,
        spec,
        ..
    } = sess.current_full()?;
    let Some(proj) = src.project() else {
        return Err(RpcErr::app(if ops.is_some() {
            "write_back is only for project files"
        } else {
            "save_project is only for project files (use save for a system file)"
        }));
    };
    let before = proj.clone();
    let mut proj = proj.clone();
    let from_rpc = ops.is_some();
    let ops: Vec<EditOp> = ops.unwrap_or_else(|| {
        log.iter()
            .flat_map(|c| match c {
                Change::Set(v) => v.iter().map(|(n, _, x)| set_op(n, *x)).collect(),
                Change::Edit(o) => o.clone(),
            })
            .collect()
    });
    // 書き戻したプロジェクトは今の状態・経路で展開できること（展開で断る上書きは外して notes）
    let cat = lensbench_core::catalog::LensCatalog::load_registered(&spec.lens_catalog)?;
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let env = lensbench_project::Env {
        step_files: Some(&lensbench_cad::cached_step),
        base: spec
            .file
            .parent()
            .map(|x| x.to_path_buf())
            .unwrap_or_default(),
        glass_dirs: spec.glass_dir.clone(),
        catalog: Some(&look),
    };
    let cache = match &*src {
        Source::Project { cache, .. } => cache.clone(),
        _ => Default::default(),
    };
    let check = |q: &lensbench_project::Project| -> Result<(), String> {
        let o = lensbench_project::ExpandOpts {
            state: spec.state.as_deref(),
            path: spec.path.as_deref(),
            cache: Some(&cache),
            ..Default::default()
        };
        lensbench_project::expand(q, &env, &o)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    // 編集する前の系: write_back は読み込んだまま（ops を当てない）、保存は読み込んだプロジェクトを展開し直す
    let fresh;
    let base: &lensbench_core::System = if log.is_empty() || from_rpc {
        &sys
    } else {
        let o = lensbench_project::ExpandOpts {
            state: spec.state.as_deref(),
            path: spec.path.as_deref(),
            cache: Some(&cache),
            ..Default::default()
        };
        fresh = lensbench_project::expand(&before, &env, &o)?.system;
        &fresh
    };
    let ed = lensbench_project::writeback::Edited {
        system: &sys,
        faces: &faces,
        before: base,
    };
    let notes = lensbench_project::writeback::write_back(&mut proj, ed, &ops, &check);
    Ok((before, proj, notes, spec))
}

pub(super) fn save_project(sess: &Session, path: &Path) -> Result<Value, RpcErr> {
    let (_, proj, mut notes, spec) = written(sess, None)?;
    let out = sess.files.check_path(path, true)?;
    // 相対パス（.zmx・カタログ・ユーザーカタログ）は書いたまま。別のフォルダへ書くと指す先が変わる
    if out.parent() != spec.file.parent() {
        notes.push(
            "the project is written to another folder; relative paths in it are kept as written"
                .into(),
        );
    }
    std::fs::write(&out, proj.to_json())
        .map_err(|e| RpcErr::app(format!("{}: {e}", path.display())))?;
    Ok(json!({ "written": path, "notes": notes }))
}

/// `write_back`: 読み込んだプロジェクトに面の操作 ops（edit の操作）を書き戻し、面の上書き・操作が変わった
/// ブロック（id のあるもの）を返す。保存も、読み込んだ系の変更もしない（GUI の組合せが要素へ当てて組み直す）
pub(super) fn write_back(sess: &Session, ops: Vec<EditOp>) -> Result<Value, RpcErr> {
    let (before, after, notes, _) = written(sess, Some(ops))?;
    let old = lensbench_project::writeback::face_mods(&before);
    let blocks: Vec<Value> = lensbench_project::writeback::face_mods(&after)
        .into_iter()
        .filter(|(id, f)| {
            let was = old.iter().find(|(i, _)| i == id).map(|(_, g)| json!(g));
            was != Some(json!(f))
        })
        .map(|(id, f)| json!({ "id": id, "faces": f }))
        .collect();
    Ok(json!({ "blocks": blocks, "notes": notes }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_names_take_the_param_aliases() {
        for (n, k) in [
            ("3.radius", "r"),
            ("surf:3.thickness", "t"),
            ("3.curv", "c"),
            ("3.K", "k"),
        ] {
            match set_op(n, 1.0) {
                EditOp::SetSurface {
                    surface: Ref::Index(3),
                    fields,
                } => {
                    assert!(fields.contains_key(k), "{n}")
                }
                o => panic!("{n}: {o:?}"),
            }
        }
        assert!(matches!(set_op("3.dx", 1.0), EditOp::SetParam { .. }));
    }
}
