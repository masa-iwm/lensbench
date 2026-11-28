//! 光線の出来事と経路: 光線が面で何をしたか（透過・反射・全反射・散乱・吸収・検出）の列。
//!
//! 非順次の追跡は経路を追跡の結果として見つけ、光束（[`crate::Beam`] の path・turn）とゴースト（面 a・b の 2 回の
//! 反射）は経路を 1 本決めて順次の系に展開する。どちらも同じ型で書き、面はプロジェクトの面の名前 [`FaceId`]
//! （ブロックの木の位置＋面の番号）で指す。順次の系の経路は、展開した系の面の列から作る（[`RayPath::sequential`]・
//! [`RayPath::ghost`]）。
//!
//! 鏡の `back`・`turn` の復路の面は、往路と別の名前（`back`、たどった順の番号）になる。物として同じ面かどうかは、
//! 物を大域に 1 回だけ置く場面（非順次の段 1）で決める。

use crate::{FaceId, SurfOrigin};
use lensbench_core::{invalid, Result, System};
use std::fmt;

/// 出来事の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EventKind {
    /// 透過（屈折）
    Transmit,
    /// 反射（鏡・境界の部分反射・BS の反射）
    Reflect,
    /// 全反射
    Tir,
    /// 散乱（透過した側へ）
    Scatter,
    /// 散乱（反射した側へ。鏡・光を通さない面の散乱も）
    ScatterReflect,
    /// 吸収（光線が終わる）
    Absorb,
    /// 検出（検出器・像面に届いて終わる）
    Detect,
}

impl EventKind {
    /// 短い記号（T・R・TIR・S・SR・A・D）
    pub fn code(self) -> &'static str {
        match self {
            EventKind::Transmit => "T",
            EventKind::Reflect => "R",
            EventKind::Tir => "TIR",
            EventKind::Scatter => "S",
            EventKind::ScatterReflect => "SR",
            EventKind::Absorb => "A",
            EventKind::Detect => "D",
        }
    }
    /// 向きを変える出来事（反射・全反射・反射した側への散乱）
    pub fn reflects(self) -> bool {
        matches!(
            self,
            EventKind::Reflect | EventKind::Tir | EventKind::ScatterReflect
        )
    }
    /// 光線が終わる出来事（吸収・検出）
    pub fn ends(self) -> bool {
        matches!(self, EventKind::Absorb | EventKind::Detect)
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// 出来事: どの面で何をしたか
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Event {
    /// プロジェクトの面の名前（プロジェクトから展開していない系・像面では None）
    pub face: Option<FaceId>,
    /// 経路を作った順次の系の面の番号（非順次では 0）
    pub surf: usize,
    pub kind: EventKind,
}

/// 経路: 出来事の列（光源から順に）
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct RayPath(pub Vec<Event>);

impl RayPath {
    /// 順次の系 s の面の列を通る経路: 無視する面・座標ブレーク・物体面を除く面 1..像面の前を通り（鏡と部分反射の面
    /// （光束の turn、BS の反射）は反射、ほかは透過）、像面で検出。map は展開の面の出どころ（[`crate::Expansion::map`]、
    /// 無ければ面の名前は None）
    pub fn sequential(s: &System, map: Option<&[SurfOrigin]>) -> Result<RayPath> {
        check_map(s, map)?;
        let img = s.image();
        let mut v: Vec<Event> = (1..img)
            .filter(|&j| counts(s, j))
            .map(|j| event(map, j, kind_of(s, j)))
            .collect();
        v.push(event(map, img, EventKind::Detect));
        Ok(RayPath(v))
    }

    /// 順次の系 s で、面 a で反射して戻り面 b（b < a）で反射して像面へ向かうゴーストの経路（解析のゴースト
    /// （`ghost_system`）と同じ面の約束。a は像面でもよい）
    pub fn ghost(s: &System, map: Option<&[SurfOrigin]>, a: usize, b: usize) -> Result<RayPath> {
        check_map(s, map)?;
        let img = s.image();
        if !(b < a && a <= img && b >= 1) {
            invalid!("ghost path: surfaces a = {a}, b = {b} (need 1 ≤ b < a ≤ {img})");
        }
        for k in [a, b] {
            if !counts(s, k) && k != img {
                invalid!("ghost path: surface {k} is ignored or a coordinate break");
            }
        }
        let t = |j: usize| event(map, j, kind_of(s, j));
        let mut v: Vec<Event> = (1..a).filter(|&j| counts(s, j)).map(t).collect();
        v.push(event(map, a, EventKind::Reflect));
        v.extend((b + 1..a).rev().filter(|&j| counts(s, j)).map(t));
        v.push(event(map, b, EventKind::Reflect));
        v.extend((b + 1..img).filter(|&j| counts(s, j)).map(t));
        v.push(event(map, img, EventKind::Detect));
        Ok(RayPath(v))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 反射（全反射を含む）の回数
    pub fn reflections(&self) -> usize {
        self.0.iter().filter(|e| e.kind.reflects()).count()
    }

    /// 出来事の種類の列（例: `T T R T R T D`）
    pub fn code(&self) -> String {
        self.0
            .iter()
            .map(|e| e.kind.code())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// 経路に数える面（物体面・無視する面・座標ブレークを除く）
fn counts(s: &System, j: usize) -> bool {
    j >= 1 && !s.surfaces[j].ignore && !s.surfaces[j].is_coord_break()
}

/// 順次の系の面 j での出来事（鏡・部分反射は反射）
fn kind_of(s: &System, j: usize) -> EventKind {
    let f = &s.surfaces[j];
    if f.is_mirror() || f.reflect.is_some() {
        EventKind::Reflect
    } else {
        EventKind::Transmit
    }
}

fn event(map: Option<&[SurfOrigin]>, j: usize, kind: EventKind) -> Event {
    Event {
        face: map.and_then(|m| m[j].face.clone()),
        surf: j,
        kind,
    }
}

fn check_map(s: &System, map: Option<&[SurfOrigin]>) -> Result<()> {
    if let Some(m) = map {
        if m.len() != s.surfaces.len() {
            invalid!(
                "the surface origins ({}) don't match the system ({} surfaces)",
                m.len(),
                s.surfaces.len()
            );
        }
    }
    Ok(())
}
