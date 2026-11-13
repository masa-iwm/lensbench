//! lens_data の試験（lens_data.rs から分けた）
use super::*;

fn unit(name: &str, spans: &[[usize; 2]], auto: bool) -> Unit {
    Unit {
        name: name.into(),
        spans: spans.to_vec(),
        auto,
        ..Default::default()
    }
}

#[test]
fn tree_nests_by_surfaces_and_hides_auto_lenses_inside_blocks() {
    let units = vec![
        unit("lens1", &[[3, 4]], true),
        unit("lens2", &[[9, 10]], true),
        unit("lens3", &[[12, 13]], true),
        unit("L1", &[[2, 5]], false),
        unit("G", &[[6, 10]], false),
        unit("M1", &[[6, 8]], false),
        unit("L2", &[[9, 10]], false),
    ];
    let t = tree(&units);
    let names: Vec<&str> = t.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["L1", "G", "M1", "L2", "lens3"]);
    let parent = |n: &str| {
        let b = t.iter().find(|b| b.name == n).unwrap();
        b.parent.map(|p| t[p].name.as_str())
    };
    assert_eq!(parent("L1"), None);
    assert_eq!(parent("M1"), Some("G"));
    assert_eq!(parent("L2"), Some("G"));
    assert_eq!(parent("lens3"), None);
    assert_eq!(path_to(&t, "L2").unwrap(), ["G", "L2"]);
    let locked: BTreeSet<String> = ["G".to_string()].into();
    assert_eq!(locked_at(&t, &locked, 9).as_deref(), Some("G"));
    assert_eq!(locked_at(&t, &locked, 3), None);
    assert!(block_locked(&t, &locked, 3));
}

#[test]
fn blocks_fold_to_one_line() {
    // 面 0..12、L1 = 2..5、G = 6..10（往路）と 12..13（復路のつもり）
    let blk = |j: usize| match j {
        2..=5 => Some(0),
        6..=10 | 12..=13 => Some(1),
        _ => None,
    };
    let surfs: Vec<usize> = (0..15).collect();
    let v = fold_lines(&surfs, blk);
    assert_eq!(
        v,
        vec![
            (0, None),
            (1, None),
            (5, Some((0, 2))),
            (10, Some((1, 6))),
            (11, None),
            (13, Some((1, 12))),
            (14, None),
        ]
    );
}

#[test]
fn open_blocks_show_their_surfaces_in_place() {
    // 面 0..12、L1 = 2..5（中に lens = 3..4）、G = 6..10
    let blocks = vec![
        Block {
            name: "L1".into(),
            surfs: (2..=5).collect(),
            parent: None,
        },
        Block {
            name: "lens".into(),
            surfs: vec![3, 4],
            parent: Some(0),
        },
        Block {
            name: "G".into(),
            surfs: (6..=10).collect(),
            parent: None,
        },
    ];
    let surfs: Vec<usize> = (0..12).collect();
    let keys = |v: &[Line]| {
        v.iter()
            .map(|l| (l.key, l.fold.map(|f| f.0)))
            .collect::<Vec<_>>()
    };
    // たたんだまま（階層の表示と同じ）
    let v = lines_of(&blocks, None, &surfs, &|_| false, 0);
    assert_eq!(
        keys(&v),
        [
            (0, None),
            (1, None),
            (5, Some(0)),
            (10, Some(2)),
            (11, None)
        ]
    );
    // L1 を開く: 中の lens はたたんだ 1 行
    let v = lines_of(&blocks, None, &surfs, &|i| i == 0, 0);
    assert_eq!(
        keys(&v),
        [
            (0, None),
            (1, None),
            (2, None),
            (4, Some(1)),
            (5, None),
            (10, Some(2)),
            (11, None)
        ]
    );
    assert_eq!(v[2].heads, [0]);
    assert_eq!((v[2].depth, v[2].blk), (1, Some(0)));
    assert_eq!((v[3].depth, v[3].blk), (1, Some(1)));
    // L1 と lens を開く: 面 3 から lens も始まる
    let v = lines_of(&blocks, None, &surfs, &|i| i <= 1, 0);
    let k: Vec<usize> = v.iter().map(|l| l.key).collect();
    assert_eq!(k, [0, 1, 2, 3, 4, 5, 10, 11]);
    assert_eq!(v[3].heads, [1]);
    assert_eq!((v[3].depth, v[3].blk), (2, Some(1)));
    assert_eq!((v[5].depth, v[5].blk), (1, Some(0)));
    // 先頭の面からいくつものブロックが始まる
    let b2 = vec![
        blocks[0].clone(),
        Block {
            name: "in".into(),
            surfs: vec![2, 3],
            parent: Some(0),
        },
    ];
    let v = lines_of(&b2, None, &surfs, &|_| true, 0);
    assert_eq!(v[2].heads, [0, 1]);
    assert_eq!(run_end(&blocks[2], 7), 10);
    // たたんだ中の面は、たたんだ行へ
    let v = lines_of(&blocks, None, &surfs, &|_| false, 0);
    assert_eq!(visible_key(&v, &blocks, 3), Some(5));
    assert_eq!(visible_key(&v, &blocks, 7), Some(10));
    assert_eq!(visible_key(&v, &blocks, 1), Some(1));
    assert_eq!(visible_key(&v[..2], &blocks, 7), None);
}

#[test]
fn parse_surface_reply() {
    let v = json!({
        "surfaces": [
            {"type": "STANDARD", "comment": "", "r": {"$f": "inf"}, "t": {"$f": "inf"}, "material": "", "coating": "", "semi": 0.0, "k": 0.0, "stop": false},
            {"type": "STANDARD", "comment": "front", "r": 50.0, "t": 4.0, "material": "N-BK7", "coating": "AR", "semi": 10.0, "k": -1.0, "stop": true},
        ],
        "units": [{"name": "lens1", "spans": [[1, 2]], "auto": true}],
    });
    let (rows, units) = parse(&v);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].r.is_infinite());
    assert_eq!(Col::R.text(&rows[0]), "Infinity");
    assert_eq!(rows[1].comment, "front");
    assert!(rows[1].stop);
    assert_eq!(Col::Conic.text(&rows[1]), "-1");
    assert_eq!(units, vec![unit("lens1", &[[1, 2]], true)]);
}

#[test]
fn parse_input() {
    assert_eq!(Col::R.parse("inf").unwrap(), json!(0.0));
    assert_eq!(Col::R.parse("").unwrap(), json!(0.0));
    assert_eq!(Col::R.parse(" -40.5 ").unwrap(), json!(-40.5));
    assert!(Col::T.parse("abc").is_err());
    assert!(Col::T.parse("").is_err());
    assert_eq!(Col::Material.parse(" F2 ").unwrap(), json!("F2"));
}

#[test]
fn element_spans_fill_unnamed_elements() {
    let rows: Vec<Row> = (0..12)
        .map(|j| Row {
            stop: j == 1,
            ..Default::default()
        })
        .collect();
    let units = vec![unit("L1", &[[2, 5]], false), unit("L2", &[[9, 10]], false)];
    let ids: Vec<String> = ["L1", "BS1", "L2"].iter().map(|s| s.to_string()).collect();
    let v = elem_spans(&ids, &units, true, &rows);
    assert_eq!(v, vec![Some([2, 5]), Some([6, 8]), Some([9, 10])]);
}

#[test]
fn type_column_matches_by_prefix_then_description() {
    assert_eq!(type_matches("ev")[0], "EVENASPH");
    assert_eq!(type_matches("co")[0], "COORDBRK");
    assert!(type_matches("非球面").contains(&"EVENASPH"));
    assert_eq!(type_matches("").len(), TYPE_NAMES.len());
    assert!(type_matches("xyz").is_empty());
    assert_eq!(Col::Type.parse("st"), Ok(json!("STANDARD")));
    assert!(Col::Type.parse("xyz").is_err());
}

/// セルの「多重構成の行にする」の名前は core の多重構成の名前として読める
#[test]
fn config_param_names() {
    let cb = Row {
        typ: "COORDBRK".into(),
        ..Default::default()
    };
    let asph = Row {
        typ: "EVENASPH".into(),
        ..Default::default()
    };
    let std = Row {
        typ: "STANDARD".into(),
        ..Default::default()
    };
    let cases = [
        (config_param(&std, 2, Col::R), Some("2.r")),
        (config_param(&std, 2, Col::T), Some("2.t")),
        (config_param(&std, 2, Col::Conic), Some("2.k")),
        (config_param(&std, 2, Col::Material), Some("2.dn")),
        (config_param(&std, 2, Col::Semi), None),
        (config_param(&asph, 3, Col::Par(1)), Some("3.a2")),
        (config_param(&asph, 3, Col::Par(4)), Some("3.a8")),
        (config_param(&cb, 4, Col::Par(3)), Some("4.tx")),
        (config_param(&cb, 4, Col::Par(6)), None),
    ];
    for (got, want) in cases {
        assert_eq!(got.as_deref(), want);
        if let Some(n) = got {
            let k = n.split_once('.').unwrap().1;
            lensbench_core::param::Kind::parse(k).unwrap();
        }
    }
}

#[test]
fn parameters_follow_opticstudio_numbers() {
    let cb = Row {
        typ: "COORDBRK".into(),
        cb: vec![1.0, 2.0, 3.0, 4.0, 5.0],
        cb_order: 1.0,
        ..Default::default()
    };
    assert_eq!(par_count("COORDBRK"), 6);
    assert_eq!(par_value(&cb, 3), Some(3.0));
    assert_eq!(par_value(&cb, 6), Some(1.0));
    assert_eq!(par_value(&cb, 7), None);
    let f = par_fields(&cb, 2, 9.0).unwrap();
    assert_eq!(f["cb"], json!([1.0, 9.0, 3.0, 4.0, 5.0]));
    assert!(par_fields(&cb, 6, 2.0).is_err());
    let asph = Row {
        typ: "EVENASPH".into(),
        asph: vec![0.0, 1e-5],
        ..Default::default()
    };
    assert_eq!(par_count("EVENASPH"), 8);
    assert_eq!(par_value(&asph, 2), Some(1e-5));
    assert_eq!(par_value(&asph, 5), Some(0.0));
    assert_eq!(
        par_fields(&asph, 4, 2e-9).unwrap()["asph"],
        json!([0.0, 1e-5, 0.0, 2e-9])
    );
    let tor = Row {
        typ: "TOROIDAL".into(),
        cx: 0.02,
        ..Default::default()
    };
    assert_eq!(par_value(&tor, 1), Some(50.0));
    assert_eq!(par_fields(&tor, 1, 25.0).unwrap()["cx"], json!(0.04));
    assert_eq!(par_count("STANDARD"), 0);
    assert_eq!(par_count("PARAXIAL"), 1);
    assert_eq!(par_count("JONESMAT"), 8);
}

#[test]
fn commit_only_changed_values() {
    let r = Row {
        typ: "STANDARD".into(),
        r: f64::INFINITY,
        t: 5.0,
        ..Default::default()
    };
    assert_eq!(commit_act(&r, 1, Col::T, "5"), None);
    assert_eq!(commit_act(&r, 1, Col::R, ""), None);
    assert_eq!(
        commit_act(&r, 1, Col::R, "-20"),
        Some(Act::Edit {
            j: 1,
            col: Col::R,
            value: json!(-20.0)
        })
    );
    assert_eq!(
        commit_act(&r, 1, Col::Type, "ev"),
        Some(Act::Edit {
            j: 1,
            col: Col::Type,
            value: json!("EVENASPH")
        })
    );
    assert!(matches!(
        commit_act(&r, 1, Col::T, "x"),
        Some(Act::Refused(_))
    ));
    let cb = Row {
        typ: "COORDBRK".into(),
        cb: vec![0.0; 5],
        ..Default::default()
    };
    assert!(matches!(
        commit_act(&cb, 2, Col::Par(4), "10"),
        Some(Act::Fields { j: 2, .. })
    ));
}

#[test]
fn copy_and_paste_rows_as_tab_separated_text() {
    let rows = vec![
        Row {
            typ: "STANDARD".into(),
            r: f64::INFINITY,
            t: 10.0,
            ..Default::default()
        },
        Row {
            typ: "STANDARD".into(),
            r: 50.0,
            t: 5.0,
            material: "N-BK7".into(),
            semi: 5.0,
            ..Default::default()
        },
        Row {
            typ: "STANDARD".into(),
            r: -50.0,
            t: 40.0,
            semi: 5.0,
            ..Default::default()
        },
        Row {
            typ: "STANDARD".into(),
            r: f64::INFINITY,
            ..Default::default()
        },
    ];
    let cols = Col::all(8);
    let t = copy_text(&rows, &[1, 2], &cols);
    let lines: Vec<&str> = t.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(
        lines[0].starts_with("STANDARD\t\t50\t5\tN-BK7\t\t5\t0"),
        "{}",
        lines[0]
    );
    let ro = |_: usize, _: Col| None;
    // 1 行を別の行へ（Excel の \r\n も）: 違う値だけ変える
    let acts = paste_acts(&rows, &[2], &cols, 0, &format!("{}\r\n", lines[0]), &ro);
    assert_eq!(acts.len(), 1);
    let Act::Fields { j: 2, fields } = &acts[0] else {
        panic!("{acts:?}")
    };
    assert_eq!(fields.len(), 3, "{fields:?}"); // r・t・material
    assert_eq!(fields["material"], json!("N-BK7"));
    // 面を挿入して貼り付け: 2 行なら面 1 の前に 2 面を挿入し、挿入した面 1・2 に値を書く
    let acts = insert_paste_acts(&rows, &[], None, 1, &cols, &t);
    let Act::Ops(ops) = &acts[0] else {
        panic!("{acts:?}")
    };
    assert_eq!(
        ops,
        &vec![
            json!({ "op": "insert_surface", "at": 1, "surface": {} }),
            json!({ "op": "insert_surface", "at": 2, "surface": {} }),
        ]
    );
    let js: Vec<usize> = acts[1..]
        .iter()
        .map(|a| match a {
            Act::Fields { j, fields } => {
                assert!(fields.contains_key("t"), "{fields:?}");
                *j
            }
            _ => panic!("{a:?}"),
        })
        .collect();
    assert_eq!(js, vec![1, 2]);
    // 1 つの値を選んだ範囲の行の今の列に
    let acts = paste_acts(&rows, &[1, 2], &cols, 3, "7", &ro);
    assert_eq!(acts.len(), 2);
    // 種類を変えてからパラメータ（EVENASPH の r⁴）
    let acts = paste_acts(
        &rows,
        &[1],
        &[Col::Type, Col::Par(1), Col::Par(2)],
        0,
        "EVENASPH\t0\t1e-5",
        &ro,
    );
    assert_eq!(
        acts[0],
        Act::Edit {
            j: 1,
            col: Col::Type,
            value: json!("EVENASPH")
        }
    );
    let Act::Fields { fields, .. } = &acts[1] else {
        panic!("{acts:?}")
    };
    assert_eq!(fields["asph"], json!([0.0, 1e-5]));
    // 変えられないセルは飛ばす
    let no = |_: usize, _: Col| Some(String::from("x"));
    assert!(matches!(
        paste_acts(&rows, &[1], &cols, 3, "7", &no)[0],
        Act::Refused(_)
    ));
    // 選択の範囲
    assert_eq!(selection(&[0, 1, 2, 3], Some(3), Some(1)), vec![1, 2, 3]);
    assert_eq!(selection(&[0, 1, 2, 3], None, Some(2)), vec![2]);
}

#[test]
fn inserting_at_the_start_of_the_entered_block_keeps_the_new_surface_inside() {
    let units = vec![
        unit("G12", &[[3, 6], [8, 11]], false),
        unit("G2", &[[3, 4], [10, 11]], false),
    ];
    let b = tree(&units);
    let g12 = b.iter().position(|x| x.name == "G12");
    let ops = grow_blocks(&b, g12, 3, false);
    assert_eq!(
        ops,
        vec![json!({ "op": "set_unit", "unit": "G12", "spans": [[3, 7], [9, 12]] })]
    );
    // 途中の面の前なら範囲はもともと伸びる
    assert!(grow_blocks(&b, g12, 5, false).is_empty());
    // 中のブロック G2 に入っていれば G2 と、同じ所から始まる G12 も
    let g2 = b.iter().position(|x| x.name == "G2");
    assert_eq!(grow_blocks(&b, g2, 3, false).len(), 2);
    assert!(grow_blocks(&b, None, 3, false).is_empty());
    // 最後の面の後に足す（Insert）: 面 6 の後（at 7）なら最初の組が伸びる
    assert_eq!(
        grow_blocks(&b, g12, 7, true),
        vec![json!({ "op": "set_unit", "unit": "G12", "spans": [[3, 7], [9, 12]] })]
    );
    // 途中の面の後なら範囲はもともと伸びる
    assert!(grow_blocks(&b, g12, 5, true).is_empty());
}

/// 鏡の後の戻り（ダブルパス）: 面 4・5 を面 1・2 から逆の順に。曲率はそのまま、間の硝材は元の間、厚さは符号を変える
#[test]
fn range_pickup_return_and_flip() {
    use lensbench_core::edit::pick_kind_parse;
    use lensbench_core::param::Pickup;
    let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS ___BLANK 1 0 1.62 36.4 0 0 0 0 0\n  CONI -0.5\n\
SURF 2\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 10\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ -10\n  GLAS MIRROR 0 0 1.5 40 0 0 0 0 0 0\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ -1\n\
SURF 5\n  TYPE STANDARD\n  CURV 0.0\n  DISZ -20\n\
SURF 6\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let raw = lensbench_core::zmx::parse_text(txt).unwrap();
    let base = lensbench_core::zmx::build(&raw, &Default::default(), &[], None).unwrap();
    let rows: Vec<Row> = base
        .surfaces
        .iter()
        .map(|d| Row {
            typ: "STANDARD".into(),
            material: d.mat.clone(),
            ..Default::default()
        })
        .collect();
    let apply = |mode: RangePick, dst: &[usize]| {
        let mut s = base.clone();
        for op in range_pick_ops(&rows, dst, &[1, 2], mode).unwrap() {
            let w = pick_kind_parse(op["what"].as_str().unwrap()).unwrap();
            let p = Pickup::new(
                op["dst"].as_u64().unwrap() as usize,
                op["src"].as_u64().unwrap() as usize,
                w,
                op["scale"].as_f64().unwrap(),
                0.0,
            );
            s.add_pickup(p).unwrap();
        }
        s
    };
    let s = apply(RangePick::Return, &[4, 5]);
    let d = &s.surfaces;
    assert_eq!((d[4].c, d[5].c), (-0.01, 0.02));
    assert_eq!((d[4].k, d[5].k), (0.0, -0.5));
    assert_eq!(d[4].mat, d[1].mat);
    assert_eq!(d[4].n, d[1].n);
    assert_eq!(d[4].t, -4.0);
    // 戻りの最後の面の後（玉の外）は変えない
    assert_eq!(d[5].t, -20.0);
    // 元の値を変えると追従する
    let mut s2 = s.clone();
    s2.set_material(1, "1.7,30").unwrap();
    assert_eq!(s2.surfaces[4].mat, "1.7,30");
    // 反転: 曲率の符号が変わり、厚さはそのまま
    let s = apply(RangePick::Flip, &[4, 5]);
    assert_eq!((s.surfaces[4].c, s.surfaces[5].c), (0.01, -0.02));
    assert_eq!(s.surfaces[4].t, 4.0);
    // 数が合わない・重なりはエラー
    assert!(range_pick_ops(&rows, &[4], &[1, 2], RangePick::Same).is_err());
    assert!(range_pick_ops(&rows, &[2, 3], &[1, 2], RangePick::Same).is_err());
    let b = vec![Block {
        name: "L1".into(),
        surfs: vec![1, 2],
        parent: None,
    }];
    assert_eq!(range_src("L1", &b, 9).unwrap(), vec![1, 2]);
    assert_eq!(range_src("3-5", &b, 9).unwrap(), vec![3, 4, 5]);
    assert_eq!(range_src("3", &b, 2).unwrap(), vec![3, 4]);
    assert!(range_src("x", &b, 2).is_err());
}
