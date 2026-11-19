//! 単体試験で .zmx から系を作る。.zmx の読込は lensbench-formats にあり、その系は core の別の組み立て（試験でない
//! もの）の型なので、文書を書いて読み直してこのクレートの [`System`] に移す（キャッシュは読み直しで作り直す）
//! `#[serde(skip)]` の欄（`field_wave`）は運ばない（.zmx の読込はこの欄を埋めないので、読んだ系では同じ）

use crate::System;
pub(crate) use lensbench_formats::zmx::{parse_text, LoadOptions};
use std::path::{Path, PathBuf};

fn conv(s: lensbench_formats::prelude::System) -> System {
    // JSON は無限遠（DISZ INFINITY）を null にするので、f64 をそのまま運べる CBOR で
    let mut b = Vec::new();
    ciborium::into_writer(&s, &mut b).unwrap();
    ciborium::from_reader(&b[..]).unwrap()
}

pub(crate) fn build(
    raw: &lensbench_formats::zmx::Raw,
    opt: &LoadOptions,
    dirs: &[PathBuf],
    base: Option<&Path>,
) -> lensbench_formats::prelude::Result<System> {
    lensbench_formats::zmx::build(raw, opt, dirs, base).map(conv)
}

pub(crate) fn load(path: &Path) -> lensbench_formats::prelude::Result<System> {
    lensbench_formats::zmx::load(path).map(conv)
}
