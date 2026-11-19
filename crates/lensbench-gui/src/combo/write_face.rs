//! レンズデータの表で変えた面の値を、要素の中身に書く（文書モデル）。中身が面そのものの要素（自作の玉・
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
    // 行で持つ面は中身を変えない（面の上書きにする）
    if row.kind != SurfKind::Standard || row.special().is_some() {
        return false;
    }
    match key {
        "r" => radius_of(v).map(|r| row.radius = sign * r).is_some(),
        // 曲率（0 は平面: 半径 0 の約束）
        "c" => v
            .as_f64()
            .map(|c| row.radius = if c == 0.0 { 0.0 } else { sign / c })
            .is_some(),
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
        // 欄の無い SurfSpec の値（more）。空にすると外す
        "comment" | "coating" => match v {
            Value::String(t) => {
                let k = if key == "coating" { "coat" } else { key };
                set_more(row, k, (!t.is_empty()).then(|| json!(t)));
                true
            }
            Value::Null => {
                set_more(row, if key == "coating" { "coat" } else { key }, None);
                true
            }
            _ => false,
        },
        // 偶数次非球面の係数（反転した玉では形が逆なので符号も逆。標準面の形のときだけ）
        "asph" if row.more.get("shape").is_none() => match v {
            Value::Array(a) => {
                let Some(c) = a.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>() else {
                    return false;
                };
                let c: Vec<f64> = c.iter().map(|x| sign * x).collect();
                set_more(row, "asph", c.iter().any(|x| *x != 0.0).then(|| json!(c)));
                true
            }
            Value::Null => {
                set_more(row, "asph", None);
                true
            }
            _ => false,
        },
        _ => false,
    }
}

/// more の鍵 k を v にする（None なら外す）
fn set_more(row: &mut SurfRow, k: &str, v: Option<Value>) {
    match v {
        Some(v) => {
            row.more.insert(k.into(), v);
        }
        None => {
            row.more.remove(k);
        }
    }
}

/// 面の後の値（厚さ・ガラス）を行に書く。書けたか
fn write_after(row: &mut SurfRow, key: &str, v: &Value) -> bool {
    if row.special().is_some() {
        return false;
    }
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
    pub(crate) fn set_edit(&mut self, surface: usize, field: &str, value: Value) {
        self.edits
            .retain(|e| !(e.surface == surface && e.field == field));
        self.edits.push(SurfEdit {
            surface,
            field: field.into(),
            value,
        });
    }

    /// 状態 n の面 surface の値 field の上書き（多重構成の表の面の行）
    pub(crate) fn state_edit(&self, n: &str, surface: usize, field: &str) -> Option<&Value> {
        self.state_edits
            .get(n)?
            .iter()
            .find(|e| e.surface == surface && e.field == field)
            .map(|e| &e.value)
    }

    /// 基本の面 surface の値 field の上書き
    pub(crate) fn base_edit(&self, surface: usize, field: &str) -> Option<&Value> {
        self.edits
            .iter()
            .find(|e| e.surface == surface && e.field == field)
            .map(|e| &e.value)
    }

    /// 状態 n の面 surface の値 field を value にする（同じ面・同じ値の上書きは置き換える）
    pub(crate) fn set_state_edit(&mut self, n: &str, surface: usize, field: &str, value: Value) {
        let v = self.state_edits.entry(n.to_string()).or_default();
        v.retain(|e| !(e.surface == surface && e.field == field));
        v.push(SurfEdit {
            surface,
            field: field.into(),
            value,
        });
        // 面の数え方の印（並びが変わったら外す、[`Pick::drop_stale_edits`]）
        if self.edits_for.is_empty() {
            self.edits_for = self.surf_sig();
        }
    }

    /// 状態 n の面 surface の値 field の上書きをやめる（基本の値に戻す）
    pub(crate) fn clear_state_edit(&mut self, n: &str, surface: usize, field: &str) {
        if let Some(v) = self.state_edits.get_mut(n) {
            v.retain(|e| !(e.surface == surface && e.field == field));
            if v.is_empty() {
                self.state_edits.remove(n);
            }
        }
    }

    /// 状態 n で使う面の上書き: 基本の上書きに状態の上書きを重ねたもの（状態の上書きが無ければ None）。
    /// プロジェクトの状態の set の surface_edits（ブロックの surface_edits を丸ごと置き換える）
    pub(crate) fn edits_in(&self, n: &str) -> Option<Vec<SurfEdit>> {
        let own = self.state_edits.get(n).filter(|v| !v.is_empty())?;
        let mut out: Vec<SurfEdit> = self
            .edits
            .iter()
            .filter(|e| {
                !own.iter()
                    .any(|o| o.surface == e.surface && o.field == e.field)
            })
            .cloned()
            .collect();
        out.extend(own.iter().cloned());
        Some(out)
    }

    /// 要素の面 rel（座標ブレークを除いた番号、系の並びの向き）の値 key を変える。中身に書けなければ面の上書き
    pub(crate) fn write_face(&mut self, rel: usize, key: &str, value: Value) {
        if self.write_source(rel, key, &value) {
            // 中身に書いた値を、前の上書きが隠さないように
            self.edits.retain(|e| !(e.surface == rel && e.field == key));
        } else {
            self.set_edit(rel, key, value);
        }
    }

    /// 前に保存した面の上書きのうち、中身に書けるものを中身に移す（開いたとき。前は自作の玉の値も上書きだった）
    pub(crate) fn bake_edits(&mut self) {
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
        // 座標ブレークの値は上書きで持つ
        if key.starts_with(crate::combo_tree::CB_FIELD) {
            return false;
        }
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
                // 面の並びは空気中だけ（ガラスは書かない）。最後の面の後は要素の後の空気。偏光子・波長板・Jones の
                // 面は厚さ・半径だけ（形は持てない）。種類ごとの値は要素の設定欄で
                match key {
                    "t" => i + 1 < n && write_after(row, key, v),
                    "semi" if row.kind != SurfKind::Standard => v
                        .as_f64()
                        .filter(|s| *s >= 0.0)
                        .map(|s| row.semi = s)
                        .is_some(),
                    _ => write_row(row, key, v, 1.0),
                }
            }
            ElemKind::Plate => match (rel, key) {
                (0, "t") => v.as_f64().map(|t| self.custom_mut().t = t).is_some(),
                (0, "material") => v
                    .as_str()
                    .map(|g| self.custom_mut().glass = g.trim().into())
                    .is_some(),
                _ => false,
            },
            // 行で書いた鏡（Zernike など）は欄を使わないので、面の上書きに回す
            ElemKind::Mirror if self.mirror_row().is_some() => false,
            ElemKind::Mirror => match (rel, key) {
                (0, "r") => radius_of(v).map(|r| self.mirror_mut().radius = r).is_some(),
                (0, "k") => v.as_f64().map(|k| self.mirror_mut().conic = k).is_some(),
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
            && rows.iter().all(|r| r.conic == 0.0 && r.more.is_empty())
            && rows[0].semi == rows[1].semi
            && rows[0].semi_auto == rows[1].semi_auto;
        if simple {
            self.custom_mut().r1 = rows[0].radius;
            self.custom_mut().r2 = rows[1].radius;
            self.custom_mut().t = rows[0].t;
            self.custom_mut().glass = rows[0].glass.clone();
            self.semi = rows[0].semi;
            self.custom_mut().semi_auto = rows[0].semi_auto;
        } else {
            self.surfs = rows;
        }
        true
    }
}

impl Pick {
    /// 要素に面を足す（レンズデータの表の挿入）。pos は系の並びの向きで、足す面の前にある要素の面の数
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
    pub(crate) fn insert_face(&mut self, pos: usize) -> Result<(), String> {
        let n = self.lens_rows().len();
        self.insert_rows(pos)?;
        self.shift_faces(&|i| Some(if i >= pos { i + 1 } else { i }));
        // 玉の端に足すと、空気との境目は足した面に移る。端の面の膜の上書きも新しい端の面へ（透過・偏光を変えない）
        if self.kind == ElemKind::Lens && (pos == 0 || pos == n) {
            let (from, to) = if pos == 0 { (1, 0) } else { (n - 1, n) };
            for e in &mut self.edits {
                if e.surface == from && e.field == "coating" {
                    e.surface = to;
                }
            }
        }
        Ok(())
    }

    /// 面を消す（[`Self::delete_rows`]）。消した面の上書き・操作は外し、後ろの面の番号を詰める
    pub(crate) fn delete_face(&mut self, rel: usize) -> Result<(), String> {
        self.delete_rows(rel)?;
        self.shift_faces(&|i| match i.cmp(&rel) {
            std::cmp::Ordering::Less => Some(i),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(i - 1),
        });
        Ok(())
    }

    /// この要素の面（座標ブレークを除く番号）を map で付け替える。None になった面の上書き・操作は外す
    pub(crate) fn shift_faces(&mut self, map: &dyn Fn(usize) -> Option<usize>) {
        let remap = |v: &mut Vec<SurfEdit>| {
            // 座標ブレークの上書きは座標ブレークの番号なので面の付け替えでは変えない
            v.retain_mut(|e| match map(e.surface) {
                _ if e.field.starts_with(crate::combo_tree::CB_FIELD) => true,
                Some(i) => {
                    e.surface = i;
                    true
                }
                None => false,
            })
        };
        remap(&mut self.edits);
        for v in self.state_edits.values_mut() {
            remap(v);
        }
        self.state_edits.retain(|_, v| !v.is_empty());
        self.shift_face_picks(map);
        self.state_stop.retain(|_, i| match map(*i) {
            Some(j) => {
                *i = j;
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

/// 面の並びの行を付け替えた（足す・消す・種類を変える）ときの、面（座標ブレークを除く番号）と座標ブレーク
/// （座標ブレークだけの番号）の付け替え。添字は前の番号、値は後の番号（無くなれば None）
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RowRemap {
    pub face: Vec<Option<usize>>,
    pub cb: Vec<Option<usize>>,
}

impl RowRemap {
    /// old・new は前と後の行ごとの座標ブレークか、row は前の行 → 後の行。種類が変わった行は無くなる
    pub(crate) fn of(old: &[bool], new: &[bool], row: &dyn Fn(usize) -> Option<usize>) -> RowRemap {
        let index =
            |kinds: &[bool], r: usize| kinds[..r].iter().filter(|&&c| c == kinds[r]).count();
        let mut out = RowRemap {
            face: Vec::new(),
            cb: Vec::new(),
        };
        for (r, &c) in old.iter().enumerate() {
            let to = row(r)
                .filter(|&n| new.get(n) == Some(&c))
                .map(|n| index(new, n));
            if c {
                out.cb.push(to);
            } else {
                out.face.push(to);
            }
        }
        out
    }

    /// 番号が変わらない
    pub(crate) fn is_identity(&self) -> bool {
        let same = |v: &[Option<usize>]| v.iter().enumerate().all(|(i, x)| *x == Some(i));
        same(&self.face) && same(&self.cb)
    }

    pub(crate) fn face_of(&self, i: usize) -> Option<usize> {
        self.face.get(i).copied().flatten()
    }

    pub(crate) fn cb_of(&self, i: usize) -> Option<usize> {
        self.cb.get(i).copied().flatten()
    }
}

impl Pick {
    /// 面の並びの行ごとの座標ブレークか
    pub(crate) fn cb_rows(&self) -> Vec<bool> {
        self.surfs
            .iter()
            .map(|r| r.kind == SurfKind::Coordbrk)
            .collect()
    }

    /// 面の並びの行を足した（add）・消した（del）・種類を変えた後に、前の行の種類 before から番号を付け替える。
    /// 変わったらその付け替え
    pub(crate) fn rows_changed(
        &mut self,
        before: &[bool],
        add: Option<usize>,
        del: Option<usize>,
    ) -> Option<RowRemap> {
        let row = |r: usize| -> Option<usize> {
            match (add, del) {
                (Some(i), _) => Some(r + usize::from(r >= i)),
                (_, Some(i)) => (r != i).then(|| r - usize::from(r > i)),
                _ => Some(r),
            }
        };
        let m = RowRemap::of(before, &self.cb_rows(), &row);
        if m.is_identity() {
            return None;
        }
        self.shift_rows(&m);
        Some(m)
    }

    /// この要素の面・座標ブレークの番号を m で付け替える（上書き・ピックアップ・絞り・自分の面の操作）。
    /// ほかの要素からの参照は [`Combo::shift_block_refs`]・[`Combo::shift_block_cb_refs`]
    pub(crate) fn shift_rows(&mut self, m: &RowRemap) {
        self.shift_faces(&|i| m.face_of(i));
        let remap = |v: &mut Vec<SurfEdit>| {
            v.retain_mut(|e| {
                if !e.field.starts_with(crate::combo_tree::CB_FIELD) {
                    return true;
                }
                match m.cb_of(e.surface) {
                    Some(i) => {
                        e.surface = i;
                        true
                    }
                    None => false,
                }
            })
        };
        remap(&mut self.edits);
        for v in self.state_edits.values_mut() {
            remap(v);
        }
        self.state_edits.retain(|_, v| !v.is_empty());
        self.shift_cb_picks(&|i| m.cb_of(i));
    }
}

/// 操作の面を f で書き換える。f が false を返した面があれば false（その操作は外す）
fn map_refs(o: &mut SurfaceOp, f: &dyn Fn(&mut BlockRef) -> bool) -> bool {
    match o {
        SurfaceOp::SetStop(s) => f(&mut s.surface),
        SurfaceOp::AddMacroSolve(s) => f(&mut s.surface),
        SurfaceOp::AddSolve(s) => f(&mut s.surface) && s.other.as_mut().is_none_or(f),
        SurfaceOp::AddPickup(s) => f(&mut s.dst) && f(&mut s.src),
        SurfaceOp::RemoveSolve { surface, .. } => f(surface),
        SurfaceOp::RemovePickup { dst, .. } => f(dst),
    }
}

impl Combo {
    /// 要素 id の面を map で付け替えたとき、ほかの要素の操作（{"block": id, "surface": i}）も合わせる
    pub(crate) fn shift_block_refs(&mut self, id: &str, map: &dyn Fn(usize) -> Option<usize>) {
        fn walk(ps: &mut [Pick], f: &dyn Fn(&mut BlockRef) -> bool) {
            for p in ps {
                p.ops.retain_mut(|o| map_refs(o, f));
                for b in p.ports.iter_mut() {
                    walk(&mut b.picks, f);
                }
            }
        }
        let f = |r: &mut BlockRef| match r {
            BlockRef::Block(b) if b.block == id && !b.coordbrk => {
                map(b.surface).map(|j| b.surface = j).is_some()
            }
            _ => true,
        };
        walk(&mut self.picks, &f);
        self.shift_glrs(id, false, map);
    }

    /// 大域座標の基準の面（`glrs` の {"block": id, "surface": i}、cb なら座標ブレーク）を map で付け替える。
    /// 面が無くなれば基準を既定（面 1）に戻す
    fn shift_glrs(&mut self, id: &str, cb: bool, map: &dyn Fn(usize) -> Option<usize>) {
        let Some(g) = self.extra.glrs.as_mut() else {
            return;
        };
        if g["block"] != id || (g["coordbrk"] == true) != cb {
            return;
        }
        let Some(i) = g["surface"].as_u64() else {
            return;
        };
        match map(i as usize) {
            Some(j) => g["surface"] = json!(j),
            None => self.extra.glrs = None,
        }
    }

    /// 要素 id の座標ブレークを map で付け替えたとき、操作の座標ブレークの参照（coordbrk のブロックの面、自分のも）
    /// も合わせる
    pub(crate) fn shift_block_cb_refs(&mut self, id: &str, map: &dyn Fn(usize) -> Option<usize>) {
        fn walk(ps: &mut [Pick], f: &dyn Fn(&mut BlockRef) -> bool) {
            for p in ps {
                p.ops.retain_mut(|o| map_refs(o, f));
                for b in p.ports.iter_mut() {
                    walk(&mut b.picks, f);
                }
            }
        }
        let f = |r: &mut BlockRef| match r {
            BlockRef::Block(b) if b.block == id && b.coordbrk => {
                map(b.surface).map(|j| b.surface = j).is_some()
            }
            _ => true,
        };
        walk(&mut self.picks, &f);
        self.shift_glrs(id, true, map);
    }

    /// 要素 id の行を m で付け替えたとき、ほかの要素からの面・座標ブレークの参照を合わせる
    pub(crate) fn shift_row_refs(&mut self, id: &str, m: &RowRemap) {
        if id.is_empty() || m.is_identity() {
            return;
        }
        self.shift_block_refs(id, &|i| m.face_of(i));
        self.shift_block_cb_refs(id, &|i| m.cb_of(i));
    }
}

impl Combo {
    /// 要素（分岐の先も）の前に保存した面の上書きのうち、中身に書けるものを中身に移す（開いたとき）
    pub(crate) fn bake_edits(&mut self) {
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
        p.custom_mut().r1 = 50.0;
        p.custom_mut().r2 = -50.0;
        p.custom_mut().t = 5.0;
        p.custom_mut().glass = "N-BK7".into();
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
        assert_eq!(
            (p.custom().r1, p.custom().t, p.custom().glass.as_str()),
            (60.0, 4.0, "F2")
        );
        assert!(p.edits.is_empty() && p.surfs.is_empty());
        // 平面
        p.write_face(1, "r", json!({ "$f": "inf" }));
        assert_eq!(p.custom().r2, 0.0);
        p.write_face(1, "k", json!(-1.0));
        assert_eq!(p.surfs.len(), 2);
        assert_eq!((p.surfs[0].radius, p.surfs[1].conic), (60.0, -1.0));
        // 玉の中を空気にするのは上書き
        p.write_face(0, "material", json!(""));
        assert_eq!((p.custom().glass.as_str(), p.edits.len()), ("F2", 1));
        p.edits.clear();
        // 最後の面の後（要素の後の空気）は上書き。欄の無い SurfSpec の値（膜）は面の more に
        p.write_face(1, "t", json!(9.0));
        p.write_face(0, "coating", json!("AR"));
        assert_eq!(p.edits.len(), 1);
        assert_eq!(p.surfs[0].more["coat"], "AR");
        p.write_face(0, "asph", json!([0.0, 1e-5]));
        assert_eq!(p.surfs[0].more["asph"], json!([0.0, 1e-5]));
        p.write_face(0, "coating", json!(""));
        assert!(p.surfs[0].more.get("coat").is_none());
    }

    /// 反転した玉: 系の面 0 は表の最後の面（半径の符号が逆）、系の面 0 の後の厚さは表の面 0 の後
    #[test]
    fn reversed_lens_maps_faces() {
        let mut p = lens();
        p.reversed = true;
        p.write_face(0, "r", json!(40.0));
        assert_eq!(p.custom().r2, -40.0);
        p.write_face(0, "t", json!(6.0));
        assert_eq!(p.custom().t, 6.0);
        p.write_face(1, "r", json!(-30.0));
        assert_eq!(p.custom().r1, 30.0);
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
            let s = lensbench_project::expand(
                &pr,
                &env,
                &lensbench_project::ExpandOpts {
                    state: None,
                    ..Default::default()
                },
            )
            .map(|x| x.system)
            .unwrap();
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
        plate.custom_mut().glass = "N-BK7".into();
        let mut surfs = Pick::new(ElemKind::Surfaces);
        surfs.surfs = vec![
            SurfRow {
                radius: 80.0,
                t: 3.0,
                semi: 8.0,
                ..Default::default()
            },
            SurfRow {
                t: 2.0,
                semi: 8.0,
                ..Default::default()
            },
            SurfRow {
                kind: SurfKind::Polarizer,
                axis_deg: 30.0,
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
            for rel in 0..4 {
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

    /// 行で書いた鏡（.zmx を分けた自由曲面の鏡）の r・k・semi は欄に書かず面の上書きになり、系に効く
    #[test]
    fn row_mirror_takes_overrides() {
        let mut m = Pick::new(ElemKind::Mirror);
        m.extra = json!({ "row": { "semi": 6, "action": "reflect",
            "shape": { "irregular": { "c": 0.01, "zs": 0.001 } } } });
        for (key, v) in [("r", json!(80.0)), ("k", json!(-0.5)), ("semi", json!(7.0))] {
            let mut a = m.clone();
            a.write_face(0, key, v.clone());
            assert_eq!(a.edits.len(), 1, "{key}");
            assert_eq!(a.mirror(), m.mirror(), "{key}");
            assert_eq!(a.semi, m.semi, "{key}");
            assert!(a.block("M")["row"].is_object(), "{key}");
            // 上書きは展開した系の鏡に効く
            let c = Combo {
                picks: vec![a],
                ..Default::default()
            };
            let pr = lensbench_project::Project::from_json(&c.project().to_string()).unwrap();
            let env = lensbench_project::Env::default();
            let s = lensbench_project::expand(&pr, &env, &Default::default())
                .unwrap()
                .system;
            let f = s.surfaces.iter().find(|f| f.is_mirror()).unwrap();
            match key {
                "r" => assert_eq!(f.c, 1.0 / 80.0),
                "k" => assert_eq!(f.k, -0.5),
                _ => assert_eq!(f.semi, 7.0),
            }
        }
    }

    /// 前に保存した上書きは、書けるものだけ中身に移る（膜は面の more に）
    #[test]
    fn bake_old_edits() {
        let mut p = lens();
        p.set_edit(0, "r", json!(60.0));
        p.set_edit(0, "coating", json!("AR"));
        p.edits_for = p.surf_sig();
        p.bake_edits();
        assert_eq!(p.surfs[0].radius, 60.0);
        assert_eq!(p.surfs[0].more["coat"], "AR");
        assert!(p.edits.is_empty());
        assert_eq!(p.edits_for, "");
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
            lensbench_project::expand(
                &pr,
                &env,
                &lensbench_project::ExpandOpts {
                    state: None,
                    ..Default::default()
                },
            )
            .map(|x| x.system)
            .unwrap()
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

    /// 玉の端に足すと、端の面の膜の上書きは新しい端の面へ移る（中に足したときは番号がずれるだけ）
    #[test]
    fn coating_moves_to_new_end() {
        let faces = |pos: usize, on: usize| {
            let mut p = lens();
            p.set_edit(on, "coating", json!("AR"));
            p.set_edit(on, "semi", json!(9.0));
            p.insert_face(pos).unwrap();
            let f = |k: &str| p.edits.iter().find(|e| e.field == k).unwrap().surface;
            (f("coating"), f("semi"))
        };
        // 前の端: 膜は新しい面 0 へ、半径の上書きは元の面（1）に
        assert_eq!(faces(0, 0), (0, 1));
        // 後ろの端: 膜は新しい面 2 へ
        assert_eq!(faces(2, 1), (2, 1));
        // 中: どちらも番号がずれるだけ
        assert_eq!(faces(1, 1), (2, 2));
        assert_eq!(faces(1, 0), (0, 0));
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

    /// 面の並びの行を足す・消す・種類を変えると、面と座標ブレークの番号が付け替わる（上書き・状態の上書き・
    /// 多重構成のピックアップ・操作の参照）。種類が変わった行の上書きは外す
    #[test]
    fn surface_rows_remap_faces_and_coordinate_breaks() {
        // 行: 面 0・座標ブレーク 0・面 1・座標ブレーク 1。行 1 の前に面を足す
        let old = [false, true, false, true];
        let new = [false, false, true, false, true];
        let m = RowRemap::of(&old, &new, &|r| Some(r + usize::from(r >= 1)));
        assert_eq!(m.face, [Some(0), Some(2)]);
        assert_eq!(m.cb, [Some(0), Some(1)]);
        // 行 1（座標ブレーク 0）を消す
        let m = RowRemap::of(&old, &[false, false, true], &|r| {
            (r != 1).then(|| r - usize::from(r > 1))
        });
        assert_eq!(m.face, [Some(0), Some(1)]);
        assert_eq!(m.cb, [None, Some(0)]);
        // 行 2（面 1）を座標ブレークに
        let m = RowRemap::of(&old, &[false, true, true, true], &Some);
        assert_eq!(m.face, [Some(0), None]);
        assert_eq!(m.cb, [Some(0), Some(2)]);
        assert!(!m.is_identity());
        assert!(RowRemap::of(&old, &old, &Some).is_identity());

        let mut p = Pick::new(ElemKind::Surfaces);
        p.id = "S".into();
        let cbf = |k: &str| format!("{}{k}", crate::combo_tree::CB_FIELD);
        p.edits = vec![
            SurfEdit {
                surface: 1,
                field: "t".into(),
                value: json!(3.0),
            },
            SurfEdit {
                surface: 1,
                field: cbf("ignore"),
                value: json!(true),
            },
        ];
        p.set_state_edit("A", 0, &cbf("ignore"), json!(true));
        p.set_state_edit("A", 1, "t", json!(4.0));
        p.mc_picks = vec![McPick {
            key: format!("face:1:{}", cbf("ignore")),
            to: "A".into(),
            from: "B".into(),
            row: String::new(),
            scale: 1.0,
            offset: 0.0,
        }];
        let op: SurfaceOp = serde_json::from_value(json!({ "op": "add_pickup",
            "dst": { "block": "S", "surface": 1, "coordbrk": true },
            "src": { "block": "S", "surface": 0, "coordbrk": true }, "what": "p1" }))
        .unwrap();
        p.ops = vec![op];
        let mut c = Combo::default();
        c.picks.push(p);
        // 行 1（座標ブレーク 0）を消す: 座標ブレーク 1 → 0、座標ブレーク 0 の上書き・ピックアップの元は外れる
        let m = RowRemap::of(&old, &[false, false, true], &|r| {
            (r != 1).then(|| r - usize::from(r > 1))
        });
        c.picks[0].shift_rows(&m);
        c.shift_row_refs("S", &m);
        let p = &c.picks[0];
        assert_eq!(p.base_edit(1, "t"), Some(&json!(3.0)));
        assert_eq!(p.base_edit(0, &cbf("ignore")), Some(&json!(true)));
        assert_eq!(p.state_edit("A", 0, &cbf("ignore")), None);
        assert_eq!(p.state_edit("A", 1, "t"), Some(&json!(4.0)));
        assert_eq!(p.mc_picks[0].key, format!("face:0:{}", cbf("ignore")));
        // 元の座標ブレークが無くなったピックアップの操作は外す
        assert!(p.ops.is_empty());
    }

    /// 要素の面を付け替えると、大域座標の基準の面（glrs）も付いていく。消した面なら既定に戻す
    #[test]
    fn glrs_follows_face_shifts() {
        let mut c = Combo::default();
        c.extra.glrs = Some(json!({ "block": "S", "surface": 1 }));
        c.shift_block_refs("T", &|x| Some(x + 1));
        assert_eq!(c.extra.glrs, Some(json!({ "block": "S", "surface": 1 })));
        c.shift_block_refs("S", &|x| Some(if x >= 1 { x + 1 } else { x }));
        assert_eq!(c.extra.glrs, Some(json!({ "block": "S", "surface": 2 })));
        // 座標ブレークの付け替えは面の基準を動かさない
        c.shift_block_cb_refs("S", &|_| None);
        assert_eq!(c.extra.glrs, Some(json!({ "block": "S", "surface": 2 })));
        c.extra.glrs = Some(json!({ "block": "S", "surface": 0, "coordbrk": true }));
        c.shift_block_cb_refs("S", &|x| Some(x + 1));
        assert_eq!(
            c.extra.glrs,
            Some(json!({ "block": "S", "surface": 1, "coordbrk": true }))
        );
        c.shift_block_refs("S", &|_| None);
        assert!(c.extra.glrs.is_some());
        c.shift_block_cb_refs("S", &|_| None);
        assert_eq!(c.extra.glrs, None);
    }
}
