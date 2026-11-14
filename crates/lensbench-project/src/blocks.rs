//! ブロックごとの展開（玉・空気・絞り・鏡・面の並び・.zmx・ユーザーカタログ・群）。展開の途中の状態 `Ctx`

use super::*;
/// レンズカタログを引く関数: (メーカー, 型番) → .zmx の本文
pub type CatalogLookup<'a> = &'a dyn Fn(&str, &str) -> Result<String, String>;

/// 展開の環境
pub struct Env<'a> {
    /// 相対パスの基準（プロジェクトファイルのフォルダ）
    pub base: PathBuf,
    pub glass_dirs: Vec<PathBuf>,
    pub catalog: Option<CatalogLookup<'a>>,
}

pub(crate) struct Ctx<'a, 'b> {
    pub(crate) env: &'a Env<'b>,
    /// 相対パスの基準（ユーザーカタログの項目の中ではカタログのフォルダ）
    pub(crate) base: PathBuf,
    /// 展開中のユーザーカタログの項目（正規化したパス, 項目）。循環を見つける
    pub(crate) users: Vec<(PathBuf, String)>,
    pub(crate) sf: Vec<Surface>,
    /// sf の各面の出どころ（zmx ブロックの取り込みの番号, 元の .zmx の面番号）。ピックアップを付け替える
    pub(crate) org: Vec<Option<(usize, usize)>>,
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
    /// id のあるブロック・群の面の範囲（id, sf の最初, 最後）。玉・群のパラメータにする
    pub(crate) spans: Vec<(String, usize, usize)>,
    /// spans のうち、鏡の back で復路にたどった範囲（面の値の上書きを往路から当てる）
    pub(crate) back_spans: Vec<(String, usize, usize)>,
    /// id のある空気のブロックが入った面（id, 面番号, 向き）。差し替え探索の間隔の変数にする
    pub(crate) gaps: Vec<Gap>,
    /// ビームスプリッターの経路
    pub(crate) split: SplitState,
    /// 枠付きのユーザーカタログの品（名前, 枠, sf の最初, 最後）。枠を機構部品として置く
    pub(crate) frames: Vec<(String, MechSpec, usize, usize)>,
    /// 裏返して置いた玉（`reversed` の玉のブロック）の往路の面の範囲（sf の最初, 最後）。玉・群の `flipped` にする
    pub(crate) rev_spans: Vec<(usize, usize)>,
    /// 取り込んだ .zmx の環境（パス, 温度, 気圧, 補正）。プロジェクトの環境と違えば注意を出す
    pub(crate) zenv: Vec<(String, f64, f64, bool)>,
}

/// 品の枠（向きを当てる前）と向き
pub(crate) type ItemFrame = Option<(MechSpec, bool)>;

pub(crate) fn is_air(m: &str) -> bool {
    m.is_empty() || m.eq_ignore_ascii_case("AIR")
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
pub(crate) fn check_crystal_order(v: &[Surface]) -> Result<(), String> {
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
                            "複屈折材の板の出口の後のガラスは、厚さ 0 で同じ材料の次の板の入口へ"
                                .into(),
                        );
                    }
                }
            }
            (Some(Bire::In { .. }), true) => {
                return Err("複屈折材の板の入口が続く（出口が要る）".into())
            }
            (Some(Bire::Out), false) => return Err("複屈折材の板の出口の前に入口が無い".into()),
            (None, true) => return Err("複屈折材の板の入口と出口の間にほかの面は置けない".into()),
            (None, false) => {
                if s.typ != SurfType::CoordBrk && !is_air(&s.mat) {
                    return Err("面の並びのブロックは空気中の面だけ（ガラスは玉で）".into());
                }
            }
        }
    }
    if inside {
        return Err("複屈折材の板に出口が無い".into());
    }
    Ok(())
}

/// (dx, dy, tx, ty, tz) の座標ブレーク
pub(crate) fn cb(d: [f64; 5], order: u8, t: f64) -> Surface {
    Surface {
        typ: SurfType::CoordBrk,
        mat: "-".into(),
        cb: d,
        cb_order: order,
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
    pub(crate) solves: Vec<ThickSolve>,
    /// 取り込んだ .zmx の像面の番号
    pub(crate) image: usize,
}

impl<'a, 'b> Ctx<'a, 'b> {
    /// 複屈折材の波長板（`waveplate` の `material`・`plates`）を、厚さのある結晶板の面（入口・出口）に置き換える。
    /// 板ごとに入口（後ろは常光の材料、厚さは板の厚さ、結晶軸は面内で遅い軸の向き）と出口（後ろは次の板の材料か空気）。
    /// 外側の 2 面の膜は波長板の面の `coat`、無ければ透過率から理想膜（1 面あたり √T、T = 1 は I.99999999）
    pub(crate) fn crystal_plates(&self, v: Vec<Surface>) -> Result<Vec<Surface>, String> {
        let mut out = Vec::with_capacity(v.len());
        for s in v {
            let Some(r0) = s.retarder.as_ref().filter(|r| !r.plates.is_empty()) else {
                out.push(s);
                continue;
            };
            if s.before.is_some() {
                return Err("複屈折材の波長板に tilt は未対応".into());
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
                    mat: p.material.clone(),
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
                        String::new()
                    } else {
                        r.plates[i + 1].material.clone()
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
        self.org.push(None);
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

    /// 光の進む向きで書いた面を、今の向きに直して足す（厚さ・曲率・非球面）
    pub(crate) fn push(&mut self, mut s: Surface) {
        if self.parity < 0.0 {
            s.c = -s.c;
            s.asph.iter_mut().for_each(|a| *a = -*a);
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
        self.sf[start.min(self.sf.len())..].iter().any(|s| s.mirror)
    }

    /// 符号つきの長さ（光の進む向き）: 今の位置から n 面前までの厚さの和
    pub(crate) fn length_since(&self, start: usize) -> f64 {
        self.sf[start..].iter().map(|s| s.t).sum::<f64>() * self.parity
    }

    /// 偏心・傾きをつけて body を入れる。戻りの座標ブレークで元の軸の、長さ L だけ進んだ点へ戻す
    pub(crate) fn placed(
        &mut self,
        place: &Place,
        body: &mut dyn FnMut(&mut Self) -> Result<(), String>,
    ) -> Result<(), String> {
        if !place.on {
            if place.off == OffMode::Air {
                // 長さを測るため一度入れて、空気に置き換える
                // 中身が足した面の範囲・記録は全部巻き戻す（残すと後ろの別のブロックの面を指す）
                let start = self.sf.len();
                let (p0, n0, lead0) = (self.parity, self.nstop, self.lead);
                let lens = (
                    self.spans.len(),
                    self.back_spans.len(),
                    self.gaps.len(),
                    self.frames.len(),
                    self.rev_spans.len(),
                    self.zpicks.len(),
                );
                body(self)?;
                if self.parity != p0 {
                    return Err("鏡を含むブロックは「空気で残す」にできない".into());
                }
                let l = self.length_since(start) + (self.lead - lead0);
                self.sf.truncate(start);
                self.org.truncate(start);
                self.spans.truncate(lens.0);
                self.back_spans.truncate(lens.1);
                self.gaps.truncate(lens.2);
                self.frames.truncate(lens.3);
                self.rev_spans.truncate(lens.4);
                self.zpicks.truncate(lens.5);
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
            return Err("偏心・傾きをつけたブロックに鏡は入れられない（鏡の傾きは鏡で）".into());
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
                s.c = -s.c;
                s.asph.iter_mut().for_each(|a| *a = -*a);
                s.t = -s.t;
            }
            if s.mirror {
                self.parity = -self.parity;
            }
            if s.stop {
                self.nstop += 1;
            }
            self.sf.push(s);
            self.org.push(inst.map(|k| (k, e.from + i)));
        }
    }

    /// id のあるブロックが sf の start から足した面を、玉・群の範囲として覚える
    pub(crate) fn note(&mut self, id: &str, start: usize) {
        if !id.is_empty() && self.sf.len() > start {
            let v = (id.to_string(), start, self.sf.len() - 1);
            if !self.spans.contains(&v) {
                self.spans.push(v);
            }
        }
    }

    /// 往路のブロックを復路でたどる（鏡のあと）
    pub(crate) fn back_block(&mut self, b: &Block) -> Result<(), String> {
        let start = self.sf.len();
        self.back_block_inner(b)?;
        self.note(block_id(b), start);
        let id = block_id(b);
        if !id.is_empty() && self.sf.len() > start {
            let v = (id.to_string(), start, self.sf.len() - 1);
            if !self.back_spans.contains(&v) {
                self.back_spans.push(v);
            }
        }
        Ok(())
    }

    pub(crate) fn back_block_inner(&mut self, b: &Block) -> Result<(), String> {
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
                    e = elements::reverse(&e).map_err(|x| x.to_string())?;
                }
                let len = e.length();
                if !place.on {
                    if place.off == OffMode::Air {
                        self.gap(len);
                    }
                    return Ok(());
                }
                let mut r = elements::reverse(&e).map_err(|x| x.to_string())?;
                r.surfaces.iter_mut().for_each(|s| s.stop = false);
                self.back_placed(place, len, &r.surfaces)?;
            }
            Block::Surfaces {
                surfaces, place, ..
            } => {
                let v: Vec<Surface> = surfaces.iter().map(surf).collect::<Result<_, _>>()?;
                if v.iter().any(|s| {
                    matches!(s.typ, SurfType::Jones | SurfType::CoordBrk) || s.bire.is_some()
                }) {
                    return Err(
                        "往路を戻る範囲の面の並びに jones・coordbrk・複屈折材の面は未対応".into(),
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
                self.back_placed(place, len, &reverse_surfaces(&v))?;
            }
            Block::Group {
                blocks,
                place,
                shift,
                ..
            } => {
                if place.moved() {
                    return Err(
                        "往路を戻る範囲の、偏心・傾きをつけた群は未対応（群の中の玉に付ける）"
                            .into(),
                    );
                }
                if !place.on {
                    return Err("往路を戻る範囲の、OFF の群は未対応".into());
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
                self.users.push(key);
                let r = self.back_block(&g);
                self.users.pop();
                self.base = base;
                r?;
            }
            Block::Mirror { .. } => return Err("往路を戻る範囲に鏡がある".into()),
            Block::Splitter { id, spec, .. } => {
                if spec.has_branches() {
                    return Err(format!(
                        "BS {id}: 分岐のある BS を後ろから戻ることはない（内部エラー）"
                    ));
                }
                self.splitter_back(id)?;
            }
            Block::Detector(_) => return Err("検出器は最後のブロックだけ".into()),
            Block::Zmx { .. } => {
                return Err("往路を戻る範囲の .zmx の取り込みは未対応（玉のブロックにする）".into())
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
    ) -> Result<(), String> {
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

    pub(crate) fn load_zmx(&mut self, path: &str, cfg: usize) -> Result<System, String> {
        if cfg == 0 {
            return Err(format!("{path}: 構成の番号は 1 以上"));
        }
        let p = self.base.join(path);
        // lensbench の系（系ファイル・プロジェクトの .json）も同じように取り込む（#132）
        if p.extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        {
            if cfg != 1 {
                return Err(format!("{path}: .json の系では構成の番号は 1 だけ"));
            }
            thread_local! {
                static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
            }
            if DEPTH.with(|d| d.get()) >= 8 {
                return Err(format!("{path}: 系の取り込みが深すぎる（循環？）"));
            }
            DEPTH.with(|d| d.set(d.get() + 1));
            let r = if sysfile::is_system_file(&p) {
                sysfile::load(&p).map(|x| x.0)
            } else {
                load_state(&p, &self.env.glass_dirs, self.env.catalog, None)
            };
            DEPTH.with(|d| d.set(d.get() - 1));
            let s = r?;
            self.merge_glasses(&s);
            return Ok(s);
        }
        let opt = zmx::LoadOptions {
            cfg,
            catalog_dirs: self.env.glass_dirs.clone(),
            ..Default::default()
        };
        let s = zmx::load_with(&p, &opt).map_err(|e| format!("{}: {e}", p.display()))?;
        self.merge_glasses(&s);
        Ok(s)
    }

    /// ユーザーカタログの項目を群にする（と、その中の相対パスの基準）。項目のガラスカタログを足す
    pub(crate) fn user_group(
        &mut self,
        b: &Block,
    ) -> Result<(Block, PathBuf, (PathBuf, String)), String> {
        self.user_group_frame(b).map(|(g, d, k, _)| (g, d, k))
    }

    /// [`Self::user_group`] と、品の枠
    #[allow(clippy::type_complexity)]
    pub(crate) fn user_group_frame(
        &mut self,
        b: &Block,
    ) -> Result<(Block, PathBuf, (PathBuf, String), ItemFrame), String> {
        let Block::User {
            id,
            catalog,
            item,
            place,
        } = b
        else {
            unreachable!()
        };
        let path = self.base.join(catalog);
        let key = (
            lensbench_core::vfs::canonicalize(&path).unwrap_or_else(|_| path.clone()),
            item.clone(),
        );
        if self.users.contains(&key) {
            return Err(format!(
                "{}: 項目 {item} が自分自身を参照している（循環）",
                path.display()
            ));
        }
        let cat = UserCatalog::load_shared(&path)?;
        let it = cat
            .items
            .get(item)
            .ok_or_else(|| format!("{}: 項目 {item} はない", path.display()))?;
        let dirs = glass::catalog_dirs(&self.env.glass_dirs);
        let (g, missing) = glass::load_catalogs_shared(&it.glass_catalogs, &dirs);
        if !missing.is_empty() {
            return Err(format!(
                "{item}: ガラスカタログが見つからない: {}",
                missing.join(", ")
            ));
        }
        self.add_glasses(&g);
        // カタログの膜（フィルタの透過特性）。同じ名前で中身の違う膜は誤り
        for (k, spec) in &cat.coatings {
            let t = spec
                .table()
                .map_err(|e| format!("{}: 膜 {k}: {e}", path.display()))?;
            match self.coats.get(k) {
                Some(old) if *old != t => {
                    return Err(format!("{}: 膜 {k} がほかのカタログと違う", path.display()))
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

    pub(crate) fn lens_element(&mut self, src: &LensSource) -> Result<Element, String> {
        match src {
            LensSource::Zmx { zmx, lens, config } => {
                let s = self.load_zmx(zmx, *config)?;
                pick_lens(&s, *lens).map_err(|e| format!("{zmx}: {e}"))
            }
            LensSource::Catalog { vendor, part } => {
                let f = self
                    .env
                    .catalog
                    .ok_or_else(|| format!("レンズカタログ {vendor} {part}: カタログを読めない"))?;
                let txt = f(vendor, part)?;
                let raw = zmx::parse_text(&txt).map_err(|e| e.to_string())?;
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
                pick_lens(&s, 1).map_err(|e| format!("{vendor} {part}: {e}"))
            }
            LensSource::Surfaces { surfaces } => {
                if surfaces.len() < 2 {
                    return Err("玉には面が 2 つ以上要る".into());
                }
                let n = surfaces.len();
                let mut v = Vec::new();
                for (i, q) in surfaces.iter().enumerate() {
                    let mut s = surf(q)?;
                    if i + 1 == n {
                        s.t = 0.0;
                        s.mat.clear();
                    } else if is_air(&s.mat) {
                        return Err(format!("玉の面 {}: 後がガラスでない", i + 1));
                    }
                    v.push(s);
                }
                Ok(Element {
                    kind: Kind::Lens,
                    from: 0,
                    surfaces: v,
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
    ) -> Result<(), String> {
        let start = self.sf.len();
        self.block_inner(b, siblings, idx)?;
        // 鏡は鏡の面だけ（往路を戻る面は除く）。中で覚える
        if !matches!(b, Block::Mirror { .. } | Block::Splitter { .. }) {
            self.note(block_id(b), start);
        }
        Ok(())
    }

    pub(crate) fn block_inner(
        &mut self,
        b: &Block,
        siblings: &[Block],
        idx: usize,
    ) -> Result<(), String> {
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
                    e = elements::reverse(&e).map_err(|x| x.to_string())?;
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
                        c.rev_spans.push((start, c.sf.len() - 1));
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
            } => {
                if !on {
                    return Ok(());
                }
                let start = self.sf.len();
                let back = *back;
                let fold = tilt[0] != 0.0 || tilt[1] != 0.0;
                if fold {
                    self.put(cb([0.0, 0.0, tilt[0], tilt[1], 0.0], 0, 0.0));
                }
                let mut m = Surface {
                    mirror: true,
                    mat: "MIRROR".into(),
                    c: curv(*radius),
                    k: *conic,
                    semi: *semi,
                    coat: coat.clone(),
                    ap: surf_ap(*aperture, *semi),
                    ..Default::default()
                };
                if self.parity < 0.0 {
                    m.c = -m.c;
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
                self.note(id, start);
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
                            "鏡の後のブロックと、BS の分岐の両方がある（復路が BS の分岐へ進む経路）".into(),
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
                ..
            } => {
                let s = self.load_zmx(path, *config)?;
                self.zenv
                    .push((path.clone(), s.temperature, s.pressure, s.adjust_index));
                let els = elements::split(&s).map_err(|e| format!("{path}: {e}"))?;
                let inst = self.zpicks.len();
                self.zpicks.push(ZmxLinks {
                    path: path.clone(),
                    sign: 1.0,
                    pickups: s.pickups.clone(),
                    solves: s.solves.clone(),
                    image: s.image(),
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
                        c.note(&gr.id, start);
                        if gr.shift != 0.0 && c.has_mirror_since(start) {
                            return Err("鏡を含む群は動かせない（shift は 0 に）".into());
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
                self.users.push(key);
                let start = self.sf.len();
                let r = self.block(&g, &[], 0);
                self.users.pop();
                self.base = base;
                r?;
                // 品の枠は、外側の品（項目の中の項目でない）だけ置く
                if let Some((f, rev)) =
                    frame.filter(|_| self.users.is_empty() && self.sf.len() > start)
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
                            .filter(|q| q.typ != SurfType::CoordBrk)
                            .map(|q| q.t)
                            .sum();
                        f.flipped(t)
                    } else {
                        f
                    };
                    self.frames.push((name, f, start, self.sf.len() - 1));
                }
            }
            Block::Splitter { id, spec, place } => {
                self.splitter_forward(id, spec, place, siblings, idx)?;
            }
            Block::Detector(_) => {
                return Err(
                    "検出器は最後のブロックだけ（群・ユーザーカタログの中には置けない）".into(),
                )
            }
            Block::Group {
                blocks,
                place,
                shift,
                ..
            } => {
                let bl = blocks.clone();
                self.gap(*shift);
                let start = self.sf.len();
                self.split.levels.push(Level::Other);
                let r = self.placed(place, &mut |c: &mut Self| {
                    for (i, x) in bl.iter().enumerate() {
                        if c.split.done {
                            break;
                        }
                        c.block(x, &bl, i)?;
                    }
                    Ok(())
                });
                self.split.levels.pop();
                r?;
                if *shift != 0.0 && self.has_mirror_since(start) {
                    return Err("鏡を含む群は動かせない（shift は 0 に）".into());
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
