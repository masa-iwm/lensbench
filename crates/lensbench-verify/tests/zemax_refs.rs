//! OpticStudio 26.1 の基準データ（tests/refs）との照合（中身は `lensbench_verify::zemax_refs`）。
//!
//! 許容差: エイミングなしは丸め誤差まで（位置 1e-11 mm、方向 1e-12）、エイミングありは
//! OpticStudio 自身のエイミングの収束（〜1e-7 mm）で位置 2e-7 mm・方向 1e-8。OPD [波] はエイミングなし 5e-6
//! （理想レンズの積分モード D2 は 5e-5）、ありは 2e-3。ガラスはテスト用カタログ tests/glass（refractiveindex.info、CC0）。
//! LB_REFS_DIR・LB_GLASS_DIR で基準・ガラスを差し替え、LB_REFS=A0,B1 で系を絞る。

use lensbench_verify::zemax_refs::{check_dir, glass_dirs, refs_dir};

#[test]
fn against_opticstudio() {
    let only = std::env::var("LB_REFS").ok();
    let fails = check_dir(&refs_dir(), &glass_dirs(), only.as_deref());
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}
