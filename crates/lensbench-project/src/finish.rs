//! プロジェクトの面の仕上げ（[`Project::finishes`]）: 散乱と、光を通さない物の反射率を、ブロックの面・玉の縁・機構部品に
//! 付ける。展開で系の面の番号の仕上げ（[`lensbench_core::scatter::Finish`]、[`System::finishes`]）に直す。
//! 非順次の追跡（[`crate::nsc`]）だけが使い、順次の追跡・解析には効かない。

use crate::edit_ops::BlockFace;
use crate::{block_id, Block, SurfOrigin};
use lensbench_core::scatter::{Finish, Scatter};
use lensbench_core::{invalid, Result, System};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 面の仕上げ 1 つ。対象は `face`（ブロックの面）・`edge`（その面を持つ玉の縁）・`mech`（機構部品の名前）のどれか 1 つ
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FinishSpec {
    /// ブロックの面（光学面。空気中の絞り・遮光板のブロックなら開口の外の板）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<BlockFace>,
    /// このブロックの面を持つ玉の縁（縁の輪と円柱）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<BlockFace>,
    /// 機構部品の名前（壁）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mech: Option<String>,
    /// 光を通さない面の反射率（0〜1、残りは吸収）。機構部品・絞りの板は省略で 0（黒）。玉の縁に書くと縁は光を
    /// 通さない（塗った縁）。光学面には書けない（反射は膜込みの Fresnel）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflect: Option<f64>,
    /// 散乱（光学面・透明な縁では反射と透過の光、光を通さない面では反射した光）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scatter: Option<Scatter>,
}

impl FinishSpec {
    /// 系の面の番号を入れる前の仕上げ（対象はまだ空）
    fn props(&self) -> Finish {
        Finish {
            reflect: self.reflect,
            scatter: self.scatter.clone(),
            ..Default::default()
        }
    }

    /// 値の確かめ（対象がちょうど 1 つ、ブロックがプロジェクトにある、値の範囲）
    pub fn check(&self, blocks: &[Block]) -> Result<()> {
        let n = usize::from(self.face.is_some())
            + usize::from(self.edge.is_some())
            + usize::from(self.mech.is_some());
        if n != 1 {
            invalid!("finishes: give one of face, edge and mech ({n} given)");
        }
        for f in [&self.face, &self.edge].into_iter().flatten() {
            if f.coordbrk {
                invalid!("finishes: block {} is a coordinate break", f.block);
            }
            if !has_block(blocks, &f.block) {
                invalid!("finishes: no block {}", f.block);
            }
        }
        let mut p = self.props();
        p.mech = Some(String::new());
        if self.mech.is_none() && self.reflect.is_none() && self.scatter.is_none() {
            invalid!("finishes: give reflect or scatter");
        }
        p.check()
    }
}

/// id のブロックが木（群・BS の腕の中も）にあるか
fn has_block(blocks: &[Block], id: &str) -> bool {
    blocks.iter().any(|b| {
        block_id(b) == id
            || match b {
                Block::Group { blocks, .. } => has_block(blocks, id),
                Block::Splitter { spec, .. } => {
                    [&spec.t, &spec.r, &spec.d].iter().any(|k| has_block(k, id))
                }
                _ => false,
            }
    })
}

/// 展開した系 s（面の出どころ map）の仕上げ: ブロックの面を系の面の番号に直す。この展開に無いブロック・機構部品の
/// 仕上げは除く（BS のほかの経路など）
pub(crate) fn resolve(s: &System, specs: &[FinishSpec], map: &[SurfOrigin]) -> Result<Vec<Finish>> {
    let mut out = Vec::new();
    for f in specs {
        let mut x = f.props();
        if let Some(b) = &f.face {
            match crate::faces::block_face_of(s, b, map)? {
                Some(j) => x.surface = Some(j),
                None => continue,
            }
        } else if let Some(b) = &f.edge {
            match crate::faces::block_face_of(s, b, map)? {
                Some(j) => x.edge = Some(j),
                None => continue,
            }
        } else if let Some(m) = &f.mech {
            if !s.mechanics.iter().any(|p| &p.name == m) {
                continue;
            }
            x.mech = Some(m.clone());
        }
        out.push(x);
    }
    Ok(out)
}
