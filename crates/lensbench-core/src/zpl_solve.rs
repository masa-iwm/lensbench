//! ZPL Macro Solve: .zmx のソルブが呼ぶ ZPL のマクロを [`crate::zpl`] で走らせ、曲率・厚さ・円錐係数・半直径・面タイプのパラメータを
//! 決める。
//!
//! マクロの文（.ZPL）は読込のときに探す（[`crate::zmx::LoadOptions::macro_dirs`]、.zmx のフォルダ、その隣の
//! Macros、環境変数 `LENSBENCH_MACROS`）。見つからない・読めない・lensbench が対応していない命令・関数を使うマクロは、ソルブを外して
//! .zmx に保存された値で固定し、[`System::macro_warnings`] に理由を残す。
//!
//! マクロから読める系の値（[`SysHost`]）: SOSO(0)（ソルブのある面）、CURV・RADI・THIC・CONI・SDIA・PARM・INDX
//! （面の後の媒質の主波長の屈折率）、NSUR（像面の番号）・NWAV・WAVL（µm）・PWAV・NFLD・FLDX・FLDY。値は呼んだときの
//! 系のもの（ほかのソルブ・ピックアップを解いた後）。どれも OpticStudio 26.1.1 で値を確かめた（FLDX・FLDY を除く）。
//! 円錐係数・半直径のマクロは返した値がそのまま入る（半直径は絶対値）。
//!
//! 光線（OpticStudio 26.1.1 で値を確かめた）: `RAYTRACE hx, hy, px, py[, 波長]`（正規化した視野・瞳、エイミング・けられ係数は
//! 系のとおり、波長を省けば主波長、小数は切り捨て）の後、面 k の RAYX・RAYY・RAYZ（局所座標の交点）・RAYL・RAYM・RAYN
//! （面の後の向き）・RANX・RANY・RANZ（法線、−z 寄り）・RAGX〜RAGN（大域座標）・RAYT・RAYO（前の面からの長さ・光路長）・
//! OPTH（物体からの光路長。無限遠の物体では面 1 の頂点を通り光線に垂直な面から）、RAYE()（0、面 k に当たらなければ k、
//! 全反射なら −k）・RAYV()（けられた最初の面）・OPDC()（主光線基準の OPD [波]）。
//! - hx・hy・px・py のどれかの絶対値が 1 を超える、波長の番号がない（0 も）ときはマクロを止め、値を変えない
//! - RAYTRACE の前は 0。OpticStudio は先に同じ視野・波長の主光線を追うので、止まった光線の止まった面から後ろの値は
//!   主光線の値になる。止まった光線の OPTH・OPDC は 0
//! - 合わせていないこと: OpticStudio は光線の値を前のマクロから引き継ぐ（lensbench はマクロごとに 0 から）。
//!   1 回の更新で値を返さないソルブがあると後ろのソルブを評価しない（lensbench は評価する）。外れた面の法線。
//!   OPDC は lensbench の OPD で、参照球（射出瞳）の求め方の差で収差の大きい系では相対 1e-4 ほどずれることがある
//!
//! 自分の値を読むマクロ（例: 面 1 の厚さのソルブが THIC(1) を読む）は解き直すたびに値が変わる。OpticStudio は更新の
//! たびに決まった回数だけ走らせる（26.1.1 で THIC(1) + 1 が 15 増えた）が、lensbench では収まらないのでソルブを外す。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::zpl::{Flow, Host, Program, Value};
use crate::System;

/// 系に付けたマクロ（名前と文）。系ファイル（JSON）には文をそのまま入れ、戻すときにファイルは読まない
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Macro {
    /// .zmx に書かれた名前（拡張子なし）
    pub name: String,
    /// マクロの文（.ZPL の中身）
    pub source: String,
    /// 読んだファイル（記録だけ。系ファイルには書かない）
    #[cfg_attr(feature = "serde", serde(skip))]
    pub path: PathBuf,
    /// 解釈した文（JSON から戻したときは None で、使うときに source から解釈し直す）
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) prog: Option<Arc<Program>>,
}

impl Macro {
    /// 文 src を直に与えたマクロ（ファイルなし。テスト・埋め込み用）
    pub fn from_source(name: &str, src: &str) -> Result<Macro> {
        let prog = Program::parse(src).map_err(|e| e.at(format!("マクロ {name}")))?;
        Ok(Macro {
            name: name.to_string(),
            source: src.to_string(),
            path: PathBuf::new(),
            prog: Some(Arc::new(prog)),
        })
    }

    pub(crate) fn program(&self) -> Result<Arc<Program>> {
        match &self.prog {
            Some(p) => Ok(p.clone()),
            None => Ok(Arc::new(
                Program::parse(&self.source).map_err(|e| e.at(format!("マクロ {}", self.name)))?,
            )),
        }
    }
}

/// マクロを探す場所
pub fn macro_dirs(extra: &[PathBuf], zmx_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = extra.to_vec();
    if let Some(d) = zmx_dir {
        // フォルダを書かずに渡した .zmx（parent() が ""）は今のフォルダ。隣の Macros も探せるよう絶対パスにする
        let d = if d.as_os_str().is_empty() {
            Path::new(".")
        } else {
            d
        };
        let d = std::path::absolute(d).unwrap_or_else(|_| d.to_path_buf());
        if let Some(p) = d.parent() {
            v.push(d.clone());
            v.push(p.join("Macros"));
        } else {
            v.push(d);
        }
    }
    if let Some(e) = std::env::var_os("LENSBENCH_MACROS") {
        v.extend(std::env::split_paths(&e));
    }
    v
}

/// 名前 name（拡張子なし、大文字・小文字は区別しない）のマクロを探して解釈する
pub fn find_macro(name: &str, dirs: &[PathBuf]) -> Result<Macro> {
    let want = format!("{}.ZPL", name.trim()).to_uppercase();
    for d in dirs {
        let Ok(rd) = crate::vfs::read_dir(d) else {
            continue;
        };
        for e in rd {
            if e.file_name()
                .map(|n| n.to_string_lossy().to_uppercase())
                .unwrap_or_default()
                == want
            {
                let path = e;
                let source = crate::glass::read_text(&path)
                    .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
                let prog = Program::parse(&source).map_err(|e| e.at(path.display()))?;
                return Ok(Macro {
                    name: name.to_string(),
                    source,
                    path,
                    prog: Some(Arc::new(prog)),
                });
            }
        }
    }
    Err(Error::Io(format!(
        "マクロ {want} が見つからない（{} か所を探した）",
        dirs.len()
    )))
}

/// 面ごとの光線の値（RAYX … の順）
#[derive(Debug, Clone, Copy, Default)]
pub struct RayVals {
    /// 局所座標の交点・面の後の向き・法線
    pub xyz: [f64; 3],
    pub lmn: [f64; 3],
    pub nor: [f64; 3],
    /// 前の面からの幾何的な長さ・光路長（RAYT・RAYO）、物体からの光路長（OPTH）
    pub len: f64,
    pub opl: f64,
    pub opth: f64,
    /// 大域座標の交点・向き
    pub gp: [f64; 3],
    pub gk: [f64; 3],
}

/// マクロから系を読む口
pub struct SysHost<'a> {
    pub sys: &'a System,
    /// ソルブのある面
    pub surf: usize,
    /// 光線の値（面ごと）。OpticStudio と同じく、RAYTRACE で通らなかった面は前の光線の値が残る（初めは 0）
    pub ray: Vec<RayVals>,
    /// 最後の RAYTRACE の RAYE・RAYV・OPDC
    pub raye: f64,
    pub rayv: f64,
    pub opdc: f64,
}

impl<'a> SysHost<'a> {
    pub fn new(sys: &'a System, surf: usize) -> Self {
        SysHost {
            sys,
            surf,
            ray: vec![RayVals::default(); sys.surfaces.len()],
            raye: 0.0,
            rayv: 0.0,
            opdc: 0.0,
        }
    }

    /// RAYTRACE hx, hy, px, py[, 波長の番号]: 正規化した視野・瞳の実光線を像面まで追う（エイミング・けられ係数は系のとおり、
    /// 波長を省けば主波長）。OpticStudio 26.1.1 と同じく、hx・hy・px・py のどれかの絶対値が 1 を超える、波長の番号（小数は
    /// 切り捨て）がない、ならマクロを止めて値を変えない
    fn raytrace(&mut self, a: &[Value]) -> Result<Flow> {
        if !(4..=5).contains(&a.len()) {
            crate::notimpl!("RAYTRACE の引数 {} 個（4 か 5 個だけ）", a.len());
        }
        let mut x = [0.0; 5];
        for (i, v) in a.iter().enumerate() {
            match v {
                Value::Num(v) => x[i] = *v,
                _ => crate::invalid!("RAYTRACE の引数は数"),
            }
        }
        if let Some(v) = x[..4].iter().find(|v| !(v.abs() <= 1.0)) {
            return Ok(Flow::Abort(format!("RAYTRACE の座標 {v} が 1 を超える")));
        }
        let s = self.sys;
        let w = if a.len() == 5 {
            let w = x[4].trunc();
            if !(w >= 1.0 && w as usize <= s.wavelengths.len()) {
                return Ok(Flow::Abort(format!("RAYTRACE の波長の番号 {}", x[4])));
            }
            w as usize - 1
        } else {
            s.primary
        };
        let field = crate::system::FieldSel::Norm(x[0], x[1]);
        // OpticStudio は先に同じ視野・波長の主光線を追う: 外れ・全反射で止まった光線の、止まった面から後ろの値は
        // 主光線の値になる（26.1.1 で実測）
        if (x[2], x[3]) != (0.0, 0.0) {
            let c = s.try_trace(field, &[0.0], &[0.0], Some(w), None, true)?;
            self.fill(&c, w);
        }
        let t = s.try_trace(field, &[x[2]], &[x[3]], Some(w), None, true)?;
        self.fill(&t, w);
        let r = &t.rays[0];
        if r.err != 0 {
            // 止まった光線の OPTH は面によらず 0
            self.ray.iter_mut().for_each(|v| v.opth = 0.0);
        }
        self.raye = if r.tir { -(r.err as f64) } else { r.err as f64 };
        self.rayv = r.vig as f64;
        self.opdc = if r.err != 0 {
            0.0
        } else {
            crate::analysis::opd(s, field, &[x[2]], &[x[3]], Some(w), true)?.opd[0]
        };
        Ok(Flow::Go)
    }

    /// 光線 t（1 本、面ごとの記録つき）の値を、止まった面の手前まで入れる
    fn fill(&mut self, t: &crate::system::Trace, w: usize) {
        use crate::math::*;
        let s = self.sys;
        let fr = s.frames();
        // 物体面: 有限なら物体面の局所座標の出発点、無限遠なら面 1 の前の座標系の z = 0 面を通る点
        let (p0, k0) = (t.p0[0], t.k0[0]);
        let (o, m) = match &fr.surf[0] {
            Some(f) => (f.o, f.r),
            None => (fr.obj.o, fr.obj.r),
        };
        let q = mtv(&m, sub(p0, o));
        let kq = mtv(&m, k0);
        let (q, start) = if fr.surf[0].is_some() {
            (q, p0)
        } else {
            let u = -q[2] / kq[2];
            (add(q, scale(kq, u)), add(p0, scale(k0, u)))
        };
        // 物体が無限遠なら、光路長は面 1 の頂点を通り光線に垂直な面から（OpticStudio 26.1.1 で実測）
        let opl0 = if fr.surf[0].is_none() {
            s.index_before(1, w) * dot(q, kq)
        } else {
            0.0
        };
        let n0 = s.index_before(1, w);
        // 光路長の基準: 面 1 の前は出発点（無限遠なら z = 0 の面）からの長さ × 物体空間の屈折率
        let mut off = None;
        let (mut prev, mut kprev, mut lprev) = (start, k0, 0.0);
        let mut vals = vec![(
            0usize,
            RayVals {
                xyz: q,
                lmn: kq,
                // 法線は −z 寄り（OpticStudio と同じ向き）
                nor: [0.0, 0.0, -1.0],
                len: 0.0,
                opl: 0.0,
                opth: 0.0,
                gp: start,
                gk: k0,
            },
        )];
        for e in &t.rec {
            let Some(f) = &fr.surf[e.surf] else { continue };
            let en = e.entry(0);
            if !en.xyz[0].is_finite() {
                break;
            }
            let gk = mv(&f.r, en.lmn);
            let off = *off.get_or_insert_with(|| n0 * dot(sub(en.p, start), k0) - en.opl);
            // 前の面からの長さ（光線の向きに測る。後ろへ戻れば負）と、その光路長
            let opl = en.opl + off;
            vals.push((
                e.surf,
                RayVals {
                    xyz: en.xyz,
                    lmn: en.lmn,
                    nor: scale(
                        crate::trace::local_normal(&s.surfaces[e.surf], en.xyz),
                        -1.0,
                    ),
                    len: dot(sub(en.p, prev), kprev),
                    opl: opl - lprev,
                    opth: opl + opl0,
                    gp: en.p,
                    gk,
                },
            ));
            (prev, kprev, lprev) = (en.p, gk, opl);
        }
        for (j, v) in vals {
            self.ray[j] = v;
        }
    }
}

impl SysHost<'_> {
    fn surf_arg(&self, a: &[Value], name: &str) -> Result<usize> {
        let k = match a {
            [Value::Num(x)] => *x,
            _ => return Err(Error::Invalid(format!("{name} の引数は面の番号 1 つ"))),
        };
        let k = k.trunc();
        if k < 0.0 || k as usize >= self.sys.surfaces.len() {
            return Err(Error::Invalid(format!("{name}({k}): 面がない")));
        }
        Ok(k as usize)
    }
}

fn num(x: f64) -> Option<Result<Value>> {
    Some(Ok(Value::Num(x)))
}

impl Host for SysHost<'_> {
    fn function(&mut self, name: &str, a: &[Value]) -> Option<Result<Value>> {
        let s = self.sys;
        let surf = |h: &Self| h.surf_arg(a, name);
        let r = match name {
            "SOSO" => match a {
                [Value::Num(x)] if *x == 0.0 => num(self.surf as f64),
                _ => Some(Err(Error::NotImplemented(format!(
                    "SOSO の引数 {a:?}（0 だけ）"
                )))),
            },
            "CURV" => Some(surf(self).map(|k| Value::Num(s.surfaces[k].c))),
            "THIC" => Some(surf(self).map(|k| Value::Num(s.surfaces[k].t))),
            "CONI" => Some(surf(self).map(|k| Value::Num(s.surfaces[k].k))),
            "SDIA" => Some(surf(self).map(|k| Value::Num(s.surfaces[k].semi))),
            // 曲率半径（平面は未対応: OpticStudio の値を確かめていない）
            "RADI" => Some(surf(self).and_then(|k| match s.surfaces[k].c {
                0.0 => Err(Error::NotImplemented(format!("平面（面 {k}）の RADI"))),
                c => Ok(Value::Num(1.0 / c)),
            })),
            "RAYX" | "RAYY" | "RAYZ" | "RAYL" | "RAYM" | "RAYN" | "RANX" | "RANY" | "RANZ"
            | "RAYT" | "RAYO" | "OPTH" | "RAGX" | "RAGY" | "RAGZ" | "RAGL" | "RAGM" | "RAGN" => {
                Some(surf(self).map(|k| {
                    let v = &self.ray[k];
                    let i = |c: u8| (c - b'X') as usize;
                    let last = name.as_bytes()[3];
                    Value::Num(match &name[..3] {
                        "RAY" if b"XYZ".contains(&last) => v.xyz[i(last)],
                        "RAY" if b"LMN".contains(&last) => v.lmn[(last - b'L') as usize],
                        "RAN" => v.nor[i(last)],
                        "RAG" if b"XYZ".contains(&last) => v.gp[i(last)],
                        "RAG" => v.gk[(last - b'L') as usize],
                        "RAY" if last == b'T' => v.len,
                        "RAY" => v.opl,
                        _ => v.opth,
                    })
                }))
            }
            "RAYE" if a.is_empty() => num(self.raye),
            "RAYV" if a.is_empty() => num(self.rayv),
            "OPDC" if a.is_empty() => num(self.opdc),
            "INDX" => Some(surf(self).map(|k| Value::Num(s.surfaces[k].n[s.primary]))),
            "PARM" => Some(match a {
                [Value::Num(n), Value::Num(k)] => {
                    let k = k.trunc();
                    if k < 0.0 || k as usize >= s.surfaces.len() || *n < 0.0 || *n > 255.0 {
                        Err(Error::Invalid(format!("PARM({n}, {k})")))
                    } else {
                        let d = &s.surfaces[k as usize];
                        // Standard の面にはパラメータがなく、OpticStudio は 0 を返す（26.1.1 で実測）
                        let std0 = (d.typ == crate::surface::SurfType::Standard).then_some(0.0);
                        crate::param::surf_parm(d, *n as u8)
                            .or(std0)
                            .map(Value::Num)
                            .ok_or_else(|| Error::NotImplemented(format!("面 {k} の PARM {n}")))
                    }
                }
                _ => Err(Error::Invalid("PARM の引数は (番号, 面)".into())),
            }),
            "NSUR" if a.is_empty() => num((s.surfaces.len() - 1) as f64),
            "NWAV" if a.is_empty() => num(s.wavelengths.len() as f64),
            "PWAV" if a.is_empty() => num((s.primary + 1) as f64),
            "NFLD" if a.is_empty() => num(s.fields.len() as f64),
            "WAVL" | "FLDX" | "FLDY" => Some(match a {
                [Value::Num(i)] if *i >= 1.0 => {
                    let i = *i as usize - 1;
                    match name {
                        "WAVL" => s.wavelengths.get(i).copied(),
                        "FLDX" => s.fields.get(i).map(|f| f.x),
                        _ => s.fields.get(i).map(|f| f.y),
                    }
                    .map(Value::Num)
                    .ok_or_else(|| Error::Invalid(format!("{name}({}) がない", i + 1)))
                }
                _ => Err(Error::Invalid(format!("{name} の引数は番号 1 つ"))),
            }),
            _ => None,
        };
        r
    }

    fn command(&mut self, name: &str, a: &[Value]) -> Option<Result<Flow>> {
        match name {
            "RAYTRACE" => Some(self.raytrace(a)),
            _ => None,
        }
    }
}

/// 曲率・厚さ以外の列の ZPL マクロのソルブ（.zmx の PZPL 番号 "名前"・KZPL "名前"・DIAM 値 4 0 0 1 "名前"）
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MacroParm {
    pub surf: usize,
    /// パラメータの番号（.zmx の PARM の番号。col が Parm のときだけ使う）
    pub parm: u8,
    /// [`System::macros`] の番号
    pub id: u16,
    /// 列（既定はパラメータ）
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "MacroCol::is_parm")
    )]
    pub col: MacroCol,
}

/// マクロのソルブを付ける列
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum MacroCol {
    /// 面タイプのパラメータ（PZPL）
    #[default]
    Parm,
    /// 円錐係数（KZPL）
    Conic,
    /// 半直径（DIAM 値 4 0 0 1 "名前"）
    Semi,
}

impl MacroCol {
    pub fn is_parm(&self) -> bool {
        *self == MacroCol::Parm
    }
    /// 値を読み書きする量
    pub(crate) fn pick(self, parm: u8) -> crate::param::PickKind {
        use crate::param::PickKind;
        match self {
            MacroCol::Parm => PickKind::Parm(parm),
            MacroCol::Conic => PickKind::Conic,
            MacroCol::Semi => PickKind::Semi,
        }
    }
    /// 列の名前（警告用）
    pub fn label(self, parm: u8) -> String {
        match self {
            MacroCol::Parm => format!("パラメータ {parm}"),
            MacroCol::Conic => "円錐係数".into(),
            MacroCol::Semi => "半直径".into(),
        }
    }
}

impl System {
    /// マクロ m を面 surf のソルブとして走らせた値（OpticStudio と同じ扱い、[`Program::solve`]）。None は値を変えない
    pub(crate) fn run_macro(&self, m: &Macro, surf: usize) -> Result<Option<f64>> {
        let o = m
            .program()?
            .solve(&mut SysHost::new(self, surf))
            .map_err(|e| e.at(format!("面 {surf} のマクロ {}", m.name)))?;
        Ok(o.value)
    }

    /// パラメータのマクロのソルブを解く（[`System::solve_thickness`] の中で呼ぶ）。値を変えたら true
    pub(crate) fn solve_macro_parms(&mut self) -> Result<bool> {
        let mut moved = false;
        for i in 0..self.macro_parms.len() {
            let q = self.macro_parms[i];
            if self.surfaces[q.surf].ignore {
                continue;
            }
            let Some(m) = self.macros.get(q.id as usize) else {
                crate::invalid!("面 {}: マクロ {} がない", q.surf, q.id);
            };
            let Some(mut v) = self.run_macro(m, q.surf)? else {
                continue;
            };
            // 負の半直径は絶対値にする（OpticStudio 26.1.1 で実測）
            if q.col == MacroCol::Semi {
                v = v.abs();
            }
            let what = q.col.pick(q.parm);
            let old = crate::param::pick_get(&self.surfaces[q.surf], what);
            if old != Some(v) {
                if !crate::param::pick_set(&mut self.surfaces[q.surf], what, v) {
                    crate::notimpl!(
                        "面 {} の{}にマクロの値を入れる",
                        q.surf,
                        q.col.label(q.parm)
                    );
                }
                if !self.pickups.is_empty() {
                    self.resolve_pickups()?;
                }
                self.changed()?;
                let tol = 1e-14 * (1.0 + v.abs());
                moved |= old.map_or(true, |o| (o - v).abs() > tol);
            }
        }
        Ok(moved)
    }
}

impl System {
    /// マクロのソルブを外して保存された値で固定したもの（面・マクロの名前・理由）。空なら全部のマクロを解いている
    pub fn macro_warnings(&self) -> Vec<String> {
        self.macro_notes.clone()
    }
}
