//! OpticStudio 26.1 の MTF 系の基準（tests/refs/<系>/mtf.json・mtf_weights.json・mtf_family.json・psf_os.npz）との照合
//! （中身と許容差は `lensbench_verify::mtf_refs`）。LB_REFS_DIR・LB_GLASS_DIR で基準・ガラスを差し替え、
//! LB_REFS=A0,B1 で系を絞る。

use lensbench_verify::mtf_refs;
use lensbench_verify::zemax_refs::{glass_dirs, refs_dir};
use std::path::{Path, PathBuf};

type Check = fn(&Path, &[PathBuf], Option<&str>) -> Vec<String>;

/// `file` を持つ系が 1 つはある（フォルダを動かしたときに 0 系で黙って通らないように）うえで照合する
fn run(file: &str, check: Check) {
    let only = std::env::var("LB_REFS").ok();
    let refs = refs_dir();
    assert!(
        !mtf_refs::systems_with(&refs, file, only.as_deref()).is_empty(),
        "{} に {file} のある系が無い",
        refs.display()
    );
    let fails = check(&refs, &glass_dirs(), only.as_deref());
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

#[test]
fn fft_mtf() {
    run("mtf.json", mtf_refs::fft_mtf);
}

#[test]
fn polychromatic_mtf_uses_the_wavelength_weights() {
    let fails = mtf_refs::mtf_weights(&refs_dir(), &glass_dirs());
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

#[test]
fn through_focus_mtf() {
    run("mtf_family.json", mtf_refs::through_focus);
}

#[test]
fn mtf_vs_field() {
    run("mtf_family.json", mtf_refs::mtf_vs_field);
}

#[test]
fn rms_family() {
    run("mtf_family.json", mtf_refs::rms_family);
}

#[test]
fn geometric_mtf() {
    run("mtf_family.json", mtf_refs::geometric_mtf);
}

#[test]
fn fft_psf() {
    run("mtf_family.json", mtf_refs::fft_psf);
}
