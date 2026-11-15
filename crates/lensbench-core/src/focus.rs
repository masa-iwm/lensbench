//! 像面だけをずらした系の光線を、ずらす前の系の追跡から作る（スルーフォーカス・RMS vs Focus）。
//!
//! 像面を光軸の向きにずらしても、像面の手前の面までの光線は変わらない。[`crate::sweep::defocus_shared`] で
//! 作った系の追跡（像面まで・記録なし）は、ずらす前の系で追った光線を共有の表から引き、像面の平面まで
//! 直線で進めるだけにする。表は同じ掃引のずらしの点で共有するので、同じ瞳の格子（補間の格子・OPD の傾きの
//! 見積もりの 33×33 など）は 1 度しか追わない。値は像面の系を追い直すのと丸めの差だけ違う。
//!
//! 使えるのは、像面が平らで開口がなく、ずらした像面の座標系がずらす前の像面の座標系を局所の z の向きに
//! 平行に動かしたものになっている系だけ（像面自身の傾き・偏心があれば使わない。像面の手前の媒質が GRIN の系も
//! 使わない）。視野は角度か物体高（実像高・近軸像高・条件で定める視野は像面で決まるので使わない）。
//! アフォーカル像空間も使わない。

use crate::error::Result;
use crate::math::*;
use crate::surface::{Aperture, SurfType};
use crate::system::{FieldSel, FieldType, System, Trace};
use crate::LockExt;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// 表に置く光線の本数の上限（1 本およそ 210 バイト、上限で 110 MB ほど）。超えた分は表に置かずに毎回追う
const MAX_RAYS: usize = 1 << 19;

/// ずらす前の系で追った 1 回分（瞳座標の並びごと）
struct Entry {
    px: Vec<f64>,
    py: Vec<f64>,
    trace: Arc<Trace>,
}

/// 表の鍵: 視野 (hx, hy)・波長・本数・瞳座標の並びのハッシュ
type Key = (u64, u64, usize, usize, u64);

/// ずらす前の系と、掃引の点で共有する光線の表
pub(crate) struct FocusBase {
    base: System,
    rays: Mutex<HashMap<Key, Vec<Entry>>>,
    count: AtomicUsize,
}

impl std::fmt::Debug for FocusBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FocusBase").finish_non_exhaustive()
    }
}

/// ずらした系 1 つ分: 共有の表と、ずらす前の像面の局所座標でのずらした像面の位置 z
#[derive(Debug, Clone)]
pub(crate) struct FocusShift {
    pub(crate) base: Arc<FocusBase>,
    pub(crate) z: f64,
}

/// 瞳座標の並びのハッシュ（FNV-1a を 64 ビットの語ごとに）
fn hash_xy(px: &[f64], py: &[f64]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for v in px.iter().chain(py) {
        h = (h ^ v.to_bits()).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

impl FocusBase {
    /// 使えない系（モジュールの説明）なら None
    pub(crate) fn new(s: &System) -> Option<Arc<FocusBase>> {
        let im = s.surfaces.last()?;
        // 像面の手前の媒質（座標ブレークを飛ばした有効な面の後ろ）が GRIN なら光線は直線に進まない
        let grin = s.surfaces[..s.image()]
            .iter()
            .rev()
            .find(|d| !d.ignore && d.typ != SurfType::CoordBrk)
            .is_some_and(|d| d.grin.is_some());
        let ok = !grin
            && matches!(s.field_type, FieldType::Angle | FieldType::ObjH)
            && !s.afocal
            && im.typ == SurfType::Standard
            && im.c == 0.0
            && !im.ignore
            && !im.mirror
            && matches!(im.ap, Aperture::None)
            && s.frames().surf[s.image()].is_some();
        ok.then(|| {
            Arc::new(FocusBase {
                base: s.clone(),
                rays: Mutex::new(HashMap::new()),
                count: AtomicUsize::new(0),
            })
        })
    }

    /// ずらした系 d（像面の手前の面の厚さだけを変えて changed() を呼んだもの）の、ずらす前の像面の局所座標での
    /// 像面の位置 z。座標系が平行移動でなければ None
    pub(crate) fn shift_of(&self, d: &System) -> Option<f64> {
        let (j, b) = (d.image(), &self.base);
        let (f0, f1) = (b.frames().surf[j].as_ref()?, d.frames().surf[j].as_ref()?);
        if f0.r != f1.r {
            return None;
        }
        let v = mtv(&f0.r, sub(f1.o, f0.o));
        let tol = 1e-12 * (1.0 + v[2].abs() + f0.o.iter().map(|x| x.abs()).sum::<f64>());
        (v[0].abs() <= tol && v[1].abs() <= tol).then_some(v[2])
    }
}

impl FocusShift {
    /// d（この掃引の点の系）の視野 field の光線 (px, py) を、像面まで記録なしで追った結果
    pub(crate) fn trace(
        &self,
        d: &System,
        field: FieldSel,
        hxy: (f64, f64),
        px: &[f64],
        py: &[f64],
        w: usize,
    ) -> Result<Trace> {
        let fb = &self.base;
        let key = (
            hxy.0.to_bits(),
            hxy.1.to_bits(),
            w,
            px.len(),
            hash_xy(px, py),
        );
        let same = |e: &Entry| e.px == px && e.py == py;
        let hit = fb
            .rays
            .lock_ok()
            .get(&key)
            .and_then(|v| v.iter().find(|e| same(e)).map(|e| e.trace.clone()));
        let base = match hit {
            Some(t) => t,
            None => {
                let t = Arc::new(fb.base.try_trace(field, px, py, Some(w), None, false)?);
                if fb.count.fetch_add(px.len(), Ordering::Relaxed) + px.len() <= MAX_RAYS {
                    fb.rays.lock_ok().entry(key).or_default().push(Entry {
                        px: px.to_vec(),
                        py: py.to_vec(),
                        trace: t.clone(),
                    });
                } else {
                    fb.count.fetch_sub(px.len(), Ordering::Relaxed);
                }
                t
            }
        };
        let j = d.image();
        let fr = d.frames().surf[j].expect("像面の座標系");
        let n = d.image_index(w);
        let z = self.z;
        let mut t = Trace {
            rays: base.rays.clone(),
            parity: base.parity,
            p0: base.p0.clone(),
            k0: base.k0.clone(),
            opl_os: base.opl_os.clone(),
            rec: Vec::new(),
            last: j,
            hx_hy: hxy,
        };
        for (i, r) in t.rays.iter_mut().enumerate() {
            if r.err == 0 {
                // ずらす前の像面（z = 0）からずらした像面（z）まで直線で進め、ずらした像面の局所座標に直す
                let tt = (z - r.xyz[2]) / r.lmn[2];
                r.xyz = [r.xyz[0] + r.lmn[0] * tt, r.xyz[1] + r.lmn[1] * tt, 0.0];
                r.p = add(fr.o, mv(&fr.r, r.xyz));
                r.opl += n * tt;
                if let Some(o) = t.opl_os.get_mut(i) {
                    *o += n * tt;
                }
            }
        }
        Ok(t)
    }
}
