//! 全部けられる視野がある系（P19 の 20°、機構部品でけられる）: 視野ごとの解析はその視野だけを注意にして飛ばし、
//! ほかの視野の結果を出す。その視野だけを頼めば誤り

use std::process::Command;

fn run(args: &[&str]) -> (bool, String, String) {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let o = Command::new(env!("CARGO_BIN_EXE_lensbench"))
        .current_dir(root)
        .args(args)
        .env_remove("LENSBENCH_GLASSCAT")
        .output()
        .unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

const P19: &str = "examples/projects/P19_tube_cage.json";

#[test]
fn analyses_skip_a_fully_vignetted_field() {
    for (cmd, key, extra) in [
        ("mtf", "mtf", &["--freq-steps", "2"][..]),
        (
            "geo-mtf",
            "geometric_mtf",
            &["--freq-steps", "2", "--n", "32"][..],
        ),
        (
            "huygens-mtf",
            "mtf",
            &["--image", "32", "--freq-steps", "2"][..],
        ),
        ("psf", "sampling", &["--image", "32"][..]),
        ("wavefront", "wavefront", &["--n", "16"][..]),
        ("zernike", "zernike", &["--n", "16", "--terms", "9"][..]),
    ] {
        let mut a = vec![cmd, P19, "--glass-dir", "tests/glass", "--json"];
        a.extend_from_slice(extra);
        let (ok, out, err) = run(&a);
        assert!(ok, "{cmd}: {err}");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let fields: Vec<u64> = v[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["field"].as_u64().unwrap())
            .collect();
        assert!(
            !fields.is_empty() && !fields.contains(&3),
            "{cmd}: {fields:?}"
        );
        assert!(fields.contains(&1), "{cmd}: {fields:?}");
        let w = v["warnings"].as_array().unwrap();
        assert!(
            w.iter()
                .any(|x| x.as_str().unwrap().starts_with("field 3: ")
                    && x.as_str().unwrap().contains("vignetted")),
            "{cmd}: {w:?}"
        );
        assert!(err.contains("field 3: "), "{cmd}: {err}");
        // その視野だけなら誤り（符号は system.all_vignetted）
        let mut a = vec![
            cmd,
            P19,
            "--glass-dir",
            "tests/glass",
            "--json",
            "--field",
            "3",
        ];
        a.extend_from_slice(extra);
        let (ok, _, err) = run(&a);
        assert!(!ok && err.contains("vignetted"), "{cmd}: {err}");
    }
}
