//! ゴースト: 屈折面 a と b（b < a）で 1 回ずつ反射する経路（OpticStudio の Ghost Focus Generator の 2 回反射）。
//!
//! 経路は「面 0〜a を進む → a で反射 → a−1〜b を逆にたどる → b で反射 → b+1〜像面を進む」。これを順次の面の列に
//! 展開した系（[`ghost_system`]）を作り、追跡・偏光・解析は普通の系と同じに使う。
//!
//! - 反射する面は鏡にし、[`Surface::reflect`] に反対側の媒質を持たせる。偏光追跡の係数は、入射側と反対側の境界
//!   （膜込み）の反射 r（[`crate::coating::Coatings::prepare`]）。
//! - 逆にたどる区間の面は元の面の写し（曲率などの形はそのまま、後ろの媒質は元の面の前の媒質）。面の位置・向きは
//!   元の系と同じ（大域の座標で一致する）: 共軸の区間は厚さの符号を変えるだけ（OpticStudio の鏡のあとの約束）、
//!   傾き・偏心・座標ブレークのある区間は、元の面の座標系へ移す座標ブレークを挟む。
//! - 絞りは最初に通る写しだけに残す。像側で決める開口（F 値・作動 F 値）は、元の系の入射瞳径に置き換える
//!   （ゴーストの系の近軸量で入射光束が変わらないように）。ピックアップは解決済みの値のまま外す。
//! - 元の面に戻せる対応 [`GhostSystem::origin`] を持つ（挟んだ座標ブレークは None）。

use crate::coating;
use crate::math::*;
use crate::surface::{After, Reflect, SurfType, Surface, Tilt};
use crate::system::{apply_cb, step_frame, ApertureKind, System};
use crate::{invalid, Result};

/// 展開したゴーストの系
#[derive(Debug, Clone)]
pub struct GhostSystem {
    pub sys: System,
    /// 先に反射する面（元の系の番号）
    pub a: usize,
    /// 後で反射する面（元の系の番号、b < a）
    pub b: usize,
    /// 展開した系の面ごとの元の面の番号（挟んだ座標ブレークは None）
    pub origin: Vec<Option<usize>>,
    /// 展開した系での反射の面の番号（a, b の順）
    pub at: (usize, usize),
}

/// 反射する面になれる面の番号（境界のある面: 媒質が変わるか膜がある。鏡・座標ブレーク・理想レンズ・Jones 面・
/// 無視する面・物体面は除く）を昇順で。像面は膜があるときだけ入れる（先に反射する面 a にだけなれる。
/// 検出器の反射は像面に理想膜 `I.xx` などを付けて表す）
pub fn reflecting_surfaces(sys: &System) -> Vec<usize> {
    let (keys, _) = coating::media_keys(sys);
    let img = sys.image();
    (1..=img)
        .filter(|&k| {
            let d = &sys.surfaces[k];
            keys[k].is_some()
                && (k < img || !d.coat.is_empty())
                && !d.mirror
                && !d.ignore
                && matches!(
                    d.typ,
                    SurfType::Standard
                        | SurfType::EvenAsph
                        | SurfType::Toroidal
                        | SurfType::ZernikeStd
                        | SurfType::ZernikeFringe
                        | SurfType::GridSag
                )
        })
        .collect()
}

/// 2 回反射の組 (a, b)（a > b、a は先に反射する面）。first..=last の面だけを使う（OpticStudio の First / Last Surface）
pub fn pairs(sys: &System, first: usize, last: usize) -> Vec<(usize, usize)> {
    let s: Vec<usize> = reflecting_surfaces(sys)
        .into_iter()
        .filter(|&k| first <= k && k <= last)
        .collect();
    let mut out = Vec::new();
    for (ia, &a) in s.iter().enumerate() {
        for &b in &s[..ia] {
            out.push((a, b));
        }
    }
    // OpticStudio と同じ並び: 後で反射する面 b の順、その中で a の順
    out.sort_by_key(|&(a, b)| (b, a));
    out
}

/// 傾き・偏心・座標ブレークで軸を動かさない面（厚さだけで進む）
fn plain(d: &Surface) -> bool {
    d.ignore
        || (d.before.is_none()
            && matches!(d.after, After::None | After::Reverse)
            && (d.typ != SurfType::CoordBrk || d.cb.iter().all(|&v| v == 0.0)))
}

/// 面 a で反射し、面 b で反射するゴーストの経路を展開した系
pub fn ghost_system(sys: &System, a: usize, b: usize) -> Result<GhostSystem> {
    let ok = reflecting_surfaces(sys);
    if !(b < a) {
        invalid!("ゴースト: 先に反射する面 {a} は後で反射する面 {b} より後ろ（b < a）");
    }
    if let Some(j) =
        (1..sys.surfaces.len()).find(|&j| !sys.surfaces[j].ignore && sys.surfaces[j].grin.is_some())
    {
        crate::notimpl!("ゴースト: GRIN の媒質（面 {j}）を含む系");
    }
    let img = sys.image();
    if b == img {
        invalid!("ゴースト: 像面は先に反射する面 a にだけなれる");
    }
    // 像面は膜がなくても a にできる（反射の係数は膜なしの境界のもの）
    let image_ok = |k: usize| {
        k == img
            && !sys.surfaces[k].mirror
            && matches!(
                sys.surfaces[k].typ,
                SurfType::Standard
                    | SurfType::EvenAsph
                    | SurfType::Toroidal
                    | SurfType::ZernikeStd
                    | SurfType::ZernikeFringe
                    | SurfType::GridSag
            )
    };
    for k in [a, b] {
        if !ok.contains(&k) && !image_ok(k) {
            invalid!(
                "ゴースト: 面 {k} は反射する面にできない（境界のない面・鏡・座標ブレークなど）"
            );
        }
    }
    let sf = &sys.surfaces;
    let nw = sys.wavelengths.len();
    let (_, before_name) = coating::media_keys(sys);
    let idx_before = |m: usize| -> Vec<f64> { (0..nw).map(|w| sys.index_before(m, w)).collect() };
    let air = |s: &str| {
        if s == "AIR" {
            String::new()
        } else {
            s.to_string()
        }
    };
    // 元の系の、面ごとの「面に入る前の座標系」（面の傾き・座標ブレークをかける前）
    let mut pre = Vec::with_capacity(sf.len());
    {
        let (mut o, mut r) = ([0.0; 3], I3);
        for (j, d) in sf.iter().enumerate() {
            pre.push((o, r));
            if j == 0 {
                continue;
            }
            (_, (o, r)) = step_frame(o, r, d);
        }
    }
    // 元の a の後ろの媒質
    let after_a = {
        let mut m = before_name[a].clone();
        let d = &sf[a];
        if !d.mirror {
            m = if d.mat.is_empty() {
                "AIR".to_string()
            } else {
                d.mat.clone()
            };
        }
        m
    };

    let mut out: Vec<Surface> = Vec::new();
    let mut origin: Vec<Option<usize>> = Vec::new();
    let mut stop_done = false;
    let mut push = |out: &mut Vec<Surface>,
                    origin: &mut Vec<Option<usize>>,
                    mut d: Surface,
                    o: Option<usize>| {
        if d.stop {
            if stop_done {
                d.stop = false;
            }
            stop_done = true;
        }
        out.push(d);
        origin.push(o);
    };
    // 0〜a−1: そのまま
    for (j, d) in sf.iter().enumerate().take(a) {
        push(&mut out, &mut origin, d.clone(), Some(j));
    }
    // a: 反射（入射側は a の前の媒質、反対側は a の後ろの媒質）
    {
        let d = &sf[a];
        let mut g = d.clone();
        g.mirror = true;
        g.mat = "MIRROR".into();
        g.reflect = Some(Reflect {
            mat: after_a.clone(),
            n: d.n.clone(),
        });
        g.n = idx_before(a);
        g.n_base = Vec::new();
        g.dn = 0.0;
        g.disp = 0.0;
        g.after = After::None;
        g.t = 0.0;
        push(&mut out, &mut origin, g, Some(a));
    }
    let at_a = out.len() - 1;
    // 今の座標系（展開した系で、次の面へ渡す座標系）。a の座標系から始める
    let (mut o, mut r) = {
        let (o0, r0) = pre[a];
        step_frame(o0, r0, &out[at_a]).1
    };
    // 直前に置いた面（厚さで z を進める面）と、その元の番号
    let mut last_src = a;
    // a−1〜b を逆に
    let mut at_b = 0;
    for m in (b..a).rev() {
        let d = &sf[m];
        if d.ignore || d.typ == SurfType::CoordBrk {
            continue;
        }
        let lp = out.len() - 1;
        if (m..=last_src).all(|k| plain(&sf[k])) {
            // 共軸: 元の厚さの符号を変える（座標ブレークの厚さも含む。無視する面の厚さは数えない）
            out[lp].t = -(m..last_src)
                .filter(|&k| !sf[k].ignore)
                .map(|k| sf[k].t)
                .sum::<f64>();
            (o, r) = pre[m];
        } else {
            // 元の面に入る前の座標系へ: 直前の面の厚さで z を、座標ブレーク（順序 0）で x・y と向きを合わせる
            let (ot, rt) = pre[m];
            let dv = mtv(&r, sub(ot, o));
            out[lp].t = dv[2];
            o = add(o, scale(col(&r, 2), dv[2]));
            let [tx, ty, tz] = xyz_angles(&mm(&transpose(&r), &rt));
            let cb = Tilt {
                d: [dv[0], dv[1], tx, ty, tz],
                order: 0,
            };
            if cb.d.iter().any(|&v| v != 0.0) {
                (o, r) = apply_cb(o, r, &cb);
                let c = Surface {
                    typ: SurfType::CoordBrk,
                    mat: "-".into(),
                    cb: cb.d,
                    cb_order: 0,
                    n: out[lp].n.clone(),
                    ..Default::default()
                };
                push(&mut out, &mut origin, c, None);
            }
        }
        // 写し: 形はそのまま、後ろの媒質は元の面の前の媒質
        let mut g = d.clone();
        if m == b {
            g.mirror = true;
            g.mat = "MIRROR".into();
            g.reflect = Some(Reflect {
                mat: before_name[b].clone(),
                n: idx_before(b),
            });
            // 反射のあとは元の b の後ろの媒質のまま、元の b と同じ厚さ・傾きで b+1 へ
            g.n_base = Vec::new();
            g.dn = 0.0;
            g.disp = 0.0;
        } else {
            if !d.mirror {
                g.mat = air(&before_name[m]);
                g.n = idx_before(m);
                // 媒質のずれ（param）は元の前の媒質の面のもの。解決済みの n を使う
                g.n_base = Vec::new();
                g.dn = 0.0;
                g.disp = 0.0;
            }
            g.after = After::None;
            g.t = 0.0;
            (_, (o, r)) = step_frame(o, r, &g);
        }
        push(&mut out, &mut origin, g, Some(m));
        if m == b {
            at_b = out.len() - 1;
        }
        last_src = m;
    }
    // b+1〜像面: そのまま
    for (j, d) in sf.iter().enumerate().skip(b + 1) {
        push(&mut out, &mut origin, d.clone(), Some(j));
    }

    let mut g = sys.clone();
    g.surfaces = out;
    g.pickups.clear();
    g.units.clear();
    g.name = format!("{} ghost {a}-{b}", sys.name);
    // 大域の座標の基準の面: 最初に通る写し
    g.glrs = origin
        .iter()
        .position(|&x| x == Some(sys.glrs))
        .unwrap_or(1);
    if matches!(g.aperture.0, ApertureKind::FNum | ApertureKind::Pwfn) {
        g.aperture = (ApertureKind::Epd, sys.paraxial().epd);
    }
    g.changed()?;
    Ok(GhostSystem {
        sys: g,
        a,
        b,
        origin,
        at: (at_a, at_b),
    })
}

/// ゴーストの近軸の量（主波長）
#[derive(Debug, Clone, Copy)]
pub struct GhostParaxial {
    pub a: usize,
    pub b: usize,
    /// 像面での近軸の周辺光線の高さ（ゴーストのぼけの半径）
    pub marginal_y: f64,
    /// 像面での近軸の周辺光線の角度（像空間の媒質での u）
    pub marginal_u: f64,
    /// 像面から近軸のゴーストの焦点までの距離（光の進む向きが正。無限遠は inf）
    pub focus: f64,
    /// 近軸の作動 F 値（ゴーストの光束の、像空間での）
    pub wfno: f64,
}

/// ゴーストの系の近軸の量
pub fn paraxial(g: &GhostSystem) -> GhostParaxial {
    let s = &g.sys;
    let p = s.paraxial();
    let img = s.image();
    let (ym, um) = (&p.marginal.0, &p.marginal.1);
    let (y, u) = (ym[img], um[img - 1]);
    GhostParaxial {
        a: g.a,
        b: g.b,
        marginal_y: y,
        marginal_u: u,
        focus: if u != 0.0 { -y / u } else { f64::INFINITY },
        wfno: p.wfno,
    }
}

/// ゴーストの一覧の 1 行
#[derive(Debug, Clone)]
pub struct GhostInfo {
    pub par: GhostParaxial,
    /// 軸上の視野の主光線の強度の比（ゴースト / 元の系、主波長、x・y の偏光の平均）。膜込みの反射率・透過率・吸収を含む。
    /// 求められないとき（光線が届かない、膜の係数がない）は Err に理由
    pub intensity: std::result::Result<f64, String>,
}

/// first..=last の面の全部の組の近軸の量と強度の比
pub fn survey(
    sys: &System,
    first: usize,
    last: usize,
    coat: &crate::coating::Coatings,
) -> Result<Vec<GhostInfo>> {
    use crate::coating::C64;
    use crate::system::FieldSel;
    let pols = [
        [C64::new(1.0, 0.0), C64::new(0.0, 0.0)],
        [C64::new(0.0, 0.0), C64::new(1.0, 0.0)],
    ];
    let power = |s: &System| -> std::result::Result<f64, String> {
        let t = s
            .try_trace_pol(FieldSel::Norm(0.0, 0.0), &[0.0], &[0.0], None, &pols, coat)
            .map_err(|e| e.to_string())?;
        let r = &t.trace.rays[0];
        if r.err != 0 {
            return Err(format!("主光線が面 {} で止まる", r.err));
        }
        let p: f64 = t.e[0]
            .iter()
            .map(|e| e.iter().map(|z| z.norm_sqr()).sum::<f64>())
            .sum::<f64>()
            / 2.0;
        if !p.is_finite() {
            return Err("偏光の係数が求められない".into());
        }
        Ok(p)
    };
    let main = power(sys);
    let mut out = Vec::new();
    for (a, b) in pairs(sys, first, last) {
        let g = ghost_system(sys, a, b)?;
        let intensity = match (&main, power(&g.sys)) {
            (Ok(m), Ok(p)) => Ok(p / m),
            (Err(e), _) => Err(format!("元の系: {e}")),
            (_, Err(e)) => Err(e),
        };
        out.push(GhostInfo {
            par: paraxial(&g),
            intensity,
        });
    }
    Ok(out)
}
