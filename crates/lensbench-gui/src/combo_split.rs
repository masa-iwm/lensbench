//! 開いた .zmx を組合せの要素（自作の玉・絞り・面の並びと空気）に分ける（11/16 masa 決定: 開いた .zmx の変更は
//! 取り込みの上書きでなく中身へ）。ソルブ・ピックアップ・絞りは要素の操作にする。分けた組合せの系が元の系と
//! 同じときだけ使い、書けない面（ゴーストの反射など）があれば今までどおり .zmx を取り込む。ZPL マクロはプロジェクトの macros に。
//! 空気中の鏡は鏡の要素（半径・コーニックで書けない形は行で）。鏡の後の面は光の進む向きに裏返して書く。
//! SurfSpec の欄で書けない形（Zernike・Grid Sag など）はシステムファイルの行（SurfSpec の row）で書く
//! 多重構成の .zmx は構成 1 で分け、構成ごとの違いを状態の値（面の上書き・絞りの面・空気・系の値）にする（書けなければ取り込み）

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use lensbench_core::solves::SolveKind;
use lensbench_core::surface::SurfType;
use lensbench_core::System;
use serde_json::{json, Map, Value};

use crate::combo::{Combo, ElemKind};

/// 系 s（.zmx を構成 1 で読んだもの）を、p（[`lensbench_project::zmx_project`] の設定）の blocks を要素に分けた
/// 組合せにする。同じ系にならなければ None
pub(crate) fn split(s: &System, p: &Value, base: &Path, glass_dirs: &[PathBuf]) -> Option<Combo> {
    finish(split_project(s, p)?, s, &[], base, glass_dirs)
}

/// 多重構成の .zmx（構成ごとの系 cfgs、p は [`lensbench_project::zmx_project`] の設定と状態「Config k」）を、
/// 構成 1 で要素に分け、構成 k の違いを状態の値（面の上書き・絞りの面・空気・系の値）にした組合せにする。
/// 面の数・種類・ソルブ・ピックアップが構成で違うなど書けない違いがあるか、状態ごとに同じ系にならなければ None
pub(crate) fn split_configs(
    cfgs: &[System],
    p: &Value,
    base: &Path,
    glass_dirs: &[PathBuf],
) -> Option<Combo> {
    let s0 = cfgs.first()?;
    let a = split_parts(s0, p)?;
    let sts = p["states"].as_array()?;
    if sts.len() != cfgs.len() {
        return None;
    }
    let mut v = a.v.clone();
    let mut named = BTreeSet::new();
    let mut states = Vec::new();
    for (st, sk) in sts.iter().zip(cfgs) {
        let mut st = st.clone();
        st["set"] = Value::Object(config_set(s0, sk, &a, p, &mut named)?);
        states.push(st);
    }
    // 状態で変える空気には「前の要素の id_air」を付ける（組合せの空気の名前と同じ）
    for (ai, id, _) in &a.airs {
        if named.contains(id) {
            v["blocks"][*ai]["id"] = json!(format!("{id}_air"));
        }
    }
    v["states"] = json!(states);
    finish(v, s0, cfgs, base, glass_dirs)
}

/// 分けたプロジェクト v を組合せにする。系 s（状態があれば状態ごとの系 cfgs）と同じにならなければ None
fn finish(
    mut v: Value,
    s: &System,
    cfgs: &[System],
    base: &Path,
    glass_dirs: &[PathBuf],
) -> Option<Combo> {
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
    let q = lensbench_project::Project::from_json(&c.project().to_string()).ok()?;
    let env = lensbench_project::Env::in_dir(base, glass_dirs, None);
    let same = |q: &lensbench_project::Project, s: &System| -> Option<()> {
        let t = lensbench_project::expand(q, &env, &Default::default())
            .ok()?
            .system;
        lensbench_project::same_system(s, &t).ok()
    };
    if cfgs.is_empty() {
        same(&q, s)?;
    } else {
        let names = q.state_names();
        if names.len() != cfgs.len() {
            return None;
        }
        for (n, sk) in names.iter().zip(cfgs) {
            same(&q.with_state(n).ok()?, sk)?;
        }
    }
    Some(c)
}

/// 構成 sk の、構成 1（s0、a はそれを分けたもの）との違いを状態の set（ブロック id → 値）に。状態で変える
/// 空気の前の要素の id を named に足す。書けない違いがあれば None
fn config_set(
    s0: &System,
    sk: &System,
    a: &Parts,
    p: &Value,
    named: &mut BTreeSet<String>,
) -> Option<Map<String, Value>> {
    if sk.surfaces.len() != s0.surfaces.len()
        || sk.solves != s0.solves
        || sk.pickups != s0.pickups
        || sk.macro_parms != s0.macro_parms
        || sk.glrs != s0.glrs
    {
        return None;
    }
    // 構成 k も同じ要素に分かれ、絞りのほかの操作が同じこと
    let b = split_parts(sk, p)?;
    let (ba, bb) = (a.v["blocks"].as_array()?, b.v["blocks"].as_array()?);
    let no_stop = |x: &Value| -> Vec<Value> {
        let ops = x["surface_ops"].as_array().into_iter().flatten();
        ops.filter(|o| o["op"] != "set_stop").cloned().collect()
    };
    if ba.len() != bb.len()
        || ba.iter().zip(bb).any(|(x, y)| {
            x["type"] != y["type"]
                || x["id"] != y["id"]
                || x.get("medium") != y.get("medium")
                || no_stop(x) != no_stop(y)
        })
    {
        return None;
    }
    let mut set = Map::new();
    // 空気（要素の後の面の厚さ）
    let mut in_air = BTreeSet::new();
    for (ai, id, k) in &a.airs {
        in_air.insert(*k);
        if ba[*ai]["t"] != bb[*ai]["t"] {
            named.insert(id.clone());
            set.insert(format!("{id}_air"), json!({ "t": bb[*ai]["t"] }));
        }
    }
    // 面の値（系の面の値。曲率は半径で）
    let mut edits: BTreeMap<String, BTreeMap<usize, Map<String, Value>>> = BTreeMap::new();
    for j in 1..s0.surfaces.len() - 1 {
        let x = lensbench_project::edit_ops::surf_fields_json(s0, j).ok()?;
        let y = lensbench_project::edit_ops::surf_fields_json(sk, j).ok()?;
        for (name, yv) in y.as_object()? {
            let skip = x.get(name) == Some(yv)
                || matches!(name.as_str(), "n" | "c" | "stop")
                || (name == "t" && in_air.contains(&j))
                || (name == "semi" && s0.surfaces[j].semi_auto && sk.surfaces[j].semi_auto)
                || derived(s0, j, name);
            if skip {
                continue;
            }
            let (id, i) = a.face[j].as_ref()?;
            let val = match name.as_str() {
                "mirror" => return None,
                "r" => json!(radius_of(sk.surfaces[j].c)?),
                _ => yv.clone(),
            };
            edits
                .entry(id.clone())
                .or_default()
                .entry(*i)
                .or_default()
                .insert(name.clone(), val);
        }
    }
    for (id, m) in edits {
        let es: Vec<Value> = m
            .into_iter()
            .map(|(i, f)| json!({ "surface": i, "fields": f }))
            .collect();
        set.entry(id).or_insert_with(|| json!({}))["surface_edits"] = json!(es);
    }
    // 絞りの面（移した先のブロックに set_stop、元のブロックは set_stop を除いた操作）
    let stop = |s: &System| s.surfaces.iter().position(|d| d.stop);
    if stop(s0) != stop(sk) {
        let (nb, ni) = a.face.get(stop(sk)?)?.clone()?;
        let ops_of = |id: &str| ba.iter().find(|x| x["id"] == id).map(no_stop);
        let mut ops = ops_of(&nb)?;
        ops.push(json!({ "op": "set_stop", "surface": ni }));
        set.entry(nb.clone()).or_insert_with(|| json!({}))["surface_ops"] = json!(ops);
        if let Some((ob, _)) = stop(s0).and_then(|j| a.face.get(j)?.clone()) {
            if ob != nb {
                set.entry(ob.clone()).or_insert_with(|| json!({}))["surface_ops"] =
                    json!(ops_of(&ob)?);
            }
        }
    }
    Some(set)
}

/// 面 j の値 name が、ソルブ・ピックアップ・マクロで決まるか（構成で違っても状態の値にしない）
fn derived(s: &System, j: usize, name: &str) -> bool {
    let solve = |curv: bool| {
        s.solves
            .iter()
            .any(|q| q.surf == j && q.kind.is_curv() == curv)
    };
    let by_solve = match name {
        "t" => solve(false),
        "r" => solve(true),
        _ => false,
    };
    by_solve
        || s.pickups.iter().any(|q| q.dst == j)
        || (matches!(name, "k" | "semi" | "asph") && s.macro_parms.iter().any(|q| q.surf == j))
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
    split_parts(s, p).map(|x| x.v)
}

/// 分けたプロジェクトと、系の面とブロックの対応
struct Parts {
    v: Value,
    /// 系の面 → (ブロック id, ブロックの面)（座標ブレーク・物体・像は None）
    face: Vec<Option<(String, usize)>>,
    /// 空気のブロックの blocks の位置・その前のブロックの id・厚さを持つ系の面
    airs: Vec<(usize, String, usize)>,
}

fn split_parts(s: &System, p: &Value) -> Option<Parts> {
    if s.surfaces.len() < 3 {
        return None;
    }
    let img = s.surfaces.len() - 1;
    let mut blocks = Vec::new();
    // 系の面 → (ブロック id, ブロックの面)
    let mut face: Vec<Option<(String, usize)>> = vec![None; img + 1];
    let mut stop: Option<(String, usize)> = None;
    let mut airs = Vec::new();
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
        airs.push((blocks.len(), id.clone(), k));
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
    // マクロはプロジェクトの macros（系と同じ並び）を名前で指す。同じ名前のマクロが 2 つあれば分けない
    let mname = |i: u16| -> Option<String> {
        let n = &s.macros.get(i as usize)?.name;
        (s.macros
            .iter()
            .filter(|m| m.name.eq_ignore_ascii_case(n))
            .count()
            == 1)
            .then(|| n.clone())
    };
    for q in &s.solves {
        let (kind, value, param, other) = lensbench_core::solves::solve_kind_info(q.kind);
        let mut o = json!({ "op": "add_solve", "surface": at(q.surf)?, "kind": kind,
            "value": value });
        match q.kind {
            SolveKind::MacroCurv { index } | SolveKind::MacroThick { index } => {
                o["macro"] = json!(mname(index)?)
            }
            _ => o["param"] = json!(param),
        }
        if let Some(x) = other {
            o["other"] = at(x)?;
        }
        ops.push(o);
    }
    // 円錐・半直径・面タイプのパラメータのマクロのソルブ
    for q in &s.macro_parms {
        ops.push(
            json!({ "op": "add_macro_solve", "surface": at(q.surf)?, "macro": mname(q.id)?,
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
    Some(Parts { v, face, airs })
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
        // マクロのファイルを消した後もプロジェクトから開け、ソルブが解ける。ソルブはマクロを名前で指す
        let v = c.project();
        let t = v.to_string();
        assert!(
            t.contains(r#""macro":"LBMIRR""#) && t.contains(r#""macro":"LBCONI""#),
            "{t}"
        );
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

    /// 多重構成の .zmx（ズームのように間隔・曲率・厚さ・材料・波長・絞りの面を構成で変える）は、構成 1 で
    /// 要素に分かれ、構成の違いは状態の値（要素の後の空気・面の上書き・絞りの面・系の値）になる。状態 k は構成 k の系と同じ
    #[test]
    fn a_multi_config_zmx_splits_with_state_values() {
        let tmp = std::env::temp_dir().join(format!("lb_gui_mce_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("zoom.zmx");
        std::fs::write(
            &f,
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 5\n  DIAM 5 0 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 20\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 3\n  DIAM 5 0 0 0 1 \"\"\n\
SURF 5\n  TYPE STANDARD\n  CURV 0.015\n  DISZ 3\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 6\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 40\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 7\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n\
MNUM 2 1\n\
THIC 3 1 20 0 0 0 1 1 1 0 0 \"\" 0\nTHIC 3 2 12 0 0 0 1 1 1 0 0 \"\" 0\n\
THIC 2 1 4 0 0 0 1 1 1 0 0 \"\" 0\nTHIC 2 2 5 0 0 0 1 1 1 0 0 \"\" 0\n\
CRVT 5 1 0.015 0 0 0 1 1 1 0 0 \"\" 0\nCRVT 5 2 0.02 0 0 0 1 1 1 0 0 \"\" 0\n\
GLSS 5 1 N-BK7 0 0 0 1 1 0 0.0 \"\" 0\nGLSS 5 2 F2 0 0 0 1 1 0 0.0 \"\" 0\n\
WAVE 1 1 0.5876 0 0 0 1 1 1 0 0 \"\" 0\nWAVE 1 2 0.6563 0 0 0 1 1 1 0 0 \"\" 0\n\
STPS 0 1 1 0 0 0 1 1 1 0 0 \"\" 0\nSTPS 0 2 4 0 0 0 1 1 1 0 0 \"\" 0\n",
        )
        .unwrap();
        let glass = vec![root().join("tests/glass")];
        let r = Combo::from_zmx(&f, &glass, &[]);
        std::fs::remove_dir_all(&tmp).ok();
        let c = r.unwrap();
        assert!(
            !c.picks
                .iter()
                .any(|p| matches!(p.kind, ElemKind::ZmxRange | ElemKind::Raw)),
            "{:?}",
            c.picks.iter().map(|p| &p.kind).collect::<Vec<_>>()
        );
        assert_eq!(c.states, ["Config 1", "Config 2"]);
        let n = "Config 2";
        // 玉 1 の後の空気・玉 1 の厚さ・玉 2 の曲率と材料は状態の値。絞りは面 S2 へ移る
        let (l1, l2, s2) = (&c.picks[1], &c.picks[3], &c.picks[2]);
        assert_eq!(
            (l1.kind, s2.kind, l2.kind),
            (ElemKind::Lens, ElemKind::Surfaces, ElemKind::Lens)
        );
        assert_eq!(l1.state_air.get(n), Some(&12.0));
        assert_eq!(l1.state_edit(n, 0, "t"), Some(&json!(5.0)));
        assert_eq!(l2.state_edit(n, 0, "r"), Some(&json!(50.0)));
        assert_eq!(l2.state_edit(n, 0, "material"), Some(&json!("F2")));
        assert_eq!(s2.state_stop.get(n), Some(&0));
        assert!(c.state_sys.contains_key(n));
        assert!(!c.state_sys.contains_key("Config 1"));
    }
}
