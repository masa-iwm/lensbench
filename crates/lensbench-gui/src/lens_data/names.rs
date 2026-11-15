//! 面タイプの名前（lens_data.rs から分けた）

use crate::i18n::n_;

/// 面タイプの名前と説明（種類の列の候補）
pub const TYPE_NAMES: &[(&str, &str)] = &[
    ("STANDARD", n_("Standard")),
    ("EVENASPH", n_("Even asphere")),
    ("COORDBRK", n_("Coordinate break")),
    ("PARAXIAL", n_("Paraxial lens")),
    ("TOROIDAL", n_("Toroidal")),
    ("JONESMAT", n_("Jones matrix")),
    ("SZERNSAG", "Zernike (Standard)"),
    ("FZERNSAG", "Zernike (Fringe)"),
    ("GRID_SAG", "Grid Sag"),
    ("FRESNELS", n_("Fresnel")),
    ("CFRESNEL", n_("Cylinder Fresnel")),
    ("BINARY_2", "Binary 2"),
    ("GRINSUR8", "GRIN (Gradium)"),
    ("GRINSUR9", "GRIN (Gradient 9)"),
    ("USER", n_("User defined")),
];
