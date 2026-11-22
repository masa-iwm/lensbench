//! 面タイプの名前（lens_data.rs から分けた）

use super::{type_matches, Col};
use crate::i18n::{n_, tr};

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

/// 入力できるドロップダウンの候補（値、説明）。種類は面タイプ、硝材はガラスカタログ（と鏡）、膜は系で使える膜
/// （names に硝材・膜の候補）。filter なら text で絞る（完全一致を先に、次に前方一致、部分一致）
pub(super) fn list_choices(
    col: Col,
    names: &[GlassChoice],
    used: &[String],
    text: &str,
    filter: bool,
) -> Vec<(String, String)> {
    let narrow = |all: Vec<(String, String)>| -> Vec<(String, String)> {
        if !filter {
            return all;
        }
        let q = text.trim().to_uppercase();
        let (mut exact, mut head, mut rest) = (Vec::new(), Vec::new(), Vec::new());
        for (n, d) in all {
            let u = n.to_uppercase();
            match crate::widgets::match_rank(&u, &d.to_uppercase(), &q) {
                Some(true) if u == q => exact.push((n, d)),
                Some(true) => head.push((n, d)),
                Some(false) => rest.push((n, d)),
                None => {}
            }
        }
        exact.extend(head);
        exact.extend(rest);
        exact
    };
    // この系で使っている名前のうち、候補に無いもの
    let extra = |skip: &dyn Fn(&str) -> bool| -> Vec<(String, String)> {
        used.iter()
            .filter(|u| !skip(u))
            .filter(|u| !names.iter().any(|g| g.name.eq_ignore_ascii_case(u)))
            .map(|u| (u.clone(), tr("used in this system").to_string()))
            .collect()
    };
    match col {
        Col::Type => {
            let names: Vec<&str> = if filter {
                type_matches(text)
            } else {
                TYPE_NAMES.iter().map(|x| x.0).collect()
            };
            names
                .into_iter()
                .map(|t| {
                    let d = TYPE_NAMES.iter().find(|x| x.0 == t).map_or("", |x| tr(x.1));
                    (t.to_string(), d.to_string())
                })
                .collect()
        }
        Col::Material => {
            // この系で使っている硝材（カタログに無いものも）を先に、次に鏡、カタログ
            let all = extra(&|u| u.eq_ignore_ascii_case("MIRROR"))
                .into_iter()
                .chain(std::iter::once((
                    "MIRROR".to_string(),
                    tr("Mirror").to_string(),
                )))
                .chain(names.iter().map(|g| (g.name.clone(), g.desc.clone())))
                .collect();
            narrow(all)
        }
        Col::Coating => {
            // 系で使える膜（由来つき）を先に、次にこの系で使っているがほかに無い名前（膜のファイル・モデルの名前など）
            let all = names
                .iter()
                .map(|g| (g.name.clone(), g.desc.clone()))
                .chain(extra(&|_| false))
                .collect();
            narrow(all)
        }
        _ => Vec::new(),
    }
}

/// 硝材の候補（GUI のガラスカタログの名前と、カタログ・nd・νd の説明）
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GlassChoice {
    pub name: String,
    pub desc: String,
}
