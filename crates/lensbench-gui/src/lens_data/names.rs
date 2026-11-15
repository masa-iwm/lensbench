//! 面タイプの名前（lens_data.rs から分けた）

use crate::i18n::n_;

/// 面タイプの名前と説明（種類の列の候補）
pub const TYPE_NAMES: &[(&str, &str)] = &[
    ("STANDARD", n_("標準")),
    ("EVENASPH", n_("偶数次非球面")),
    ("COORDBRK", n_("座標ブレーク")),
    ("PARAXIAL", n_("近軸レンズ")),
    ("TOROIDAL", n_("トロイダル")),
    ("JONESMAT", n_("Jones 行列")),
    ("SZERNSAG", "Zernike（Standard）"),
    ("FZERNSAG", "Zernike（Fringe）"),
    ("GRID_SAG", "Grid Sag"),
    ("FRESNELS", n_("フレネル")),
    ("CFRESNEL", n_("円柱フレネル")),
    ("BINARY_2", "Binary 2"),
    ("GRINSUR8", "GRIN（Gradium）"),
    ("GRINSUR9", "GRIN（Gradient 9）"),
    ("USER", n_("利用者定義")),
];
