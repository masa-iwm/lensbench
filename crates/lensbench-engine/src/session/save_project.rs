//! `save_project`: 読み込んだプロジェクトに、読み込んだ後の面の変更をブロックの面の上書き・操作として書き戻して
//! 保存する（PRJ-04、[`lensbench_project::writeback`]）

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

pub(super) fn save_project(sess: &Session, p: &Value) -> Result<(Value, Vec<Bin>), RpcErr> {
    let path = p["path"]
        .as_str()
        .ok_or_else(|| RpcErr::params("save_project needs path"))?;
    let Current {
        sys,
        src,
        faces,
        log,
        spec,
        ..
    } = sess.current_full()?;
    let Some(proj) = src.project() else {
        return Err(RpcErr::app(
            "save_project is only for project files (use save for a system file)",
        ));
    };
    let mut proj = proj.clone();
    let ops: Vec<EditOp> = log
        .iter()
        .flat_map(|c| match c {
            Change::Set(v) => v.iter().map(|(n, _, x)| set_op(n, *x)).collect(),
            Change::Edit(o) => o.clone(),
        })
        .collect();
    let ed = lensbench_project::writeback::Edited {
        system: &sys,
        faces: &faces,
    };
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
        };
        lensbench_project::expand(q, &env, &o)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    let mut notes = lensbench_project::writeback::write_back(&mut proj, ed, &ops, &check);
    let out = sess.files.check_path(Path::new(path), true)?;
    // 相対パス（.zmx・カタログ・ユーザーカタログ）は書いたまま。別のフォルダへ書くと指す先が変わる
    if out.parent() != spec.file.parent() {
        notes.push(
            "the project is written to another folder; relative paths in it are kept as written"
                .into(),
        );
    }
    std::fs::write(&out, proj.to_json()).map_err(|e| RpcErr::app(format!("{path}: {e}")))?;
    Ok((json!({ "written": path, "notes": notes }), Vec::new()))
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
