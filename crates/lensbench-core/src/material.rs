//! 面の後ろの媒質（材料）。.zmx・系ファイルの名前の書き方（空・"AIR"・"MIRROR"・"-"・`nd,vd[,ΔPgF]`・ガラスの名前）は
//! 読むときに [`Material::parse`] でこの型にし、名前に戻すのは [`Material::name`]（serde も同じ文字列）。

use std::fmt;

/// 面の後ろの媒質
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Material {
    /// 空気（屈折率 1）
    #[default]
    Air,
    /// 名前で引く媒質（カタログのガラス・GRIN の分布の名前・複屈折材の常光の材料）
    Glass(String),
    /// モデルガラス（Abbe 数 vd ≠ 0。ΔPgF は無ければ 0）
    Model { nd: f64, vd: f64, dpgf: f64 },
    /// どの波長でも同じ屈折率の媒質（分散のないモデルガラス、屈折率だけ決めた媒質）
    Index(f64),
    /// 鏡（後ろの媒質は前の媒質のまま）
    Mirror,
    /// 媒質を持たない面（座標ブレーク。後ろの媒質は前の媒質のまま）
    Inherit,
}

impl Material {
    /// 名前から。空・"AIR" は空気、"MIRROR" は鏡、"-" は媒質なし、`nd,vd[,ΔPgF]` はモデルガラス（vd = 0 は屈折率だけ）、
    /// ほかはガラスの名前（前後の空白は除く。大文字小文字はガラスの名前では区別する）
    pub fn parse(s: &str) -> Material {
        let m = s.trim();
        if m.is_empty() || m.eq_ignore_ascii_case("AIR") {
            return Material::Air;
        }
        if m.eq_ignore_ascii_case("MIRROR") {
            return Material::Mirror;
        }
        if m == "-" {
            return Material::Inherit;
        }
        if let Some((nd, vd, dpgf)) = model_numbers(m) {
            return if vd == 0.0 {
                Material::Index(nd)
            } else {
                Material::Model { nd, vd, dpgf }
            };
        }
        Material::Glass(m.to_string())
    }

    /// 名前（[`Material::parse`] で同じ値に戻る。空気は空、屈折率だけの媒質は "nd,0"）
    pub fn name(&self) -> String {
        match self {
            Material::Air => String::new(),
            Material::Glass(g) => g.clone(),
            Material::Model { nd, vd, dpgf } if *dpgf == 0.0 => format!("{nd},{vd}"),
            Material::Model { nd, vd, dpgf } => format!("{nd},{vd},{dpgf}"),
            Material::Index(nd) => format!("{nd},0"),
            Material::Mirror => "MIRROR".into(),
            Material::Inherit => "-".into(),
        }
    }

    /// 膜の境界の鍵・表示に使う媒質の名前（空気は "AIR"、屈折率だけの媒質は小数 2 桁に丸めた "1.52,0.0"）
    pub fn label(&self) -> String {
        match self {
            Material::Air => "AIR".into(),
            Material::Index(nd) => format!("{nd:.2},0.0"),
            m => m.name(),
        }
    }

    pub fn is_air(&self) -> bool {
        matches!(self, Material::Air)
    }

    pub fn is_mirror(&self) -> bool {
        matches!(self, Material::Mirror)
    }

    /// 屈折率を持つ媒質（空気・ガラス・モデルガラス・屈折率だけ）。鏡と媒質なしは持たない
    pub fn is_medium(&self) -> bool {
        !matches!(self, Material::Mirror | Material::Inherit)
    }

    /// ガラスの名前（名前で引く媒質だけ）
    pub fn glass(&self) -> Option<&str> {
        match self {
            Material::Glass(g) => Some(g),
            _ => None,
        }
    }
}

impl fmt::Display for Material {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

/// 名前と比べる（名前を [`Material::parse`] で読んだ値と同じか。"AIR" と空は同じ）
impl PartialEq<str> for Material {
    fn eq(&self, other: &str) -> bool {
        *self == Material::parse(other)
    }
}

impl PartialEq<&str> for Material {
    fn eq(&self, other: &&str) -> bool {
        *self == Material::parse(other)
    }
}

impl From<&str> for Material {
    fn from(s: &str) -> Material {
        Material::parse(s)
    }
}

impl From<String> for Material {
    fn from(s: String) -> Material {
        Material::parse(&s)
    }
}

/// モデルガラスの名前 "nd,vd[,dpgf]" の (nd, vd, dpgf)
fn model_numbers(mat: &str) -> Option<(f64, f64, f64)> {
    let v: Vec<f64> = mat
        .split(',')
        .map(|x| x.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .ok()?;
    ((2..=3).contains(&v.len()) && v.iter().all(|x| x.is_finite()) && v[0] > 0.0)
        .then(|| (v[0], v[1], v.get(2).copied().unwrap_or(0.0)))
}

impl serde::Serialize for Material {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.name())
    }
}

impl<'de> serde::Deserialize<'de> for Material {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Material, D::Error> {
        let s = <String as serde::Deserialize>::deserialize(d)?;
        Ok(Material::parse(&s))
    }
}

impl schemars::JsonSchema for Material {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Material".into()
    }

    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        <String as schemars::JsonSchema>::json_schema(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for s in [
            "",
            "MIRROR",
            "-",
            "N-BK7",
            "1.5168,64.17",
            "1.6,30,0.01",
            "1.523456789,0",
        ] {
            let m = Material::parse(s);
            assert_eq!(Material::parse(&m.name()), m, "{s}");
        }
        assert_eq!(Material::parse("AIR"), Material::Air);
        assert_eq!(Material::parse("1.52,0.0"), Material::Index(1.52));
        assert_eq!(Material::Index(1.5234).label(), "1.52,0.0");
    }
}
