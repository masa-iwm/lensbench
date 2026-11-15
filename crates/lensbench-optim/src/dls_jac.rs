//! ヤコビアン 1 回の中で項の間で共有する計算（変数をずらした系・開始の光線・射出瞳の微分）

use super::{launch_of, typical, waves, TermKind};
use lensbench_core::error::Result;
use lensbench_core::param::Param;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::LockExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// ヤコビアン 1 回の中で項の間で共有する計算: 変数をずらした系・開始の光線・[`JacCache::launch_moves`] の答え・射出瞳の微分。
/// どれも項ごとに求めたときと同じ値（同じ計算を 1 回にするだけ）
pub(super) struct JacCache {
    nv: usize,
    /// (構成, 変数) ごとの、変数を刻みだけずらした系（作れなければ None）
    shifted: Vec<OnceLock<Option<System>>>,
    /// (構成, 視野, 波長) ごとの元の系の開始の光線（求まらなければ None）
    launch: Mutex<Vec<(LaunchKey, Launch)>>,
    /// (構成, 変数, 視野, 波長の指定) ごとの [`JacCache::launch_moves`] の答え
    moves: Mutex<Vec<(MovesKey, bool)>>,
    /// 構成ごとの射出瞳の微分
    pub(super) pupil: Vec<PupilCache>,
}

type LaunchKey = (usize, FieldSel, usize);
/// 開始の光線（求まらなければ None）
type Launch = Option<Arc<Vec<f64>>>;
type MovesKey = (usize, usize, FieldSel, Option<usize>);

impl JacCache {
    pub(super) fn new(nc: usize, nv: usize) -> JacCache {
        JacCache {
            nv,
            shifted: (0..nc * nv).map(|_| OnceLock::new()).collect(),
            launch: Mutex::new(Vec::new()),
            moves: Mutex::new(Vec::new()),
            pupil: (0..nc).map(|_| Mutex::new(HashMap::new())).collect(),
        }
    }

    fn base_launch(
        &self,
        c: usize,
        s: &System,
        field: FieldSel,
        w: usize,
    ) -> Option<Arc<Vec<f64>>> {
        let key = (c, field, w);
        if let Some((_, v)) = self.launch.lock_ok().iter().find(|e| e.0 == key) {
            return v.clone();
        }
        let v = launch_of(s, field, w).ok().map(Arc::new);
        self.launch.lock_ok().push((key, v.clone()));
        v
    }

    /// 構成 c の系 s で、変数 q（パラメータ p、値 v）を h ずらすと、項 k の光線の始め方が変わるか。求まらなければ
    /// 変わるとみなす
    #[allow(clippy::too_many_arguments)]
    pub(super) fn launch_moves(
        &self,
        c: usize,
        s: &System,
        k: &TermKind,
        q: usize,
        p: Param,
        v: f64,
        h: f64,
    ) -> bool {
        let (field, wave) = match k {
            TermKind::Spot { field, wave, .. }
            | TermKind::Wavefront { field, wave, .. }
            | TermKind::Centroid { field, wave, .. }
            | TermKind::Ray { field, wave, .. }
            | TermKind::Opd { field, wave, .. } => (*field, *wave),
            _ => return true,
        };
        let key = (c, q, field, wave);
        if let Some((_, m)) = self.moves.lock_ok().iter().find(|e| e.0 == key) {
            return *m;
        }
        let s2 =
            self.shifted[c * self.nv + q].get_or_init(|| s.with_params_fast(&[(p, v + h)]).ok());
        let m = match s2 {
            None => true,
            Some(s2) => launch_differs(waves_with_primary(s, wave), |w| {
                Some((
                    self.base_launch(c, s, field, w)?,
                    launch_of(s2, field, w).ok()?,
                ))
            }),
        };
        self.moves.lock_ok().push((key, m));
        m
    }
}

/// 波長の指定 wave の波長に主波長を足したもの
fn waves_with_primary(s: &System, wave: Option<usize>) -> Vec<usize> {
    let mut ws = waves(s, wave);
    if !ws.contains(&s.primary) {
        ws.push(s.primary);
    }
    ws
}

/// 波長ごとに元の系と変えた系の開始の光線を比べ、どれかが違えば true（求まらなければ true）
fn launch_differs(
    ws: Vec<usize>,
    mut pair: impl FnMut(usize) -> Option<(Arc<Vec<f64>>, Vec<f64>)>,
) -> bool {
    for w in ws {
        let Some((a, b)) = pair(w) else {
            return true;
        };
        let scale = a.iter().fold(1.0f64, |m, x| m.max(x.abs()));
        if a.iter()
            .zip(&b)
            .any(|(x, y)| !((x - y).abs() <= 1e-12 * scale))
        {
            return true;
        }
    }
    false
}

/// 射出瞳の位置の微分と、像空間の屈折率の微分を求めるための ± にずらした系（パラメータ p での中心差分）
pub(super) struct PupilD {
    h: f64,
    pub(super) de: f64,
    a: System,
    b: System,
}

impl PupilD {
    /// 像空間の屈折率（波長 w）の微分
    pub(super) fn dn(&self, w: usize) -> f64 {
        (self.a.image_index(w) - self.b.image_index(w)) / (2.0 * self.h)
    }
}

/// 構成ごとの [`PupilD`] の共有（射出瞳・像空間の屈折率は視野・光線によらないので、ヤコビアン 1 回の中で
/// 項の間で (構成, 変数) ごとに 1 回だけ求める）
pub(super) type PupilCache = Mutex<HashMap<Param, Arc<PupilD>>>;

/// 射出瞳の位置と像空間の屈折率の、パラメータ p での微分（中心差分）。pc があればそこで共有する
pub(super) fn pupil_d(s: &System, p: Param, pc: Option<&PupilCache>) -> Result<Arc<PupilD>> {
    if let Some(d) = pc.and_then(|c| c.lock_ok().get(&p).cloned()) {
        return Ok(d);
    }
    let v = s.get_param(p)?;
    // 回転対称な系の射出瞳は近軸で滑らか。回転対称でない系は実光線の外挿で求めるので 1e-8 程度の
    // 揺れがあり、刻みを大きめにする（中心差分なので、偏心 0 で近軸と実光線の求め方が切り替わる分も
    // 打ち消し合う）
    let at = |h: f64| -> Result<(System, System)> {
        Ok((
            s.with_params_fast(&[(p, v + h)])?,
            s.with_params_fast(&[(p, v - h)])?,
        ))
    };
    let sc = v.abs().max(typical(p.kind));
    let mut h = 1e-6 * sc;
    let (mut a, mut b) = at(h)?;
    if !(a.symmetric() && b.symmetric()) {
        h = 1e-3 * sc;
        (a, b) = at(h)?;
    }
    let de = (a.exit_pupil() - b.exit_pupil()) / (2.0 * h);
    let d = Arc::new(PupilD { h, de, a, b });
    if let Some(c) = pc {
        c.lock_ok().insert(p, d.clone());
    }
    Ok(d)
}
