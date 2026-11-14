//! 展開の入口（状態・経路）と、面の並びのあとの仕上げ（機構部品の配置・面の操作と編集・群の移動・ピックアップの保持）

use super::*;
use lensbench_core::glass::DIRS_HINT as HINT;
/// .zmx の取り込みの群の、要素の範囲 (群の番号, 最初, 最後)。最初・最後は面が from〜to に始まる要素（空気でない）
pub(crate) fn zmx_group_spans(
    keep: &[Element],
    groups: &[ZmxGroup],
) -> Result<Vec<(usize, usize, usize)>, String> {
    let mut out: Vec<(usize, usize, usize)> = Vec::new();
    for (g, gr) in groups.iter().enumerate() {
        let label = if gr.id.is_empty() { &gr.name } else { &gr.id };
        if gr.from > gr.to {
            return Err(format!("群 {label}: 面 {}〜{} の順が逆", gr.from, gr.to));
        }
        let inside: Vec<usize> = (0..keep.len())
            .filter(|&i| keep[i].kind != Kind::Air && (gr.from..=gr.to).contains(&keep[i].from))
            .collect();
        let (Some(&a), Some(&z)) = (inside.first(), inside.last()) else {
            return Err(format!(
                "群 {label}: 面 {}〜{} に取り込んだ玉・面がない",
                gr.from, gr.to
            ));
        };
        if let Some(o) = out.iter().find(|o| a <= o.2 && o.1 <= z) {
            return Err(format!("群 {label} が群 {} と重なる", groups[o.0].name));
        }
        out.push((g, a, z));
    }
    Ok(out)
}

/// 前の座標ブレーク（順序 0 で偏心 d・傾き t）で傾けた座標の z に l 進んだ点から、元の座標の z に l 進んだ点へ戻る
/// 座標ブレーク（順序 0 で −t、偏心 (dx2, dy2)、厚さ t2）の値: (dx2, dy2, t2) = l e_z − d − R e_z l（R = Rx Ry Rz）
pub(crate) fn cb_return(place: &Place, l: f64) -> (f64, f64, f64) {
    let [dx, dy] = place.decenter;
    let [tx, ty, tz] = place.tilt;
    let r: M3 = lensbench_core::math::mm(
        &lensbench_core::math::mm(&rot(0, tx), &rot(1, ty)),
        &rot(2, tz),
    );
    let rz = mv(&r, [0.0, 0.0, l]);
    (-dx - rz[0], -dy - rz[1], l - rz[2])
}

/// 空気中の面の並びを逆向きに（厚さは 1 つ前の面のもの、曲率の符号を変える）。最後の面の厚さは 0
pub(crate) fn reverse_surfaces(v: &[Surface]) -> Vec<Surface> {
    let n = v.len();
    (0..n)
        .map(|i| {
            let mut s = v[n - 1 - i].clone();
            s.c = -s.c;
            s.asph.iter_mut().for_each(|a| *a = -*a);
            s.t = if i + 1 < n { v[n - 2 - i].t } else { 0.0 };
            s.stop = false;
            s
        })
        .collect()
}

pub(crate) fn curv(r: Option<f64>) -> f64 {
    match r {
        Some(r) if r != 0.0 && r.is_finite() => 1.0 / r,
        _ => 0.0,
    }
}

/// 面の開口（JSON）を面の開口に。浮動の開口は半径 semi
pub(crate) fn surf_ap(ap: Option<SurfAperture>, semi: f64) -> Aperture {
    match ap {
        None => Aperture::None,
        Some(SurfAperture::Circular { min, max, dx, dy }) => {
            Aperture::Circular { min, max, dx, dy }
        }
        Some(SurfAperture::Rect { hx, hy, dx, dy }) => Aperture::Rect { hx, hy, dx, dy },
        Some(SurfAperture::Ellipse { hx, hy, dx, dy }) => Aperture::Ellipse { hx, hy, dx, dy },
        Some(SurfAperture::Obscuration { min, max, dx, dy }) => Aperture::Obsc { min, max, dx, dy },
        Some(SurfAperture::Spider {
            width,
            arms,
            dx,
            dy,
        }) => Aperture::Spider {
            width,
            arms,
            dx,
            dy,
        },
        Some(SurfAperture::Floating) => Aperture::Floating { semi },
    }
}

pub(crate) fn surf(q: &SurfSpec) -> Result<Surface, String> {
    let mut s = Surface {
        typ: match q.shape {
            SurfShape::Standard if q.asph.iter().any(|a| *a != 0.0) => SurfType::EvenAsph,
            SurfShape::Standard => SurfType::Standard,
            SurfShape::OddAsph => SurfType::OddAsph,
            SurfShape::Toroidal => SurfType::Toroidal,
            SurfShape::Fresnel => SurfType::Fresnel,
        },
        cx: if q.shape == SurfShape::Toroidal {
            curv(q.radius_x)
        } else {
            0.0
        },
        c: curv(q.radius),
        k: q.conic,
        asph: q.asph.clone(),
        t: q.t,
        semi: q.semi.value(),
        semi_auto: q.semi.is_auto(),
        coat: q.coat.clone(),
        comment: q.comment.clone(),
        ..Default::default()
    };
    s.ap = surf_ap(q.aperture, q.semi.value());
    let special = q.jones.is_some() as u8
        + q.coordbrk.is_some() as u8
        + q.polarizer.is_some() as u8
        + q.waveplate.is_some() as u8
        + q.birefringent.is_some() as u8;
    if special > 1 {
        return Err(
            "面に jones・polarizer・waveplate・coordbrk・birefringent のうち 2 つ以上は書けない"
                .into(),
        );
    }
    if let Some(b) = q.birefringent {
        s.bire = Some(match b {
            BireSpec::In { mode, axis } => {
                if !matches!(mode, 0 | 2) {
                    return Err(format!("複屈折材の板の mode {mode}（0 か 2）"));
                }
                if !(axis.iter().all(|v| v.is_finite()) && axis.iter().any(|v| *v != 0.0)) {
                    return Err("複屈折材の板の結晶軸 axis が 0 か数でない".into());
                }
                if q.glass.as_deref().is_none_or(|g| is_air(g.trim())) {
                    return Err("複屈折材の板の入口の面には glass（常光の材料）を書く".into());
                }
                lensbench_core::surface::Bire::In {
                    mode,
                    axis,
                    ne: Vec::new(),
                }
            }
            BireSpec::Out => lensbench_core::surface::Bire::Out,
        });
    }
    if special == 1 && q.birefringent.is_none() {
        let what = if q.jones.is_some() {
            "jones"
        } else if q.polarizer.is_some() {
            "polarizer"
        } else if q.waveplate.is_some() {
            "waveplate"
        } else {
            "coordbrk"
        };
        if curv(q.radius) != 0.0
            || q.conic != 0.0
            || s.typ == SurfType::EvenAsph
            || q.shape != SurfShape::Standard
        {
            return Err(format!("{what} の面に曲率・円錐係数・非球面は書けない"));
        }
        if q.glass.as_deref().is_some_and(|g| !is_air(g.trim())) {
            return Err(format!("{what} の面の後にガラスは書けない"));
        }
    }
    if let Some(j) = q.jones {
        s.typ = SurfType::Jones;
        s.jones = j;
    }
    let ret = match (&q.polarizer, &q.waveplate) {
        (Some(p), _) => Some(p.retarder()?),
        (_, Some(w)) => Some(w.retarder()?),
        _ => None,
    };
    if let Some(r) = ret {
        // jones は設計波長（偏光子は波長によらない）の値。偏光の追跡は波長ごとに retarder から
        s.typ = SurfType::Jones;
        s.jones = r.jones8(r.wl0);
        s.retarder = Some(r);
    }
    if let Some([tx, ty]) = q.tilt {
        if q.coordbrk.is_some() {
            return Err("座標ブレークの面に tilt は書けない".into());
        }
        if !(tx.is_finite() && ty.is_finite()) {
            return Err("tilt が数でない".into());
        }
        if tx != 0.0 || ty != 0.0 {
            s.before = Some(lensbench_core::surface::Tilt {
                d: [0.0, 0.0, tx, ty, 0.0],
                order: 0,
            });
            s.after = After::Reverse;
        }
    }
    if let Some(d) = q.coordbrk {
        if q.order > 1 {
            return Err(format!("座標ブレークの順序 {}（0 か 1）", q.order));
        }
        if q.aperture.is_some() {
            return Err("座標ブレークの面に開口は書けない".into());
        }
        s.typ = SurfType::CoordBrk;
        s.mat = "-".into();
        s.cb = d;
        s.cb_order = q.order;
        return Ok(s);
    }
    if let Some(g) = &q.glass {
        if let Ok(nd) = g.trim().parse::<f64>() {
            // モデルガラス（分散なし）: 名前は .zmx 読込と同じ形、屈折率は n に入れておく
            s.mat = format!("{nd:.2},{:.1}", 0.0);
            s.n = vec![nd];
        } else {
            s.mat = g.trim().to_string();
        }
    }
    Ok(s)
}

pub(crate) fn pick_lens(s: &System, k: usize) -> lensbench_core::Result<Element> {
    let els = elements::split(s)?;
    let lenses: Vec<&Element> = els.iter().filter(|e| e.kind == Kind::Lens).collect();
    match lenses.get(k.wrapping_sub(1)) {
        Some(e) => Ok((*e).clone()),
        None => Err(lensbench_core::Error::Invalid(format!(
            "玉 {k} はない（{} 個）",
            lenses.len()
        ))),
    }
}

/// プロジェクトの状態 state（None なら変更なし）を順次の面の列の系に展開する
pub fn expand_state(p: &Project, env: &Env, state: Option<&str>) -> Result<System, String> {
    match state {
        Some(n) => expand(&p.with_state(n)?, env),
        None => expand(p, env),
    }
}

/// プロジェクトを順次の面の列の系に展開する
pub fn expand(p: &Project, env: &Env) -> Result<System, String> {
    expand_report(p, env).map(|r| r.0)
}

/// expand と同じ。展開で落としたもの（取り込んだ範囲の外を参照するピックアップなど）の説明も返す
pub fn expand_report(p: &Project, env: &Env) -> Result<(System, Vec<String>), String> {
    expand_inner(p, env, p.path.as_deref(), false).map(|r| (r.0, r.1))
}

/// id のある空気のブロックが入る面: (id, 面番号, 向き)。[`expand_gaps`]
pub type Gap = (String, usize, f64);

/// id のある空気のブロックの間隔（差し替え探索の変数）: (id, その空気が入る面の番号, 向き)。
/// 向きは鏡の後（光が −z へ進む）で −1（面の厚さは −t）。往復で 2 回通る空気は 2 つ並ぶ。
/// 面の厚さには、同じ面に入る他の空気・群の移動も足されている
pub fn expand_gaps(p: &Project, env: &Env) -> Result<(System, Vec<Gap>), String> {
    expand_inner(p, env, p.path.as_deref(), false).map(|r| (r.0, r.3))
}

/// ビームスプリッターの経路 path（BS を通る順の T・R）を展開する。None ならプロジェクトの path
pub fn expand_path(
    p: &Project,
    env: &Env,
    path: Option<&str>,
) -> Result<(System, Vec<String>), String> {
    expand_inner(p, env, path.or(p.path.as_deref()), false).map(|r| (r.0, r.1))
}

/// プロジェクトでとれるビームスプリッターの経路の全部（BS を通る順の T・R の文字列）。BS がなければ空。
/// 経路は BS ごとに透過・反射を選んで、BS を通らなくなるまで（像面まで）たどったもの
pub fn paths(p: &Project, env: &Env) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut todo = vec![String::new()];
    while let Some(q) = todo.pop() {
        match expand_inner(p, env, Some(&q), true) {
            Ok(_) => out.push(q),
            Err(e) if e == splitter::NEED => {
                if out.len() + todo.len() > 1024 {
                    return Err("経路が 1024 より多い".into());
                }
                todo.push(format!("{q}R"));
                todo.push(format!("{q}T"));
            }
            Err(e) => {
                return Err(if q.is_empty() {
                    e
                } else {
                    format!("経路 {q}: {e}")
                })
            }
        }
    }
    if out == [""] {
        out.clear();
    }
    out.sort_by(|a, b| a.len().cmp(&b.len()).then(b.cmp(a)));
    Ok(out)
}

/// BS の id を集める（重なりを調べる）
pub(crate) fn splitter_ids<'x>(v: &'x [Block], out: &mut Vec<&'x str>) {
    for b in v {
        match b {
            Block::Splitter { id, spec, .. } => {
                out.push(id);
                for l in [&spec.t, &spec.r, &spec.d] {
                    splitter_ids(l, out);
                }
            }
            Block::Group { blocks, .. } => splitter_ids(blocks, out),
            _ => {}
        }
    }
}

pub(crate) fn expand_inner(
    p: &Project,
    env: &Env,
    path: Option<&str>,
    strict: bool,
) -> Result<(System, Vec<String>, usize, Vec<Gap>), String> {
    {
        let mut ids = Vec::new();
        splitter_ids(&p.blocks, &mut ids);
        for (i, id) in ids.iter().enumerate() {
            if id.is_empty() {
                return Err("BS（splitter）には id が要る".into());
            }
            if ids[..i].contains(id) {
                return Err(format!("BS の id {id} が 2 つ以上ある"));
            }
        }
    }
    if p.wavelengths.is_empty() {
        return Err("波長がない".into());
    }
    if p.primary == 0 || p.primary > p.wavelengths.len() {
        return Err(format!(
            "主波長 {}（波長は {} 個）",
            p.primary,
            p.wavelengths.len()
        ));
    }
    let dirs = glass::catalog_dirs(&env.glass_dirs);
    let (glasses, missing) = glass::load_catalogs_shared(&p.glass_catalogs, &dirs);
    if !missing.is_empty() {
        return Err(format!(
            "ガラスカタログが見つからない: {}",
            missing.join(", ")
        ));
    }
    let mut c = Ctx {
        env,
        base: env.base.clone(),
        users: Vec::new(),
        sf: Vec::new(),
        org: Vec::new(),
        zpicks: Vec::new(),
        lead: 0.0,
        parity: 1.0,
        glasses,
        coats: HashMap::new(),
        nstop: 0,
        spans: Vec::new(),
        back_spans: Vec::new(),
        gaps: Vec::new(),
        split: SplitState {
            path: splitter::parse_path(path.unwrap_or(""))?,
            strict,
            ..Default::default()
        },
        frames: Vec::new(),
        rev_spans: Vec::new(),
        zenv: Vec::new(),
    };
    for (i, b) in p.blocks.iter().enumerate() {
        if c.split.done {
            break;
        }
        // 検出器（最後のブロック）は面を足さない。像面に置く
        if matches!(b, Block::Detector(_)) && i + 1 == p.blocks.len() {
            continue;
        }
        c.block(b, &p.blocks, i)?;
    }
    if c.split.pos < c.split.path.len() {
        return Err(format!(
            "経路 {}: 文字が多い（BS を通るのは {} 回）",
            path.unwrap_or(""),
            c.split.pos
        ));
    }
    let npass = c.split.pos;
    // 絞りを面に置く操作（surface_ops の set_stop）があれば、ブロックの絞りは無くてよい
    let stop_op = p
        .surface_ops
        .iter()
        .any(|o| o.get("op").and_then(Value::as_str) == Some("set_stop"));
    if c.nstop > 1 || (c.nstop == 0 && !stop_op) {
        return Err(format!("絞りが {} 個（ちょうど 1 つ要る）", c.nstop));
    }
    if c.nstop == 0 {
        // 仮の絞り（set_stop で移す）
        if let Some(f) = c.sf.first_mut() {
            f.stop = true;
        }
    }
    let wl: Vec<f64> = p.wavelengths.iter().map(|w| w.um).collect();
    let wt: Vec<f64> = p.wavelengths.iter().map(|w| w.weight).collect();
    let obj = Surface {
        t: p.object_distance.unwrap_or(f64::INFINITY),
        ..Default::default()
    };
    let mut sf = vec![obj];
    if p.object_distance.is_some() {
        sf[0].t += c.lead;
    }
    if c.sf.len() != c.org.len() {
        // 面は put か element_from で足す（出どころの表を面と揃える）
        return Err(format!(
            "内部エラー: 面 {} 個に出どころ {} 個",
            c.sf.len(),
            c.org.len()
        ));
    }
    sf.append(&mut c.sf);
    let org = std::mem::take(&mut c.org);
    let zpicks = std::mem::take(&mut c.zpicks);
    sf.push(Surface::default());
    // 面の後の屈折率（波長ごと）
    let envr = p.environment;
    let mut prev = vec![1.0; wl.len()];
    for (j, d) in sf.iter_mut().enumerate() {
        let n = if j > 0 && (d.mirror || d.typ == SurfType::CoordBrk) {
            prev.clone()
        } else if is_air(&d.mat) {
            vec![1.0; wl.len()]
        } else if let Some(g) = c.glasses.get(&d.mat) {
            wl.iter()
                .map(|&l| g.index(l, envr.temperature_c, envr.pressure_atm, envr.adjust_index))
                .collect()
        } else if let Some(&n0) = d.n.first() {
            if d.n.iter().any(|&v| v != n0) {
                return Err(format!("面 {j}: ガラス {} がカタログにない{HINT}", d.mat));
            }
            vec![n0; wl.len()]
        } else {
            return Err(format!("面 {j}: ガラス {} がカタログにない{HINT}", d.mat));
        };
        d.n = n.clone();
        prev = n;
        // 複屈折材の板の入口: 異常光の屈折率
        if let Some(lensbench_core::surface::Bire::In { ne, .. }) = d.bire.as_mut() {
            let en = lensbench_core::surface::Bire::e_name(&d.mat);
            let g = c.glasses.get(&en).ok_or_else(|| {
                format!(
                    "面 {j}: 複屈折材 {} の異常光のガラス {en} がカタログにない",
                    d.mat
                )
            })?;
            *ne = wl
                .iter()
                .map(|&l| g.index(l, envr.temperature_c, envr.pressure_atm, envr.adjust_index))
                .collect();
        }
        // 複屈折材の波長板: 板のガラスを引き、設計波長の行列を作り直す
        if let Some(r) = d.retarder.as_mut().filter(|r| !r.plates.is_empty()) {
            r.resolve(&c.glasses).map_err(|e| format!("面 {j}: {e}"))?;
            d.jones = r.jones8(r.wl0);
        }
        // 屈折面での反射（ビームスプリッター）: 反対側の媒質の屈折率
        if let Some(r) = &mut d.reflect {
            r.n = if is_air(&r.mat) {
                vec![1.0; wl.len()]
            } else if let Some(g) = c.glasses.get(&r.mat) {
                wl.iter()
                    .map(|&l| g.index(l, envr.temperature_c, envr.pressure_atm, envr.adjust_index))
                    .collect()
            } else {
                return Err(format!("面 {j}: ガラス {} がカタログにない", r.mat));
            };
        }
    }
    let ap = match p.aperture.kind {
        ApertureType::Epd => ApertureKind::Epd,
        ApertureType::Fnum => ApertureKind::FNum,
        ApertureType::Na => ApertureKind::Na,
        ApertureType::Float => ApertureKind::Float,
        ApertureType::Pwfn => ApertureKind::Pwfn,
        ApertureType::Cone => ApertureKind::Cone,
    };
    let ft = match p.fields.kind {
        FieldKind::Angle => FieldType::Angle,
        FieldKind::ObjectHeight => FieldType::ObjH,
        FieldKind::ParaxialImageHeight => FieldType::ParH,
        FieldKind::RealImageHeight => FieldType::RealH,
        // 面の名前は展開した後で引く
        FieldKind::Solve => FieldType::Angle,
    };
    let fields: Vec<Field> = p
        .fields
        .points
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let mut f = Field::xy(q[0], q[1]);
            f.w = p.fields.weights.get(i).copied().unwrap_or(1.0);
            f
        })
        .collect();
    let aim = match p.aiming {
        AimingSpec::Off => Aiming::Off,
        AimingSpec::Paraxial => Aiming::Paraxial,
        AimingSpec::Real => Aiming::Real,
    };
    let mut s = System::new(
        sf,
        wl,
        wt,
        p.primary - 1,
        fields,
        ft,
        FieldNorm::Radial,
        (ap, p.aperture.value),
        p.telecentric,
        aim,
        c.glasses,
    )
    .map_err(|e| e.to_string())?;
    s.name = p.name.clone();
    s.temperature = envr.temperature_c;
    s.pressure = envr.pressure_atm;
    s.adjust_index = envr.adjust_index;
    s.floating_clip = p.floating_clip;
    // 膜: プロジェクトの coatings はカタログの同じ名前より優先
    s.coat_tables = c.coats;
    for (k, spec) in &p.coatings {
        let t = spec.table().map_err(|e| format!("膜 {k}: {e}"))?;
        s.coat_tables.insert(k.clone(), t);
    }
    if let Some(u) = &p.afocal {
        s.afocal = true;
        s.afocal_unit = AfocalUnit::from_code(u)
            .ok_or_else(|| format!("afocal の単位 {u}（ur・mr・rr・as・am・dg のどれか）"))?;
    }
    if p.fields.kind == FieldKind::Solve {
        let Some(c) = &p.fields.solve else {
            return Err("fields の type が solve なら solve に条件を書く".into());
        };
        let f = |t: &str| s.find_surface(t).map_err(|e| format!("視野の条件: {e}"));
        let cond = match c {
            FieldSolveSpec::Position { surface } => FieldCond::Position { surf: f(surface)? },
            FieldSolveSpec::DirSum { s1, s2 } => FieldCond::DirSum {
                s1: f(s1)?,
                s2: f(s2)?,
            },
            FieldSolveSpec::DirDiff { s1, s2 } => FieldCond::DirDiff {
                s1: f(s1)?,
                s2: f(s2)?,
            },
        };
        s.field_type = FieldType::Solve(cond);
        s.changed().map_err(|e| e.to_string())?;
    } else if p.fields.solve.is_some() {
        return Err("fields の solve は type が solve のときだけ".into());
    }
    let mut warn = keep_pickups(&mut s, &org, &zpicks)?;
    for (path, t, pr, adj) in &c.zenv {
        let e = &p.environment;
        // どちらも補正しないなら温度・気圧は屈折率に効かない
        if !*adj && !e.adjust_index {
            continue;
        }
        // 屈折率の補正は 20 °C・1 atm の外でだけ効く
        let off_ref = e.temperature_c != 20.0 || e.pressure_atm != 1.0;
        if (t - e.temperature_c).abs() > 1e-9
            || (pr - e.pressure_atm).abs() > 1e-9
            || (off_ref && *adj != e.adjust_index)
        {
            warn.push(format!(
                "{path}: .zmx の環境（{t} °C・{pr} atm）はプロジェクトの環境（{} °C・{} atm）と違う。屈折率はプロジェクトの環境で計算する（environment で変える）",
                e.temperature_c, e.pressure_atm
            ));
        }
    }
    // 玉・群のパラメータ: 玉モードの玉 lens1 … と、id のあるブロック・群（同じ id の範囲は 1 つの玉・群）
    s.units = s.lens_units();
    let mut ids: Vec<&str> = Vec::new();
    for (id, _, _) in &c.spans {
        if !ids.contains(&id.as_str()) {
            ids.push(id);
        }
    }
    for id in ids {
        let spans: Vec<[usize; 2]> = c
            .spans
            .iter()
            .filter(|x| x.0 == id)
            .map(|x| [x.1 + 1, x.2 + 1])
            .collect();
        match s.define_unit(id, &spans, [0.0; 3]) {
            // 群の移動はもう厚さに入っているので、その分を玉・群の shift の今の値にする（状態ごとの群の位置を
            // `id.shift` で比べられるように）
            Ok(u) => s.units[u].shift = group_shift(&p.blocks, id).unwrap_or(0.0),
            Err(e) => warn.push(format!("ブロック {id} を玉・群のパラメータにしない: {e}")),
        }
    }
    // 裏返して置いた玉（id の無い玉の lens1 … も）。最初の範囲が玉の面と同じ玉・群
    for u in s.units.iter_mut() {
        u.flipped = c
            .rev_spans
            .iter()
            .any(|&(a, b)| u.spans.first() == Some(&[a + 1, b + 1]));
    }
    warn.extend(apply_surface_edits(
        &mut s,
        &p.surface_edits,
        &c.spans,
        &c.back_spans,
    )?);
    let (w, stopped) = apply_surface_ops(&mut s, &p.surface_ops, &c.spans, &c.back_spans)?;
    if c.nstop == 0 && !stopped {
        // 仮の絞りのまま（set_stop のブロックが無い）にはしない
        return Err(
            "絞りが 0 個（surface_ops の set_stop のブロックが無い。ちょうど 1 つ要る）".into(),
        );
    }
    warn.extend(w);
    // 自動の半直径を光線から求める。機構部品の検査・描画・書き出しが使う
    s.update_semi();
    s.mechanics = place_mechanics(
        &s,
        &p.mechanics,
        &env.base,
        &c.spans,
        &c.back_spans,
        &mut warn,
    )?;
    s.mechanics.extend(place_frames(&s, &c.frames, &mut warn));
    s.validate().map_err(|e| e.to_string())?;
    Ok((s, warn, npass, c.gaps))
}

/// 機構部品を系の面に置く（[`Project::mechanics`]）。ブロックが無い（状態で外したなど）部品は置かずに注意を出す
pub(crate) fn place_mechanics(
    s: &System,
    ms: &[MechPlacement],
    base: &Path,
    spans: &[(String, usize, usize)],
    back_spans: &[(String, usize, usize)],
    warn: &mut Vec<String>,
) -> Result<Vec<lensbench_core::mech::MechPart>, String> {
    let mut out = Vec::new();
    // 置いた部品の寸法（after でねじを見る）
    let mut specs: Vec<MechSpec> = Vec::new();
    for (k, m) in ms.iter().enumerate() {
        let tag = format!("mechanics {}", k + 1);
        // 取ってキャッシュにある品の STEP（詳しい形の描画に使う、#121）
        let mut step_file = String::new();
        let (spec, item_name) = match (&m.mech, &m.catalog, &m.item) {
            (Some(x), None, None) => (x.clone(), String::new()),
            (None, Some(cat), Some(item)) => {
                let path = base.join(cat);
                let c = UserCatalog::load_shared(&path)?;
                let it = c
                    .items
                    .get(item)
                    .ok_or_else(|| format!("{tag}: {} に品 {item} はない", path.display()))?;
                let x = it
                    .mech
                    .clone()
                    .ok_or_else(|| format!("{tag}: 品 {item} は機構部品でない（mech がない）"))?;
                // URL の無い品も、共有の置き場から取ってあればキャッシュにある
                {
                    if let Some(f) = catalog_fill::cad_cache_find(
                        &catalog_fill::cad_cache_dir(),
                        item,
                        &it.cad.step,
                    ) {
                        step_file = f.display().to_string();
                    }
                }
                (x, item.clone())
            }
            _ => {
                return Err(format!(
                    "{tag}: 部品は mech か、catalog と item のどちらか一方で書く"
                ))
            }
        };
        spec.validate().map_err(|e| format!("{tag}: {e}"))?;
        // 裏返す（z の大きい端から光が入る）。座に合わせるなら、座は保持する素子の後ろの面に当たる
        let spec = if m.reversed {
            let t = if m.seat {
                m.holds
                    .first()
                    .and_then(|b| spans.iter().find(|x| &x.0 == b && !back_spans.contains(x)))
                    .map_or(0.0, |x| {
                        (x.1 + 1..=x.2)
                            .filter(|&j| {
                                j < s.surfaces.len() && s.surfaces[j].typ != SurfType::CoordBrk
                            })
                            .map(|j| s.surfaces[j].t)
                            .sum::<f64>()
                    })
            } else {
                0.0
            };
            spec.flipped(t)
        } else {
            spec
        };
        let name = if !m.name.is_empty() {
            m.name.clone()
        } else if !item_name.is_empty() {
            item_name.clone()
        } else {
            format!("mech{}", k + 1)
        };
        // 前の部品の後ろの端につなぐ
        if let Some(prev_name) = &m.after {
            if m.seat || !m.at.is_unset() {
                return Err(format!("{tag}: after は at・seat と一緒に使えない"));
            }
            if m.tilt != [0.0, 0.0] {
                return Err(format!(
                    "{tag}: after では tilt は使えない（前の部品の傾きになる）"
                ));
            }
            let Some((prev, prev_spec)) = out
                .iter()
                .zip(&specs)
                .rev()
                .find(|(p, _): &(&lensbench_core::mech::MechPart, _)| &p.name == prev_name)
            else {
                return Err(format!("{tag}: after の部品 {prev_name} が前に無い"));
            };
            let (prev, prev_spec): (&lensbench_core::mech::MechPart, &MechSpec) = (prev, prev_spec);
            let overlap = match (&prev_spec.back_thread, &spec.front_thread) {
                (Some(a), Some(b)) => {
                    if a.thread != b.thread || a.male == b.male {
                        let kind = |t: &ThreadEnd| {
                            format!("{} の{}", t.thread, if t.male { "オス" } else { "メス" })
                        };
                        warn.push(format!(
                            "{tag}: {prev_name} の後ろの端（{}）と、{name} の前の端（{}）のねじが合わない",
                            kind(a),
                            kind(b)
                        ));
                        0.0
                    } else if b.male {
                        b.length + a.recess
                    } else {
                        a.length + b.recess
                    }
                }
                _ => 0.0,
            };
            let mut part0 = spec.part(&name, prev.surface, [0.0; 3], prev.tilt);
            let z = prev.z_range()[1] - overlap - part0.z_range()[0];
            let r = lensbench_core::math::mm(
                &lensbench_core::math::rot(0, prev.tilt[0]),
                &lensbench_core::math::rot(1, prev.tilt[1]),
            );
            let d = lensbench_core::math::mv(&r, [m.offset[0], m.offset[1], m.offset[2] + z]);
            part0.offset = [
                prev.offset[0] + d[0],
                prev.offset[1] + d[1],
                prev.offset[2] + d[2],
            ];
            if !part0.offset.iter().all(|v| v.is_finite()) {
                return Err(format!("{tag}: offset が数でない"));
            }
            for b in &m.holds {
                match spans.iter().find(|x| &x.0 == b && !back_spans.contains(x)) {
                    Some(x) => part0.holds.extend(
                        (x.1 + 1..=x.2 + 1).filter(|&j| s.surfaces[j].typ != SurfType::CoordBrk),
                    ),
                    None => warn.push(format!("{tag}: holds のブロック {b} が無い")),
                }
            }
            part0.step = step_file;
            part0.step_flip = m.reversed;
            out.push(part0);
            specs.push(spec);
            continue;
        }
        let mut offset = m.offset;
        // 座に合わせる: 保持する最初の素子の前面に、部品の座の z を合わせる
        let seat_at = if m.seat {
            let seat = spec
                .holds
                .ok_or_else(|| format!("{tag}: seat には部品の寸法の holds（座の位置）が要る"))?
                .seat_z;
            let b = m
                .holds
                .first()
                .ok_or_else(|| format!("{tag}: seat には holds（保持する素子）が要る"))?;
            offset[2] -= seat;
            match block_face(s, b, 0, spans, back_spans).map_err(|e| format!("{tag}: {e}"))? {
                Some(j) => Some(j),
                None => {
                    warn.push(format!("{tag}: ブロック {b} が無いので置かない"));
                    continue;
                }
            }
        } else {
            None
        };
        let surface = match (seat_at, &m.at.block) {
            (Some(j), _) => j,
            (None, block) => match block {
                Some(b) => match block_face(s, b, m.at.surface, spans, back_spans)
                    .map_err(|e| format!("{tag}: {e}"))?
                {
                    Some(j) => j,
                    None => {
                        warn.push(format!("{tag}: ブロック {b} が無いので置かない"));
                        continue;
                    }
                },
                None => {
                    if m.at.is_unset() {
                        return Err(format!("{tag}: 置く所（at）か seat が要る"));
                    }
                    if m.at.surface == 0 || m.at.surface > s.image() {
                        return Err(format!("{tag}: 面 {}（1〜{}）", m.at.surface, s.image()));
                    }
                    m.at.surface
                }
            },
        };
        if !offset.iter().chain(&m.tilt).all(|v| v.is_finite()) {
            return Err(format!("{tag}: offset・tilt が数でない"));
        }
        let mut part = spec.part(&name, surface, offset, m.tilt);
        for b in &m.holds {
            match spans.iter().find(|x| &x.0 == b && !back_spans.contains(x)) {
                Some(x) => part.holds.extend(
                    (x.1 + 1..=x.2 + 1).filter(|&j| s.surfaces[j].typ != SurfType::CoordBrk),
                ),
                None => warn.push(format!("{tag}: holds のブロック {b} が無い")),
            }
        }
        part.step = step_file;
        part.step_flip = m.reversed;
        out.push(part);
        specs.push(spec);
    }
    Ok(out)
}

/// 枠付きのユーザーカタログの品の枠を、ガラスの最初の面に置く（枠の座 `holds.seat_z` をガラスの前面に合わせる）。
/// 保持する素子はその品の面
pub(crate) fn place_frames(
    s: &System,
    frames: &[(String, MechSpec, usize, usize)],
    warn: &mut Vec<String>,
) -> Vec<lensbench_core::mech::MechPart> {
    let mut out = Vec::new();
    for (name, f, a, b) in frames {
        let tag = format!("{name} の枠");
        if let Err(e) = f.validate() {
            warn.push(format!("{tag}: {e}（置かない）"));
            continue;
        }
        let faces: Vec<usize> = (a + 1..=b + 1)
            .filter(|&j| j < s.surfaces.len() && s.surfaces[j].typ != SurfType::CoordBrk)
            .collect();
        let Some(&first) = faces.first() else {
            continue;
        };
        let seat = f.holds.map_or(0.0, |h| h.seat_z);
        let mut part = f.part(&tag, first, [0.0, 0.0, -seat], [0.0, 0.0]);
        part.holds = faces;
        out.push(part);
    }
    out
}

/// ブロック block の面 i（座標ブレークを除いた番号、往路）の系の面の番号。ブロックが無ければ None
pub(crate) fn block_face(
    s: &System,
    block: &str,
    i: usize,
    spans: &[(String, usize, usize)],
    back_spans: &[(String, usize, usize)],
) -> Result<Option<usize>, String> {
    let Some(x) = spans
        .iter()
        .find(|x| x.0 == block && !back_spans.contains(x))
    else {
        return Ok(None);
    };
    let faces: Vec<usize> = (x.1 + 1..=x.2 + 1)
        .filter(|&j| s.surfaces[j].typ != SurfType::CoordBrk)
        .collect();
    faces.get(i).copied().map(Some).ok_or(format!(
        "ブロック {block} の面 {i}（面は {} 枚）",
        faces.len()
    ))
}

/// 面の絞り・ソルブ・ピックアップを当てる（[`Project::surface_ops`]）。当てなかった操作の注意と、set_stop を
/// 当てたかを返す
pub(crate) fn apply_surface_ops(
    s: &mut System,
    ops: &[Value],
    spans: &[(String, usize, usize)],
    back_spans: &[(String, usize, usize)],
) -> Result<(Vec<String>, bool), String> {
    let mut warn = Vec::new();
    let mut out = Vec::new();
    'ops: for (k, op) in ops.iter().enumerate() {
        let name = op.get("op").and_then(Value::as_str).unwrap_or_default();
        if !["set_stop", "add_solve", "add_pickup"].contains(&name) {
            return Err(format!(
                "surface_ops {}: 操作は set_stop・add_solve・add_pickup（{name}）",
                k + 1
            ));
        }
        let mut o = op.clone();
        for key in ["surface", "dst", "src", "other"] {
            let Some(r) = op.get(key).and_then(Value::as_object) else {
                continue;
            };
            let (Some(b), Some(i)) = (
                r.get("block").and_then(Value::as_str),
                r.get("surface").and_then(Value::as_u64),
            ) else {
                return Err(format!(
                    "surface_ops {}: {key} は {{\"block\": id, \"surface\": i}} か面の番号",
                    k + 1
                ));
            };
            match block_face(s, b, i as usize, spans, back_spans)
                .map_err(|x| format!("surface_ops {}: {x}", k + 1))?
            {
                Some(j) => o[key] = Value::from(j),
                None => {
                    warn.push(format!(
                        "surface_ops {}: ブロック {b} が無いので {name} を当てない",
                        k + 1
                    ));
                    continue 'ops;
                }
            }
        }
        out.push(o);
    }
    let stopped = out.iter().any(|o| o["op"] == "set_stop");
    if !out.is_empty() {
        warn.extend(
            edit_ops::apply(s, &Value::Array(out)).map_err(|x| format!("surface_ops: {x}"))?,
        );
    }
    Ok((warn, stopped))
}

/// 面の値の上書きを当てる（ブロック id の面のうち座標ブレークを除いた番号）。
/// ブロックが系に無い（状態で外した・OFF の群の中など）上書きは `surface_ops` と同じく当てずに注意を返す
pub(crate) fn apply_surface_edits(
    s: &mut System,
    edits: &[SurfaceEdit],
    spans: &[(String, usize, usize)],
    back_spans: &[(String, usize, usize)],
) -> Result<Vec<String>, String> {
    let mut warn = Vec::new();
    // 範囲の面（座標ブレークを除く）
    let faces = |s: &System, a: usize, b: usize| -> Vec<usize> {
        (a + 1..=b + 1)
            .filter(|&j| s.surfaces[j].typ != SurfType::CoordBrk)
            .collect()
    };
    for e in edits {
        let err = |x: String| {
            format!(
                "surface_edits: ブロック {} の面 {}: {x}",
                e.block, e.surface
            )
        };
        let mine = |x: &&(String, usize, usize)| x.0 == e.block;
        // 往路（と、同じ向きに通るほかの範囲）・復路
        let fwd: Vec<Vec<usize>> = spans
            .iter()
            .filter(mine)
            .filter(|x| !back_spans.contains(x))
            .map(|x| faces(s, x.1, x.2))
            .collect();
        let back: Vec<Vec<usize>> = back_spans
            .iter()
            .filter(mine)
            .map(|x| faces(s, x.1, x.2))
            .collect();
        let Some(n) = fwd.first().map(Vec::len).filter(|&n| n > 0) else {
            warn.push(format!(
                "surface_edits: ブロック {} が無いので面 {} の上書きを当てない",
                e.block, e.surface
            ));
            continue;
        };
        let i = e.surface;
        if i >= n {
            return Err(format!(
                "surface_edits: ブロック {} の面 {}（面は {n} 枚）",
                e.block, e.surface
            ));
        }
        // 面の値の名前（SURF_FIELDS）と、それ以外の面のデータの項目（set_surface_data）
        let (named, data): (Map<String, Value>, Map<String, Value>) = e
            .fields
            .clone()
            .into_iter()
            .partition(|(k, _)| lensbench_core::edit::SURF_FIELDS.contains(&k.as_str()));
        let data = Value::Object(data);
        let has_data = data.as_object().is_some_and(|m| !m.is_empty());
        let f = edit_ops::surf_fields(&Value::Object(named))
            .map_err(|x| format!("surface_edits: {x}"))?;
        for v in &fwd {
            let j = *v
                .get(i)
                .ok_or_else(|| err("同じ向きの範囲の面の数が違う".into()))?;
            if !f.is_empty() {
                s.set_surface_fields(j, &f)
                    .map_err(|x| err(x.to_string()))?;
            }
            if has_data {
                edit_ops::set_surface_data(s, j, &data).map_err(err)?;
            }
        }
        if has_data && !back.is_empty() {
            if ["t", "mat", "dn", "disp"]
                .iter()
                .any(|k| data.get(*k).is_some())
            {
                return Err(err(
                    "往復するブロックの厚さ・材料は t・material の名前で変える".into(),
                ));
            }
            // 復路では面の前後が入れ替わり向きも逆になるので、前後の傾き偏心はそのまま当てられない
            if ["before", "after"].iter().any(|k| data.get(*k).is_some()) {
                return Err(err(
                    "往復するブロックの面の前後の傾き偏心は変えられない（ブロックの偏心・傾きで）"
                        .into(),
                ));
            }
            for v in &back {
                if v.len() == n {
                    edit_ops::set_surface_data(s, v[n - 1 - i], &data).map_err(err)?;
                }
            }
        }
        // 復路は逆の順にたどる: 面の値（曲率・コーニック・半径など）は同じ面 n−1−i に同じ値、面の後の媒質の値
        // （厚さ・材料）は n−2−i に（厚さは符号が逆）。往路の最後の面の後はブロックの外（後の空気）なので、往復する
        // ブロックでは変えない
        for v in &back {
            if v.len() != n {
                return Err(err(format!("復路の面の数 {}（往路は {n}）", v.len())));
            }
            let (face, gap): (Vec<_>, Vec<_>) = f
                .iter()
                .cloned()
                .partition(|(k, _)| !matches!(k.as_str(), "t" | "material" | "dn" | "disp"));
            if !face.is_empty() {
                s.set_surface_fields(v[n - 1 - i], &face)
                    .map_err(|x| err(x.to_string()))?;
            }
            if !gap.is_empty() {
                if i + 1 == n {
                    return Err(err(
                        "往復するブロックの最後の面の厚さ・材料は、後の空気のブロックで変える"
                            .into(),
                    ));
                }
                let gap: Vec<_> = gap
                    .into_iter()
                    .map(|(k, x)| match (k.as_str(), x) {
                        ("t", lensbench_core::edit::SurfValue::Num(t)) => {
                            (k, lensbench_core::edit::SurfValue::Num(-t))
                        }
                        (_, x) => (k, x),
                    })
                    .collect();
                s.set_surface_fields(v[n - 2 - i], &gap)
                    .map_err(|x| err(x.to_string()))?;
            }
        }
    }
    Ok(warn)
}

/// id の群（ブロックの群・.zmx の取り込みの中の群）の移動
pub(crate) fn group_shift(blocks: &[Block], id: &str) -> Option<f64> {
    for b in blocks {
        match b {
            Block::Group {
                id: g,
                shift,
                blocks,
                ..
            } => {
                if g == id {
                    return Some(*shift);
                }
                if let Some(v) = group_shift(blocks, id) {
                    return Some(v);
                }
            }
            Block::Zmx { groups, .. } => {
                if let Some(g) = groups.iter().find(|g| g.id == id) {
                    return Some(g.shift);
                }
            }
            Block::Splitter { spec, .. } => {
                for arm in [&spec.t, &spec.r, &spec.d] {
                    if let Some(v) = group_shift(arm, id) {
                        return Some(v);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// zmx ブロックで取り込んだ面のピックアップと厚さのソルブを、展開した系の面番号に付け替えて残す。
/// 元か先が取り込んだ範囲の外にあるもの、展開で値の関係が崩れたもの（群の移動で厚さが変わったなど）は
/// 値で固定し、説明を返す。org[i] は展開した系の面 i + 1 の出どころ
pub(crate) fn keep_pickups(
    s: &mut System,
    org: &[Option<(usize, usize)>],
    zpicks: &[ZmxLinks],
) -> Result<Vec<String>, String> {
    let mut at: HashMap<(usize, usize), usize> = HashMap::new();
    for (i, o) in org.iter().enumerate() {
        if let Some(k) = o {
            // 同じ取り込みで同じ面が 2 回出ることはない
            at.insert(*k, i + 1);
        }
    }
    let mut warn = Vec::new();
    for (inst, z) in zpicks.iter().enumerate() {
        let (path, picks) = (&z.path, &z.pickups);
        for q in picks {
            let (Some(&dst), src) = (at.get(&(inst, q.dst)), at.get(&(inst, q.src))) else {
                continue; // 先が取り込んでいない面
            };
            let Some(&src) = src else {
                warn.push(format!(
                    "{path}: 面 {} の {:?} は範囲の外の面 {} からのピックアップ（値で固定）",
                    q.dst, q.what, q.src
                ));
                continue;
            };
            // 鏡のあと（向きが逆、sign = −1）で取り込むと、曲率・厚さの符号が変わる。先 dst' = σd·dst、元 src' = σs·src
            // （σ は量の符号が変われば sign、変わらなければ 1）なら、dst = a·src + b は dst' = (σd·σs·a)·src' + σd·b。
            // 向きから決めた組を先に試し、合わなければ（面タイプのパラメータの意味は型で違う）ほかの符号の組を試す
            use lensbench_core::param::PickKind as Pk;
            let flips = |k: Pk| matches!(k, Pk::Curv | Pk::Thick | Pk::Radius);
            let sd = if flips(q.what) { z.sign } else { 1.0 };
            let ss = if flips(q.from.unwrap_or(q.what)) {
                z.sign
            } else {
                1.0
            };
            let mut cands = vec![Pickup {
                dst,
                src,
                scale: sd * ss * q.scale,
                offset: sd * q.offset,
                ..*q
            }];
            for (a, b) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
                let c = Pickup {
                    dst,
                    src,
                    scale: a * q.scale,
                    offset: b * q.offset,
                    ..*q
                };
                if !cands.contains(&c) {
                    cands.push(c);
                }
            }
            match cands.iter().position(|c| c.holds(s)) {
                Some(0) => s.pickups.push(cands[0]),
                Some(k) => {
                    // 向きから決めた組と違う符号で値が合った（元の値が 0 だと符号を区別できない）
                    s.pickups.push(cands[k]);
                    warn.push(format!(
                        "{path}: 面 {} の {:?}（面 {} から）は値の照合で倍率・ずれの符号を決めた（倍率 {}・ずれ {}）。意図どおりか確かめる",
                        q.dst, q.what, q.src, cands[k].scale, cands[k].offset
                    ));
                }
                None => warn.push(format!(
                    "{path}: 面 {} の {:?}（面 {} から）は展開で値の関係が変わった（値で固定）",
                    q.dst, q.what, q.src
                )),
            }
        }
    }
    if !s.pickups.is_empty() {
        s.resolve_pickups().map_err(|e| e.to_string())?;
    }
    // ソルブ（厚さ・曲率）: 関わる面（厚さは次の面、玉のパワーは前の面、基準の面）が同じ並びで取り込まれていれば
    // 残し、展開した系で解き直す（雛形の像面の手前の面の厚さのソルブは、展開した系でも像面の手前にあれば残す）
    for (inst, z) in zpicks.iter().enumerate() {
        for q in &z.solves {
            let Some(&k) = at.get(&(inst, q.surf)) else {
                continue;
            };
            // 雛形の面 j が、展開した系で面 k からの同じずれにあるか
            let same = |j: usize| -> bool {
                at.get(&(inst, j))
                    .is_some_and(|&n| n as i64 - k as i64 == j as i64 - q.surf as i64)
            };
            use lensbench_core::param::SolveKind as Sk;
            let mut ok = true;
            // 位置ソルブは高さを見ないので次の面は要らない（基準の面との間が並んでいればよい）
            if !q.kind.is_curv() && !matches!(q.kind, Sk::Position(_)) {
                ok &= same(q.surf + 1) || (q.surf + 1 == z.image && k + 1 == s.image());
            }
            if q.kind == Sk::ElementPower {
                ok &= q.surf >= 1 && same(q.surf - 1);
            }
            let mut kind = q.kind;
            if let Some(o) = q.kind.other_surface() {
                ok &= same(o);
                // 基準の面との距離で決まるものは、間の面（基準の面が後ろでも）も並んで取り込まれていること
                if matches!(
                    q.kind,
                    Sk::Position(_)
                        | Sk::CenterOfCurvature(_)
                        | Sk::ConcentricSurface(_)
                        | Sk::ConcentricRadius(_)
                ) {
                    ok &= (o.min(q.surf)..=o.max(q.surf)).all(same);
                }
                if let Some(&n) = at.get(&(inst, o)) {
                    kind = kind.with_other_surface(n);
                }
            }
            if ok {
                s.solves.push(ThickSolve {
                    surf: k,
                    kind,
                    ..*q
                });
            } else {
                warn.push(format!(
                    "{}: 面 {} のソルブ（{:?}）は、関わる面が範囲の外か離れている（値で固定）",
                    z.path, q.surf, q.kind
                ));
            }
        }
    }
    if !s.solves.is_empty() {
        let val = |s: &System, q: &ThickSolve| {
            let d = &s.surfaces[q.surf];
            if q.kind.is_curv() {
                d.c
            } else {
                d.t
            }
        };
        let before: Vec<f64> = s.solves.iter().map(|q| val(s, q)).collect();
        s.solve_thickness().map_err(|e| e.to_string())?;
        for (q, b) in s.solves.iter().zip(before) {
            let a = val(s, q);
            if (a - b).abs() > 1e-9 * (1.0 + b.abs()) {
                let what = if q.kind.is_curv() { "曲率" } else { "厚さ" };
                warn.push(format!(
                    "面 {} の{what}をソルブ（{:?}、値 {}）で解き直した: {b} → {a}",
                    q.surf, q.kind, q.height
                ));
            }
        }
    }
    Ok(warn)
}
