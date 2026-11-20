//! 面タイプの名前（lens_data.rs から分けた）

use crate::i18n::n_;

/// 面タイプの名前と説明（種類の列の候補）
pub(crate) const TYPE_NAMES: &[(&str, &str)] = &[
    ("STANDARD", n_("Standard")),
    ("EVENASPH", n_("Even asphere")),
    ("COORDBRK", n_("Coordinate break")),
    ("PARAXIAL", n_("Paraxial lens")),
    ("TOROIDAL", n_("Toroidal")),
    ("JONESMAT", n_("Jones matrix")),
    ("SZERNSAG", "Zernike (Standard)"),
    ("FZERNSAG", "Zernike (Fringe)"),
    ("GRID_SAG", "Grid Sag"),
    ("ODDASPHE", n_("Odd asphere")),
    ("TILTSURF", n_("Tilted")),
    ("IRREGULA", n_("Irregular")),
    ("FRESNELS", n_("Fresnel")),
    ("CFRESNEL", n_("Cylinder Fresnel")),
    ("BINARY_2", "Binary 2"),
    ("GRINSUR8", "GRIN (Gradium)"),
    ("GRINSUR9", "GRIN (Gradient 9)"),
    ("USER", n_("User defined")),
];

/// Odd Asphere のパラメータ 1〜8（β_1..β_8）
pub(super) const ODD_LABELS: [&str; 8] = [
    n_("r¹ coeff"),
    n_("r² coeff"),
    n_("r³ coeff"),
    n_("r⁴ coeff"),
    n_("r⁵ coeff"),
    n_("r⁶ coeff"),
    n_("r⁷ coeff"),
    n_("r⁸ coeff"),
];
/// Tilted のパラメータ 1・2（tan α・tan β）
pub(super) const TILT_LABELS: [&str; 2] = [n_("X tangent"), n_("Y tangent")];
/// Irregular のパラメータ 1〜8（傾きと角度は度）
pub(super) const IRR_LABELS: [&str; 8] = [
    n_("Decenter X"),
    n_("Decenter Y"),
    n_("Tilt X"),
    n_("Tilt Y"),
    n_("Z spherical"),
    n_("Z astigmatism"),
    n_("Z coma"),
    n_("Angle"),
];
