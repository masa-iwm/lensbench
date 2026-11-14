//! 面からブロックへの逆変換（玉モードの玉・ユーザーカタログへの保存）

use super::*;
/// 玉（玉モードの要素）を、面を直接書いた玉のブロックにする（ほかのプロジェクトへ持ち出せる形）
pub fn lens_to_block(e: &Element, id: &str) -> Result<Block, String> {
    if e.kind != Kind::Lens {
        return Err("玉でない要素".into());
    }
    let n = e.surfaces.len();
    let mut v = Vec::new();
    for (i, s) in e.surfaces.iter().enumerate() {
        let shape = match s.typ {
            SurfType::Standard | SurfType::EvenAsph => Some(SurfShape::Standard),
            SurfType::OddAsph => Some(SurfShape::OddAsph),
            SurfType::Toroidal => Some(SurfShape::Toroidal),
            SurfType::Fresnel => Some(SurfShape::Fresnel),
            _ => None,
        };
        let msg = "標準面・偶数次非球面・Odd Asphere・トロイダル・Fresnel だけ保存できる";
        let shape = shape.ok_or_else(|| format!("玉の面 {}: {msg}", i + 1))?;
        if s.mirror {
            return Err(format!("玉の面 {}: 鏡の面のある玉は保存できない", i + 1));
        }
        let glass = if i + 1 == n || is_air(&s.mat) {
            None
        } else if let Some((nd, vd)) = s.mat.split_once(',') {
            // モデルガラス（.zmx 読込の名前）: 分散なしだけ
            match (nd.trim().parse::<f64>(), vd.trim().parse::<f64>()) {
                (Ok(_), Ok(0.0)) => Some(format!("{}", s.n.first().copied().unwrap_or(0.0))),
                _ => {
                    return Err(format!(
                        "玉の面 {}: 分散のあるモデルガラス {} は保存できない",
                        i + 1,
                        s.mat
                    ))
                }
            }
        } else {
            Some(s.mat.clone())
        };
        v.push(SurfSpec {
            radius: (s.c != 0.0).then(|| 1.0 / s.c),
            conic: s.k,
            asph: if s.typ == SurfType::Standard {
                Vec::new()
            } else {
                s.asph.clone()
            },
            shape,
            radius_x: (shape == SurfShape::Toroidal && s.cx != 0.0).then(|| 1.0 / s.cx),
            t: if i + 1 == n { 0.0 } else { s.t },
            glass,
            // カタログの玉は径が決まっているので固定（自動ならいまの値）
            semi: Semi::Value(s.semi),
            coat: s.coat.clone(),
            comment: s.comment.clone(),
            ..Default::default()
        });
    }
    Ok(Block::Lens {
        id: id.to_string(),
        source: LensSource::Surfaces { surfaces: v },
        reversed: false,
        stop: false,
        place: Place::default(),
    })
}

/// 系の玉 k（1 始まり、玉だけを数える）をユーザーカタログの項目にする
pub fn lens_item(s: &System, k: usize, description: &str) -> Result<UserItem, String> {
    let e = pick_lens(s, k).map_err(|e| e.to_string())?;
    let mut cats: Vec<String> = Vec::new();
    for d in &e.surfaces {
        if let Some(g) = s.glasses.get(&d.mat) {
            if !g.catalog.is_empty() && !cats.contains(&g.catalog) {
                cats.push(g.catalog.clone());
            }
        }
    }
    Ok(UserItem {
        description: description.to_string(),
        glass_catalogs: cats,
        blocks: vec![lens_to_block(&e, "")?],
        ..Default::default()
    })
}

/// プロジェクトのブロック id（入れ子も探す）をユーザーカタログの項目にする。
/// .zmx の玉は面を直接書いた玉に、.zmx の取り込み・ユーザーカタログの参照のパスは絶対パスにする
pub fn block_item(p: &Project, env: &Env, id: &str, description: &str) -> Result<UserItem, String> {
    fn find<'x>(v: &'x [Block], id: &str, out: &mut Vec<&'x Block>) {
        for b in v {
            let bid = match b {
                Block::Lens { id, .. }
                | Block::Air { id, .. }
                | Block::Stop { id, .. }
                | Block::Mirror { id, .. }
                | Block::Surfaces { id, .. }
                | Block::Zmx { id, .. }
                | Block::User { id, .. }
                | Block::Detector(DetectorBlock { id, .. })
                | Block::Splitter { id, .. }
                | Block::Group { id, .. } => id,
            };
            if !id.is_empty() && bid == id {
                out.push(b);
            }
            if let Block::Group { blocks, .. } = b {
                find(blocks, id, out);
            }
            if let Block::Splitter { spec, .. } = b {
                for l in [&spec.t, &spec.r, &spec.d] {
                    find(l, id, out);
                }
            }
        }
    }
    let mut hits = Vec::new();
    find(&p.blocks, id, &mut hits);
    let b = match hits.as_slice() {
        [b] => (*b).clone(),
        [] => return Err(format!("ブロック {id} はない")),
        _ => return Err(format!("id {id} のブロックが 2 つ以上ある")),
    };
    let mut c = Ctx {
        env,
        base: env.base.clone(),
        users: Vec::new(),
        sf: Vec::new(),
        org: Vec::new(),
        zpicks: Vec::new(),
        lead: 0.0,
        parity: 1.0,
        glasses: Default::default(),
        coats: HashMap::new(),
        nstop: 0,
        spans: Vec::new(),
        back_spans: Vec::new(),
        gaps: Vec::new(),
        split: SplitState::default(),
        frames: Vec::new(),
        rev_spans: Vec::new(),
        zenv: Vec::new(),
    };
    let mut cats = p.glass_catalogs.clone();
    let b = portable(&mut c, b, &mut cats)?;
    Ok(UserItem {
        description: description.to_string(),
        glass_catalogs: cats,
        blocks: vec![b],
        ..Default::default()
    })
}

/// プロジェクトの最上位の並びの、id `ids[0]` から `ids[last]` までのブロック（間の空気も含む）をユーザーカタログの
/// 項目にする（GUI の組合せの要素・範囲の保存）。`ids` が 1 つなら [`block_item`] と同じ（入れ子も探す）。
/// 面の表で変えた値（[`Project::surface_edits`]）は中身に焼き込む: 変えた玉は展開した系の面を直接書いた玉にする
/// （玉のほかのブロックの変更は焼き込めないので誤り）
pub fn blocks_item(
    p: &Project,
    env: &Env,
    ids: &[String],
    description: &str,
) -> Result<UserItem, String> {
    let (first, last) = match ids {
        [] => return Err("保存するブロックの id がない".into()),
        [a] => (a, a),
        [a, .., b] => (a, b),
    };
    // 範囲（最上位の並び）
    let top = |id: &str| -> Result<usize, String> {
        let v: Vec<usize> = p
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| !id.is_empty() && block_id(b) == id)
            .map(|(i, _)| i)
            .collect();
        match v.as_slice() {
            [i] => Ok(*i),
            [] => Err(format!("ブロック {id} は最上位の並びにない")),
            _ => Err(format!("id {id} のブロックが 2 つ以上ある")),
        }
    };
    let mut item = if ids.len() == 1 {
        block_item(p, env, first, description)?
    } else {
        let (i, j) = (top(first)?, top(last)?);
        if i > j {
            return Err(format!("{first} は {last} より後ろ"));
        }
        let mut blocks = Vec::new();
        let mut cats = Vec::new();
        for b in &p.blocks[i..=j] {
            let id = block_id(b);
            if id.is_empty() {
                // 空気・絞りなど id の無いブロックはそのまま
                blocks.push(b.clone());
                continue;
            }
            let it = block_item(p, env, id, "")?;
            for c in it.glass_catalogs {
                if !cats.contains(&c) {
                    cats.push(c);
                }
            }
            blocks.extend(it.blocks);
        }
        UserItem {
            description: description.to_string(),
            glass_catalogs: cats,
            blocks,
            ..Default::default()
        }
    };
    // 面の表の変更を焼き込む
    let edited: Vec<&str> = p
        .surface_edits
        .iter()
        .map(|e| e.block.as_str())
        .filter(|b| item_has_id(&item.blocks, b))
        .collect();
    if edited.is_empty() {
        return Ok(item);
    }
    let s = expand(p, env)?;
    for id in edited {
        bake_lens(&s, &mut item, id)?;
    }
    Ok(item)
}

pub(crate) fn item_has_id(blocks: &[Block], id: &str) -> bool {
    blocks.iter().any(|b| {
        block_id(b) == id || matches!(b, Block::Group { blocks, .. } if item_has_id(blocks, id))
    })
}

/// 展開した系 s の玉 id の面（面の表の変更を当てた後）で、項目の玉 id を面を直接書いた玉にする
pub(crate) fn bake_lens(s: &System, item: &mut UserItem, id: &str) -> Result<(), String> {
    fn find<'x>(v: &'x mut [Block], id: &str) -> Option<&'x mut Block> {
        for b in v {
            if block_id(b) == id {
                return Some(b);
            }
            if let Block::Group { blocks, .. } = b {
                if let Some(x) = find(blocks, id) {
                    return Some(x);
                }
            }
        }
        None
    }
    let nope = || {
        format!(
            "ブロック {id}: 面の表で変えた値を焼き込めるのは玉だけ（ほかは変更を戻してから保存）"
        )
    };
    let b = find(&mut item.blocks, id).ok_or_else(nope)?;
    let Block::Lens {
        reversed,
        source,
        place,
        ..
    } = b
    else {
        return Err(nope());
    };
    let u = s
        .units
        .iter()
        .find(|u| u.name == id)
        .ok_or_else(|| format!("ブロック {id} の面が系に無い"))?;
    let [a, z] = u.spans[0];
    if s.surfaces[..a].iter().any(|x| x.mirror) {
        return Err(format!(
            "ブロック {id}: 鏡の後の玉の変更は焼き込めない（未対応）"
        ));
    }
    // 面の表の番号と同じく、座標ブレークを除いた面（偏心・傾きの座標ブレークは place のまま残る）
    let faces: Vec<&Surface> = (a..=z)
        .map(|j| &s.surfaces[j])
        .filter(|x| x.typ != SurfType::CoordBrk)
        .collect();
    let span_cbs = (a..=z)
        .filter(|&j| s.surfaces[j].typ == SurfType::CoordBrk)
        .count();
    let place_cbs = if place.moved() { 2 } else { 0 };
    // 元の面の並び（面を直接書いた玉なら、中の座標ブレークはそのまま残す）
    let orig: Option<Vec<SurfSpec>> = match &*source {
        LensSource::Surfaces { surfaces } => Some(surfaces.clone()),
        _ => None,
    };
    let inner_cbs = orig
        .as_ref()
        .map_or(0, |v| v.iter().filter(|q| q.coordbrk.is_some()).count());
    if span_cbs != place_cbs + inner_cbs || (*reversed && inner_cbs > 0) {
        return Err(format!(
            "ブロック {id}: 中に座標ブレークのある玉の変更は焼き込めない（未対応）"
        ));
    }
    let n = faces.len();
    let mut specs = Vec::with_capacity(n + inner_cbs);
    let mut k = 0;
    let take = |k: &mut usize| -> Result<SurfSpec, String> {
        let f = faces
            .get(*k)
            .ok_or_else(|| format!("ブロック {id}: 面の数が元の玉と違う"))?;
        *k += 1;
        let mut q = surface_spec(f, *k == n).map_err(|e| format!("ブロック {id} の面 {k}: {e}"))?;
        if *k == n {
            q.t = 0.0;
        }
        Ok(q)
    };
    match &orig {
        Some(v) if !*reversed => {
            for q in v {
                if q.coordbrk.is_some() {
                    specs.push(q.clone());
                } else {
                    specs.push(take(&mut k)?);
                }
            }
        }
        _ => {
            for _ in 0..n {
                specs.push(take(&mut k)?);
            }
        }
    }
    if k != n {
        return Err(format!("ブロック {id}: 面の数が元の玉と違う"));
    }
    for d in &faces {
        if let Some(g) = s.glasses.get(&d.mat) {
            if !g.catalog.is_empty() && !item.glass_catalogs.contains(&g.catalog) {
                item.glass_catalogs.push(g.catalog.clone());
            }
        }
    }
    // 展開した系の面はもう反転の後の向き
    *source = LensSource::Surfaces { surfaces: specs };
    *reversed = false;
    Ok(())
}

/// 展開した系の面を、面を直接書いた玉の面（[`SurfSpec`]）に戻す。書けない値（面の前後の傾き・偏心、
/// 標準面・偶数次非球面・Jones 面のほかの型、鏡、ignore、dn・disp、分散のあるモデルガラス）は誤り
pub(crate) fn surface_spec(f: &Surface, last: bool) -> Result<SurfSpec, String> {
    if !matches!(
        f.typ,
        SurfType::Standard | SurfType::EvenAsph | SurfType::Jones
    ) {
        return Err(format!("面の型 {:?} は書けない", f.typ));
    }
    if f.mirror {
        return Err("鏡の面は書けない".into());
    }
    // 頂点のまわりの傾きだけで後で戻す面は tilt で書ける（tilt は tx → ty の順。順序 1 は tx・ty の片方が 0 のときだけ同じ）
    let tilt = match (&f.before, &f.after) {
        (None, After::None) => None,
        (Some(t), After::Reverse)
            if t.d[0] == 0.0
                && t.d[1] == 0.0
                && t.d[4] == 0.0
                && (t.order == 0 || t.d[2] == 0.0 || t.d[3] == 0.0) =>
        {
            Some([t.d[2], t.d[3]])
        }
        _ => return Err("面の前後の傾き・偏心（before・after）は書けない".into()),
    };
    if f.ignore || f.dn != 0.0 || f.disp != 0.0 || f.cx != 0.0 {
        return Err("ignore・dn・disp・cx は書けない".into());
    }
    let glass = if last || is_air(&f.mat) {
        None
    } else if let Some((nd, vd)) = f.mat.split_once(',') {
        match (nd.trim().parse::<f64>(), vd.trim().parse::<f64>()) {
            (Ok(_), Ok(0.0)) => Some(format!("{}", f.n.first().copied().unwrap_or(0.0))),
            _ => return Err(format!("分散のあるモデルガラス {} は書けない", f.mat)),
        }
    } else {
        Some(f.mat.clone())
    };
    let aperture = match f.ap {
        Aperture::None => None,
        Aperture::Circular { min, max, dx, dy } => {
            Some(SurfAperture::Circular { min, max, dx, dy })
        }
        Aperture::Rect { hx, hy, dx, dy } => Some(SurfAperture::Rect { hx, hy, dx, dy }),
        Aperture::Ellipse { hx, hy, dx, dy } => Some(SurfAperture::Ellipse { hx, hy, dx, dy }),
        Aperture::Obsc { min, max, dx, dy } => Some(SurfAperture::Obscuration { min, max, dx, dy }),
        Aperture::Spider {
            width,
            arms,
            dx,
            dy,
        } => Some(SurfAperture::Spider {
            width,
            arms,
            dx,
            dy,
        }),
        Aperture::Floating { .. } => Some(SurfAperture::Floating),
    };
    // 偏光子・波長板の面はその書き方で戻す
    let (mut polarizer, mut waveplate) = (None, None);
    if let (SurfType::Jones, Some(r)) = (f.typ, &f.retarder) {
        // tx = ty は波長板（遅延 0 でも設計波長を残す）、tx > ty の遅延なしは偏光子
        if r.retardance == 0.0 && r.order == 0 && r.tx > r.ty {
            let t = r.tx * r.tx;
            polarizer = Some(PolarizerSpec {
                axis_deg: r.axis_deg,
                extinction_ratio: if r.ty == 0.0 {
                    f64::INFINITY
                } else {
                    t / (r.ty * r.ty)
                },
                transmittance: t,
            });
        } else if r.tx == r.ty {
            // 板は厚さを書いて戻す（1 枚で軸 0 は material の書き方）
            let (material, thickness_mm, plates) = match r.plates.as_slice() {
                [] => (None, None, Vec::new()),
                [p] if p.axis_deg == 0.0 => {
                    (Some(p.material.clone()), Some(p.thickness), Vec::new())
                }
                ps => (
                    None,
                    None,
                    ps.iter()
                        .map(|p| PlateSpec {
                            material: p.material.clone(),
                            thickness_mm: p.thickness,
                            axis_deg: p.axis_deg,
                        })
                        .collect(),
                ),
            };
            waveplate = Some(WaveplateSpec {
                retardance: r.retardance,
                wavelength_um: r.wl0,
                axis_deg: r.axis_deg,
                order: r.order,
                transmittance: r.tx * r.tx,
                material,
                thickness_mm,
                plates,
            });
        }
    }
    let jones =
        (f.typ == SurfType::Jones && polarizer.is_none() && waveplate.is_none()).then_some(f.jones);
    Ok(SurfSpec {
        radius: (f.c != 0.0).then(|| 1.0 / f.c),
        conic: f.k,
        asph: if f.typ == SurfType::EvenAsph {
            f.asph.clone()
        } else {
            Vec::new()
        },
        t: f.t,
        glass,
        semi: Semi::of(f),
        coat: f.coat.clone(),
        comment: f.comment.clone(),
        jones,
        tilt,
        polarizer,
        waveplate,
        aperture,
        birefringent: f.bire.as_ref().map(|b| match b {
            lensbench_core::surface::Bire::In { mode, axis, .. } => BireSpec::In {
                mode: *mode,
                axis: *axis,
            },
            lensbench_core::surface::Bire::Out => BireSpec::Out,
        }),
        ..Default::default()
    })
}

/// ブロックを持ち出せる形に（.zmx の玉は面に、パスは絶対パスに）
pub(crate) fn portable(c: &mut Ctx, b: Block, cats: &mut Vec<String>) -> Result<Block, String> {
    let abs = |c: &Ctx, p: &str| -> String {
        let q = c.base.join(p);
        lensbench_core::vfs::canonicalize(&q)
            .unwrap_or(q)
            .to_string_lossy()
            .into_owned()
    };
    Ok(match b {
        Block::Lens {
            id,
            source: source @ LensSource::Zmx { .. },
            reversed,
            stop,
            place,
        } => {
            let e = c.lens_element(&source)?;
            for d in &e.surfaces {
                if let Some(g) = c.glasses.get(&d.mat) {
                    if !g.catalog.is_empty() && !cats.contains(&g.catalog) {
                        cats.push(g.catalog.clone());
                    }
                }
            }
            let Block::Lens { source, .. } = lens_to_block(&e, &id)? else {
                unreachable!()
            };
            Block::Lens {
                id,
                source,
                reversed,
                stop,
                place,
            }
        }
        Block::Zmx {
            id,
            path,
            from,
            to,
            config,
            stop,
            groups,
            place,
        } => Block::Zmx {
            id,
            path: abs(c, &path),
            from,
            to,
            config,
            stop,
            groups,
            place,
        },
        Block::User {
            id,
            catalog,
            item,
            place,
        } => Block::User {
            id,
            catalog: abs(c, &catalog),
            item,
            place,
        },
        Block::Group {
            id,
            name,
            role,
            shift,
            blocks,
            place,
        } => Block::Group {
            id,
            name,
            role,
            shift,
            blocks: blocks
                .into_iter()
                .map(|x| portable(c, x, cats))
                .collect::<Result<_, _>>()?,
            place,
        },
        Block::Splitter { id, spec, place } => {
            let mut spec = spec;
            for l in [&mut spec.t, &mut spec.r, &mut spec.d] {
                *l = std::mem::take(l)
                    .into_iter()
                    .map(|x| portable(c, x, cats))
                    .collect::<Result<_, _>>()?;
            }
            Block::Splitter { id, spec, place }
        }
        x => x,
    })
}
