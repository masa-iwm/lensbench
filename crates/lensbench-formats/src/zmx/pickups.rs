//! .zmx のピックアップ: 厚さ・パラメータ・面の後の傾き偏心・円錐係数の値を解き、つながりを系に残す

use super::surfaces::AfterPick;
use super::*;

/// 面ごとの厚さ（多重構成の THIC と厚さのピックアップを当てる）。ピックアップの元の面と、ソルブを置ける面かも確かめる
pub(super) fn thicknesses(raw: &Raw, ops: &Ops) -> Result<Vec<f64>> {
    let ns = raw.surfs.len();
    let mut th: Vec<f64> = raw.surfs.iter().map(|s| s.disz.unwrap_or(0.0)).collect();
    for (k, t) in th.iter_mut().enumerate() {
        if let Some(v) = ops.get(&("THIC".to_string(), k as i64)) {
            *t = fv(v)?;
        }
    }
    for (k, r) in raw.surfs.iter().enumerate() {
        for src in [
            r.pzup.map(|p| p.0),
            r.pkup.map(|p| p.0),
            r.curv_pickup.map(|p| p.0),
            r.diam_pickup.map(|p| p.0),
            r.mema_pickup.map(|p| p.0),
        ]
        .into_iter()
        .flatten()
        .chain(r.ppar.values().map(|p| p.0))
        {
            if src >= ns {
                invalid!("surface {k}: no pickup source surface {src}");
            }
        }
        if r.tsolve.is_some() || r.tmacro.is_some() {
            if k == 0 || k + 1 >= ns {
                notimpl!("surface {k}: thickness solve on the object or image surface");
            }
            if ops.contains_key(&("THIC".to_string(), k as i64)) {
                notimpl!("surface {k}: thickness solve and a multi-configuration THIC");
            }
            if r.pzup.is_some() {
                invalid!("surface {k}: both a solve and a pickup on the thickness");
            }
        }
        if r.csolve.is_some() || r.cmacro.is_some() {
            if k == 0 || k + 1 >= ns {
                notimpl!("surface {k}: curvature solve on the object or image surface");
            }
            if ops.contains_key(&("CRVT".to_string(), k as i64)) {
                notimpl!("surface {k}: curvature solve and a multi-configuration CRVT");
            }
        }
    }
    thickness_pickups(raw, &mut th)?;
    Ok(th)
}

/// 面ごとの PARM の値（構成 cfg の多重構成の PRAM と、同じ列からのパラメータのピックアップを当てる）
pub(super) fn parms(raw: &Raw, cfg: i64) -> Result<Vec<BTreeMap<i64, f64>>> {
    let ns = raw.surfs.len();
    let mut parm: Vec<BTreeMap<i64, f64>> = raw.surfs.iter().map(|s| s.parm.clone()).collect();
    // 多重構成の PRAM（面の PARM の値）: `PRAM 面 構成 値 0 PARM の番号 …`
    for (op, p1, c, v, rest) in &raw.mce {
        if op == "PRAM" && *c == cfg {
            let (Some(k), Some(p)) = (
                usize::try_from(*p1).ok().filter(|&k| k < ns),
                rest.get(1).and_then(|x| x.parse::<i64>().ok()),
            ) else {
                invalid!("multi-configuration PRAM {p1}: bad surface or PARM number");
            };
            parm[k].insert(p, fv(v)?);
        }
    }
    for _ in 0..=ns {
        let mut changed = false;
        for k in 0..ns {
            for (&p, &(src, sc, off, from)) in &raw.surfs[k].ppar {
                // 別の列からのものは系を作ってから解決する（[`System::resolve_pickups`]）
                if from.is_some() {
                    continue;
                }
                let v = sc * parm[src].get(&p).copied().unwrap_or(0.0) + off;
                if parm[k].get(&p) != Some(&v) {
                    parm[k].insert(p, v);
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(parm);
        }
    }
    invalid!("parameter pickups don't converge (a loop?)")
}

/// 面の後の傾き偏心のピックアップ（after[k] = (元の面, 反転)）: 元の面の前の値と順序（反転は符号と順序が逆）。
/// ファイルの SCBD 2 の値は使わない
pub(super) fn after_tilts(sf: &mut [Surface], after: &[AfterPick]) {
    for k in 0..sf.len() {
        if let Some((src, rev)) = after[k] {
            sf[k].after = match sf[src].before {
                Some(b) if rev => After::Tilt(Tilt {
                    d: b.d.map(|v| -v),
                    order: 1 - b.order.min(1),
                }),
                Some(b) => After::Tilt(b),
                None => After::None,
            };
        }
    }
}

/// 円錐係数のピックアップ（元の面は後ろにあってもよい）
pub(super) fn conics(raw: &Raw, sf: &mut [Surface]) -> Result<()> {
    let ns = sf.len();
    for _ in 0..=ns {
        let mut changed = false;
        for k in 0..ns {
            if let Some((src, sc, off, None)) = raw.surfs[k].pkup {
                if src >= ns {
                    invalid!("surface {k}: no conic pickup source surface {src}");
                }
                let v = sc * sf[src].k + off;
                if v != sf[k].k {
                    sf[k].k = v;
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(());
        }
    }
    invalid!("conic pickups don't converge (a loop?)")
}

/// ピックアップのつながり（系に残し、パラメータを変えたときに解決し直す）。
/// 元か先の面タイプにそのパラメータがなければ、読込の値だけにする
pub(super) fn links(raw: &Raw, sf: &[Surface], after: &[AfterPick]) -> Vec<Pickup> {
    let ns = sf.len();
    let mut pickups: Vec<Pickup> = Vec::new();
    for (k, r) in raw.surfs.iter().enumerate() {
        let mut add = |src: usize, what: Kind, scale: f64, offset: f64, from: Option<Kind>| {
            pickups.push(Pickup {
                from,
                ..Pickup::new(k, src, what, scale, offset)
            })
        };
        if let Some((src, sc, off, from)) = r.pzup {
            add(src, Kind::Thick, sc, off, from);
        }
        if let Some((src, sc, off, from)) = r.pkup {
            add(src, Kind::Conic, sc, off, from);
        }
        if let Some((src, sc, from)) = r.curv_pickup {
            add(src, Kind::Curv, sc, 0.0, from);
        }
        if let Some((src, sc, from)) = r.diam_pickup {
            add(src, Kind::Semi, sc, 0.0, from);
        }
        if let Some((src, sc)) = r.mema_pickup {
            add(src, Kind::MechSemi, sc, 0.0, None);
        }
        if let Some(src) = r.glas_pickup {
            // 材料は読込で元の面のものにした。屈折率のずれ（dn・disp）も元の面に追従させる
            add(src, Kind::Dn, 1.0, 0.0, None);
            add(src, Kind::Disp, 1.0, 0.0, None);
        }
        for (&p, &(src, sc, off, from)) in &r.ppar {
            let Ok(i) = u8::try_from(p) else { continue };
            let has = |j: usize, w: Kind| match w {
                Kind::Parm(i) => surf_parm(&sf[j], i).is_some(),
                _ => true,
            };
            if src < ns && has(k, Kind::Parm(i)) && has(src, from.unwrap_or(Kind::Parm(i))) {
                add(src, Kind::Parm(i), sc, off, from);
            }
        }
        if let Some((src, rev)) = after[k] {
            // 面の後の傾き偏心 = 元の面の前の傾き偏心（反転は値の符号と順序が逆）
            for i in 0..5u8 {
                pickups.push(Pickup {
                    from: Some(Kind::Before(i)),
                    ..Pickup::new(k, src, Kind::After(i), if rev { -1.0 } else { 1.0 }, 0.0)
                });
            }
        }
    }
    pickups
}

/// 厚さのピックアップ（PZUP）を値にする
fn thickness_pickups(raw: &Raw, th: &mut [f64]) -> Result<()> {
    for _ in 0..=th.len() {
        let mut changed = false;
        for (k, r) in raw.surfs.iter().enumerate() {
            if let Some((src, sc, off, None)) = r.pzup {
                // 無限大の厚さは 1e10 として掛ける（OpticStudio が書く値と同じ）
                let t = th.get(src).copied().unwrap_or(0.0);
                let v =
                    sc * if t.is_infinite() {
                        1e10f64.copysign(t)
                    } else {
                        t
                    } + off;
                if v != th[k] {
                    th[k] = v;
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(());
        }
    }
    invalid!("thickness pickups don't converge (a loop?)")
}
