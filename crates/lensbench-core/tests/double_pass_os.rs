//! OpticStudio 26.1 で作ったダブルパスの見本（tests/double_pass/os_dp_*.zmx）の照合。
//!
//! N-BK7 の両凸レンズ（R ±50、厚さ 5）→ 空気 30 → 平面鏡 → 同じレンズを戻る。戻りの面は往路の面からのピックアップ
//! （厚さは負）、戻りの最後のレンズ面の後ろは空気 −50 とダミー面で、ダミー面の厚さが周辺光線の高さ 0 のソルブ
//! （MAZH）。os_dp_singlet は鏡を座標ブレークで 2° 傾け、os_dp_return_solve は傾けない。鏡が焦点より手前にあるので
//! 像は虚像（レンズの鏡の側）になる。値は OpticStudio の評価関数（TTHI・RSCE・RSCH Ring 6）と REAY。
//! スポット図の RMS（六角配置）は求積と違うので比べない
use lensbench_analysis::analysis::spot_pupil;
use lensbench_core::opd::pupil_gauss_os;
use lensbench_core::report::system_data;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::Waves;
use lensbench_formats::zmx;
use std::path::PathBuf;

/// (系, ソルブの面, ソルブの厚さ, 視野ごとの RSCE・RSCH・主光線の y [mm])
type Case = (&'static str, usize, f64, [[f64; 3]; 3]);

const CASES: [Case; 2] = [
    (
        "os_dp_singlet",
        8,
        71.51821008161872,
        [
            [0.07681762323235496, 0.07731720575893722, 1.7271242518305634],
            [0.1543025690434592, 0.15985070995140827, 5.420791612840047],
            [0.24579788193259983, 0.25466670703754524, 7.958073200842627],
        ],
    ),
    (
        "os_dp_return_solve",
        6,
        71.51821008161872,
        [
            [0.061513882580133135, 0.061513882580133135, 0.0],
            [0.09316337059744871, 0.09825417521514539, 3.6263429842725925],
            [0.1495479351515368, 0.1588188829874017, 6.076146882693531],
        ],
    ),
];

const EFFL: f64 = 69.05458246289597;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests")
}

/// .zmx を、ソルブの面の厚さを t に書き換えて読む（ソルブが解き直すことを確かめる）
fn load(name: &str, t: f64) -> System {
    let txt = std::fs::read_to_string(root().join(format!("double_pass/{name}.zmx"))).unwrap();
    const DISZ: &str = "DISZ 71.518210081618719";
    assert!(
        txt.contains(DISZ),
        "{name}.zmx: the thickness to replace ({DISZ}) is gone"
    );
    let txt = txt.replace(DISZ, &format!("DISZ {t}"));
    let tmp = std::env::temp_dir().join(format!("lb_dp_os_{}_{name}.zmx", std::process::id()));
    std::fs::write(&tmp, txt).unwrap();
    let opts = zmx::LoadOptions {
        catalog_dirs: vec![root().join("glass")],
        ..Default::default()
    };
    let s = zmx::load_with(&tmp, &opts).unwrap();
    let _ = std::fs::remove_file(&tmp);
    s
}

#[test]
fn opticstudio_double_pass_samples_match() {
    let pupil = pupil_gauss_os(6);
    for (name, k, t, rows) in CASES {
        let s = load(name, 60.0);
        assert!(
            (s.surfaces[k].t - t).abs() < 1e-9,
            "{name}: {}",
            s.surfaces[k].t
        );
        let effl = system_data(&s).effl;
        assert!((effl - EFFL).abs() < 1e-9, "{name}: {effl}");
        for (f, [rsce, rsch, cy]) in rows.into_iter().enumerate() {
            let sp = spot_pupil(&s, FieldSel::Index(f), Waves::all_or(Some(0)), &pupil).unwrap();
            assert!(
                (sp.rms_centroid - rsce).abs() < 1e-10,
                "{name} F{f}: {}",
                sp.rms_centroid
            );
            assert!(
                (sp.rms_chief - rsch).abs() < 1e-10,
                "{name} F{f}: {}",
                sp.rms_chief
            );
            assert!(
                (sp.chief.1 - cy).abs() < 1e-10,
                "{name} F{f}: {}",
                sp.chief.1
            );
            assert_eq!(sp.rays.vignetted, 0, "{name} F{f}");
        }
    }
}
