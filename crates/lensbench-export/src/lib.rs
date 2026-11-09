//! lensbench の書き出し: レイアウト（2D: SVG・PNG・DXF、3D: glTF・OBJ・STL・STEP）と追跡結果（CSV・JSON・npz）。
//!
//! レイアウトは [`scene::build`] で系から場面（玉の立体・鏡・面・光線）を作り、各形式の書き手に渡す。

// 添字で回す数値計算のループはそのままにする
#![allow(clippy::needless_range_loop)]

pub mod d2;
pub mod image;
pub mod mesh;
pub mod scene;
pub mod step;
pub mod table;

use std::path::Path;

/// 8 bit の色
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
    pub fn linear(self) -> [f32; 3] {
        // sRGB → 線形（glTF の色は線形）
        [self.0, self.1, self.2].map(|v| {
            let c = v as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        })
    }
}

/// 視野・波長の色（OpticStudio のレイアウトに近い並び: 青・緑・赤・…）
pub fn palette(i: usize) -> Rgb {
    const P: [Rgb; 12] = [
        Rgb(0, 0, 255),
        Rgb(0, 160, 0),
        Rgb(220, 0, 0),
        Rgb(230, 170, 0),
        Rgb(200, 0, 200),
        Rgb(0, 170, 190),
        Rgb(110, 60, 0),
        Rgb(110, 110, 110),
        Rgb(0, 0, 120),
        Rgb(0, 90, 0),
        Rgb(120, 0, 0),
        Rgb(255, 100, 0),
    ];
    P[i % P.len()]
}

/// レイアウトの書き出し形式（拡張子で決める）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutFormat {
    Svg,
    Png,
    Dxf,
    Gltf,
    Glb,
    Obj,
    Stl,
    Step,
}

impl LayoutFormat {
    /// 2D の形式か（svg・png・dxf）
    pub fn is_2d(self) -> bool {
        matches!(
            self,
            LayoutFormat::Svg | LayoutFormat::Png | LayoutFormat::Dxf
        )
    }

    pub fn from_path(p: &Path) -> Option<LayoutFormat> {
        let e = p.extension()?.to_str()?.to_ascii_lowercase();
        Some(match e.as_str() {
            "svg" => LayoutFormat::Svg,
            "png" => LayoutFormat::Png,
            "dxf" => LayoutFormat::Dxf,
            "gltf" => LayoutFormat::Gltf,
            "glb" => LayoutFormat::Glb,
            "obj" => LayoutFormat::Obj,
            "stl" => LayoutFormat::Stl,
            "step" | "stp" => LayoutFormat::Step,
            _ => return None,
        })
    }
}

/// 場面を書き出す（形式はファイルの拡張子で決める）
pub fn write_layout(
    path: &Path,
    sc: &scene::Scene,
    o2: &d2::Options2D,
    o3: &mesh::Options3D,
) -> Result<LayoutFormat, String> {
    let f = LayoutFormat::from_path(path).ok_or_else(|| {
        format!(
            "{}: 拡張子は svg・png・dxf・gltf・glb・obj・stl・step・stp のどれか",
            path.display()
        )
    })?;
    // 中身を作ってから書く（wasm ではファイルの置き場へ、lensbench_core::vfs）
    let mut w: Vec<u8> = Vec::new();
    let r = match f {
        LayoutFormat::Svg => d2::write_svg(&mut w, sc, o2),
        LayoutFormat::Png => d2::write_png(&mut w, sc, o2),
        LayoutFormat::Dxf => d2::write_dxf(&mut w, sc, o2),
        LayoutFormat::Gltf => mesh::write_gltf(&mut w, sc, o3, false),
        LayoutFormat::Glb => mesh::write_gltf(&mut w, sc, o3, true),
        LayoutFormat::Obj => mesh::write_obj(&mut w, sc, o3),
        LayoutFormat::Stl => mesh::write_stl(&mut w, sc, o3),
        LayoutFormat::Step => {
            step::write_step(&mut w, sc, &step::OptionsStep::default()).map(|_| ())
        }
    };
    r.and_then(|_| lensbench_core::vfs::write(path, w))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(f)
}
