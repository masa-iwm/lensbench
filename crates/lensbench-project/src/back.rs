//! 面からブロックへの逆変換（玉モードの玉・ユーザーカタログへの保存）

use super::*;
/// 玉（玉モードの要素）を、面を直接書いた玉のブロックにする（ほかのプロジェクトへ持ち出せる形）
pub fn lens_to_block(e: &Element, id: &str) -> lensbench_core::Result<Block> {
    if e.kind != Kind::Lens {
        return Err("not a lens element".into());
    }
    let n = e.surfaces.len();
    let mut v = Vec::new();
    for (i, s) in e.surfaces.iter().enumerate() {
        let last = i + 1 == n;
        let mut q = surface_spec(s, last)
            .map_err(|x| x.map_message(|e| format!("lens surface {}: {e}", i + 1)))?;
        if q.coordbrk.is_some() || s.typ() == SurfType::Jones {
            return Err(format!(
                "lens surface {}: only standard, even asphere, Odd Asphere, toroidal and Fresnel surfaces can be saved",
                i + 1
            )
            .into());
        }
        if last {
            q.t = 0.0;
        }
        // カタログの玉は径が決まっているので固定（自動ならいまの値）
        q.semi = Semi::Value(s.semi);
        v.push(q);
    }
    Ok(Block::Lens {
        id: id.to_string(),
        source: LensSource::Surfaces { surfaces: v },
        reversed: false,
        stop: false,
        place: Place::default(),
        faces: FaceMods::default(),
    })
}

/// 系の玉 k（1 始まり、玉だけを数える）をユーザーカタログの項目にする
pub fn lens_item(s: &System, k: usize, description: &str) -> lensbench_core::Result<UserItem> {
    let e = pick_lens(s, k)?;
    let mut cats: Vec<String> = Vec::new();
    for d in &e.surfaces {
        if let Some(g) = d.mat.glass().and_then(|g| s.glasses.get(g)) {
            if !g.catalog.is_empty() && !cats.contains(&g.catalog) {
                cats.push(g.catalog.clone());
            }
        }
    }
    Ok(UserItem::optic(
        description,
        cats,
        vec![lens_to_block(&e, "")?],
    ))
}

/// プロジェクトのブロック id（入れ子も探す）をユーザーカタログの項目にする。
/// .zmx の玉は面を直接書いた玉に、.zmx の取り込み・ユーザーカタログの参照のパスは絶対パスにする
pub fn block_item(
    p: &Project,
    env: &Env,
    id: &str,
    description: &str,
) -> lensbench_core::Result<UserItem> {
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
        [] => {
            return Err(lensbench_core::Error::Invalid(format!("no block {id}"))
                .with_code(lensbench_core::error::code::BLOCK_NOT_FOUND))
        }
        _ => return Err(format!("two or more blocks with id {id}").into()),
    };
    let mut c = Ctx {
        env,
        cache: None,
        base: env.base.clone(),
        loading: Vec::new(),
        outer: 0,
        sf: Vec::new(),
        map: Vec::new(),
        noted: Vec::new(),
        nflip: 0,
        flip_err: None,
        zpicks: Vec::new(),
        lead: 0.0,
        parity: 1.0,
        glasses: Default::default(),
        coats: HashMap::new(),
        nstop: 0,
        gaps: Vec::new(),
        split: SplitState::default(),
        turn: Default::default(),
        media: HashMap::new(),
        frames: Vec::new(),
        zenv: Vec::new(),
        nodes: HashMap::new(),
        leaf: Vec::new(),
        failed: None,
        notes: Vec::new(),
    };
    let mut cats = p.glass_catalogs.clone();
    let b = portable(&mut c, b, &mut cats)?;
    if let Some(e) = c.flip_err.take() {
        return Err(e);
    }
    Ok(UserItem::optic(description, cats, vec![b]))
}

/// プロジェクトの最上位の並びの、id `ids[0]` から `ids[last]` までのブロック（間の空気も含む）をユーザーカタログの
/// 項目にする（GUI の組合せの要素・範囲の保存）。`ids` が 1 つなら [`block_item`] と同じ（入れ子も探す）。
/// ブロックの面の表で変えた値（[`FaceMods::surface_edits`]）は中身に焼き込む: 変えた玉は展開した系の面を直接書いた玉にする
/// （玉のほかのブロックの変更は焼き込めないので誤り）
pub fn blocks_item(
    p: &Project,
    env: &Env,
    ids: &[String],
    description: &str,
) -> lensbench_core::Result<UserItem> {
    let (first, last) = match ids {
        [] => return Err("no block id to save".into()),
        [a] => (a, a),
        [a, .., b] => (a, b),
    };
    // 範囲（最上位の並び）
    let top = |id: &str| -> lensbench_core::Result<usize> {
        let v: Vec<usize> = p
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| !id.is_empty() && block_id(b) == id)
            .map(|(i, _)| i)
            .collect();
        match v.as_slice() {
            [i] => Ok(*i),
            [] => Err(format!("block {id} is not in the top-level list").into()),
            _ => Err(format!("two or more blocks with id {id}").into()),
        }
    };
    let mut item = if ids.len() == 1 {
        block_item(p, env, first, description)?
    } else {
        let (i, j) = (top(first)?, top(last)?);
        if i > j {
            return Err(format!("{first} is after {last}").into());
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
            for c in &it.glass_catalogs {
                if !cats.contains(c) {
                    cats.push(c.clone());
                }
            }
            blocks.extend(it.blocks().iter().cloned());
        }
        UserItem::optic(description, cats, blocks)
    };
    // 面の表の変更を焼き込む（操作は持ち出さない）
    let mut edited = Vec::new();
    if let Some(b) = item.blocks_mut() {
        take_faces(b, &mut edited)?;
    }
    if edited.is_empty() {
        return Ok(item);
    }
    let s = expand(p, env, &ExpandOpts::default())?.system;
    for id in edited {
        bake_lens(&s, &mut item, &id)?;
    }
    Ok(item)
}

/// 項目のブロックの面の上書き・操作を外し、上書きのあったブロックの id を集める
fn take_faces(blocks: &mut [Block], edited: &mut Vec<String>) -> lensbench_core::Result<()> {
    for b in blocks {
        let id = block_id(b).to_string();
        if let Some(f) = b.faces_mut() {
            if !f.surface_edits.is_empty() {
                if id.is_empty() {
                    return Err(
                        "a block without an id can't bake in values changed in the surface table"
                            .into(),
                    );
                }
                edited.push(id);
            }
            *f = FaceMods::default();
        }
        if let Block::Group { blocks, .. } = b {
            take_faces(blocks, edited)?;
        }
    }
    Ok(())
}

/// 展開した系 s の玉 id の面（面の表の変更を当てた後）で、項目の玉 id を面を直接書いた玉にする
pub(crate) fn bake_lens(s: &System, item: &mut UserItem, id: &str) -> lensbench_core::Result<()> {
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
            "block {id}: only lenses can bake in values changed in the surface table (undo the changes on others before saving)"
        )
    };
    let UserItem {
        body,
        glass_catalogs,
        ..
    } = item;
    let b = match body {
        ItemBody::Optic { blocks, .. } => find(blocks, id),
        _ => None,
    }
    .ok_or_else(nope)?;
    let Block::Lens {
        reversed,
        source,
        place,
        ..
    } = b
    else {
        return Err(lensbench_core::Error::Invalid(nope()));
    };
    let u = s
        .units
        .iter()
        .find(|u| u.name == id)
        .ok_or_else(|| format!("the surfaces of block {id} are not in the system"))?;
    let [a, z] = u.spans[0];
    if s.surfaces[..a].iter().any(|x| x.is_mirror()) {
        return Err(format!(
            "block {id}: changes to a lens after a mirror can't be baked in (not supported)"
        )
        .into());
    }
    // 面の表の番号と同じく、座標ブレークを除いた面（偏心・傾きの座標ブレークは place のまま残る）
    let faces: Vec<&Surface> = (a..=z)
        .map(|j| &s.surfaces[j])
        .filter(|x| !x.is_coord_break())
        .collect();
    let span_cbs = (a..=z).filter(|&j| s.surfaces[j].is_coord_break()).count();
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
            "block {id}: changes to a lens with a coordinate break inside can't be baked in (not supported)"
        ).into());
    }
    let n = faces.len();
    let mut specs = Vec::with_capacity(n + inner_cbs);
    let mut k = 0;
    let take = |k: &mut usize| -> lensbench_core::Result<SurfSpec> {
        let f = faces.get(*k).ok_or_else(|| {
            format!("block {id}: the number of surfaces differs from the original lens")
        })?;
        *k += 1;
        let mut q = surface_spec(f, *k == n).map_err(|e| format!("block {id} surface {k}: {e}"))?;
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
        return Err(
            format!("block {id}: the number of surfaces differs from the original lens").into(),
        );
    }
    for d in &faces {
        if let Some(g) = d.mat.glass().and_then(|g| s.glasses.get(g)) {
            if !g.catalog.is_empty() && !glass_catalogs.contains(&g.catalog) {
                glass_catalogs.push(g.catalog.clone());
            }
        }
    }
    // 展開した系の面はもう反転の後の向き
    *source = LensSource::Surfaces { surfaces: specs };
    *reversed = false;
    Ok(())
}

/// 面を前後反転できるか（[`lensbench_core::elements::reversible`]。読めない面は false）。GUI が玉の反転を
/// 出すかを決める
pub fn spec_reversible(q: &SurfSpec) -> bool {
    crate::expand::surf(q).is_ok_and(|s| lensbench_core::elements::reversible(&s))
}

/// 展開した系の面を、面を直接書いた玉の面（[`SurfSpec`]）に戻す。[`surf`] が作る面はすべて書け、
/// `surface_spec` → `surf` で同じ面に戻る。欄で書けない面は行（[`SurfSpec::row`]）で書く。書けない値
/// （面の前後の傾き・偏心、鏡、ゴーストの反射）は誤り
pub fn surface_spec(f: &Surface, last: bool) -> lensbench_core::Result<SurfSpec> {
    surface_fields(f, last).or_else(|e| row_spec(f, last).map_err(|_| e))
}

/// ほかの欄で書けない面を、システムファイル（版 2）の面の行で（[`SurfSpec::row`]）。鏡（ブロックの鏡で書く）・
/// ゴーストの反射・座標ブレークは書かない。面の前後の傾き偏心は行に書く（その玉は裏返せず、復路も通れない）
fn row_spec(f: &Surface, last: bool) -> lensbench_core::Result<SurfSpec> {
    if f.is_mirror() || f.reflect.is_some() || f.typ() == SurfType::CoordBrk {
        return Err("this surface can't be written as a row".into());
    }
    let mut r = crate::sysfile_v2::write::surface_row(f, false)?;
    r.stop = false;
    if last {
        r.t = 0.0;
        r.medium = None;
        r.dn = 0.0;
        r.disp = 0.0;
        r.dnd = 0.0;
        r.dvd = 0.0;
    }
    Ok(SurfSpec {
        row: Some(Box::new(r)),
        ..Default::default()
    })
}

/// 玉の中の鏡（裏面鏡・プリズムの反射）を、玉の面の行（[`SurfSpec::row`]）に。厚さ（鏡の後の向きで、次の面まで）は書く。
/// 絞りの印は書かない。座標ブレーク・ゴーストの反射は書けない
pub fn lens_mirror_spec(f: &Surface) -> lensbench_core::Result<SurfSpec> {
    if !f.is_mirror() || f.reflect.is_some() || f.typ() == SurfType::CoordBrk {
        return Err("this surface can't be written as a mirror in a lens".into());
    }
    let mut r = crate::sysfile_v2::write::surface_row(f, false)?;
    r.stop = false;
    Ok(SurfSpec {
        row: Some(Box::new(r)),
        ..Default::default()
    })
}

/// 鏡の面を、鏡のブロックの行（[`Block::Mirror`] の row）に。厚さ・絞りの印は書かない。面の前後の傾き偏心
/// （before・after）は行に書く。座標ブレーク・ゴーストの反射は書けない
pub fn mirror_row(f: &Surface) -> lensbench_core::Result<crate::sysfile_v2::Row> {
    if !f.is_mirror() || f.reflect.is_some() || f.typ() == SurfType::CoordBrk {
        return Err("this surface can't be written as a mirror row".into());
    }
    let mut r = crate::sysfile_v2::write::surface_row(f, false)?;
    r.stop = false;
    r.t = 0.0;
    Ok(r)
}

/// 面を SurfSpec の欄で（書けなければ誤り）
fn surface_fields(f: &Surface, last: bool) -> lensbench_core::Result<SurfSpec> {
    let shape = match f.typ() {
        SurfType::Standard | SurfType::EvenAsph | SurfType::Jones | SurfType::CoordBrk => {
            SurfShape::Standard
        }
        SurfType::OddAsph => SurfShape::OddAsph,
        SurfType::Toroidal => SurfShape::Toroidal,
        SurfType::Fresnel => SurfShape::Fresnel,
        t => return Err(format!("surface type {:?} can't be written", t).into()),
    };
    if f.is_mirror() {
        return Err("a mirror surface can't be written".into());
    }
    if f.dn != 0.0 || f.disp != 0.0 || f.dnd != 0.0 || f.dvd != 0.0 {
        return Err("dn, disp and the glass nd and vd offsets can't be written".into());
    }
    if f.cx() != 0.0 && shape != SurfShape::Toroidal {
        return Err("cx is only for a toroidal surface".into());
    }
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
    if f.typ() == SurfType::CoordBrk {
        if f.before.is_some() || f.after != After::None {
            return Err(
                "tilts and decenters before or after a surface (before, after) can't be written"
                    .into(),
            );
        }
        return Ok(SurfSpec {
            t: f.t,
            semi: Semi::of(f),
            coat: f.coat.clone(),
            comment: f.comment.clone(),
            coordbrk: Some(f.cb()),
            order: f.cb_order(),
            aperture,
            ignore: f.ignore,
            ..Default::default()
        });
    }
    // 頂点のまわりの傾きだけで後で戻す面は tilt で書ける（tilt は tx → ty の順。順序 1 は tx・ty の片方が 0 のときだけ同じ）
    let tilt =
        match (&f.before, &f.after) {
            (None, After::None) => None,
            (Some(t), After::Reverse)
                if t.d[0] == 0.0
                    && t.d[1] == 0.0
                    && t.d[4] == 0.0
                    && (t.order == 0 || t.d[2] == 0.0 || t.d[3] == 0.0) =>
            {
                Some([t.d[2], t.d[3]])
            }
            _ => return Err(
                "tilts and decenters before or after a surface (before, after) can't be written"
                    .into(),
            ),
        };
    let glass = if last {
        None
    } else {
        match &f.mat {
            Material::Air => None,
            Material::Index(nd) => Some(format!("{nd}")),
            m => Some(m.name()),
        }
    };
    // 偏光子・波長板の面はその書き方で戻す
    let (mut polarizer, mut waveplate) = (None, None);
    if let (SurfType::Jones, Some(r)) = (f.typ(), &f.retarder) {
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
    let jones = (f.typ() == SurfType::Jones && polarizer.is_none() && waveplate.is_none())
        .then_some(f.jones());
    Ok(SurfSpec {
        radius: (f.c != 0.0).then(|| 1.0 / f.c),
        conic: f.k,
        asph: if matches!(f.typ(), SurfType::Standard | SurfType::Jones) {
            Vec::new()
        } else {
            f.asph().to_vec()
        },
        shape,
        radius_x: (f.cx() != 0.0).then(|| 1.0 / f.cx()),
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
        ignore: f.ignore,
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
pub(crate) fn portable(
    c: &mut Ctx,
    b: Block,
    cats: &mut Vec<String>,
) -> lensbench_core::Result<Block> {
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
            faces,
        } => {
            let e = c.lens_element(&source)?;
            for d in &e.surfaces {
                if let Some(g) = d.mat.glass().and_then(|g| c.glasses.get(g)) {
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
                faces,
            }
        }
        Block::Zmx {
            id,
            path,
            from,
            to,
            config,
            stop,
            state,
            beam_path,
            groups,
            place,
            faces,
        } => Block::Zmx {
            id,
            path: abs(c, &path),
            from,
            to,
            config,
            stop,
            state,
            beam_path,
            groups,
            place,
            faces,
        },
        Block::User {
            id,
            catalog,
            item,
            place,
            faces,
        } => Block::User {
            id,
            catalog: abs(c, &catalog),
            item,
            place,
            faces,
        },
        Block::Group {
            id,
            name,
            role,
            shift,
            blocks,
            place,
            faces,
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
            faces,
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
