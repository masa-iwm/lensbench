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
    // 非球面の係数は面の more に持つ自作の玉
    let mut asph = lens.clone();
    asph["source"]["surfaces"][0]["asph"] = json!([0.0, 1e-5]);
    let c = with(json!([stop, air, asph, air])).unwrap();
    assert_eq!(c.picks[0].kind, ElemKind::Lens);
    assert_eq!(c.picks[0].surfs[0].more["asph"][1], 1e-5);
    // 玉の中の Jones 面は書けない: ブロックをそのまま持つ要素に（id も残る）
    let mut jones = lens.clone();
    jones["source"]["surfaces"][0]["jones"] = json!([1, 0, 0, 0, 0, 0, 1, 0]);
    let c = with(json!([stop, air, jones, air])).unwrap();
    assert_eq!(c.picks[0].kind, ElemKind::Raw);
    assert_eq!(c.picks[0].id, "A");
    // 前面を絞りにしたブロックは絞り（前の絞りを置かない）
    let mut sasph = jones.clone();
    sasph["stop"] = json!(true);
    assert!(!with(json!([air, sasph, air])).unwrap().lead_stop());
    // 鏡の開口も
    let m =
        json!({ "type": "mirror", "semi": 5, "aperture": { "type": "rect", "hx": 3, "hy": 2 } });
    assert_eq!(
        with(json!([stop, air, m, air])).unwrap().picks[0].kind,
        ElemKind::Raw
    );
    // 分岐のある BS を要素の欄で書けない（口の最初の空気に id）: そのまま持たず、組合せにしない
    let dir = root().join("examples/projects");
    let mut tg: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("P16_twyman_green.json")).unwrap())
            .unwrap();
    assert!(Combo::from_project(&tg, &dir).is_ok());
    tg["blocks"][2]["t"][0]["id"] = json!("arm");
    assert!(Combo::from_project(&tg, &dir).is_err());
    // 入れ子の群は組合せにしない
    let nest = json!({ "type": "group", "id": "G", "blocks": [{ "type": "group", "id": "H", "blocks": [lens] }] });
    assert!(with(json!([stop, air, nest, air])).is_err());
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
    // ほかのブロックの面を指す面の操作
    let mut picked = lens.clone();
    picked["id"] = json!("B");
    picked["surface_ops"] = json!([{ "op": "add_pickup", "dst": 0,
        "src": { "block": "A", "surface": 1 }, "what": "c", "scale": -1 }]);
    assert!(with(json!([stop, air, lens, air, lens, air, picked, air]))
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

/// ブロックをそのまま持つ要素の系は、元のファイルと同じ図になる
#[test]
fn raw_blocks_build_the_same() {
    let r = root();
    let tmp =
        lensbench_core::vfs::temp_dir().join(format!("lensbench-combo-raw-{}", std::process::id()));
    lensbench_core::vfs::create_dir_all(&tmp).unwrap();
    let v = json!({ "wavelengths": [{ "um": 0.55 }], "aperture": { "type": "epd", "value": 4 },
        "fields": { "points": [[0, 0], [0, 2]] }, "blocks": [
        { "type": "stop", "semi": 2 }, { "type": "air", "t": 5 },
        { "type": "lens", "id": "ASPH", "source": { "surfaces": [
            { "radius": 30, "t": 4, "glass": "N-BK7", "semi": 6, "asph": [0, -2e-5] },
            { "radius": -40, "semi": 6 }] }, "tilt": [0.5, 0, 0] },
        { "type": "air", "t": 20 },
        { "type": "mirror", "semi": 8, "tilt": [10, 0], "aperture": { "type": "rect", "hx": 6, "hy": 4 } },
        { "type": "air", "t": 30 }] });
    let c = Combo::from_project(&v, &tmp).unwrap();
    let kinds: Vec<ElemKind> = c.picks.iter().map(|p| p.kind).collect();
    // 非球面の玉は自作の玉、開口のある鏡はそのまま持つ要素
    assert_eq!(kinds, [ElemKind::Lens, ElemKind::Raw]);
    let build = |name: &str, x: &Value| {
        let f = tmp.join(name);
        lensbench_core::vfs::write(&f, x.to_string()).unwrap();
        let load = json!({ "file": f.display().to_string(),
            "glass_dirs": [r.join("tests/glass").display().to_string()] });
        let mut v = lensbench_engine::preview_layout(&load, &json!({})).unwrap();
        v.as_object_mut().map(|o| o.remove("units"));
        v
    };
    let (a, b) = (build("a.json", &v), build("b.json", &c.project()));
    assert!(a == b, "{}", super::first_diff(&a, &b, ""));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// 試験用の .zmx は、像面を傾けた C0・C1 と多重構成のもの以外が組合せになり、組合せから作った系が元と同じ。
/// 玉の表（units）は同じ玉に、要素の id の群（.zmx の全体）が 1 つ加わるだけ
#[test]
fn zmx_files_become_combos() {
    let gd = vec![root().join("tests/glass")];
    let mut files: Vec<_> = std::fs::read_dir(root().join("tests/refs"))
        .unwrap()
        .map(|e| e.unwrap().path().join("system.zmx"))
        .filter(|p| p.exists())
        .collect();
    files.sort();
    let (mut ok, mut no, mut split) = (Vec::new(), Vec::new(), Vec::new());
    for f in &files {
        let name = f
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let c = match Combo::from_zmx(f, &gd, &[]) {
            Ok(c) => c,
            Err(e) => {
                no.push(format!("{name}: {e}"));
                continue;
            }
        };
        // 書ける .zmx は要素に分ける（同じ系かは分けるときに確かめる）
        if !c.picks.iter().any(|p| p.kind == ElemKind::ZmxRange) {
            split.push(name);
            continue;
        }
        assert_eq!(c.picks.len(), 1, "{name}");
        let o = lensbench_project::open::OpenSpec {
            glass_dirs: &gd,
            cfg: 1,
            ..Default::default()
        };
        let s = lensbench_project::open::open(f, &o).unwrap().system;
        let base = f.parent().unwrap();
        let t = lensbench_project::Project::from_json(&c.project().to_string())
            .and_then(|q| {
                lensbench_project::expand(
                    &q,
                    &lensbench_project::Env::in_dir(base, &gd, None),
                    &Default::default(),
                )
            })
            .map(|x| x.system)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        lensbench_project::same_system(&s, &t).unwrap_or_else(|e| panic!("{name}: {e}"));
        let units = |s: &lensbench_core::System| {
            s.units
                .iter()
                .map(|u| (u.name.clone(), u.spans.clone()))
                .collect::<Vec<_>>()
        };
        let (got, want) = (units(&t), units(&s));
        assert_eq!(got[..got.len() - 1], want[..], "{name}");
        assert_eq!(
            got[got.len() - 1].1,
            vec![[1, s.surfaces.len() - 2]],
            "{name}"
        );
        ok.push(name);
    }
    assert!(ok.len() + split.len() >= 30, "{ok:?} {split:?} {no:?}");
    assert!(
        ok.len() >= 10 && split.contains(&"A0".to_string()),
        "{ok:?} {split:?}"
    );
    // 作業フォルダからの相対パスで開いても（GUI の引数）。G0 は分けずに取り込む
    let rel = Path::new("../../tests/refs/G0/system.zmx");
    let c = Combo::from_zmx(rel, &gd, &[]).unwrap();
    assert!(
        Path::new(&c.picks[0].path).is_absolute(),
        "{}",
        c.picks[0].path
    );
    // A0 の GLRS 8 は要素の面で持つ（前に要素を足しても同じ面）。分けた A0 では像の前のダミー面
    let c = Combo::from_zmx(Path::new("../../tests/refs/A0/system.zmx"), &gd, &[]).unwrap();
    assert_eq!(c.extra.glrs, Some(json!({ "block": "S2", "surface": 0 })));
    assert!(
        no.iter()
            .all(|e| e.starts_with("C0") || e.starts_with("C1")),
        "{no:?}"
    );
    for n in ["A0", "A6", "F2"] {
        assert!(ok.iter().chain(&split).any(|x| x == n), "{n}: {no:?}");
    }
}

/// 保存はブロックが元（文書モデル段階 4）: examples の組合せをブロックに書いて開き直すと、2 回目からは変わらない
/// （1 回目は id の無い要素に id が付く・状態の経路が基本と同じなら書かない）
#[test]
fn blocks_are_the_saved_form() {
    let r = root();
    let mut n = 0;
    for dir in ["examples/combos", "examples/projects"] {
        let dir = r.join(dir);
        for e in std::fs::read_dir(&dir).unwrap() {
            let f = e.unwrap().path();
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
            let Ok(c) = Combo::from_project(&v, &dir) else {
                continue;
            };
            let c1 = Combo::from_project(&c.project(), &dir).unwrap();
            let c2 = Combo::from_project(&c1.project(), &dir).unwrap();
            let (a, b) = (
                serde_json::to_value(&c1).unwrap(),
                serde_json::to_value(&c2).unwrap(),
            );
            assert!(a == b, "{}: {}", f.display(), super::first_diff(&a, &b, ""));
            n += 1;
        }
    }
    assert_eq!(n, 21);
}
