//! 瞳の格子の光線の追跡と画素への補間（干渉の解析で共通）: 1 本の経路の光線の格子（[`RayGrid`]）、検出面の画素への
//! 三角形の補間（[`rasterize`]）、ファイバの入射偏光（[`InputPol`]）と偏光追跡（[`trace_path`]・[`trace_jones`]）。
//!
//! 単色の干渉縞（[`crate::fringe`]）と、拡張のプラグイン（干渉の計算など）が使う。
//!
//! - 強度は幾何光学: 瞳の格子のセル 1 つを三角形 2 つに分け、三角形ごとに
//!   ファイバのモードの重み（ファイバ端の方向余弦のガウス）× 方向余弦の面積 / 検出面の面積。
//!   画素は、その中心を含む最初の三角形の値を取る（ねじれた三角形 = 折り返しは除く）。
//! - 場は中心波長の偏光追跡の Jones ベクトル（検出面の局所 x・y、約束は exp(+i k L)）、
//!   光路長は追跡した波長ごと。画素の値は三角形の重心座標で補間する。

use crate::coating::{Coatings, C64};
use crate::math::V3;
use crate::surface::SurfType;
use crate::system::{FieldSel, RecSel, System};
use crate::{invalid, Result};
use rayon::prelude::*;

fn czero() -> C64 {
    C64::new(0.0, 0.0)
}

// ---------------------------------------------------------------------------------------------------------------
// 光線の格子と画素への補間

/// 1 本の経路の光線の格子（瞳の G × G、番号 = iy·G + ix、瞳座標 −1 + 2·i/(G−1)）
#[derive(Debug, Clone, PartialEq)]
pub struct RayGrid {
    pub g: usize,
    /// 検出面に届いた光線（瞳の円の外・ケラレ・開口の外は false）
    pub ok: Vec<bool>,
    /// 検出面の局所座標の位置 [mm]
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    /// ファイバ端での方向余弦（モードの重みに使う）
    pub sx: Vec<f64>,
    pub sy: Vec<f64>,
    /// [波長][光線] の光路長 [mm]（波長 0 = 中心）。どの波長も中心波長の検出面の着地点 (xs, ys) での値（[`trace_jones`] が
    /// 波長ごとの着地点のずれを 1 次で直す）
    pub l: Vec<Vec<f64>>,
    /// 検出面の局所 x・y の場（中心波長、約束は exp(+i k L)）
    pub ex: Vec<C64>,
    pub ey: Vec<C64>,
}

impl RayGrid {
    pub fn new(g: usize, nw: usize) -> RayGrid {
        let n = g * g;
        RayGrid {
            g,
            ok: vec![false; n],
            xs: vec![0.0; n],
            ys: vec![0.0; n],
            sx: vec![0.0; n],
            sy: vec![0.0; n],
            l: vec![vec![0.0; n]; nw],
            ex: vec![czero(); n],
            ey: vec![czero(); n],
        }
    }

    /// 配列の大きさが G² か確かめる
    pub fn check(&self) -> Result<()> {
        let n = self.g * self.g;
        if self.g < 2
            || self.ok.len() != n
            || [&self.xs, &self.ys, &self.sx, &self.sy]
                .iter()
                .any(|v| v.len() != n)
            || self.l.is_empty()
            || self.l.iter().any(|v| v.len() != n)
            || self.ex.len() != n
            || self.ey.len() != n
        {
            invalid!("光線の格子の大きさが合わない（G = {}）", self.g);
        }
        Ok(())
    }
}

/// 画素の格子（N × N、画素 (i, j) の中心は (x0 + i·dx, y0 + j·dx)、番号 j·N + i）に補間した 1 本の経路
#[derive(Debug, Clone)]
pub struct PixelData {
    pub n: usize,
    pub x0: f64,
    pub y0: f64,
    pub dx: f64,
    pub ok: Vec<bool>,
    /// 強度（モードの重み × 方向余弦の面積 / 検出面の面積）
    pub i: Vec<f64>,
    pub l: Vec<Vec<f64>>,
    pub ex: Vec<C64>,
    pub ey: Vec<C64>,
    /// 折り返した（向きが逆の）三角形の割合
    pub fold_fraction: f64,
}

/// ファイバのモードの重み（方向余弦のガウス、NA で 1/e²）。NA が無限大なら一様（重み 1、単色の干渉縞が使う）
pub fn mode_weight(sx: f64, sy: f64, na: f64) -> f64 {
    if na == f64::INFINITY {
        return 1.0;
    }
    (-2.0 * (sx * sx + sy * sy) / (na * na)).exp() / (std::f64::consts::PI * na * na / 2.0)
}

fn tri_area(x: &[f64], y: &[f64], t: [usize; 3]) -> f64 {
    0.5 * ((x[t[1]] - x[t[0]]) * (y[t[2]] - y[t[0]]) - (x[t[2]] - x[t[0]]) * (y[t[1]] - y[t[0]]))
}

/// 光線の格子を画素の格子へ（瞳のセルごとに三角形 2 つ）。画素はその中心を含む最初の三角形の値を取る
pub fn rasterize(r: &RayGrid, npx: usize, x0: f64, y0: f64, dx: f64, na: f64) -> PixelData {
    let nw = r.l.len();
    let g = r.g;
    let mut tris = Vec::new();
    for iy in 0..g - 1 {
        for ix in 0..g - 1 {
            let a = iy * g + ix;
            let (b, c) = (a + 1, a + g);
            let d = c + 1;
            if r.ok[a] && r.ok[b] && r.ok[d] {
                tris.push([a, b, d]);
            }
            if r.ok[a] && r.ok[d] && r.ok[c] {
                tris.push([a, d, c]);
            }
        }
    }
    // 多い方の向き。裏返った三角形（折り返し・焦線）は除く
    let pos = tris
        .iter()
        .filter(|t| tri_area(&r.xs, &r.ys, **t) > 0.0)
        .count();
    let sgn = if pos * 2 >= tris.len() { 1.0 } else { -1.0 };
    let mut fold = 0usize;
    // 使う三角形: 頂点、強度、行の範囲
    let mut good: Vec<([usize; 3], f64, usize, usize)> = Vec::with_capacity(tris.len());
    let last = npx as i64 - 1;
    for t in &tris {
        let a_s = tri_area(&r.xs, &r.ys, *t);
        if a_s * sgn <= 0.0 {
            fold += 1;
            continue;
        }
        let a_d = tri_area(&r.sx, &r.sy, *t).abs();
        let w = (mode_weight(r.sx[t[0]], r.sy[t[0]], na)
            + mode_weight(r.sx[t[1]], r.sy[t[1]], na)
            + mode_weight(r.sx[t[2]], r.sy[t[2]], na))
            / 3.0;
        let (ya, yb, yc) = (r.ys[t[0]], r.ys[t[1]], r.ys[t[2]]);
        let j0 = (((ya.min(yb.min(yc)) - y0) / dx).floor() as i64).max(0);
        let j1 = (((ya.max(yb.max(yc)) - y0) / dx).ceil() as i64).min(last);
        if j0 > j1 {
            good.push((*t, w * a_d / a_s.abs(), 1, 0));
            continue;
        }
        good.push((*t, w * a_d / a_s.abs(), j0 as usize, j1 as usize));
    }
    let n2 = npx * npx;
    let mut ok = vec![false; n2];
    let mut ii = vec![0.0; n2];
    let mut ll = vec![0.0; n2 * nw];
    let mut ex = vec![czero(); n2];
    let mut ey = vec![czero(); n2];
    // 行ごとに並列。行の中では三角形を同じ順に見るので、最初に覆った三角形が勝つ（逐次と同じ結果）
    type Row<'a> = (
        usize,
        (
            ((&'a mut [bool], &'a mut [f64]), &'a mut [f64]),
            (&'a mut [C64], &'a mut [C64]),
        ),
    );
    let row = |(j, (((okr, ir), lr), (exr, eyr))): Row| {
        let y = y0 + j as f64 * dx;
        for &(t, irr, j0, j1) in &good {
            if j < j0 || j > j1 {
                continue;
            }
            let (xa, ya, xb, yb, xc, yc) = (
                r.xs[t[0]], r.ys[t[0]], r.xs[t[1]], r.ys[t[1]], r.xs[t[2]], r.ys[t[2]],
            );
            let i0 = (((xa.min(xb.min(xc)) - x0) / dx).floor() as i64).max(0);
            let i1 = (((xa.max(xb.max(xc)) - x0) / dx).ceil() as i64).min(last);
            if i0 > i1 {
                continue;
            }
            let den = (yb - yc) * (xa - xc) + (xc - xb) * (ya - yc);
            for i in i0 as usize..=i1 as usize {
                if okr[i] {
                    continue;
                }
                let x = x0 + i as f64 * dx;
                let l1 = ((yb - yc) * (x - xc) + (xc - xb) * (y - yc)) / den;
                let l2 = ((yc - ya) * (x - xc) + (xa - xc) * (y - yc)) / den;
                let l3 = 1.0 - l1 - l2;
                if l1 < -1e-12 || l2 < -1e-12 || l3 < -1e-12 {
                    continue;
                }
                okr[i] = true;
                ir[i] = irr;
                for w in 0..nw {
                    lr[w * npx + i] = l1 * r.l[w][t[0]] + l2 * r.l[w][t[1]] + l3 * r.l[w][t[2]];
                }
                exr[i] = r.ex[t[0]] * l1 + r.ex[t[1]] * l2 + r.ex[t[2]] * l3;
                eyr[i] = r.ey[t[0]] * l1 + r.ey[t[1]] * l2 + r.ey[t[2]] * l3;
            }
        }
    };
    // 逐次の指定では rayon を使わない（fork した子プロセスでも動く）
    if crate::par::sequential() {
        ok.chunks_mut(npx)
            .zip(ii.chunks_mut(npx))
            .zip(ll.chunks_mut(npx * nw))
            .zip(ex.chunks_mut(npx).zip(ey.chunks_mut(npx)))
            .enumerate()
            .for_each(row);
    } else {
        ok.par_chunks_mut(npx)
            .zip(ii.par_chunks_mut(npx))
            .zip(ll.par_chunks_mut(npx * nw))
            .zip(ex.par_chunks_mut(npx).zip(ey.par_chunks_mut(npx)))
            .enumerate()
            .for_each(row);
    }
    // 行ごとの [波長][画素] を [波長][全画素] に並べ直す
    let mut l = vec![vec![0.0; n2]; nw];
    for j in 0..npx {
        for w in 0..nw {
            l[w][j * npx..(j + 1) * npx]
                .copy_from_slice(&ll[j * npx * nw + w * npx..j * npx * nw + (w + 1) * npx]);
        }
    }
    PixelData {
        n: npx,
        x0,
        y0,
        dx,
        ok,
        i: ii,
        l,
        ex,
        ey,
        fold_fraction: if tris.is_empty() {
            0.0
        } else {
            fold as f64 / tris.len() as f64
        },
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 追跡

/// ファイバの入射偏光（物体の座標系の Jones ベクトル、約束は exp(+i k L)）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputPol {
    X,
    Y,
    /// 直線（x から測った角度 [deg]）
    Angle(f64),
    /// 円 +（x → y の向きに回る、S3 > 0）
    CircularPlus,
    CircularMinus,
}

impl InputPol {
    pub fn jones(self) -> [C64; 2] {
        let r = 0.5f64.sqrt();
        match self {
            InputPol::X => [C64::new(1.0, 0.0), czero()],
            InputPol::Y => [czero(), C64::new(1.0, 0.0)],
            InputPol::Angle(d) => [
                C64::new(d.to_radians().cos(), 0.0),
                C64::new(d.to_radians().sin(), 0.0),
            ],
            InputPol::CircularPlus => [C64::new(r, 0.0), C64::new(0.0, r)],
            InputPol::CircularMinus => [C64::new(r, 0.0), C64::new(0.0, -r)],
        }
    }

    pub fn parse(s: &str) -> Result<InputPol> {
        let t = s.trim().to_ascii_lowercase();
        Ok(match t.as_str() {
            "x" | "s" => InputPol::X,
            "y" | "p" => InputPol::Y,
            "+" | "circ+" | "circular+" | "rcp" => InputPol::CircularPlus,
            "-" | "circ-" | "circular-" | "lcp" => InputPol::CircularMinus,
            _ => match t.parse::<f64>() {
                Ok(d) => InputPol::Angle(d),
                Err(_) => invalid!("入射偏光 {s}（x・y・角度 [deg]・circ+・circ-）"),
            },
        })
    }
}

/// 位相の項（理想レンズ）がある系か
fn has_phase_terms(sys: &System) -> bool {
    sys.surfaces
        .iter()
        .any(|s| !s.ignore && matches!(s.typ, SurfType::Paraxial | SurfType::User))
}

/// 1 本の経路（系 `sys` = その構成、波長は追跡する波長 [`traced_wavelengths`] に置き換え、主波長 0）の光線の格子を
/// 瞳 G × G で追う。視野 `field`、入射偏光 `input`、膜 `coat`。
///
/// - 光路長: 生の光路長（物体から最後の面まで）。理想レンズがあれば、瞳の中は位相を含めた光路（主光線の生の
///   光路長 + (光線 − 主光線) の位相込みの光路）
/// - ファイバ端の方向余弦: 有限物体は物体点からの方向。無限遠物体は視野の光線が 1 点に集まる最初の面での方向
///   （その面がなければ一様な瞳）
/// - 場: 中心波長の偏光追跡を検出面の局所 x・y で、exp(+i k L) の約束に直したもの
pub fn trace_path(
    sys: &System,
    field: FieldSel,
    g: usize,
    input: InputPol,
    coat: &Coatings,
) -> Result<RayGrid> {
    let t = trace_jones(sys, field, g, coat, None)?;
    let jin = input.jones();
    let mut r = t.rays;
    for q in 0..g * g {
        if r.ok[q] {
            let j = t.jones[q];
            r.ex[q] = j[0][0] * jin[0] + j[0][1] * jin[1];
            r.ey[q] = j[1][0] * jin[0] + j[1][1] * jin[1];
        }
    }
    Ok(r)
}

/// [`trace_jones`] の結果: 光線の格子（場 `ex`・`ey` は 0 のまま）、光線ごとの Jones 行列（行 = 検出面の局所 x・y、
/// 列 = ファイバ端の入射 x・y、中心波長、exp(+i k L) の約束）と、面 `surf` を指定したときのその面の局所座標の交点
#[derive(Debug, Clone)]
pub struct JonesGrid {
    pub rays: RayGrid,
    pub jones: Vec<[[C64; 2]; 2]>,
    /// 指定面の交点 [mm]（面を指定しないときは空。有効でない光線は NaN）
    pub xf: Vec<f64>,
    pub yf: Vec<f64>,
    /// [波長][光線] の、光路長を中心波長の着地点へ移した量 [mm]（`rays.l` に足してある。波長 0 は 0）。
    /// 引けば各波長の光線の着地点での値になる（追跡の照合用）
    pub landing: Vec<Vec<f64>>,
}

/// [`trace_path`] と同じ追跡で、入射の x・y の 2 つを追って Jones 行列にする。`surf` を指定すると、その面の
/// 局所座標の交点も記録する（全視野干渉の解析格子を指定面の座標で作るため）。
pub fn trace_jones(
    sys: &System,
    field: FieldSel,
    g: usize,
    coat: &Coatings,
    surf: Option<usize>,
) -> Result<JonesGrid> {
    if let Some(s) = surf {
        if s == 0 || s > sys.image() {
            invalid!("面 {s} が範囲外（1〜{}）", sys.image());
        }
    }
    if g < 2 {
        invalid!("瞳の格子は 2 以上");
    }
    if sys.primary != 0 {
        invalid!("主波長は中心波長（波長 1）に");
    }
    let nw = sys.wavelengths.len();
    let mut r = RayGrid::new(g, nw);
    let mut idx = Vec::new();
    for iy in 0..g {
        for ix in 0..g {
            let (px, py) = (
                -1.0 + 2.0 * ix as f64 / (g - 1) as f64,
                -1.0 + 2.0 * iy as f64 / (g - 1) as f64,
            );
            if px * px + py * py <= 1.0 {
                idx.push(iy * g + ix);
            }
        }
    }
    let pxs: Vec<f64> = idx
        .iter()
        .map(|&q| -1.0 + 2.0 * (q % g) as f64 / (g - 1) as f64)
        .collect();
    let pys: Vec<f64> = idx
        .iter()
        .map(|&q| -1.0 + 2.0 * (q / g) as f64 / (g - 1) as f64)
        .collect();
    let mut good = vec![false; g * g];
    for &q in &idx {
        good[q] = true;
    }
    let phase = has_phase_terms(sys);
    let mut raw0 = vec![0.0; g * g];
    // 中心波長の検出面の着地点の z（曲がった検出面でも 3 次元で直すため）
    let mut zs0 = vec![0.0; g * g];
    let mut landing = vec![vec![0.0; g * g]; nw];
    let mut t0 = None;
    for w in 0..nw {
        // 指定面の交点は中心波長の追跡で一緒に記録する（同じ光線をもう一度追わない）
        let t = match (w, surf) {
            (0, Some(s)) => {
                sys.try_trace_rec(field, &pxs, &pys, Some(w), None, RecSel::Surfs(&[s]))?
            }
            _ => sys.try_trace(field, &pxs, &pys, Some(w), None, false)?,
        };
        let chief = if phase {
            let c = sys.try_trace(field, &[0.0], &[0.0], Some(w), None, false)?;
            if !c.traced(0) {
                invalid!("主光線がセンサに届かない");
            }
            Some((c.opl_os[0], c.opl_os[0] + c.rays[0].lens_opd))
        } else {
            None
        };
        // 像空間の屈折率（向きの符号は使わない）
        let nimg = sys.image_index(w).abs();
        for (m, &q) in idx.iter().enumerate() {
            if !t.ok(m) {
                good[q] = false;
                continue;
            }
            let lraw = t.opl_os[m];
            let mut l = match chief {
                Some((craw, cpath)) => craw + (lraw + t.rays[m].lens_opd - cpath),
                None => lraw,
            };
            if w == 0 {
                raw0[q] = lraw;
                r.xs[q] = t.rays[m].xyz[0];
                r.ys[q] = t.rays[m].xyz[1];
                zs0[q] = t.rays[m].xyz[2];
            } else if good[q] {
                // 波長 w の光線は中心波長の着地点 x0 とは別の点 x_w に着く（倍率の色収差）。格子の値は中心波長の
                // 着地点の値なので、波面を 1 次で x0 へ移す: L_w(x0) = L_w(x_w) + n s_w·(x0 − x_w)（アイコナール）
                let (p, d) = (t.rays[m].xyz, t.rays[m].lmn);
                let dl = nimg
                    * (d[0] * (r.xs[q] - p[0]) + d[1] * (r.ys[q] - p[1]) + d[2] * (zs0[q] - p[2]));
                landing[w][q] = dl;
                l += dl;
            }
            r.l[w][q] = l;
        }
        if w == 0 {
            t0 = Some(t);
        }
    }
    let t0 = t0.unwrap();
    // ファイバ端の方向余弦
    if !sys.infinite() {
        let of = sys.object_frame();
        for (m, &q) in idx.iter().enumerate() {
            // 物体の座標系へ（回転の列が軸）
            let d = crate::math::mtv(&of.r, t0.k0[m]);
            r.sx[q] = d[0];
            r.sy[q] = d[1];
        }
    } else {
        let probe = [(0.0, 0.0), (0.7, 0.0), (-0.7, 0.0), (0.0, 0.7), (0.0, -0.7)];
        let ppx: Vec<f64> = probe.iter().map(|p| p.0).collect();
        let ppy: Vec<f64> = probe.iter().map(|p| p.1).collect();
        let ima = sys.image();
        let tp = sys.try_trace_rec(field, &ppx, &ppy, Some(0), None, RecSel::All)?;
        let mut fs = None;
        for k in 1..ima {
            let Some(rec) = tp.rec_at(k) else { continue };
            let pts: Vec<V3> = rec.xyz.clone();
            if pts.iter().any(|p| !p[0].is_finite()) {
                continue;
            }
            if pts
                .iter()
                .all(|p| (p[0] - pts[0][0]).abs() < 1e-6 && (p[1] - pts[0][1]).abs() < 1e-6)
            {
                fs = Some(k);
                break;
            }
        }
        match fs {
            None => {
                for (m, &q) in idx.iter().enumerate() {
                    r.sx[q] = 1e-4 * pxs[m];
                    r.sy[q] = 1e-4 * pys[m];
                }
            }
            Some(fsurf) => {
                let ta =
                    sys.try_trace_rec(field, &pxs, &pys, Some(0), None, RecSel::Surfs(&[fsurf]))?;
                let rec = ta.rec_at(fsurf).expect("record");
                for (m, &q) in idx.iter().enumerate() {
                    let d = rec.lmn[m];
                    if !d[0].is_finite() {
                        good[q] = false;
                        continue;
                    }
                    r.sx[q] = d[0];
                    r.sy[q] = d[1];
                }
            }
        }
    }
    // 偏光: 検出面の局所 x・y の場。伝搬の位相を除いた振幅（OpticStudio の E = J exp(−i k L_raw)）の共役が
    // exp(+i k L) の約束の場
    let pols = [[C64::new(1.0, 0.0), czero()], [czero(), C64::new(1.0, 0.0)]];
    let pt = sys.try_trace_pol_opts(field, &pxs, &pys, Some(0), &pols, coat, false)?;
    let mut jones = vec![[[czero(); 2]; 2]; g * g];
    for (m, &q) in idx.iter().enumerate() {
        if !pt.trace.ok(m) || !good[q] {
            good[q] = false;
            continue;
        }
        for (b, pol) in pt.e[m].iter().enumerate().take(2) {
            jones[q][0][b] = pol[0].conj();
            jones[q][1][b] = pol[1].conj();
        }
    }
    let (mut xf, mut yf) = (Vec::new(), Vec::new());
    if let Some(s) = surf {
        xf = vec![f64::NAN; g * g];
        yf = vec![f64::NAN; g * g];
        let tr = &t0;
        if let Some(rec) = tr.rec_at(s) {
            for (m, &q) in idx.iter().enumerate() {
                if good[q] && tr.ok(m) && rec.xyz[m][0].is_finite() {
                    xf[q] = rec.xyz[m][0];
                    yf[q] = rec.xyz[m][1];
                }
            }
        }
    }
    let _ = raw0;
    r.ok = good;
    Ok(JonesGrid {
        rays: r,
        jones,
        xf,
        yf,
        landing,
    })
}
