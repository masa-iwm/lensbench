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
                // 最後の面の後は要素の後の空気（呼ぶ側で扱う）
                match after.filter(|&i| i + 1 < n) {
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
