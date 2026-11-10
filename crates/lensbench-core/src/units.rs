//! 玉・群の単位のパラメータ。
//!
//! 玉（レンズ 1 枚、貼り合わせも 1 玉）や群（ズーム群・フォーカス群など）を、面の範囲の組 [`Unit`] として系に持たせ、
//! 面ごとのパラメータ（[`crate::param`]）と同じ形（`名前.種類`）で変える。種類は [`UnitKind`]:
//!
//! - `dx` `dy` `tx` `ty` `tz`: 玉・群全体を剛体として動かす偏心・傾き [mm, deg]。回転の中心は [`Unit::pivot`]
//!   （最初の範囲の、最初の面の直前の座標系での位置。既定は最初の面の頂点）。点 x は
//!   `pivot + (dx, dy, 0) + R (x − pivot)` へ動く（R = Rx(tx) Ry(ty) Rz(tz)、OpticStudio の座標ブレークの順序 0 と同じ
//!   「偏心のあと X→Y→Z」）。範囲の外の面は動かない（後ろの面は元の位置のまま）。
//!   範囲が 2 つ以上ある玉（ダブルパスの往きと戻りなど、同じ玉を 2 回通る系）は、すべての範囲を空間で同じだけ動かす。
//!   近軸量は、面の傾き偏心と同じく動きを見ない。
//! - `shift`: 群の移動 [mm]（光の進む向きが正）。範囲ごとに、前の間隔（範囲の最初の面の前の面の厚さ）に足し、
//!   後の間隔（範囲の最後の面の厚さ）から引く。厚さは鏡のあとで負になるので、共軸のダブルパスの戻りの範囲でも
//!   同じ式で空間の同じ向きに動く。後の間隔が厚さのソルブならソルブに任せる。物体が無限遠で範囲が面 1 からなら、
//!   前の間隔は表さないので後の間隔だけが変わる。
//! - `dn` `disp`: 範囲の中の媒質（ガラス）すべての屈折率のずれ・分散の相対変化（面ごとの `dn` `disp` と同じ意味）。
//!
//! 面ごとの変更との関係: `shift` `dn` `disp` は範囲の面々のパラメータの組合せそのもので、同じ値を面ごとに与えた系と
//! ビット一致する。偏心・傾きは座標ブレークを範囲の前後に入れた系（[`System::flatten_units`]、.zmx への書き出しで使う）と
//! 丸め誤差の範囲で一致する。dOPL/dp（[`crate::fermat`]）・着地点の微分（[`crate::landing`]）も同じ名前で返す。
//!
//! 玉は .zmx の読込で玉モードの番号（[`crate::elements::split`] の玉だけを数えて 1 から）で `lens1` `lens2` … と
//! 自動で作る。群などは [`System::define_unit`] で作る（ブロックで組む系では、ブロック・群の id で作られる）。
//! 玉・群が入れ子になってよい（範囲は互いに含むか離れているか）。入れ子では内側を先に動かし、外側がそれを
//! まとめて動かす。同じ範囲なら先に定義した方が内側。

use crate::error::Result;
use crate::math::*;
use crate::surface::{After, SurfType, Surface, Tilt};
use crate::system::{Frame, System};
use crate::{invalid, notimpl};

/// 玉・群のパラメータの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum UnitKind {
    Dx,
    Dy,
    /// 傾き [deg]
    Tx,
    Ty,
    Tz,
    /// 群の移動（前の間隔に足し、後の間隔から引く）
    Shift,
    /// 範囲の中の媒質の屈折率のずれ
    Dn,
    /// 範囲の中の媒質の分散の相対変化
    Disp,
}

impl UnitKind {
    pub fn parse(s: &str) -> Result<UnitKind> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "dx" => UnitKind::Dx,
            "dy" => UnitKind::Dy,
            "tx" => UnitKind::Tx,
            "ty" => UnitKind::Ty,
            "tz" => UnitKind::Tz,
            "shift" => UnitKind::Shift,
            "dn" => UnitKind::Dn,
            "disp" => UnitKind::Disp,
            _ => invalid!("玉・群のパラメータの種類 {s}（dx dy tx ty tz shift dn disp）"),
        })
    }

    pub fn name(&self) -> &'static str {
        match self {
            UnitKind::Dx => "dx",
            UnitKind::Dy => "dy",
            UnitKind::Tx => "tx",
            UnitKind::Ty => "ty",
            UnitKind::Tz => "tz",
            UnitKind::Shift => "shift",
            UnitKind::Dn => "dn",
            UnitKind::Disp => "disp",
        }
    }

    /// 剛体の動き（dx dy tx ty tz）の値の位置
    pub fn motion_index(&self) -> Option<usize> {
        match self {
            UnitKind::Dx => Some(0),
            UnitKind::Dy => Some(1),
            UnitKind::Tx => Some(2),
            UnitKind::Ty => Some(3),
            UnitKind::Tz => Some(4),
            _ => None,
        }
    }
}

/// 玉・群: 面の範囲の組と、その剛体の動き・移動
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Unit {
    pub name: String,
    /// 面の範囲 [最初, 最後]（両端を含む、光の通る順）
    pub spans: Vec<[usize; 2]>,
    /// 回転の中心（最初の範囲の最初の面の直前の座標系、その面の頂点が原点）
    pub pivot: [f64; 3],
    /// 剛体の動き dx dy tx ty tz [mm, deg]
    pub motion: [f64; 5],
    /// 群の移動 [mm]（今の厚さに入っている分）
    pub shift: f64,
    /// 読込で作った玉（lens1 …）
    pub auto: bool,
}

impl Unit {
    pub fn moved(&self) -> bool {
        self.motion.iter().any(|&v| v != 0.0)
    }

    /// 範囲の面の数の合計（入れ子の順を決める）
    fn coverage(&self) -> usize {
        self.spans.iter().map(|s| s[1] + 1 - s[0]).sum()
    }

    pub fn contains(&self, j: usize) -> bool {
        self.spans.iter().any(|s| s[0] <= j && j <= s[1])
    }

    /// 入口の座標系での剛体の動き x → R x + t（R と t）
    pub fn local(&self) -> (M3, V3) {
        let [dx, dy, tx, ty, tz] = self.motion;
        let r = mm(&mm(&rot(0, tx), &rot(1, ty)), &rot(2, tz));
        let p = self.pivot;
        (r, sub(add(p, [dx, dy, 0.0]), mv(&r, p)))
    }

    /// 剛体の動きの、motion[i] による微分（dR と dt）
    pub(crate) fn local_d(&self, i: usize) -> (M3, V3) {
        let [_, _, tx, ty, tz] = self.motion;
        let p = self.pivot;
        if i < 2 {
            let mut d = [0.0; 3];
            d[i] = 1.0;
            return ([[0.0; 3]; 3], d);
        }
        let (rx, ry, rz) = (rot(0, tx), rot(1, ty), rot(2, tz));
        let dr = match i {
            2 => mm(&mm(&drot(0, tx), &ry), &rz),
            3 => mm(&mm(&rx, &drot(1, ty)), &rz),
            _ => mm(&mm(&rx, &ry), &drot(2, tz)),
        };
        (dr, scale(mv(&dr, p), -1.0))
    }
}

/// 回転行列（軸 ax の周りに a [deg]）。system の座標ブレークと同じ
fn rot(ax: usize, a: f64) -> M3 {
    crate::math::rot(ax, a)
}

/// 回転行列の角度 [deg] による微分
pub(crate) fn drot(ax: usize, a_deg: f64) -> M3 {
    let a = a_deg.to_radians();
    let (s, c) = a.sin_cos();
    let k = std::f64::consts::PI / 180.0;
    let m = match ax {
        0 => [[0.0, 0.0, 0.0], [0.0, -s, -c], [0.0, c, -s]],
        1 => [[-s, 0.0, c], [0.0, 0.0, 0.0], [-c, 0.0, -s]],
        _ => [[-s, -c, 0.0], [c, -s, 0.0], [0.0, 0.0, 0.0]],
    };
    m.map(|r| r.map(|v| v * k))
}

/// 動かす順（内側が先: 範囲の面の数が少ない順、同じなら先に定義した順）。動いていない玉・群は除く
pub(crate) fn order(units: &[Unit]) -> Vec<usize> {
    let mut v: Vec<usize> = (0..units.len()).filter(|&i| units[i].moved()).collect();
    v.sort_by_key(|&i| (units[i].coverage(), i));
    v
}

/// 入口の座標系 e（物体に固定した座標）で剛体の動き (R, t) を、大域の写像 X → c + Q X にする
pub(crate) fn global(e: &Frame, r: &M3, t: V3) -> (V3, M3) {
    let et = transpose(&e.r);
    let q = mm(&mm(&e.r, r), &et);
    let c = sub(add(e.o, mv(&e.r, t)), mv(&q, e.o));
    (c, q)
}

/// 面ごとの座標系（物体に固定した座標、動かす前）に、玉・群の動きをかける。entry[u][k] は玉・群 u の範囲 k の入口
pub(crate) fn apply_frames(
    units: &[Unit],
    entry: &[Vec<Option<Frame>>],
    out: &mut [Option<Frame>],
) {
    for u in order(units) {
        let Some(e) = entry[u][0] else { continue };
        let (r, t) = units[u].local();
        let (c, q) = global(&e, &r, t);
        for s in &units[u].spans {
            for f in out[s[0]..=s[1]].iter_mut().flatten() {
                *f = Frame {
                    o: add(c, mv(&q, f.o)),
                    r: mm(&q, &f.r),
                };
            }
        }
    }
}

impl System {
    /// 玉・群を名前（大文字小文字は区別しない）で探す
    pub fn find_unit(&self, name: &str) -> Option<usize> {
        let t = name.trim();
        self.units
            .iter()
            .position(|u| u.name.eq_ignore_ascii_case(t))
    }

    /// 玉・群を定義する（同じ名前の自動の玉は置き換える）。範囲は 1 ≤ 最初 ≤ 最後 < 像面、互いに重ならない。
    /// ほかの玉・群とは、範囲が互いに含むか離れているかのどちらか
    pub fn define_unit(
        &mut self,
        name: &str,
        spans: &[[usize; 2]],
        pivot: [f64; 3],
    ) -> Result<usize> {
        let name = name.trim();
        if name.is_empty() || name.contains('.') || name.parse::<usize>().is_ok() {
            invalid!("玉・群の名前 {name:?}（空・数字だけ・「.」を含む名前は使えない）");
        }
        if spans.is_empty() {
            invalid!("玉・群 {name} の範囲がない");
        }
        let ima = self.image();
        let mut sp = spans.to_vec();
        for s in &sp {
            if s[0] == 0 || s[0] > s[1] || s[1] >= ima {
                invalid!(
                    "玉・群 {name} の範囲 {s:?}（面 1〜{} の中で 最初 ≤ 最後）",
                    ima - 1
                );
            }
        }
        sp.sort();
        for w in sp.windows(2) {
            if w[1][0] <= w[0][1] {
                invalid!("玉・群 {name} の範囲 {:?} と {:?} が重なる", w[0], w[1]);
            }
        }
        if !pivot.iter().all(|v| v.is_finite()) {
            invalid!("玉・群 {name} の回転の中心 {pivot:?}");
        }
        if let Some(i) = self.find_unit(name) {
            if !self.units[i].auto {
                invalid!("玉・群 {name} は定義済み");
            }
            self.units.remove(i);
        }
        for u in &self.units {
            for a in &sp {
                for b in &u.spans {
                    let apart = a[1] < b[0] || b[1] < a[0];
                    let nested = (a[0] <= b[0] && b[1] <= a[1]) || (b[0] <= a[0] && a[1] <= b[1]);
                    if !apart && !nested {
                        invalid!(
                            "玉・群 {name} の範囲 {a:?} が {} の範囲 {b:?} と一部だけ重なる",
                            u.name
                        );
                    }
                }
            }
        }
        // 範囲は定義の順（光の通る順）のまま持つ
        self.units.push(Unit {
            name: name.to_string(),
            spans: spans.to_vec(),
            pivot,
            motion: [0.0; 5],
            shift: 0.0,
            auto: false,
        });
        self.invalidate_frames();
        Ok(self.units.len() - 1)
    }

    /// 玉・群の名前を変える（読込で作った玉も、変えれば利用者の定義になる）
    pub fn rename_unit(&mut self, u: usize, name: &str) -> Result<()> {
        let name = name.trim();
        if u >= self.units.len() {
            invalid!("玉・群 {u}");
        }
        if name.is_empty() || name.contains('.') || name.parse::<usize>().is_ok() {
            invalid!("玉・群の名前 {name:?}（空・数字だけ・「.」を含む名前は使えない）");
        }
        if self.find_unit(name).is_some_and(|i| i != u) {
            invalid!("玉・群 {name} は定義済み");
        }
        self.units[u].name = name.to_string();
        self.units[u].auto = false;
        Ok(())
    }

    /// 玉・群の範囲を変える（並びの位置・名前・回転の中心はそのまま）。動き・移動が 0 の玉・群だけ
    pub fn set_unit_spans(&mut self, u: usize, spans: &[[usize; 2]]) -> Result<()> {
        let Some(cur) = self.units.get(u).cloned() else {
            invalid!("玉・群 {u}");
        };
        if cur.moved() || cur.shift != 0.0 {
            invalid!(
                "玉・群 {} は動かしてある（動き・移動を 0 にしてから範囲を変える）",
                cur.name
            );
        }
        let mut t = self.clone();
        t.units.remove(u);
        t.define_unit(&cur.name, spans, cur.pivot)?;
        let mut nu = t.units.pop().expect("定義した玉・群");
        nu.auto = false;
        t.units.insert(u, nu);
        self.units = t.units;
        self.invalidate_frames();
        Ok(())
    }

    /// 玉・群を外す（多重構成の表のその玉・群の行も外す）。動き・移動が 0 の玉・群だけ。説明を返す
    pub fn remove_unit(&mut self, u: usize) -> Result<Vec<String>> {
        let Some(cur) = self.units.get(u).cloned() else {
            invalid!("玉・群 {u}");
        };
        if cur.moved() || cur.shift != 0.0 {
            invalid!(
                "玉・群 {} は動かしてある（動き・移動を 0 にしてから外す）",
                cur.name
            );
        }
        let mut notes = Vec::new();
        if let Some(c) = &mut self.configs {
            c.rows.retain(|r| {
                let used = matches!(r.param.kind, crate::param::Kind::Unit(_)) && r.param.surf == u;
                if used {
                    notes.push(format!("多重構成の行 {} を外した", r.param.name()));
                }
                !used
            });
            for r in c.rows.iter_mut() {
                if matches!(r.param.kind, crate::param::Kind::Unit(_)) && r.param.surf > u {
                    r.param.surf -= 1;
                }
            }
        }
        self.units.remove(u);
        self.invalidate_frames();
        Ok(notes)
    }

    /// 回転の中心を変える
    pub fn set_unit_pivot(&mut self, u: usize, pivot: [f64; 3]) -> Result<()> {
        if u >= self.units.len() || !pivot.iter().all(|v| v.is_finite()) {
            invalid!("玉・群 {u} の回転の中心 {pivot:?}");
        }
        self.units[u].pivot = pivot;
        self.invalidate_frames();
        Ok(())
    }

    /// 読込で作る玉 lens1, lens2, …（玉モードの番号、[`crate::elements::split`]）。玉に分けられない系では空
    pub fn lens_units(&self) -> Vec<Unit> {
        let Ok(els) = crate::elements::split(self) else {
            return Vec::new();
        };
        els.iter()
            .filter(|e| e.kind == crate::elements::Kind::Lens)
            .enumerate()
            .map(|(k, e)| Unit {
                name: format!("lens{}", k + 1),
                spans: vec![[e.from, e.from + e.surfaces.len() - 1]],
                pivot: [0.0; 3],
                motion: [0.0; 5],
                shift: 0.0,
                auto: true,
            })
            .collect()
    }

    /// 玉・群の範囲の中の、媒質を持つ面（面の後がガラス。鏡・座標ブレーク・無視する面は除く）
    pub fn unit_media(&self, u: usize) -> Vec<usize> {
        let air = crate::elements::air_after(self);
        let mut v = Vec::new();
        for s in &self.units[u].spans {
            for j in s[0]..=s[1] {
                let d = &self.surfaces[j];
                if !air[j] && !d.mirror && d.typ != SurfType::CoordBrk && !d.ignore {
                    v.push(j);
                }
            }
        }
        v
    }

    /// 玉・群の剛体の動きを、範囲の前後に入れた座標ブレークにした系（面の番号が変わる。ピックアップ・ソルブ・
    /// 玉・群の範囲は付け替える）。座標ブレークはなるべく厚さを持たせず偏心で動かすので、厚さの和（近軸量）は動かす前の
    /// 系と同じ。ただし回転が光軸を傾けないのに光軸方向へずらす必要がある場合（範囲の中に戻さない傾きがある玉の偏心など）は
    /// 座標ブレークに厚さが付き、近軸量がその分だけ動かした系（近軸量は動きを見ない）と違う。2 つ目からの
    /// 範囲の入口の向きが最初の範囲と違う玉・群は未対応
    pub fn flatten_units(&self) -> Result<System> {
        let mut s = self.clone();
        loop {
            let ord = order(&s.units);
            let Some(&u) = ord.first() else { break };
            let mut still = s.clone();
            still.units.iter_mut().for_each(|x| x.motion = [0.0; 5]);
            let fr = raw_frames(&still);
            let unit = s.units[u].clone();
            let (r, t) = unit.local();
            let first = unit.spans[0];
            let Some(e0) = fr.entry[u][0] else {
                invalid!("玉・群 {} の入口の座標系がない", unit.name);
            };
            let (c, q) = global(&e0, &r, t);
            // 後ろの範囲から入れる（前の範囲の番号がずれない）
            let mut spans: Vec<(usize, [usize; 2])> =
                unit.spans.iter().copied().enumerate().collect();
            spans.sort_by_key(|x| std::cmp::Reverse(x.1[0]));
            for (k, [a, b]) in spans {
                let Some(e) = fr.entry[u][k] else {
                    invalid!(
                        "玉・群 {} の範囲 {:?} の入口の座標系がない",
                        unit.name,
                        [a, b]
                    );
                };
                // この範囲の入口の座標での動き: x → R2 x + t2
                let et = transpose(&e.r);
                let r2 = mm(&mm(&et, &q), &e.r);
                let t2 = mtv(&e.r, sub(add(c, mv(&q, e.o)), e.o));
                if k > 0 && !close_m(&r2, &r) {
                    notimpl!(
                        "玉・群 {} の範囲 {:?}: 入口の向きが最初の範囲 {:?} と違う（座標ブレークにできない）",
                        unit.name,
                        [a, b],
                        first
                    );
                }
                // 範囲の中を通る変換（入口 → 最後の面の後）K: x → Rk x + k
                let Some(fb) = fr.post[b] else {
                    invalid!("面 {b} の座標系がない");
                };
                let rk = mm(&et, &fb.r);
                let k_ = mtv(&e.r, sub(fb.o, e.o));
                // 戻り U = K⁻¹ T⁻¹ K: x → Ru x + u（Ru = Rkᵀ Rᵀ Rk、u = Rkᵀ (Rᵀ (k − t2) − k)）。
                // 厚さのない座標ブレークだけで動かす（近軸の厚さの和を変えない: 近軸量は動かす前の系と同じ）
                let rt = transpose(&r);
                let ru = mm(&mm(&transpose(&rk), &rt), &rk);
                let u_ = mtv(&rk, sub(mv(&rt, sub(k_, t2)), k_));
                let Some(bang) = euler_xyz(&ru) else {
                    notimpl!(
                        "玉・群 {} の範囲 {:?}: 戻りの回転を座標ブレークの角にできない",
                        unit.name,
                        [a, b]
                    );
                };
                let tb = s.surfaces[b].t;
                let ang = [unit.motion[2], unit.motion[3], unit.motion[4]];
                let cb = |d: [f64; 5], order: u8, t: f64| Surface {
                    typ: SurfType::CoordBrk,
                    mat: "-".into(),
                    cb: d,
                    cb_order: order,
                    t,
                    ..Default::default()
                };
                // 行き: 偏心 (a, b) と回転 R、回した座標で偏心 (c, d)
                let (g1, g2, gz) = split_shift(&r, t2);
                let go = vec![
                    cb([g1[0], g1[1], ang[0], ang[1], ang[2]], 0, gz),
                    cb([g2[0], g2[1], 0.0, 0.0, 0.0], 0, 0.0),
                ];
                // 戻り: 偏心 (a, b) と回転 Ru、回した座標で偏心 (c, d)、その後に元の厚さ
                let (h1, h2, hz) = split_shift(&ru, u_);
                let back = vec![
                    cb([h1[0], h1[1], bang[0], bang[1], bang[2]], 0, hz),
                    cb([h2[0], h2[1], 0.0, 0.0, 0.0], 0, tb),
                ];
                s.surfaces[b].t = 0.0;
                let nb = back.len();
                insert_surfaces(&mut s, b + 1, back, InsertAt::After);
                // 面 b の厚さ（後の間隔）は戻りの最後の座標ブレークへ移したので、そのソルブ・ピックアップも移す
                let to = b + nb;
                for q in s.solves.iter_mut() {
                    if q.surf == b && !q.kind.is_curv() {
                        q.surf = to;
                    }
                    // 面 b の厚さを参照する補償のソルブ
                    if q.kind == crate::param::SolveKind::Compensator(b) {
                        q.kind = q.kind.with_other_surface(to);
                    }
                }
                for p in s.pickups.iter_mut() {
                    use crate::param::PickKind::Thick;
                    if p.src == b && p.source() == Thick {
                        p.src = to;
                    }
                    if p.dst == b && p.what == Thick {
                        p.dst = to;
                    }
                }
                insert_surfaces(&mut s, a, go, InsertAt::Before);
            }
            s.units[u].motion = [0.0; 5];
        }
        // 座標ブレークの媒質は前の媒質
        s.fill_cb_indices();
        s.changed()?;
        Ok(s)
    }

    /// 座標ブレークの面の n を前の媒質にする
    fn fill_cb_indices(&mut self) {
        let mut prev = self.surfaces[0].n.clone();
        for d in self.surfaces.iter_mut().skip(1) {
            if d.typ == SurfType::CoordBrk {
                d.n = prev.clone();
            }
            if !d.ignore {
                prev = d.n.clone();
            }
        }
    }
}

/// 平行移動 v を 2 つの偏心に分ける: v = (a, b, 0) + R (c, d, 0) + (0, 0, τ)。z の分は、R が z 軸を傾けるなら
/// 回した座標の偏心で出し（τ = 0、厚さを足さない）、傾けないなら厚さ τ で出す（R は z 軸を保つので順は問わない）
fn split_shift(r: &M3, v: V3) -> ([f64; 2], [f64; 2], f64) {
    let (p, q) = (r[2][0], r[2][1]);
    let n2 = p * p + q * q;
    // 偏心が z の分より桁違いに大きくなる（ほとんど傾けない）なら厚さで
    if n2 > 0.0 && v[2].abs() <= 1e6 * n2.sqrt() * (1.0 + norm(v)) {
        let cd = [v[2] * p / n2, v[2] * q / n2];
        let w = mv(r, [cd[0], cd[1], 0.0]);
        ([v[0] - w[0], v[1] - w[1]], cd, v[2] - w[2])
    } else {
        ([v[0], v[1]], [0.0, 0.0], v[2])
    }
}

/// 回転 m を Rx(a) Ry(b) Rz(g)（座標ブレークの順序 0）の角 [deg] にする。作り直して合わなければ None
fn euler_xyz(m: &M3) -> Option<[f64; 3]> {
    let b = m[0][2].clamp(-1.0, 1.0).asin();
    let a = (-m[1][2]).atan2(m[2][2]);
    let g = (-m[0][1]).atan2(m[0][0]);
    let d = [a.to_degrees(), b.to_degrees(), g.to_degrees()];
    let r = mm(&mm(&rot(0, d[0]), &rot(1, d[1])), &rot(2, d[2]));
    close_m(&r, m).then_some(d)
}

fn close_m(a: &M3, b: &M3) -> bool {
    (0..3).all(|i| (0..3).all(|j| (a[i][j] - b[i][j]).abs() <= 1e-13))
}

#[derive(Clone, Copy, PartialEq)]
enum InsertAt {
    /// 範囲の最初の面の前（範囲の最初にする）
    Before,
    /// 範囲の最後の面の後（範囲の最後にする）
    After,
}

/// 面 at に面 v を入れ、ピックアップ・ソルブ・玉・群の面番号を付け替える。
/// 範囲の端が at に当たるとき、Before なら入れた面を範囲の最初に、After なら範囲の最後に含める
fn insert_surfaces(s: &mut System, at: usize, v: Vec<Surface>, how: InsertAt) {
    let n = v.len();
    let m = |j: usize| if j >= at { j + n } else { j };
    s.surfaces.splice(at..at, v);
    for p in s.pickups.iter_mut() {
        p.src = m(p.src);
        p.dst = m(p.dst);
    }
    for q in s.solves.iter_mut() {
        q.surf = m(q.surf);
        if let Some(o) = q.kind.other_surface() {
            q.kind = q.kind.with_other_surface(m(o));
        }
    }
    for u in s.units.iter_mut() {
        for sp in u.spans.iter_mut() {
            let [a, b] = *sp;
            let a2 = if how == InsertAt::Before && a == at {
                a
            } else {
                m(a)
            };
            let b2 = if how == InsertAt::After && b + 1 == at {
                b + n
            } else {
                m(b)
            };
            *sp = [a2, b2];
        }
    }
    if s.glrs >= at {
        s.glrs += n;
    }
}

/// 物体に固定した座標での面ごとの座標系（動かす前）と、玉・群の範囲の入口、面の後（厚さの前）の座標系
pub(crate) struct RawFrames {
    pub surf: Vec<Option<Frame>>,
    pub entry: Vec<Vec<Option<Frame>>>,
    pub post: Vec<Option<Frame>>,
}

pub(crate) fn raw_frames(s: &System) -> RawFrames {
    let sf = &s.surfaces;
    let mut o: V3 = [0.0; 3];
    let mut r: M3 = I3;
    let mut out: Vec<Option<Frame>> = vec![None; sf.len()];
    let mut post: Vec<Option<Frame>> = vec![None; sf.len()];
    let mut entry: Vec<Vec<Option<Frame>>> =
        s.units.iter().map(|u| vec![None; u.spans.len()]).collect();
    let t0 = sf[0].t;
    if t0.is_finite() {
        out[0] = Some(Frame {
            o: [0.0, 0.0, -t0],
            r: I3,
        });
    }
    for (j, d) in sf.iter().enumerate().skip(1) {
        for (u, x) in s.units.iter().enumerate() {
            for (k, sp) in x.spans.iter().enumerate() {
                if sp[0] == j {
                    entry[u][k] = Some(Frame { o, r });
                }
            }
        }
        if d.ignore {
            continue;
        }
        if d.typ == SurfType::CoordBrk {
            (o, r) = crate::system::apply_cb(
                o,
                r,
                &Tilt {
                    d: d.cb,
                    order: d.cb_order,
                },
            );
            out[j] = Some(Frame { o, r });
        } else {
            let (o_pre, r_pre) = (o, r);
            if let Some(b) = &d.before {
                (o, r) = crate::system::apply_cb(o, r, b);
            }
            out[j] = Some(Frame { o, r });
            match &d.after {
                After::Reverse => {
                    o = o_pre;
                    r = r_pre;
                }
                After::Tilt(a) => (o, r) = crate::system::apply_cb(o, r, a),
                After::None => {}
            }
        }
        post[j] = Some(Frame { o, r });
        o = add(o, scale(col(&r, 2), d.t));
    }
    RawFrames {
        surf: out,
        entry,
        post,
    }
}
