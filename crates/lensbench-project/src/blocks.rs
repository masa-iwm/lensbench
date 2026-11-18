//! ブロックごとの展開（玉・空気・絞り・鏡・面の並び・.zmx・ユーザーカタログ・群）。展開の途中の状態 `Ctx`

use super::*;
use lensbench_core::sync::LockExt;
/// レンズカタログを引く関数: (メーカー, 型番) → .zmx の本文
pub type CatalogLookup<'a> = &'a dyn Fn(&str, &str) -> lensbench_core::Result<String>;

/// 展開でファイル・カタログから読んだものの使い回し（[`ExpandOpts::cache`]）。
/// - カタログの玉: (メーカー, 型番) → (玉, 玉の .zmx のガラス)
/// - `.zmx`・系ファイルのブロック: (正規化したパス, 構成番号) → 系。ファイルの更新時刻と大きさが変われば読み直す
///
/// 同じ環境（ガラスのフォルダ・カタログ）の展開の間だけで使う。状態・経路・差し替えの組・GUI の編集ごとの展開が、
/// 同じファイルを読んで解析し直さないため（入れ子のプロジェクトは中の葉だけ覚える）
#[derive(Default)]
pub struct SourceCache {
    lenses: std::sync::Mutex<HashMap<(String, String), CachedLens>>,
    files: std::sync::Mutex<HashMap<(PathBuf, usize), (lensbench_core::vfs::Stamp, System)>>,
    scope: std::sync::Mutex<String>,
}

type CachedLens = std::sync::Arc<(Element, std::sync::Arc<HashMap<String, Glass>>)>;

impl std::fmt::Debug for SourceCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceCache")
            .field("lenses", &self.lenses.lock_ok().len())
            .field("files", &self.files.lock_ok().len())
            .finish()
    }
}

impl SourceCache {
    /// 覚えているファイルの数（試験用）
    pub fn file_count(&self) -> usize {
        self.files.lock_ok().len()
    }

    /// 環境（ガラスのフォルダ・レンズカタログなど）を表す文字列 key が前と違えば、覚えたものを捨てる。
    /// 環境を変えうる呼び手（[`crate::open::Source`]）が展開の前に呼ぶ
    pub fn scope(&self, key: &str) {
        let mut s = self.scope.lock_ok();
        if *s != key {
            self.lenses.lock_ok().clear();
            self.files.lock_ok().clear();
            *s = key.to_string();
        }
    }

    fn file(&self, k: &(PathBuf, usize), st: &lensbench_core::vfs::Stamp) -> Option<System> {
        match self.files.lock_ok().get(k) {
            Some((s, sys)) if s == st => Some(sys.clone()),
            _ => None,
        }
    }
}

/// 機構部品の詳しい形の STEP のファイルを引く関数: (品の名前, STEP の公式の URL) → 手元のファイル
/// （lensbench-cad の `cached_step` は取った STEP のキャッシュから引く）
pub type StepLookup<'a> = &'a (dyn Fn(&str, &str) -> Option<PathBuf> + Sync);

/// 展開の環境（既定はフォルダ・カタログ・STEP の口なし）
#[derive(Default)]
pub struct Env<'a> {
    /// 相対パスの基準（プロジェクトファイルのフォルダ）
    pub base: PathBuf,
    pub glass_dirs: Vec<PathBuf>,
    pub catalog: Option<CatalogLookup<'a>>,
    /// カタログの機構部品の詳しい形の STEP（[`lensbench_core::mech::MechPart::step`]）を引く口。None なら引かない
    /// （描画・書き出しは簡易外形。以前はキャッシュのフォルダをこのクレートの中で決めて探していた）
    pub step_files: Option<StepLookup<'a>>,
}

impl<'a> Env<'a> {
    /// プロジェクトファイル path の環境（相対パスはそのフォルダから。STEP は引かない）
    pub fn for_file(
        path: &Path,
        glass_dirs: &[PathBuf],
        catalog: Option<CatalogLookup<'a>>,
    ) -> Env<'a> {
        Env::in_dir(path.parent().unwrap_or(Path::new("")), glass_dirs, catalog)
    }

    /// 相対パスをフォルダ base から引く環境（STEP は引かない）
    pub fn in_dir(
        base: &Path,
        glass_dirs: &[PathBuf],
        catalog: Option<CatalogLookup<'a>>,
    ) -> Env<'a> {
        Env {
            base: base.to_path_buf(),
            glass_dirs: glass_dirs.to_vec(),
            catalog,
            step_files: None,
        }
    }
}

pub(crate) struct Ctx<'a, 'b> {
    pub(crate) env: &'a Env<'b>,
    pub(crate) cache: Option<&'a SourceCache>,
    /// 相対パスの基準（ユーザーカタログの項目の中ではカタログのフォルダ）
    pub(crate) base: PathBuf,
    /// 展開中のファイル・ユーザーカタログの項目（外側の展開から受けたものが先）。品と取り込んだプロジェクトの
    /// 両方の循環をここで見つける
    pub(crate) loading: Vec<Loading>,
    /// loading のうち外側の展開から受けた数（この展開で積んだ項目は後ろ）
    pub(crate) outer: usize,
    pub(crate) sf: Vec<Surface>,
    /// sf の各面の出どころ（[`SurfOrigin`]: 面を足したブロック、往路・復路、.zmx の元の面）。sf と同じ長さ
    pub(crate) map: Vec<SurfOrigin>,
    /// 面を覚えた id のあるブロック・群（覚えた順）。玉・群のパラメータの順
    pub(crate) noted: Vec<String>,
    /// 裏返して置いた玉の数（[`SurfOrigin::flip`] の番号）
    pub(crate) nflip: usize,
    /// 鏡の後で面を裏返せなかった最初の誤り（裏返しに対応しない利用者定義の面）。ブロックを足し終えたら返す
    pub(crate) flip_err: Option<lensbench_core::Error>,
    /// zmx ブロックの取り込みごとの、元の系のピックアップ
    /// zmx ブロックごとのピックアップと厚さのソルブ
    pub(crate) zpicks: Vec<ZmxLinks>,
    /// 物体面の後の、最初の面までの距離（最初のブロックの前の空気）
    pub(crate) lead: f64,
    /// 鏡の数の偶奇（−1 なら光は −z へ進む）
    pub(crate) parity: f64,
    /// 系のガラス（プロジェクトのカタログ＋玉・ユーザーカタログで足したもの）。足す名前がなければ共有のまま
    pub(crate) glasses: std::sync::Arc<HashMap<String, Glass>>,
    /// ユーザーカタログの `coatings`（TABLE 膜）
    pub(crate) coats: HashMap<String, lensbench_core::coating::Table>,
    pub(crate) nstop: usize,
    /// id のある空気のブロックが入った面（id, 面番号, 向き）。差し替え探索の間隔の変数にする
    pub(crate) gaps: Vec<Gap>,
    /// ビームスプリッターの経路
    pub(crate) split: SplitState,
    /// 枠付きのユーザーカタログの品（名前, 枠）。品の面は [`SurfOrigin::frame`] の番号で引く（面を巻き戻せば品も外れる）
    pub(crate) frames: Vec<(String, MechSpec)>,
    /// 取り込んだ .zmx の環境（パス, 温度, 気圧, 補正, 膜ファイル）。プロジェクトの環境と違えば注意を出す。
    /// 膜ファイルは展開した系の膜ファイルになる
    pub(crate) zenv: Vec<(String, f64, f64, bool, Option<std::path::PathBuf>)>,
    /// プロジェクトの木のブロック（アドレス）とその位置（[`node_paths`]）。木の外（ユーザーカタログの品の中、写した BS の腕）は無い
    pub(crate) nodes: HashMap<usize, NodePath>,
    /// たどっている木のブロック（外から内へ。アドレス）。足した面の出どころ [`SurfOrigin::leaf`] は最後のもの
    pub(crate) leaf: Vec<usize>,
    /// 誤りが起きたいちばん内の木のブロック（[`ExpandError::block`]）
    pub(crate) failed: Option<NodePath>,
}

/// 品の枠（向きを当てる前）と向き
pub(crate) type ItemFrame = Option<(MechSpec, bool)>;

pub(crate) fn is_air(m: &Material) -> bool {
    m.is_air()
}

/// 名前（膜の反射の反対側の媒質など）が空気か
pub(crate) fn is_air_name(m: &str) -> bool {
    m.trim().is_empty() || m.trim().eq_ignore_ascii_case("AIR")
}

/// 結晶板の面の元（波長板の面の半直径・開口を写した平面の標準面）
pub(crate) fn plate_face(s: &Surface) -> Surface {
    Surface {
        semi: s.semi,
        semi_auto: s.semi_auto,
        ap: s.ap,
        ..Default::default()
    }
}

/// cos・sin の丸め誤差（6e-17 など）を 0・±1 に
pub(crate) fn round_unit(v: f64) -> f64 {
    if v.abs() < 1e-15 {
        0.0
    } else if (v.abs() - 1.0).abs() < 1e-15 {
        v.signum()
    } else {
        v
    }
}

/// 強度透過率 T の板の外側の面 1 つの理想膜（√T。OpticStudio の I.xx と同じ書き方で、1 は I.99999999）
pub(crate) fn ideal_face(t: f64) -> String {
    let f = (t.sqrt() * 1e8).floor().min(99_999_999.0) as u64;
    format!("I.{f:08}")
}

/// 面の並びのブロックの面の後の媒質: 空気（座標ブレークも）のほかは、複屈折材の板の入口の後（結晶）と、
/// 貼り合わせの出口の後（次の入口へ厚さ 0）だけ。板の入口と出口は交互に並び、最後は空気
pub(crate) fn check_crystal_order(v: &[Surface]) -> lensbench_core::Result<()> {
    use lensbench_core::surface::Bire;
    let mut inside = false;
    for (i, s) in v.iter().enumerate() {
        match (&s.bire, inside) {
            (Some(Bire::In { .. }), false) => inside = true,
            (Some(Bire::Out), true) => {
                inside = false;
                if !is_air(&s.mat) {
                    let next_in = matches!(
                        v.get(i + 1).and_then(|n| n.bire.as_ref()),
                        Some(Bire::In { .. })
                    );
                    if !next_in || s.t != 0.0 || v[i + 1].mat != s.mat {
                        return Err(
                            "the glass after the exit of a birefringent plate goes to the entrance of the next plate of the same material with thickness 0"
                                .into(),
                        );
                    }
                }
            }
            (Some(Bire::In { .. }), true) => {
                return Err("two birefringent plate entrances in a row (an exit is needed)".into())
            }
            (Some(Bire::Out), false) => {
                return Err("no entrance before the exit of a birefringent plate".into())
            }
            (None, true) => {
                return Err(
                    "no other surface can go between the entrance and exit of a birefringent plate"
                        .into(),
                )
            }
            (None, false) => {
                if !s.is_coord_break() && !is_air(&s.mat) {
                    return Err(
                        "a surface list block holds only surfaces in air (glass goes in lenses)"
                            .into(),
                    );
                }
            }
        }
    }
    if inside {
        return Err("a birefringent plate has no exit".into());
    }
    Ok(())
}

/// (dx, dy, tx, ty, tz) の座標ブレーク
pub(crate) fn cb(d: [f64; 5], order: u8, t: f64) -> Surface {
    Surface {
        form: Form::CoordBrk { cb: d, order },
        mat: "-".into(),
        t,
        ..Default::default()
    }
}

/// zmx ブロックで取り込んだ .zmx のピックアップと厚さのソルブ（面番号は取り込んだ .zmx の番号）
pub(crate) struct ZmxLinks {
    pub(crate) path: String,
    /// 取り込んだ向き（1 か、鏡の後で −1。曲率・厚さの符号を返した）
    pub(crate) sign: f64,
    pub(crate) pickups: Vec<Pickup>,
    pub(crate) solves: Vec<Solve>,
    /// 取り込んだ .zmx の像面の番号
    pub(crate) image: usize,
    /// 取り込んだ .zmx の物体面のコメント（面 1 から系の面 1 に取り込んだとき、系の物体面のコメントにする）
    pub(crate) obj_comment: String,
    /// 取り込んだ .zmx の物体空間の媒質（面 1 から系の面 1 に取り込んだとき、系の物体空間の媒質と違えば注意を出す）
    pub(crate) obj_mat: Material,
    /// 取り込んだ .zmx のマクロのソルブの注意（展開した系の注意にする）
    pub(crate) macro_notes: Vec<String>,
    /// 取り込んだ .zmx の ZPL マクロと、パラメータ・円錐係数・半直径のマクロのソルブ（ソルブの番号は macros の番号）
    pub(crate) macros: Vec<lensbench_core::zpl_solve::Macro>,
    pub(crate) macro_parms: Vec<lensbench_core::zpl_solve::MacroParm>,
}

impl<'a, 'b> Ctx<'a, 'b> {
    /// 複屈折材の波長板（`waveplate` の `material`・`plates`）を、厚さのある結晶板の面（入口・出口）に置き換える。
    /// 板ごとに入口（後ろは常光の材料、厚さは板の厚さ、結晶軸は面内で遅い軸の向き）と出口（後ろは次の板の材料か空気）。
    /// 外側の 2 面の膜は波長板の面の `coat`、無ければ透過率から理想膜（1 面あたり √T、T = 1 は I.99999999）
    pub(crate) fn crystal_plates(&self, v: Vec<Surface>) -> lensbench_core::Result<Vec<Surface>> {
        let mut out = Vec::with_capacity(v.len());
        for s in v {
            let Some(r0) = s.retarder.as_ref().filter(|r| !r.plates.is_empty()) else {
                out.push(s);
                continue;
            };
            if s.before.is_some() {
                return Err("tilt is not supported on a birefringent waveplate".into());
            }
            let mut r = r0.clone();
            r.resolve(&self.glasses)?;
            let coat = if !s.coat.is_empty() {
                s.coat.clone()
            } else {
                ideal_face(r.tx * r.tx)
            };
            let n = r.plates.len();
            for (i, p) in r.plates.iter().enumerate() {
                // 結晶軸（光学軸）は異常光の向き: ne > no なら遅い軸 = 速い軸 + 90°
                let fast = r.axis_deg + p.axis_deg;
                let oa = if p.dn(r.wl0) > 0.0 { fast + 90.0 } else { fast }.to_radians();
                let axis = [round_unit(oa.cos()), round_unit(oa.sin()), 0.0];
                out.push(Surface {
                    mat: Material::Glass(p.material.clone()),
                    t: p.thickness,
                    stop: s.stop && i == 0,
                    coat: if i == 0 { coat.clone() } else { String::new() },
                    comment: if i == 0 {
                        s.comment.clone()
                    } else {
                        String::new()
                    },
                    bire: Some(lensbench_core::surface::Bire::In {
                        mode: 2,
                        axis,
                        ne: Vec::new(),
                    }),
                    ..plate_face(&s)
                });
                let last = i + 1 == n;
                out.push(Surface {
                    mat: if last {
                        Material::Air
                    } else {
                        Material::Glass(r.plates[i + 1].material.clone())
                    },
                    t: if last { s.t } else { 0.0 },
                    coat: if last { coat.clone() } else { String::new() },
                    bire: Some(lensbench_core::surface::Bire::Out),
                    ..plate_face(&s)
                });
            }
        }
        Ok(out)
    }

    /// 面をそのまま足す（出どころなし）
    pub(crate) fn put(&mut self, s: Surface) {
        self.sf.push(s);
        self.map.push(SurfOrigin {
            leaf: self.cur_leaf(),
            ..Default::default()
        });
    }

    /// 今たどっている木のいちばん内のブロック（アドレス、無ければ 0）
    pub(crate) fn cur_leaf(&self) -> usize {
        self.leaf.last().copied().unwrap_or(0)
    }

    /// b が木のブロックなら、たどっている間は面の出どころにする（f の後に外す）
    pub(crate) fn in_node<T>(
        &mut self,
        b: &Block,
        f: impl FnOnce(&mut Self) -> lensbench_core::Result<T>,
    ) -> lensbench_core::Result<T> {
        let k = b as *const Block as usize;
        let on = self.nodes.contains_key(&k);
        if on {
            self.leaf.push(k);
        }
        let r = f(self);
        if on {
            // 誤りはいちばん内のブロックで覚える（外へ戻る間は上書きしない）
            if r.is_err() && self.failed.is_none() {
                self.failed = self.nodes.get(&k).cloned();
            }
            self.leaf.pop();
        }
        r
    }

    /// id の空気が入る面（gap の前に呼ぶ）
    pub(crate) fn note_gap(&mut self, id: &str) {
        if !id.is_empty() {
            self.gaps.push((id.to_string(), self.sf.len(), self.parity));
        }
    }

    pub(crate) fn gap(&mut self, t: f64) {
        match self.sf.last_mut() {
            Some(s) => s.t += t * self.parity,
            None => self.lead += t,
        }
    }

    /// 面を z について裏返す（[`Surface::flip_z`]）。裏返せなければ最初の誤りを覚える（面の番号を付けて）
    fn flip(&mut self, s: &mut Surface) {
        if let Err(e) = s.flip_z() {
            if self.flip_err.is_none() {
                let j = self.sf.len() + 1;
                self.flip_err = Some(e.map_message(|e| format!("surface {j} after a mirror: {e}")));
            }
        }
    }

    /// 光の進む向きで書いた面を、今の向きに直して足す（厚さ・曲率・非球面）
    pub(crate) fn push(&mut self, mut s: Surface) {
        if self.parity < 0.0 {
            self.flip(&mut s);
            s.t = -s.t;
        }
        if s.stop {
            self.nstop += 1;
        }
        self.put(s);
    }

    /// sf の start 以降に鏡があるか。鏡を含む群の移動は枚数によらず断る:
    /// 偶数枚で向きが戻っても、出ていく軸が入ってくる軸からずれると間隔の補正では表せない
    pub(crate) fn has_mirror_since(&self, start: usize) -> bool {
        self.sf[start.min(self.sf.len())..]
            .iter()
            .any(|s| s.is_mirror())
    }

    /// 符号つきの長さ（光の進む向き）: 今の位置から n 面前までの厚さの和
    pub(crate) fn length_since(&self, start: usize) -> f64 {
        self.sf[start..].iter().map(|s| s.t).sum::<f64>() * self.parity
    }

    /// 偏心・傾きをつけて body を入れる。戻りの座標ブレークで元の軸の、長さ L だけ進んだ点へ戻す
    pub(crate) fn placed(
        &mut self,
        place: &Place,
        body: &mut dyn FnMut(&mut Self) -> lensbench_core::Result<()>,
    ) -> lensbench_core::Result<()> {
        if !place.on {
            if place.off == OffMode::Air {
                // 長さを測るため一度入れて、空気に置き換える
                // 中身が足した面の範囲・記録は全部巻き戻す（残すと後ろの別のブロックの面を指す）
                let start = self.sf.len();
                let (p0, n0, lead0) = (self.parity, self.nstop, self.lead);
                let lens = (self.noted.len(), self.gaps.len(), self.zpicks.len());
                body(self)?;
                if self.parity != p0 {
                    return Err("a block with a mirror can't be 'kept as air'".into());
                }
                let l = self.length_since(start) + (self.lead - lead0);
                self.sf.truncate(start);
                self.map.truncate(start);
                self.noted.truncate(lens.0);
                self.gaps.truncate(lens.1);
                self.zpicks.truncate(lens.2);
                self.nstop = n0;
                self.lead = lead0;
                self.gap(l);
            }
            return Ok(());
        }
        if !place.moved() {
            return body(self);
        }
        let [dx, dy] = place.decenter;
        let [tx, ty, tz] = place.tilt;
        self.push(cb([dx, dy, tx, ty, tz], 0, 0.0));
        let start = self.sf.len();
        let p0 = self.parity;
        body(self)?;
        if self.parity != p0 {
            return Err(
                "a mirror can't go in a decentered or tilted block (tilt the mirror itself)".into(),
            );
        }
        // 中身の最後の面の厚さを戻りの後へ回す
        let tail = self.sf.last().map(|s| s.t).unwrap_or(0.0);
        if let Some(s) = self.sf.last_mut() {
            s.t = 0.0;
        }
        let l: f64 = self.sf[start..].iter().map(|s| s.t).sum(); // 今の座標の z で
        let (dx2, dy2, t2) = cb_return(place, l);
        // 戻りの座標ブレークの x・y の偏心は傾きを戻したあとの（元の）座標で
        self.put(cb([dx2, dy2, -tx, -ty, -tz], 1, t2 + tail));
        Ok(())
    }

    /// OpticStudio の座標のままの要素（.zmx から取り込んだもの、鏡のあとは負の厚さ）を足す。
    /// sign は取り込み始めの向き（逆なら全部の符号を返す）
    pub(crate) fn element(&mut self, e: &Element, sign: f64) {
        self.element_from(e, sign, None)
    }

    /// element と同じ。inst は zmx ブロックの取り込みの番号（面の出どころ = 要素の元の面番号から）
    pub(crate) fn element_from(&mut self, e: &Element, sign: f64, inst: Option<usize>) {
        if e.kind == Kind::Air {
            match self.sf.last_mut() {
                Some(s) => s.t += e.t * sign,
                None => self.lead += e.t * sign,
            }
            return;
        }
        for (i, s) in e.surfaces.iter().enumerate() {
            let mut s = s.clone();
            if sign < 0.0 {
                self.flip(&mut s);
                s.t = -s.t;
            }
            if s.is_mirror() {
                self.parity = -self.parity;
            }
            if s.stop {
                self.nstop += 1;
            }
            self.sf.push(s);
            self.map.push(SurfOrigin {
                zmx: inst.map(|k| (k, e.from + i)),
                leaf: self.cur_leaf(),
                ..Default::default()
            });
        }
    }

    /// id のあるブロックが sf の start から足した面を、玉・群の範囲として覚える。faces に展開の番号
    /// （[`FaceMods::site`]）があれば、その番号も残す
    pub(crate) fn note(&mut self, id: &str, faces: Option<&FaceMods>, start: usize) {
        if let Some(k) = faces.map(|f| f.site).filter(|&k| k > 0) {
            let k = k - 1;
            for o in &mut self.map[start..] {
                if !o.mods.contains(&k) {
                    o.mods.push(k);
                }
            }
        }
        if !id.is_empty() && self.sf.len() > start {
            for o in &mut self.map[start..] {
                if !o.blocks.iter().any(|b| b == id) {
                    o.blocks.push(id.to_string());
                }
            }
            if !self.noted.iter().any(|b| b == id) {
                self.noted.push(id.to_string());
            }
        }
    }

    /// 往路のブロックを復路でたどる（鏡のあと）
    pub(crate) fn back_block(&mut self, b: &Block) -> lensbench_core::Result<()> {
        let start = self.sf.len();
        self.in_node(b, |c| c.back_block_inner(b))?;
        self.note(block_id(b), b.faces(), start);
        for o in &mut self.map[start..] {
            o.back = true;
        }
        Ok(())
    }

    pub(crate) fn back_block_inner(&mut self, b: &Block) -> lensbench_core::Result<()> {
        match b {
            Block::Air { id, t } => {
                self.note_gap(id);
                self.gap(*t)
            }
            Block::Stop { semi, .. } => {
                // 復路で絞りの面を通る位置（絞りにはしない）
                self.push(Surface {
                    semi: *semi,
                    comment: "STOP (return)".into(),
                    ..Default::default()
                });
            }
            Block::Lens {
                source,
                reversed,
                place,
                ..
            } => {
                let mut e = self.lens_element(source)?;
                if *reversed {
                    e = elements::reverse(&e)?;
                }
                let len = e.length();
                if !place.on {
                    if place.off == OffMode::Air {
                        self.gap(len);
                    }
                    return Ok(());
                }
                let mut r = elements::reverse_folded(&e)?;
                r.surfaces.iter_mut().for_each(|s| s.stop = false);
                self.back_placed(place, len, &r.surfaces)?;
            }
            Block::Surfaces {
                surfaces, place, ..
            } => {
                let v: Vec<Surface> = surfaces.iter().map(surf).collect::<Result<_, _>>()?;
                if v.iter().any(|s| {
                    matches!(s.typ(), SurfType::Jones | SurfType::CoordBrk) || s.bire.is_some()
                }) {
                    return Err(
                        "jones, coordbrk and birefringent surfaces are not supported in a surface list on the return path".into(),
                    );
                }
                // 面の並びは空気中なので、理想レンズも向きを変えずに使える
                if v.iter().any(|s| {
                    (!lensbench_core::elements::reversible(s) && s.typ() != SurfType::Paraxial)
                        || s.grin().is_some()
                }) {
                    return Err(
                        "GRIN and user-defined surfaces are not supported in a surface list on the return path".into(),
                    );
                }
                if !place.on {
                    if place.off == OffMode::Air {
                        let l: f64 = v.iter().map(|s| s.t).sum();
                        self.gap(l);
                    }
                    return Ok(());
                }
                // 往路の最後の面の厚さ（後の空気）は、復路では最初の面の前の空気
                let tail = v.last().map(|s| s.t).unwrap_or(0.0);
                let len: f64 = v.iter().map(|s| s.t).sum::<f64>() - tail;
                self.gap(tail);
                self.back_placed(place, len, &reverse_surfaces(&v)?)?;
            }
            Block::Group {
                blocks,
                place,
                shift,
                ..
            } => {
                if place.moved() {
                    return Err(
                        "decentered or tilted groups are not supported on the return path (put it on the lenses in the group)"
                            .into(),
                    );
                }
                if !place.on {
                    return Err("groups that are OFF are not supported on the return path".into());
                }
                // 復路では後の間隔が先: 往路で引いた分を引き、群の後（往路の前の間隔）で足す
                self.gap(-shift);
                for x in blocks.iter().rev() {
                    self.back_block(x)?;
                    if self.split.stopped {
                        return Ok(());
                    }
                }
                self.gap(*shift);
            }
            Block::User { .. } => {
                let (g, dir, key) = self.user_group(b)?;
                let base = std::mem::replace(&mut self.base, dir);
                self.loading.push(key);
                let r = self.back_block(&g);
                self.loading.pop();
                self.base = base;
                r?;
            }
            Block::Mirror { .. } => return Err("a mirror on the return path".into()),
            Block::Splitter { id, spec, .. } => {
                if spec.has_branches() {
                    return Err(format!(
                        "BS {id}: a BS with branches is never passed backwards (internal error)"
                    )
                    .into());
                }
                self.splitter_back(id)?;
            }
            Block::Detector(_) => return Err("a detector must be the last block".into()),
            Block::Zmx { .. } => {
                return Err(
                    "importing a .zmx on the return path is not supported (use a lens block)"
                        .into(),
                )
            }
        }
        Ok(())
    }

    /// 復路で、往路に偏心・傾き place・長さ len（光の進む向き）で置いたものを、逆向きの面 rev でたどる
    pub(crate) fn back_placed(
        &mut self,
        place: &Place,
        len: f64,
        rev: &[Surface],
    ) -> lensbench_core::Result<()> {
        if !place.moved() {
            for s in rev {
                self.push(s.clone());
            }
            return Ok(());
        }
        // 往路: CB1（順序 0、d・t）→ 中身（今の座標で l）→ CB2（順序 1、−t・d2、厚さ t2）。復路はその逆:
        // 元の軸で t2 戻る → CB2 の逆（順序 0、−d2・t）→ 逆向きの中身 → CB1 の逆（順序 1、−d・−t）
        let pf = -self.parity;
        let l = len * pf;
        let (dx2, dy2, t2) = cb_return(place, l);
        let [dx, dy] = place.decenter;
        let [tx, ty, tz] = place.tilt;
        // 今の座標で −t2 進む（gap は光の進む向きの長さを取る）
        self.gap(-t2 * self.parity);
        self.put(cb([-dx2, -dy2, tx, ty, tz], 0, 0.0));
        for s in rev {
            self.push(s.clone());
        }
        if let Some(s) = self.sf.last_mut() {
            s.t = 0.0;
        }
        self.put(cb([-dx, -dy, -tx, -ty, -tz], 1, 0.0));
        Ok(())
    }

    /// 展開中のファイル・項目に key があれば、循環の道を並べた誤り
    pub(crate) fn not_loading(&self, key: &Loading) -> lensbench_core::Result<()> {
        let Some(i) = self.loading.iter().position(|l| l == key) else {
            return Ok(());
        };
        let chain: Vec<String> = self.loading[i..]
            .iter()
            .chain([key])
            .map(ToString::to_string)
            .collect();
        Err(format!("a loop of imports: {}", chain.join(" → ")).into())
    }

    /// .zmx・系ファイル・プロジェクトの系を読む。pick はプロジェクトのときの状態と BS の経路
    pub(crate) fn load_zmx(
        &mut self,
        path: &str,
        cfg: usize,
        pick: (Option<&str>, Option<&str>),
    ) -> lensbench_core::Result<System> {
        if cfg == 0 {
            return Err(format!("{path}: the configuration number must be 1 or more").into());
        }
        let p = self.base.join(path);
        let picked = pick.0.is_some() || pick.1.is_some();
        if picked
            && !p
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        {
            return Err(format!("{path}: state and beam_path are for a project file").into());
        }
        // 覚えた系（ファイルが変わっていなければ）
        let file = lensbench_core::vfs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        let key = (file.clone(), cfg);
        let stamp = self.cache.and_then(|_| lensbench_core::vfs::stamp(&p).ok());
        if let (Some(c), Some(st), false) = (self.cache, &stamp, picked) {
            if let Some(s) = c.file(&key, st) {
                self.merge_glasses(&s);
                return Ok(s);
            }
        }
        let keep = |c: &Self, s: &System| {
            if let (Some(c), Some(st)) = (c.cache, stamp) {
                c.files.lock_ok().insert(key, (st, s.clone()));
            }
        };
        // lensbench の系（系ファイル・プロジェクトの .json）も同じように取り込む
        if p.extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        {
            if cfg != 1 {
                return Err(format!("{path}: a .json system has only configuration 1").into());
            }
            // ファイルは 1 回だけ読む（系ファイルかプロジェクトかは中身で見る）
            let txt = lensbench_core::vfs::read_to_string(&p)
                .map_err(lensbench_core::Error::io_at(p.display()))?;
            if sysfile::is_system_json(&txt) {
                if picked {
                    return Err(
                        format!("{path}: state and beam_path are for a project file").into(),
                    );
                }
                let s = sysfile::load_text_with(&p, &txt, &self.env.glass_dirs)?.0;
                keep(self, &s);
                self.merge_glasses(&s);
                return Ok(s);
            }
            let me = Loading { file, item: None };
            self.not_loading(&me)?;
            let mut within = self.loading.clone();
            within.push(me);
            // 入れ子のプロジェクトは中のファイルが変わりうるので系は覚えず、中の葉だけ覚える
            let s = Project::from_json(&txt)
                .and_then(|q| {
                    let env = Env::for_file(&p, &self.env.glass_dirs, self.env.catalog);
                    let o = ExpandOpts {
                        state: pick.0,
                        path: pick.1,
                        cache: self.cache,
                        within: &within,
                    };
                    expand(&q, &env, &o)
                        .map(|x| x.system)
                        .map_err(lensbench_core::Error::from)
                })
                .map_err(|e| e.at(p.display()))?;
            self.merge_glasses(&s);
            return Ok(s);
        }
        let opt = zmx::LoadOptions {
            cfg,
            catalog_dirs: self.env.glass_dirs.clone(),
            ..Default::default()
        };
        let s = zmx::load_with(&p, &opt).map_err(|e| format!("{}: {e}", p.display()))?;
        keep(self, &s);
        self.merge_glasses(&s);
        Ok(s)
    }

    /// ユーザーカタログの項目を群にする（と、その中の相対パスの基準）。項目のガラスカタログを足す
    pub(crate) fn user_group(
        &mut self,
        b: &Block,
    ) -> lensbench_core::Result<(Block, PathBuf, Loading)> {
        self.user_group_frame(b).map(|(g, d, k, _)| (g, d, k))
    }

    /// [`Self::user_group`] と、品の枠
    #[allow(clippy::type_complexity)]
    pub(crate) fn user_group_frame(
        &mut self,
        b: &Block,
    ) -> lensbench_core::Result<(Block, PathBuf, Loading, ItemFrame)> {
        let Block::User {
            id,
            catalog,
            item,
            place,
            ..
        } = b
        else {
            unreachable!()
        };
        let path = self.base.join(catalog);
        let key = Loading {
            file: lensbench_core::vfs::canonicalize(&path).unwrap_or_else(|_| path.clone()),
            item: Some(item.clone()),
        };
        self.not_loading(&key)?;
        let cat = UserCatalog::load_shared(&path)?;
        let it = cat
            .items
            .get(item)
            .ok_or_else(|| format!("{}: no item {item}", path.display()))?;
        let dirs = glass::catalog_dirs(&self.env.glass_dirs);
        let (g, missing) = glass::load_catalogs_shared(&it.glass_catalogs, &dirs);
        if !missing.is_empty() {
            return Err(lensbench_core::Error::Invalid(format!(
                "{item}: glass catalog not found: {}. Pass catalog folders with --glass-dir (CLI), glass_dirs (serve, Python) or LENSBENCH_GLASSCAT",
                missing.join(", ")
            ))
            .with_code(lensbench_core::error::code::GLASS_NOT_FOUND));
        }
        self.add_glasses(&g);
        // カタログの膜（フィルタの透過特性）。同じ名前で中身の違う膜は誤り
        for (k, spec) in &cat.coatings {
            let t = spec
                .table()
                .map_err(|e| format!("{}: coating {k}: {e}", path.display()))?;
            match self.coats.get(k) {
                Some(old) if *old != t => {
                    return Err(format!(
                        "{}: coating {k} differs from another catalog",
                        path.display()
                    )
                    .into())
                }
                Some(_) => {}
                None => {
                    self.coats.insert(k.clone(), t);
                }
            }
        }
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok((
            Block::Group {
                id: id.clone(),
                name: item.clone(),
                role: None,
                shift: 0.0,
                blocks: it.blocks.clone(),
                place: place.clone(),
                // 項目のブロックの面の上書き・操作は当てない（User のブロックのものは User のブロックで当てる）
                faces: FaceMods::default(),
            },
            dir,
            key,
            it.frame.clone().map(|f| (f, it.frame_reversed)),
        ))
    }

    pub(crate) fn merge_glasses(&mut self, s: &System) {
        self.add_glasses(&s.glasses);
    }

    /// 無い名前のガラスだけ足す（全部あれば写さない）
    pub(crate) fn add_glasses(&mut self, g: &std::sync::Arc<HashMap<String, Glass>>) {
        if std::sync::Arc::ptr_eq(&self.glasses, g)
            || g.keys().all(|k| self.glasses.contains_key(k))
        {
            return;
        }
        if self.glasses.is_empty() {
            self.glasses = g.clone();
            return;
        }
        let m = std::sync::Arc::make_mut(&mut self.glasses);
        for (k, v) in g.iter() {
            m.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    pub(crate) fn lens_element(&mut self, src: &LensSource) -> lensbench_core::Result<Element> {
        match src {
            LensSource::Zmx { zmx, lens, config } => {
                let s = self.load_zmx(zmx, *config, (None, None))?;
                Ok(pick_lens(&s, *lens).map_err(|e| format!("{zmx}: {e}"))?)
            }
            LensSource::Catalog { vendor, part } => {
                let key = (vendor.clone(), part.clone());
                let hit = self
                    .cache
                    .and_then(|c| c.lenses.lock_ok().get(&key).cloned());
                if let Some(h) = hit {
                    self.add_glasses(&h.1);
                    return Ok(h.0.clone());
                }
                let f = self.env.catalog.ok_or_else(|| {
                    format!("lens catalog {vendor} {part}: the catalog can't be read")
                })?;
                // 品が無い（カタログの口の誤り）は符号 catalog.lens_not_found
                // （口が符号を付けていればそのまま）
                let txt = f(vendor, part).map_err(|e| match e.code() == e.kind() {
                    true => e.with_code(lensbench_core::error::code::LENS_NOT_FOUND),
                    false => e,
                })?;
                let raw = zmx::parse_text(&txt)?;
                let s = zmx::build(
                    &raw,
                    &zmx::LoadOptions {
                        cfg: 1,
                        fixed_semi: true,
                        ..Default::default()
                    },
                    &glass::catalog_dirs(&self.env.glass_dirs),
                    None,
                )
                .map_err(|e| format!("{vendor} {part}: {e}"))?;
                self.merge_glasses(&s);
                // カタログの玉は径が決まっているので半直径は固定（fixed_semi）
                let el = pick_lens(&s, 1).map_err(|e| format!("{vendor} {part}: {e}"))?;
                if let Some(c) = self.cache {
                    let v = std::sync::Arc::new((el.clone(), s.glasses.clone()));
                    c.lenses.lock_ok().insert(key, v);
                }
                Ok(el)
            }
            LensSource::Surfaces { surfaces } => {
                if surfaces.len() < 2 {
                    return Err("a lens needs 2 or more surfaces".into());
                }
                let n = surfaces.len();
                let mut v = Vec::new();
                for (i, q) in surfaces.iter().enumerate() {
                    let mut s = surf(q)?;
                    if i + 1 == n {
                        s.t = 0.0;
                        s.mat = Material::Air;
                    } else if is_air(&s.mat) {
                        return Err(format!("lens surface {}: not glass after it", i + 1).into());
                    }
                    v.push(s);
                }
                Ok(Element {
                    kind: Kind::Lens,
                    from: 0,
                    surfaces: v,
                    n: Vec::new(),
                    t: 0.0,
                    reversed: false,
                })
            }
        }
    }

    pub(crate) fn block(
        &mut self,
        b: &Block,
        siblings: &[Block],
        idx: usize,
    ) -> lensbench_core::Result<()> {
        let start = self.sf.len();
        self.in_node(b, |c| c.block_inner(b, siblings, idx))?;
        // 鏡は鏡の面だけ（往路を戻る面は除く）。中で覚える
        if !matches!(b, Block::Mirror { .. } | Block::Splitter { .. }) {
            self.note(block_id(b), b.faces(), start);
        }
        Ok(())
    }

    pub(crate) fn block_inner(
        &mut self,
        b: &Block,
        siblings: &[Block],
        idx: usize,
    ) -> lensbench_core::Result<()> {
        match b {
            Block::Air { id, t } => {
                self.note_gap(id);
                self.gap(*t)
            }
            Block::Stop { semi, .. } => {
                let s = Surface {
                    stop: true,
                    semi: *semi,
                    comment: "STOP".into(),
                    ..Default::default()
                };
                self.push(s);
            }
            Block::Lens {
                source,
                reversed,
                stop,
                place,
                ..
            } => {
                let mut e = self.lens_element(source)?;
                if *reversed {
                    e = elements::reverse(&e)?;
                }
                e.surfaces.iter_mut().for_each(|s| s.stop = false);
                if *stop {
                    e.surfaces[0].stop = true;
                }
                let rev = *reversed;
                self.placed(place, &mut |c: &mut Self| {
                    let start = c.sf.len();
                    for s in &e.surfaces {
                        c.push(s.clone());
                    }
                    if rev && c.sf.len() > start {
                        c.nflip += 1;
                        let k = c.nflip;
                        for o in &mut c.map[start..] {
                            o.flip = Some(k);
                        }
                    }
                    Ok(())
                })?;
            }
            Block::Surfaces {
                surfaces, place, ..
            } => {
                let v: Vec<Surface> = surfaces.iter().map(surf).collect::<Result<_, _>>()?;
                let v = self.crystal_plates(v)?;
                check_crystal_order(&v)?;
                self.placed(place, &mut |c: &mut Self| {
                    for s in &v {
                        c.push(s.clone());
                    }
                    Ok(())
                })?;
            }
            Block::Mirror {
                id,
                radius,
                conic,
                tilt,
                semi,
                aperture,
                coat,
                on,
                back,
                row,
                faces,
            } => {
                if !on {
                    return Ok(());
                }
                let start = self.sf.len();
                let back = *back;
                let fold = tilt[0] != 0.0 || tilt[1] != 0.0;
                let m = match row {
                    Some(r) => {
                        if radius.is_some_and(|x| x != 0.0)
                            || *conic != 0.0
                            || *semi != 0.0
                            || aperture.is_some()
                            || !coat.is_empty()
                        {
                            return Err("a mirror written as row can't have radius, conic, semi, aperture or coat (write them in the row)".into());
                        }
                        mirror_row(r)?
                    }
                    None => Surface {
                        mat: Material::Mirror,
                        c: curv(*radius),
                        k: *conic,
                        semi: *semi,
                        coat: coat.clone(),
                        ap: surf_ap(*aperture, *semi),
                        ..Default::default()
                    },
                };
                if fold {
                    self.put(cb([0.0, 0.0, tilt[0], tilt[1], 0.0], 0, 0.0));
                }
                let mut m = m;
                if self.parity < 0.0 {
                    if row.is_some() {
                        self.flip(&mut m);
                    } else {
                        m.c = -m.c;
                    }
                }
                self.put(m);
                self.parity = -self.parity;
                if fold && back {
                    // 往路を戻る鏡の傾き（据え付けの誤差など）: 鏡だけを傾け、座標は元の軸へ戻す（順序 1 で逆の傾き）。
                    // 復路の面は往路と同じ位置にあり、光線は鏡の傾きの 2 倍の角でそこを戻る
                    self.put(cb([0.0, 0.0, -tilt[0], -tilt[1], 0.0], 1, 0.0));
                } else if fold {
                    // 鏡の後の軸を反射した光軸に乗せる: 前の座標ブレークが R = Rx(tx)·Ry(ty) なら、鏡の座標で入射の軸は
                    // Rᵀz、反射はその z 成分の反転。後の回転 R₂ は R₂z = diag(−1, −1, 1)·Rᵀz を満たせばよく、
                    // R₂ = Ry(ty)·Rx(tx)、つまり同じ角で回す順を逆にした座標ブレーク（順序 1）。1 軸の傾きでは順序によらない
                    self.put(cb([0.0, 0.0, tilt[0], tilt[1], 0.0], 1, 0.0));
                }
                self.note(id, Some(faces), start);
                if back {
                    // 前のブロックを戻る。BS で A 以外の口へ出たら止まり、並びの初めまで戻れば（BS の分岐なら）BS を通る
                    self.split.stopped = false;
                    for x in siblings[..idx].iter().rev() {
                        self.back_block(x)?;
                        if self.split.stopped {
                            break;
                        }
                    }
                    if !self.split.stopped {
                        let lv = self.split.levels.last().cloned().unwrap_or(Level::Other);
                        self.return_at(lv)?;
                    }
                    self.split.stopped = false;
                    if self.split.done && idx + 1 < siblings.len() {
                        return Err(
                            "both blocks after a mirror and BS branches (a return path going into a BS branch)".into(),
                        );
                    }
                }
            }
            Block::Zmx {
                path,
                from,
                to,
                config,
                stop,
                groups,
                place,
                state,
                beam_path,
                ..
            } => {
                let s = self.load_zmx(path, *config, (state.as_deref(), beam_path.as_deref()))?;
                self.zenv.push((
                    path.clone(),
                    s.temperature,
                    s.pressure,
                    s.adjust_index,
                    s.coating_file.clone(),
                ));
                // 物体空間・像空間が空気でない .zmx（液浸など）もまるごと取り込む（媒質は物体面・最後の要素が持ち越す）
                let els = elements::split_any(&s);
                let inst = self.zpicks.len();
                self.zpicks.push(ZmxLinks {
                    path: path.clone(),
                    sign: 1.0,
                    pickups: s.pickups.clone(),
                    solves: s.solves.clone(),
                    image: s.image(),
                    obj_comment: s.surfaces[0].comment.clone(),
                    obj_mat: s.surfaces[0].mat.clone(),
                    macro_notes: s.macro_notes.clone(),
                    macros: s.macros.clone(),
                    macro_parms: s.macro_parms.clone(),
                });
                let (a, z) = (from.unwrap_or(1), to.unwrap_or(s.image() - 1));
                // 範囲の中の要素と、その直後の空気だけを取る（範囲の外の要素の後の空気は除く）
                let mut keep = Vec::new();
                let mut last_in = false;
                for e in els {
                    if e.kind == Kind::Air {
                        if last_in {
                            keep.push(e);
                        }
                    } else {
                        last_in = e.from >= a && e.from <= z;
                        if last_in {
                            keep.push(e);
                        }
                    }
                }
                if !*stop {
                    for e in &mut keep {
                        e.surfaces.iter_mut().for_each(|s| s.stop = false);
                    }
                }
                let spans = zmx_group_spans(&keep, groups).map_err(|e| format!("{path}: {e}"))?;
                self.placed(place, &mut |c: &mut Self| {
                    let sign = c.parity;
                    c.zpicks[inst].sign = sign;
                    let mut i = 0;
                    while i < keep.len() {
                        let Some(&(g, a, z)) = spans.iter().find(|q| q.1 == i) else {
                            c.element_from(&keep[i], sign, Some(inst));
                            i += 1;
                            continue;
                        };
                        let gr = &groups[g];
                        c.element(&Element::air(gr.shift), sign);
                        let start = c.sf.len();
                        c.placed(&gr.place, &mut |c: &mut Self| {
                            for e in &keep[a..=z] {
                                c.element_from(e, sign, Some(inst));
                            }
                            Ok(())
                        })?;
                        c.note(&gr.id, Some(&gr.faces), start);
                        if gr.shift != 0.0 && c.has_mirror_since(start) {
                            return Err(
                                "a group with a mirror can't be moved (set shift to 0)".into()
                            );
                        }
                        c.element(&Element::air(-gr.shift), sign);
                        i = z + 1;
                    }
                    Ok(())
                })?;
            }
            Block::User { id, item, .. } => {
                let (g, dir, key, frame) = self.user_group_frame(b)?;
                let base = std::mem::replace(&mut self.base, dir);
                self.loading.push(key);
                let start = self.sf.len();
                let r = self.block(&g, &[], 0);
                self.loading.pop();
                self.base = base;
                r?;
                // 品の枠は、外側の品（項目の中の項目でない）だけ置く
                if let Some((f, rev)) =
                    frame.filter(|_| self.loading.len() == self.outer && self.sf.len() > start)
                {
                    let name = if id.is_empty() {
                        item.clone()
                    } else {
                        id.clone()
                    };
                    // 逆向きの枠は、品のガラスの厚さ（最初の面から最後の面まで）で座を移す
                    let f = if rev {
                        let t: f64 = self.sf[start..self.sf.len() - 1]
                            .iter()
                            .filter(|q| !q.is_coord_break())
                            .map(|q| q.t)
                            .sum();
                        f.flipped(t)
                    } else {
                        f
                    };
                    let k = self.frames.len();
                    self.frames.push((name, f));
                    for o in &mut self.map[start..] {
                        o.frame = Some(k);
                    }
                }
            }
            Block::Splitter { id, spec, place } => {
                self.splitter_forward(id, spec, place, siblings, idx)?;
            }
            Block::Detector(_) => {
                return Err(
                    "a detector must be the last block (not inside a group or user catalog)".into(),
                )
            }
            Block::Group {
                blocks,
                place,
                shift,
                ..
            } => {
                let bl = blocks;
                self.gap(*shift);
                let start = self.sf.len();
                self.split.levels.push(Level::Other);
                let r = self.placed(place, &mut |c: &mut Self| {
                    for (i, x) in bl.iter().enumerate() {
                        if c.split.done {
                            break;
                        }
                        c.block(x, bl, i)?;
                    }
                    Ok(())
                });
                self.split.levels.pop();
                r?;
                if *shift != 0.0 && self.has_mirror_since(start) {
                    return Err("a group with a mirror can't be moved (set shift to 0)".into());
                }
                self.gap(-shift);
            }
        }
        Ok(())
    }
}

/// ブロックの id
pub(crate) fn block_id(b: &Block) -> &str {
    match b {
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
    }
}

/// 鏡のブロックの行（[`Block::Mirror`] の row）を面に。鏡の働きで、厚さ・媒質・絞り・傾きの無いもの
fn mirror_row(r: &crate::sysfile_v2::Row) -> lensbench_core::Result<Surface> {
    if r.stop || r.object || r.image || r.t != 0.0 || r.medium.is_some() {
        return Err("row of a mirror can't have stop, object, image, thickness or medium".into());
    }
    let m = crate::sysfile_v2::read::surface(r.clone())?;
    if !m.is_mirror() || m.reflect.is_some() {
        return Err("row of a mirror needs a reflecting action".into());
    }
    if m.typ() == SurfType::CoordBrk || m.before.is_some() || m.after != After::None {
        return Err("row of a mirror can't tilt or decenter (use tilt of the block)".into());
    }
    Ok(m)
}
