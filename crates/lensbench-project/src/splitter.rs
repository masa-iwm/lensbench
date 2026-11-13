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

use lensbench_core::math::{
    add, col, mm, mtv, mv, rot, scale, sub, transpose, xyz_angles, I3, M3, V3,
};
use lensbench_core::surface::{Aperture, Reflect, Surface, Tilt};
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
}

pub(crate) const A: usize = 0;
pub(crate) const B: usize = 1;
const TPAIR: [usize; 4] = [1, 0, 3, 2];
const RPAIR: [usize; 4] = [2, 3, 0, 1];
/// z を逆に（y のまわりに 180°）
const Y180: M3 = [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]];
/// 経路の文字が足りない（[`crate::paths`] が次の文字を試す）
pub(crate) const NEED: &str = "\u{0}経路の文字が足りない";
const MAX_PASSES: usize = 64;

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
    fn split_coat(&self) -> Result<String, String> {
        let n = self.ratio.is_some() as u8 + !self.coat.is_empty() as u8 + self.pbs.is_some() as u8;
        if n > 1 {
            return Err("ratio・coat・pbs はどれか 1 つ".into());
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

    pub(crate) fn check(&self, id: &str) -> Result<(), String> {
        let e = |m: &str| Err(format!("BS {id}: {m}"));
        if id.is_empty() {
            return Err("BS（splitter）には id が要る".into());
        }
        self.split_coat().map_err(|m| format!("BS {id}: {m}"))?;
        if let Some(t) = self.ratio {
            if !(0.0..=1.0).contains(&t) {
                return e("ratio は 0〜1");
            }
        }
        if let Some(p) = &self.pbs {
            if !(0.0..=1.0).contains(&p.tp) || !(0.0..=1.0).contains(&p.ts) {
                return e("pbs の tp・ts は 0〜1");
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
                "faces の {k} はない（膜を付けられる面: {}）",
                if names.is_empty() {
                    "なし".to_string()
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
                    return e("キューブの angle は [±45, 0] か [0, ±45]");
                }
                if self.size.is_nan() || self.size <= 0.0 {
                    return e("キューブの size（一辺）が要る");
                }
                if self.glass.is_empty() {
                    return e("キューブの glass が要る");
                }
            }
            SplitForm::Plate => {
                if self.thickness.is_nan() || self.thickness <= 0.0 {
                    return e("平板の thickness が要る");
                }
                if self.glass.is_empty() {
                    return e("平板の glass が要る");
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
    parent: Rc<Vec<Block>>,
    pidx: usize,
    plevel: Level,
    spec: Rc<SplitterSpec>,
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
}

/// 経路の文字列（T・R、大小どちらも）
pub(crate) fn parse_path(s: &str) -> Result<Vec<bool>, String> {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            'T' | 't' => Ok(true),
            'R' | 'r' => Ok(false),
            _ => Err(format!("経路 {s}: 文字は T か R")),
        })
        .collect()
}

impl<'a, 'b> Ctx<'a, 'b> {
    fn split_key(&self, id: &str) -> String {
        let mut k = String::new();
        for (p, it) in &self.users {
            k.push_str(&format!("{}#{it}/", p.display()));
        }
        k.push_str(id);
        k
    }

    fn letter(&mut self, arm: Arm) -> Result<bool, String> {
        let st = &mut self.split;
        st.passes += 1;
        if st.passes > MAX_PASSES {
            return Err(format!(
                "BS を {MAX_PASSES} 回より多く通る（経路が戻り続ける）"
            ));
        }
        if st.pos < st.path.len() {
            st.pos += 1;
            return Ok(st.path[st.pos - 1]);
        }
        if st.strict {
            return Err(NEED.into());
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
        let d = [
            snap(dv[0], 1e-12),
            snap(dv[1], 1e-12),
            snap(ang[0], 1e-10),
            snap(ang[1], 1e-10),
            snap(ang[2], 1e-10),
        ];
        self.sf.last_mut().unwrap().t += snap(dv[2], 1e-12);
        if d.iter().any(|&v| v != 0.0) {
            self.put(crate::cb(d, 0, 0.0));
        }
    }

    /// BS を口 p から通す（面を足して、出る口の軸へ移す）。出る口を返す
    fn split_pass(&mut self, key: &str, p: usize, t: bool) -> Result<usize, String> {
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
                Act::T(m) => s.mat = if m == "AIR" { String::new() } else { m },
                Act::R(m) => {
                    s.mirror = true;
                    s.mat = "MIRROR".into();
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
    fn traverse_branch(&mut self, key: &str, port: usize) -> Result<(), String> {
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
    ) -> Result<(), String> {
        spec.check(id)?;
        if spec.has_branches() && idx + 1 < siblings.len() {
            return Err(format!(
                "BS {id}: 分岐（t・r・d）のある BS の後ろにはブロックを置けない（分岐の中に書く）"
            ));
        }
        let key = self.split_key(id);
        let level = self.split.levels.last().cloned().unwrap_or(Level::Other);
        if self.sf.is_empty() {
            self.put(crate::cb([0.0; 5], 0, 0.0));
        }
        let (o, r) = self.current_frame();
        let r = if self.parity < 0.0 { mm(&r, &Y180) } else { r };
        let [dx, dy] = place.decenter;
        let [tx, ty, tz] = place.tilt;
        let s = apply_cb(
            o,
            r,
            &Tilt {
                d: [dx, dy, tx, ty, tz],
                order: 0,
            },
        );
        self.split.info.insert(
            key.clone(),
            SplitInfo {
                s,
                fwd: B,
                off: !place.on,
                parent: Rc::new(siblings.to_vec()),
                pidx: idx,
                plevel: level,
                spec: Rc::new(spec.clone()),
                id: id.to_string(),
            },
        );
        if !place.on {
            if place.off == OffMode::Air {
                self.gap(spec.length());
            }
            if spec.has_branches() {
                self.traverse_branch(&key, B)?;
            }
            return Ok(());
        }
        let t = self.letter(spec.arm)?;
        let out = self.split_pass(&key, A, t)?;
        if spec.has_branches() {
            self.traverse_branch(&key, out)?;
        } else {
            self.split.info.get_mut(&key).unwrap().fwd = out;
        }
        Ok(())
    }

    /// 復路で分岐のない BS を通る（往路で出た口から入る）
    pub(crate) fn splitter_back(&mut self, id: &str) -> Result<(), String> {
        let key = self.split_key(id);
        let Some(info) = self.split.info.get(&key).cloned() else {
            return Err(format!("BS {id}: 往路で通っていない"));
        };
        if info.off {
            if info.spec.length() != 0.0 && self.split_off_air(&info) {
                self.gap(info.spec.length());
            }
            return Ok(());
        }
        let t = self.letter(info.spec.arm)?;
        let out = self.split_pass(&key, info.fwd, t)?;
        if out != A {
            self.split.stopped = true;
        }
        Ok(())
    }

    fn split_off_air(&self, info: &SplitInfo) -> bool {
        match &info.parent[info.pidx] {
            Block::Splitter { place, .. } => place.off == OffMode::Air,
            _ => false,
        }
    }

    /// 復路で並びの初めまで戻った（level はその並びの段）。BS の分岐なら BS を通る
    pub(crate) fn return_at(&mut self, level: Level) -> Result<(), String> {
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
            let out = self.split_pass(&key, port, t)?;
            if out != A {
                if !info.spec.branch(out).is_empty() {
                    self.traverse_branch(&key, out)?;
                }
                self.split.stopped = true;
                return Ok(());
            }
        }
        for x in info.parent[..info.pidx].iter().rev() {
            self.back_block(x)?;
            if self.split.stopped {
                return Ok(());
            }
        }
        self.return_at(info.plevel.clone())
    }
}
