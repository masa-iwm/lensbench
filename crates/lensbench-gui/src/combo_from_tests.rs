use super::*;

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// examples のプロジェクトを組合せにする（書けるものは同じ系に戻る。書けないものは誤り）
#[test]
fn examples_round_trip() {
    let dir = root().join("examples/projects");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    names.sort();
    let mut ok = Vec::new();
    for f in names {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        let name = f.file_stem().unwrap().to_string_lossy().into_owned();
        match Combo::from_project(&v, &dir) {
            Ok(c) => {
                // 組合せの保存（serde）を通しても同じ
                let c2: Combo = serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
                assert_eq!(c, c2, "{name}");
                ok.push(name);
            }
            Err(e) => eprintln!("{name}: {e}"),
        }
    }
    eprintln!("ok: {ok:?}");
    for n in ["P8_catalog_two", "P2_off_air", "P6_states_focus_group"] {
        assert!(ok.iter().any(|x| x == n), "{n} を組合せにできない");
    }
}

#[test]
fn keeps_ids_and_lead_stop() {
    let dir = root().join("examples/projects");
    let v: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("P6_states_focus_group.json")).unwrap(),
    )
    .unwrap();
    let c = Combo::from_project(&v, &dir).unwrap();
    let ids: Vec<String> = c.ids_all().into_iter().map(|x| x.1).collect();
    assert_eq!(ids, ["L1", "L2", "STO1", "L3"]);
    assert_eq!(c.picks[1].air_id, "d");
    assert_eq!(c.groups[0].state_shift["near"], -0.4);
    assert!(c.picks[3].off_air);
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("P8_catalog_two.json")).unwrap())
            .unwrap();
    let c = Combo::from_project(&v, &dir).unwrap();
    assert!(c.lead_stop());
    assert_eq!((c.epd, c.stop_gap), (8.0, 5.0));
    assert!(c.picks[1].reversed);
}

#[test]
fn refuses_what_it_cannot_write() {
    let base = json!({ "wavelengths": [{ "um": 0.55 }], "aperture": { "type": "epd", "value": 4 },
        "fields": { "points": [[0, 0]] } });
    let with = |blocks: Value| {
        let mut v = base.clone();
        v["blocks"] = blocks;
        Combo::from_project(&v, Path::new("/"))
    };
    let lens = json!({ "type": "lens", "id": "A", "source": { "surfaces": [
        { "radius": 20, "t": 3, "glass": "N-BK7", "semi": 5 }, { "radius": -20, "semi": 5 }] } });
    let stop = json!({ "type": "stop", "semi": 2 });
    let air = json!({ "type": "air", "t": 10 });
    assert!(with(json!([stop, air, lens, air])).is_ok());
    // 非球面の係数は組合せの欄に無い
    let mut asph = lens.clone();
    asph["source"]["surfaces"][0]["asph"] = json!([0.0, 1e-5]);
    assert!(with(json!([stop, air, asph, air])).is_err());
    // 最後の要素の後に空気が無い: 空気 0 と同じ
    assert_eq!(with(json!([stop, air, lens])).unwrap().picks[0].air, 0.0);
    // 先頭の空気（前の絞りが無い系）は組合せの前の空気に
    let mut stopped = lens.clone();
    stopped["stop"] = json!(true);
    assert_eq!(
        with(json!([air, stopped, air])).unwrap().lead_air,
        Some(10.0)
    );
    // 空気が続く
    assert!(with(json!([stop, air, lens, air, air])).is_err());
    // 知らないキー（ふつうの読込と同じく誤り）
    let mut typo = lens.clone();
    typo["reversd"] = json!(true);
    assert!(with(json!([stop, air, typo, air])).is_err());
    let mut v = base.clone();
    v["apertur"] = json!(1);
    v["blocks"] = json!([stop, air, lens, air]);
    assert!(Combo::from_project(&v, Path::new("/")).is_err());
    // 重なった id: 指されていなければ組合せにする（同じカタログの玉 2 つなど）。状態・面の上書き・面の操作・機構部品が
    // 指すなら誤り
    assert!(with(json!([stop, air, lens, air, lens, air])).is_ok());
    let two = |k: &str, x: Value| {
        let mut v = base.clone();
        v["blocks"] = json!([stop, air, lens, air, lens, air]);
        v[k] = x;
        Combo::from_project(&v, Path::new("/"))
    };
    assert!(two(
        "states",
        json!([{ "name": "s", "set": { "A": { "on": false } } }])
    )
    .unwrap_err()
    .contains("two or more"));
    assert!(two(
        "surface_edits",
        json!([{ "block": "A", "surface": 0, "fields": { "conic": -1 } }])
    )
    .unwrap_err()
    .contains("two or more"));
    assert!(two(
        "surface_ops",
        json!([{ "op": "set_stop", "surface": { "block": "A", "surface": 0 } }])
    )
    .unwrap_err()
    .contains("two or more"));
}

/// 組合せにした examples は、元のファイルと同じ図（面の並び・光線）になる。.zmx・カタログの相対パスも読める
#[test]
fn examples_build_the_same() {
    let r = root();
    let dir = r.join("examples/projects");
    let tmp = lensbench_core::vfs::temp_dir()
        .join(format!("lensbench-combo-from-{}", std::process::id()));
    lensbench_core::vfs::create_dir_all(&tmp).unwrap();
    let build = |f: &Path, st: Option<&String>| {
        let mut load = json!({ "file": f.display().to_string(),
            "glass_dirs": [r.join("tests/glass").display().to_string()],
            "lens_catalogs": [r.join("examples/lenses/LB").display().to_string()] });
        if let Some(st) = st {
            load["state"] = json!(st);
        }
        lensbench_engine::preview_layout(&load, &json!({})).map_err(|e| format!("{e:?}"))
    };
    let mut n = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let f = e.unwrap().path();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        let Ok(c) = Combo::from_project(&v, &dir) else {
            continue;
        };
        let g = tmp.join("combo.json");
        lensbench_core::vfs::write(&g, c.project().to_string()).unwrap();
        // 基本と各状態
        for st in std::iter::once(None).chain(c.states.iter().map(Some)) {
            let (a, b) = (build(&f, st), build(&g, st));
            assert!(a.is_ok(), "{} {st:?}: {:?}", f.display(), a);
            // 玉の単位（units）は id の無かった要素に組合せが id を付けるので増える。系の面・図は同じ
            let (mut a, mut b) = (a.unwrap(), b.unwrap());
            for x in [&mut a, &mut b] {
                x.as_object_mut().map(|o| o.remove("units"));
            }
            assert!(
                a == b,
                "{} {st:?}: {}",
                f.display(),
                super::first_diff(&a, &b, "")
            );
        }
        n += 1;
    }
    assert_eq!(n, 18);
    let _ = std::fs::remove_dir_all(&tmp);
}
