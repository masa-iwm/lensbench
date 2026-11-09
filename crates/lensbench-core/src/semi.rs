//! 自動の半直径（OpticStudio の Automatic）: 全視野・全波長の主光線と瞳の縁の光線が面を通る高さの最大
//! から半直径を求める。
//!
//! 光線の取り方は OpticStudio が保存した .zmx の値で確かめた:
//! - 回転対称な系（[`System::symmetric`]。ただし Zernike・Grid Sag の面は対称とみる）: 視野ごとに、視野の値 (x, y)
//!   の向きの瞳の縁 ±(x, y)/|(x, y)| の 2 本（軸上は py = ±1）。矩形の視野の正規化でも正規化視野 (hx, hy) の向きではない
//!   （A6v で確かめた）
//! - そうでない系: 瞳の縁を 45° おきに 8 本（py = 1 から px の正の向きへ）
//!
//! 主光線は使わない（主光線が縁の光線より外を通る面でも、OpticStudio の値は縁の光線の高さ）。物体面は
//! 有限なら物点の高さ。ケラレ係数をかけた瞳で追い、開口ではけらない（けられても追跡を続ける）。
//! OpticStudio の既定（Fast Semi-Diameters がオン）の値で、2026 R1 の 17 系の全部の面で 1e-9 で一致を
//! 確かめた（ケラレ係数・VAN・実光線エイミング・面の傾き偏心・テレセントリック・有限物体を含む）。
//! 合わないもの: Grid Sag の系の像面（相対 2e-6、OpticStudio の Grid Sag の追跡の差）と、矩形の視野の正規化で
//! ケラレ係数・VAN のある視野の系（A6、最大 8%）

use crate::surface::Aperture;
use crate::system::{FieldSel, RecSel};
use crate::System;

impl System {
    /// 面ごとの、全視野・全波長の主光線と瞳の縁の光線が通る高さ（面の局所座標の √(x²+y²)）の最大。
    /// 物体面は有限なら物点の高さ、無限遠なら 0。どの光線も届かない面は None
    pub fn ray_heights(&self) -> Vec<Option<f64>> {
        let ns = self.surfaces.len();
        let mut h: Vec<Option<f64>> = vec![None; ns];
        let mut up = |j: usize, v: f64| {
            if v.is_finite() {
                let e = h[j].get_or_insert(0.0);
                *e = e.max(v);
            }
        };
        let sym = self.symmetric_opts(false);
        let (mut px, mut py) = (vec![0.0], vec![0.0]);
        if !sym {
            for k in 0..8 {
                let (s, c) = (std::f64::consts::FRAC_PI_4 * k as f64).sin_cos();
                px.push(s);
                py.push(c);
            }
        }
        for i in 0..self.fields.len() {
            if sym {
                // 向きは視野の値 (x, y) の向き（矩形の正規化でも正規化視野 (hx, hy) ではない）
                let (hx, hy) = (self.fields[i].x, self.fields[i].y);
                let r = hx.hypot(hy);
                let (ux, uy) = if r > 0.0 {
                    (hx / r, hy / r)
                } else {
                    (0.0, 1.0)
                };
                px = vec![0.0, ux, -ux];
                py = vec![0.0, uy, -uy];
            }
            for w in 0..self.wavelengths.len() {
                let Ok(t) =
                    self.try_trace_rec(FieldSel::Index(i), &px, &py, Some(w), None, RecSel::All)
                else {
                    continue;
                };
                if !self.infinite() {
                    for p in &t.p0 {
                        up(0, p[0].hypot(p[1]));
                    }
                }
                for r in &t.rec {
                    // 主光線（先頭）は使わない（OpticStudio と同じ）
                    for q in r.xyz.iter().skip(1) {
                        up(r.surf, q[0].hypot(q[1]));
                    }
                }
            }
        }
        if self.infinite() {
            h[0] = Some(0.0);
        }
        h
    }

    /// 自動の半直径が追跡に効くか: 自動の面に浮動開口がある（ける）、Irregular の面（半直径で正規化）、
    /// 半直径のピックアップ、半径 0 のコバ厚のソルブ（半直径で測る）。最適化・公差の内側
    /// （[`System::with_params_fast`]）はこのときだけ求め直す
    pub(crate) fn semi_affects_trace(&self) -> bool {
        use crate::surface::SurfType;
        let auto = |k: usize| self.surfaces.get(k).is_some_and(|d| d.semi_auto);
        self.surfaces.iter().any(|d| {
            d.semi_auto
                && (matches!(d.ap, Aperture::Floating { .. }) || d.typ == SurfType::Irregular)
        }) || self
            .pickups
            .iter()
            .any(|p| p.what == crate::param::PickKind::Semi && auto(p.src))
            || self.solves.iter().any(|q| {
                matches!(q.kind, crate::param::SolveKind::Edge(r) if r == 0.0)
                    && (auto(q.surf) || auto(q.surf + 1))
            })
    }

    /// 自動の半直径の面（[`crate::surface::Surface::semi_auto`]）の半直径を光線から求め直す（Floating 開口の半径も）。
    /// 光線が届かない面は値を変えない。半直径のピックアップは解決し直す
    pub fn update_semi(&mut self) {
        if !self.surfaces.iter().any(|d| d.semi_auto) {
            return;
        }
        // 複製で追う（座標系などのキャッシュをこの系に残さない。読込のあとに面を直接書き換えて
        // changed() を呼ばない使い方でも古いキャッシュを使わないように）
        let h = self.clone().ray_heights();
        // 絞りの大きさで決める系（Float By Stop Size）の絞りの半直径は開口そのものなので求めない
        let fixed_stop =
            (self.aperture.0 == crate::system::ApertureKind::Float).then(|| self.stop());
        for (j, (d, v)) in self.surfaces.iter_mut().zip(h).enumerate() {
            let Some(v) = v else { continue };
            if !d.semi_auto || Some(j) == fixed_stop {
                continue;
            }
            d.semi = v;
            if let Aperture::Floating { semi } = &mut d.ap {
                *semi = v;
            }
        }
        if self
            .pickups
            .iter()
            .any(|p| p.what == crate::param::PickKind::Semi)
        {
            let _ = self.resolve_pickups();
        }
    }
}
