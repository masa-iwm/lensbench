//! 面の種類と、種類ごとの値（[`Form`]）。
//!
//! 種類にしか意味の無い値（座標ブレークのずらし・理想レンズの焦点距離・Jones 行列・Zernike の項・格子・
//! 屈折率分布・利用者定義の面など）は種類の枝の中に持つ。曲率・円錐係数・非球面の係数（`asph`）のように多くの
//! 種類が共通に使う値は [`crate::Surface`] に残し、種類ごとの意味は形の見方 [`crate::Surface::shape`] で分ける。
//! 種類だけを比べるときは [`crate::Surface::typ`]（[`SurfType`]）

use crate::freeform::{GridSag, ZernSag};
use crate::grin::Grin;
use crate::surface::{Binary2, CylFresnel, SurfType, Surface};
use crate::usersurf::UserSurf;
use std::sync::Arc;

/// 単位行列の Jones 行列（実部・虚部の順に Jxx Jxy Jyx Jyy）
pub const JONES_IDENTITY: [f64; 8] = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// 面の種類と、種類ごとの値。組み込みの種類を足しても外のクレートが壊れないよう `non_exhaustive`
///
/// serde では面の表の中に平たく（`typ` と、どの種類でも全部の値の欄。種類に無い値は既定）書く
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(into = "FormFlat", from = "FormFlat"))]
#[non_exhaustive]
pub enum Form {
    #[default]
    Standard,
    EvenAsph,
    /// 座標ブレーク: [偏心 x, 偏心 y, 傾き x°, 傾き y°, 傾き z°] と順序（0 は偏心してから傾け、1 は逆）
    CoordBrk {
        cb: [f64; 5],
        order: u8,
    },
    /// Jones 面: 行列（実部・虚部の順に Jxx Jxy Jyx Jyy）
    Jones {
        jones: [f64; 8],
    },
    /// 理想レンズ: 焦点距離 [mm] と OPD の扱い（.zmx の PARM 2）
    Paraxial {
        focal: f64,
        opd_mode: i32,
    },
    /// トロイダル面: 回転の曲率（1 / 回転半径、0 は円柱面）
    Toroidal {
        cx: f64,
    },
    /// Zernike Standard Sag の項
    ZernikeStd {
        zern: Option<Arc<ZernSag>>,
    },
    /// Zernike Fringe Sag の項
    ZernikeFringe {
        zern: Option<Arc<ZernSag>>,
    },
    /// Grid Sag の格子と、その .DAT のファイル名（書き出し用）、足す Zernike（Standard）の項
    GridSag {
        grid: Option<Arc<GridSag>>,
        grid_file: String,
        zern: Option<Arc<ZernSag>>,
    },
    Fresnel,
    /// Cylinder Fresnel の屈折の面
    CylFresnel {
        cylf: Option<Arc<CylFresnel>>,
    },
    /// Binary 2 の位相
    Binary2 {
        bin2: Option<Arc<Binary2>>,
    },
    /// Gradium: 面の後ろの屈折率分布（`n` は頂点の屈折率）
    Gradium {
        grin: Option<Arc<Grin>>,
    },
    /// Gradient 9: 面の後ろの屈折率分布
    Gradient9 {
        grin: Option<Arc<Grin>>,
    },
    OddAsph,
    TiltSurf,
    Irregular,
    /// 利用者定義の面のモデルと設定
    User {
        user: Option<UserSurf>,
    },
}

impl Form {
    /// 種類 t の、値が既定の枝（Jones は単位行列）
    pub fn of(t: SurfType) -> Form {
        match t {
            SurfType::Standard => Form::Standard,
            SurfType::EvenAsph => Form::EvenAsph,
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
            SurfType::Toroidal => Form::Toroidal { cx: 0.0 },
            SurfType::ZernikeStd => Form::ZernikeStd { zern: None },
            SurfType::ZernikeFringe => Form::ZernikeFringe { zern: None },
            SurfType::GridSag => Form::GridSag {
                grid: None,
                grid_file: String::new(),
                zern: None,
            },
            SurfType::Fresnel => Form::Fresnel,
            SurfType::CylFresnel => Form::CylFresnel { cylf: None },
            SurfType::Binary2 => Form::Binary2 { bin2: None },
            SurfType::Gradium => Form::Gradium { grin: None },
            SurfType::Gradient9 => Form::Gradient9 { grin: None },
            SurfType::OddAsph => Form::OddAsph,
            SurfType::TiltSurf => Form::TiltSurf,
            SurfType::Irregular => Form::Irregular,
            SurfType::User => Form::User { user: None },
        }
    }

    /// 種類（値を除く）
    pub fn typ(&self) -> SurfType {
        match self {
            Form::Standard => SurfType::Standard,
            Form::EvenAsph => SurfType::EvenAsph,
            Form::CoordBrk { .. } => SurfType::CoordBrk,
            Form::Jones { .. } => SurfType::Jones,
            Form::Paraxial { .. } => SurfType::Paraxial,
            Form::Toroidal { .. } => SurfType::Toroidal,
            Form::ZernikeStd { .. } => SurfType::ZernikeStd,
            Form::ZernikeFringe { .. } => SurfType::ZernikeFringe,
            Form::GridSag { .. } => SurfType::GridSag,
            Form::Fresnel => SurfType::Fresnel,
            Form::CylFresnel { .. } => SurfType::CylFresnel,
            Form::Binary2 { .. } => SurfType::Binary2,
            Form::Gradium { .. } => SurfType::Gradium,
            Form::Gradient9 { .. } => SurfType::Gradient9,
            Form::OddAsph => SurfType::OddAsph,
            Form::TiltSurf => SurfType::TiltSurf,
            Form::Irregular => SurfType::Irregular,
            Form::User { .. } => SurfType::User,
        }
    }
}

impl Surface {
    /// 面の種類
    pub fn typ(&self) -> SurfType {
        self.form.typ()
    }

    /// 種類を t に変える。同じ種類なら何もしない。持ち越せる値（Zernike の項・屈折率分布）は持ち越し、
    /// ほかの種類ごとの値は既定に戻す
    pub fn set_type(&mut self, t: SurfType) {
        if self.typ() == t {
            return;
        }
        let zern = match &self.form {
            Form::ZernikeStd { zern }
            | Form::ZernikeFringe { zern }
            | Form::GridSag { zern, .. } => zern.clone(),
            _ => None,
        };
        let grin = match &self.form {
            Form::Gradium { grin } | Form::Gradient9 { grin } => grin.clone(),
            _ => None,
        };
        self.form = Form::of(t);
        match &mut self.form {
            Form::ZernikeStd { zern: z }
            | Form::ZernikeFringe { zern: z }
            | Form::GridSag { zern: z, .. } => *z = zern,
            Form::Gradium { grin: g } | Form::Gradient9 { grin: g } => *g = grin,
            _ => {}
        }
    }

    /// トロイダル面の回転の曲率（ほかの種類は 0）
    pub fn cx(&self) -> f64 {
        match self.form {
            Form::Toroidal { cx } => cx,
            _ => 0.0,
        }
    }

    /// Zernike の項（Zernike Sag・Grid Sag）
    pub fn zern(&self) -> Option<&ZernSag> {
        match &self.form {
            Form::ZernikeStd { zern }
            | Form::ZernikeFringe { zern }
            | Form::GridSag { zern, .. } => zern.as_deref(),
            _ => None,
        }
    }

    /// Zernike の項の置き場（Zernike Sag・Grid Sag の面だけ）
    pub fn zern_mut(&mut self) -> Option<&mut Option<Arc<ZernSag>>> {
        match &mut self.form {
            Form::ZernikeStd { zern }
            | Form::ZernikeFringe { zern }
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
            Form::Binary2 { bin2 } => bin2.as_deref(),
            _ => None,
        }
    }

    /// Cylinder Fresnel の屈折の面
    pub fn cylf(&self) -> Option<&CylFresnel> {
        match &self.form {
            Form::CylFresnel { cylf } => cylf.as_deref(),
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
            Form::Toroidal { cx } => Some(cx),
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
            Form::Binary2 { bin2 } => Some(bin2),
            _ => None,
        }
    }

    /// Cylinder Fresnel の屈折の面の置き場（Cylinder Fresnel だけ）
    pub fn cylf_mut(&mut self) -> Option<&mut Option<Arc<CylFresnel>>> {
        match &mut self.form {
            Form::CylFresnel { cylf } => Some(cylf),
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

    /// 屈折率分布の置き場（Gradium・Gradient 9 だけ）
    pub fn grin_mut(&mut self) -> Option<&mut Option<Arc<Grin>>> {
        match &mut self.form {
            Form::Gradium { grin } | Form::Gradient9 { grin } => Some(grin),
            _ => None,
        }
    }
}

/// serde での [`Form`] の平たい形（面の表の中の `typ` と種類ごとの値の欄）
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct FormFlat {
    typ: SurfType,
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
}

#[cfg(feature = "serde")]
impl From<Form> for FormFlat {
    fn from(f: Form) -> FormFlat {
        let mut s = Surface {
            form: f,
            ..Default::default()
        };
        FormFlat {
            typ: s.typ(),
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
        }
    }
}

#[cfg(feature = "serde")]
impl From<FormFlat> for Form {
    /// 種類の枝を作り、その種類の値だけを取る（ほかの欄は捨てる）
    fn from(v: FormFlat) -> Form {
        match v.typ {
            SurfType::CoordBrk => Form::CoordBrk {
                cb: v.cb,
                order: v.cb_order,
            },
            SurfType::Jones => Form::Jones { jones: v.jones },
            SurfType::Paraxial => Form::Paraxial {
                focal: v.focal,
                opd_mode: v.opd_mode,
            },
            SurfType::Toroidal => Form::Toroidal { cx: v.cx },
            SurfType::ZernikeStd => Form::ZernikeStd { zern: v.zern },
            SurfType::ZernikeFringe => Form::ZernikeFringe { zern: v.zern },
            SurfType::GridSag => Form::GridSag {
                grid: v.grid,
                grid_file: v.grid_file,
                zern: v.zern,
            },
            SurfType::CylFresnel => Form::CylFresnel { cylf: v.cylf },
            SurfType::Binary2 => Form::Binary2 { bin2: v.bin2 },
            SurfType::Gradium => Form::Gradium { grin: v.grin },
            SurfType::Gradient9 => Form::Gradient9 { grin: v.grin },
            SurfType::User => Form::User { user: v.user },
            t => Form::of(t),
        }
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Form {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Form".into()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        FormFlat::json_schema(g)
    }
}
