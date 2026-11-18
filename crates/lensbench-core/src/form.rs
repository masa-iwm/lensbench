//! 面の種類と、種類ごとの値（[`Form`]）。
//!
//! 種類にしか意味の無い値（座標ブレークのずらし・理想レンズの焦点距離・Jones 行列・Zernike の項・格子・
//! 屈折率分布・利用者定義の面、非球面の係数・Tilted の傾き・Irregular の値）は種類の枝の中に持つ。曲率・円錐係数の
//! ように多くの種類が共通に使う値は [`crate::Surface`] に残し、種類ごとの意味は形の見方 [`crate::Surface::shape`] で
//! 分ける。非球面の係数の並びは [`crate::Surface::asph`] で、どの種類でも同じ形で読み書きできる（Tilted・Irregular は
//! 決まった数の値の並び）。
//! 種類だけを比べるときは [`crate::Surface::typ`]（[`SurfType`]）

use crate::freeform::{GridSag, ZernSag};
use crate::grin::Grin;
use crate::surface::{Binary2, CylFresnel, SurfType, Surface};
use crate::usershape::UserShape;
use crate::usersurf::UserSurf;
use std::sync::Arc;

/// 単位行列の Jones 行列（実部・虚部の順に Jxx Jxy Jyx Jyy）
pub const JONES_IDENTITY: [f64; 8] = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// 面の種類と、種類ごとの値。組み込みの種類を足しても外のクレートが壊れないよう `non_exhaustive`
///
/// serde では面の表の中に平たく（`typ` と、どの種類でも全部の値の欄。種類に無い値は既定）書く。読むときは
/// 種類に無い欄に既定でない値があれば誤り
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(into = "FormFlat", try_from = "FormFlat")]
#[non_exhaustive]
pub enum Form {
    #[default]
    Standard,
    /// 偶数次非球面: 係数 r², r⁴, …
    EvenAsph { asph: Vec<f64> },
    /// 座標ブレーク: [偏心 x, 偏心 y, 傾き x°, 傾き y°, 傾き z°] と順序（0 は偏心してから傾け、1 は逆）
    CoordBrk { cb: [f64; 5], order: u8 },
    /// Jones 面: 行列（実部・虚部の順に Jxx Jxy Jyx Jyy）
    Jones { jones: [f64; 8] },
    /// 理想レンズ: 焦点距離 [mm] と OPD の扱い（.zmx の PARM 2）
    Paraxial { focal: f64, opd_mode: i32 },
    /// トロイダル面: 回転の曲率（1 / 回転半径、0 は円柱面）と YZ 面の曲線の係数 y², y⁴, …
    Toroidal { cx: f64, asph: Vec<f64> },
    /// Zernike Standard Sag: 土台の偶数次非球面の係数と Zernike の項
    ZernikeStd {
        asph: Vec<f64>,
        zern: Option<Arc<ZernSag>>,
    },
    /// Zernike Fringe Sag: 土台の偶数次非球面の係数と Zernike の項
    ZernikeFringe {
        asph: Vec<f64>,
        zern: Option<Arc<ZernSag>>,
    },
    /// Grid Sag: 土台の偶数次非球面の係数、格子と、その .DAT のファイル名（書き出し用）、足す Zernike（Standard）の項
    GridSag {
        asph: Vec<f64>,
        grid: Option<Arc<GridSag>>,
        grid_file: String,
        zern: Option<Arc<ZernSag>>,
    },
    /// Fresnel: 屈折の傾きを決める偶数次非球面の係数
    Fresnel { asph: Vec<f64> },
    /// Cylinder Fresnel: 基板の係数（y だけ）と屈折の面
    CylFresnel {
        asph: Vec<f64>,
        cylf: Option<Arc<CylFresnel>>,
    },
    /// Binary 2: 形の偶数次非球面の係数と位相
    Binary2 {
        asph: Vec<f64>,
        bin2: Option<Arc<Binary2>>,
    },
    /// Gradium: 面の後ろの屈折率分布（`n` は頂点の屈折率）
    Gradium { grin: Option<Arc<Grin>> },
    /// Gradient 9: 面の後ろの屈折率分布
    Gradient9 { grin: Option<Arc<Grin>> },
    /// Odd Asphere: β_1..β_8（Σ β_i r^i）
    OddAsph { asph: Vec<f64> },
    /// Tilted: [tan α, tan β]（z = x tan α + y tan β）
    TiltSurf { tan: [f64; 2] },
    /// Irregular: [偏心 x, 偏心 y, 傾き x°, 傾き y°, Zs, Za, Zc, 角度°]（[`crate::shape::Irregular`]）
    Irregular { p: [f64; 8] },
    /// 利用者定義の面のモデルと設定
    User { user: Option<UserSurf> },
    /// 利用者定義の形のモデルと設定（土台は面の c・k）
    UserShape { shape: Option<UserShape> },
}

impl Form {
    /// 種類 t の、値が既定の枝（Jones は単位行列）
    pub fn of(t: SurfType) -> Form {
        match t {
            SurfType::Standard => Form::Standard,
            SurfType::EvenAsph => Form::EvenAsph { asph: Vec::new() },
            SurfType::CoordBrk => Form::CoordBrk {
                cb: [0.0; 5],
                order: 0,
            },
            SurfType::Jones => Form::Jones {
                jones: JONES_IDENTITY,
            },
            SurfType::Paraxial => Form::Paraxial {
                focal: 0.0,
                opd_mode: 0,
            },
            SurfType::Toroidal => Form::Toroidal {
                cx: 0.0,
                asph: Vec::new(),
            },
            SurfType::ZernikeStd => Form::ZernikeStd {
                asph: Vec::new(),
                zern: None,
            },
            SurfType::ZernikeFringe => Form::ZernikeFringe {
                asph: Vec::new(),
                zern: None,
            },
            SurfType::GridSag => Form::GridSag {
                asph: Vec::new(),
                grid: None,
                grid_file: String::new(),
                zern: None,
            },
            SurfType::Fresnel => Form::Fresnel { asph: Vec::new() },
            SurfType::CylFresnel => Form::CylFresnel {
                asph: Vec::new(),
                cylf: None,
            },
            SurfType::Binary2 => Form::Binary2 {
                asph: Vec::new(),
                bin2: None,
            },
            SurfType::Gradium => Form::Gradium { grin: None },
            SurfType::Gradient9 => Form::Gradient9 { grin: None },
            SurfType::OddAsph => Form::OddAsph { asph: Vec::new() },
            SurfType::TiltSurf => Form::TiltSurf { tan: [0.0; 2] },
            SurfType::Irregular => Form::Irregular { p: [0.0; 8] },
            SurfType::User => Form::User { user: None },
            SurfType::UserShape => Form::UserShape { shape: None },
        }
    }

    /// 種類（値を除く）
    pub fn typ(&self) -> SurfType {
        match self {
            Form::Standard => SurfType::Standard,
            Form::EvenAsph { .. } => SurfType::EvenAsph,
            Form::CoordBrk { .. } => SurfType::CoordBrk,
            Form::Jones { .. } => SurfType::Jones,
            Form::Paraxial { .. } => SurfType::Paraxial,
            Form::Toroidal { .. } => SurfType::Toroidal,
            Form::ZernikeStd { .. } => SurfType::ZernikeStd,
            Form::ZernikeFringe { .. } => SurfType::ZernikeFringe,
            Form::GridSag { .. } => SurfType::GridSag,
            Form::Fresnel { .. } => SurfType::Fresnel,
            Form::CylFresnel { .. } => SurfType::CylFresnel,
            Form::Binary2 { .. } => SurfType::Binary2,
            Form::Gradium { .. } => SurfType::Gradium,
            Form::Gradient9 { .. } => SurfType::Gradient9,
            Form::OddAsph { .. } => SurfType::OddAsph,
            Form::TiltSurf { .. } => SurfType::TiltSurf,
            Form::Irregular { .. } => SurfType::Irregular,
            Form::User { .. } => SurfType::User,
            Form::UserShape { .. } => SurfType::UserShape,
        }
    }
}

impl Surface {
    /// 面の種類
    pub fn typ(&self) -> SurfType {
        self.form.typ()
    }

    /// 種類を t に変える。同じ種類なら何もしない。持ち越せる値（Zernike の項・屈折率分布・非球面の係数の並び）は
    /// 持ち越し（係数を持たない種類では捨てる）、ほかの種類ごとの値は既定に戻す
    pub fn set_type(&mut self, t: SurfType) {
        if self.typ() == t {
            return;
        }
        let zern = match &self.form {
            Form::ZernikeStd { zern, .. }
            | Form::ZernikeFringe { zern, .. }
            | Form::GridSag { zern, .. } => zern.clone(),
            _ => None,
        };
        let grin = match &self.form {
            Form::Gradium { grin } | Form::Gradient9 { grin } => grin.clone(),
            _ => None,
        };
        let asph = self.asph().to_vec();
        self.form = Form::of(t);
        self.set_asph(asph);
        match &mut self.form {
            Form::ZernikeStd { zern: z, .. }
            | Form::ZernikeFringe { zern: z, .. }
            | Form::GridSag { zern: z, .. } => *z = zern,
            Form::Gradium { grin: g } | Form::Gradient9 { grin: g } => *g = grin,
            _ => {}
        }
    }

    /// トロイダル面の回転の曲率（ほかの種類は 0）
    pub fn cx(&self) -> f64 {
        match self.form {
            Form::Toroidal { cx, .. } => cx,
            _ => 0.0,
        }
    }

    /// Zernike の項（Zernike Sag・Grid Sag）
    pub fn zern(&self) -> Option<&ZernSag> {
        match &self.form {
            Form::ZernikeStd { zern, .. }
            | Form::ZernikeFringe { zern, .. }
            | Form::GridSag { zern, .. } => zern.as_deref(),
            _ => None,
        }
    }

    /// Zernike の項の置き場（Zernike Sag・Grid Sag の面だけ）
    pub fn zern_mut(&mut self) -> Option<&mut Option<Arc<ZernSag>>> {
        match &mut self.form {
            Form::ZernikeStd { zern, .. }
            | Form::ZernikeFringe { zern, .. }
            | Form::GridSag { zern, .. } => Some(zern),
            _ => None,
        }
    }

    /// Grid Sag の格子
    pub fn grid(&self) -> Option<&GridSag> {
        match &self.form {
            Form::GridSag { grid, .. } => grid.as_deref(),
            _ => None,
        }
    }

    /// Grid Sag の .DAT のファイル名（ほかの種類は空）
    pub fn grid_file(&self) -> &str {
        match &self.form {
            Form::GridSag { grid_file, .. } => grid_file,
            _ => "",
        }
    }

    /// Binary 2 の位相
    pub fn bin2(&self) -> Option<&Binary2> {
        match &self.form {
            Form::Binary2 { bin2, .. } => bin2.as_deref(),
            _ => None,
        }
    }

    /// Cylinder Fresnel の屈折の面
    pub fn cylf(&self) -> Option<&CylFresnel> {
        match &self.form {
            Form::CylFresnel { cylf, .. } => cylf.as_deref(),
            _ => None,
        }
    }

    /// 面の後ろの屈折率分布（Gradium・Gradient 9）
    pub fn grin(&self) -> Option<&Grin> {
        match &self.form {
            Form::Gradium { grin } | Form::Gradient9 { grin } => grin.as_deref(),
            _ => None,
        }
    }

    /// 座標ブレークのずらし（ほかの種類は 0）
    pub fn cb(&self) -> [f64; 5] {
        match self.form {
            Form::CoordBrk { cb, .. } => cb,
            _ => [0.0; 5],
        }
    }

    /// 座標ブレークの順序（ほかの種類は 0）
    pub fn cb_order(&self) -> u8 {
        match self.form {
            Form::CoordBrk { order, .. } => order,
            _ => 0,
        }
    }

    /// 理想レンズの焦点距離（ほかの種類は 0）
    pub fn focal(&self) -> f64 {
        match self.form {
            Form::Paraxial { focal, .. } => focal,
            _ => 0.0,
        }
    }

    /// 理想レンズの OPD の扱い（ほかの種類は 0）
    pub fn opd_mode(&self) -> i32 {
        match self.form {
            Form::Paraxial { opd_mode, .. } => opd_mode,
            _ => 0,
        }
    }

    /// Jones 面の行列（ほかの種類は単位行列）
    pub fn jones(&self) -> [f64; 8] {
        match self.form {
            Form::Jones { jones } => jones,
            _ => JONES_IDENTITY,
        }
    }

    /// 利用者定義の面のモデルと設定
    pub fn user(&self) -> Option<&UserSurf> {
        match &self.form {
            Form::User { user } => user.as_ref(),
            _ => None,
        }
    }

    /// 利用者定義の形のモデルと設定
    pub fn user_shape(&self) -> Option<&UserShape> {
        match &self.form {
            Form::UserShape { shape } => shape.as_ref(),
            _ => None,
        }
    }

    /// 座標ブレークのずらしと順序の置き場（座標ブレークだけ）
    pub fn cb_mut(&mut self) -> Option<(&mut [f64; 5], &mut u8)> {
        match &mut self.form {
            Form::CoordBrk { cb, order } => Some((cb, order)),
            _ => None,
        }
    }

    /// 理想レンズの焦点距離と OPD の扱いの置き場（理想レンズだけ）
    pub fn paraxial_mut(&mut self) -> Option<(&mut f64, &mut i32)> {
        match &mut self.form {
            Form::Paraxial { focal, opd_mode } => Some((focal, opd_mode)),
            _ => None,
        }
    }

    /// Jones 行列の置き場（Jones 面だけ）
    pub fn jones_mut(&mut self) -> Option<&mut [f64; 8]> {
        match &mut self.form {
            Form::Jones { jones } => Some(jones),
            _ => None,
        }
    }

    /// トロイダル面の回転の曲率の置き場（トロイダル面だけ）
    pub fn cx_mut(&mut self) -> Option<&mut f64> {
        match &mut self.form {
            Form::Toroidal { cx, .. } => Some(cx),
            _ => None,
        }
    }

    /// Grid Sag の .DAT のファイル名の置き場（Grid Sag だけ）
    pub fn grid_file_mut(&mut self) -> Option<&mut String> {
        match &mut self.form {
            Form::GridSag { grid_file, .. } => Some(grid_file),
            _ => None,
        }
    }

    /// Grid Sag の格子の置き場（Grid Sag だけ）
    pub fn grid_mut(&mut self) -> Option<&mut Option<Arc<GridSag>>> {
        match &mut self.form {
            Form::GridSag { grid, .. } => Some(grid),
            _ => None,
        }
    }

    /// Binary 2 の位相の置き場（Binary 2 だけ）
    pub fn bin2_mut(&mut self) -> Option<&mut Option<Arc<Binary2>>> {
        match &mut self.form {
            Form::Binary2 { bin2, .. } => Some(bin2),
            _ => None,
        }
    }

    /// Cylinder Fresnel の屈折の面の置き場（Cylinder Fresnel だけ）
    pub fn cylf_mut(&mut self) -> Option<&mut Option<Arc<CylFresnel>>> {
        match &mut self.form {
            Form::CylFresnel { cylf, .. } => Some(cylf),
            _ => None,
        }
    }

    /// 利用者定義の面のモデルの置き場（利用者定義の面だけ）
    pub fn user_mut(&mut self) -> Option<&mut Option<UserSurf>> {
        match &mut self.form {
            Form::User { user } => Some(user),
            _ => None,
        }
    }

    /// 利用者定義の形のモデルの置き場（利用者定義の形の面だけ）
    pub fn user_shape_mut(&mut self) -> Option<&mut Option<UserShape>> {
        match &mut self.form {
            Form::UserShape { shape } => Some(shape),
            _ => None,
        }
    }

    /// 屈折率分布の置き場（Gradium・Gradient 9 だけ）
    pub fn grin_mut(&mut self) -> Option<&mut Option<Arc<Grin>>> {
        match &mut self.form {
            Form::Gradium { grin } | Form::Gradient9 { grin } => Some(grin),
            _ => None,
        }
    }

    /// 非球面の係数の並び（偶数次非球面・トロイダル・Zernike・Grid Sag・Fresnel・Cylinder Fresnel・Binary 2 は
    /// 偶数次の係数、Odd Asphere は β、Tilted は [tan α, tan β]、Irregular はその 8 つの値。ほかの種類は空）
    pub fn asph(&self) -> &[f64] {
        match &self.form {
            Form::EvenAsph { asph }
            | Form::Toroidal { asph, .. }
            | Form::ZernikeStd { asph, .. }
            | Form::ZernikeFringe { asph, .. }
            | Form::GridSag { asph, .. }
            | Form::Fresnel { asph }
            | Form::CylFresnel { asph, .. }
            | Form::Binary2 { asph, .. }
            | Form::OddAsph { asph } => asph,
            Form::TiltSurf { tan } => tan,
            Form::Irregular { p } => p,
            _ => &[],
        }
    }

    /// 非球面の係数の並びの置き場（長さを変えられる種類だけ。Tilted・Irregular は [`Surface::asph_fixed_mut`]）
    pub fn asph_mut(&mut self) -> Option<&mut Vec<f64>> {
        match &mut self.form {
            Form::EvenAsph { asph }
            | Form::Toroidal { asph, .. }
            | Form::ZernikeStd { asph, .. }
            | Form::ZernikeFringe { asph, .. }
            | Form::GridSag { asph, .. }
            | Form::Fresnel { asph }
            | Form::CylFresnel { asph, .. }
            | Form::Binary2 { asph, .. }
            | Form::OddAsph { asph } => Some(asph),
            _ => None,
        }
    }

    /// 面を z について裏返す（矢高 z(x, y) → −z(x, y)）: 曲率・トロイダル面の回転の曲率・非球面の係数・Zernike の項・
    /// 格子・Tilted の傾き・Cylinder Fresnel の屈折の面の符号を変える。Irregular は傾きと Zs・Za・Zc の符号を変え、
    /// 偏心と項の角度はそのまま（P = d + Rx(tx) Ry(ty) Q を z について映すと Rx・Ry の角度の符号が変わる）。
    /// 鏡の後の座標で面を書くときと、面の並びの復路に使う。面の後の媒質も裏返す（GRIN の分布、複屈折材の結晶軸の
    /// z 成分）。Binary 2 の位相・理想レンズ・Jones の行列は面内の量なので変えない。利用者定義の面はモデルの
    /// [`crate::usersurf::SurfaceModel::flip`] で裏返し、対応しないモデルなら誤り（面は変えない）
    pub fn flip_z(&mut self) -> crate::Result<()> {
        if let Form::User { user: Some(u) } = &self.form {
            let f = u.flipped(crate::usersurf::Flip::Z)?;
            self.form = Form::User { user: Some(f) };
        }
        self.c = -self.c;
        if let Some(crate::surface::Bire::In { axis, .. }) = &mut self.bire {
            axis[2] = -axis[2];
        }
        match &mut self.form {
            Form::Gradium { grin } | Form::Gradient9 { grin } => {
                if let Some(g) = grin {
                    Arc::make_mut(g).flip_z();
                }
            }
            Form::Toroidal { cx, asph } => {
                *cx = -*cx;
                asph.iter_mut().for_each(|a| *a = -*a);
            }
            Form::ZernikeStd { asph, zern } | Form::ZernikeFringe { asph, zern } => {
                asph.iter_mut().for_each(|a| *a = -*a);
                if let Some(z) = zern {
                    Arc::make_mut(z).negate();
                }
            }
            Form::GridSag {
                asph, grid, zern, ..
            } => {
                asph.iter_mut().for_each(|a| *a = -*a);
                if let Some(g) = grid {
                    Arc::make_mut(g).negate();
                }
                if let Some(z) = zern {
                    Arc::make_mut(z).negate();
                }
            }
            Form::CylFresnel { asph, cylf } => {
                asph.iter_mut().for_each(|a| *a = -*a);
                if let Some(f) = cylf {
                    let f = Arc::make_mut(f);
                    f.c = -f.c;
                    f.coef.iter_mut().for_each(|a| *a = -*a);
                }
            }
            Form::TiltSurf { tan } => tan.iter_mut().for_each(|a| *a = -*a),
            Form::UserShape { shape: Some(u) } => u.flip_z = !u.flip_z,
            Form::Irregular { p } => {
                for i in 2..7 {
                    p[i] = -p[i];
                }
            }
            _ => {
                if let Some(a) = self.asph_mut() {
                    a.iter_mut().for_each(|a| *a = -*a);
                }
            }
        }
        Ok(())
    }

    /// 面を x について裏返す（矢高 z(x, y) → z(−x, y)）: Zernike の項・格子・Tilted の x の傾きを映し、Irregular は
    /// 偏心 x・傾き y・項の角度の符号を変える（Rx はそのまま、Ry・Rz は角度の符号が変わる）。開口はそのまま。
    /// [`Surface::flip_z`] と合わせて y 軸のまわりに 180° 回した向き（玉の反転）になる。回転対称な形と、x について
    /// 対称な形（トロイダル面・Cylinder Fresnel）は何も変わらない。GRIN の分布の軸・形の傾き、複屈折材の結晶軸の
    /// x 成分も映す。利用者定義の面はモデルの [`crate::usersurf::SurfaceModel::flip`] で映し、対応しなければ誤り
    pub fn mirror_x(&mut self) -> crate::Result<()> {
        if let Form::User { user: Some(u) } = &self.form {
            let f = u.flipped(crate::usersurf::Flip::X)?;
            self.form = Form::User { user: Some(f) };
        }
        if let Some(crate::surface::Bire::In { axis, .. }) = &mut self.bire {
            axis[0] = -axis[0];
        }
        match &mut self.form {
            Form::Gradium { grin } | Form::Gradient9 { grin } => {
                if let Some(g) = grin {
                    Arc::make_mut(g).mirror_x();
                }
            }
            Form::ZernikeStd { zern, .. } | Form::ZernikeFringe { zern, .. } => {
                if let Some(z) = zern {
                    Arc::make_mut(z).mirror_x();
                }
            }
            Form::GridSag { grid, zern, .. } => {
                if let Some(g) = grid {
                    Arc::make_mut(g).mirror_x();
                }
                if let Some(z) = zern {
                    Arc::make_mut(z).mirror_x();
                }
            }
            Form::TiltSurf { tan } => tan[0] = -tan[0],
            Form::UserShape { shape: Some(u) } => u.mirror_x = !u.mirror_x,
            Form::Irregular { p } => {
                p[0] = -p[0];
                p[3] = -p[3];
                p[7] = -p[7];
            }
            _ => {}
        }
        Ok(())
    }

    /// 面タイプのパラメータ i（.zmx の PARM の番号）の値。この面タイプで値を持たない番号は None
    pub fn parm(&self, i: u8) -> Option<f64> {
        let d = self;
        let i = i as usize;
        let at = |v: &[f64], j: usize| -> Option<f64> {
            (j < 8).then(|| v.get(j).copied().unwrap_or(0.0))
        };
        match d.typ() {
            SurfType::Toroidal if i == 1 => Some(if d.cx() == 0.0 { 0.0 } else { 1.0 / d.cx() }),
            t if asph_parm_base(t).is_some_and(|b| (b..=8).contains(&i)) => {
                at(d.asph(), i - asph_parm_base(t).unwrap_or(1))
            }
            SurfType::CoordBrk if (1..=5).contains(&i) => Some(d.cb()[i - 1]),
            SurfType::CoordBrk if i == 6 => Some(d.cb_order() as f64),
            SurfType::Paraxial if i == 1 => Some(d.focal()),
            SurfType::Paraxial if i == 2 => Some(d.opd_mode() as f64),
            SurfType::Jones if (1..=8).contains(&i) => Some(d.jones()[i - 1]),
            SurfType::Gradium | SurfType::Gradient9 => {
                use crate::grin::GrinKind;
                let g = d.grin()?;
                match (&g.kind, i) {
                    (_, 1) => Some(g.dt),
                    (GrinKind::Gradium { prof, .. }, 0) => Some(prof.zmax),
                    (GrinKind::Gradium { dz, .. }, 2) => Some(*dz),
                    (GrinKind::Gradium { .. }, 3) => g.ref_index(),
                    (GrinKind::Gradium { dec, .. }, 4..=7) => Some(dec[i - 4]),
                    (GrinKind::Gradium { cap, .. }, 8) => Some(*cap as f64),
                    (GrinKind::Grad9 { tan, .. }, 2 | 3) => Some(tan[i - 2]),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// 面タイプのパラメータ i（.zmx の PARM の番号）を書く。値を持たない番号なら false（.zmx の読込・ピックアップ・
    /// パラメータが使う、番号と値の対応はここと [`Surface::parm`] の 1 か所）
    pub fn set_parm(&mut self, i: u8, v: f64) -> bool {
        let d = self;
        let i = i as usize;
        let t = d.typ();
        if let Some(b) = asph_parm_base(t).filter(|b| (*b..=8).contains(&i)) {
            if let Some(a) = d.asph_mut().filter(|a| a.len() < 9 - b) {
                a.resize(9 - b, 0.0);
            }
            return d.set_asph_at(i - b, v);
        }
        match (&mut d.form, i) {
            (Form::Toroidal { cx, .. }, 1) => *cx = if v == 0.0 { 0.0 } else { 1.0 / v },
            (Form::CoordBrk { cb, .. }, 1..=5) => cb[i - 1] = v,
            (Form::CoordBrk { order, .. }, 6) => *order = v as u8,
            (Form::Paraxial { focal, .. }, 1) => *focal = v,
            (Form::Paraxial { opd_mode, .. }, 2) => *opd_mode = v as i32,
            (Form::Jones { jones }, 1..=8) => {
                jones[i - 1] = v;
                d.retarder = None;
            }
            // GRIN: Δt と Gradient 9 の形の傾きだけ（Δz などは頂点の屈折率 n も変わるので未対応）
            (Form::Gradium { grin } | Form::Gradient9 { grin }, _) => {
                use crate::grin::GrinKind;
                let Some(g) = grin.as_mut() else {
                    return false;
                };
                let g = std::sync::Arc::make_mut(g);
                match (&mut g.kind, i) {
                    (_, 1) => g.dt = v,
                    (GrinKind::Grad9 { tan, .. }, 2 | 3) => tan[i - 2] = v,
                    _ => return false,
                }
            }
            _ => return false,
        }
        true
    }

    /// Tilted・Irregular の決まった数の値の置き場
    pub fn asph_fixed_mut(&mut self) -> Option<&mut [f64]> {
        match &mut self.form {
            Form::TiltSurf { tan } => Some(tan),
            Form::Irregular { p } => Some(p),
            _ => None,
        }
    }

    /// 非球面の係数の並びを v にする。Tilted・Irregular は先頭から決まった数だけ取り（足りなければ 0）、
    /// 係数を持たない種類は何もしない。v のうち置けなかった値に 0 でないものがあれば false
    pub fn set_asph(&mut self, v: Vec<f64>) -> bool {
        if let Some(a) = self.asph_mut() {
            *a = v;
            return true;
        }
        let fixed = self.asph_fixed_mut();
        let n = fixed.as_ref().map_or(0, |f| f.len());
        if let Some(f) = fixed {
            for (i, x) in f.iter_mut().enumerate() {
                *x = v.get(i).copied().unwrap_or(0.0);
            }
        }
        v.iter().skip(n).all(|&x| x == 0.0)
    }

    /// 非球面の係数 i を x にする（長さを変えられる種類は足りなければ 0 で伸ばす）。置けなければ false
    pub fn set_asph_at(&mut self, i: usize, x: f64) -> bool {
        if let Some(a) = self.asph_mut() {
            if a.len() <= i {
                a.resize(i + 1, 0.0);
            }
            a[i] = x;
            return true;
        }
        match self.asph_fixed_mut().and_then(|f| f.get_mut(i)) {
            Some(v) => {
                *v = x;
                true
            }
            None => false,
        }
    }
}

/// serde での [`Form`] の平たい形（面の表の中の `typ` と種類ごとの値の欄）
#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
struct FormFlat {
    typ: SurfType,
    asph: Vec<f64>,
    cx: f64,
    zern: Option<Arc<ZernSag>>,
    grid: Option<Arc<GridSag>>,
    grid_file: String,
    bin2: Option<Arc<Binary2>>,
    cylf: Option<Arc<CylFresnel>>,
    grin: Option<Arc<Grin>>,
    cb: [f64; 5],
    cb_order: u8,
    focal: f64,
    opd_mode: i32,
    jones: [f64; 8],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<UserSurf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shape: Option<UserShape>,
}

impl From<Form> for FormFlat {
    fn from(f: Form) -> FormFlat {
        let mut s = Surface {
            form: f,
            ..Default::default()
        };
        FormFlat {
            typ: s.typ(),
            asph: s.asph().to_vec(),
            cx: s.cx(),
            zern: s.zern_mut().and_then(|z| z.take()),
            grid: s.grid_mut().and_then(|g| g.take()),
            grid_file: s.grid_file().to_string(),
            bin2: s.bin2_mut().and_then(|b| b.take()),
            cylf: s.cylf_mut().and_then(|c| c.take()),
            grin: s.grin_mut().and_then(|g| g.take()),
            cb: s.cb(),
            cb_order: s.cb_order(),
            focal: s.focal(),
            opd_mode: s.opd_mode(),
            jones: s.jones(),
            user: s.user_mut().and_then(|u| u.take()),
            shape: s.user_shape_mut().and_then(|u| u.take()),
        }
    }
}

impl TryFrom<FormFlat> for Form {
    type Error = String;

    /// 種類の枝を作り、その種類の値を取る。種類に無い欄に既定でない値があれば誤り
    fn try_from(v: FormFlat) -> Result<Form, String> {
        let typ = v.typ;
        let mut s = Surface {
            form: Form::of(typ),
            ..Default::default()
        };
        let mut unused = Vec::new();
        if !s.set_asph(v.asph) {
            unused.push("asph");
        }
        macro_rules! put {
            ($name:literal, $slot:expr, $val:expr, $default:expr) => {
                match $slot {
                    Some(x) => *x = $val,
                    None if $val != $default => unused.push($name),
                    None => {}
                }
            };
        }
        put!("cx", s.cx_mut(), v.cx, 0.0);
        put!("zern", s.zern_mut(), v.zern, None);
        put!("grid", s.grid_mut(), v.grid, None);
        put!("bin2", s.bin2_mut(), v.bin2, None);
        put!("cylf", s.cylf_mut(), v.cylf, None);
        put!("grin", s.grin_mut(), v.grin, None);
        put!("jones", s.jones_mut(), v.jones, JONES_IDENTITY);
        put!("user", s.user_mut(), v.user, None);
        put!("shape", s.user_shape_mut(), v.shape, None);
        match &mut s.form {
            Form::GridSag { grid_file, .. } => *grid_file = v.grid_file,
            _ if !v.grid_file.is_empty() => unused.push("grid_file"),
            _ => {}
        }
        match s.cb_mut() {
            Some((cb, order)) => (*cb, *order) = (v.cb, v.cb_order),
            None => {
                if v.cb != [0.0; 5] {
                    unused.push("cb");
                }
                if v.cb_order != 0 {
                    unused.push("cb_order");
                }
            }
        }
        match s.paraxial_mut() {
            Some((focal, mode)) => (*focal, *mode) = (v.focal, v.opd_mode),
            None => {
                if v.focal != 0.0 {
                    unused.push("focal");
                }
                if v.opd_mode != 0 {
                    unused.push("opd_mode");
                }
            }
        }
        if !unused.is_empty() {
            let names = unused.join(", ");
            return Err(format!(
                "{names} not used by a {typ:?} surface (remove the values or change typ)"
            ));
        }
        Ok(s.form)
    }
}

impl schemars::JsonSchema for Form {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Form".into()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        FormFlat::json_schema(g)
    }
}

/// 面タイプの .zmx の PARM の番号のうち、係数の並び `Surface::asph` の最初に当たる番号（asph[k] は PARM base + k、
/// PARM 8 まで）。偶数次非球面・フレネル・バイナリ 2・自由曲面（奇数次非球面・傾いた面・不規則面・Zernike・Grid Sag）は 1、
/// トロイダルは 2（PARM 1 は回転半径）。係数を持たない面タイプは None（面の量の対応はここ 1 か所）
pub fn asph_parm_base(t: SurfType) -> Option<usize> {
    match t {
        SurfType::EvenAsph | SurfType::Fresnel | SurfType::CylFresnel | SurfType::Binary2 => {
            Some(1)
        }
        SurfType::Toroidal => Some(2),
        t if crate::freeform::is_freeform(t) => Some(1),
        _ => None,
    }
}
