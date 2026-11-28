//! ビームスプリッター（`"type": "splitter"`）と経路（BS を通るたびの透過 T・反射 R の並び）。
//!
//! BS は 4 つの口（ポート）を持つ: A（入射）、B（A の透過）、C（A の反射）、D（C の反対、B から戻る光の反射）。
//! 透過は A↔B・C↔D、反射は A↔C・B↔D をつなぐ。BS の面（キューブの 4 面と斜面、平板の表裏、理想面）は
//! BS の座標（A から入る軸が +z、原点は A の面の中心・平板の表の頂点・理想面）に固定し、通る順に座標ブレークで
//! その位置へ移して足す。どの口から入っても面の位置は同じなので、往きと戻りで同じ面を通る。
//!
//! - 経路は BS を通る順の T・R の文字列（`TR` = 1 回目透過・2 回目反射）。足りない文字は BS の `arm`。
//! - 分岐 `t`・`r`・`d` は口 B・C・D から先のブロック（光の進む向きで書く）。分岐のある BS は並びの最後に置く。
//!   分岐のない BS は、後ろのブロックが 1 回目に出た口（B か C）の先に続く。
//! - 鏡の `back` で戻って BS に来たら、文字で口を選ぶ。A へ出ればさらに前のブロックを戻り、ほかの口へ出れば
//!   その口の分岐へ進む（分岐がなければ鏡の後のブロックへ続く）。
//! - 口の軸: 原点はキューブなら各面の中心、平板なら A・B は表の頂点と軸上で厚さだけ進んだ点（傾けた玉と同じ約束）、
//!   C・D は分ける面の頂点、理想面は面の上。向きは A の軸（B）と、分ける面で反射した向き（C）・その逆（D）。

use std::rc::Rc;
use std::sync::Arc;

use lensbench_core::math::{
    add, col, dot, mm, mtv, mv, rot, scale, sub, transpose, xyz_angles, I3, M3, V3,
};
use lensbench_core::surface::{Aperture, Material, Reflect, Surface, Tilt};
use lensbench_core::system::{apply_cb, step_frame};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Block, Ctx, OffMode, Place};

/// BS の形
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SplitForm {
    /// キューブ（2 つの直角プリズムを貼り合わせた形、斜面が 45°）
    #[default]
    Cube,
    /// 平板（表裏の 2 面、くさびも可）
    Plate,
    /// 厚さのない理想の面（空気中）
    Ideal,
}

/// 平板で分ける面
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SplitFace {
    #[default]
    Front,
    Back,
}

/// 経路の既定（文字を指定しないとき）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Arm {
    #[default]
    T,
    R,
}

/// 理想の偏光ビームスプリッター（強度の透過率。反射は 1 − T）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct PbsSpec {
    #[serde(default = "one_f")]
    pub tp: f64,
    #[serde(default)]
    pub ts: f64,
}

fn one_f() -> f64 {
    1.0
}

/// 大域の座標での置き方（[`SplitterSpec::at`]）。大域の座標は最初のブロックの頂点が原点で、z が最初の光軸の向き
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct At {
    /// 口 A の軸の原点 [mm]
    pub o: [f64; 3],
    /// 口 A の軸（入る光の向きが z）の傾き x・y・z [deg]（大域の軸から x → y → z の順に回す）
    #[serde(default)]
    pub tilt: [f64; 3],
}

impl At {
    /// 口 A の軸の座標（大域）
    fn frame(&self) -> (V3, M3) {
        let [tx, ty, tz] = self.tilt;
        apply_cb(
            self.o,
            I3,
            &Tilt {
                d: [0.0, 0.0, tx, ty, tz],
                order: 0,
            },
        )
    }
}

pub(crate) fn default_angle() -> [f64; 2] {
    [45.0, 0.0]
}

/// splitter ブロックの中身（[`Block::Splitter`]）
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SplitterSpec {
    #[serde(default)]
    pub form: SplitForm,
    /// キューブの一辺 [mm]
    #[serde(default)]
    pub size: f64,
    /// 平板の中心の厚さ [mm]（表の法線の向き）
    #[serde(default)]
    pub thickness: f64,
    /// キューブ・平板の硝材
    #[serde(default)]
    pub glass: String,
    /// 平板・理想面の半径 [mm]（0 なら切らない）
    #[serde(default)]
    pub semi: f64,
    /// 平板のくさび: 裏面を表面に対して傾ける角 x・y [deg]（裏面の頂点のまわり）
    #[serde(default)]
    pub wedge: [f64; 2],
    /// 平板で分ける面（front: 表、back: 裏）
    #[serde(default)]
    pub split_face: SplitFace,
    /// 分ける面の傾き x・y [deg]（鏡の tilt と同じ約束）。キューブは ±45 の一方だけ
    #[serde(default = "default_angle")]
    pub angle: [f64; 2],
    /// 分岐比（偏光によらない理想膜の強度の透過率）。ratio・coat・pbs のどれもなければ 0.5
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    /// 分ける面の膜（膜ファイル・LBCM の名前）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub coat: String,
    /// 理想の偏光ビームスプリッター
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pbs: Option<PbsSpec>,
    /// 分ける面以外の面の膜の既定（空なら膜なし）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ar: String,
    /// 面ごとの膜（キューブは a・b・c・d、平板は front・back。分ける面には付けない）
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub faces: std::collections::BTreeMap<String, String>,
    /// 経路の文字がないときに選ぶ口
    #[serde(default)]
    pub arm: Arm,
    /// 口 B（透過）の先のブロック
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub t: Vec<Block>,
    /// 口 C（反射）の先のブロック
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub r: Vec<Block>,
    /// 口 D（B から戻る光の反射）の先のブロック
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub d: Vec<Block>,
    /// 大域の座標に置く（省くと、ブロックの並びで前のブロックの後に置く）。前のブロックから BS の口 A までは
    /// 座標ブレークで移す（前の air の長さは位置に効かない）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<At>,
}

pub(crate) const A: usize = 0;
pub(crate) const B: usize = 1;
const TPAIR: [usize; 4] = [1, 0, 3, 2];
const RPAIR: [usize; 4] = [2, 3, 0, 1];
/// z を逆に（y のまわりに 180°）
const Y180: M3 = [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]];
const MAX_PASSES: usize = 64;

/// 経路の一覧（[`paths`]）に入れる輪の周回の数。同じ経路で一度通った BS へ合流で戻る（リング: サニャック・リング
/// 共振器）たびに 1 周と数え、これを超える経路は一覧に入れない（周回は限りなく続きうるので）。光束に書く経路は
/// これより多く回ってもよい（BS を通る回数の上限 MAX_PASSES まで）
pub const RING_TRIPS: usize = 3;

/// 口へ入る光束の向きと口の内向きの軸の内積（余弦）の下限。これより口の面に沿う光束は口へ入らないとする
const HEAD_INTO_MIN: f64 = 1e-6;

/// 面での作用
enum Act {
    /// 透過して媒質へ（"AIR" か硝材）
    T(String),
    /// 反射（反対側の媒質）
    R(String),
}

/// 面の位置（BS の座標）・開口・膜・半径
struct Face {
    o: V3,
    r: M3,
    ap: Aperture,
    coat: String,
    semi: f64,
}

impl SplitterSpec {
    pub(crate) fn branch(&self, port: usize) -> &[Block] {
        match port {
            1 => &self.t,
            2 => &self.r,
            3 => &self.d,
            _ => &[],
        }
    }
    pub(crate) fn has_branches(&self) -> bool {
        !(self.t.is_empty() && self.r.is_empty() && self.d.is_empty())
    }

    /// 分ける面の膜の名前
    fn split_coat(&self) -> lensbench_core::Result<String> {
        let n = self.ratio.is_some() as u8 + !self.coat.is_empty() as u8 + self.pbs.is_some() as u8;
        if n > 1 {
            return Err("exactly one of ratio, coat and pbs".into());
        }
        if let Some(p) = &self.pbs {
            return Ok(format!("I.s={},p={}", p.ts, p.tp));
        }
        if !self.coat.is_empty() {
            return Ok(self.coat.clone());
        }
        let t = self.ratio.unwrap_or(0.5);
        Ok(format!("I.s={t},p={t}"))
    }

    fn face_coat(&self, name: &str) -> String {
        self.faces
            .get(name)
            .cloned()
            .unwrap_or_else(|| self.ar.clone())
    }

    /// 分ける面の傾き R(θ) = Rx(tx) Ry(ty)
    fn tilt(&self) -> M3 {
        mm(&rot(0, self.angle[0]), &rot(1, self.angle[1]))
    }

    /// B の口までの軸上の長さ（OFF で空気に残す長さ）
    pub(crate) fn length(&self) -> f64 {
        match self.form {
            SplitForm::Cube => self.size,
            SplitForm::Plate => self.thickness,
            SplitForm::Ideal => 0.0,
        }
    }

    pub(crate) fn check(&self, id: &str) -> lensbench_core::Result<()> {
        let e = |m: &str| Err(lensbench_core::Error::Invalid(format!("BS {id}: {m}")));
        if id.is_empty() {
            return Err("a BS (splitter) needs an id".into());
        }
        self.split_coat().map_err(|m| format!("BS {id}: {m}"))?;
        if let Some(t) = self.ratio {
            if !(0.0..=1.0).contains(&t) {
                return e("ratio must be 0 to 1");
            }
        }
        if let Some(p) = &self.pbs {
            if !(0.0..=1.0).contains(&p.tp) || !(0.0..=1.0).contains(&p.ts) {
                return e("pbs tp and ts must be 0 to 1");
            }
        }
        let names: &[&str] = match self.form {
            SplitForm::Cube => &["a", "b", "c", "d"],
            SplitForm::Plate => match self.split_face {
                SplitFace::Front => &["back"],
                SplitFace::Back => &["front"],
            },
            SplitForm::Ideal => &[],
        };
        if let Some(k) = self.faces.keys().find(|k| !names.contains(&k.as_str())) {
            return e(&format!(
                "no {k} in faces (faces that can take a coating: {})",
                if names.is_empty() {
                    "none".to_string()
                } else {
                    names.join(", ")
                }
            ));
        }
        match self.form {
            SplitForm::Cube => {
                let [tx, ty] = self.angle;
                let ok = (tx.abs() == 45.0 && ty == 0.0) || (ty.abs() == 45.0 && tx == 0.0);
                if !ok {
                    return e("cube angle must be [±45, 0] or [0, ±45]");
                }
                if self.size.is_nan() || self.size <= 0.0 {
                    return e("a cube needs size (the edge length)");
                }
                if self.glass.is_empty() {
                    return e("a cube needs glass");
                }
            }
            SplitForm::Plate => {
                if self.thickness.is_nan() || self.thickness <= 0.0 {
                    return e("a plate needs thickness");
                }
                if self.glass.is_empty() {
                    return e("a plate needs glass");
                }
            }
            SplitForm::Ideal => {}
        }
        Ok(())
    }

    /// 反射した軸の座標（C・D の口の面の向き）: 鏡の折り曲げ（傾き R の座標ブレークの後に、同じ角で回す順を逆にした
    /// 座標ブレーク）と同じ R·Ry(ty)·Rx(tx)。z は −dC。1 軸の傾きでは R·R と同じ
    fn fold(&self) -> M3 {
        mm(
            &self.tilt(),
            &mm(&rot(1, self.angle[1]), &rot(0, self.angle[0])),
        )
    }

    /// 分ける面の法線（BS の座標）と、A から入る軸の反射の向き dC
    fn dirs(&self) -> (V3, V3) {
        let n = col(&self.tilt(), 2);
        let ez = [0.0, 0.0, 1.0];
        (n, sub(ez, scale(n, 2.0 * n[2])))
    }

    fn faces(&self) -> Vec<Face> {
        let r = self.tilt();
        let r2 = self.fold();
        let (n, dc) = self.dirs();
        match self.form {
            SplitForm::Cube => {
                let a = self.size;
                let h = 0.5 * a;
                let c = [0.0, 0.0, h];
                let sq = Aperture::Rect {
                    hx: h,
                    hy: h,
                    dx: 0.0,
                    dy: 0.0,
                };
                let semi = h * std::f64::consts::SQRT_2;
                let f = |o: V3, r: M3, name: &str| Face {
                    o,
                    r,
                    ap: sq,
                    coat: self.face_coat(name),
                    semi,
                };
                let (hx, hy) = if self.angle[0] != 0.0 {
                    (h, semi)
                } else {
                    (semi, h)
                };
                vec![
                    f([0.0; 3], I3, "a"),
                    f([0.0, 0.0, a], I3, "b"),
                    f(add(c, scale(dc, h)), r2, "c"),
                    f(add(c, scale(dc, -h)), r2, "d"),
                    Face {
                        o: c,
                        r,
                        ap: Aperture::Rect {
                            hx,
                            hy,
                            dx: 0.0,
                            dy: 0.0,
                        },
                        coat: String::new(),
                        semi: hx.max(hy),
                    },
                ]
            }
            SplitForm::Plate => {
                let ap = if self.semi > 0.0 {
                    Aperture::Circular {
                        min: 0.0,
                        max: self.semi,
                        dx: 0.0,
                        dy: 0.0,
                    }
                } else {
                    Aperture::None
                };
                let rk = mm(&mm(&r, &rot(0, self.wedge[0])), &rot(1, self.wedge[1]));
                vec![
                    Face {
                        o: [0.0; 3],
                        r,
                        ap,
                        coat: self.face_coat("front"),
                        semi: self.semi,
                    },
                    Face {
                        o: scale(n, self.thickness),
                        r: rk,
                        ap,
                        coat: self.face_coat("back"),
                        semi: self.semi,
                    },
                ]
            }
            SplitForm::Ideal => vec![Face {
                o: [0.0; 3],
                r,
                ap: if self.semi > 0.0 {
                    Aperture::Circular {
                        min: 0.0,
                        max: self.semi,
                        dx: 0.0,
                        dy: 0.0,
                    }
                } else {
                    Aperture::None
                },
                coat: String::new(),
                semi: self.semi,
            }],
        }
    }

    /// 分ける面の番号
    fn split_index(&self) -> usize {
        match (self.form, self.split_face) {
            (SplitForm::Cube, _) => 4,
            (SplitForm::Plate, SplitFace::Front) => 0,
            (SplitForm::Plate, SplitFace::Back) => 1,
            (SplitForm::Ideal, _) => 0,
        }
    }

    /// 口 p から入って t（透過）か反射で通る面と作用の列、出る口
    fn seq(&self, p: usize, t: bool) -> (Vec<(usize, Act)>, usize) {
        let out = if t { TPAIR[p] } else { RPAIR[p] };
        let g = || self.glass.clone();
        let air = || "AIR".to_string();
        let v = match self.form {
            SplitForm::Cube => vec![
                (p, Act::T(g())),
                (4, if t { Act::T(g()) } else { Act::R(g()) }),
                (out, Act::T(air())),
            ],
            SplitForm::Ideal => vec![(0, if t { Act::T(air()) } else { Act::R(air()) })],
            SplitForm::Plate => {
                // 表の側の口は A・C、裏の側は B・D
                let front_side = p == 0 || p == 2;
                let (s, o) = match self.split_face {
                    SplitFace::Front => (0, 1),
                    SplitFace::Back => (1, 0),
                };
                // 分ける面の側から入るか
                let on_split_side = front_side == (s == 0);
                if on_split_side {
                    if t {
                        vec![(s, Act::T(g())), (o, Act::T(air()))]
                    } else {
                        vec![(s, Act::R(g()))]
                    }
                } else if t {
                    vec![(o, Act::T(g())), (s, Act::T(air()))]
                } else {
                    vec![(o, Act::T(g())), (s, Act::R(air())), (o, Act::T(air()))]
                }
            }
        };
        (v, out)
    }

    /// 口 p の軸（原点、z が外向きの回転）
    fn port(&self, p: usize) -> (V3, M3) {
        let r2 = self.fold();
        let (n, dc) = self.dirs();
        let (oc, od) = match self.form {
            SplitForm::Cube => {
                let h = 0.5 * self.size;
                let c = [0.0, 0.0, h];
                (add(c, scale(dc, h)), add(c, scale(dc, -h)))
            }
            SplitForm::Plate => {
                let v = match self.split_face {
                    SplitFace::Front => [0.0; 3],
                    SplitFace::Back => scale(n, self.thickness),
                };
                (v, v)
            }
            SplitForm::Ideal => ([0.0; 3], [0.0; 3]),
        };
        match p {
            0 => ([0.0; 3], Y180),
            1 => ([0.0, 0.0, self.length()], I3),
            2 => (oc, mm(&r2, &Y180)),
            _ => (od, r2),
        }
    }
}

/// 展開中の BS の情報
#[derive(Clone)]
pub(crate) struct SplitInfo {
    /// BS の座標（大域）
    s: (V3, M3),
    /// 分岐のない BS で、1 回目に出た口（後ろのブロックが続く口）
    fwd: usize,
    off: bool,
    /// BS を置いた並びとその中の位置、その並びの段
    parent: Arc<Vec<Block>>,
    pidx: usize,
    plevel: Level,
    /// BS を置いたときの段の全部（外から。戻る道で親の並びを往路でたどり直すときに使う）
    plevels: Vec<Level>,
    spec: Arc<SplitterSpec>,
    id: String,
}

/// 今たどっている並びの段
#[derive(Clone, Debug)]
pub(crate) enum Level {
    /// 一番外・群の中
    Other,
    /// BS の分岐（BS の鍵、口）
    Branch { key: String, port: usize },
}

/// 経路の状態
#[derive(Default)]
pub(crate) struct SplitState {
    pub path: Vec<bool>,
    pub pos: usize,
    pub strict: bool,
    pub info: std::collections::HashMap<String, SplitInfo>,
    pub levels: Vec<Level>,
    /// 復路が BS で A 以外の口へ出た（前のブロックはもう戻らない）
    pub stopped: bool,
    /// 分岐の先まで進んだ（経路の終わり）
    pub done: bool,
    pub passes: usize,
    /// strict で経路の文字が足りなくなった
    pub need: bool,
    /// 復路が分岐の無い BS で A 以外の口へ出た（光束の `turn` では戻った先を置けない）
    pub loose: bool,
    /// 同じ経路で一度通った BS へ合流で戻った回数（輪の周回）
    pub rings: usize,
    /// strict で輪の周回が [`RING_TRIPS`] を超えた（[`paths`] の一覧に入れない）
    pub over: bool,
    /// 合流（[`Block::Join`]）の先の BS の置き方（ほかの経路でたどったもの、鍵ごと）
    pub(crate) places: Arc<std::collections::HashMap<String, SplitInfo>>,
    /// BS の置き方を集める（合流で経路を終える。[`splitter_places`]）
    pub(crate) collect:
        Option<Rc<std::cell::RefCell<std::collections::HashMap<String, SplitInfo>>>>,
}

/// 経路の文字列（T・R、大小どちらも）
pub(crate) fn parse_path(s: &str) -> lensbench_core::Result<Vec<bool>> {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            'T' | 't' => Ok(true),
            'R' | 'r' => Ok(false),
            _ => Err(format!("path {s}: letters must be T or R").into()),
        })
        .collect()
}

impl<'a, 'b> Ctx<'a, 'b> {
    fn split_key(&self, id: &str) -> String {
        let mut k = String::new();
        for l in &self.loading[self.outer..] {
            if let Some(it) = &l.item {
                k.push_str(&format!("{}#{it}/", l.file.display()));
            }
        }
        k.push_str(id);
        k
    }

    fn letter(&mut self, arm: Arm) -> lensbench_core::Result<bool> {
        let st = &mut self.split;
        st.passes += 1;
        if st.passes > MAX_PASSES {
            return Err(format!(
                "the BS is passed more than {MAX_PASSES} times (the path keeps coming back)"
            )
            .into());
        }
        if st.pos < st.path.len() {
            st.pos += 1;
            return Ok(st.path[st.pos - 1]);
        }
        if st.strict {
            // 経路の文字が足りない（[`crate::paths`] が次の文字を試す）
            st.need = true;
            return Err("the BS path has too few letters".into());
        }
        Ok(arm == Arm::T)
    }

    /// 今の位置の座標系（次の面へ渡す座標系、面 1 の前が原点）
    fn current_frame(&self) -> (V3, M3) {
        let (mut o, mut r) = ([0.0; 3], I3);
        for d in &self.sf {
            (_, (o, r)) = step_frame(o, r, d);
        }
        (o, r)
    }

    /// 座標系を (ot, rt) へ移す: 直前の面の厚さで z を、座標ブレーク（順序 0）で x・y と向きを
    fn goto(&mut self, ot: V3, rt: M3) {
        if self.sf.is_empty() {
            self.put(crate::cb([0.0; 5], 0, 0.0));
        }
        let (o, r) = self.current_frame();
        let dv = mtv(&r, sub(ot, o));
        let ang = xyz_angles(&mm(&transpose(&r), &rt));
        let snap = |v: f64, eps: f64| if v.abs() < eps { 0.0 } else { v };
        // 向きを変えて新しい軸の上を進むときは、回す座標ブレークの厚さで進む（OpticStudio の書き方と同じ。
        // 近軸の追跡が軸の上の長さを数えるように、ずらしで移さない）
        let dn = mtv(&rt, sub(ot, o));
        let turn = ang.iter().any(|a| a.abs() >= 1e-10);
        if turn && dn[0].abs() < 1e-12 && dn[1].abs() < 1e-12 && dn[2].abs() >= 1e-12 {
            let d = [
                0.0,
                0.0,
                snap(ang[0], 1e-10),
                snap(ang[1], 1e-10),
                snap(ang[2], 1e-10),
            ];
            self.put(crate::cb(d, 0, dn[2]));
            return;
        }
        let d = [
            snap(dv[0], 1e-12),
            snap(dv[1], 1e-12),
            snap(ang[0], 1e-10),
            snap(ang[1], 1e-10),
            snap(ang[2], 1e-10),
        ];
        self.sf.last_mut().expect("non-empty surfaces").t += snap(dv[2], 1e-12);
        if d.iter().any(|&v| v != 0.0) {
            self.put(crate::cb(d, 0, 0.0));
        }
    }

    /// BS を口 p から通す（面を足して、出る口の軸へ移す）。出る口を返す
    fn split_pass(&mut self, key: &str, p: usize, t: bool) -> lensbench_core::Result<usize> {
        let info = self.split.info[key].clone();
        let sp = &info.spec;
        let faces = sp.faces();
        let coat = sp.split_coat()?;
        let si = sp.split_index();
        let (so, sr) = info.s;
        let (v, out) = sp.seq(p, t);
        for (k, act) in v {
            let f = &faces[k];
            self.goto(add(so, mv(&sr, f.o)), mm(&sr, &f.r));
            let c = if k == si {
                coat.clone()
            } else {
                f.coat.clone()
            };
            let mut s = Surface {
                ap: f.ap,
                semi: f.semi,
                coat: c,
                comment: format!("{} {}", info.id, if k == si { "split" } else { "face" }),
                ..Default::default()
            };
            match act {
                Act::T(m) => s.mat = Material::parse(&m),
                Act::R(m) => {
                    s.mat = Material::Mirror;
                    s.reflect = Some(Reflect {
                        mat: m,
                        n: Vec::new(),
                    });
                    self.parity = -self.parity;
                }
            }
            self.put(s);
        }
        let (po, pr) = sp.port(out);
        let pr = if self.parity < 0.0 {
            mm(&pr, &Y180)
        } else {
            pr
        };
        self.goto(add(so, mv(&sr, po)), mm(&sr, &pr));
        Ok(out)
    }

    /// 分岐（口 port の先）を進む。終わりまで進んだら経路の終わり
    fn traverse_branch(&mut self, key: &str, port: usize) -> lensbench_core::Result<()> {
        let spec = self.split.info[key].spec.clone();
        let bl = spec.branch(port);
        self.split.levels.push(Level::Branch {
            key: key.to_string(),
            port,
        });
        let mut r = Ok(());
        for (i, x) in bl.iter().enumerate() {
            if self.split.done {
                break;
            }
            r = self.block(x, bl, i);
            if r.is_err() {
                break;
            }
        }
        self.split.levels.pop();
        r?;
        self.split.done = true;
        Ok(())
    }

    /// 往路で BS に来た（口 A から）
    pub(crate) fn splitter_forward(
        &mut self,
        id: &str,
        spec: &SplitterSpec,
        place: &Place,
        siblings: &[Block],
        idx: usize,
    ) -> lensbench_core::Result<()> {
        spec.check(id)?;
        if spec.has_branches() && idx + 1 < siblings.len() {
            return Err(format!(
                "BS {id}: no block can go after a BS with branches (t, r, d); write it in the branch"
            ).into());
        }
        let key = self.split_key(id);
        let level = self.split.levels.last().cloned().unwrap_or(Level::Other);
        if self.sf.is_empty() {
            self.put(crate::cb([0.0; 5], 0, 0.0));
        }
        let s = if let Some(at) = &spec.at {
            if place.moved() {
                return Err(format!("BS {id}: at can't be used with decenter or tilt").into());
            }
            at.frame()
        } else {
            let (o, r) = self.current_frame();
            let r = if self.parity < 0.0 { mm(&r, &Y180) } else { r };
            let [dx, dy] = place.decenter;
            let [tx, ty, tz] = place.tilt;
            apply_cb(
                o,
                r,
                &Tilt {
                    d: [dx, dy, tx, ty, tz],
                    order: 0,
                },
            )
        };
        self.split.info.insert(
            key.clone(),
            SplitInfo {
                s,
                fwd: B,
                off: !place.on,
                parent: Arc::new(siblings.to_vec()),
                pidx: idx,
                plevel: level,
                plevels: self.split.levels.clone(),
                spec: Arc::new(spec.clone()),
                id: id.to_string(),
            },
        );
        if !place.on {
            self.collect_place(&key);
            if place.off == OffMode::Air {
                self.gap(spec.length());
            }
            if spec.has_branches() {
                self.traverse_branch(&key, B)?;
            }
            return Ok(());
        }
        if spec.at.is_some() {
            self.head_into(id, &key, A)?;
        }
        let t = self.letter(spec.arm)?;
        let start = self.sf.len();
        let out = self.split_pass(&key, A, t)?;
        self.tag_split(id, start);
        if !spec.has_branches() {
            self.split
                .info
                .get_mut(&key)
                .expect("entry inserted above")
                .fwd = out;
        }
        self.collect_place(&key);
        if spec.has_branches() {
            self.traverse_branch(&key, out)?;
        }
        Ok(())
    }

    /// BS の置き方を集める（[`splitter_places`] の間だけ。最初にたどった置き方）
    fn collect_place(&mut self, key: &str) {
        if let Some(c) = &self.split.collect {
            c.borrow_mut()
                .entry(key.to_string())
                .or_insert_with(|| self.split.info[key].clone());
        }
    }

    /// 合流: BS id へ口 port から入る（[`Block::Join`]）。BS をまだ通っていなければ、ほかの経路でたどった置き方を使う
    pub(crate) fn join_forward(&mut self, id: &str, port: usize) -> lensbench_core::Result<()> {
        let key = self.split_key(id);
        if self.split.collect.is_some() {
            // 置き方を集める間は、合流で経路を終える
            self.split.done = true;
            return Ok(());
        }
        if self.split.info.contains_key(&key) {
            // 同じ経路で通った BS へ戻る（輪を 1 周した）
            self.split.rings += 1;
            if self.split.strict && self.split.rings > RING_TRIPS {
                self.split.over = true;
                return Err(
                    format!("join to {id}: more than {RING_TRIPS} trips around the ring").into(),
                );
            }
        } else {
            let Some(info) = self.split.places.get(&key).cloned() else {
                return Err(format!(
                    "join to {id}: no BS {id} is placed on a path that doesn't go through a join"
                )
                .into());
            };
            self.split.info.insert(key.clone(), info);
        }
        let info = self.split.info[&key].clone();
        if info.off {
            return Err(format!("join to {id}: BS {id} is OFF").into());
        }
        self.head_into(id, &key, port)?;
        let t = self.letter(info.spec.arm)?;
        let start = self.sf.len();
        let out = self.split_pass(&key, port, t)?;
        self.tag_split(id, start);
        if !info.spec.branch(out).is_empty() {
            self.traverse_branch(&key, out)?;
        } else if !info.spec.has_branches() && out == info.fwd {
            // 分岐の無い BS: 1 回目に出た口なら BS の後のブロックへ続く
            let parent = info.parent.clone();
            self.split.levels.push(info.plevel.clone());
            let mut r = Ok(());
            for (i, x) in parent.iter().enumerate().skip(info.pidx + 1) {
                if self.split.done {
                    break;
                }
                r = self.block(x, &parent, i);
                if r.is_err() {
                    break;
                }
            }
            self.split.levels.pop();
            r?;
        }
        self.split.done = true;
        Ok(())
    }

    /// 大域に置いた BS・合流の口へ入る光束は、口の外から口へ向かう（口の座標の z は外向き。鏡の後の向きは parity で逆）。
    /// 口の面に沿って来る（直角の配置で口を選び間違えた）光束は、内積が丸めで負になっても入らないので誤り
    fn head_into(&self, id: &str, key: &str, port: usize) -> lensbench_core::Result<()> {
        let info = &self.split.info[key];
        let (_, r) = self.current_frame();
        let (_, pr) = info.spec.port(port);
        let out_dir = mv(&mm(&info.s.1, &pr), [0.0, 0.0, 1.0]);
        let dir = scale(mv(&r, [0.0, 0.0, 1.0]), self.parity);
        if dot(dir, out_dir) > -HEAD_INTO_MIN {
            let name = ["A", "B", "C", "D"][port];
            return Err(format!("BS {id}: the beam doesn't head into port {name}").into());
        }
        Ok(())
    }

    /// 復路で分岐のない BS を通る（往路で出た口から入る）
    pub(crate) fn splitter_back(&mut self, id: &str) -> lensbench_core::Result<()> {
        let key = self.split_key(id);
        let Some(info) = self.split.info.get(&key).cloned() else {
            return Err(format!("BS {id}: not passed on the way out").into());
        };
        if info.off {
            if info.spec.length() != 0.0 && self.split_off_air(&info) {
                self.gap(info.spec.length());
            }
            return Ok(());
        }
        let t = self.letter(info.spec.arm)?;
        let start = self.sf.len();
        let out = self.split_pass(&key, info.fwd, t)?;
        self.tag_split(id, start);
        if out != A {
            self.split.stopped = true;
            self.split.loose = true;
        }
        Ok(())
    }

    /// BS を通って sf の start から足した面に BS の id を付ける（口の先の面は入れない。往路も復路も。GUI の要素の
    /// 面の範囲・レンズデータの表で BS の面の行を選ぶ）。玉・群のパラメータ（System::units）は増やさないので
    /// note は使わない
    fn tag_split(&mut self, id: &str, start: usize) {
        if id.is_empty() {
            return;
        }
        for o in &mut self.map[start..] {
            if !o.blocks.iter().any(|b| b == id) {
                o.blocks.push(id.to_string());
            }
        }
    }

    fn split_off_air(&self, info: &SplitInfo) -> bool {
        match &info.parent[info.pidx] {
            Block::Splitter { place, .. } => place.off == OffMode::Air,
            _ => false,
        }
    }

    /// 復路で並びの初めまで戻った（level はその並びの段）。BS の分岐なら BS を通る
    pub(crate) fn return_at(&mut self, level: Level) -> lensbench_core::Result<()> {
        let Level::Branch { key, port } = level else {
            return Ok(());
        };
        let info = self.split.info[&key].clone();
        if info.off {
            if self.split_off_air(&info) {
                self.gap(info.spec.length());
            }
        } else {
            let t = self.letter(info.spec.arm)?;
            let start = self.sf.len();
            let out = self.split_pass(&key, port, t)?;
            self.tag_split(&info.id, start);
            if out != A {
                if !info.spec.branch(out).is_empty() {
                    self.traverse_branch(&key, out)?;
                }
                self.split.stopped = true;
                return Ok(());
            }
        }
        self.back_seq(&info.parent, info.pidx, Some(&info.plevels))?;
        if self.split.stopped {
            return Ok(());
        }
        self.return_at(info.plevel.clone())
    }
}

/// 展開のしかた（合流の先の BS の置き方）
#[derive(Clone)]
pub(crate) enum Mode {
    /// ふつう（合流があれば、先に BS の置き方を集める）
    Normal,
    /// BS の置き方を集める（合流で経路を終える）
    Collect(Rc<std::cell::RefCell<std::collections::HashMap<String, SplitInfo>>>),
}

impl Mode {
    /// 展開の始めの BS の状態
    pub(crate) fn split_state(
        &self,
        p: &crate::Project,
        env: &crate::Env,
        cache: Option<&crate::SourceCache>,
        within: &[crate::Loading],
        path: Option<&str>,
        strict: bool,
    ) -> lensbench_core::Result<SplitState> {
        Ok(SplitState {
            path: parse_path(path.unwrap_or(""))?,
            strict,
            places: match self {
                Mode::Normal if has_join(&p.blocks) => {
                    splitter_places_cached(p, env, cache, within)?
                }
                _ => Default::default(),
            },
            collect: match self {
                Mode::Collect(c) => Some(c.clone()),
                Mode::Normal => None,
            },
            ..Default::default()
        })
    }
}

/// 展開（[`crate::expand::expand_body`]）を mode で
pub(crate) fn expand_mode(
    p: &crate::Project,
    env: &crate::Env,
    path: Option<&str>,
    search: bool,
    cache: Option<&crate::SourceCache>,
    within: &[crate::Loading],
    mode: Mode,
) -> Result<crate::expand::Walk, crate::ExpandError> {
    let mut failed = None;
    crate::expand::expand_body(p, env, path, search, cache, within, &mut failed, mode).map_err(
        |error| crate::ExpandError {
            error,
            block: failed.map(|n| crate::BlockAt::new(&p.blocks, n)),
        },
    )
}

/// BS の置き方（[`splitter_places`]）を cache に覚えて使い回す（合流のある系は展開のたびに置き方が要るので、
/// 経路を全部展開する呼び手（[`paths`]・場面の展開）では経路の数の 2 乗の展開になっていた）。覚える鍵は
/// プロジェクトの中身・環境（基準のフォルダ・ガラスのフォルダ）・外側の展開。集める間に読んだファイルの更新の印が
/// 変わっていれば集め直す。cache が無ければ毎回集める。当てた光束の `turn`（ファイルに書かない）・経路は外して集める
pub(crate) fn splitter_places_cached(
    p: &crate::Project,
    env: &crate::Env,
    cache: Option<&crate::SourceCache>,
    within: &[crate::Loading],
) -> lensbench_core::Result<Arc<std::collections::HashMap<String, SplitInfo>>> {
    // BS の置き方はブロックの木で決まる（光束の `turn` で戻る経路では BS を通らないことがある）ので、turn を外して
    // 集める。経路は集めるときに 1 本ずつ渡すので、プロジェクトの経路も外す（光束ごとの展開で置き方を使い回せる）
    let plain;
    let p = if !p.turn.is_empty() || p.path.is_some() {
        plain = crate::Project {
            turn: Vec::new(),
            path: None,
            ..p.clone()
        };
        &plain
    } else {
        p
    };
    let Some(c) = cache else {
        return Ok(Arc::new(splitter_places(p, env, None, within, false)?));
    };
    let key = places_key(p, env, within);
    if let Some(got) = c.places_get(&key) {
        return Ok(got);
    }
    let (got, read) =
        lensbench_core::vfs::recording(|| splitter_places(p, env, Some(c), within, false));
    let got = Arc::new(got?);
    c.places_put(key, &read, got.clone());
    Ok(got)
}

/// [`splitter_places_cached`] の鍵
fn places_key(p: &crate::Project, env: &crate::Env, within: &[crate::Loading]) -> String {
    let mut k = serde_json::to_string(p).unwrap_or_default();
    k.push_str(&format!("\n{:?}\n{:?}\n", env.base, env.glass_dirs));
    for l in within {
        k.push_str(&format!("{l}\n"));
    }
    k
}

/// 合流（[`Block::Join`]）の先の BS の置き方: 合流で経路を終えて経路を全部たどり、BS ごとに最初に置いた置き方を集める。
/// 合流の先の BS（木の中の合流の `splitter`）が全部集まれば、残りの経路はたどらない（深さ優先の順は同じなので、
/// 集まる置き方は全部たどったときと同じ。合流の先でない BS の置き方は使わない）。all なら全部の経路をたどる
pub(crate) fn splitter_places(
    p: &crate::Project,
    env: &crate::Env,
    cache: Option<&crate::SourceCache>,
    within: &[crate::Loading],
    all: bool,
) -> lensbench_core::Result<std::collections::HashMap<String, SplitInfo>> {
    let mut want = Vec::new();
    if !all {
        join_targets(&p.blocks, &mut want);
    }
    let col = Rc::new(std::cell::RefCell::new(std::collections::HashMap::new()));
    let mut todo = vec![String::new()];
    let mut n = 0;
    while let Some(q) = todo.pop() {
        n += 1;
        if n > 1024 {
            return Err("more than 1024 paths".into());
        }
        let mode = Mode::Collect(col.clone());
        if let Ok(crate::expand::Walk::Need) =
            expand_mode(p, env, Some(&q), true, cache, within, mode)
        {
            todo.push(format!("{q}R"));
            todo.push(format!("{q}T"));
        }
        if !want.is_empty() && want.iter().all(|w| col.borrow().contains_key(w)) {
            break;
        }
    }
    let out = col.borrow().clone();
    Ok(out)
}

/// プロジェクトの木の BS ごとの大域の置き方（[`At`]、BS の id → 置き方）。BS を大域に置く（`at`）ときに、今の
/// 置き方から始めるため（この値を `at` に書き、BS の偏心・傾きを外すと、BS は同じ所に置かれる）。どの経路でも
/// 通らない BS は無い
pub fn splitter_frames(
    p: &crate::Project,
    env: &crate::Env,
    cache: Option<&crate::SourceCache>,
) -> lensbench_core::Result<std::collections::BTreeMap<String, At>> {
    let got = splitter_places(p, env, cache, &[], true)?;
    Ok(got
        .into_iter()
        .map(|(k, v)| {
            let tilt =
                lensbench_core::math::xyz_angles(&v.s.1)
                    .map(|a| if a.abs() < 1e-12 { 0.0 } else { a });
            (k, At { o: v.s.0, tilt })
        })
        .collect())
}

/// ブロックの木の合流の先の BS の id（並びの中の順、重なりなし）
fn join_targets(blocks: &[Block], out: &mut Vec<String>) {
    for b in blocks {
        match b {
            Block::Join { splitter, .. } => {
                if !out.contains(splitter) {
                    out.push(splitter.clone());
                }
            }
            Block::Group { blocks, .. } => join_targets(blocks, out),
            Block::Splitter { spec, .. } => {
                for l in [&spec.t, &spec.r, &spec.d] {
                    join_targets(l, out);
                }
            }
            _ => {}
        }
    }
}

/// ブロックの木に合流があるか
pub(crate) fn has_join(blocks: &[Block]) -> bool {
    blocks.iter().any(|b| match b {
        Block::Join { .. } => true,
        Block::Group { blocks, .. } => has_join(blocks),
        Block::Splitter { spec, .. } => [&spec.t, &spec.r, &spec.d].iter().any(|l| has_join(l)),
        _ => false,
    })
}

/// プロジェクトでとれるビームスプリッターの経路の全部（BS を通る順の T・R の文字列）。BS がなければ空。
/// 合流で同じ BS へ戻る輪は [`RING_TRIPS`] 周までの経路。
/// 経路は BS ごとに透過・反射を選んで、BS を通らなくなるまで（像面まで）たどったもの。経路ごとにブロックを
/// たどって面を並べる（BS の復路でどの口へ出るかは面の位置で決まるので、面は作る）が、系を組んで確かめる・
/// 自動の半直径を光線で求める・機構部品を置く段はしない（その段の誤りは、その経路を展開したときに出る）
pub fn paths(p: &crate::Project, env: &crate::Env) -> lensbench_core::Result<Vec<String>> {
    paths_cached(p, env, &crate::SourceCache::default())
}

/// [`paths`] で、ファイル・カタログから読んだものを cache で使い回す
pub fn paths_cached(
    p: &crate::Project,
    env: &crate::Env,
    cache: &crate::SourceCache,
) -> lensbench_core::Result<Vec<String>> {
    let mut out = Vec::new();
    let mut todo = vec![String::new()];
    while let Some(q) = todo.pop() {
        match crate::expand::expand_inner(p, env, Some(&q), true, Some(cache), &[])
            .map_err(crate::ExpandError::into_error)
        {
            Ok(crate::expand::Walk::Done(_) | crate::expand::Walk::Walked) => out.push(q),
            // 輪を回りすぎる経路は一覧に入れない
            Ok(crate::expand::Walk::Over) => {}
            Ok(crate::expand::Walk::Need) => {
                if out.len() + todo.len() > 1024 {
                    return Err("more than 1024 paths".into());
                }
                todo.push(format!("{q}R"));
                todo.push(format!("{q}T"));
            }
            Err(e) => {
                return Err(if q.is_empty() {
                    e
                } else {
                    format!("path {q}: {e}").into()
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
