//! 展開の誤り（[`ExpandError`]）と、誤りの起きたブロック（[`BlockAt`]）、展開の注意（[`Note`]）

use crate::{block_id, Block, NodePath, NodeStep};

/// 展開の誤り: 誤りと、それが起きたプロジェクトのブロック（ブロックの中で起きたとき）
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandError {
    pub error: lensbench_core::Error,
    pub block: Option<BlockAt>,
}

/// 誤りの起きたブロック: 木の中の位置と、表示の名前（各段の id、id の無いブロックは「#番号」、BS の腕は t・r・d）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockAt {
    pub node: NodePath,
    pub name: String,
}

impl BlockAt {
    pub(crate) fn new(blocks: &[Block], node: NodePath) -> BlockAt {
        let mut parts = Vec::new();
        let mut v = blocks;
        for (k, st) in node.0.iter().enumerate() {
            let (arm, i) = match *st {
                NodeStep::Block(i) => (None, i),
                NodeStep::Arm(a, i) => (Some(a), i),
            };
            if let Some(a) = arm {
                parts.push(["t", "r", "d"][usize::from(a).min(2)].to_string());
            }
            let Some(b) = v.get(i) else { break };
            let id = block_id(b);
            parts.push(if id.is_empty() {
                format!("#{}", i + 1)
            } else {
                id.to_string()
            });
            v = match (b, arm) {
                (Block::Group { blocks, .. }, _) => blocks,
                (Block::Splitter { spec, .. }, _) => match node.0.get(k + 1) {
                    Some(NodeStep::Arm(0, _)) => &spec.t,
                    Some(NodeStep::Arm(1, _)) => &spec.r,
                    Some(NodeStep::Arm(_, _)) => &spec.d,
                    _ => &[],
                },
                _ => &[],
            };
        }
        BlockAt {
            node,
            name: parts.join("/"),
        }
    }
}

impl ExpandError {
    /// 誤りの種類（[`lensbench_core::Error::kind`]）
    pub fn kind(&self) -> &'static str {
        self.error.kind()
    }
    /// 誤りの文（ブロックの名前は付けない。[`lensbench_core::Error::message`]）
    pub fn message(&self) -> Option<&str> {
        self.error.message()
    }
    /// 誤りの符号（[`lensbench_core::Error::code`]）
    pub fn code(&self) -> &'static str {
        self.error.code()
    }
    /// ブロックの名前を前に付けた誤り（ブロックの外なら誤りそのもの）
    pub fn into_error(self) -> lensbench_core::Error {
        match self.block {
            Some(b) => {
                let at = b.name;
                self.error.map_message(|e| format!("block {at}: {e}"))
            }
            None => self.error,
        }
    }
}

impl std::fmt::Display for ExpandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.clone().into_error().fmt(f)
    }
}

impl std::error::Error for ExpandError {}

impl From<lensbench_core::Error> for ExpandError {
    fn from(error: lensbench_core::Error) -> Self {
        ExpandError { error, block: None }
    }
}

impl From<ExpandError> for lensbench_core::Error {
    fn from(e: ExpandError) -> Self {
        e.into_error()
    }
}

impl From<ExpandError> for String {
    fn from(e: ExpandError) -> Self {
        e.to_string()
    }
}

/// 展開の注意（[`crate::Expansion::notes`]）: 展開で落とした・変えたもの
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub kind: NoteKind,
    /// 注意の元のブロック・群の id（ブロックに依らない注意・取り込んだ .zmx の注意は None）
    pub block: Option<String>,
    /// 注意の文（ブロックの名前・.zmx のパスも含む）
    pub text: String,
}

/// 注意の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NoteKind {
    /// 取り込んだ .zmx のピックアップ・ソルブを展開で保てなかった・符号を値で選んだ、ソルブで値を求め直した
    Pickup,
    /// 取り込んだ .zmx の温度・気圧がプロジェクトと違う
    Environment,
    /// 取り込んだ .zmx の膜ファイルが前の .zmx と違う
    Coating,
    /// id のブロック・群を玉・群のパラメータにできなかった
    Unit,
    /// 面の操作（`surface_ops`）を当てなかった・当てた操作の注意
    SurfaceOp,
    /// 全体の基準の面（`glrs`）のブロックが無い
    Glrs,
    /// 機構部品を置かなかった・ねじが合わない
    Mechanics,
}

impl Note {
    pub fn new(kind: NoteKind, block: Option<String>, text: String) -> Note {
        Note { kind, block, text }
    }

    pub(crate) fn all(kind: NoteKind, block: Option<String>, v: Vec<String>) -> Vec<Note> {
        v.into_iter()
            .map(|t| Note::new(kind, block.clone(), t))
            .collect()
    }
}

impl From<Note> for String {
    fn from(n: Note) -> String {
        n.text
    }
}

impl std::fmt::Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}
