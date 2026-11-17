//! 展開の誤り（[`ExpandError`]）と、誤りの起きたブロック（[`BlockAt`]）

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
