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
    /// 玉の面での部分反射で折り返す（ゴーストの経路、干渉計の裏面反射など）。面の反射は境界（膜込み）の反射。
    /// 1 つ（オブジェクト）か並び: 1 つ目は往路で、2 つ目は戻る道で、3 つ目はまた往路で…と向きを替えながら順に
    /// 折り返す。戻りは鏡の `back` と同じく前のブロックをたどり、BS では経路の文字で口を選ぶ。戻ったまま終われば
    /// 戻った先で、往路で終われば並びの終わりで系は終わる（像面）。玉（`lens` ブロック）の面だけ。`pass` は前の
    /// 折り返しからその向きで何回目にそのブロックを通るか（折り返した玉の中を続けて通るのは 0）
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        with = "crate::beams::one_or_many"
    )]
    #[schemars(with = "crate::beams::TurnsSchema")]
    pub turn: Vec<crate::edit_spec::BlockFace>,
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
            for t in &b.turn {
                if t.back || t.coordbrk {
                    return Err(format!(
                        "beam {name}: turn takes a lens surface as numbered on the way out (not back or coordbrk)"
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

/// 光束をまたぐ最適化の変数の名前 `ブロック:面.種類`（例 `L:0.c`、面は座標ブレークを除いて 0 から）を
/// (ブロックの id, 面, 種類) に分ける。この形でなければ None
pub fn face_var(name: &str) -> Option<(&str, usize, &str)> {
    let (block, rest) = name.trim().split_once(':')?;
    let (k, kind) = rest.split_once('.')?;
    let k = k.trim().parse().ok()?;
    let block = block.trim();
    (!block.is_empty() && !kind.trim().is_empty()).then_some((block, k, kind.trim()))
}

/// ブロック block の面 surface（座標ブレークを除いて 0 から）が系 s で通るすべての面の番号（往路の各回と、鏡の `back`・
/// 光束の `turn` で戻る復路の各回。復路は往路の逆の順に通る）。spans は展開の [`crate::block_spans`]。系に無ければ空
pub fn face_surfaces(
    s: &lensbench_core::System,
    spans: &[crate::BlockSpans],
    block: &str,
    surface: usize,
) -> lensbench_core::Result<Vec<usize>> {
    let Some(b) = spans.iter().find(|b| b.id == block) else {
        return Ok(Vec::new());
    };
    let faces = |r: &[usize; 2]| -> Vec<usize> {
        (r[0]..=r[1].min(s.surfaces.len().saturating_sub(1)))
            .filter(|&j| !s.surfaces[j].is_coord_break())
            .collect()
    };
    // 光束の turn で折り返しながらたどった範囲は、玉の面の番号で引く（範囲は玉の途中から始まる・途中で終わる）
    let parted = |r: &[usize; 2]| b.parts.iter().any(|&(j, _)| r[0] <= j && j <= r[1]);
    let part = |r: &[usize; 2]| {
        b.parts
            .iter()
            .filter(|&&(j, i)| r[0] <= j && j <= r[1] && i == surface)
            .map(|&(j, _)| j)
            .collect::<Vec<_>>()
    };
    let mut out = Vec::new();
    for r in &b.forward {
        if parted(r) {
            out.extend(part(r));
            continue;
        }
        let f = faces(r);
        let Some(&j) = f.get(surface) else {
            return Err(format!("block {block} surface {surface} ({} surfaces)", f.len()).into());
        };
        out.push(j);
    }
    for r in &b.back {
        if parted(r) {
            out.extend(part(r));
            continue;
        }
        let f = faces(r);
        if let Some(k) = f.len().checked_sub(surface + 1) {
            out.push(f[k]);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// 展開した系の面ごとの、光束の `turn` で指せる玉（`lens` ブロック）の面（id のあるブロック、往路の面だけ。BS の腕の
/// 中の玉も）。map は展開の面の出どころ（[`crate::Expansion::map`]）
pub fn turn_points(
    p: &Project,
    map: &[crate::SurfOrigin],
) -> Vec<Option<crate::edit_spec::BlockFace>> {
    fn find<'x>(blocks: &'x [crate::Block], id: &str) -> Option<&'x crate::Block> {
        for b in blocks {
            if crate::block_id(b) == id {
                return Some(b);
            }
            let kids: Vec<&Vec<crate::Block>> = match b {
                crate::Block::Group { blocks, .. } => vec![blocks],
                crate::Block::Splitter { spec, .. } => vec![&spec.t, &spec.r, &spec.d],
                _ => Vec::new(),
            };
            if let Some(x) = kids.into_iter().find_map(|k| find(k, id)) {
                return Some(x);
            }
        }
        None
    }
    map.iter()
        .enumerate()
        .map(|(j, o)| {
            if o.back || o.placed {
                return None;
            }
            let id = o.blocks.first()?;
            if !matches!(find(&p.blocks, id), Some(crate::Block::Lens { .. })) {
                return None;
            }
            let runs = crate::block_runs(map, id, Some(false));
            let pass = runs.iter().position(|r| r[0] <= j && j <= r[1])?;
            let surface = (runs[pass][0]..j).filter(|&i| !map[i].placed).count();
            Some(crate::edit_spec::BlockFace {
                block: id.clone(),
                surface,
                coordbrk: false,
                back: false,
                pass,
            })
        })
        .collect()
}

/// 光束の `turn`: 1 つならオブジェクト、2 つ以上なら並びで書く（1 つの折り返しの前のファイルと同じ形）
pub(crate) mod one_or_many {
    use crate::edit_spec::BlockFace;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(BlockFace),
        Many(Vec<BlockFace>),
    }

    pub(crate) fn serialize<S: Serializer>(v: &[BlockFace], s: S) -> Result<S::Ok, S::Error> {
        match v {
            [one] => one.serialize(s),
            _ => v.serialize(s),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<BlockFace>, D::Error> {
        Ok(match Option::<OneOrMany>::deserialize(d)? {
            None => Vec::new(),
            Some(OneOrMany::One(x)) => vec![x],
            Some(OneOrMany::Many(v)) => v,
        })
    }
}

/// [`Beam::turn`] のスキーマ（ブロックの面 1 つか、その並び）
#[derive(schemars::JsonSchema)]
#[serde(untagged)]
#[allow(dead_code)]
pub(crate) enum TurnsSchema {
    One(crate::edit_spec::BlockFace),
    Many(Vec<crate::edit_spec::BlockFace>),
}
