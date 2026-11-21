//! 玉の縁でける浮動開口（プロジェクトの展開の最後に付ける）

/// 半直径を固定した玉の面（前か後ろがガラス。ガラスは鏡でない媒質）で開口の無い面に浮動開口を付け、玉の縁の外を通る光線をける
/// （玉の外の光線は玉を通らない。.zmx の読込・OpticStudio 2026 R1 の読込と同じ規則）。座標ブレークと、絞りの大きさで
/// 決める系（Float By Stop Size）の絞り（半直径が瞳そのもの）と、半直径を書いていない面（0）には付けない
pub(crate) fn lens_edge_apertures(s: &mut lensbench_core::System) -> lensbench_core::Result<()> {
    use lensbench_core::surface::Aperture;
    let glass = |d: &lensbench_core::surface::Surface| {
        d.mat.is_medium() && !d.mat.is_air() && !d.is_mirror()
    };
    let float_stop =
        (s.aperture.0 == lensbench_core::system::ApertureKind::Float).then(|| s.stop());
    let ks: Vec<usize> = (1..s.surfaces.len().saturating_sub(1))
        .filter(|&k| {
            let d = &s.surfaces[k];
            !d.semi_auto
                && d.semi > 0.0
                && d.ap == Aperture::None
                && d.typ() != lensbench_core::surface::SurfType::CoordBrk
                && Some(k) != float_stop
                && (glass(d) || glass(&s.surfaces[k - 1]))
        })
        .collect();
    if ks.is_empty() {
        return Ok(());
    }
    s.edit_raw(|rx| {
        for k in ks {
            let semi = rx.surfaces[k].semi;
            rx.surfaces[k].ap = Aperture::Floating { semi };
        }
        Ok(())
    })
}
