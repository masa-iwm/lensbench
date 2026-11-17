//! 展開の入口（状態・経路）と、面の並びのあとの仕上げ（機構部品の配置・面の操作と編集・群の移動・ピックアップの保持）

use super::*;
use lensbench_core::error::code;
use lensbench_core::glass::DIRS_HINT as HINT;
/// .zmx の取り込みの群の、要素の範囲 (群の番号, 最初, 最後)。最初・最後は面が from〜to に始まる要素（空気でない）
pub(crate) fn zmx_group_spans(
    keep: &[Element],
    groups: &[ZmxGroup],
) -> lensbench_core::Result<Vec<(usize, usize, usize)>> {
    let mut out: Vec<(usize, usize, usize)> = Vec::new();
    for (g, gr) in groups.iter().enumerate() {
        let label = if gr.id.is_empty() { &gr.name } else { &gr.id };
        if gr.from > gr.to {
            return Err(format!(
                "group {label}: surfaces {} to {} are in reverse order",
                gr.from, gr.to
            )
            .into());
        }
        let inside: Vec<usize> = (0..keep.len())
            .filter(|&i| keep[i].kind != Kind::Air && (gr.from..=gr.to).contains(&keep[i].from))
            .collect();
        let (Some(&a), Some(&z)) = (inside.first(), inside.last()) else {
            return Err(format!(
                "group {label}: no imported lenses or surfaces in surfaces {} to {}",
                gr.from, gr.to
            )
            .into());
        };
        if let Some(o) = out.iter().find(|o| a <= o.2 && o.1 <= z) {
            return Err(format!("group {label} overlaps group {}", groups[o.0].name).into());
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
pub(crate) fn reverse_surfaces(v: &[Surface]) -> lensbench_core::Result<Vec<Surface>> {
    let n = v.len();
    (0..n)
        .map(|i| {
            let mut s = v[n - 1 - i].clone();
            s.flip_z()?;
            s.t = if i + 1 < n { v[n - 2 - i].t } else { 0.0 };
            s.stop = false;
            Ok(s)
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

pub(crate) fn surf(q: &SurfSpec) -> lensbench_core::Result<Surface> {
    if let Some(r) = &q.row {
        let alone = SurfSpec {
            row: None,
            ..q.clone()
        };
        if serde_json::to_value(&alone).ok() != serde_json::to_value(SurfSpec::default()).ok() {
            return Err("a surface written as row can't have the other keys".into());
        }
        if r.stop || r.object || r.image {
            return Err(
                "row of a surface can't have stop, object or image (use set_stop of the block)"
                    .into(),
            );
        }
        return crate::sysfile_v2::read::surface((**r).clone()).map_err(Into::into);
    }
    let mut s = Surface {
        form: match q.shape {
            SurfShape::Standard if q.asph.iter().any(|a| *a != 0.0) => Form::EvenAsph {
                asph: q.asph.clone(),
            },
            SurfShape::Standard => Form::Standard,
            SurfShape::OddAsph => Form::OddAsph {
                asph: q.asph.clone(),
            },
            SurfShape::Toroidal => Form::Toroidal {
                cx: curv(q.radius_x),
                asph: q.asph.clone(),
            },
            SurfShape::Fresnel => Form::Fresnel {
                asph: q.asph.clone(),
            },
        },
        c: curv(q.radius),
        k: q.conic,
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
            "a surface can't have two or more of jones, polarizer, waveplate, coordbrk and birefringent"
                .into(),
        );
    }
    if let Some(b) = q.birefringent {
        s.bire = Some(match b {
            BireSpec::In { mode, axis } => {
                if !matches!(mode, 0 | 2) {
                    return Err(format!("birefringent plate mode {mode} (0 or 2)").into());
                }
                if !(axis.iter().all(|v| v.is_finite()) && axis.iter().any(|v| *v != 0.0)) {
                    return Err(
                        "the crystal axis of the birefringent plate is 0 or not a number".into(),
                    );
                }
                if q.glass.as_deref().is_none_or(is_air_name) {
                    return Err("write glass (the ordinary material) on the entrance surface of a birefringent plate".into());
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
            || s.typ() == SurfType::EvenAsph
            || q.shape != SurfShape::Standard
        {
            return Err(
                format!("a {what} surface can't have curvature, conic or asphere terms").into(),
            );
        }
        if q.glass.as_deref().is_some_and(|g| !is_air_name(g)) {
            return Err(format!("no glass after a {what} surface").into());
        }
    }
    if let Some(j) = q.jones {
        s.form = Form::Jones { jones: j };
    }
    let ret = match (&q.polarizer, &q.waveplate) {
        (Some(p), _) => Some(p.retarder()?),
        (_, Some(w)) => Some(w.retarder()?),
        _ => None,
    };
    if let Some(r) = ret {
        // jones は設計波長（偏光子は波長によらない）の値。偏光の追跡は波長ごとに retarder から
        s.form = Form::Jones {
            jones: r.jones8(r.wl0),
        };
        s.retarder = Some(r);
    }
    if let Some([tx, ty]) = q.tilt {
        if q.coordbrk.is_some() {
            return Err("a coordinate break surface can't have tilt".into());
        }
        if !(tx.is_finite() && ty.is_finite()) {
            return Err("tilt is not a number".into());
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
            return Err(format!("coordinate break order {} (0 or 1)", q.order).into());
        }
        if q.aperture.is_some() {
            return Err("a coordinate break surface can't have an aperture".into());
        }
        s.form = Form::CoordBrk {
            cb: d,
            order: q.order,
        };
        s.mat = "-".into();
        return Ok(s);
    }
    if let Some(g) = &q.glass {
        if let Ok(nd) = g.trim().parse::<f64>() {
            // 数は屈折率だけの媒質（分散なし）
            s.mat = Material::Index(nd);
        } else {
            s.mat = Material::parse(g);
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
            "no lens {k} ({} lenses)",
            lenses.len()
        ))),
    }
}

/// 展開の選び方（[`expand`]）
#[derive(Clone, Copy, Default)]
pub struct ExpandOpts<'a> {
    /// 状態（None なら変更なし）
    pub state: Option<&'a str>,
    /// ビームスプリッターの経路（BS を通る順の T・R）。None なら状態・プロジェクトの path
    pub path: Option<&'a str>,
    /// カタログの玉を使い回す（同じ環境の展開の間だけで共有する）
    pub cache: Option<&'a SourceCache>,
    /// 外側で展開中のファイル・項目（取り込んだプロジェクトを展開するとき、循環を見つける）。ふつうは空
    pub within: &'a [Loading],
}

/// 展開中のファイル（正規化したパス）と、ユーザーカタログならその項目
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loading {
    pub file: PathBuf,
    pub item: Option<String>,
}

impl std::fmt::Display for Loading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.item {
            Some(it) => write!(f, "{} item {it}", self.file.display()),
            None => write!(f, "{}", self.file.display()),
        }
    }
}

/// 展開の結果
#[derive(Debug, Clone)]
pub struct Expansion {
    /// 順次の面の列の系
    pub system: System,
    /// 展開で落としたもの（取り込んだ範囲の外を参照するピックアップなど）の注意
    pub notes: Vec<Note>,
    /// id のある空気のブロックが入る面（差し替え探索の間隔の変数、[`Gap`]）
    pub gaps: Vec<Gap>,
    /// 系の面ごとの出どころ（`system.surfaces` と同じ長さ。物体面・像面は既定値）
    pub map: Vec<SurfOrigin>,
}

/// 展開した系の面 1 つの出どころ（[`Expansion::map`]）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SurfOrigin {
    /// 面を足した id のあるブロック・群（内から外へ。.zmx の取り込みの群も）
    pub blocks: Vec<String>,
    /// 鏡の `back` で復路にたどった面
    pub back: bool,
    /// 裏返して置いた玉（`reversed`）の面なら、その玉の番号（1 始まり、展開の中で数える）
    pub flip: Option<usize>,
    /// 枠付きのユーザーカタログの品の面なら、その品の番号（[`Ctx::frames`]）
    pub(crate) frame: Option<usize>,
    /// .zmx の取り込みの番号と、元の .zmx の面番号（ピックアップを付け替える）
    pub(crate) zmx: Option<(usize, usize)>,
    /// 面を足した、面の上書き・操作を持つブロック・群（[`Ctx::sites`] の番号）
    pub(crate) mods: Vec<usize>,
    /// 面を足したプロジェクトの木のいちばん内のブロックと、その中の何番目の面か（[`FaceId`]。物体面・像面は None）。
    /// 状態・経路の違う展開の間で同じ面を探す（[`Expansion::find_face`]）
    pub face: Option<FaceId>,
    /// 展開中だけ: 面を足した木のブロックのアドレス（[`Ctx::leaf`]）。展開の終わりに face にして 0 に戻す
    pub(crate) leaf: usize,
}

/// プロジェクトのブロックの木の中の位置（上から順の段）
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct NodePath(pub Vec<NodeStep>);

/// [`NodePath`] の段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeStep {
    /// 並び（最上位・群の中）の i 番目のブロック
    Block(usize),
    /// BS の腕（0 透過 t・1 反射 r・2 d）の i 番目のブロック
    Arm(u8, usize),
}

/// 展開した系の面の、プロジェクトの中での名前（[`SurfOrigin::face`]）。ブロックの木が同じなら、状態・BS の経路が違う
/// 展開でも同じ面は同じ名前になる
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FaceId {
    /// 面を足したいちばん内のブロック（ユーザーカタログの品は品のブロック、BS の腕の中は BS）
    pub node: NodePath,
    /// 鏡の `back` で復路にたどった面
    pub back: bool,
    /// そのブロックを同じ向きで何回目にたどった範囲か（0 始まり）
    pub pass: usize,
    /// 座標ブレーク（偏心・傾き・折り曲げの座標の面）か
    pub cb: bool,
    /// 範囲の中の番号（0 始まり、たどった順）。座標ブレークとほかの面は別に数える（偏心の有無が状態で変わっても、
    /// ほかの面の番号はずれない）
    pub index: usize,
}

/// ブロックの木のブロック（アドレス）ごとの位置
pub(crate) fn node_paths(blocks: &[Block]) -> HashMap<usize, NodePath> {
    fn walk(
        v: &[Block],
        pre: &mut Vec<NodeStep>,
        arm: Option<u8>,
        out: &mut HashMap<usize, NodePath>,
    ) {
        for (i, b) in v.iter().enumerate() {
            pre.push(match arm {
                Some(a) => NodeStep::Arm(a, i),
                None => NodeStep::Block(i),
            });
            out.insert(b as *const Block as usize, NodePath(pre.clone()));
            match b {
                Block::Group { blocks, .. } => walk(blocks, pre, None, out),
                Block::Splitter { spec, .. } => {
                    for (a, l) in [&spec.t, &spec.r, &spec.d].into_iter().enumerate() {
                        walk(l, pre, Some(a as u8), out);
                    }
                }
                _ => {}
            }
            pre.pop();
        }
    }
    let mut out = HashMap::new();
    walk(blocks, &mut Vec::new(), None, &mut out);
    out
}

/// 展開中に覚えた木のブロックのアドレス（[`SurfOrigin::leaf`]）を、面の名前 [`FaceId`] にする
fn name_faces(map: &mut [SurfOrigin], sf: &[Surface], nodes: &HashMap<usize, NodePath>) {
    // (ブロック, 向き) ごとのたどった範囲の数
    let mut passes: HashMap<(usize, bool), usize> = HashMap::new();
    let mut prev: Option<(usize, bool)> = None;
    // 範囲の中の面・座標ブレークの数
    let mut k = [0usize; 2];
    for (j, o) in map.iter_mut().enumerate() {
        let leaf = std::mem::take(&mut o.leaf);
        let Some(node) = nodes.get(&leaf) else {
            prev = None;
            continue;
        };
        let key = (leaf, o.back);
        let n = passes.entry(key).or_insert(0);
        if prev != Some(key) {
            *n += 1;
            k = [0, 0];
        }
        let cb = sf[j].is_coord_break();
        o.face = Some(FaceId {
            node: node.clone(),
            back: o.back,
            pass: *n - 1,
            cb,
            index: k[cb as usize],
        });
        k[cb as usize] += 1;
        prev = Some(key);
    }
}

impl Expansion {
    /// 面の名前が id の面（[`SurfOrigin::face`]）。無ければ None
    pub fn find_face(&self, id: &FaceId) -> Option<usize> {
        self.map.iter().position(|o| o.face.as_ref() == Some(id))
    }
}

impl SurfOrigin {
    /// 面が id のブロック・群のものか
    pub fn is_in(&self, id: &str) -> bool {
        self.blocks.iter().any(|b| b == id)
    }
}

/// id のあるブロック・群 1 つの系の面の範囲（[`Expansion::block_spans`]）
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct BlockSpans {
    pub id: String,
    /// 往路の範囲（系の面の番号 [最初, 最後]、たどった順）
    pub forward: Vec<[usize; 2]>,
    /// 鏡の `back` で復路にたどった範囲
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub back: Vec<[usize; 2]>,
}

/// id のあるブロック・群（BS・開口・絞り・群も。面を足さない空気は出ない）ごとの系の面の範囲。並びは最初の面の順
pub fn block_spans(map: &[SurfOrigin]) -> Vec<BlockSpans> {
    let mut ids: Vec<&str> = Vec::new();
    for o in map {
        // 内から外への並びを、外（群）が先に出るように逆にたどる
        for b in o.blocks.iter().rev() {
            if !ids.contains(&b.as_str()) {
                ids.push(b);
            }
        }
    }
    ids.into_iter()
        .map(|id| BlockSpans {
            id: id.to_string(),
            forward: block_runs(map, id, Some(false)),
            back: block_runs(map, id, Some(true)),
        })
        .collect()
}

impl Expansion {
    /// [`block_spans`]
    pub fn block_spans(&self) -> Vec<BlockSpans> {
        block_spans(&self.map)
    }
}

/// id のブロック・群の面の範囲（系の面の番号 [最初, 最後]）。続けて同じ向きにたどった範囲ごとに、たどった順。
/// back が Some なら往路（false）か復路（true）の範囲だけ
pub fn block_runs(map: &[SurfOrigin], id: &str, back: Option<bool>) -> Vec<[usize; 2]> {
    if id.is_empty() {
        return Vec::new();
    }
    runs_where(map, |o| o.is_in(id), back)
}

/// [`block_runs`] と同じ。面が入るかを f で決める
pub(crate) fn runs_where(
    map: &[SurfOrigin],
    f: impl Fn(&SurfOrigin) -> bool,
    back: Option<bool>,
) -> Vec<[usize; 2]> {
    let mut out: Vec<[usize; 2]> = Vec::new();
    let mut prev: Option<(usize, bool)> = None;
    for (j, o) in map.iter().enumerate() {
        if !f(o) {
            prev = None;
            continue;
        }
        match (prev, out.last_mut()) {
            (Some((pj, pb)), Some(r)) if pj + 1 == j && pb == o.back => r[1] = j,
            _ => out.push([j, j]),
        }
        prev = Some((j, o.back));
    }
    out.retain(|r| back.is_none_or(|b| map[r[0]].back == b));
    out
}

/// id のブロック・群の往路の最初の範囲
pub(crate) fn fwd_run(map: &[SurfOrigin], id: &str) -> Option<[usize; 2]> {
    block_runs(map, id, Some(false)).into_iter().next()
}

/// id のある空気のブロックが入る面: (id, 面番号, 向き)。向きは鏡の後（光が −z へ進む）で −1（面の厚さは −t）。
/// 往復で 2 回通る空気は 2 つ並ぶ。面の厚さには、同じ面に入る他の空気・群の移動も足されている
pub type Gap = (String, usize, f64);

/// プロジェクトを順次の面の列の系に展開する。誤りはブロックの中で起きたらその位置を持つ（[`ExpandError`]。
/// `?` で [`lensbench_core::Error`] に直すとブロックの名前が文の前に付く）
pub fn expand(p: &Project, env: &Env, o: &ExpandOpts) -> Result<Expansion, ExpandError> {
    let with;
    let p = match o.state {
        Some(n) => {
            with = p.with_state(n)?;
            &with
        }
        None => p,
    };
    match expand_inner(
        p,
        env,
        o.path.or(p.path.as_deref()),
        false,
        o.cache,
        o.within,
    )? {
        Walk::Done(x) => Ok(*x),
        _ => Err(lensbench_core::Error::from(
            "internal error: the BS path is short outside the path search",
        )
        .into()),
    }
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

/// 展開の行き着いた先
pub(crate) enum Walk {
    /// 経路の文字が足りない（経路を探すときだけ）
    Need,
    /// 経路を探すとき: ブロックを終わりまでたどれた（系は組まない）
    Walked,
    Done(Box<Expansion>),
}

/// 展開の本体。search（経路を探す）なら、経路の文字が足りなければ [`Walk::Need`]、ブロックを終わりまで
/// たどれたら系を組まずに [`Walk::Walked`]
pub(crate) fn expand_inner(
    p: &Project,
    env: &Env,
    path: Option<&str>,
    search: bool,
    cache: Option<&SourceCache>,
    within: &[Loading],
) -> Result<Walk, ExpandError> {
    let mut failed = None;
    expand_body(p, env, path, search, cache, within, &mut failed).map_err(|error| ExpandError {
        error,
        block: failed.map(|n| BlockAt::new(&p.blocks, n)),
    })
}

fn expand_body(
    p: &Project,
    env: &Env,
    path: Option<&str>,
    search: bool,
    cache: Option<&SourceCache>,
    within: &[Loading],
    failed: &mut Option<NodePath>,
) -> lensbench_core::Result<Walk> {
    {
        let mut ids = Vec::new();
        splitter_ids(&p.blocks, &mut ids);
        for (i, id) in ids.iter().enumerate() {
            if id.is_empty() {
                return Err("a BS (splitter) needs an id".into());
            }
            if ids[..i].contains(id) {
                return Err(format!("BS id {id} appears twice or more").into());
            }
        }
    }
    if p.wavelengths.is_empty() {
        return Err("no wavelengths".into());
    }
    if p.primary == 0 || p.primary > p.wavelengths.len() {
        return Err(format!(
            "primary wavelength {} (there are {} wavelengths)",
            p.primary,
            p.wavelengths.len()
        )
        .into());
    }
    let mut dirs = env.glass_dirs.clone();
    crate::model::zmx_dirs(&p.blocks, &env.base, &mut dirs);
    let dirs = glass::catalog_dirs(&dirs);
    let (glasses, missing) = glass::load_catalogs_shared(&p.glass_catalogs, &dirs);
    if !missing.is_empty() {
        return Err(lensbench_core::Error::Invalid(format!(
            "glass catalog not found: {}. Pass catalog folders with --glass-dir (CLI), glass_dirs (serve, Python) or LENSBENCH_GLASSCAT",
            missing.join(", ")
        ))
        .with_code(code::GLASS_NOT_FOUND));
    }
    // 面の上書き・操作を持つブロックに番号を付けた写し（無ければそのまま）
    let keyed;
    let p = match number_sites(p) {
        Some(q) => {
            keyed = q;
            &keyed
        }
        None => p,
    };
    let mut sites = Vec::new();
    face_sites(&p.blocks, &mut sites);
    let mut c = Ctx {
        env,
        cache,
        base: env.base.clone(),
        loading: within.to_vec(),
        outer: within.len(),
        sf: Vec::new(),
        map: Vec::new(),
        noted: Vec::new(),
        nflip: 0,
        flip_err: None,
        zpicks: Vec::new(),
        lead: 0.0,
        parity: 1.0,
        glasses,
        coats: HashMap::new(),
        nstop: 0,
        gaps: Vec::new(),
        split: SplitState {
            path: splitter::parse_path(path.unwrap_or(""))?,
            strict: search,
            ..Default::default()
        },
        frames: Vec::new(),
        zenv: Vec::new(),
        nodes: node_paths(&p.blocks),
        leaf: Vec::new(),
        failed: None,
    };
    for (i, b) in p.blocks.iter().enumerate() {
        if c.split.done {
            break;
        }
        // 検出器（最後のブロック）は面を足さない。像面に置く
        if matches!(b, Block::Detector(_)) && i + 1 == p.blocks.len() {
            continue;
        }
        match c.block(b, &p.blocks, i) {
            // 経路の文字が足りない（経路を探すとき。paths が次の文字を試す）
            Err(_) if c.split.need => return Ok(Walk::Need),
            Err(e) => {
                *failed = c.failed.take();
                return Err(e);
            }
            Ok(()) => {}
        }
        if let Some(e) = c.flip_err.take() {
            return Err(e);
        }
    }
    if c.split.pos < c.split.path.len() {
        return Err(format!(
            "path {}: too many letters (the BS is passed {} times)",
            path.unwrap_or(""),
            c.split.pos
        )
        .into());
    }
    // 絞りを面に置く操作（surface_ops の set_stop）があれば、ブロックの絞りは無くてよい
    let stop_op = sites.iter().any(|(_, f)| {
        f.surface_ops
            .iter()
            .any(|o| matches!(o, SurfaceOp::SetStop(_)))
    });
    if c.nstop > 1 || (c.nstop == 0 && !stop_op) {
        return Err(format!("{} stops (exactly one is needed)", c.nstop).into());
    }
    if search {
        return Ok(Walk::Walked);
    }
    if c.nstop == 0 {
        // 仮の絞り（set_stop で移す）
        if let Some(f) = c.sf.first_mut() {
            f.stop = true;
        }
    }
    let wl: Vec<f64> = p.wavelengths.iter().map(|w| w.um).collect();
    let wt: Vec<f64> = p.wavelengths.iter().map(|w| w.weight).collect();
    let mut obj = crate::zmx_links::object_surface(p.object.as_deref())?;
    obj.t = p.object_distance.unwrap_or(f64::INFINITY);
    let mut sf = vec![obj];
    if p.object_distance.is_some() {
        sf[0].t += c.lead;
    }
    if c.sf.len() != c.map.len() {
        // 面は put か element_from で足す（出どころの表を面と揃える）
        return Err(format!(
            "internal error: {} surfaces but {} sources",
            c.sf.len(),
            c.map.len()
        )
        .into());
    }
    sf.append(&mut c.sf);
    let org: Vec<Option<(usize, usize)>> = c.map.iter().map(|o| o.zmx).collect();
    // 系の面ごとの出どころ（物体面・像面はブロックの外）
    let mut map = vec![SurfOrigin::default()];
    map.append(&mut c.map);
    map.push(SurfOrigin::default());
    let zpicks = std::mem::take(&mut c.zpicks);
    sf.push(match &p.image {
        None => Surface::default(),
        Some(r) => {
            if r.stop
                || r.object
                || r.t != 0.0
                || r.medium.is_some()
                || r.coordinate_break.is_some()
            {
                return Err(
                    "image: stop, object, t, medium and coordinate_break can't be written".into(),
                );
            }
            let mut r = (**r).clone();
            r.image = false;
            crate::sysfile_v2::read::surface(r).map_err(|e| format!("image: {e}"))?
        }
    });
    name_faces(&mut map, &sf, &c.nodes);
    // 面の後の媒質が決まるか（屈折率は系を作るときに解く。GRIN の面の材料名は分布の名前でガラスではない）
    let envr = p.environment;
    for (j, d) in sf.iter_mut().enumerate() {
        let known = (j > 0 && (d.is_mirror() || d.is_coord_break()))
            || is_air(&d.mat)
            || d.mat.glass().is_some_and(|g| c.glasses.contains_key(g))
            || matches!(d.mat, Material::Index(_) | Material::Model { .. })
            || d.grin().is_some();
        if !known {
            return Err(glass_missing(j, &d.mat.name()));
        }
        // 複屈折材の板の入口: 異常光の屈折率
        if let Some(lensbench_core::surface::Bire::In { ne, .. }) = d.bire.as_mut() {
            let en = lensbench_core::surface::Bire::e_name(&d.mat.name());
            let g = c.glasses.get(&en).ok_or_else(|| {
                format!(
                    "surface {j}: extraordinary glass {en} of birefringent material {} is not in the catalogs",
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
            r.resolve(&c.glasses)
                .map_err(|e| format!("surface {j}: {e}"))?;
            let m = r.jones8(r.wl0);
            if let Some(j) = d.jones_mut() {
                *j = m;
            }
        }
        // 屈折面での反射（ビームスプリッター）: 反対側の媒質の屈折率
        if let Some(r) = &mut d.reflect {
            r.n = if is_air_name(&r.mat) {
                vec![1.0; wl.len()]
            } else if let Some(g) = c.glasses.get(r.mat.trim()) {
                wl.iter()
                    .map(|&l| g.index(l, envr.temperature_c, envr.pressure_atm, envr.adjust_index))
                    .collect()
            } else {
                return Err(format!("surface {j}: glass {} is not in the catalogs", r.mat).into());
            };
        }
    }
    // 条件で定める視野の面の名前は展開した後で引く
    let ft = p.fields.kind.plain().unwrap_or(FieldType::Angle);
    if p.fields.vignetting.len() > p.fields.points.len() {
        return Err(format!(
            "fields: {} vignetting rows for {} fields",
            p.fields.vignetting.len(),
            p.fields.points.len()
        )
        .into());
    }
    let fields: Vec<Field> = p
        .fields
        .points
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let mut f = Field::xy(q[0], q[1]);
            f.w = p.fields.weights.get(i).copied().unwrap_or(1.0);
            if let Some(&[vdx, vdy, vcx, vcy, van]) = p.fields.vignetting.get(i) {
                (f.vdx, f.vdy, f.vcx, f.vcy, f.van) = (vdx, vdy, vcx, vcy, van);
            }
            f
        })
        .collect();
    let mut s = System::new(
        sf,
        wl,
        wt,
        p.primary - 1,
        fields,
        ft,
        p.fields.norm,
        (p.aperture.kind, p.aperture.value),
        p.telecentric,
        p.aiming,
        c.glasses,
    )?
    .into_prescription();
    s.name = p.name.clone();
    // 同梱のカタログ（屈折率だけ）から取ったガラスを知らせる
    let b = glass::builtin::note_for(&s, &p.glass_catalogs, &dirs);
    s.import_notes.extend(b);
    s.temperature = envr.temperature_c;
    s.pressure = envr.pressure_atm;
    s.adjust_index = envr.adjust_index;
    s.floating_clip = p.floating_clip;
    s.apodization = p.apodization;
    s.ropd = p.ropd;
    // 膜: プロジェクトの coatings はカタログの同じ名前より優先
    s.coat_tables = c.coats;
    for (k, spec) in &p.coatings {
        let t = spec.table().map_err(|e| format!("coating {k}: {e}"))?;
        s.coat_tables.insert(k.clone(), t);
    }
    if let Some(u) = &p.afocal {
        s.afocal = true;
        s.afocal_unit = AfocalUnit::from_code(u)
            .ok_or_else(|| format!("afocal unit {u} (ur, mr, rr, as, am or dg)"))?;
    }
    if p.fields.kind == FieldKind::Solve {
        let Some(c) = &p.fields.solve else {
            return Err("when the fields type is solve, write the condition in solve".into());
        };
        let f = |t: &str| {
            s.find_surface(t)
                .map_err(|e| format!("field condition: {e}"))
        };
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
    } else if p.fields.solve.is_some() {
        return Err("fields solve only with type solve".into());
    }
    let mut s = s.build_raw()?;
    let mut warn = Note::all(NoteKind::Pickup, None, keep_pickups(&mut s, &org, &zpicks)?);
    for (path, t, pr, adj, _) in &c.zenv {
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
            warn.push(Note::new(NoteKind::Environment, None, format!(
                "{path}: the .zmx environment ({t} °C, {pr} atm) differs from the project environment ({} °C, {} atm); indices are computed in the project environment (change it with environment)",
                e.temperature_c, e.pressure_atm
            )));
        }
    }
    // 玉・群のパラメータ: 玉モードの玉 lens1 … と、id のあるブロック・群（同じ id の範囲は 1 つの玉・群）
    let mut rx = s.prescription().clone();
    rx.units = s.lens_units();
    // 取り込んだ .zmx の膜ファイル（面の膜の名前はそのファイルで引く）。違うファイルの .zmx が 2 つ以上なら最初のもの
    crate::zmx_links::carry_notes(&mut rx, &zpicks);
    let mut cofs = c.zenv.iter().filter_map(|z| Some((&z.0, z.4.as_ref()?)));
    if let Some((_, f)) = cofs.next() {
        rx.coating_file = Some(f.clone());
        if let Some((path, g)) = cofs.find(|(_, g)| *g != f) {
            warn.push(Note::new(NoteKind::Coating, None, format!(
                "{path}: the coating file {} differs from {} of an earlier .zmx; coatings are looked up in {}",
                g.display(),
                f.display(),
                f.display()
            )));
        }
    }
    let mut s = rx.build_raw()?;
    for id in &c.noted {
        let id = id.as_str();
        let spans = block_runs(&map, id, None);
        match s.define_unit(id, &spans, [0.0; 3]) {
            // 群の移動はもう厚さに入っているので、その分を玉・群の shift の今の値にする（状態ごとの群の位置を
            // `id.shift` で比べられるように）
            Ok(u) => {
                let shift = group_shift(&p.blocks, id).unwrap_or(0.0);
                s.edit_raw(|rx| {
                    rx.units[u].shift = shift;
                    Ok(())
                })?
            }
            Err(e) => warn.push(Note::new(
                NoteKind::Unit,
                Some(id.to_string()),
                format!("block {id} is not made a lens or group parameter: {e}"),
            )),
        }
    }
    // 裏返して置いた玉（id の無い玉の lens1 … も）: 最初の範囲の面（偏心・傾きの座標ブレークを除く）が、ちょうど
    // 1 つの裏返した玉の面すべて（前は座標ブレークを含めて範囲を比べたので、偏心した玉は立たなかった）
    let flipped = |s: &System, r: &[usize; 2]| -> bool {
        let faces: Vec<usize> = (r[0]..=r[1].min(map.len() - 1))
            .filter(|&j| !s.surfaces[j].is_coord_break())
            .collect();
        let Some(k) = faces.first().and_then(|&j| map[j].flip) else {
            return false;
        };
        faces.iter().all(|&j| map[j].flip == Some(k))
            && map.iter().filter(|o| o.flip == Some(k)).count() == faces.len()
    };
    let marks: Vec<bool> = s
        .units
        .iter()
        .map(|u| u.spans.first().is_some_and(|r| flipped(&s, r)))
        .collect();
    s.edit_raw(|rx| {
        for (u, f) in rx.units.iter_mut().zip(marks) {
            u.flipped = f;
        }
        Ok(())
    })?;
    apply_surface_edits(&mut s, &sites, &map)?;
    let (w, stopped) = apply_surface_ops(&mut s, &sites, &map)?;
    if c.nstop == 0 && !stopped {
        // 仮の絞りのまま（set_stop のブロックが無い）にはしない
        return Err("0 stops (no set_stop block in surface_ops; exactly one is needed)".into());
    }
    warn.extend(w);
    if let Some(r) = &p.glrs {
        let g = match r {
            BlockRef::Sys(r) => Some(edit_ops::surf_of(&s, r, "glrs")?),
            BlockRef::Block(f) => {
                let j =
                    block_face(&s, &f.block, f.surface, &map).map_err(|x| format!("glrs: {x}"))?;
                if j.is_none() {
                    warn.push(Note::new(
                        NoteKind::Glrs,
                        Some(f.block.clone()),
                        format!(
                            "glrs: block {} not found, so the global reference stays at surface {}",
                            f.block, s.glrs
                        ),
                    ));
                }
                j
            }
        };
        if let Some(g) = g {
            s.edit_raw(|rx| {
                rx.glrs = g;
                Ok(())
            })?;
            // ZPL マクロは大域座標（RAGX など）を読めるので、基準の面を決めてから解き直す
            if !s.macros.is_empty() {
                s.resolve_links()?;
            }
        }
    }
    // 自動の半直径を光線から求める。機構部品の検査・描画・書き出しが使う
    s.update_semi();
    let mut mw = Vec::new();
    let mut mech = place_mechanics(&s, &p.mechanics, env, &map, &mut mw)?;
    mech.extend(place_frames(&s, &c.frames, &map, &mut mw));
    warn.extend(Note::all(NoteKind::Mechanics, None, mw));
    s.edit_raw(|rx| {
        rx.mechanics = mech;
        Ok(())
    })?;
    s.validate()?;
    Ok(Walk::Done(Box::new(Expansion {
        system: s,
        notes: warn,
        gaps: c.gaps,
        map,
    })))
}

/// 機構部品を系の面に置く（[`Project::mechanics`]）。ブロックが無い（状態で外したなど）部品は置かずに注意を出す
pub(crate) fn place_mechanics(
    s: &System,
    ms: &[MechPlacement],
    env: &Env<'_>,
    map: &[SurfOrigin],
    warn: &mut Vec<String>,
) -> lensbench_core::Result<Vec<lensbench_core::mech::MechPart>> {
    let mut out = Vec::new();
    // 置いた部品の寸法（after でねじを見る）
    let mut specs: Vec<MechSpec> = Vec::new();
    for (k, m) in ms.iter().enumerate() {
        let tag = format!("mechanics {}", k + 1);
        // 取ってキャッシュにある品の STEP（詳しい形の描画に使う）
        let mut step_file = String::new();
        let (spec, item_name) = match (&m.mech, &m.catalog, &m.item) {
            (Some(x), None, None) => (x.clone(), String::new()),
            (None, Some(cat), Some(item)) => {
                let path = env.base.join(cat);
                let c = UserCatalog::load_shared(&path)?;
                let it = c
                    .items
                    .get(item)
                    .ok_or_else(|| format!("{tag}: no item {item} in {}", path.display()))?;
                let x = it.mech.clone().ok_or_else(|| {
                    format!("{tag}: item {item} is not a mechanical part (no mech)")
                })?;
                // URL の無い品も、共有の置き場から取ってあればキャッシュにある（引き方は呼ぶ側が渡す）
                if let Some(f) = env.step_files.and_then(|look| look(item, &it.cad.step)) {
                    step_file = f.display().to_string();
                }
                (x, item.clone())
            }
            _ => {
                return Err(format!(
                    "{tag}: write a part with mech, or with catalog and item, not both"
                )
                .into())
            }
        };
        spec.validate().map_err(|e| format!("{tag}: {e}"))?;
        // 裏返す（z の大きい端から光が入る）。座に合わせるなら、座は保持する素子の後ろの面に当たる
        let spec = if m.reversed {
            let t = if m.seat {
                m.holds
                    .first()
                    .and_then(|b| fwd_run(map, b))
                    .map_or(0.0, |x| {
                        (x[0]..x[1])
                            .filter(|&j| j < s.surfaces.len() && !s.surfaces[j].is_coord_break())
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
                return Err(format!("{tag}: after can't be used with at or seat").into());
            }
            if m.tilt != [0.0, 0.0] {
                return Err(format!(
                    "{tag}: tilt can't be used with after (it takes the previous part's tilt)"
                )
                .into());
            }
            let Some((prev, prev_spec)) = out
                .iter()
                .zip(&specs)
                .rev()
                .find(|(p, _): &(&lensbench_core::mech::MechPart, _)| &p.name == prev_name)
            else {
                return Err(format!("{tag}: the after part {prev_name} is not before it").into());
            };
            let (prev, prev_spec): (&lensbench_core::mech::MechPart, &MechSpec) = (prev, prev_spec);
            let overlap = match (&prev_spec.back_thread, &spec.front_thread) {
                (Some(a), Some(b)) => {
                    if a.thread != b.thread || a.male == b.male {
                        let kind = |t: &ThreadEnd| {
                            if t.male {
                                format!("{} male", t.thread)
                            } else {
                                format!("{} female", t.thread)
                            }
                        };
                        warn.push(format!(
                            "{tag}: the threads of the back end of {prev_name} ({why}) and the front end of {name} ({hint}) don't match",
                            why = kind(a),
                            hint = kind(b)
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
                return Err(format!("{tag}: offset is not a number").into());
            }
            for b in &m.holds {
                match fwd_run(map, b) {
                    Some(x) => part0
                        .holds
                        .extend((x[0]..=x[1]).filter(|&j| !s.surfaces[j].is_coord_break())),
                    None => warn.push(format!("{tag}: no block {b} for holds")),
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
                .ok_or_else(|| {
                    format!("{tag}: seat needs holds (the seat position) in the part size")
                })?
                .seat_z;
            let b = m
                .holds
                .first()
                .ok_or_else(|| format!("{tag}: seat needs holds (the held element)"))?;
            offset[2] -= seat;
            match block_face(s, b, 0, map).map_err(|e| format!("{tag}: {e}"))? {
                Some(j) => Some(j),
                None => {
                    warn.push(format!("{tag}: no block {b}, not placed"));
                    continue;
                }
            }
        } else {
            None
        };
        let surface = match (seat_at, &m.at.block) {
            (Some(j), _) => j,
            (None, block) => match block {
                Some(b) => {
                    match block_face(s, b, m.at.surface, map).map_err(|e| format!("{tag}: {e}"))? {
                        Some(j) => j,
                        None => {
                            warn.push(format!("{tag}: no block {b}, not placed"));
                            continue;
                        }
                    }
                }
                None => {
                    if m.at.is_unset() {
                        return Err(format!("{tag}: a place (at) or seat is needed").into());
                    }
                    if m.at.surface == 0 || m.at.surface > s.image() {
                        return Err(format!(
                            "{tag}: surface {} (1 to {})",
                            m.at.surface,
                            s.image()
                        )
                        .into());
                    }
                    m.at.surface
                }
            },
        };
        if !offset.iter().chain(&m.tilt).all(|v| v.is_finite()) {
            return Err(format!("{tag}: offset or tilt is not a number").into());
        }
        let mut part = spec.part(&name, surface, offset, m.tilt);
        for b in &m.holds {
            match fwd_run(map, b) {
                Some(x) => part
                    .holds
                    .extend((x[0]..=x[1]).filter(|&j| !s.surfaces[j].is_coord_break())),
                None => warn.push(format!("{tag}: no block {b} for holds")),
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
/// 保持する素子はその品の面（map で品の番号を持つ面）
pub(crate) fn place_frames(
    s: &System,
    frames: &[(String, MechSpec)],
    map: &[SurfOrigin],
    warn: &mut Vec<String>,
) -> Vec<lensbench_core::mech::MechPart> {
    let mut out = Vec::new();
    for (k, (name, f)) in frames.iter().enumerate() {
        let tag = format!("{name} mount");
        if let Err(e) = f.validate() {
            warn.push(format!("{tag}: {e} (not placed)"));
            continue;
        }
        let faces: Vec<usize> = (0..s.surfaces.len().min(map.len()))
            .filter(|&j| map[j].frame == Some(k) && !s.surfaces[j].is_coord_break())
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
) -> lensbench_core::Result<Vec<String>> {
    let mut at: HashMap<(usize, usize), usize> = HashMap::new();
    for (i, o) in org.iter().enumerate() {
        if let Some(k) = o {
            // 同じ取り込みで同じ面が 2 回出ることはない
            at.insert(*k, i + 1);
        }
    }
    crate::zmx_links::carry_object(s, zpicks, &mut at)?;
    let mut warn = Vec::new();
    for (inst, z) in zpicks.iter().enumerate() {
        let (path, picks) = (&z.path, &z.pickups);
        for q in picks {
            let (Some(&dst), src) = (at.get(&(inst, q.dst)), at.get(&(inst, q.src))) else {
                continue; // 先が取り込んでいない面
            };
            let Some(&src) = src else {
                warn.push(format!(
                    "{path}: surface {} {:?} is a pickup from surface {} outside the range (fixed to the value)",
                    q.dst, q.what, q.src
                ));
                continue;
            };
            // 鏡のあと（向きが逆、sign = −1）で取り込むと、曲率・厚さの符号が変わる。先 dst' = σd·dst、元 src' = σs·src
            // （σ は量の符号が変われば sign、変わらなければ 1）なら、dst = a·src + b は dst' = (σd·σs·a)·src' + σd·b。
            // 向きから決めた組を先に試し、合わなければ（面タイプのパラメータの意味は型で違う）ほかの符号の組を試す
            use lensbench_core::param::Kind as Pk;
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
                Some(0) => s.edit_raw(|rx| {
                    rx.pickups.push(cands[0]);
                    Ok(())
                })?,
                Some(k) => {
                    // 向きから決めた組と違う符号で値が合った（元の値が 0 だと符号を区別できない）
                    s.edit_raw(|rx| {
                        rx.pickups.push(cands[k]);
                        Ok(())
                    })?;
                    warn.push(format!(
                        "{path}: the sign of the scale and offset of surface {} {:?} (from surface {}) was chosen by matching values (scale {}, offset {}); check it is as intended",
                        q.dst, q.what, q.src, cands[k].scale, cands[k].offset
                    ));
                }
                None => warn.push(format!(
                    "{path}: the value relation of surface {} {:?} (from surface {}) changed in the expansion (fixed to the value)",
                    q.dst, q.what, q.src
                )),
            }
        }
    }
    if !s.pickups.is_empty() {
        s.resolve_links()?;
    }
    let moff = crate::zmx_links::carry_macros(s, zpicks, &at)?;
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
            use lensbench_core::solves::SolveKind as Sk;
            let mut ok = true;
            // 位置ソルブは高さを見ないので次の面は要らない（基準の面との間が並んでいればよい）
            if !q.kind.is_curv() && !matches!(q.kind, Sk::Position { .. }) {
                ok &= same(q.surf + 1) || (q.surf + 1 == z.image && k + 1 == s.image());
            }
            if matches!(q.kind, Sk::ElementPower { .. }) {
                ok &= q.surf >= 1 && same(q.surf - 1);
            }
            let mut kind = q.kind;
            crate::zmx_links::shift_macro(&mut kind, moff[inst]);
            if let Some(o) = q.kind.other_surface() {
                ok &= same(o);
                // 基準の面との距離で決まるものは、間の面（基準の面が後ろでも）も並んで取り込まれていること
                if matches!(
                    q.kind,
                    Sk::Position { .. }
                        | Sk::CenterOfCurvature { .. }
                        | Sk::ConcentricSurface { .. }
                        | Sk::ConcentricRadius { .. }
                ) {
                    ok &= (o.min(q.surf)..=o.max(q.surf)).all(same);
                }
                if let Some(&n) = at.get(&(inst, o)) {
                    kind = kind.with_other_surface(n);
                }
            }
            if ok {
                s.edit_raw(|rx| {
                    rx.solves.push(Solve { surf: k, kind });
                    Ok(())
                })?;
            } else {
                warn.push(format!(
                    "{}: the solve of surface {} ({:?}) involves surfaces outside the range or far away (fixed to the value)",
                    z.path, q.surf, q.kind
                ));
            }
        }
    }
    if !s.solves.is_empty() || !s.macro_parms.is_empty() {
        let val = |s: &System, q: &Solve| {
            let d = &s.surfaces[q.surf];
            if q.kind.is_curv() {
                d.c
            } else {
                d.t
            }
        };
        let before: Vec<f64> = s.solves.iter().map(|q| val(s, q)).collect();
        s.resolve_links()?;
        for (q, b) in s.solves.iter().zip(before) {
            let a = val(s, q);
            if (a - b).abs() > 1e-9 * (1.0 + b.abs()) {
                let what = if q.kind.is_curv() {
                    "curvature"
                } else {
                    "thickness"
                };
                warn.push(format!(
                    "solved the {what} of surface {} again with the solve ({}, value {}): {b} → {a}",
                    q.surf,
                    lensbench_core::solves::solve_kind_info(q.kind).0,
                    q.kind.value()
                ));
            }
        }
    }
    Ok(warn)
}

/// 面 j のガラス mat がカタログに無い誤り（符号 [`code::GLASS_NOT_FOUND`]）
fn glass_missing(j: usize, mat: &str) -> lensbench_core::Error {
    lensbench_core::Error::Invalid(format!(
        "surface {j}: glass {mat} is not in the catalogs{HINT}"
    ))
    .with_code(code::GLASS_NOT_FOUND)
}
