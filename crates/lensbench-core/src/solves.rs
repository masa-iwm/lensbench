//! ソルブ（厚さ・曲率を条件で決める）: 種類と解き方。

use crate::error::Result;
use crate::linalg::Lu;
use crate::param::{Kind, Param};
use crate::surface::{After, SurfType};
use crate::system::{Keep, System};
use crate::{invalid, notimpl};

/// ソルブの種類と、その条件の値（.zmx の書き方は OpticStudio 2026 R1.01 で実測）。other は参照する別の面
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[derive(schemars::JsonSchema)]
pub enum SolveKind {
    /// 厚さ: 次の面での主波長の近軸マージナル光線の高さ = height（MAZH 高さ 0）
    MarginalHeight { height: f64 },
    /// 厚さ: 次の面での実光線（軸上、瞳 (0, zone)、主波長）の高さ = height（MAZH 高さ 帯、帯 ≠ 0）
    MarginalHeightZone { height: f64, zone: f64 },
    /// 厚さ: 次の面での近軸主光線の高さ = height（CHZH 高さ 0）
    ChiefHeight { height: f64 },
    /// 厚さ: 軸上の視野・主波長の実光線（瞳 (0, zone)）の像面での OPD [波] = opd（OPDZ OPD 帯）。OPD は
    /// [`crate::analysis::opd`]（OpticStudio の光線の OPD、OPDC と同じ）
    Opd { opd: f64, zone: f64 },
    /// 厚さ: 次の面を近軸の瞳（その空間で近軸主光線が軸を切る位置）に置く（TPUP）
    PupilPosition,
    /// 厚さ: 半径方向の高さ radius（0 は面の半直径）でのコバ厚 = thickness（EDGE コバ厚 r）
    Edge { thickness: f64, radius: f64 },
    /// 厚さ: 面 other から次の面までの距離（厚さの和 Σ t[other..=この面]）= length（TOLE s 長さ）。other が後ろなら、
    /// この面から面 other までの距離 Σ t[この面..other] = length
    Position { other: usize, length: f64 },
    /// 厚さ: 面 other の厚さ + この面の厚さ = sum（TCOM s 和）
    Compensator { other: usize, sum: f64 },
    /// 厚さ: 次の面を面 other の曲率中心に置く（TCOC s 0。平面は R = 1e10）
    CenterOfCurvature { other: usize },
    /// 曲率: 面の後の近軸マージナル光線の傾き = angle（CURV c 2 値）
    MarginalAngle { angle: f64 },
    /// 曲率: 面の後の近軸主光線の傾き = angle（CURV c 3 値）
    ChiefAngle { angle: f64 },
    /// 曲率: 近軸マージナル光線が面に垂直に入る（CURV c 5）
    MarginalNormal,
    /// 曲率: 近軸主光線が面に垂直に入る（CURV c 6）
    ChiefNormal,
    /// 曲率: 近軸マージナル光線で不遊（u′/n′ = u/n、CURV c 7）
    Aplanatic,
    /// 曲率: この面を含む玉（後ろがガラスなら次の面と、そうでなければ前の面と）のパワー = power（CURV c 8 値）
    ElementPower { power: f64 },
    /// 曲率: 曲率中心を面 other の頂点に置く（CURV c 9 s）
    ConcentricSurface { other: usize },
    /// 曲率: 曲率中心を面 other の曲率中心に置く（CURV c 10 s。平面は R = 1e10）
    ConcentricRadius { other: usize },
    /// 曲率: 面の後の近軸マージナル光線の傾き = −1/(2 fnum)（CURV c 11 F）
    FNumber { fnum: f64 },
    /// 曲率: ZPL のマクロ（[`System::macros`] の番号 index）の SOLVERETURN の値（CURV c 12 … "名前"）
    MacroCurv { index: u16 },
    /// 厚さ: ZPL のマクロ（[`System::macros`] の番号 index）の SOLVERETURN の値
    MacroThick { index: u16 },
}

impl Default for SolveKind {
    fn default() -> SolveKind {
        SolveKind::MarginalHeight { height: 0.0 }
    }
}

impl SolveKind {
    /// 曲率のソルブ（ほかは厚さ）
    pub fn is_curv(&self) -> bool {
        matches!(
            self,
            SolveKind::MarginalAngle { .. }
                | SolveKind::ChiefAngle { .. }
                | SolveKind::MarginalNormal
                | SolveKind::ChiefNormal
                | SolveKind::Aplanatic
                | SolveKind::ElementPower { .. }
                | SolveKind::ConcentricSurface { .. }
                | SolveKind::ConcentricRadius { .. }
                | SolveKind::FNumber { .. }
                | SolveKind::MacroCurv { .. }
        )
    }

    /// 決めるパラメータの種類
    pub fn param_kind(&self) -> Kind {
        if self.is_curv() {
            Kind::Curv
        } else {
            Kind::Thick
        }
    }

    /// 参照する別の面（あれば）
    pub fn other_surface(&self) -> Option<usize> {
        match *self {
            SolveKind::Position { other, .. }
            | SolveKind::Compensator { other, .. }
            | SolveKind::CenterOfCurvature { other }
            | SolveKind::ConcentricSurface { other }
            | SolveKind::ConcentricRadius { other } => Some(other),
            _ => None,
        }
    }

    /// 参照する別の面を付け替えたもの
    pub fn with_other_surface(mut self, s: usize) -> SolveKind {
        match &mut self {
            SolveKind::Position { other, .. }
            | SolveKind::Compensator { other, .. }
            | SolveKind::CenterOfCurvature { other }
            | SolveKind::ConcentricSurface { other }
            | SolveKind::ConcentricRadius { other } => *other = s,
            _ => {}
        }
        self
    }

    /// 条件の値（高さ・OPD・コバ厚・長さ・和・角度・パワー・F/#。値のない種類は 0。.zmx のソルブの 1 つ目の値）
    pub fn value(&self) -> f64 {
        match *self {
            SolveKind::MarginalHeight { height }
            | SolveKind::MarginalHeightZone { height, .. }
            | SolveKind::ChiefHeight { height } => height,
            SolveKind::Opd { opd, .. } => opd,
            SolveKind::Edge { thickness, .. } => thickness,
            SolveKind::Position { length, .. } => length,
            SolveKind::Compensator { sum, .. } => sum,
            SolveKind::MarginalAngle { angle } | SolveKind::ChiefAngle { angle } => angle,
            SolveKind::ElementPower { power } => power,
            SolveKind::FNumber { fnum } => fnum,
            _ => 0.0,
        }
    }

    /// 条件の値を v にしたもの（値のない種類はそのまま）
    pub fn with_value(mut self, v: f64) -> SolveKind {
        match &mut self {
            SolveKind::MarginalHeight { height }
            | SolveKind::MarginalHeightZone { height, .. }
            | SolveKind::ChiefHeight { height } => *height = v,
            SolveKind::Opd { opd, .. } => *opd = v,
            SolveKind::Edge { thickness, .. } => *thickness = v,
            SolveKind::Position { length, .. } => *length = v,
            SolveKind::Compensator { sum, .. } => *sum = v,
            SolveKind::MarginalAngle { angle } | SolveKind::ChiefAngle { angle } => *angle = v,
            SolveKind::ElementPower { power } => *power = v,
            SolveKind::FNumber { fnum } => *fnum = v,
            _ => {}
        }
        self
    }
}

/// ソルブ: 面 surf の厚さか曲率（[`SolveKind::is_curv`]）を、種類 kind の条件で決める
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Solve {
    pub surf: usize,
    #[serde(flatten)]
    pub kind: SolveKind,
}

impl System {
    /// 読込で、解けないソルブ（周辺光線が光軸に平行な厚さのソルブなど）を外す。ファイル全体を断らず、マクロでない
    /// ソルブをソルブのないところから 1 つずつ足して解けなくするものだけ外し、その面は保存された値で固定して理由を
    /// [`System::import_notes`] に残す（マクロのソルブの `zpl_solve::resolve_on_load` と同じ方針）。マクロのソルブは
    /// ここでは外して試し（マクロの誤りで普通のソルブを外さない）、この後に `resolve_on_load` が扱う。
    /// ソルブを全部外しても解けない（ソルブの外の誤り）ときと、全部解けるときは何も変えない（誤りは後の
    /// [`System::solve_thickness`] が返す）。読んだあとの編集で解けなくなったときは、今までどおり誤り
    pub(crate) fn drop_unsolvable_on_load(&mut self) {
        let is_macro = |k: &SolveKind| {
            matches!(
                k,
                SolveKind::MacroCurv { .. } | SolveKind::MacroThick { .. }
            )
        };
        let n = self.solves.len();
        let with = |keep: &[bool]| {
            let mut c = self.clone();
            let mut i = 0;
            let r = c.rx_mut();
            r.solves.retain(|_| {
                i += 1;
                keep[i - 1]
            });
            r.macro_parms.clear();
            c
        };
        let plain: Vec<bool> = self.solves.iter().map(|q| !is_macro(&q.kind)).collect();
        if !plain.contains(&true) || with(&plain).solve_thickness().is_ok() {
            return;
        }
        let mut keep = vec![false; n];
        if with(&keep).solve_thickness().is_err() {
            return;
        }
        let mut notes = Vec::new();
        for j in 0..n {
            if !plain[j] {
                continue;
            }
            keep[j] = true;
            if let Err(e) = with(&keep).solve_thickness() {
                keep[j] = false;
                let q = &self.solves[j];
                let what = if q.kind.is_curv() {
                    "curvature"
                } else {
                    "thickness"
                };
                let (kind, k) = (solve_kind_info(q.kind).0, q.surf);
                notes.push(format!(
                    "removed the {what} solve {kind} of surface {k} and fixed the saved value: can't solve it ({e})"
                ));
            }
        }
        // 外したソルブの面は保存された値のまま（ここでは解いた値を入れない）。マクロのソルブは残す
        let mut i = 0;
        let r = self.rx_mut();
        r.solves.retain(|q| {
            i += 1;
            keep[i - 1] || is_macro(&q.kind)
        });
        r.import_notes.extend(notes);
    }

    /// ソルブ（[`Solve`]: 厚さ・曲率）を解き直す。読込と、パラメータを変えた後に呼ぶ。
    /// 近軸の量は主波長で、無視した面は飛ばす（近軸の追跡と同じ）。面の順に解き、どれかを変えると前のソルブの
    /// 条件も変わりうる（入射瞳・F/# の絞りなど）ので、全部が収まるまで繰り返す。ピックアップは途中で解決し直す。
    /// 既に解けている値（相対 1e-14）は変えない（読み戻し・関係ないパラメータの変更で同じ系になるように）
    pub(crate) fn solve_thickness(&mut self) -> Result<()> {
        let ns = self.surfaces.len();
        for q in &self.solves {
            let what = if q.kind.is_curv() {
                Kind::Curv
            } else {
                Kind::Thick
            };
            if q.surf == 0 || q.surf + 1 >= ns {
                let surf = q.surf;
                invalid!(
                    "surface {surf}: solve on the object or image surface (or a missing surface)"
                );
            }
            if let Some(o) = q.kind.other_surface() {
                if o >= ns {
                    invalid!("surface {}: no reference surface {o} for the solve", q.surf);
                }
            }
            if self
                .pickups
                .iter()
                .any(|p| p.dst == q.surf && p.what == what)
            {
                invalid!("surface {}: both a solve and a pickup ({:?})", q.surf, what);
            }
            if q.kind.is_curv() && self.surfaces[q.surf].is_coord_break() {
                invalid!("surface {}: curvature solve on a coordinate break", q.surf);
            }
            if matches!(q.kind, SolveKind::Position { other: 0, .. }) {
                let surf = q.surf;
                invalid!("surface {surf}: the position solve refers to the object surface");
            }
        }
        let solves: Vec<Solve> = self
            .solves
            .iter()
            .filter(|q| !self.surfaces[q.surf].ignore)
            .copied()
            .collect();
        if solves.is_empty() && self.macro_parms.is_empty() {
            return Ok(());
        }
        for _ in 0..100 {
            let mut moved = false;
            for q in &solves {
                moved |= self.solve_one(q)?;
            }
            if !self.macro_parms.is_empty() {
                moved |= self.solve_macro_parms()?;
            }
            if !moved {
                return Ok(());
            }
        }
        invalid!("the solve doesn't converge")
    }

    /// 解くソルブ（無視する面のものを除く、解く順）
    pub(crate) fn active_solves(&self) -> Vec<Solve> {
        self.solves
            .iter()
            .filter(|q| !self.surfaces[q.surf].ignore)
            .copied()
            .collect()
    }

    /// 傾き・偏心だけを変えた系（近軸量は同じ）で、ソルブがすべて解けたままか。実光線を使うもの・軸上の距離を
    /// 使うもの（傾き偏心をまたぐと解けない）は false。閉じた形のものはビットまで同じ値、光線の高さのものは
    /// 解きの許容差の中
    pub(crate) fn solves_still_hold(&self) -> bool {
        self.active_solves().iter().all(|q| {
            if matches!(
                q.kind,
                SolveKind::MarginalHeightZone { .. }
                    | SolveKind::Opd { .. }
                    | SolveKind::CenterOfCurvature { .. }
                    | SolveKind::ConcentricSurface { .. }
                    | SolveKind::ConcentricRadius { .. }
                    | SolveKind::MacroCurv { .. }
                    | SolveKind::MacroThick { .. }
            ) {
                return false;
            }
            let v = self.solved_value(q);
            match q.kind {
                SolveKind::MarginalHeight { .. }
                | SolveKind::ChiefHeight { .. }
                | SolveKind::PupilPosition => self
                    .paraxial_height_residual(q)
                    .is_ok_and(|(r, du)| (r / du).abs() <= 1e-13 * (1.0 + v.abs())),
                _ => self.closed_value(q).is_ok_and(|c| c == v),
            }
        })
    }

    /// ソルブで決まる量（[`System::active_solves`] の順）の、パラメータ ps それぞれへの微分 ds/dp
    /// （[パラメータ][ソルブ]）。陰関数の式 ds/dp = −(∂R/∂s)⁻¹ ∂R/∂p で求める（R はソルブの条件の残差
    /// [`System::solve_residual`]）。偏微分は、ソルブを解き直さない系の R の中心差分（R は滑らかなので、
    /// 解き直した系の差分と違ってソルブの許容差の分の誤差が乗らない。解き直しの反復もない）。
    /// ps はソルブで決まる量・ピックアップの先・玉や群・物点でない面のパラメータ
    pub(crate) fn solve_sensitivity(&self, ps: &[Param]) -> Result<Vec<Vec<f64>>> {
        let qs = self.active_solves();
        let ns = qs.len();
        if ns == 0 {
            return Ok(vec![Vec::new(); ps.len()]);
        }
        let resid = |s: &System| -> Result<Vec<f64>> {
            let r: Vec<f64> = qs
                .iter()
                .map(|q| s.solve_residual(q))
                .collect::<Result<_>>()?;
            if r.iter().any(|x| !x.is_finite()) {
                invalid!("the solve residual is not finite");
            }
            Ok(r)
        };
        let step = |v: f64| 1e-6 * (1.0 + v.abs());
        // ∂R/∂s（列 j がソルブ j の量への微分）。ソルブの量を直接書き換えた系で
        let mut js = vec![0.0; ns * ns];
        for (j, q) in qs.iter().enumerate() {
            let v = self.solved_value(q);
            let h = step(v);
            let mut r = [Vec::new(), Vec::new()];
            for (i, x) in [v + h, v - h].into_iter().enumerate() {
                let mut s = self.clone_keep(Keep {
                    frames: false,
                    paraxial: false,
                });
                s.put_solved(q.surf, q.kind.is_curv(), x)?;
                r[i] = resid(&s)?;
            }
            for i in 0..ns {
                js[i * ns + j] = (r[0][i] - r[1][i]) / (2.0 * h);
            }
        }
        let lu = Lu::new(js, ns).ok_or_else(|| {
            crate::error::Error::Invalid(
                "the solve target doesn't change with the solved value (no derivative)".into(),
            )
        })?;
        ps.iter()
            .map(|&p| {
                let v = self.get_param(p)?;
                let h = step(v);
                let a = resid(&self.with_params_solve(&[(p, v + h)], false)?)?;
                let b = resid(&self.with_params_solve(&[(p, v - h)], false)?)?;
                let jp: Vec<f64> = a
                    .iter()
                    .zip(&b)
                    .map(|(x, y)| -(x - y) / (2.0 * h))
                    .collect();
                Ok(lu.solve(&jp))
            })
            .collect()
    }

    /// 近軸主光線（[`crate::system::Paraxial::chief`]、物体空間の傾き 1 か物体の高さ 1）を最大視野（正規化視野 1）
    /// の主光線にする倍率
    fn chief_scale(&self) -> Result<f64> {
        let (_, ny) = self.field_norm_values();
        Ok(match self.field_type {
            crate::system::FieldType::Angle => crate::quantity::Angle::from_deg(ny).tan(),
            crate::system::FieldType::ObjH => ny,
            crate::system::FieldType::ParH => ny / self.paraxial().chief_parimage_per_unit,
            t => notimpl!("chief ray solve with field type {t:?}"),
        })
    }

    /// 値 v を面 k の厚さか曲率に入れて系を更新する（ピックアップも）
    fn put_solved(&mut self, k: usize, curv: bool, v: f64) -> Result<()> {
        if curv {
            self.rx_mut().surfaces[k].c = v;
        } else {
            self.rx_mut().surfaces[k].t = v;
        }
        if !self.pickups.is_empty() {
            self.resolve_pickups()?;
        }
        self.changed()
    }

    /// 面 k の前の、無視しない面の番号（なければ 0）
    fn prev_live(&self, k: usize) -> usize {
        (1..k)
            .rev()
            .find(|&j| !self.surfaces[j].ignore)
            .unwrap_or(0)
    }

    /// 面 a から面 b（a ≤ b）の頂点までの軸上の距離（無視しない面の厚さの和。鏡・傾き偏心をまたぐものは未対応）
    fn axial(&self, a: usize, b: usize) -> Result<f64> {
        let mut z = 0.0;
        for j in a..b {
            let d = &self.surfaces[j];
            if d.ignore {
                continue;
            }
            let tilted = (d.is_coord_break() && d.cb().iter().any(|&v| v != 0.0))
                || d.before.is_some()
                || matches!(d.after, After::Tilt(_));
            if d.is_mirror() || tilted {
                notimpl!("solve distance across a mirror or a tilt or decenter (surface {j})");
            }
            z += d.t;
        }
        Ok(z)
    }

    /// ソルブ 1 つを解く。値を変えたら true
    fn solve_one(&mut self, q: &Solve) -> Result<bool> {
        let k = q.surf;
        let curv = q.kind.is_curv();
        match q.kind {
            SolveKind::MarginalHeight { .. }
            | SolveKind::ChiefHeight { .. }
            | SolveKind::PupilPosition => {
                return self.solve_paraxial_height(q);
            }
            SolveKind::MarginalHeightZone { zone: z, .. } => {
                return self.solve_real_height(k, q.kind.value(), z)
            }
            SolveKind::Opd { zone: z, .. } => return self.solve_opd(k, q.kind.value(), z),
            _ => {}
        }
        let old = self.solved_value(q);
        let tol = |v: f64| 1e-14 * (1.0 + v.abs());
        let v = self.closed_value(q)?;
        // 前の値に関わらず同じ値にする（パラメータを変えて戻した系が、読み直した系とビット一致するように）
        if v != old {
            self.put_solved(k, curv, v)?;
        }
        Ok((v - old).abs() > tol(v))
    }

    /// ソルブ q で決まる量（厚さか曲率）の今の値
    fn solved_value(&self, q: &Solve) -> f64 {
        let d = &self.surfaces[q.surf];
        if q.kind.is_curv() {
            d.c
        } else {
            d.t
        }
    }

    /// ソルブ q の条件の残差（解けていれば 0）。閉じた形のソルブは「今の値 − 解いた値」、光線の高さのソルブは
    /// 「次の面での高さ − 目標」。どちらも系を書き換えずに求める（[`System::solve_sensitivity`] で使う）
    pub(crate) fn solve_residual(&self, q: &Solve) -> Result<f64> {
        Ok(match q.kind {
            SolveKind::MarginalHeight { .. }
            | SolveKind::ChiefHeight { .. }
            | SolveKind::PupilPosition => self.paraxial_height_residual(q)?.0,
            SolveKind::MarginalHeightZone { zone: z, .. } => {
                self.real_height_residual(q.surf, q.kind.value(), z)?
            }
            SolveKind::Opd { zone: z, .. } => self.opd_residual(q.kind.value(), z)?,
            _ => self.solved_value(q) - self.closed_value(q)?,
        })
    }

    /// 閉じた形で決まるソルブの値（光線の高さのソルブ以外）。系の今の値から求め、系は書き換えない
    fn closed_value(&self, q: &Solve) -> Result<f64> {
        let k = q.surf;
        let h = q.kind.value();
        let radius = |c: f64| if c == 0.0 { 1e10 } else { 1.0 / c };
        let closed: Option<f64> = match q.kind {
            SolveKind::MarginalHeight { .. }
            | SolveKind::ChiefHeight { .. }
            | SolveKind::PupilPosition
            | SolveKind::MarginalHeightZone { .. }
            | SolveKind::Opd { .. } => unreachable!(),
            SolveKind::Edge { radius: r, .. } => {
                let d = &self.surfaces[k];
                let n = &self.surfaces[k + 1];
                for (j, x) in [(k, d), (k + 1, n)] {
                    if !matches!(
                        x.typ(),
                        SurfType::Standard | SurfType::EvenAsph | SurfType::OddAsph
                    ) {
                        notimpl!("edge thickness solve on surface {j} ({:?})", x.typ());
                    }
                }
                let r = if r == 0.0 { d.semi } else { r };
                let r2 = r * r;
                // Odd Asphere は回転対称なので y 軸の上の矢高で測る
                let sag_r = |x: &crate::surface::Surface| match x.typ() {
                    SurfType::OddAsph => crate::freeform::sag_xy(x, 0.0, r).0,
                    _ => crate::trace::sag(x.c, x.k, x.asph(), r2).0,
                };
                let z1 = sag_r(d);
                let z2 = sag_r(n);
                Some(h + z1 - z2)
            }
            SolveKind::Position { other: f, .. } => {
                // 基準の面からこの面の手前までの厚さの和（基準の面が後ろなら、次の面から基準の面の手前までの和。
                // ピックアップで決まる厚さも含む）
                let rest = if f <= k { f..k } else { k + 1..f };
                let t = h - self.surfaces[rest].iter().map(|d| d.t).sum::<f64>();
                if !t.is_finite() {
                    invalid!("surface {k}: the position solve thickness is not finite (sum from surface {f})");
                }
                Some(t)
            }
            SolveKind::Compensator { other: s0, .. } => {
                if s0 == k {
                    invalid!("surface {k}: compensator solve referring to itself");
                }
                Some(h - self.surfaces[s0].t)
            }
            SolveKind::CenterOfCurvature { other: s0 } => {
                if s0 > k {
                    notimpl!("surface {k}: center of curvature solve on the later surface {s0}");
                }
                Some(radius(self.surfaces[s0].c) - self.axial(s0, k)?)
            }
            SolveKind::ConcentricSurface { other: j } => {
                let z = if j >= k {
                    self.axial(k, j)?
                } else {
                    -self.axial(j, k)?
                };
                Some(if z == 0.0 { 0.0 } else { 1.0 / z })
            }
            SolveKind::ConcentricRadius { other: j } => {
                let z = if j >= k {
                    self.axial(k, j)?
                } else {
                    -self.axial(j, k)?
                };
                // 平面の基準は R = −1e10（OpticStudio 2026 R1.01 の値 c = −1e-10 に合わせる。曲率中心のソルブは +1e10）
                let cj = self.surfaces[j].c;
                let r = z + if cj == 0.0 { -1e10 } else { 1.0 / cj };
                Some(if r == 0.0 { 0.0 } else { 1.0 / r })
            }
            SolveKind::ElementPower { .. } => Some(self.element_power_curv(k, h)?),
            SolveKind::MacroCurv { index: i } | SolveKind::MacroThick { index: i } => {
                let Some(m) = self.macros.get(i as usize) else {
                    invalid!("surface {k}: no macro {i} ({} in all)", self.macros.len());
                };
                // 値を入れずに止まったマクロ（OpticStudio は値を変えない）は今の値のまま
                Some(self.run_macro(m, k)?.unwrap_or(self.solved_value(q)))
            }
            _ => Some(self.paraxial_curv(q)?),
        };
        let v = closed.expect("closed form exists");
        if !v.is_finite() {
            invalid!(
                "surface {k}: solve {} can't be solved ({v})",
                solve_kind_info(q.kind).0
            );
        }
        Ok(v)
    }

    /// 近軸光線の傾き・垂直・不遊・F/# の曲率のソルブ
    fn paraxial_curv(&self, q: &Solve) -> Result<f64> {
        let k = q.surf;
        let d = &self.surfaces[k];
        if d.is_mirror()
            || self.surfaces[1..k]
                .iter()
                .any(|x| x.is_mirror() && !x.ignore)
        {
            notimpl!("surface {k}: ray curvature solve on a mirror or a surface after a mirror");
        }
        if !matches!(
            d.typ(),
            SurfType::Standard | SurfType::EvenAsph | SurfType::Toroidal
        ) {
            notimpl!("curvature solve on surface {k} ({:?})", d.typ());
        }
        let pw = self.primary;
        let par = self.paraxial();
        let chief = matches!(
            q.kind,
            SolveKind::ChiefAngle { .. } | SolveKind::ChiefNormal
        );
        let (y, u) = if chief { &par.chief } else { &par.marginal };
        // 主光線は最大視野（正規化視野 1）のもの（OpticStudio と同じ）
        let f = if chief { self.chief_scale()? } else { 1.0 };
        let (y, u) = (f * y[k], f * u[self.prev_live(k)]);
        let n = self.index_before(k, pw);
        let n2 = self.n(k)[pw];
        // 近軸の曲率のうち c に依らない分（偶数次非球面の r²・トロイダル面の y² の項。近軸の追跡と同じ）
        let a1 = d.shape().paraxial_extra().unwrap_or(0.0);
        if y == 0.0 {
            invalid!("surface {k}: the ray height is 0, so the curvature solve can't be solved");
        }
        let ce = match q.kind {
            // n′u′ = n u − y c (n′ − n)
            SolveKind::MarginalAngle { .. }
            | SolveKind::ChiefAngle { .. }
            | SolveKind::FNumber { .. } => {
                if n2 == n {
                    invalid!("surface {k}: the same index on both sides, so the ray angle solve can't be solved");
                }
                let a = if matches!(q.kind, SolveKind::FNumber { .. }) {
                    if q.kind.value() == 0.0 {
                        invalid!("surface {k}: F/# 0 solve");
                    }
                    -1.0 / (2.0 * q.kind.value())
                } else {
                    q.kind.value()
                };
                (n * u - n2 * a) / (y * (n2 - n))
            }
            SolveKind::MarginalNormal | SolveKind::ChiefNormal => -u / y,
            SolveKind::Aplanatic => -u * (n2 + n) / (n * y),
            _ => unreachable!(),
        };
        Ok(ce - a1)
    }

    /// 玉のパワーの曲率のソルブ: 前の面と面 k の組（厚肉レンズ）のパワーを p にする（OpticStudio 2026 R1.01 で実測:
    /// 面 k の後ろがガラスでも前の面と組む）
    fn element_power_curv(&self, k: usize, p: f64) -> Result<f64> {
        let pw = self.primary;
        let (a, b) = (self.prev_live(k), k);
        if a == 0 {
            invalid!("surface {k}: surface pair of the lens power solve (surfaces {a} and {b})");
        }
        for j in [a, b] {
            let d = &self.surfaces[j];
            if d.is_mirror() || !matches!(d.typ(), SurfType::Standard | SurfType::EvenAsph) {
                notimpl!(
                    "lens power solve on a lens with surface {j} ({:?})",
                    d.typ()
                );
            }
        }
        let n0 = self.index_before(a, pw);
        let ng = self.n(a)[pw];
        let n1 = self.n(b)[pw];
        let t = self.axial(a, b)?;
        let ce = |j: usize| {
            let d = &self.surfaces[j];
            let a1 = d.shape().paraxial_extra().unwrap_or(0.0);
            (d.c + a1, a1)
        };
        // φ = φa + φb − (t/ng) φa φb、φa = ca (ng − n0)、φb = cb (n1 − ng)
        let (fa, fb) = (ng - n0, n1 - ng);
        let c = if k == a {
            let (cb, _) = ce(b);
            let pb = cb * fb;
            let den = fa * (1.0 - t / ng * pb);
            (p - pb) / den
        } else {
            let (ca, _) = ce(a);
            let pa = ca * fa;
            let den = fb * (1.0 - t / ng * pa);
            (p - pa) / den
        };
        Ok(c - ce(k).1)
    }

    /// 近軸マージナル光線・主光線の高さ（瞳の位置は主光線の高さ 0）の厚さのソルブ。ソルブの面より後ろに絞りがあると
    /// 入射瞳が動いて光線もその厚さで変わるので、割線法で解く
    fn solve_paraxial_height(&mut self, q: &Solve) -> Result<bool> {
        let k = q.surf;
        let chief = !matches!(q.kind, SolveKind::MarginalHeight { .. });
        if q.kind == SolveKind::PupilPosition && k < self.stop() {
            // OpticStudio は絞りより前の面では厚さを 0 にする（意味のある位置がない）
            notimpl!("surface {k}: pupil position solve on a surface before the stop");
        }
        // 面 k の厚さを t にした系で、次の面での光線の高さ − 目標と、高さの t への傾き（t を固定した近似）
        let eval = |s: &mut System, t: f64| -> Result<(f64, f64)> {
            s.put_solved(k, false, t)?;
            s.paraxial_height_residual(q)
        };
        // 前の値に関わらず厚さ 0 から解く（解きの値が前の値の最後の桁で変わらないように）
        let old = self.surfaces[k].t;
        let (f0, du) = eval(self, 0.0)?;
        if !(du.abs() > 0.0) || !f0.is_finite() {
            self.put_solved(k, false, old)?;
            if chief {
                invalid!("surface {k}: the chief ray is parallel to the axis (thickness solve)");
            }
            invalid!("surface {k}: the marginal ray is parallel to the axis (thickness solve)");
        }
        let t = self.secant(k, 0.0, f0, -f0 / du, |s, t| Ok(eval(s, t)?.0))?;
        self.finish_thick(k, old, t)
    }

    /// 近軸光線の高さ・瞳の位置の厚さのソルブの、今の厚さでの次の面での高さ − 目標と、高さの厚さへの傾き
    /// （面 k での高さと傾きを固定した値）
    fn paraxial_height_residual(&self, q: &Solve) -> Result<(f64, f64)> {
        let k = q.surf;
        let (chief, h) = match q.kind {
            SolveKind::MarginalHeight { .. } => (false, q.kind.value()),
            SolveKind::ChiefHeight { .. } => (true, q.kind.value()),
            _ => (true, 0.0),
        };
        let t = self.surfaces[k].t;
        let par = self.paraxial();
        let (y, u) = if chief { &par.chief } else { &par.marginal };
        let (y, u) = (y[k], u[k]);
        // 主光線は最大視野（正規化視野 1）のもの
        let f = if chief && h != 0.0 {
            self.chief_scale()?
        } else {
            1.0
        };
        // 鏡のあとは展開した向き（近軸の追跡と同じく、厚さは sgn·t で進む）
        let nmir = self.surfaces[1..=k]
            .iter()
            .filter(|d| d.is_mirror() && !d.ignore)
            .count();
        let du = f * if nmir % 2 == 1 { -u } else { u };
        Ok((f * y + du * t - h, du))
    }

    /// 実光線（軸上、瞳 (0, 帯)）の次の面での高さの厚さのソルブ
    fn solve_real_height(&mut self, k: usize, h: f64, zone: f64) -> Result<bool> {
        let (d, n) = (&self.surfaces[k], &self.surfaces[k + 1]);
        if d.is_mirror()
            || d.is_coord_break()
            || d.after != After::None
            || n.before.is_some()
            || n.is_coord_break()
            || n.ignore
        {
            notimpl!(
                "surface {k}: real ray height thickness solve next to a mirror, tilt or decenter, coordinate break or ignored surface"
            );
        }
        let eval = |s: &mut System, t: f64| -> Result<f64> {
            s.put_solved(k, false, t)?;
            s.real_height_residual(k, h, zone)
        };
        // 前の値に関わらず厚さ 0 から解く（高さは面 k の後の光線を延ばして測るので、次の面の位置によらない）
        let old = self.surfaces[k].t;
        let t0 = 0.0;
        let f0 = eval(self, t0)?;
        if !f0.is_finite() {
            self.put_solved(k, false, old)?;
            invalid!("surface {k}: the real ray (pupil zone {zone}) doesn't pass, so the thickness solve can't be solved");
        }
        // 初めの傾きは差分で
        let dt = 1e-6;
        let f1 = eval(self, t0 + dt)?;
        let slope = (f1 - f0) / dt;
        if !(slope.abs() > 0.0) {
            self.put_solved(k, false, old)?;
            invalid!("surface {k}: the real ray height doesn't change with thickness, so the thickness solve can't be solved");
        }
        let t = self.secant(k, t0, f0, t0 - f0 / slope, eval)?;
        self.finish_thick(k, old, t)
    }

    /// 実光線の高さの厚さのソルブの、今の厚さでの次の面での高さ − 目標（光線が通らなければ NaN）
    fn real_height_residual(&self, k: usize, h: f64, zone: f64) -> Result<f64> {
        let t = self.surfaces[k].t;
        // 高さは次の面の頂点の接平面で測る（OpticStudio 2026 R1.01 の値に合う）: 面 k の後の光線を延ばす
        let tr = self.try_trace(
            crate::system::FieldSel::Norm(0.0, 0.0),
            &[0.0],
            &[zone],
            None,
            Some(k),
            false,
        )?;
        let r = &tr.rays[0];
        if r.status.surf().is_some() || r.lmn[2] == 0.0 {
            return Ok(f64::NAN);
        }
        Ok(r.xyz[1] + r.lmn[1] / r.lmn[2] * (t - r.xyz[2]) - h)
    }

    /// OPD の厚さのソルブ: 像面での OPD [波]（軸上の視野、主波長、瞳 (0, 帯)）を opd にする。OPD は厚さに
    /// 対して単調とは限らない（根が複数ありうる）ので、今の厚さから割線法で解く。
    ///
    /// ほかのソルブ（像面の焦点の MAZH など）は厚さを変えるたびに解き直してから OPD を測る（全部のソルブを同時に
    /// 満たす値。ほかのソルブを止めたままでは解がないことがある: 焦点ソルブの前の面の OPD のソルブ）。
    /// OPD のソルブどうしは外の繰り返し（[`System::solve_thickness`]）で揃える
    fn solve_opd(&mut self, k: usize, opd: f64, zone: f64) -> Result<bool> {
        if self.surfaces[k].is_mirror() || self.surfaces[k].is_coord_break() {
            notimpl!("surface {k}: OPD thickness solve on a mirror or coordinate break");
        }
        if k + 1 >= self.surfaces.len() {
            invalid!("surface {k}: OPD thickness solve on the image surface");
        }
        let others: Vec<Solve> = self
            .active_solves()
            .into_iter()
            .filter(|q| !matches!(q.kind, SolveKind::Opd { .. }))
            .collect();
        let eval = |s: &mut System, t: f64| -> Result<f64> {
            s.put_solved(k, false, t)?;
            if !others.is_empty() || !s.macro_parms.is_empty() {
                let mut done = false;
                for _ in 0..100 {
                    let mut moved = false;
                    for q in &others {
                        moved |= s.solve_one(q)?;
                    }
                    if !s.macro_parms.is_empty() {
                        moved |= s.solve_macro_parms()?;
                    }
                    if !moved {
                        done = true;
                        break;
                    }
                }
                if !done {
                    invalid!("surface {k}: another solve doesn't converge inside the OPD thickness solve");
                }
            }
            s.opd_residual(opd, zone)
        };
        let old = self.surfaces[k].t;
        let f0 = eval(self, old)?;
        if !f0.is_finite() {
            self.put_solved(k, false, old)?;
            invalid!("surface {k}: the real ray (pupil zone {zone}) doesn't pass, so the OPD thickness solve can't be solved");
        }
        // OPD は光路長の差なので丸めの揺れ（1e-11 波ほど）がある。揺れの中なら解けているとして動かさない
        // （動かすと外の繰り返しが揺れで止まらない）
        if f0.abs() <= OPD_SOLVE_TOL {
            return Ok(false);
        }
        let dt = 1e-6 * (1.0 + old.abs());
        let f1 = eval(self, old + dt)?;
        let slope = (f1 - f0) / dt;
        if !(slope.abs() > 0.0) || !slope.is_finite() {
            self.put_solved(k, false, old)?;
            invalid!("surface {k}: the OPD doesn't change with thickness, so the OPD thickness solve can't be solved");
        }
        let t = self.secant(k, old, f0, old - f0 / slope, eval)?;
        self.finish_thick(k, old, t)
    }

    /// OPD の厚さのソルブの、今の系での像面の OPD [波] − 目標（光線が通らなければ NaN）
    fn opd_residual(&self, opd: f64, zone: f64) -> Result<f64> {
        let o = crate::analysis::opd(
            self,
            crate::system::FieldSel::Norm(0.0, 0.0),
            &[0.0],
            &[zone],
            None,
            true,
        )?;
        if !o.ok(0) {
            return Ok(f64::NAN);
        }
        Ok(o.opd[0] - opd)
    }

    /// 解いた厚さ t を入れる。前の値 old から許容より大きく変わったら true
    fn finish_thick(&mut self, k: usize, old: f64, t: f64) -> Result<bool> {
        self.put_solved(k, false, t)?;
        Ok((t - old).abs() > 1e-14 * (1.0 + t.abs()))
    }

    /// 厚さの割線法（t0 で f0、次の候補 t1）。根を返す（系の厚さは途中の値のまま）
    fn secant(
        &mut self,
        k: usize,
        t0: f64,
        f0: f64,
        t1: f64,
        mut eval: impl FnMut(&mut System, f64) -> Result<f64>,
    ) -> Result<f64> {
        let tol = |t: f64| 1e-14 * (1.0 + t.abs());
        let (mut t0, mut f0, mut t1) = (t0, f0, t1);
        let mut done = false;
        for _ in 0..100 {
            if (t1 - t0).abs() <= tol(t1) {
                done = true;
                break;
            }
            let f1 = eval(self, t1)?;
            if f1 == 0.0 || f1 == f0 {
                done = f1 == 0.0 || (t1 - t0).abs() <= 1e-9 * (1.0 + t1.abs());
                break;
            }
            let t2 = t1 - f1 * (t1 - t0) / (f1 - f0);
            (t0, f0, t1) = (t1, f1, t2);
            if !t1.is_finite() {
                break;
            }
        }
        if !done {
            invalid!("surface {k}: the thickness solve doesn't converge");
        }
        Ok(t1)
    }
}

/// OPD の厚さのソルブの許容 [波]（解けているとみなす OPD の残り）
const OPD_SOLVE_TOL: f64 = 1e-9;

/// ピックアップの量がソルブの決める量（厚さ・曲率）に当たるならその種類
pub(crate) fn solve_kind_of(w: crate::param::Kind) -> Option<Kind> {
    use crate::param::Kind;
    match w {
        Kind::Thick => Some(Kind::Thick),
        Kind::Curv => Some(Kind::Curv),
        _ => None,
    }
}

/// ソルブの種類の名前（[`solve_kind_parse`]・[`solve_kind_info`]）
pub const SOLVE_KINDS: &[&str] = &[
    "marginal_height",
    "marginal_height_zone",
    "chief_height",
    "pupil_position",
    "edge",
    "opd",
    "position",
    "compensator",
    "center_of_curvature",
    "marginal_angle",
    "chief_angle",
    "marginal_normal",
    "chief_normal",
    "aplanatic",
    "element_power",
    "concentric_surface",
    "concentric_radius",
    "f_number",
];

/// ソルブの種類を名前から。value は条件の値（[`SolveKind::value`]）、param は marginal_height_zone・opd の瞳の帯・
/// edge の高さ・マクロの番号、other は別の面を参照する種類の面
pub fn solve_kind_parse(
    name: &str,
    value: f64,
    param: f64,
    other: Option<usize>,
) -> Result<SolveKind> {
    use SolveKind as K;
    let n = name.trim().to_ascii_lowercase();
    let need = || -> Result<usize> {
        match other {
            Some(s) => Ok(s),
            None => invalid!("solve {n} needs another surface (other)"),
        }
    };
    let v = value;
    Ok(match n.as_str() {
        "marginal_height" => K::MarginalHeight { height: v },
        "marginal_height_zone" => K::MarginalHeightZone {
            height: v,
            zone: param,
        },
        "chief_height" => K::ChiefHeight { height: v },
        "pupil_position" => K::PupilPosition,
        "edge" => K::Edge {
            thickness: v,
            radius: param,
        },
        "opd" => K::Opd {
            opd: v,
            zone: param,
        },
        "position" => K::Position {
            other: need()?,
            length: v,
        },
        "compensator" => K::Compensator {
            other: need()?,
            sum: v,
        },
        "center_of_curvature" => K::CenterOfCurvature { other: need()? },
        "marginal_angle" => K::MarginalAngle { angle: v },
        "chief_angle" => K::ChiefAngle { angle: v },
        "marginal_normal" => K::MarginalNormal,
        "chief_normal" => K::ChiefNormal,
        "aplanatic" => K::Aplanatic,
        "element_power" => K::ElementPower { power: v },
        "concentric_surface" => K::ConcentricSurface { other: need()? },
        "concentric_radius" => K::ConcentricRadius { other: need()? },
        "f_number" => K::FNumber { fnum: v },
        // ZPL のマクロ: param は系のマクロの番号（System::macros）。マクロは .zmx の読込で付くので、
        // 新しく選ぶ種類の一覧（SOLVE_KINDS）には入れない
        "macro_curv" | "macro_thick" => {
            if !(param >= 0.0 && param.fract() == 0.0 && param <= u16::MAX as f64) {
                invalid!("macro number {param}");
            }
            let index = param as u16;
            if n == "macro_curv" {
                K::MacroCurv { index }
            } else {
                K::MacroThick { index }
            }
        }
        _ => invalid!("solve type {name} ({})", SOLVE_KINDS.join(" ")),
    })
}

/// ソルブの種類の (名前, value, param, other)（[`solve_kind_parse`] の逆）
pub fn solve_kind_info(k: SolveKind) -> (&'static str, f64, f64, Option<usize>) {
    use SolveKind as K;
    let (o, v) = (k.other_surface(), k.value());
    let (name, param) = match k {
        K::MarginalHeight { .. } => ("marginal_height", 0.0),
        K::MarginalHeightZone { zone, .. } => ("marginal_height_zone", zone),
        K::ChiefHeight { .. } => ("chief_height", 0.0),
        K::PupilPosition => ("pupil_position", 0.0),
        K::Edge { radius, .. } => ("edge", radius),
        K::Opd { zone, .. } => ("opd", zone),
        K::Position { .. } => ("position", 0.0),
        K::Compensator { .. } => ("compensator", 0.0),
        K::CenterOfCurvature { .. } => ("center_of_curvature", 0.0),
        K::MarginalAngle { .. } => ("marginal_angle", 0.0),
        K::ChiefAngle { .. } => ("chief_angle", 0.0),
        K::MarginalNormal => ("marginal_normal", 0.0),
        K::ChiefNormal => ("chief_normal", 0.0),
        K::Aplanatic => ("aplanatic", 0.0),
        K::ElementPower { .. } => ("element_power", 0.0),
        K::ConcentricSurface { .. } => ("concentric_surface", 0.0),
        K::ConcentricRadius { .. } => ("concentric_radius", 0.0),
        K::FNumber { .. } => ("f_number", 0.0),
        K::MacroCurv { index } => ("macro_curv", index as f64),
        K::MacroThick { index } => ("macro_thick", index as f64),
    };
    (name, v, param, o)
}
