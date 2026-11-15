//! 表の行: 今の階層の面と、たたんだブロック・折りたたみ表示で開いたブロックの中の面（lens_data.rs から分けた、GPN-03）

use super::Block;

/// 表の行（鍵の面、たたんだブロックなら (ブロック, 最初の面)）。ブロックの行の鍵は最後の面（その厚さ = ブロックの後の間隔）。
/// 往復するブロックは往路・復路でそれぞれ 1 行
pub(super) fn fold_lines(
    surfs: &[usize],
    child_of: impl Fn(usize) -> Option<usize>,
) -> Vec<(usize, Option<(usize, usize)>)> {
    let mut v: Vec<(usize, Option<(usize, usize)>)> = Vec::new();
    for &j in surfs {
        match child_of(j) {
            Some(i) => match v.last_mut() {
                Some((key, Some((b, _)))) if *b == i && *key + 1 == j => *key = j,
                _ => v.push((j, Some((i, j)))),
            },
            None => v.push((j, None)),
        }
    }
    v
}

/// 表の 1 行
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Line {
    /// 鍵の面（たたんだブロックはその最後の面）
    pub(super) key: usize,
    /// たたんだブロック（ブロック, 最初の面）
    pub(super) fold: Option<(usize, usize)>,
    /// 開いたブロックの深さ（字下げ）
    pub(super) depth: usize,
    /// この行から始まる開いたブロック（外から順）
    pub(super) heads: Vec<usize>,
    /// この行の属するいちばん内側のブロック（たたんだブロックはそのもの）
    pub(super) blk: Option<usize>,
}

/// 階層 scope の面 surfs を表の行にする。open(i) のブロックはたたまずに中の面（と中のブロック）を字下げして並べる
pub(super) fn lines_of(
    blocks: &[Block],
    scope: Option<usize>,
    surfs: &[usize],
    open: &dyn Fn(usize) -> bool,
    depth: usize,
) -> Vec<Line> {
    // 面 → 直下のブロック（最初のもの）。面ごとに全ブロックを探さない（GPN-23）
    let mut owner: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for (i, b) in blocks.iter().enumerate().rev() {
        if b.parent == scope {
            for &j in &b.surfs {
                owner.insert(j, i);
            }
        }
    }
    let child_of = |j: usize| owner.get(&j).copied();
    let mut v = Vec::new();
    for (key, f) in fold_lines(surfs, child_of) {
        match f {
            Some((i, a)) if open(i) => {
                let inner: Vec<usize> = blocks[i]
                    .surfs
                    .iter()
                    .copied()
                    .filter(|&x| a <= x && x <= key)
                    .collect();
                let mut sub = lines_of(blocks, Some(i), &inner, open, depth + 1);
                if let Some(first) = sub.first_mut() {
                    first.heads.insert(0, i);
                }
                v.extend(sub);
            }
            Some((i, a)) => v.push(Line {
                key,
                fold: Some((i, a)),
                depth,
                heads: Vec::new(),
                blk: Some(i),
            }),
            None => v.push(Line {
                key,
                fold: None,
                depth,
                heads: Vec::new(),
                blk: scope,
            }),
        }
    }
    v
}

/// 面 j を表で見せる行の鍵（その行か、j を含むたたんだ行）。表に無ければ None
pub(super) fn visible_key(lines: &[Line], blocks: &[Block], j: usize) -> Option<usize> {
    lines
        .iter()
        .find(|l| match l.fold {
            None => l.key == j,
            Some((i, a)) => a <= j && j <= l.key && blocks[i].surfs.contains(&j),
        })
        .map(|l| l.key)
}

/// ブロック i の中で、面 j から続く面の最後（たたんだときの行の鍵）
pub(super) fn run_end(b: &Block, j: usize) -> usize {
    let mut k = j;
    while b.surfs.contains(&(k + 1)) {
        k += 1;
    }
    k
}
