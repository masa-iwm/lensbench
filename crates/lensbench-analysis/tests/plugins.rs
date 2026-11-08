//! 同梱の解析のプラグインは core の関数を直接呼んだ結果とビット一致し、視野をまとめた呼び出しはスレッド数によらない
use std::path::Path;
use std::sync::Arc;

use lensbench_core::analysis::{self, MtfMethod, ZernKind};
use lensbench_core::coating::Coatings;
use lensbench_core::plugin::{run_many, Array, Arrays, Config, Context, Registry, Value};
use lensbench_core::pop::{self, OutGrid, PopMethod, RayBeamOpts};
use lensbench_core::psf::{self, PsfOpts};
use lensbench_core::sweep::{self, RmsOpts, RmsRef};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{FieldSel, System};

fn sys() -> Arc<System> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    Arc::new(zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap())
}

fn bits(v: &[f64]) -> Vec<u64> {
    v.iter().map(|x| x.to_bits()).collect()
}

fn reg() -> Registry {
    let mut r = Registry::new();
    lensbench_analysis::register(&mut r);
    r
}

fn field(i: f64) -> Arrays {
    Arrays::from([("field".into(), Array::scalar(i))])
}

fn run(name: &str, cfg: Config, s: &Arc<System>, f: f64) -> Arrays {
    reg()
        .prepare(name, &cfg)
        .unwrap()
        .run(&Context::with_system(s), &field(f))
        .unwrap()
}

fn get<'a>(o: &'a Arrays, k: &str) -> &'a [f64] {
    o[k].as_f64().unwrap()
}

#[test]
fn all_registered() {
    let names: Vec<String> = reg().list().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        [
            "geo_mtf",
            "ghost",
            "huygens_mtf",
            "huygens_psf",
            "interferogram",
            "mtf",
            "pop",
            "psf",
            "rms",
            "spot",
            "wavefront",
            "zernike"
        ]
    );
}

#[test]
fn spot_wavefront_zernike_match_core() {
    let s = sys();
    let o = run(
        "spot",
        Config::from([("rings".into(), Value::Num(4.0))]),
        &s,
        2.0,
    );
    let r = analysis::spot(&s, FieldSel::Index(1), None, 4).unwrap();
    assert_eq!(bits(get(&o, "x")), bits(&r.x));
    assert_eq!(get(&o, "rms_centroid"), &[r.rms_centroid]);
    let o = run(
        "wavefront",
        Config::from([
            ("n".into(), Value::Num(32.0)),
            ("wave".into(), Value::Num(1.0)),
        ]),
        &s,
        2.0,
    );
    let r = analysis::wavefront(&s, FieldSel::Index(1), Some(0), 32, true).unwrap();
    assert_eq!(o["w"].shape, vec![32, 32]);
    assert_eq!(bits(get(&o, "w")), bits(&r.w.concat()));
    let cfg = Config::from([
        ("kind".into(), Value::Str("fringe".into())),
        ("terms".into(), Value::Num(16.0)),
    ]);
    let o = run("zernike", cfg, &s, 1.0);
    let r =
        analysis::zernike(&s, FieldSel::Index(0), None, ZernKind::Fringe, 16, 64, true).unwrap();
    assert_eq!(bits(get(&o, "coef")), bits(&r.coef));
    assert_eq!(get(&o, "strehl"), &[r.strehl]);
}

#[test]
fn mtf_and_psf_match_core() {
    let s = sys();
    let fs = vec![0.0, 10.0, 25.0, 40.0];
    let cfg = Config::from([
        ("freqs".into(), Value::Nums(fs.clone())),
        ("method".into(), Value::Str("fft".into())),
    ]);
    let o = run("mtf", cfg, &s, 2.0);
    let r = analysis::mtf_with(&s, FieldSel::Index(1), None, None, &fs, MtfMethod::Fft).unwrap();
    assert_eq!(bits(get(&o, "tan")), bits(&r.tan));
    assert_eq!(bits(get(&o, "sag")), bits(&r.sag));
    // 周波数を省くと CLI と同じ 21 点
    let o = run("mtf", Config::new(), &s, 1.0);
    assert_eq!(get(&o, "freqs").len(), 21);
    let cfg = Config::from([
        ("pupil".into(), Value::Num(32.0)),
        ("image".into(), Value::Num(64.0)),
    ]);
    let o = run("psf", cfg, &s, 1.0);
    let opts = PsfOpts {
        pupil: 32,
        image: 64,
        delta: None,
    };
    let r = psf::psf(&s, FieldSel::Index(0), None, opts).unwrap();
    assert_eq!(o["i"].shape, vec![64, 64]);
    assert_eq!(bits(get(&o, "i")), bits(&r.i.concat()));
}

#[test]
fn many_fields_do_not_depend_on_threads() {
    let s = sys();
    let inst = reg()
        .prepare(
            "mtf",
            &Config::from([("freqs".into(), Value::Nums(vec![5.0, 20.0]))]),
        )
        .unwrap();
    let ctx = Context::with_system(&s);
    let inputs = vec![
        field(1.0),
        field(2.0),
        Arrays::from([("field".into(), Array::f64(vec![2], vec![0.0, 0.5]).unwrap())]),
    ];
    let a = lensbench_core::with_threads(Some(1), || run_many(inst.as_ref(), &ctx, &inputs));
    let b = lensbench_core::with_threads(Some(3), || run_many(inst.as_ref(), &ctx, &inputs));
    assert_eq!(a, b);
    // 誤り: 系なし・視野の範囲外・知らない設定・波長の範囲外
    assert!(inst.run(&Context::default(), &field(1.0)).is_err());
    assert!(inst.run(&ctx, &field(9.0)).is_err());
    assert!(reg()
        .prepare("spot", &Config::from([("ring".into(), Value::Num(3.0))]))
        .is_err());
    let w = reg()
        .prepare("spot", &Config::from([("wave".into(), Value::Num(99.0))]))
        .unwrap();
    assert!(w.run(&ctx, &field(1.0)).is_err());
}

#[test]
fn rms_geo_mtf_pop_ghost_match_core() {
    let s = sys();
    let cfg = Config::from([("reference".into(), Value::Str("chief".into()))]);
    let o = run("rms", cfg, &s, 2.0);
    let o2 = RmsOpts {
        rings: 6,
        reference: RmsRef::Chief,
        ..Default::default()
    };
    let r = sweep::rms(&s, FieldSel::Index(1), None, o2).unwrap();
    assert_eq!(get(&o, "wfe"), &[r.wfe]);
    assert_eq!(get(&o, "spot"), &[r.spot]);
    let fs = vec![0.0, 20.0, 40.0];
    let cfg = Config::from([
        ("freqs".into(), Value::Nums(fs.clone())),
        ("n".into(), Value::Num(64.0)),
    ]);
    let o = run("geo_mtf", cfg, &s, 1.0);
    let r = psf::geometric_mtf(&s, FieldSel::Index(0), None, 64, &fs, false).unwrap();
    assert_eq!(bits(get(&o, "tan")), bits(&r.tan));
    // Huygens PSF・MTF
    let cfg = Config::from([
        ("wave".into(), Value::Num(1.0)),
        ("pupil".into(), Value::Num(16.0)),
        ("image".into(), Value::Num(16.0)),
    ]);
    let ho = lensbench_core::huygens::HuygensOpts {
        pupil: 16,
        image: 16,
        delta: None,
    };
    let o = run("huygens_psf", cfg.clone(), &s, 2.0);
    let r = lensbench_core::huygens::huygens_psf(&s, FieldSel::Index(1), Some(0), ho).unwrap();
    assert_eq!(bits(get(&o, "i")), bits(&r.i.concat()));
    let mut cfg = cfg;
    cfg.insert("freqs".into(), Value::Nums(fs.clone()));
    let o = run("huygens_mtf", cfg, &s, 2.0);
    let r = lensbench_core::huygens::huygens_mtf(&s, FieldSel::Index(1), Some(0), ho, &fs).unwrap();
    assert_eq!(bits(get(&o, "sag")), bits(&r.sag));
    // POP: 絞りの後の場を 5 mm 伝搬
    let surf = s.stop();
    let cfg = Config::from([
        ("surface".into(), Value::Num(surf as f64)),
        ("wave".into(), Value::Num(1.0)),
        ("rays".into(), Value::Num(33.0)),
        ("n".into(), Value::Num(64.0)),
        ("dist".into(), Value::Num(5.0)),
    ]);
    let o = run("pop", cfg, &s, 1.0);
    let ro = RayBeamOpts {
        rays: 33,
        n: 64,
        ..Default::default()
    };
    let rb = pop::beam_from_rays(&s, FieldSel::Index(0), Some(0), surf, &ro).unwrap();
    let d = 5.0;
    let p = pop::propagate(&rb.beam, d, PopMethod::Auto, OutGrid::default()).unwrap();
    let u = o["u"].as_c64().unwrap();
    assert_eq!(o["u"].shape, vec![p.beam.ny, p.beam.nx]);
    assert!(u
        .iter()
        .zip(&p.beam.u)
        .all(|(a, b)| a.re.to_bits() == b.re.to_bits() && a.im.to_bits() == b.im.to_bits()));
    assert_eq!(get(&o, "distance"), &[d]);
    // ゴースト: 視野を使わない
    let inst = reg().prepare("ghost", &Config::new()).unwrap();
    let o = inst.run(&Context::with_system(&s), &Arrays::new()).unwrap();
    let co = Coatings::for_system(&s).unwrap();
    let v = lensbench_core::ghost::survey(&s, 1, s.image(), &co).unwrap();
    assert_eq!(get(&o, "a").len(), v.len());
    let want: Vec<f64> = v
        .iter()
        .map(|g| g.intensity.as_ref().copied().unwrap_or(f64::NAN))
        .collect();
    assert_eq!(bits(get(&o, "intensity")), bits(&want));
    assert!(inst.run(&Context::with_system(&s), &field(1.0)).is_err());
    assert!(reg().prepare("pop", &Config::new()).is_err());
}

#[test]
fn interferogram_matches_core_with_the_configurations() {
    use lensbench_core::fringe::{self, PupilOpts};
    use lensbench_core::param::{Kind, Param};
    let mut s = (*sys()).clone();
    let im = s.image();
    let t = s.surfaces[im - 1].t;
    s.set_config_count(2).unwrap();
    s.set_config_row(Param::new(im - 1, Kind::Thick), vec![t, t + 0.02])
        .unwrap();
    let s = Arc::new(s);
    let cfg = Config::from([
        ("n".into(), Value::Num(32.0)),
        ("scale".into(), Value::Num(2.0)),
        ("xtilt".into(), Value::Num(1.5)),
    ]);
    let o = run("interferogram", cfg, &s, 2.0);
    let opts = PupilOpts {
        n: 32,
        scale: 2.0,
        xtilt: 1.5,
        ytilt: 0.0,
    };
    let c1 = s.config(1).unwrap();
    let c2 = s.config(2).unwrap();
    let r = fringe::interferogram(&c1, Some(&c2), FieldSel::Index(1), None, &opts).unwrap();
    assert_eq!(bits(get(&o, "delta")), bits(&r.delta.concat()));
    assert_eq!(bits(get(&o, "intensity")), bits(&r.intensity.concat()));
    assert!(r.pv > 0.1, "{}", r.pv);
    // beam2 = 0 は参照球
    let cfg = Config::from([
        ("n".into(), Value::Num(32.0)),
        ("beam2".into(), Value::Num(0.0)),
    ]);
    let o = run("interferogram", cfg, &s, 1.0);
    let w = fringe::opd_grid(&c1, FieldSel::Index(0), None, &fringe::pupil_grid(32)).unwrap();
    assert_eq!(bits(get(&o, "delta")), bits(&w.concat()));
}
