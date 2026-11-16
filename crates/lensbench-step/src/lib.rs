//! STEP（ISO 10303-21）の文字の読み手（PRJ-06）。依存の無い小さなクレートで、品の寸法の測り（lensbench-catalog の
//! `step`）と、書き出す STEP への立体の写し（lensbench-export の `step_embed`）が同じ表 [`StepFile`] を使う。
//! 三角形にするのは truck（lensbench-cad）。
//!
//! 以前は読み手が 2 つ（lensbench-project の `step_geom` と lensbench-export の `step_embed`）あった。

#![warn(unreachable_pub)]
pub mod p21;

pub use p21::{refs, split, Arg, Entity, Part, StepFile};
