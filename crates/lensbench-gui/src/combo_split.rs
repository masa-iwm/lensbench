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

#[cfg(test)]
mod back_solve_tests;
mod reversed;

/// 系 s（.zmx を構成 1 で読んだもの）を、p（[`lensbench_project::zmx_project`] の設定）の blocks を要素に分けた
/// 組合せにする。同じ系にならなければ None
pub(crate) fn split(s: &System, p: &Value, base: &Path, glass_dirs: &[PathBuf]) -> Option<Combo> {
    // 鏡で折り返して同じ面を戻る系（ダブルパス）は、往きの要素と「往路を戻る」鏡にまとめてみる。
    // 同じ系にならなければ、今までどおり戻りも別の要素にする
    // 前の玉を裏返した処方の玉は、前の玉の処方の裏返しにしてみる（同じ系にならなければ別の処方のまま）
    [true, false].into_iter().find_map(|dp| {
        let a = split_parts(s, p, dp)?;
        let t = a.compared(s);
        let mut v = a.v.clone();
        let merged = v["blocks"]
            .as_array_mut()
            .is_some_and(|b| reversed::merge_reversed(b));
        merged
            .then(|| finish(v, &t, &[], &a.folded, base, glass_dirs))
            .flatten()
            .or_else(|| finish(a.v.clone(), &t, &[], &a.folded, base, glass_dirs))
    })
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
    // ダブルパスは往きの要素と「往路を戻る」鏡にまとめてみる（[`split`] と同じ）
    [true, false]
        .into_iter()
        .find_map(|dp| split_configs_with(cfgs, p, base, glass_dirs, dp))
}

fn split_configs_with(
    cfgs: &[System],
    p: &Value,
    base: &Path,
    glass_dirs: &[PathBuf],
    dp: bool,
) -> Option<Combo> {
    let s0 = cfgs.first()?;
    let a = split_parts(s0, p, dp)?;
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
    let cfgs: Vec<System> = cfgs.iter().map(|x| a.compared(x)).collect();
    finish(v, &cfgs[0], &cfgs, &a.folded, base, glass_dirs)
}

/// 分けたプロジェクト v を組合せにする。系 s（状態があれば状態ごとの系 cfgs）と同じにならなければ None
fn finish(
    mut v: Value,
    s: &System,
    cfgs: &[System],
    loose: &BTreeSet<usize>,
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
        // 往路を戻る鏡にまとめた戻りの面の名前（コメント）は展開が付ける（.zmx の名前は比べない）
        let mut s = s.clone();
        if !loose.is_empty() {
            s.edit_raw(|rx| {
                for &j in loose {
                    if let (Some(a), Some(b)) = (rx.surfaces.get_mut(j), t.surfaces.get(j)) {
                        a.comment.clone_from(&b.comment);
                    }
                }
                Ok(())
            })
            .ok()?;
        }
        lensbench_project::same_system(&s, &t).ok()
    };
    // 半径で書いた曲率は最後の桁が変わりうるので、元の系もそうしてから比べる
    if cfgs.is_empty() {
        same(&q, &as_written(s)?)?;
    } else {
        let names = q.state_names();
        if names.len() != cfgs.len() {
            return None;
        }
        for (n, sk) in names.iter().zip(cfgs) {
            same(&q.with_state(n).ok()?, &as_written(sk)?)?;
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
    let b = split_parts(sk, p, a.dp)?;
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
    // ブロック id → (座標ブレークか, ブロックの面の番号) → 値
    type Edits = BTreeMap<String, BTreeMap<(bool, usize), Map<String, Value>>>;
    let mut edits: Edits = BTreeMap::new();
    for j in 1..s0.surfaces.len() - 1 {
        let x = lensbench_project::edit_ops::surf_fields_json(s0, j).ok()?;
        let y = lensbench_project::edit_ops::surf_fields_json(sk, j).ok()?;
        for (name, yv) in y.as_object()? {
            let skip = x.get(name) == Some(yv)
                || matches!(name.as_str(), "n" | "c" | "stop")
                || (name == "t" && in_air.contains(&j))
                || (name == "semi" && s0.surfaces[j].semi_auto && sk.surfaces[j].semi_auto)
                // 自動の機械半径（玉の半径）は半直径から決まる
                || (name == "mech"
                    && s0.surfaces[j].mech_semi.is_none()
                    && sk.surfaces[j].mech_semi.is_none())
                || derived(s0, j, name);
            // 往路を戻る鏡にまとめた戻りの面は往きの面から決まる（往きと違う値なら、状態ごとの系の確かめで落ちる）
            // 往路を戻る鏡の厚さも戻りの空気（往きの空気）が受け持つ
            if skip || a.folded.contains(&j) || (name == "t" && Some(j) == a.back_mirror) {
                continue;
            }
            let is_cb = s0.surfaces[j].typ() == SurfType::CoordBrk;
            let (id, i) = if is_cb { &a.cb[j] } else { &a.face[j] }.as_ref()?;
            let val = match name.as_str() {
                "mirror" => return None,
                "r" => json!(radius_of(sk.surfaces[j].c)),
                _ => yv.clone(),
            };
            edits
                .entry(id.clone())
                .or_default()
                .entry((is_cb, *i))
                .or_default()
                .insert(name.clone(), val);
        }
    }
    for (id, m) in edits {
        let es: Vec<Value> = m
            .into_iter()
            .map(|((cb, i), f)| match cb {
                true => json!({ "surface": i, "coordbrk": true, "fields": f }),
                false => json!({ "surface": i, "fields": f }),
            })
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
        || (matches!(name, "k" | "semi" | "mech" | "asph")
            && s.macro_parms.iter().any(|q| q.surf == j))
}

/// 曲率 c の面の半径（平面は 0）。1/半径 が c に戻る値があればそれ。どの f64 の逆数にもならない曲率もあるので
/// （OpticStudio の CURV の 16 桁の値で 4 面に 1 つほど）、無ければ 1/半径 が c に最も近い値（曲率は最後の桁だけ違う）
fn radius_of(c: f64) -> f64 {
    if c == 0.0 {
        return 0.0;
    }
    let mut r = 1.0 / c;
    let mut best = r;
    // 1/(1/c) が c に戻らなければ、隣の値を少し探す
    for _ in 0..8 {
        if 1.0 / r == c {
            return r;
        }
        if (1.0 / r - c).abs() < (1.0 / best - c).abs() {
            best = r;
        }
        r = if 1.0 / r > c {
            r.next_up()
        } else {
            r.next_down()
        };
    }
    best
}

/// 系 s の面の曲率を、半径で書いて読み直した値に（[`radius_of`] で最後の桁が変わる面だけ変わる）。座標ブレークと、
/// ソルブ・ピックアップで決まる曲率はそのまま
fn as_written(s: &System) -> Option<System> {
    let snap: Vec<(usize, f64)> = (1..s.surfaces.len().saturating_sub(1))
        .filter(|&j| s.surfaces[j].typ() != SurfType::CoordBrk && !derived(s, j, "r"))
        .map(|j| (j, 1.0 / radius_of(s.surfaces[j].c)))
        .filter(|&(j, c)| c != s.surfaces[j].c && s.surfaces[j].c != 0.0)
        .collect();
    let mut t = s.clone();
    if snap.is_empty() {
        return Some(t);
    }
    // ソルブ・ピックアップ・自動の半直径は直した曲率で求め直す（展開した系と同じ）
    t.edit(|rx| {
        for (j, c) in snap {
            rx.surfaces[j].c = c;
        }
        Ok(())
    })
    .ok()?;
    Some(t)
}

/// 鏡の面 a（光の進む向き）を鏡のブロックに。標準面で開口・膜・傾きが無ければ半径・コーニックで、ほかは行で
fn mirror_block(a: &lensbench_core::surface::Surface, id: &str) -> Option<Value> {
    use lensbench_core::surface::Aperture;
    let plain = a.typ() == SurfType::Standard
        && matches!(a.ap, Aperture::None)
        && a.coat.is_empty()
        && !a.ignore
        && !a.semi_auto
        && a.before.is_none()
        && a.after == lensbench_core::surface::After::None;
    if plain {
        return Some(
            json!({ "type": "mirror", "id": id, "radius": radius_of(a.c),
            "conic": a.k, "semi": a.semi }),
        );
    }
    let r = lensbench_project::mirror_row(a).ok()?;
    Some(json!({ "type": "mirror", "id": id, "row": r }))
}

/// 分けたプロジェクトと、系の面とブロックの対応
struct Parts {
    v: Value,
    /// 系の面 → (ブロック id, ブロックの面)（座標ブレーク・物体・像は None）
    face: Vec<Option<(String, usize)>>,
    /// 座標ブレークの系の面 → (ブロック id, ブロックの座標ブレークだけを数えた番号)
    cb: Vec<Option<(String, usize)>>,
    /// 空気のブロックの blocks の位置・その前のブロックの id・厚さを持つ系の面
    airs: Vec<(usize, String, usize)>,
    /// ダブルパスを往路を戻る鏡にまとめたか
    dp: bool,
    /// 往路を戻る鏡にまとめて、要素に書かなくなった系の面（戻りの面と、鏡の傾きの座標ブレーク）
    folded: BTreeSet<usize>,
    /// 往路を戻る鏡の面
    back_mirror: Option<usize>,
    /// 玉の置き方にした座標ブレークの面（[`collapse_placed`]）
    placed: Vec<usize>,
}

impl Parts {
    /// 往路を戻る鏡が受け持つピックアップ（戻りの面へのもの、鏡の厚さへのもの）か
    fn drops(&self, q: &lensbench_core::pickup::Pickup) -> bool {
        self.folded.contains(&q.dst)
            || (Some(q.dst) == self.back_mirror && q.what == lensbench_core::param::Kind::Thick)
    }

    /// 組合せの系と比べる元の系: 往路を戻る鏡が受け持つピックアップを除いたもの（値はそのまま）
    fn compared(&self, s: &System) -> System {
        let mut t = s.clone();
        let cbs: Vec<usize> = self
            .folded
            .iter()
            .chain(&self.placed)
            .copied()
            .filter(|&j| s.surfaces[j].typ() == SurfType::CoordBrk)
            .collect();
        if s.pickups.iter().any(|q| self.drops(q)) || !cbs.is_empty() {
            // 値はピックアップを解いたまま。鏡の傾きにした座標ブレークの半直径は追跡に効かない（鏡の傾きの座標ブレークは 0）。
            // 除く・半直径を変えるだけなので失敗しない（失敗したら元のまま比べ、違うと出る）
            let _ = t.edit(|rx| {
                rx.pickups.retain(|q| !self.drops(q));
                for &j in &cbs {
                    rx.surfaces[j].semi = 0.0;
                    rx.surfaces[j].semi_auto = false;
                }
                Ok(())
            });
        }
        t
    }
}

/// 面を要素に分ける。`dp` なら、ダブルパス（鏡の後の要素が、鏡の前の要素を逆の順にたどる）を、往きの要素と
/// 往路を戻る鏡（`back`）にまとめる。ダブルパスの形でなければ None
fn split_parts(s: &System, p: &Value, dp: bool) -> Option<Parts> {
    if s.surfaces.len() < 3 {
        return None;
    }
    let img = s.surfaces.len() - 1;
    let mut blocks = Vec::new();
    // ブロックの系の面の範囲（空気は None）
    let mut ranges: Vec<Option<(usize, usize)>> = Vec::new();
    // 系の面 → (ブロック id, ブロックの面)
    let mut face: Vec<Option<(String, usize)>> = vec![None; img + 1];
    let mut cb: Vec<Option<(String, usize)>> = vec![None; img + 1];
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
            x["radius"] = json!(radius_of(a.c));
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
        let (mut i, mut ci) = (0, 0);
        for x in j..=k {
            if s.surfaces[x].typ() == SurfType::CoordBrk {
                if s.surfaces[x].stop {
                    return None;
                }
                cb[x] = Some((id.clone(), ci));
                ci += 1;
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
        ranges.push(Some((j, k)));
        ranges.push(None);
        airs.push((blocks.len(), id.clone(), k));
        match fill.take() {
            Some(m) => blocks.push(json!({ "type": "air", "t": t, "medium": m })),
            None => blocks.push(json!({ "type": "air", "t": t })),
        }
        j = k + 1;
    }
    let mut folded = BTreeSet::new();
    // 往路を戻る鏡の面（その厚さは戻りの空気が受け持つ）
    let mut back_mirror = None;
    let mut placed = Vec::new();
    // 往路を戻る鏡にまとめた戻りの面 → 同じ往きの面（戻りの面のソルブはブロックの面の back で書く）
    let mut back_of = BTreeMap::new();
    if dp {
        // 座標ブレークで挟んだ玉は玉の置き方に（往路を戻る鏡は置き方ごと戻る）
        placed = collapse_placed(s, &mut blocks, &mut ranges, &mut airs)?;
        for &x in &placed {
            cb[x] = None;
        }
        let (f, m, b) = fold_double_pass(s, &mut blocks, &ranges, &mut airs)?;
        back_of = b;
        back_mirror = Some(m);
        for &x in &f {
            face[x] = None;
            cb[x] = None;
        }
        if f.iter().any(|&x| s.surfaces[x].stop) || f.contains(&s.glrs) {
            return None;
        }
        folded = f;
    }
    let at = |j: usize| -> Option<Value> {
        if let Some(&y) = back_of.get(&j) {
            let (b, i) = face.get(y)?.as_ref()?;
            return Some(json!({ "block": b, "surface": i, "back": true }));
        }
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
    // 戻りの面のソルブは往きの面の back で書く。戻りの面のマクロは書けない（まとめない）。戻りの面へのピックアップは
    // 要らなくなる
    if s.macro_parms.iter().any(|q| folded.contains(&q.surf)) {
        return None;
    }
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
    let drop = |q: &lensbench_core::pickup::Pickup| {
        folded.contains(&q.dst)
            || (Some(q.dst) == back_mirror && q.what == lensbench_core::param::Kind::Thick)
    };
    for q in &s.pickups {
        if drop(q) {
            continue;
        }
        let name = lensbench_core::pickup::pick_kind_name;
        // 座標ブレークの値のピックアップは座標ブレークの番号で指す
        let any = |j: usize| -> Option<Value> {
            at(j).or_else(|| {
                let (b, i) = cb.get(j)?.as_ref()?;
                Some(json!({ "block": b, "surface": i, "coordbrk": true }))
            })
        };
        let mut o = json!({ "op": "add_pickup", "dst": any(q.dst)?, "src": any(q.src)?,
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
        // 座標ブレーク・戻りの面はブロックの面の形のまま（ブロックの面の番号は座標ブレークを数えない）
        if o[key]["coordbrk"] != true && o[key]["back"] != true {
            o[key] = o[key]["surface"].clone();
        }
        let blk = blocks.iter_mut().find(|x| x["id"] == b.as_str())?;
        match blk["surface_ops"].as_array_mut() {
            Some(a) => a.push(o),
            None => blk["surface_ops"] = json!([o]),
        }
    }
    // 固定の機械半径（.zmx の MEMA の固定・多重構成の MCSD）とチップゾーン（CHZN）は面の欄に無いので、ブロックの面の
    // 上書きに（GUI の表の機械半径・チップゾーンの列と同じ）
    for (j, d) in s.surfaces.iter().enumerate().take(img).skip(1) {
        let mut f = Map::new();
        if let Some(m) = d.mech_semi {
            f.insert("mech".into(), json!(m));
        }
        if d.chip_zone != 0.0 {
            f.insert("chip".into(), json!(d.chip_zone));
        }
        if f.is_empty() {
            continue;
        }
        let (b, i) = face.get(j)?.as_ref()?;
        let blk = blocks.iter_mut().find(|x| x["id"] == b.as_str())?;
        let e = json!({ "surface": i, "fields": f });
        match blk["surface_edits"].as_array_mut() {
            Some(a) => a.push(e),
            None => blk["surface_edits"] = json!([e]),
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
    Some(Parts {
        v,
        face,
        cb,
        airs,
        dp,
        folded,
        back_mirror,
        placed,
    })
}

/// 座標ブレークで挟んだ玉（偏心・傾きの座標ブレーク（順序 0、厚さ 0）・空気 0・玉・空気 0・戻しの座標ブレーク（順序 1）・空気）を、
/// 置き方（`decenter`・`tilt`）つきの玉と空気にする（展開の置き方と同じ形）。鏡の前の玉は置き方を座標ブレークから読み、
/// 戻しの座標ブレークの厚さから戻りの分を引いた空気にする。鏡の後の玉（戻り）は形だけ玉にする（往路を戻る鏡にまとめれば
/// 消え、まとめられなければ None）。値まで同じかは、呼び手が組合せの系と元の系を比べて確かめる。玉にした座標ブレークの面を返す
fn collapse_placed(
    s: &System,
    blocks: &mut Vec<Value>,
    ranges: &mut Vec<Option<(usize, usize)>>,
    airs: &mut Vec<(usize, String, usize)>,
) -> Option<Vec<usize>> {
    let air_t = |b: &[Value], i: usize| b.get(i).filter(|x| x["type"] == "air")?["t"].as_f64();
    let mut done = Vec::new();
    let mut i = 0;
    while i + 5 < blocks.len() {
        let cb_at = |i: usize| -> Option<usize> {
            let (j, k) = ranges.get(i).copied().flatten()?;
            (j == k && blocks[i]["type"] == "surfaces" && s.surfaces[j].typ() == SurfType::CoordBrk)
                .then_some(j)
        };
        let (Some(c1), Some(c2)) = (cb_at(i), cb_at(i + 4)) else {
            i += 1;
            continue;
        };
        let (a, b) = (&s.surfaces[c1], &s.surfaces[c2]);
        if blocks[i + 2]["type"] != "lens"
            || a.cb_order() != 0
            || b.cb_order() != 1
            || air_t(blocks, i + 1) != Some(0.0)
            || air_t(blocks, i + 3) != Some(0.0)
        {
            i += 1;
            continue;
        }
        let (lj, lk) = ranges[i + 2]?;
        let mut t = air_t(blocks, i + 5)?;
        let mut lens = blocks[i + 2].clone();
        if blocks
            .iter()
            .position(|b| b["type"] == "mirror")
            .is_none_or(|m| i < m)
        {
            // 往き: 置き方を読む。戻しの座標ブレークは傾きを戻し、偏心は玉の長さ l の分（置き方の展開と同じ）
            let [dx, dy, tx, ty, tz] = a.cb();
            let [dx2, dy2, ux, uy, uz] = b.cb();
            if [ux, uy, uz] != [-tx, -ty, -tz] {
                return None;
            }
            let l: f64 = (lj..lk).map(|x| s.surfaces[x].t).sum();
            let (rx, ry) = (-dx - dx2, -dy - dy2);
            let rz = (l * l - rx * rx - ry * ry).max(0.0).sqrt() * l.signum();
            t -= l - rz;
            lens["decenter"] = json!([dx, dy]);
            lens["tilt"] = json!([tx, ty, tz]);
        }
        let id = lens["id"].clone();
        blocks.splice(i..i + 6, [lens, json!({ "type": "air", "t": t })]);
        ranges.splice(i..i + 6, [Some((c1, c2)), None]);
        // 空気の位置: 座標ブレークの後・玉の後の空気は消え、戻しの後の空気が玉の後の空気になる
        let mut na = Vec::new();
        for (ai, aid, k) in airs.drain(..) {
            match ai {
                x if x < i => na.push((x, aid, k)),
                x if x == i + 5 => na.push((i + 1, id.as_str()?.to_string(), k)),
                x if x > i + 5 => na.push((x - 4, aid, k)),
                _ => {}
            }
        }
        *airs = na;
        done.extend([c1, c2]);
        i += 2;
    }
    Some(done)
}

/// ダブルパスの blocks（`ranges` はブロックの系の面の範囲、空気は None）を、往きの要素と往路を戻る鏡（`back`）に書き換える。
/// 形: 往きの要素と空気 E₀ A₀ … Eₙ₋₁ Aₙ₋₁、鏡の核（鏡 M、または傾きの座標ブレーク C₁ 空気(0) M 空気(0) C₂）、
/// 鏡の後の空気、戻りの要素 Eₙ₋₁′ … E₀′（往きと同じ種類・面の数で逆の順、間に空気）、その後ろ。
/// 戻りの要素と鏡の後の空気を除き、E₀′ の後の空気からを鏡の後に続ける。要素に書かなくなった系の面と、鏡の面を返す。
/// 値まで同じかは、呼び手が組合せの系と元の系を比べて確かめる
fn fold_double_pass(
    s: &System,
    blocks: &mut Vec<Value>,
    ranges: &[Option<(usize, usize)>],
    airs: &mut Vec<(usize, String, usize)>,
) -> Option<(BTreeSet<usize>, usize, BTreeMap<usize, usize>)> {
    let bm = blocks.iter().position(|b| b["type"] == "mirror")?;
    let is_air = |i: usize| blocks.get(i).is_some_and(|b| b["type"] == "air");
    let air_t = |i: usize| blocks.get(i).and_then(|b| b["t"].as_f64());
    // 鏡の傾き: 鏡の前後の、傾きだけの座標ブレーク（前は順序 0 で (tx, ty)、後は順序 1 で (−tx, −ty)）
    let cb_of = |i: usize| -> Option<([f64; 5], u8, f64)> {
        let (j, k) = ranges.get(i).copied().flatten()?;
        let d = &s.surfaces[j];
        (j == k && blocks[i]["type"] == "surfaces" && d.typ() == SurfType::CoordBrk)
            .then(|| (d.cb(), d.cb_order(), d.t))
    };
    let (bf, bc, tilt) = match (bm.checked_sub(2).and_then(cb_of), cb_of(bm + 2)) {
        (Some((c1, 0, _)), Some((c2, 1, _)))
            if c1[0] == 0.0
                && c1[1] == 0.0
                && c1[4] == 0.0
                && c2[0] == 0.0
                && c2[1] == 0.0
                && c2[4] == 0.0
                && c2[2] == -c1[2]
                && c2[3] == -c1[3]
                && air_t(bm - 1) == Some(0.0)
                && air_t(bm + 1) == Some(0.0) =>
        {
            (bm - 2, bm + 2, Some([c1[2], c1[3]]))
        }
        _ => (bm, bm, None),
    };
    // 往きの要素（空気でないブロック）。間は空気が 1 つずつ
    let fwd: Vec<usize> = (0..bf).filter(|&i| !is_air(i)).collect();
    let n = fwd.len();
    if n == 0
        || bf != 2 * n
        || fwd
            .iter()
            .any(|&i| !is_air(i + 1) || blocks[i]["type"] == "mirror")
        || !is_air(bc + 1)
    {
        return None;
    }
    // 戻りの要素: 往きと逆の順で、同じ種類・同じ面の数。戻りの面 → 同じ往きの面（逆の順にたどる）
    let nsurf = |i: usize| ranges[i].map(|(j, k)| k - j);
    let mut back_of = BTreeMap::new();
    for (q, &e) in fwd.iter().rev().enumerate() {
        let r = bc + 2 + 2 * q;
        if !is_air(r + 1) || blocks[r]["type"] != blocks[e]["type"] || nsurf(r) != nsurf(e) {
            return None;
        }
        if let (Some((jr, kr)), Some((_, ke))) = (ranges[r], ranges[e]) {
            back_of.extend((jr..=kr).map(|x| (x, ke - (x - jr))));
        }
    }
    let tail = bc + 2 * n + 1;
    // 要素に書かなくなる系の面: 戻りの要素の面と、鏡の傾きの座標ブレーク
    let mut folded = BTreeSet::new();
    for i in (bf..=bc).chain(bc + 2..tail) {
        if i == bm {
            continue;
        }
        if let Some((j, k)) = ranges[i] {
            folded.extend(j..=k);
        }
    }
    let mut m = blocks[bm].clone();
    m["back"] = json!(true);
    if let Some(t) = tilt {
        m["tilt"] = json!(t);
    }
    let mid = m["id"].as_str()?.to_string();
    let mut out: Vec<Value> = blocks[..bf].to_vec();
    out.push(m);
    out.extend(blocks[tail..].iter().cloned());
    // 空気の位置: 往きの空気はそのまま、E₀′ の後の空気からは鏡の後へ詰める（E₀′ の後の空気の前の要素は鏡）
    let shift = tail - bf - 1;
    let mut na = Vec::new();
    for (ai, id, k) in airs.drain(..) {
        if ai < bf {
            na.push((ai, id, k));
        } else if ai == tail {
            na.push((ai - shift, mid.clone(), k));
        } else if ai > tail {
            na.push((ai - shift, id, k));
        }
    }
    *airs = na;
    *blocks = out;
    Some((folded, ranges[bm]?.0, back_of))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combo::SurfKind;
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
STPS 0 1 1 0 0 0 1 1 1 0 0 \"\" 0\nSTPS 0 2 4 0 0 0 1 1 1 0 0 \"\" 0\n\
MCOM 0 1 \"wide\" 0 0 0 1 1 0 0.0 \"\" 0\nMCOM 0 2 \"tele  end\" 0 0 0 1 1 0 0.0 \"\" 0\n\
LTTL 0 1 \"m=-1.1000 D=7.9\" 0 0 0 1 1 0 0.0 \"\" 0\nLTTL 0 2 Tele 0 0 0 1 1 0 0.0 \"\" 0\n",
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
        // LTTL は構成（状態）の名前
        assert_eq!(c.states, ["m=-1.1000 D=7.9", "Tele"]);
        let n = "Tele";
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
        assert!(!c.state_sys.contains_key("m=-1.1000 D=7.9"));
        // MCOM はコメント行（構成ごとの文。空白を含む文も）
        assert_eq!(c.mc_comments.len(), 1);
        assert_eq!(c.mc_comments[0].values["m=-1.1000 D=7.9"], "wide");
        assert_eq!(c.mc_comments[0].values["Tele"], "tele  end");
    }

    /// 鏡の後の面（裏返して書く面）を構成で変える多重構成の .zmx も要素に分かれ、鏡の後の間隔・面の値が状態の値になる
    /// （系の面の値は鏡の後で符号が逆）。状態 k は構成 k の系と同じ（split_configs の中で確かめる）
    #[test]
    fn a_multi_config_zmx_after_a_mirror_splits() {
        let tmp = std::env::temp_dir().join(format!("lb_gui_mce_m_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("fold.zmx");
        std::fs::write(
            &f,
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n  DIAM 5 0 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 30\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV -0.005\n  DISZ -20\n  GLAS MIRROR 0 0 0 0 0 0 0 0 0\n  DIAM 10 0 0 0 1 \"\"\n\
SURF 4\n  TYPE STANDARD\n  CURV -0.02\n  DISZ -4\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 5\n  TYPE STANDARD\n  CURV 0.01\n  DISZ -30\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 6\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n\
MNUM 2 1\n\
THIC 3 1 -20 0 0 0 1 1 1 0 0 \"\" 0\nTHIC 3 2 -15 0 0 0 1 1 1 0 0 \"\" 0\n\
CRVT 4 1 -0.02 0 0 0 1 1 1 0 0 \"\" 0\nCRVT 4 2 -0.025 0 0 0 1 1 1 0 0 \"\" 0\n\
THIC 5 1 -30 0 0 0 1 1 1 0 0 \"\" 0\nTHIC 5 2 -35 0 0 0 1 1 1 0 0 \"\" 0\n",
        )
        .unwrap();
        let glass = vec![root().join("tests/glass")];
        let r = Combo::from_zmx(&f, &glass, &[]);
        std::fs::remove_dir_all(&tmp).ok();
        let c = r.unwrap();
        let kinds: Vec<ElemKind> = c.picks.iter().map(|p| p.kind).collect();
        assert!(
            !kinds
                .iter()
                .any(|k| matches!(k, ElemKind::ZmxRange | ElemKind::Raw)),
            "{kinds:?}"
        );
        let n = "Config 2";
        let m = c
            .picks
            .iter()
            .position(|p| p.kind == ElemKind::Mirror)
            .unwrap();
        // 鏡の後の空気は光の進む向きで正、鏡の後の玉の面の値は系の値（符号は系のまま）
        assert_eq!(c.picks[m].state_air.get(n), Some(&15.0));
        let l = &c.picks[m + 1];
        assert_eq!(l.kind, ElemKind::Lens);
        assert_eq!(l.state_air.get(n), Some(&35.0));
        assert_eq!(l.state_edit(n, 0, "r"), Some(&json!(-40.0)));
    }

    /// 座標ブレークの値のピックアップ（P0 の傾けて戻す座標ブレーク）は、座標ブレークの番号で指す操作にして要素に分ける
    #[test]
    fn coordinate_break_pickups_split() {
        let glass = vec![root().join("tests/glass")];
        for n in ["expanded", "template"] {
            let f = root().join(format!("tests/projects/P0/{n}.zmx"));
            let c = Combo::from_zmx(&f, &glass, &[]).unwrap();
            assert!(
                !c.picks
                    .iter()
                    .any(|p| matches!(p.kind, ElemKind::ZmxRange | ElemKind::Raw)),
                "{n}"
            );
            let cb_pick = c.picks.iter().flat_map(|p| &p.ops).any(|o| {
                matches!(o, SurfaceOp::AddPickup(q)
                    if matches!(&q.dst, BlockRef::Block(b) if b.coordbrk)
                        && matches!(&q.src, BlockRef::Block(b) if b.coordbrk))
            });
            assert!(cb_pick, "{n}");
        }
    }

    /// 開口（浮動の開口も）のある座標ブレーク（玉の中も）は要素に分ける
    #[test]
    fn a_coordinate_break_with_an_aperture_splits() {
        let tmp = std::env::temp_dir().join(format!("lb_gui_cbap_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("cb.zmx");
        std::fs::write(
            &f,
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n  DIAM 5 0 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 3\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 3\n  TYPE COORDBRK\n  PARM 1 0\n  PARM 2 0.1\n  PARM 3 0\n  PARM 4 0\n  PARM 5 0\n  PARM 6 0\n  DISZ 2\n  SQAP 3 3 0 0\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 20\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 5\n  TYPE COORDBRK\n  PARM 1 0\n  PARM 2 0\n  PARM 3 0\n  PARM 4 0\n  PARM 5 0\n  PARM 6 0\n  DISZ 30\n  FLAP 0 4 0\n\
SURF 6\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        )
        .unwrap();
        let glass = vec![root().join("tests/glass")];
        let r = Combo::from_zmx(&f, &glass, &[]);
        std::fs::remove_dir_all(&tmp).ok();
        let mut c = r.unwrap();
        let kinds: Vec<ElemKind> = c.picks.iter().map(|p| p.kind).collect();
        assert!(
            !kinds
                .iter()
                .any(|k| matches!(k, ElemKind::ZmxRange | ElemKind::Raw)),
            "{kinds:?}"
        );
        // 玉の中の座標ブレークは座標ブレークの行（開口を持つ）で、面に数えない。その後の面を変えると系のその面が変わる
        let l = c
            .picks
            .iter()
            .position(|p| p.kind == ElemKind::Lens)
            .unwrap();
        let rows: Vec<SurfKind> = c.picks[l].surfs.iter().map(|r| r.kind).collect();
        assert_eq!(
            rows,
            [SurfKind::Standard, SurfKind::Coordbrk, SurfKind::Standard],
            "{rows:?}"
        );
        assert!(c.picks[l].surfs[1].more.contains_key("aperture"));
        c.picks[l].write_face(1, "r", json!(-80.0));
        let q = lensbench_project::Project::from_json(&c.project().to_string()).unwrap();
        let s = lensbench_project::expand(
            &q,
            &lensbench_project::Env::in_dir(&root(), &glass, None),
            &Default::default(),
        )
        .unwrap()
        .system;
        assert_eq!(s.surfaces[4].c, -1.0 / 80.0);
        assert_eq!(s.surfaces[2].c, 0.02);
        assert!(s.surfaces[3].ap != lensbench_core::surface::Aperture::None);
        // 空気中の座標ブレークの浮動の開口もそのまま
        assert!(matches!(
            s.surfaces[5].ap,
            lensbench_core::surface::Aperture::Floating { .. }
        ));
    }

    /// どの f64 の逆数にもならない曲率（1/半径 で戻らない）は最も近い半径に。曲率は最後の桁だけ違う
    #[test]
    fn a_curvature_with_no_exact_radius_takes_the_nearest() {
        let c = 6.5564346019e-3;
        let r = radius_of(c);
        assert!((1.0 / r - c).abs() <= c * f64::EPSILON, "{r}");
        for r2 in [r.next_up(), r.next_down()] {
            assert!((1.0 / r2 - c).abs() >= (1.0 / r - c).abs());
        }
        assert_eq!(1.0 / radius_of(0.02), 0.02);
        assert_eq!(radius_of(0.0), 0.0);
    }

    /// 機械半径の行（MCSD）・チップゾーンの行（CHZN）のある多重構成の .zmx も要素に分かれ、構成 1 の値はブロックの
    /// 面の上書き、構成 2 の違いは状態の値になる。1/半径 で戻らない曲率の面があっても分かれる
    #[test]
    fn a_multi_config_zmx_with_mech_and_chip_rows_splits() {
        let tmp = std::env::temp_dir().join(format!("lb_gui_mcsd_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("mech.zmx");
        std::fs::write(
            &f,
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 5\n  DIAM 5 0 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV 6.556434601900000200E-03\n  DISZ 4\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 20\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n\
MNUM 2 1\n\
MCSD 2 1 9 0 0 0 1 1 1 0 0 \"\" 0\nMCSD 2 2 10 0 0 0 1 1 1 0 0 \"\" 0\n\
CHZN 3 1 0 0 0 0 1 1 1 0 0 \"\" 0\nCHZN 3 2 0.5 0 0 0 1 1 1 0 0 \"\" 0\n",
        )
        .unwrap();
        let glass = vec![root().join("tests/glass")];
        let r = Combo::from_zmx(&f, &glass, &[]);
        std::fs::remove_dir_all(&tmp).ok();
        let c = r.unwrap();
        let kinds: Vec<ElemKind> = c.picks.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [ElemKind::Surfaces, ElemKind::Lens], "{kinds:?}");
        let l = &c.picks[1];
        assert_eq!(l.base_edit(0, "mech"), Some(&json!(9.0)));
        assert_eq!(l.base_edit(1, "chip"), None);
        let n = &c.states[1];
        assert_eq!(l.state_edit(n, 0, "mech"), Some(&json!(10.0)));
        assert_eq!(l.state_edit(n, 1, "chip"), Some(&json!(0.5)));
    }

    /// ダブルパスの .zmx（鏡で折り返して同じ玉を戻る）は、往きの玉と「往路を戻る」鏡の組合せになる（split の中で元の系と
    /// 同じことも確かめる）。鏡の傾き（前後の座標ブレーク）は鏡の傾きに、戻りの面へのピックアップは消える。
    /// 座標ブレークで挟んだ玉は置き方つきの玉に。戻りが往きと違う系は今までどおり別の要素
    #[test]
    fn a_double_pass_zmx_splits_into_the_forward_lens_and_a_returning_mirror() {
        let glass = vec![root().join("tests/glass")];
        let open = |f: &str| {
            Combo::from_zmx(&root().join("tests/double_pass").join(f), &glass, &[]).unwrap()
        };
        let kinds = |c: &Combo| c.picks.iter().map(|p| p.kind).collect::<Vec<_>>();
        let mirror = |c: &Combo| {
            c.picks
                .iter()
                .find(|p| p.kind == ElemKind::Mirror)
                .cloned()
                .unwrap()
        };
        for (f, tilt) in [
            ("P11_flat_double_pass.zmx", [0.0, 0.0]),
            ("pickups.zmx", [0.0, 0.0]),
            ("P12_tilted_flat_double_pass.zmx", [0.5, -0.3]),
            ("P14_tilted_spherical_double_pass.zmx", [-0.4, 0.6]),
        ] {
            let c = open(f);
            assert_eq!(kinds(&c), [ElemKind::Lens, ElemKind::Mirror], "{f}");
            let m = mirror(&c);
            assert!(m.mirror().back, "{f}");
            assert_eq!([m.tilt[0], m.tilt[1]], tilt, "{f}");
            // 戻りの面へのピックアップは往路を戻る鏡が受け持つので、操作に残らない
            assert!(
                !c.picks
                    .iter()
                    .any(|p| p.ops.iter().any(|o| matches!(o, SurfaceOp::AddPickup(_)))),
                "{f}"
            );
        }
        // 戻りの玉の曲率が往きと違う: まとめず、戻りの玉も別の要素
        let c = open("return_differs.zmx");
        assert_eq!(
            kinds(&c),
            [ElemKind::Lens, ElemKind::Mirror, ElemKind::Lens],
            "{:?}",
            kinds(&c)
        );
        assert!(!mirror(&c).mirror().back);
        // 偏心・傾けた玉（座標ブレークで挟む）: 置き方つきの往きの玉と往路を戻る鏡（戻りは置き方ごと戻る）
        let c = open("P4_double_pass.zmx");
        // 絞りは空気中の面（面の並び）
        assert_eq!(
            kinds(&c),
            [ElemKind::Surfaces, ElemKind::Lens, ElemKind::Mirror]
        );
        assert!(mirror(&c).mirror().back);
        assert_eq!(c.picks[1].tilt, [1.5, -0.7, 3.0]);
        assert_eq!(c.picks[1].decenter, [0.2, -0.1]);
    }

    /// 公差: 往路を戻る鏡にまとめた系では玉は往復で 1 つで、玉の公差の組が往きと戻りに同じ誤差を入れる（曲率・厚さ・
    /// 偏心・傾き。中身は lensbench-design の double_pass_lens_errors_on_both_passes）。別の要素＋ピックアップで開くと戻りの
    /// 玉が別の玉になり、偏心・傾き・くさびは往きと別の誤差になる（ピックアップは面の値しか戻りへ写さない）
    #[test]
    fn a_folded_double_pass_tolerances_both_passes_as_one_lens() {
        let glass = vec![root().join("tests/glass")];
        let labels = |v: &Value| -> Vec<String> {
            let q = lensbench_project::Project::from_json(&v.to_string()).unwrap();
            let env =
                lensbench_project::Env::in_dir(&root().join("tests/double_pass"), &glass, None);
            let sys = lensbench_project::expand(&q, &env, &Default::default())
                .unwrap()
                .system;
            let m = lensbench_design::tolerance::spec_from_json(
                &json!({}),
                vec![sys],
                &Default::default(),
            )
            .unwrap();
            m.tols.into_iter().map(|t| t.label).collect()
        };
        for f in ["pickups.zmx", "P4_double_pass.zmx"] {
            let path = root().join("tests/double_pass").join(f);
            let s = lensbench_formats::zmx::load_with(
                &path,
                &lensbench_formats::zmx::LoadOptions {
                    catalog_dirs: glass.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
            let p = lensbench_project::zmx_project(std::slice::from_ref(&s), f).unwrap();
            let lenses = |dp: bool| -> BTreeSet<String> {
                let a = split_parts(&s, &p, dp).unwrap();
                labels(&a.v)
                    .iter()
                    .filter_map(|l| l.split(' ').next().map(str::to_string))
                    .collect()
            };
            // まとめた系: 往きの玉 L1 だけ。戻りの面にも同じ誤差が入る
            assert_eq!(lenses(true), BTreeSet::from(["L1".to_string()]), "{f}");
            // 別の要素で開いた系: 戻りの玉が別の玉になる（往きと別に引く）
            assert_eq!(lenses(false).len(), 2, "{f}: {:?}", lenses(false));
        }
    }

    /// 多重構成のダブルパス: 鏡までの空気を構成で変えると、往きの空気の状態の値になり（戻りは往路を戻る鏡が受け持つ）、
    /// 戻りの面の値・鏡の厚さは状態の値にしない
    #[test]
    fn a_multi_config_double_pass_zmx_splits_with_a_returning_mirror() {
        let tmp = std::env::temp_dir().join(format!("lb_gui_mce_dp_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("dp.zmx");
        let src =
            std::fs::read_to_string(root().join("tests/double_pass/P11_flat_double_pass.zmx"))
                .unwrap();
        std::fs::write(
            &f,
            format!(
                "{src}\nMNUM 2 1\n\
THIC 3 1 20 0 0 0 1 1 1 0 0 \"\" 0\nTHIC 3 2 25 0 0 0 1 1 1 0 0 \"\" 0\n\
THIC 4 1 -20 0 0 0 1 1 1 0 0 \"\" 0\nTHIC 4 2 -25 0 0 0 1 1 1 0 0 \"\" 0\n"
            ),
        )
        .unwrap();
        let glass = vec![root().join("tests/glass")];
        let r = Combo::from_zmx(&f, &glass, &[]);
        std::fs::remove_dir_all(&tmp).ok();
        let c = r.unwrap();
        let kinds: Vec<ElemKind> = c.picks.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [ElemKind::Lens, ElemKind::Mirror], "{kinds:?}");
        assert!(c.picks[1].mirror().back);
        assert_eq!(c.picks[0].state_air.get("Config 2"), Some(&25.0));
        assert_eq!(c.picks[1].state_air.get("Config 2"), None);
    }
}
