//! .zmx の書き出し: 読み直したときの開口

/// ガラスの中の座標ブレークは、半直径を固定にした系でも自動で書く（固定で書くと読込で半直径 0 の浮動開口になり、
/// 光線が全部けられる。プロジェクトの BS の立方体の中の座標ブレークがこの形）
#[test]
fn a_coordinate_break_in_glass_is_written_with_an_automatic_semi() {
    let dir = std::env::temp_dir().join(format!("lb_zmx_cb_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("in.zmx");
    std::fs::write(
        &src,
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 4\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 5\n  GLAS ___BLANK 1 0 1.5 0 0 0 0 0 0 0\n\
SURF 2\n  TYPE COORDBRK\n  CURV 0.0\n  PARM 3 10\n  DISZ 0\n  DIAM 0 0 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
    )
    .unwrap();
    let o = lensbench_formats::zmx::LoadOptions {
        cfg: 1,
        ..Default::default()
    };
    let mut s = lensbench_formats::zmx::load_with(&src, &o).unwrap();
    assert_eq!(s.surfaces[2].ap, lensbench_core::surface::Aperture::None);
    s.edit_raw(|rx| {
        rx.surfaces[2].semi_auto = false;
        rx.surfaces[2].semi = 0.0;
        Ok(())
    })
    .unwrap();
    let out = dir.join("out.zmx");
    lensbench_formats::zmx_write::save(&out, &s).unwrap();
    let t = lensbench_formats::zmx::load_with(&out, &o).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(
        t.surfaces[2].typ(),
        lensbench_core::surface::SurfType::CoordBrk
    );
    assert_eq!(t.surfaces[2].ap, lensbench_core::surface::Aperture::None);
}
