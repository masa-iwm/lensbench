//! 光束の組（[`Project::beams`]）: ビームスプリッターの経路に名前・状態・役割を付けたもの。名前は状態の名前と同じ所で引く

use lensbench_core::error::{code, Error};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Project;

/// 光束の役割（[`Beam::role`]）。表示の色と既定の組み合わせだけに使い、計算は変えない
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BeamRole {
    /// 試料の光束（被検の系を通る）
    Sample,
    /// 参照の光束
    Reference,
    /// 迷光の経路
    Stray,
}

/// 光束（[`Project::beams`]）: ビームスプリッターの経路 1 本に名前を付けたもの。経路を選ぶだけで、分岐比・膜・偏光は
/// 追跡で決まる。名前は状態の名前と同じ所（`--state`、干渉の光束など）で引ける
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Beam {
    /// 光束の名前（状態の名前と重ならない）
    pub name: String,
    /// ビームスプリッターの経路（BS を通る順の T・R）。省略で状態・プロジェクトの path
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// この光束に当てる状態の名前（経路ごとに系の設定を変える）。省略で状態を当てない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// 役割（`sample`・`reference`・`stray`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<BeamRole>,
    /// 玉の面での部分反射で来た道を戻る（ゴーストの経路、干渉計の裏面反射など）。面の反射は境界（膜込み）の反射で、
    /// 戻りは鏡の `back` と同じく前のブロックをたどり、BS では経路の文字で口を選ぶ。戻った先で系は終わる（像面）。
    /// 玉（`lens` ブロック）の面だけ、1 か所
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<crate::edit_spec::BlockFace>,
    /// 解析の既定の組から外す
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub off: bool,
}

impl Project {
    /// 光束の組の誤り: 名前の空・重なり（光束どうし・状態と）、無い状態、経路の文字
    pub fn check_beams(&self) -> lensbench_core::Result<()> {
        for (i, b) in self.beams.iter().enumerate() {
            let name = &b.name;
            if name.trim().is_empty() {
                return Err(format!("beam {}: the name is empty", i + 1).into());
            }
            if ["0", "ref", "reference"].contains(&name.trim().to_ascii_lowercase().as_str()) {
                return Err(format!(
                    "beam {name}: 0, ref and reference mean the reference sphere in interference (use another name)"
                )
                .into());
            }
            if self.beams[..i].iter().any(|c| c.name == *name) {
                return Err(format!("two or more beams named {name}").into());
            }
            if self.states.iter().any(|s| s.name == *name) {
                return Err(format!("beam {name}: a state has the same name").into());
            }
            if let Some(st) = &b.state {
                if !self.states.iter().any(|s| s.name == *st) {
                    return Err(Error::Invalid(format!("beam {name}: no state {st}"))
                        .with_code(code::STATE_NOT_FOUND));
                }
            }
            if let Some(path) = &b.path {
                crate::splitter::parse_path(path).map_err(|e| format!("beam {name}: {e}"))?;
            }
            if let Some(t) = &b.turn {
                if t.back || t.coordbrk {
                    return Err(format!(
                        "beam {name}: turn takes a surface on the way out (not back or coordbrk)"
                    )
                    .into());
                }
                if t.block.is_empty() {
                    return Err(format!("beam {name}: turn needs a block id").into());
                }
            }
        }
        Ok(())
    }

    /// 光束の名前（[`Project::beams`] の順）
    pub fn beam_names(&self) -> Vec<&str> {
        self.beams.iter().map(|b| b.name.as_str()).collect()
    }

    /// 名前が name の光束
    pub fn beam(&self, name: &str) -> Option<&Beam> {
        self.beams.iter().find(|b| b.name == name)
    }

    /// 状態・光束の名前 name と一緒に別の経路を渡してよいか（経路を持つ光束には渡せない）
    pub fn beam_takes_path(&self, name: &str, path_given: bool) -> lensbench_core::Result<()> {
        if path_given && self.beam(name).is_some_and(|b| b.path.is_some()) {
            return Err(Error::Invalid(format!(
                "beam {name} has its own path, so another path can't be given"
            ))
            .with_code(code::BAD_ARGUMENT));
        }
        Ok(())
    }

    /// [`Project::with_state`]。別の経路を渡すなら（path_given）、経路を持つ光束は誤り
    pub fn with_state_and_path(
        &self,
        name: &str,
        path_given: bool,
    ) -> lensbench_core::Result<Project> {
        self.beam_takes_path(name, path_given)?;
        self.with_state(name)
    }
}
