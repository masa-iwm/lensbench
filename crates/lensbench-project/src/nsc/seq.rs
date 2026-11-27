//! 非順次の経路を順次の系へ戻す: 経路の表の出来事の列が、場面の系（光束・BS の経路）・ゴーストの展開・足した光束の
//! 系の順次の経路と同じかを確かめる（干渉・MTF などへ回すため）

use crate::raypath::{EventKind, RayPath};
use crate::scene::{ObjectKind, Scene};
use lensbench_core::{Result, System};

/// 非順次で出来事にならない面（絞り、ガラスの中の働きの無い面）か
fn silent(sc: &Scene, f: usize) -> bool {
    let face = &sc.faces[f];
    let kind = sc.objects[face.object].kind;
    kind == ObjectKind::Stop
        || (kind == ObjectKind::Lens
            && face.minus.label() == face.plus.label()
            && face.coat.is_empty())
}

/// 順次の経路 path を、面の対応 to_face（系の面 → 場面の面）で場面の面の出来事の列に（非順次で出来事にならない面と
/// 場面に無い面は除く）
pub fn scene_events(
    sc: &Scene,
    path: &RayPath,
    to_face: impl Fn(usize) -> Option<usize>,
) -> Vec<(usize, EventKind)> {
    path.0
        .iter()
        .filter_map(|e| {
            let f = to_face(e.surf)?;
            (!silent(sc, f)).then_some((f, e.kind))
        })
        .collect()
}

/// 場面の系 k（[`Scene::systems`] の番号、順次の系 s）の主の経路が events と同じか
pub fn is_system_path(
    sc: &Scene,
    k: usize,
    s: &System,
    events: &[(usize, EventKind)],
) -> Result<bool> {
    let p = RayPath::sequential(s, None)?;
    Ok(scene_events(sc, &p, |j| sc.face_of(k, j)) == events)
}

/// 場面の系 k の面 a・b（b < a）で反射するゴーストのうち、events と同じ経路の (a, b)。events の 2 つの反射の面から
/// 決める（玉の境界の反射が 2 つで、どちらも系 k の面のときだけ）
pub fn ghost_of(
    sc: &Scene,
    k: usize,
    s: &System,
    events: &[(usize, EventKind)],
) -> Result<Option<(usize, usize)>> {
    let refl: Vec<usize> = events
        .iter()
        .filter(|e| e.1.reflects())
        .map(|e| e.0)
        .collect();
    let [fa, fb] = refl[..] else {
        return Ok(None);
    };
    let surf = |f: usize| -> Vec<usize> {
        sc.faces[f]
            .refs
            .iter()
            .filter(|r| r.system == k)
            .map(|r| r.surf)
            .collect()
    };
    for a in surf(fa) {
        for b in surf(fb) {
            if b >= a {
                continue;
            }
            let Ok(p) = RayPath::ghost(s, None, a, b) else {
                continue;
            };
            if scene_events(sc, &p, |j| sc.face_of(k, j)) == events {
                return Ok(Some((a, b)));
            }
        }
    }
    Ok(None)
}

/// 場面の面 f の、玉の面の書き方（光束の `turn` の JSON）: 場面の系ごとの面の書き方 keys（[`Scene::systems`] の順、
/// 系の面ごと）から
pub fn face_key(
    sc: &Scene,
    keys: &[Vec<Option<serde_json::Value>>],
    f: usize,
) -> Option<serde_json::Value> {
    sc.faces[f]
        .refs
        .iter()
        .find_map(|r| keys.get(r.system)?.get(r.surf)?.clone())
}

/// 場面に無い順次の系 s（足した光束の系など）の主の経路が events と同じか。系の面は同じ所（頂点と軸の向き）の場面の
/// 面に対応させる。像面の検出はどの検出器でもよい
pub fn is_placed_path(sc: &Scene, s: &System, events: &[(usize, EventKind)]) -> Result<bool> {
    let p = RayPath::sequential(s, None)?;
    let img = s.image();
    let fr = s.frames();
    let mut got = Vec::new();
    for e in &p.0 {
        if e.surf == img {
            continue;
        }
        let Some(f) = fr
            .surf
            .get(e.surf)
            .copied()
            .flatten()
            .and_then(|q| face_at(sc, &q))
        else {
            continue;
        };
        if !silent(sc, f) {
            got.push((f, e.kind));
        }
    }
    let Some((_, last)) = events.last() else {
        return Ok(false);
    };
    Ok(*last == EventKind::Detect && got[..] == events[..events.len() - 1])
}

/// 順次の系の面の座標 f と同じ所の場面の面（頂点が 1e-7 mm、軸が向きを問わず 1e-9 で合うもの）
fn face_at(sc: &Scene, f: &lensbench_core::system::Frame) -> Option<usize> {
    let ax = [f.r[0][2], f.r[1][2], f.r[2][2]];
    sc.faces.iter().position(|g| {
        let d = (0..3)
            .map(|i| (g.frame.o[i] - f.o[i]).abs())
            .fold(0.0, f64::max);
        let a = g.axis();
        let dot = a[0] * ax[0] + a[1] * ax[1] + a[2] * ax[2];
        d < 1e-7 && dot.abs() > 1.0 - 1e-9 && !g.refs.is_empty()
    })
}
