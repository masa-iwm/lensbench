//! レンズデータの表で変えた面の値を、要素の中身に書く（文書モデル、GPN-01）。中身が面そのものの要素（自作の玉・
//! 平板・鏡・絞り・面の並び）は欄を書き換え（平板の半径は 2 面で 1 つの欄なので面ごとには上書き）、中身を変えない要素（カタログの玉・.zmx の玉・取り込み・BS・書けない
//! ブロック）と、欄で書けない値だけ面の上書き（`edits`）にする

use super::*;

/// 面の曲率半径の値（数・null・{"$f": "inf"}）。平面は 0
fn radius_of(v: &Value) -> Option<f64> {
    match v {
        Value::Null => Some(0.0),
        Value::Object(o) => match o.get("$f").and_then(Value::as_str) {
            Some("inf" | "-inf") => Some(0.0),
            _ => None,
        },
        _ => v.as_f64().map(|r| if r.is_finite() { r } else { 0.0 }),
    }
}

/// 面の表の 1 行に値 key を書く。書けたか
fn write_row(row: &mut SurfRow, key: &str, v: &Value, sign: f64) -> bool {
    if row.kind != SurfKind::Standard {
        return false;
    }
    match key {
        "r" => radius_of(v).map(|r| row.radius = sign * r).is_some(),
        "k" => v.as_f64().map(|k| row.conic = k).is_some(),
        "semi" => v
            .as_f64()
            .filter(|s| *s >= 0.0)
            .map(|s| {
                row.semi = s;
                row.semi_auto = false;
            })
            .is_some(),
        "semi_auto" => v.as_bool().map(|b| row.semi_auto = b).is_some(),
        _ => false,
    }
}

/// 面の後の値（厚さ・ガラス）を行に書く。書けたか
fn write_after(row: &mut SurfRow, key: &str, v: &Value) -> bool {
    match key {
        "t" => v.as_f64().map(|t| row.t = t).is_some(),
        "material" => match v {
            Value::Null => {
                row.glass.clear();
                true
            }
            Value::String(s) => {
                row.glass = s.trim().to_string();
                true
            }
            _ => false,
        },
        _ => false,
    }
}

impl Pick {
    /// 面の値の上書きを足す（同じ面・同じ値の前の上書きは置き換える）
    pub fn set_edit(&mut self, surface: usize, field: &str, value: Value) {
        self.edits
            .retain(|e| !(e.surface == surface && e.field == field));
        self.edits.push(SurfEdit {
            surface,
            field: field.into(),
            value,
        });
    }

    /// 要素の面 rel（座標ブレークを除いた番号、系の並びの向き）の値 key を変える。中身に書けなければ面の上書き
    pub fn write_face(&mut self, rel: usize, key: &str, value: Value) {
        if self.write_source(rel, key, &value) {
            // 中身に書いた値を、前の上書きが隠さないように
            self.edits.retain(|e| !(e.surface == rel && e.field == key));
        } else {
            self.set_edit(rel, key, value);
        }
    }

    /// 前に保存した面の上書きのうち、中身に書けるものを中身に移す（開いたとき。前は自作の玉の値も上書きだった）
    pub fn bake_edits(&mut self) {
        if self.edits.is_empty() {
            return;
        }
        let edits = std::mem::take(&mut self.edits);
        for e in edits {
            if !self.write_source(e.surface, &e.field, &e.value) {
                self.edits.push(e);
            }
        }
        // 面の数え方の印（自作の玉は面の表にすると変わる）
        self.edits_for = if self.edits.is_empty() && self.ops.is_empty() {
            String::new()
        } else {
            self.surf_sig()
        };
    }

    /// 中身（種類ごとの欄）に書く。書けたか
    fn write_source(&mut self, rel: usize, key: &str, v: &Value) -> bool {
        match self.kind {
            ElemKind::Lens => self.write_lens(rel, key, v),
            ElemKind::Surfaces => {
                // 面の並びの面 rel は、座標ブレークを除いて数えた rel 番目の行（向きは変えない）
                let n = self.surfs.len();
                let Some(i) = (0..n)
                    .filter(|&i| self.surfs[i].kind != SurfKind::Coordbrk)
                    .nth(rel)
                else {
                    return false;
                };
                let row = &mut self.surfs[i];
                // 面の並びは空気中だけ（ガラスは書かない）。最後の面の後は要素の後の空気
                write_row(row, key, v, 1.0)
                    || (row.kind == SurfKind::Standard
                        && key == "t"
                        && i + 1 < n
                        && write_after(row, key, v))
            }
            ElemKind::Plate => match (rel, key) {
                (0, "t") => v.as_f64().map(|t| self.t = t).is_some(),
                (0, "material") => v.as_str().map(|g| self.glass = g.trim().into()).is_some(),
                _ => false,
            },
            ElemKind::Mirror => match (rel, key) {
                (0, "r") => radius_of(v).map(|r| self.r1 = r).is_some(),
                (0, "k") => v.as_f64().map(|k| self.conic = k).is_some(),
                (0, "semi") => v
                    .as_f64()
                    .filter(|s| *s >= 0.0)
                    .map(|s| self.semi = s)
                    .is_some(),
                _ => false,
            },
            ElemKind::Stop => match (rel, key) {
                (0, "semi") => v
                    .as_f64()
                    .filter(|s| *s > 0.0)
                    .map(|s| self.semi = s)
                    .is_some(),
                _ => false,
            },
            _ => false,
        }
    }

    /// 自作の玉の面に書く。反転した玉は、系の面 rel が面の表の後ろから rel 番目（半径の符号も逆）
    fn write_lens(&mut self, rel: usize, key: &str, v: &Value) -> bool {
        let mut rows = self.lens_rows();
        let n = rows.len();
        if rel >= n {
            return false;
        }
        let (face, sign) = if self.reversed {
            (n - 1 - rel, -1.0)
        } else {
            (rel, 1.0)
        };
        let ok = match key {
            // 面の後の値: 反転した玉では、系の面 rel の後は表の面 face の前（face−1 の後）
            "t" | "material" => {
                let after = if self.reversed {
                    face.checked_sub(1)
                } else {
                    Some(face)
                };
                // 最後の面の後は要素の後の空気（呼ぶ側で扱う）。玉の中の空気（ガラスを空に）は玉のブロックに書けない
                // ので面の上書きのまま（前と同じく系の上で空気になる）
                let air = key == "material" && v.as_str().is_none_or(|g| g.trim().is_empty());
                match after.filter(|&i| i + 1 < n && !air) {
                    Some(i) => write_after(&mut rows[i], key, v),
                    None => false,
                }
            }
            _ => write_row(&mut rows[face], key, v, sign),
        };
        if !ok {
            return false;
        }
        // 2 面の単玉で、r1・r2・t・glass と同じ形なら欄のまま。ほか（コーニック・面ごとの半径）は面の表に
        let simple = n == 2
            && self.surfs.len() < 2
            && rows.iter().all(|r| r.conic == 0.0)
            && rows[0].semi == rows[1].semi
            && rows[0].semi_auto == rows[1].semi_auto;
        if simple {
            self.r1 = rows[0].radius;
            self.r2 = rows[1].radius;
            self.t = rows[0].t;
            self.glass = rows[0].glass.clone();
            self.semi = rows[0].semi;
            self.semi_auto = rows[0].semi_auto;
        } else {
            self.surfs = rows;
        }
        true
    }
}

impl Pick {
    /// 要素に面を足す（レンズデータの表の挿入、GPN-19）。pos は系の並びの向きで、足す面の前にある要素の面の数
    /// （座標ブレークを除く）。足した面は厚さ 0 の平面で、前後と同じ媒質にするので系は変わらない
    fn insert_rows(&mut self, pos: usize) -> Result<(), String> {
        match self.kind {
            ElemKind::Lens => {
                let mut rows = self.lens_rows();
                let n = rows.len();
                if pos > n {
                    return Err(tr("Surface outside the combination elements").into());
                }
                let at = if self.reversed { n - pos } else { pos };
                // 足した面は厚さ 0。玉の中は前の面の後のガラスの平面。両端は端の面の形を写し、端の面は同じガラスの
                // 間の面（屈折しない）になる（玉のブロックは面の間にガラスが要るので空気は挟めない）
                let mut new = SurfRow {
                    semi: rows[at.min(n - 1)].semi,
                    semi_auto: rows[at.min(n - 1)].semi_auto,
                    ..Default::default()
                };
                if at == 0 {
                    new.radius = rows[0].radius;
                    new.conic = rows[0].conic;
                    new.glass = rows[0].glass.clone();
                    rows.insert(0, new);
                } else if at == n {
                    new.radius = rows[n - 1].radius;
                    new.conic = rows[n - 1].conic;
                    rows[n - 1].glass = rows[n - 2].glass.clone();
                    rows[n - 1].t = 0.0;
                    rows.push(new);
                } else {
                    new.glass = rows[at - 1].glass.clone();
                    rows.insert(at, new);
                }
                self.surfs = rows;
                Ok(())
            }
            ElemKind::Surfaces => {
                let at = (0..self.surfs.len())
                    .filter(|&i| self.surfs[i].kind != SurfKind::Coordbrk)
                    .nth(pos)
                    .unwrap_or(self.surfs.len());
                self.surfs.insert(at, SurfRow::default());
                Ok(())
            }
            _ => Err(tr(
                "Surfaces can be added or removed only in a custom lens or a surface list",
            )
            .into()),
        }
    }

    /// 要素の面 rel（系の並びの向き、座標ブレークを除く）を消す（レンズデータの表の削除）
    fn delete_rows(&mut self, rel: usize) -> Result<(), String> {
        match self.kind {
            ElemKind::Lens => {
                let mut rows = self.lens_rows();
                let n = rows.len();
                if n <= 2 {
                    return Err(tr("A lens needs at least 2 surfaces").into());
                }
                if rel >= n {
                    return Err(tr("Surface outside the combination elements").into());
                }
                let at = if self.reversed { n - 1 - rel } else { rel };
                rows.remove(at);
                // 最後の面の後は空気（要素の後の空気）
                if let Some(last) = rows.last_mut() {
                    last.glass.clear();
                }
                self.surfs = rows;
                Ok(())
            }
            ElemKind::Surfaces => {
                let i = (0..self.surfs.len())
                    .filter(|&i| self.surfs[i].kind != SurfKind::Coordbrk)
                    .nth(rel)
                    .ok_or_else(|| tr("Surface outside the combination elements").to_string())?;
                if self.surfs.len() <= 1 {
                    return Err(tr("A surface list needs at least 1 surface").into());
                }
                self.surfs.remove(i);
                Ok(())
            }
            _ => Err(tr(
                "Surfaces can be added or removed only in a custom lens or a surface list",
            )
            .into()),
        }
    }
}

impl Pick {
    /// 面を足す（[`Self::insert_rows`]）。この要素の面の上書き・絞り・ソルブ・ピックアップは番号をずらして残す
    pub fn insert_face(&mut self, pos: usize) -> Result<(), String> {
        self.insert_rows(pos)?;
        self.shift_faces(&|i| Some(if i >= pos { i + 1 } else { i }));
        Ok(())
    }

    /// 面を消す（[`Self::delete_rows`]）。消した面の上書き・操作は外し、後ろの面の番号を詰める
    pub fn delete_face(&mut self, rel: usize) -> Result<(), String> {
        self.delete_rows(rel)?;
        self.shift_faces(&|i| match i.cmp(&rel) {
            std::cmp::Ordering::Less => Some(i),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(i - 1),
        });
        Ok(())
    }

    /// この要素の面（座標ブレークを除く番号）を map で付け替える。None になった面の上書き・操作は外す
    pub fn shift_faces(&mut self, map: &dyn Fn(usize) -> Option<usize>) {
        self.edits.retain_mut(|e| match map(e.surface) {
            Some(i) => {
                e.surface = i;
                true
            }
            None => false,
        });
        let own = |r: &mut BlockRef| match r {
            BlockRef::Sys(Ref::Index(i)) => map(*i).map(|j| *i = j).is_some(),
            _ => true,
        };
        self.ops.retain_mut(|o| map_refs(o, &own));
        // 面の数え方の印も今の並びに（印が古いと、開いたときに上書きを外してしまう）
        if !self.edits_for.is_empty() {
            self.edits_for = self.surf_sig();
        }
    }
}

/// 操作の面を f で書き換える。f が false を返した面があれば false（その操作は外す）
fn map_refs(o: &mut SurfaceOp, f: &dyn Fn(&mut BlockRef) -> bool) -> bool {
    match o {
        SurfaceOp::SetStop(s) => f(&mut s.surface),
        SurfaceOp::AddSolve(s) => f(&mut s.surface) && s.other.as_mut().is_none_or(f),
        SurfaceOp::AddPickup(s) => f(&mut s.dst) && f(&mut s.src),
    }
}

impl Combo {
    /// 要素 id の面を map で付け替えたとき、ほかの要素の操作（{"block": id, "surface": i}）も合わせる
    pub fn shift_block_refs(&mut self, id: &str, map: &dyn Fn(usize) -> Option<usize>) {
        fn walk(ps: &mut [Pick], f: &dyn Fn(&mut BlockRef) -> bool) {
            for p in ps {
                p.ops.retain_mut(|o| map_refs(o, f));
                for b in p.ports.iter_mut() {
                    walk(&mut b.picks, f);
                }
            }
        }
        let f = |r: &mut BlockRef| match r {
            BlockRef::Block(b) if b.block == id => map(b.surface).map(|j| b.surface = j).is_some(),
            _ => true,
        };
        walk(&mut self.picks, &f);
    }
}

impl Combo {
    /// 要素（分岐の先も）の前に保存した面の上書きのうち、中身に書けるものを中身に移す（開いたとき）
    pub fn bake_edits(&mut self) {
        fn walk(ps: &mut [Pick]) {
            for p in ps {
                p.bake_edits();
                for b in p.ports.iter_mut() {
                    walk(&mut b.picks);
                }
            }
        }
        walk(&mut self.picks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lens() -> Pick {
        let mut p = Pick::new(ElemKind::Lens);
        p.r1 = 50.0;
        p.r2 = -50.0;
        p.t = 5.0;
        p.glass = "N-BK7".into();
        p.semi = 10.0;
        p
    }

    /// 自作の玉の R・厚さ・ガラスは欄に書き、上書きは残さない。コーニックは面の表にする
    #[test]
    fn custom_lens_takes_values() {
        let mut p = lens();
        p.set_edit(0, "r", json!(70.0));
        p.write_face(0, "r", json!(60.0));
        p.write_face(0, "t", json!(4.0));
        p.write_face(0, "material", json!("F2"));
        assert_eq!((p.r1, p.t, p.glass.as_str()), (60.0, 4.0, "F2"));
        assert!(p.edits.is_empty() && p.surfs.is_empty());
        // 平面
        p.write_face(1, "r", json!({ "$f": "inf" }));
        assert_eq!(p.r2, 0.0);
        p.write_face(1, "k", json!(-1.0));
        assert_eq!(p.surfs.len(), 2);
        assert_eq!((p.surfs[0].radius, p.surfs[1].conic), (60.0, -1.0));
        // 玉の中を空気にするのは上書き
        p.write_face(0, "material", json!(""));
        assert_eq!((p.glass.as_str(), p.edits.len()), ("F2", 1));
        p.edits.clear();
        // 最後の面の後（要素の後の空気）・欄の無い値は上書き
        p.write_face(1, "t", json!(9.0));
        p.write_face(0, "coating", json!("AR"));
        assert_eq!(p.edits.len(), 2);
    }

    /// 反転した玉: 系の面 0 は表の最後の面（半径の符号が逆）、系の面 0 の後の厚さは表の面 0 の後
    #[test]
    fn reversed_lens_maps_faces() {
        let mut p = lens();
        p.reversed = true;
        p.write_face(0, "r", json!(40.0));
        assert_eq!(p.r2, -40.0);
        p.write_face(0, "t", json!(6.0));
        assert_eq!(p.t, 6.0);
        p.write_face(1, "r", json!(-30.0));
        assert_eq!(p.r1, 30.0);
        assert!(p.edits.is_empty());
    }

    /// 中身に書いた系は、同じ値を面の上書きにした系と同じ（3 面の貼り合わせ玉・反転・平板・鏡・面の並び）
    #[test]
    fn same_system_as_override() {
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let env = lensbench_project::Env {
            base: root.clone(),
            glass_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let sys = |p: &Pick| {
            let c = Combo {
                picks: vec![p.clone()],
                ..Default::default()
            };
            let pr = lensbench_project::Project::from_json(&c.project().to_string()).unwrap();
            let s = lensbench_project::expand_state(&pr, &env, None).unwrap();
            format!("{:?}", s.surfaces).replace("-0.0,", "0.0,")
        };
        let mut l3 = lens();
        l3.surfs = vec![
            SurfRow {
                radius: 50.0,
                t: 5.0,
                glass: "N-BK7".into(),
                semi: 10.0,
                ..Default::default()
            },
            SurfRow {
                radius: -40.0,
                t: 2.0,
                glass: "F2".into(),
                semi: 10.0,
                ..Default::default()
            },
            SurfRow {
                radius: -120.0,
                semi: 10.0,
                ..Default::default()
            },
        ];
        let mut l3r = l3.clone();
        l3r.reversed = true;
        let mut lr = lens();
        lr.reversed = true;
        let mut plate = Pick::new(ElemKind::Plate);
        plate.glass = "N-BK7".into();
        let mut surfs = Pick::new(ElemKind::Surfaces);
        surfs.surfs = vec![
            SurfRow {
                radius: 80.0,
                t: 3.0,
                semi: 8.0,
                ..Default::default()
            },
            SurfRow {
                semi: 8.0,
                ..Default::default()
            },
        ];
        let vals = [
            ("r", json!(77.0)),
            ("r", json!({ "$f": "inf" })),
            ("k", json!(-0.5)),
            ("t", json!(3.5)),
            ("material", json!("F2")),
            ("semi", json!(7.0)),
        ];
        for base in [
            lens(),
            lr,
            l3,
            l3r,
            plate,
            Pick::new(ElemKind::Mirror),
            surfs,
        ] {
            for rel in 0..3 {
                for (key, v) in &vals {
                    let mut a = base.clone();
                    a.write_face(rel, key, v.clone());
                    if !a.edits.is_empty() {
                        continue;
                    }
                    let mut b = base.clone();
                    b.set_edit(rel, key, v.clone());
                    assert_eq!(sys(&a), sys(&b), "{:?} 面 {rel} {key}={v}", base.kind);
                }
            }
        }
    }

    /// 前に保存した上書きは、書けるものだけ中身に移る
    #[test]
    fn bake_old_edits() {
        let mut p = lens();
        p.set_edit(0, "r", json!(60.0));
        p.set_edit(0, "coating", json!("AR"));
        p.edits_for = p.surf_sig();
        p.bake_edits();
        assert_eq!(p.r1, 60.0);
        assert_eq!(p.edits.len(), 1);
        assert_eq!(p.edits_for, p.surf_sig());
    }

    /// 面を足しても系（焦点距離・バックフォーカス）は変わらず、面が 1 つ増える。消すと 1 つ減る（下限あり）
    #[test]
    fn insert_and_delete_faces() {
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let env = lensbench_project::Env {
            base: root.clone(),
            glass_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let sys = |p: &Pick| {
            let c = Combo {
                picks: vec![p.clone()],
                ..Default::default()
            };
            let pr = lensbench_project::Project::from_json(&c.project().to_string()).unwrap();
            lensbench_project::expand_state(&pr, &env, None).unwrap()
        };
        let mut l3 = lens();
        l3.surfs = vec![
            SurfRow {
                radius: 50.0,
                t: 5.0,
                glass: "N-BK7".into(),
                semi: 10.0,
                ..Default::default()
            },
            SurfRow {
                radius: -40.0,
                t: 2.0,
                glass: "F2".into(),
                semi: 10.0,
                ..Default::default()
            },
            SurfRow {
                radius: -120.0,
                semi: 10.0,
                ..Default::default()
            },
        ];
        let mut l3r = l3.clone();
        l3r.reversed = true;
        let mut lr = lens();
        lr.reversed = true;
        for base in [lens(), lr, l3, l3r] {
            let s0 = sys(&base);
            let c0 = lensbench_core::report::cardinal(&s0, 0);
            let n = base.lens_rows().len();
            for pos in 0..=n {
                let mut p = base.clone();
                p.insert_face(pos).unwrap();
                let s1 = sys(&p);
                let c1 = lensbench_core::report::cardinal(&s1, 0);
                assert_eq!(s1.surfaces.len(), s0.surfaces.len() + 1);
                assert!(
                    (c1.efl - c0.efl).abs() < 1e-9,
                    "{pos}: {} {}",
                    c1.efl,
                    c0.efl
                );
                assert!(
                    (c1.bfl - c0.bfl).abs() < 1e-9,
                    "{pos}: {} {}",
                    c1.bfl,
                    c0.bfl
                );
                // 足した面は消せる
                p.delete_face(pos).unwrap();
                assert_eq!(sys(&p).surfaces.len(), s0.surfaces.len());
            }
        }
        // 2 面の玉は消せない。カタログの玉は足せない
        assert!(lens().delete_face(0).is_err());
        assert!(Pick::new(ElemKind::Catalog).insert_face(0).is_err());
        // 面の並び: 座標ブレークを除いて数える
        let mut p = Pick::new(ElemKind::Surfaces);
        p.surfs = vec![
            SurfRow {
                kind: SurfKind::Coordbrk,
                ..Default::default()
            },
            SurfRow {
                radius: 9.0,
                ..Default::default()
            },
        ];
        p.insert_face(1).unwrap();
        assert_eq!(p.surfs.len(), 3);
        assert_eq!(p.surfs[1].radius, 9.0);
        p.delete_face(0).unwrap();
        assert_eq!(p.surfs[1].radius, 0.0);
    }

    /// 面を足し引きしても、上書き・絞り・ほかの要素からのピックアップは番号をずらして残る（消した面の分は外す）
    #[test]
    fn faces_shift_with_insert_and_delete() {
        let mut p = lens();
        p.surfs = lens().lens_rows();
        p.surfs[1].glass = "F2".into();
        p.surfs[1].t = 2.0;
        p.surfs.push(SurfRow::default());
        p.set_edit(2, "coating", json!("AR"));
        p.ops.push(SurfaceOp::SetStop(StopOp {
            surface: BlockRef::Sys(Ref::Index(2)),
        }));
        p.edits_for = p.surf_sig();
        let mut other = Pick::new(ElemKind::Lens);
        other.ops.push(
            serde_json::from_value(json!({ "op": "add_pickup",
                "dst": 0, "src": { "block": "L1", "surface": 2 }, "what": "c" }))
            .unwrap(),
        );
        let mut c = Combo {
            picks: vec![p, other],
            ..Default::default()
        };
        c.picks[0].insert_face(1).unwrap();
        c.shift_block_refs("L1", &|x| Some(if x >= 1 { x + 1 } else { x }));
        let p = &c.picks[0];
        assert_eq!(p.edits[0].surface, 3);
        assert!(
            matches!(&p.ops[0], SurfaceOp::SetStop(s) if s.surface == BlockRef::Sys(Ref::Index(3)))
        );
        assert_eq!(p.edits_for, p.surf_sig());
        let src = |c: &Combo| match &c.picks[1].ops[0] {
            SurfaceOp::AddPickup(o) => o.src.clone(),
            _ => unreachable!(),
        };
        assert!(matches!(src(&c), BlockRef::Block(b) if b.surface == 3));
        // 印の合った上書きは、開き直しても外れない
        assert_eq!(c.drop_stale_edits(), 0);
        // 消した面の分は外れる
        c.picks[0].delete_face(3).unwrap();
        c.shift_block_refs("L1", &|x| (x != 3).then(|| x - usize::from(x > 3)));
        assert!(c.picks[0].edits.is_empty() && c.picks[0].ops.is_empty());
        assert!(c.picks[1].ops.is_empty());
    }

    /// 中身を変えない要素（カタログの玉）は上書き
    #[test]
    fn catalog_keeps_overrides() {
        let mut p = Pick::new(ElemKind::Catalog);
        p.write_face(0, "r", json!(60.0));
        assert_eq!(p.edits.len(), 1);
    }

    /// 面の並び: 座標ブレークを除いて数える
    #[test]
    fn surfaces_skip_coordinate_breaks() {
        let mut p = Pick::new(ElemKind::Surfaces);
        p.surfs = vec![
            SurfRow {
                kind: SurfKind::Coordbrk,
                ..Default::default()
            },
            SurfRow::default(),
        ];
        p.write_face(0, "r", json!(25.0));
        assert_eq!(p.surfs[1].radius, 25.0);
        assert!(p.edits.is_empty());
    }
}
