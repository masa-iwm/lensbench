//! 構成ごとに BS の片方の腕を無視して両腕を書き分けた .zmx（往復は戻りの面を書き並べたもの）を、共通の先頭・
//! キューブの BS・腕（往路を戻るミラー）・後ろのブロックと、構成ごとの光束にまとめたプロジェクトに読み替える。
//! 戻りの面は書かず、ミラーの `back` で往路を戻る。読み替えた光束の系が元の構成と同じかは呼び手が光線で確かめる

use super::*;
use crate::zmx_wrap::checked;
use lensbench_core::surface::{After, Tilt};
use serde_json::json;

/// [`zmx_beams`] の BS のブロックの id
pub const BS_ID: &str = "BS";

/// BS の分ける面での働き（座標ブレーク・面・座標ブレークの 3 面）
#[derive(Debug, Clone, Copy, PartialEq)]
enum Slant {
    T,
    R,
}

/// 構成の中ほど（BS の入口の面と出口の面の間）の形
struct Middle {
    /// 往きの分ける面の働き
    out: Slant,
    /// 往きの分ける面の傾き（座標ブレークの x・y の傾き）
    angle: [f64; 2],
    /// 分ける面の膜
    coat: String,
    /// 腕の口の面（キューブの B・C 面）
    port: usize,
    /// 腕の往きの面（口の面の次からミラーの核の前まで）
    arm: std::ops::Range<usize>,
    /// ミラーの面と、前後の座標ブレーク（偏心・傾き、後ろはその逆）
    mirror: usize,
    tilt: Option<Tilt>,
    /// 戻りの分ける面の働き
    back: Slant,
}

/// 構成ごとの系 cfgs（cfgs[k - 1] が構成 k、`path` は .zmx の名前）を、BS と往路を戻るミラーのプロジェクトにする。
/// 構成は同じ面の並びで、無視する面（と絞り・厚さ）だけが違い、どの構成も共通の先頭 → キューブの入口の面 →
/// 分ける面（座標ブレーク・面・座標ブレーク）→ 腕（往路の面 → ミラー → 戻りの面）→ 分ける面 → キューブの出口の面 →
/// 共通の後ろ、をたどること。構成 k は光束 `C{k}`（経路は往きと戻りの分ける面の T・R、状態 `Config k`）になる。
/// 形が違えば誤り
///
/// env はガラス・膜を引く環境: 読み替えたプロジェクトの光束を展開し、元の構成と光線（各視野の瞳の 9 点、主波長）の
/// 像面の位置・光路長・けられが合うかを確かめる（戻りの面が往路を戻らない .zmx は誤り）
pub fn zmx_beams(cfgs: &[System], path: &str, env: &Env) -> lensbench_core::Result<Value> {
    let [s0, ..] = cfgs else {
        return Err("no system".into());
    };
    if cfgs.len() < 2 {
        return Err("two or more configurations are needed to read the arms of a BS".into());
    }
    let n = s0.surfaces.len();
    for (k, s) in cfgs.iter().enumerate().skip(1) {
        if s.surfaces.len() != n {
            return Err(format!(
                "configuration {}: {} surfaces (configuration 1 has {n})",
                k + 1,
                s.surfaces.len()
            )
            .into());
        }
        for j in 0..n {
            if !same_but_path(&s0.surfaces[j], &s.surfaces[j])? {
                return Err(format!(
                    "configuration {}: surface {j} differs in more than ignore, stop and thickness",
                    k + 1
                )
                .into());
            }
        }
    }
    let image = n - 1;
    let active: Vec<Vec<usize>> = cfgs
        .iter()
        .map(|s| (1..image).filter(|&j| !s.surfaces[j].ignore).collect())
        .collect();
    // 共通の先頭・後ろ（どの構成でも通る面）。先頭の後ろの座標ブレークは分ける面のもの
    let mut pre = (0..)
        .take_while(|&i| {
            active
                .iter()
                .all(|a| i < a.len() && a.get(i) == active[0].get(i))
        })
        .count();
    while pre > 0 && s0.surfaces[active[0][pre - 1]].is_coord_break() {
        pre -= 1;
    }
    // 後ろは同じ面か、構成ごとに書き並べた同じ値の面（`zmx --beam-paths` の書き出し）
    let same_at = |k: usize, i: usize| -> lensbench_core::Result<bool> {
        let (a, a0) = (&active[k], &active[0]);
        if i >= a.len() - pre || i >= a0.len() - pre {
            return Ok(false);
        }
        let (j, j0) = (a[a.len() - 1 - i], a0[a0.len() - 1 - i]);
        let (x, x0) = (&cfgs[k].surfaces[j], &s0.surfaces[j0]);
        Ok(j == j0 || (same_but_path(x, x0)? && x.t == x0.t && x.stop == x0.stop))
    };
    let mut post = 0;
    'post: loop {
        for k in 0..cfgs.len() {
            if !same_at(k, post)? {
                break 'post;
            }
        }
        post += 1;
    }
    while post > 0 && s0.surfaces[active[0][active[0].len() - post]].is_coord_break() {
        post -= 1;
    }
    // 構成 k の後ろの面の番号 → 構成 1 の面の番号
    let alias = |k: usize, j: usize| -> usize {
        let (a, a0) = (&active[k], &active[0]);
        (0..post)
            .find(|&i| a[a.len() - 1 - i] == j)
            .map_or(j, |i| a0[a0.len() - 1 - i])
    };
    if pre == 0 || post == 0 {
        return Err("the configurations share no surfaces before or after the BS".into());
    }
    let entry = active[0][pre - 1];
    let exit = active[0][active[0].len() - post];
    let e = &s0.surfaces[entry];
    let glass = e.mat.clone();
    if glass.is_air() || matches!(glass, Material::Mirror | Material::Inherit) {
        return Err(format!(
            "surface {entry} (the last surface the configurations share) doesn't enter a BS cube"
        )
        .into());
    }
    if s0.surfaces[exit].mat != Material::Air {
        return Err(format!("surface {exit} (the cube's exit face) doesn't leave into air").into());
    }
    let mids: Vec<Middle> = active
        .iter()
        .zip(cfgs)
        .enumerate()
        .map(|(k, (a, s))| {
            middle(s, &a[pre..a.len() - post], &glass)
                .map_err(|e| e.map_message(|m| format!("configuration {}: {m}", k + 1)))
        })
        .collect::<Result<_, _>>()?;
    let m0 = &mids[0];
    let port_of = |m: &Middle| if m.out == Slant::T { "t" } else { "r" };
    for (k, m) in mids.iter().enumerate() {
        if m.angle != m0.angle || m.coat != m0.coat {
            return Err(format!(
                "configuration {}: the BS's splitting face differs from configuration 1",
                k + 1
            )
            .into());
        }
        if m.out == m.back {
            return Err(format!(
                "configuration {}: the light leaves the BS through port A (it must come back to port D)",
                k + 1
            )
            .into());
        }
    }
    let arm_t = mids.iter().find(|m| m.out == Slant::T);
    let arm_r = mids.iter().find(|m| m.out == Slant::R);
    for m in &mids {
        let other = if m.out == Slant::T { arm_t } else { arm_r };
        if other.is_some_and(|o| o.port != m.port || o.arm != m.arm || o.mirror != m.mirror) {
            return Err(format!(
                "two configurations go out through BS port {} into different arms",
                port_of(m)
            )
            .into());
        }
    }

    // 絞り: 構成ごとに、ブロックの面（id, 番号）か入口の面（先頭の後ろの絞りの面）
    let mut stops: Vec<Option<(String, usize)>> = vec![None; cfgs.len()];
    let mut at: Vec<(usize, String, usize)> = Vec::new();
    let mut blocks = Vec::new();
    let head: Vec<usize> = active[0][..pre - 1].to_vec();
    let need_entry_stop = cfgs.iter().any(|s| s.surfaces[entry].stop);
    let mut head_s: Vec<Surface> = head.iter().map(|&j| s0.surfaces[j].clone()).collect();
    if need_entry_stop {
        // 入口の面の絞りは、入口の面に重なる空気の面に置く
        let mut d = Surface {
            semi: e.semi,
            comment: e.comment.clone(),
            ..Default::default()
        };
        if let Some(last) = head_s.last_mut() {
            d.t = std::mem::take(&mut last.t);
        }
        head_s.push(d);
    }
    let mut head_j = head.clone();
    if need_entry_stop {
        head_j.push(entry);
    }
    runs(
        &mut blocks,
        &mut at,
        "H",
        &head_s,
        &head_j,
        &Material::Air,
        false,
    )?;
    let cube = e.t * 2.0;
    let face_coat = |j: usize| s0.surfaces[j].coat.clone();
    let mut faces = serde_json::Map::new();
    faces.insert("a".into(), json!(face_coat(entry)));
    faces.insert("d".into(), json!(face_coat(exit)));
    let mut bs = json!({
        "type": "splitter",
        "id": BS_ID,
        "form": "cube",
        "size": cube,
        "glass": medium_name(&glass),
        "angle": m0.angle,
        "coat": m0.coat,
    });
    for (key, face, arm) in [("t", "b", arm_t), ("r", "c", arm_r)] {
        let Some(m) = arm else { continue };
        faces.insert(face.into(), json!(face_coat(m.port)));
        let mut v = Vec::new();
        let p = &s0.surfaces[m.port];
        // 反射の腕は鏡の後の向き: 光の進む向きに直す
        let neg = m.out == Slant::R;
        let surfs: Vec<Surface> = m.arm.clone().map(|j| s0.surfaces[j].clone()).collect();
        let js: Vec<usize> = m.arm.clone().collect();
        let lead = if neg { -p.t } else { p.t };
        let mut first = vec![Surface {
            t: lead,
            mat: p.mat.clone(),
            semi: p.semi,
            ..Default::default()
        }];
        first.extend(surfs);
        let mut jj = vec![m.port];
        jj.extend(js);
        // 口の面はキューブの面（BS が書く）。その後ろの媒質と厚さだけを最初の間隔にする
        let id = if key == "t" { "T" } else { "R" };
        arm_runs(&mut v, &mut at, id, &first, &jj, neg)?;
        let mut mi = s0.surfaces[m.mirror].clone();
        mi.ignore = false;
        if neg {
            mi.flip_z()?;
        }
        if let Some(t) = m.tilt {
            mi.before = Some(t);
            mi.after = After::Reverse;
        }
        let row = mirror_row(&mi)?;
        v.push(json!({
            "type": "mirror",
            "id": format!("M_{id}"),
            "back": true,
            "row": serde_json::to_value(&row).map_err(|e| e.to_string())?,
        }));
        bs[key] = json!(v);
    }
    bs["faces"] = Value::Object(faces);
    // 後ろ（出口の面の後から像面の前まで）
    let tail: Vec<usize> = active[0][active[0].len() - post + 1..].to_vec();
    let x = &s0.surfaces[exit];
    let mut tail_s = vec![Surface {
        t: x.t,
        semi: x.semi,
        ..Default::default()
    }];
    tail_s.extend(tail.iter().map(|&j| s0.surfaces[j].clone()));
    let mut tail_j = vec![exit];
    tail_j.extend(tail);
    let mut d = Vec::new();
    arm_runs(&mut d, &mut at, "D", &tail_s, &tail_j, false)?;
    let tail_j = &tail_j[1..];
    bs["d"] = json!(d);
    blocks.push(bs);

    for (k, s) in cfgs.iter().enumerate() {
        let Some(j) = (1..image).find(|&j| s.surfaces[j].stop && !s.surfaces[j].ignore) else {
            continue;
        };
        let j0 = alias(k, j);
        let Some((_, id, face)) = at.iter().find(|(x, ..)| *x == j0) else {
            return Err(format!(
                "configuration {}: the stop (surface {j}) is on a face the BS writes",
                k + 1
            )
            .into());
        };
        stops[k] = Some((id.clone(), *face));
    }

    // 系の値と、構成ごとの状態（系の値の違い・絞り）・光束
    let mut p = zmx_project(cfgs, path)?;
    p.as_object_mut()
        .expect("zmx_project writes an object")
        .remove("glrs");
    // 大域座標の基準の面はブロックの面で。BS が書く面・戻りの面・腕の中の面（光束によって通らない）なら
    // 基準は面 1 のまま（大域座標の原点だけが違い、光線は同じ）
    if s0.glrs != 1 {
        let shared = head_j.contains(&s0.glrs) || tail_j.contains(&s0.glrs);
        if let Some((_, id, face)) = at.iter().find(|(x, ..)| *x == s0.glrs).filter(|_| shared) {
            p["glrs"] = json!({ "block": id, "surface": face });
        }
    }
    if let Some(f) = &s0.coating_file {
        p["coating_file"] = json!(f.display().to_string());
    }
    p["blocks"] = json!(blocks);
    let ids: Vec<String> = blocks_with_ids(&p["blocks"]);
    let states = p["states"].as_array_mut().ok_or("no states")?;
    let mut beams = Vec::new();
    for (k, st) in states.iter_mut().enumerate() {
        let mut set = serde_json::Map::new();
        for id in &ids {
            let ops: Vec<Value> = match &stops[k] {
                Some((sid, face)) if sid == id => {
                    vec![json!({ "op": "set_stop", "surface": face })]
                }
                _ => Vec::new(),
            };
            set.insert(id.clone(), json!({ "surface_ops": ops }));
        }
        st["set"] = Value::Object(set);
        let m = &mids[k];
        let path = [m.out, m.back]
            .iter()
            .map(|x| if *x == Slant::T { 'T' } else { 'R' })
            .collect::<String>();
        beams.push(json!({
            "name": format!("C{}", k + 1),
            "path": path,
            "state": st["name"],
        }));
    }
    // 状態なしのプロジェクトは構成 1 の絞り
    if let Some((sid, face)) = &stops[0] {
        set_block_stop(&mut p["blocks"], sid, *face);
    }
    p["beams"] = json!(beams);
    let pj = checked(&p)?;
    for (k, s) in cfgs.iter().enumerate() {
        let name = format!("C{}", k + 1);
        let t = crate::expand(
            &pj,
            env,
            &crate::ExpandOpts {
                state: Some(&name),
                ..Default::default()
            },
        )
        .map_err(|e| format!("beam {name}: {e}"))?
        .system;
        retraces(s, &t).map_err(|e| e.map_message(|m| format!("configuration {}: {m}", k + 1)))?;
    }
    Ok(p)
}

/// 光束の系 t が元の構成 s と同じに光線を通すか（各視野の瞳の 9 点、主波長。像面の局所座標の位置・光路長・止まり・けられ）
fn retraces(s: &System, t: &System) -> lensbench_core::Result<()> {
    use lensbench_core::system::FieldSel;
    const TOL: f64 = 1e-6;
    const PX: [f64; 9] = [0.0, 0.7, -0.7, 0.0, 0.0, 0.5, -0.5, 0.5, -0.5];
    const PY: [f64; 9] = [0.0, 0.0, 0.0, 0.7, -0.7, 0.5, 0.5, -0.5, -0.5];
    let image = |x: &System| x.frames().surf.last().cloned().flatten();
    // 像面の位置が無い（無視された像面）と照らせない。黙って通さず断る
    let (Some(fa), Some(fb)) = (image(s), image(t)) else {
        return Err("the image surface has no position (is it ignored?), so the beam read from it can't be checked against it".into());
    };
    let local = |f: &lensbench_core::system::Frame, v: [f64; 3]| -> [f64; 3] {
        let d = [v[0] - f.o[0], v[1] - f.o[1], v[2] - f.o[2]];
        std::array::from_fn(|i| (0..3).map(|k| f.r[k][i] * d[k]).sum())
    };
    for f in 0..s.fields.len() {
        let a = s.try_trace(FieldSel::Index(f), &PX, &PY, None, None, false)?;
        let b = t.try_trace(FieldSel::Index(f), &PX, &PY, None, None, false)?;
        for (i, (ra, rb)) in a.rays.iter().zip(&b.rays).enumerate() {
            let off = |what: &str| -> lensbench_core::Error {
                format!(
                    "the beam read from it traces differently (field {}, ray {}: {what}); the surfaces after the mirror must retrace the arm",
                    f + 1,
                    i + 1
                )
                .into()
            };
            if ra.status.is_ok() != rb.status.is_ok() || (ra.vig == 0) != (rb.vig == 0) {
                return Err(off("stopped or vignetted in one only"));
            }
            if !ra.status.is_ok() {
                continue;
            }
            let (pa, pb) = (local(&fa, ra.p), local(&fb, rb.p));
            let dp = (0..3).map(|c| (pa[c] - pb[c]).abs()).fold(0.0, f64::max);
            if dp > TOL {
                return Err(off(&format!("{dp:.3e} mm apart")));
            }
            let dl = (ra.opl - rb.opl).abs();
            if dl > TOL {
                return Err(off(&format!("optical path {dl:.3e} mm apart")));
            }
        }
    }
    Ok(())
}

/// 2 つの面が、無視・絞り・厚さのほかは同じか
fn same_but_path(a: &Surface, b: &Surface) -> lensbench_core::Result<bool> {
    let norm = |s: &Surface| {
        let mut s = s.clone();
        s.ignore = false;
        s.stop = false;
        s.t = 0.0;
        // 自動の半径は光線から求めた値（構成ごとに違う）
        if s.semi_auto {
            s.semi = 0.0;
        }
        serde_json::to_value(&s).map_err(|e| lensbench_core::Error::from(e.to_string()))
    };
    Ok(norm(a)? == norm(b)?)
}

/// 構成の中ほどの面 m（入口の面の次から出口の面の前まで、無視しない面の番号）を見分ける
fn middle(s: &System, m: &[usize], glass: &Material) -> lensbench_core::Result<Middle> {
    let at = |i: usize| -> lensbench_core::Result<&Surface> {
        m.get(i)
            .map(|&j| &s.surfaces[j])
            .ok_or_else(|| "too few surfaces between the BS's entrance and exit faces".into())
    };
    let slant = |i: usize| -> lensbench_core::Result<(Slant, [f64; 2], String)> {
        let (c1, x, c2) = (at(i)?, at(i + 1)?, at(i + 2)?);
        let cb = |c: &Surface| {
            c.is_coord_break() && c.cb()[0] == 0.0 && c.cb()[1] == 0.0 && c.cb()[4] == 0.0
        };
        if !(cb(c1) && cb(c2) && c1.t == 0.0 && x.t == 0.0 && x.c == 0.0) {
            return Err(format!(
                "surfaces {}–{} are not a BS's splitting face (a coordinate break, a flat surface and a coordinate break)",
                m[i], m[i + 2]
            )
            .into());
        }
        let k = if x.is_mirror() {
            Slant::R
        } else if x.mat == *glass {
            Slant::T
        } else {
            return Err(format!(
                "surface {}: the splitting face neither reflects nor stays in the cube",
                m[i + 1]
            )
            .into());
        };
        Ok((k, [c1.cb()[2], c1.cb()[3]], x.coat.clone()))
    };
    let (out, angle, coat) = slant(0)?;
    let port = *m.get(3).ok_or("no BS face after the splitting face")?;
    // ミラー（往路の最初の鏡）。前後が逆の偏心・傾きの座標ブレークならミラーの傾き偏心にする
    let mi = (4..m.len())
        .find(|&i| s.surfaces[m[i]].is_mirror())
        .ok_or("no mirror in the arm")?;
    let cbs = |i: usize| s.surfaces[m[i]].is_coord_break();
    let (tilt, arm_end) = if mi >= 5 && mi + 1 < m.len() && cbs(mi - 1) && cbs(mi + 1) {
        let (a, b) = (&s.surfaces[m[mi - 1]], &s.surfaces[m[mi + 1]]);
        let inv = a.t == 0.0
            && a.cb_order() + b.cb_order() == 1
            && a.cb().iter().zip(b.cb()).all(|(x, y)| *x == -y);
        if !inv {
            return Err(format!(
                "the coordinate breaks around the mirror (surfaces {} and {}) don't undo each other",
                m[mi - 1],
                m[mi + 1]
            )
            .into());
        }
        (
            Some(Tilt {
                d: a.cb(),
                order: a.cb_order(),
            }),
            mi - 1,
        )
    } else {
        (None, mi)
    };
    let n = m.len();
    // 戻りの面が往路と違う（曲率など）と、後ろの共通の面が短くなり、ここで見る面が分ける面からずれる
    let (back, ..) = slant(n - 3).map_err(|e| {
        e.map_message(|msg| {
            format!(
                "on the way back from the mirror (surface {}), {msg}; the surfaces after the mirror must retrace the arm with the same values",
                m[mi]
            )
        })
    })?;
    Ok(Middle {
        out,
        angle,
        coat,
        port,
        arm: m[4]..m[arm_end],
        mirror: m[mi],
        tilt,
        back,
    })
}

/// 媒質の名前（空気の間隔の medium・玉の glass に書く形）
fn medium_name(m: &Material) -> String {
    match m {
        Material::Index(nd) => format!("{nd}"),
        Material::Model { nd, vd, dpgf } => format!("{nd},{vd},{dpgf}"),
        m => m.name(),
    }
}

/// 腕の面 v（v[0] は口の面の後ろの媒質と厚さだけを持つ）をブロックにする。口の後ろが空気でなければ
/// 媒質で満たす間隔の後、口の面に重なる同じ媒質の面から玉にする。neg は鏡の後の向き（光の進む向きに直す）
fn arm_runs(
    out: &mut Vec<Value>,
    at: &mut Vec<(usize, String, usize)>,
    id: &str,
    v: &[Surface],
    js: &[usize],
    neg: bool,
) -> lensbench_core::Result<()> {
    let mut v = v.to_vec();
    let m = v[0].mat.clone();
    if m.is_air() {
        out.push(json!({ "type": "air", "t": v[0].t }));
        v.remove(0);
        return runs(out, at, id, &v, &js[1..], &Material::Air, neg);
    }
    out.push(json!({ "type": "air", "t": 0, "medium": medium_name(&m) }));
    // 口の面に重なる面（同じ媒質の中、屈折しない）から玉
    runs(out, at, id, &v, js, &m, neg)
}

/// 面の並び v（系の面の番号 js）を、空気中の面は面の並び、媒質に入ってから空気に出るまでは玉のブロックにして、
/// 間を空気のブロックで並べる。neg なら鏡の後の向きで書いた面を光の進む向きに直す（形を z について裏返し、
/// 厚さの符号を変える）。before は最初の面の前の媒質（空気でなければ最初の玉の最初の面は同じ媒質の中）
fn runs(
    out: &mut Vec<Value>,
    at: &mut Vec<(usize, String, usize)>,
    id: &str,
    v: &[Surface],
    js: &[usize],
    before: &Material,
    neg: bool,
) -> lensbench_core::Result<()> {
    let (mut v, js) = wrap_decenters(v, js);
    let js = &js[..];
    // 構成 1 で無視した腕の面も、光束では通る
    for s in &mut v {
        s.ignore = false;
    }
    if neg {
        for (i, s) in v.iter_mut().enumerate() {
            // arm_runs が直した最初の面は直さない
            if i == 0 && !before.is_air() {
                continue;
            }
            s.flip_z()?;
            s.t = -s.t;
        }
    }
    let mut i = 0;
    let mut k = 0;
    let mut cur = before.clone();
    while i < v.len() {
        let bid = format!("{id}{k}");
        k += 1;
        if cur.is_air() {
            // 空気中の面の並び（媒質に入る面の前まで）
            let start = i;
            while i < v.len() && matches!(v[i].mat, Material::Air | Material::Inherit) {
                i += 1;
            }
            if i > start {
                let mut sp = Vec::new();
                let mut face = 0;
                for q in start..i {
                    if v[q].stop {
                        v[q].stop = false;
                    }
                    if !v[q].is_coord_break() {
                        at.push((js[q], bid.clone(), face));
                        face += 1;
                    }
                    sp.push(
                        serde_json::to_value(surface_spec(&v[q], false)?)
                            .map_err(|e| e.to_string())?,
                    );
                }
                out.push(json!({ "type": "surfaces", "id": bid, "surfaces": sp }));
                continue;
            }
            cur = v[i].mat.clone();
        }
        // 玉: 媒質に入る面から空気に出る面まで
        let start = i;
        while i < v.len() && !v[i].mat.is_air() {
            if !matches!(v[i].mat, Material::Inherit) {
                cur = v[i].mat.clone();
            }
            i += 1;
        }
        // 媒質の中で終わる（玉の中のミラーの前）: 屈折しない同じ媒質の面で玉を閉じ、媒質で満たす間隔で続ける
        let inside = i >= v.len();
        if inside {
            v.push(Surface {
                comment: "(medium continues)".into(),
                ..Default::default()
            });
        }
        let mut sp = Vec::new();
        let mut face = 0;
        for q in start..=i {
            let last = q == i;
            let mut s = v[q].clone();
            s.stop = false;
            if !s.is_coord_break() {
                if let Some(&j) = js.get(q) {
                    at.push((j, bid.clone(), face));
                }
                face += 1;
            }
            let mut spec = surface_spec(&s, last)?;
            if last {
                spec.t = 0.0;
            }
            sp.push(serde_json::to_value(spec).map_err(|e| e.to_string())?);
        }
        out.push(json!({ "type": "lens", "id": bid, "source": { "surfaces": sp } }));
        if inside {
            out.push(json!({ "type": "air", "t": 0, "medium": medium_name(&cur) }));
            break;
        }
        out.push(json!({ "type": "air", "t": v[i].t }));
        cur = Material::Air;
        i += 1;
    }
    Ok(())
}

/// 木のブロックの id（BS の分岐の中も）
fn blocks_with_ids(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<String>) {
        for b in v.as_array().into_iter().flatten() {
            if let Some(id) = b["id"].as_str().filter(|x| !x.is_empty()) {
                if matches!(b["type"].as_str(), Some("surfaces" | "lens")) {
                    out.push(id.to_string());
                }
            }
            for key in ["t", "r", "d", "blocks"] {
                if b[key].is_array() {
                    walk(&b[key], out);
                }
            }
        }
    }
    walk(v, &mut out);
    out
}

/// id のブロックの絞りの面を face にする
fn set_block_stop(v: &mut Value, id: &str, face: usize) {
    for b in v.as_array_mut().into_iter().flatten() {
        if b["id"] == id {
            b["surface_ops"] = json!([{ "op": "set_stop", "surface": face }]);
        }
        for key in ["t", "r", "d", "blocks"] {
            if b[key].is_array() {
                set_block_stop(&mut b[key], id, face);
            }
        }
    }
}

/// 座標ブレーク・面・逆の座標ブレーク（前の厚さ 0）の 3 面を、面の前後の傾き偏心（後ろで戻す）を持つ 1 面にまとめる。
/// 厚さは後ろの座標ブレークのもの
fn wrap_decenters(v: &[Surface], js: &[usize]) -> (Vec<Surface>, Vec<usize>) {
    let mut out = Vec::new();
    let mut oj = Vec::new();
    let mut i = 0;
    while i < v.len() {
        if i + 2 < v.len() {
            let (a, x, b) = (&v[i], &v[i + 1], &v[i + 2]);
            let inv = a.is_coord_break()
                && b.is_coord_break()
                && !x.is_coord_break()
                && !x.is_mirror()
                && x.before.is_none()
                && matches!(x.after, After::None)
                && a.t == 0.0
                && x.t == 0.0
                && a.cb_order() == 0
                && b.cb_order() == 1
                && a.cb().iter().zip(b.cb()).all(|(p, q)| *p == -q);
            if inv {
                let mut s = x.clone();
                s.before = Some(Tilt {
                    d: a.cb(),
                    order: 0,
                });
                s.after = After::Reverse;
                s.t = b.t;
                out.push(s);
                oj.push(js[i + 1]);
                i += 3;
                continue;
            }
        }
        out.push(v[i].clone());
        oj.push(js[i]);
        i += 1;
    }
    (out, oj)
}
