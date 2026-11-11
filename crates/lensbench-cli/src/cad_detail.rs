//! 取った品の STEP を測る・詳しい形（三角形）にする（公開 #121）。ネイティブと Web 版の両方で使う
//! （取る・上げるのは [`crate::cad`]、ネイティブだけ。Web 版は GUI が置き場から取って置き場（vfs）に入れる）。
use lensbench_project::{catalog_fill, UserItem};

/// STEP を catalog-step と同じ読み方で測り、品の簡易外形と比べる（測った寸法、食い違い・測れなかった理由）
pub fn measure(item: &UserItem, text: &[u8]) -> (Option<catalog_fill::StepSize>, Vec<String>) {
    match catalog_fill::read_step(&String::from_utf8_lossy(text)) {
        Ok(s) => {
            let notes = match &item.mech {
                Some(m) => catalog_fill::cad_check(m, &s),
                None => Vec::new(),
            };
            (Some(s), notes)
        }
        Err(e) => (None, vec![format!("STEP を測れない: {e}")]),
    }
}

/// 詳しい形の三角形の細かさ [mm]（面からのずれの許し）
const DETAIL_TOL_MM: f64 = 0.2;

/// STEP の文字を部品の座標の三角形にする（[`catalog_fill::step_shell_poses`] の置き方で、殻ごとに三角形にして置く）。
/// 三角形にできなかった面と、殻の辺の箱から 1% を超えて外れる面（境の読めなかった面）は除く
pub fn detail_of_step(text: &str) -> Result<lensbench_export::detail::Detail, String> {
    use truck_meshalgo::prelude::*;
    use truck_stepio::r#in::Table;
    let poses = catalog_fill::step_shell_poses(text)?;
    let all_poses = poses.clone();
    let upper_unit = catalog_fill::read_step(text)
        .map(|s| s.unit_mm)
        .unwrap_or(1.0);
    let table = Table::from_step(text).ok_or("STEP を読めない（truck）")?;
    let tol = DETAIL_TOL_MM / upper_unit;
    let mut meshed: std::collections::HashMap<u64, Vec<[[f64; 3]; 3]>> = Default::default();
    let mut tris = Vec::new();
    for (shell, m) in poses {
        if let std::collections::hash_map::Entry::Vacant(slot) = meshed.entry(shell) {
            let Some(sh) = table.shell.get(&shell) else {
                continue;
            };
            let Ok(cs) = table.to_compressed_shell(sh) else {
                continue;
            };
            let ms = cs.robust_triangulation(tol);
            let mut bb = BoundingBox::<Point3>::new();
            for e in &ms.edges {
                e.curve.iter().for_each(|p| bb.push(*p));
            }
            let d = bb.diameter() * 0.01;
            let (lo, hi) = (bb.min(), bb.max());
            let mut v = Vec::new();
            for f in &ms.faces {
                let Some(poly) = &f.surface else { continue };
                let ps = poly.positions();
                if ps
                    .iter()
                    .any(|p| (0..3).any(|i| p[i] < lo[i] - d || p[i] > hi[i] + d))
                {
                    continue;
                }
                for t in poly.faces().triangle_iter() {
                    let q = |k: usize| {
                        let p = ps[t[k].pos];
                        [p[0], p[1], p[2]]
                    };
                    v.push([q(0), q(1), q(2)]);
                }
            }
            slot.insert(v);
        }
        let apply = |p: [f64; 3]| {
            let mut q = [0.0; 3];
            for (i, r) in m.iter().enumerate() {
                q[i] = r[0] * p[0] + r[1] * p[1] + r[2] * p[2] + r[3];
            }
            q
        };
        tris.extend(meshed[&shell].iter().map(|t| t.map(apply)));
    }
    if tris.is_empty() {
        return Err("STEP を三角形にできない".into());
    }
    Ok(lensbench_export::detail::Detail {
        tris,
        step: Some(lensbench_export::step_embed::StepSource {
            text: std::sync::Arc::new(text.to_string()),
            unit_mm: upper_unit,
            shells: all_poses,
        }),
    })
}

/// 詳しい形の読み手（[`lensbench_export::detail::set_loader`] に渡す）。ファイルごとに 1 回だけ読んで覚える
pub fn load_detail(path: &str) -> Option<std::sync::Arc<lensbench_export::detail::Detail>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    type Memo = Mutex<HashMap<String, Option<Arc<lensbench_export::detail::Detail>>>>;
    static MEMO: OnceLock<Memo> = OnceLock::new();
    let memo = MEMO.get_or_init(Default::default);
    if let Some(d) = memo.lock().ok()?.get(path) {
        return d.clone();
    }
    let d = lensbench_core::vfs::read(path)
        .ok()
        .and_then(|b| detail_of_step(&String::from_utf8_lossy(&b)).ok())
        .map(Arc::new);
    memo.lock().ok()?.insert(path.to_string(), d.clone());
    d
}

/// 詳しい形の読み手を使えるようにする（CLI・serve・GUI の起動時）
pub fn install_detail_loader() {
    lensbench_export::detail::set_loader(load_detail);
}

