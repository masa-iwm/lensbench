//! BS の経路・状態ごとの多重構成の .zmxの照合用の系を書く（tests/bs_configs の基準を作るとき用）
//!
//! cargo run --release -p lensbench-project --example bs_configs -- 出力のフォルダ
//!
//! 形だけの照合なので膜は外す。ガラスのカタログは LBSCH（試験の SCHOTT.AGF を OpticStudio の Glasscat に
//! LBSCH.AGF として入れたもの。OpticStudio の SCHOTT は N-BK7 などの値が試験のものと少し違う）
use lensbench_core::System;
use lensbench_project::{expand, expand_path, paths, Env, Project};
use std::path::{Path, PathBuf};

/// (名前, プロジェクトのファイル（リポジトリの根から）, 状態で並べるか（偽なら BS の経路）)
pub const CASES: [(&str, &str, bool); 8] = [
    ("cube", "tests/splitter/cube_T.json", false),
    ("plate", "tests/splitter/plate_T.json", false),
    ("plate_back", "tests/splitter/plate_back_R.json", false),
    ("tg", "tests/splitter/tg_TR.json", false),
    ("mich", "tests/splitter/mich_TR.json", false),
    ("p16", "examples/projects/P16_twyman_green.json", false),
    ("p6", "examples/projects/P6_states_focus_group.json", true),
    ("p7", "examples/projects/P7_zmx_groups.json", true),
];

/// 構成の名前と系
pub fn systems(root: &Path, file: &str, states: bool) -> (Vec<String>, Vec<System>) {
    let txt = std::fs::read_to_string(root.join(file)).unwrap();
    let p = Project::from_json(&txt).unwrap();
    let base = root.join(file).parent().unwrap().to_path_buf();
    let env = Env {
        base,
        glass_dirs: vec![root.join("tests/glass")],
        catalog: None,
    };
    if states {
        let n: Vec<String> = p.state_names().iter().map(|x| x.to_string()).collect();
        let s = n
            .iter()
            .map(|x| expand(&p.with_state(x).unwrap(), &env).unwrap())
            .collect();
        (n, s)
    } else {
        let n = paths(&p, &env).unwrap();
        let s = n
            .iter()
            .map(|q| expand_path(&p, &env, Some(q)).unwrap().0)
            .collect();
        (n, s)
    }
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = PathBuf::from(std::env::args().nth(1).expect("出力のフォルダ"));
    std::fs::create_dir_all(&out).unwrap();
    let mut list = Vec::new();
    for (name, file, states) in CASES {
        let (names, mut ss) = systems(&root, file, states);
        for s in ss.iter_mut() {
            s.coating_file = None;
            for d in s.surfaces.iter_mut() {
                d.coat.clear();
            }
            for g in s.glasses.values_mut() {
                if g.catalog.eq_ignore_ascii_case("SCHOTT") {
                    g.catalog = "LBSCH".into();
                }
            }
        }
        lensbench_core::zmx_write::save_configs(&out.join(format!("{name}.zmx")), &ss).unwrap();
        println!("{name}: {} 構成 {:?}", ss.len(), names);
        list.push(format!("  \"{name}\": {:?}", names));
    }
    std::fs::write(
        out.join("configs.json"),
        format!("{{\n{}\n}}\n", list.join(",\n")),
    )
    .unwrap();
}
