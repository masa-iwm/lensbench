//! 系を受け取って中で解析するプラグイン（core の plugin の約束を、解析のクレートから使う）

use lensbench_core::plugin::*;
use lensbench_core::Result;
use std::sync::Arc;

/// 系を受け取って中で追跡するプラグイン（スポットの RMS）
struct SpotRms;
struct SpotRmsInst(usize);

impl Plugin for SpotRms {
    fn name(&self) -> &str {
        "spot_rms"
    }
    fn kind(&self) -> Kind {
        Kind::Analysis
    }
    fn prepare(&self, config: &Config) -> Result<Box<dyn Instance>> {
        let mut r = ConfigReader::new(config);
        let rings = r.num("rings", 6.0)? as usize;
        r.finish()?;
        Ok(Box::new(SpotRmsInst(rings)))
    }
}

impl Instance for SpotRmsInst {
    fn run(&self, ctx: &Context<'_>, inputs: &Arrays) -> Result<Arrays> {
        let s = ctx.system()?;
        let mut r = ArrayReader::new(inputs);
        let (_, f) = r.f64("field")?;
        r.finish()?;
        let sp = lensbench_analysis::analysis::spot(
            s,
            lensbench_core::FieldSel::Index(f[0] as usize),
            lensbench_core::Waves::All,
            self.0,
        )?;
        Ok(Arrays::from([(
            "rms".into(),
            Array::scalar(sp.rms_centroid),
        )]))
    }
}

#[test]
fn plugin_with_system() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = lensbench_formats::zmx::LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let sys =
        lensbench_formats::zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap();
    let sys = Arc::new(sys);
    let inst = SpotRms.prepare(&Config::new()).unwrap();
    let inp: Vec<Arrays> = (0..2)
        .map(|i| Arrays::from([("field".into(), Array::scalar(i as f64))]))
        .collect();
    assert!(inst.run(&Context::default(), &inp[0]).is_err());
    let ctx = Context::with_system(&sys);
    let out = run_many(inst.as_ref(), &ctx, &inp);
    for (i, o) in out.iter().enumerate() {
        let want = lensbench_analysis::analysis::spot(
            &sys,
            lensbench_core::FieldSel::Index(i),
            lensbench_core::Waves::All,
            6,
        )
        .unwrap();
        assert_eq!(
            o.as_ref().unwrap()["rms"].as_f64().unwrap(),
            &[want.rms_centroid]
        );
    }
}
