//! レイアウトの描き方の設定（CLI の `layout` と serve の `layout`・`layout_export` で共通）
//!
//! 光線の選び方（[`RaySel`]）と書き出しの形（[`Draw`]）の既定値・範囲はここだけに書く。CLI は旗から、serve は params
//! の JSON からこの型を作り、同じ関数で場面と書き出しの設定にする。

use lensbench_core::{Error, Result, System};
use lensbench_export::d2::{self, ColorBy, Plane};
use lensbench_export::mesh;
use lensbench_export::scene::{self, Fan};

/// 光線の本数の範囲
pub const RAYS: (usize, usize) = (1, 200);
/// PNG の幅の範囲 [px]
pub const WIDTH: (u32, u32) = (100, 20000);
/// 周の分割の範囲
pub const SEGMENTS: (usize, usize) = (4, 1024);
/// 半径の分割の範囲
pub const RINGS: (usize, usize) = (1, 256);

/// 見る面: 断面図（yz・xz）か 3D
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum View {
    #[default]
    #[serde(rename = "yz")]
    YZ,
    #[serde(rename = "xz")]
    XZ,
    #[serde(rename = "3d")]
    D3,
}

impl std::str::FromStr for View {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<View, String> {
        match s.to_ascii_lowercase().as_str() {
            "3d" => Ok(View::D3),
            x => match x.parse::<Plane>()? {
                Plane::YZ => Ok(View::YZ),
                Plane::XZ => Ok(View::XZ),
            },
        }
    }
}

impl View {
    /// 断面図の面（3D なら yz）
    pub fn plane(self) -> Plane {
        match self {
            View::XZ => Plane::XZ,
            _ => Plane::YZ,
        }
    }
}

/// 視野・波長の選び方: 全部か、番号（1 始まり）の並び
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pick {
    All,
    List(Vec<usize>),
}

impl std::str::FromStr for Pick {
    type Err = String;
    /// "all" か、カンマ区切りの番号
    fn from_str(s: &str) -> std::result::Result<Pick, String> {
        if s.trim().eq_ignore_ascii_case("all") {
            return Ok(Pick::All);
        }
        s.split(',')
            .map(|t| t.trim().parse().map_err(|_| format!("number {t}")))
            .collect::<std::result::Result<_, _>>()
            .map(Pick::List)
    }
}

/// 描く光線の選び方
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RaySel {
    /// 視野（None で全視野）
    pub fields: Option<Pick>,
    /// 波長（None で主波長だけ）
    pub waves: Option<Pick>,
    /// 光線の並び（None で 3D は円周、xz 面は x の扇、ほかは y の扇）
    pub fan: Option<Fan>,
    /// 光線の本数（None で円周は 12、ほかは 7）
    pub rays: Option<usize>,
    pub first: Option<usize>,
    pub last: Option<usize>,
    /// 光路に沿って展開する（2D だけ）
    pub unfold: bool,
}

fn index0(v: &Option<Pick>, n: usize, what: &str) -> Result<Option<Vec<usize>>> {
    v.as_ref()
        .map(|v| match v {
            Pick::All => Ok((0..n).collect()),
            Pick::List(v) => v
                .iter()
                .map(|&k| {
                    if k >= 1 && k <= n {
                        Ok(k - 1)
                    } else {
                        Err(Error::Invalid(format!("no {what} {k} (1 to {n})")))
                    }
                })
                .collect(),
        })
        .transpose()
}

impl RaySel {
    /// 系 s を view で描く場面の設定
    pub fn options(&self, s: &System, view: View) -> Result<scene::Options> {
        if self.unfold && view == View::D3 {
            return Err(Error::Invalid(
                "unfold is only for 2D (plane yz or xz)".into(),
            ));
        }
        let fan = self.fan.unwrap_or(match view {
            View::D3 => Fan::Ring,
            View::XZ => Fan::X,
            View::YZ => Fan::Y,
        });
        let n0 = if fan == Fan::Ring { 12 } else { 7 };
        Ok(scene::Options {
            fields: index0(&self.fields, s.fields.len(), "field")?,
            waves: index0(&self.waves, s.wavelengths.len(), "wavelength")?,
            fan,
            rays: self.rays.unwrap_or(n0).clamp(RAYS.0, RAYS.1),
            first: self.first,
            last: self.last,
            unfold: self.unfold,
        })
    }
}

/// 書き出しの形
#[derive(Clone)]
pub struct Draw {
    /// 2D の面（3D の書き出しでは使わない）
    pub plane: Plane,
    /// 光線の色分け（None で重ね描きなら構成ごと、ほかは視野ごと）
    pub color: Option<ColorBy>,
    /// PNG の幅（None で既定）
    pub width: Option<u32>,
    /// 周の分割（None で既定）
    pub segments: Option<usize>,
    /// 半径の分割（None で既定）
    pub rings: Option<usize>,
    /// 光線の管の半径 [mm]
    pub tube: Option<f64>,
    /// 機構部品を取った STEP の詳しい形（None で簡易外形）
    pub mech_detail: Option<lensbench_export::detail::Details>,
}

impl Default for Draw {
    fn default() -> Draw {
        Draw {
            plane: Plane::YZ,
            color: None,
            width: None,
            segments: None,
            rings: None,
            tube: None,
            mech_detail: None,
        }
    }
}

impl Draw {
    /// 色分け（overlay は構成を重ね描きするか）
    pub fn color(&self, overlay: bool) -> ColorBy {
        self.color.unwrap_or(if overlay {
            ColorBy::Config
        } else {
            ColorBy::Field
        })
    }

    /// 2D・3D の書き出しの設定
    pub fn options(&self, overlay: bool) -> Result<(d2::Options2D, mesh::Options3D)> {
        if let Some(t) = self.tube {
            if !(t > 0.0 && t.is_finite()) {
                return Err(Error::Invalid("tube must be a positive number [mm]".into()));
            }
        }
        let (a, b) = (d2::Options2D::default(), mesh::Options3D::default());
        let color = self.color(overlay);
        let segments = self
            .segments
            .unwrap_or(a.segments)
            .clamp(SEGMENTS.0, SEGMENTS.1);
        Ok((
            d2::Options2D {
                plane: self.plane,
                color,
                width: self.width.unwrap_or(a.width).clamp(WIDTH.0, WIDTH.1),
                segments,
            },
            mesh::Options3D {
                segments,
                rings: self.rings.unwrap_or(b.rings).clamp(RINGS.0, RINGS.1),
                tube: self.tube,
                color,
                mech_detail: self.mech_detail.clone(),
            },
        ))
    }
}

/// 構成ごとのずらし dx,dy,dz（3 つでなければ誤り）
pub fn offset(v: &[f64]) -> Result<[f64; 3]> {
    match v {
        [a, b, c] => Ok([*a, *b, *c]),
        _ => Err(Error::Invalid("offset takes 3 values dx,dy,dz".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_view() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let opt = lensbench_core::zmx::LoadOptions {
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s =
            lensbench_core::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap();
        let r = RaySel::default();
        assert_eq!(r.options(&s, View::YZ).unwrap().fan, Fan::Y);
        assert_eq!(r.options(&s, View::XZ).unwrap().fan, Fan::X);
        let o = r.options(&s, View::D3).unwrap();
        assert_eq!((o.fan, o.rays), (Fan::Ring, 12));
        let r = RaySel {
            fields: Some(Pick::List(vec![9])),
            ..Default::default()
        };
        assert!(r.options(&s, View::YZ).is_err());
        let r = RaySel {
            unfold: true,
            rays: Some(1000),
            ..Default::default()
        };
        assert!(r.options(&s, View::D3).is_err());
        assert_eq!(r.options(&s, View::YZ).unwrap().rays, RAYS.1);
        let d = Draw::default();
        assert_eq!(d.color(true), ColorBy::Config);
        let (a, b) = d.options(false).unwrap();
        assert_eq!((a.width, b.rings), (1600, 16));
        assert!(offset(&[1.0, 2.0]).is_err());
    }
}
