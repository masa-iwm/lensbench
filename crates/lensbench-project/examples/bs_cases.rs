//! ビームスプリッターの照合の系を .zmx と .json に書く（tests/splitter の基準を作るとき用）
//!
//! cargo run --release -p lensbench-project --example bs_cases -- 出力のフォルダ
use lensbench_project::{expand_path, Env, Project};
use std::path::PathBuf;

pub fn cases() -> Vec<(&'static str, String, &'static str)> {
    let lens = r#"{"type": "lens", "source": {"surfaces": [{"radius": 40, "t": 4, "glass": "N-BK7", "semi": 8}, {"radius": -60, "semi": 8}]}}"#;
    let cube = r#"{"type": "splitter", "id": "BS", "form": "cube", "size": 12, "glass": "N-BK7", "coat": "BS3", "ar": "AR2"}"#;
    let plate = |face: &str| {
        format!(
            r#"{{"type": "splitter", "id": "BS", "form": "plate", "thickness": 3, "glass": "N-BK7", "semi": 10, "angle": [45, 0], "wedge": [0, 0.3], "coat": "BS3", "ar": "AR2", "split_face": "{face}"}}"#
        )
    };
    let arm = r#"{"type": "air", "t": 15}, {"type": "lens", "source": {"surfaces": [{"radius": 30, "t": 3, "glass": "N-BK7", "semi": 8}, {"radius": -30, "semi": 8}]}}, {"type": "air", "t": 25}, {"type": "mirror", "back": true, "radius": -60, "semi": 12}"#;
    let mich = format!(
        r#"{{"type": "splitter", "id": "BS", "form": "cube", "size": 12, "glass": "N-BK7", "coat": "BS3", "ar": "AR2",
      "t": [{{"type": "air", "t": 30}}, {{"type": "mirror", "back": true, "tilt": [0.3, 0], "semi": 12}}],
      "r": [{{"type": "air", "t": 33}}, {{"type": "mirror", "back": true, "semi": 12}}],
      "d": [{{"type": "air", "t": 25}}, {lens}, {{"type": "air", "t": 50}}]}}"#
    );
    let one = |bs: &str| {
        format!(
            r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}}, {bs}, {{"type": "air", "t": 20}}, {lens}, {{"type": "air", "t": 60}}]"#
        )
    };
    vec![
        ("cube_T", one(cube), "T"),
        ("cube_R", one(cube), "R"),
        ("plate_T", one(&plate("front")), "T"),
        ("plate_R", one(&plate("front")), "R"),
        ("plate_back_R", one(&plate("back")), "R"),
        (
            "tg_TR",
            format!(
                r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}}, {cube}, {arm}, {{"type": "air", "t": 30}}]"#
            ),
            "TR",
        ),
        (
            "mich_TR",
            format!(r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}}, {mich}]"#),
            "TR",
        ),
        (
            "mich_RT",
            format!(r#"[{{"type": "stop", "semi": 3}}, {{"type": "air", "t": 10}}, {mich}]"#),
            "RT",
        ),
    ]
}

pub const HEAD: &str = r#""glass_catalogs": ["SCHOTT"],
  "wavelengths": [{"um": 0.4861327}, {"um": 0.5875618}, {"um": 0.6562725}], "primary": 2,
  "aperture": {"type": "na", "value": 0.05},
  "fields": {"type": "object_height", "points": [[0, 0], [0, 1.5], [1, -1]]},
  "object_distance": 50,"#;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = PathBuf::from(std::env::args().nth(1).expect("出力のフォルダ"));
    let env = Env {
        base: root.clone(),
        glass_dirs: vec![root.join("tests/glass")],
        catalog: None,
    };
    std::fs::create_dir_all(&out).unwrap();
    for (name, blocks, path) in cases() {
        let txt = format!(
            "{{ {HEAD} \"name\": \"{name}\", \"path\": \"{path}\", \"blocks\": {blocks} }}"
        );
        std::fs::write(out.join(format!("{name}.json")), &txt).unwrap();
        let p = Project::from_json(&txt).unwrap();
        let (mut s, _) = expand_path(&p, &env, None).unwrap();
        s.coating_file = Some(PathBuf::from("LBBS.DAT"));
        // OpticStudio の順次の系には屈折面での反射がないので、反射の面は膜なしの鏡で書く（形だけの照合）
        for d in s.surfaces.iter_mut() {
            if d.reflect.is_some() {
                d.coat.clear();
            }
        }
        lensbench_core::zmx_write::save(&out.join(format!("{name}.zmx")), &s).unwrap();
        println!("{name}: {} surfaces", s.surfaces.len());
    }
}
