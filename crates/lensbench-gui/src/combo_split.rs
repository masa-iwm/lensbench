//! 開いた .zmx を組合せの要素（自作の玉・絞り・面の並びと空気）に分ける（11/16 masa 決定: 開いた .zmx の変更は
//! 取り込みの上書きでなく中身へ）。ソルブ・ピックアップ・絞りは要素の操作にする。分けた組合せの系が元の系と
//! 同じときだけ使い、書けない面（ゴーストの反射など）があれば今までどおり .zmx を取り込む。ZPL マクロはプロジェクトの macros に。
//! 空気中の鏡は鏡の要素（半径・コーニックで書けない形は行で）。鏡の後の面は光の進む向きに裏返して書く。
//! SurfSpec の欄で書けない形（Zernike・Grid Sag など）はシステムファイルの行（SurfSpec の row）で書く

use std::path::{Path, PathBuf};

use lensbench_core::surface::SurfType;
use lensbench_core::System;
use serde_json::{json, Value};

use crate::combo::{Combo, ElemKind};

/// 系 s（.zmx を構成 1 で読んだもの）を、p（[`lensbench_project::zmx_project`] の設定）の blocks を要素に分けた
/// 組合せにする。同じ系にならなければ None
pub(crate) fn split(s: &System, p: &Value, base: &Path, glass_dirs: &[PathBuf]) -> Option<Combo> {
    let mut v = split_project(s, p)?;
    // .zmx の膜ファイル（フォルダの中なら相対パス。材料の検索フォルダで見つけたものはそこからの相対パスで、
    // 開くときも検索フォルダから引く。別の PC へ持っていっても絶対パスが残らないように）
    if let Some(f) = &s.coating_file {
        let f = std::iter::once(base)
            .chain(glass_dirs.iter().map(|d| d.as_path()))
            .find_map(|d| f.strip_prefix(d).ok())
            .unwrap_or(f);
        v["coating_file"] = json!(f.to_string_lossy());
    }
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
    if s.surfaces.len() < 3 {
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
        if in_lens && a.is_mirror() {
            // 玉の中の鏡: 面の行で、厚さは鏡の後の向き（光の進む向きで正）
            let mut m = a.clone();
            m.t = -m.t;
            return serde_json::to_value(lensbench_project::lens_mirror_spec(&m).ok()?).ok();
        }
        if (in_lens && a.typ() == SurfType::Jones)
            || (!a.mat.is_medium() && a.typ() != SurfType::CoordBrk)
        {
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
    // 面 j の後が空気か（座標ブレーク・鏡は前の媒質のまま）
    let mut air = Vec::with_capacity(img + 1);
    for a in &s.surfaces {
        let prev = air.last().copied().unwrap_or(true);
        air.push(if a.mat.is_medium() {
            a.mat.is_air()
        } else {
            prev
        });
    }
    let (mut nl, mut ns, mut nm) = (0, 0, 0);
    // 最後の間隔を満たす媒質（液浸）
    let mut fill: Option<String> = None;
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
        } else if air[j] {
            // 空気中の面 1 つ: 面の並び（絞りなら set_stop で。座標ブレークもここ）
            k = j;
            ns += 1;
            id = format!("S{ns}");
            blocks
                .push(json!({ "type": "surfaces", "id": id, "surfaces": [surf(j, true, false)?] }));
        } else {
            // ガラスに入る面から、空気に出る面までが 1 つの玉
            let mut e = j;
            while e < img && !air[e] {
                e += 1;
            }
            if e >= img {
                // 像空間が空気でない（液浸）: 最後の面までが玉（1 面なら面の並び）で、後ろの間隔をその媒質で満たす
                if img - 1 < j || !s.surfaces[img - 1].mat.is_medium() {
                    return None;
                }
                fill = Some(match &s.surfaces[img - 1].mat {
                    lensbench_core::material::Material::Index(nd) => format!("{nd}"),
                    m => m.name(),
                });
                e = img - 1;
            }
            k = e;
            if k == j {
                ns += 1;
                id = format!("S{ns}");
                blocks.push(
                    json!({ "type": "surfaces", "id": id, "surfaces": [surf(j, true, false)?] }),
                );
            } else {
                nl += 1;
                id = format!("L{nl}");
                let ss = (j..=k)
                    .map(|x| surf(x, x == k, true))
                    .collect::<Option<Vec<_>>>()?;
                blocks.push(json!({ "type": "lens", "id": id, "source": { "surfaces": ss } }));
            }
        }
        // ブロックの面の番号は座標ブレークを除いて数える（座標ブレークは面の上書き・操作で指せない）
        let mut i = 0;
        for x in j..=k {
            if s.surfaces[x].typ() == SurfType::CoordBrk {
                if s.surfaces[x].stop {
                    return None;
                }
                continue;
            }
            face[x] = Some((id.clone(), i));
            if s.surfaces[x].stop {
                stop = Some((id.clone(), i));
            }
            i += 1;
        }
        // 鏡の後の空気は鏡の後の向きで（dirs の鏡の面は鏡の前の向きのまま）
        let t = if s.surfaces[k].is_mirror() {
            -dirs[k].t
        } else {
            dirs[k].t
        };
        match fill.take() {
            Some(m) => blocks.push(json!({ "type": "air", "t": t, "medium": m })),
            None => blocks.push(json!({ "type": "air", "t": t })),
        }
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
    // 曲率・厚さのマクロのソルブの param はマクロの番号（分けたプロジェクトのマクロは系と同じ並び）
    for q in &s.solves {
        let (kind, value, param, other) = lensbench_core::solves::solve_kind_info(q.kind);
        let mut o = json!({ "op": "add_solve", "surface": at(q.surf)?, "kind": kind,
            "value": value, "param": param });
        if let Some(x) = other {
            o["other"] = at(x)?;
        }
        ops.push(o);
    }
    // 円錐・半直径・面タイプのパラメータのマクロのソルブ
    for q in &s.macro_parms {
        ops.push(
            json!({ "op": "add_macro_solve", "surface": at(q.surf)?, "macro": q.id,
            "what": q.what.name() }),
        );
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
    if !s.macros.is_empty() {
        v["macros"] = serde_json::to_value(&s.macros).ok()?;
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lensbench_core::solves::SolveKind;
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

    /// tests/refs の .zmx はどれも要素に分かれ、元の系と同じ（split の中で確かめる）。面の傾き偏心
    /// （C0・C1 の玉の面と折り返しの鏡）も行で書く
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
        assert!(n[1] > 0 && n[0] == 0, "{n:?}");
    }

    /// 座標ブレークを挟んだ BS の板・キューブ（透過・反射の道、ガラスの中の鏡）・膜ファイルのある .zmx、OPD のソルブのある
    /// .zmx も要素に分かれる。膜ファイルはプロジェクトの coating_file に、玉の中の座標ブレーク・鏡は玉の面に
    #[test]
    fn coordbrk_coating_and_opd_split() {
        let glass = vec![root().join("tests/glass")];
        for f in [
            "tests/splitter/cube_T.zmx",
            "tests/splitter/plate_T.zmx",
            "tests/splitter/plate_R.zmx",
            "tests/splitter/cube_R.zmx",
            "tests/splitter/plate_back_R.zmx",
            "tests/splitter/mich_RT.zmx",
            "tests/splitter/mich_TR.zmx",
            "tests/splitter/tg_TR.zmx",
            "tests/opd_solve/o2.zmx",
            "tests/opd_solve/o4.zmx",
            "tests/opd_solve/o4z.zmx",
        ] {
            let c = Combo::from_zmx(&root().join(f), &glass, &[]).unwrap();
            assert!(
                !c.picks
                    .iter()
                    .any(|p| matches!(p.kind, ElemKind::ZmxRange | ElemKind::Raw)),
                "{f}"
            );
        }
        let c = Combo::from_zmx(&root().join("tests/splitter/cube_T.zmx"), &glass, &[]).unwrap();
        assert_eq!(c.coating_file.as_deref(), Some("LBBS.DAT"));
        let v = c.project();
        assert_eq!(v["coating_file"], "LBBS.DAT");
        let lens = v["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["type"] == "lens")
            .unwrap();
        let cbs = lens["source"]["surfaces"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x.get("coordbrk").is_some())
            .count();
        assert_eq!(cbs, 2, "{lens}");
    }

    /// マクロのソルブ（曲率・厚さ・円錐など）のある .zmx も要素に分け、マクロ（名前と文）はプロジェクトの macros に入れる。
    /// 分けたプロジェクトはマクロのファイルが無くても開ける
    #[test]
    fn a_zmx_with_macro_solves_splits_and_keeps_the_macros() {
        let tmp = std::env::temp_dir().join(format!("lb_gui_zpl_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("LBMIRR.ZPL"),
            "s = SOSO(0)\nSOLVERETURN -CURV(s - 1)\n",
        )
        .unwrap();
        std::fs::write(tmp.join("LBCONI.ZPL"), "SOLVERETURN -0.5\n").unwrap();
        let f = tmp.join("m.zmx");
        std::fs::write(
            &f,
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n  DIAM 5 1 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.02\n  CONI 0\n  KZPL \"LBCONI\"\n  DISZ 5\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV -1e-3 12 0 0 0 \"LBMIRR\"\n  DISZ 45\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        )
        .unwrap();
        let glass = vec![root().join("tests/glass")];
        let c = Combo::from_zmx(&f, &glass, &[]).unwrap();
        std::fs::remove_dir_all(&tmp).ok();
        assert!(
            !c.picks
                .iter()
                .any(|p| matches!(p.kind, ElemKind::ZmxRange | ElemKind::Raw)),
            "{:?}",
            c.picks.iter().map(|p| &p.kind).collect::<Vec<_>>()
        );
        assert_eq!(c.macros.len(), 2);
        // マクロのファイルを消した後もプロジェクトから開け、ソルブが解ける
        let v = c.project();
        let p = lensbench_project::Project::from_json(&v.to_string()).unwrap();
        let s = lensbench_project::expand(
            &p,
            &lensbench_project::Env::in_dir(&root(), &glass, None),
            &Default::default(),
        )
        .unwrap()
        .system;
        assert!(s
            .solves
            .iter()
            .any(|q| matches!(q.kind, SolveKind::MacroCurv { .. })));
        assert!(
            (s.surfaces[3].c + 0.02).abs() < 1e-15,
            "{}",
            s.surfaces[3].c
        );
        assert_eq!(s.macro_parms.len(), 1);
        assert_eq!(s.surfaces[2].k, -0.5);
    }

    /// 材料の検索フォルダで見つけた膜ファイルは、絶対パスでなく検索フォルダからの相対パスでプロジェクトに書き、
    /// 開くときも検索フォルダから引く（保存したプロジェクトを別の所へ持っていっても引ける）
    #[test]
    fn a_coating_file_found_in_a_search_folder_is_written_relative_to_it() {
        let tmp = std::env::temp_dir().join(format!("lb_gui_cofn_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("cube_T.zmx");
        std::fs::copy(root().join("tests/splitter/cube_T.zmx"), &f).unwrap();
        let glass = vec![root().join("tests/glass"), root().join("tests/splitter")];
        let c = Combo::from_zmx(&f, &glass, &[]).unwrap();
        std::fs::remove_dir_all(&tmp).ok();
        assert!(
            !c.picks
                .iter()
                .any(|p| matches!(p.kind, ElemKind::ZmxRange | ElemKind::Raw)),
            "{:?}",
            c.picks.iter().map(|p| &p.kind).collect::<Vec<_>>()
        );
        assert_eq!(c.coating_file.as_deref(), Some("LBBS.DAT"));
    }
}
