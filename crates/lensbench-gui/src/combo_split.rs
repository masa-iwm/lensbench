//! 開いた .zmx を組合せの要素（自作の玉・絞り・面の並びと空気）に分ける（11/16 masa 決定: 開いた .zmx の変更は
//! 取り込みの上書きでなく中身へ）。ソルブ・ピックアップ・絞りは要素の操作にする。分けた組合せの系が元の系と
//! 同じときだけ使い、書けない面（玉の中の鏡・座標ブレーク・ゴーストの反射など）があれば今までどおり .zmx を取り込む。
//! 空気中の鏡は鏡の要素（半径・コーニックで書けない形は行で）。鏡の後の面は光の進む向きに裏返して書く。
//! SurfSpec の欄で書けない形（Zernike・Grid Sag など）はシステムファイルの行（SurfSpec の row）で書く

use std::path::{Path, PathBuf};

use lensbench_core::solves::SolveKind;
use lensbench_core::surface::SurfType;
use lensbench_core::System;
use serde_json::{json, Value};

use crate::combo::{Combo, ElemKind};

/// 系 s（.zmx を構成 1 で読んだもの）を、p（[`lensbench_project::zmx_project`] の設定）の blocks を要素に分けた
/// 組合せにする。同じ系にならなければ None
pub(crate) fn split(s: &System, p: &Value, base: &Path, glass_dirs: &[PathBuf]) -> Option<Combo> {
    let v = split_project(s, p)?;
    let c = Combo::from_project(&v, base).ok()?;
    if c.picks.iter().any(|q| q.kind == ElemKind::Raw) {
        return None;
    }
    let t = lensbench_project::Project::from_json(&c.project().to_string())
        .and_then(|q| {
            lensbench_project::expand(
                &q,
                &lensbench_project::Env::in_dir(base, glass_dirs, None),
                &Default::default(),
            )
            .map_err(lensbench_core::Error::from)
        })
        .map(|x| x.system)
        .ok()?;
    lensbench_project::same_system(s, &t).ok()?;
    Some(c)
}

/// 曲率 c の面の半径（1/半径 が c に戻る値。平面は 0）
fn radius_of(c: f64) -> Option<f64> {
    if c == 0.0 {
        return Some(0.0);
    }
    let mut r = 1.0 / c;
    // 1/(1/c) が c に戻らなければ、隣の値を少し探す
    for _ in 0..8 {
        if 1.0 / r == c {
            return Some(r);
        }
        r = if 1.0 / r > c {
            r.next_up()
        } else {
            r.next_down()
        };
    }
    None
}

/// 鏡の面 a（光の進む向き）を鏡のブロックに。標準面で開口・膜・傾きが無ければ半径・コーニックで、ほかは行で
fn mirror_block(a: &lensbench_core::surface::Surface, id: &str) -> Option<Value> {
    use lensbench_core::surface::Aperture;
    let plain = a.typ() == SurfType::Standard
        && matches!(a.ap, Aperture::None)
        && a.coat.is_empty()
        && !a.semi_auto
        && a.before.is_none()
        && a.after == lensbench_core::surface::After::None;
    if plain {
        return Some(
            json!({ "type": "mirror", "id": id, "radius": radius_of(a.c)?,
            "conic": a.k, "semi": a.semi }),
        );
    }
    let r = lensbench_project::mirror_row(a).ok()?;
    Some(json!({ "type": "mirror", "id": id, "row": r }))
}

/// blocks（面の絞り・ソルブ・ピックアップはブロックの surface_ops）・glrs を要素で書いたプロジェクト
fn split_project(s: &System, p: &Value) -> Option<Value> {
    if !s.macros.is_empty() || !s.macro_parms.is_empty() || s.surfaces.len() < 3 {
        return None;
    }
    let img = s.surfaces.len() - 1;
    let mut blocks = Vec::new();
    // 系の面 → (ブロック id, ブロックの面)
    let mut face: Vec<Option<(String, usize)>> = vec![None; img + 1];
    let mut stop: Option<(String, usize)> = None;
    // 面を光の進む向きで（ブロックは向きで書き、鏡の後は展開が裏返す）。鏡の後の面は裏返し、厚さの符号を返す
    let mut dirs = Vec::with_capacity(img + 1);
    let mut par = 1.0;
    for a in &s.surfaces {
        let mut a = a.clone();
        if par < 0.0 {
            a.flip_z().ok()?;
            a.t = -a.t;
        }
        if a.is_mirror() {
            par = -par;
        }
        dirs.push(a);
    }
    // 面 j を SurfSpec の JSON に（書けない面は None）。玉の中の面は Jones にしない
    let surf = |j: usize, last: bool, in_lens: bool| -> Option<Value> {
        let a = &dirs[j];
        if (in_lens && a.typ() == SurfType::Jones) || !a.mat.is_medium() {
            return None;
        }
        let mut x = serde_json::to_value(lensbench_project::surface_spec(a, last).ok()?).ok()?;
        // 半径は 1/半径 が曲率に戻る値に（系と同じ値にする）。行で書く面は行の曲率のまま
        if x.get("row").is_none_or(Value::is_null) {
            x["radius"] = json!(radius_of(a.c)?);
        }
        if last {
            if let Some(o) = x.as_object_mut() {
                o.remove("t");
                o.remove("glass");
            }
        }
        Some(x)
    };
    let (mut nl, mut ns, mut nm) = (0, 0, 0);
    let mut j = 1;
    while j < img {
        let id;
        let k;
        if s.surfaces[j].is_mirror() {
            // 空気中の鏡: 鏡のブロック（半径・コーニックで書けなければ行で）
            k = j;
            nm += 1;
            id = format!("M{nm}");
            blocks.push(mirror_block(&dirs[j], &id)?);
        } else if s.surfaces[j].mat.is_air() {
            // 空気中の面 1 つ: 面の並び（絞りなら set_stop で）
            k = j;
            ns += 1;
            id = format!("S{ns}");
            blocks
                .push(json!({ "type": "surfaces", "id": id, "surfaces": [surf(j, true, false)?] }));
        } else {
            // ガラスに入る面から、空気に出る面までが 1 つの玉
            let mut e = j;
            while e < img && !s.surfaces[e].mat.is_air() {
                if s.surfaces[e].is_mirror() {
                    return None;
                }
                e += 1;
            }
            if e >= img {
                return None;
            }
            k = e;
            nl += 1;
            id = format!("L{nl}");
            let ss = (j..=k)
                .map(|x| surf(x, x == k, true))
                .collect::<Option<Vec<_>>>()?;
            blocks.push(json!({ "type": "lens", "id": id, "source": { "surfaces": ss } }));
        }
        for (i, x) in (j..=k).enumerate() {
            face[x] = Some((id.clone(), i));
            if s.surfaces[x].stop {
                stop = Some((id.clone(), i));
            }
        }
        blocks.push(json!({ "type": "air", "t": dirs[k].t }));
        j = k + 1;
    }
    let at = |j: usize| -> Option<Value> {
        let (b, i) = face.get(j)?.as_ref()?;
        Some(json!({ "block": b, "surface": i }))
    };
    let mut ops = Vec::new();
    if let Some((b, i)) = stop {
        ops.push(json!({ "op": "set_stop", "surface": { "block": b, "surface": i } }));
    }
    for q in &s.solves {
        if matches!(
            q.kind,
            SolveKind::MacroCurv { .. } | SolveKind::MacroThick { .. }
        ) {
            return None;
        }
        let (kind, value, param, other) = lensbench_core::solves::solve_kind_info(q.kind);
        let mut o = json!({ "op": "add_solve", "surface": at(q.surf)?, "kind": kind,
            "value": value, "param": param });
        if let Some(x) = other {
            o["other"] = at(x)?;
        }
        ops.push(o);
    }
    for q in &s.pickups {
        let name = lensbench_core::pickup::pick_kind_name;
        let mut o = json!({ "op": "add_pickup", "dst": at(q.dst)?, "src": at(q.src)?,
            "what": name(q.what), "scale": q.scale, "offset": q.offset });
        if let Some(w) = q.from {
            o["from"] = json!(name(w));
        }
        ops.push(o);
    }
    // 操作は当てる面（surface・dst）のブロックに、面はそのブロックの面の番号で
    for mut o in ops {
        let key = if o.get("dst").is_some() {
            "dst"
        } else {
            "surface"
        };
        let b = o[key]["block"].as_str()?.to_string();
        o[key] = o[key]["surface"].clone();
        let blk = blocks.iter_mut().find(|x| x["id"] == b.as_str())?;
        match blk["surface_ops"].as_array_mut() {
            Some(a) => a.push(o),
            None => blk["surface_ops"] = json!([o]),
        }
    }
    let mut v = p.clone();
    v["blocks"] = json!(blocks);
    if let Some(o) = v.as_object_mut() {
        o.remove("glrs");
    }
    if s.glrs != 1 {
        v["glrs"] = at(s.glrs)?;
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lensbench_project::edit_ops::{BlockRef, SurfaceOp};

    fn root() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
    }

    /// A0（3 枚玉）: 玉 3 つ・絞りの面・像の前のダミー面に分かれ、最後の厚さのソルブは 3 枚目の玉の操作になる
    #[test]
    fn a0_splits_into_elements() {
        let glass = vec![root().join("tests/glass")];
        let c = Combo::from_zmx(&root().join("tests/refs/A0/system.zmx"), &glass, &[]).unwrap();
        let kinds: Vec<ElemKind> = c.picks.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            [
                ElemKind::Lens,
                ElemKind::Lens,
                ElemKind::Surfaces,
                ElemKind::Lens,
                ElemKind::Surfaces
            ]
        );
        let l3 = &c.picks[3];
        assert!(
            l3.ops.iter().any(|o| matches!(o, SurfaceOp::AddSolve(q) if q.surface == BlockRef::Sys(lensbench_project::edit_ops::Ref::Index(1)))),
            "{:?}",
            l3.ops
        );
        // 外すと要素の操作が消える（上書きにならない）
        let mut c = c;
        lensbench_project::surface_ops::drop_solve(
            &mut c.picks[3].ops,
            1,
            lensbench_project::edit_ops::SolveWhat::T,
        );
        assert!(c.picks[3].ops.is_empty());
    }

    /// 自由曲面の鏡（G3 の Grid Sag・Z2 の Zernike Standard Sag）も要素に分かれ、鏡は行の鏡になる
    #[test]
    fn freeform_mirrors_split() {
        let glass = vec![root().join("tests/glass")];
        for d in ["G3", "Z2"] {
            let f = root().join(format!("tests/refs/{d}/system.zmx"));
            let c = Combo::from_zmx(&f, &glass, &[]).unwrap();
            assert!(
                !c.picks
                    .iter()
                    .any(|p| matches!(p.kind, ElemKind::ZmxRange | ElemKind::Raw)),
                "{d}"
            );
            let m: Vec<&crate::combo::Pick> = c
                .picks
                .iter()
                .filter(|p| p.kind == ElemKind::Mirror)
                .collect();
            assert_eq!(m.len(), 1, "{d}");
            assert!(m[0].mirror_row().is_some(), "{d}");
            // 書き出したブロックは行の鏡（半径・半直径の欄は書かない）
            let b = m[0].block("M1");
            assert!(
                b.get("radius").is_none() && b.get("semi").is_none(),
                "{d}: {b}"
            );
        }
    }

    /// tests/refs の .zmx: 分けられたものは元の系と同じ（split の中で確かめる）。分けられない .zmx も開ける
    #[test]
    fn refs_split_or_import() {
        let glass = vec![root().join("tests/glass")];
        let mut n = [0, 0];
        for d in std::fs::read_dir(root().join("tests/refs")).unwrap() {
            let f = d.unwrap().path().join("system.zmx");
            if !f.is_file() {
                continue;
            }
            let Ok(c) = Combo::from_zmx(&f, &glass, &[]) else {
                continue;
            };
            let split = !c.picks.iter().any(|p| p.kind == ElemKind::ZmxRange);
            n[usize::from(split)] += 1;
            eprintln!("{} {}", f.display(), if split { "split" } else { "import" });
        }
        assert!(n[1] > 0, "{n:?}");
    }
}
