//! 欄の値の書き換え（[`Edit`]）: 文書（組合せ）を書き換えるウィジェットは組合せの下書きを変え、描いた後に
//! 変わった所だけを値として [`Msg::Edit`] に出す（[`stage`]）。当てるのは [`App::update`]（[`Edit::apply`]）。
//! 書き換えは組合せの保存の形（`gui.combo` と同じ JSON）の上の「場所と新しい値」の並びなので、記録して後から
//! 同じ文書に流し直せる

use super::*;

/// 組合せの保存の形の中の場所の 1 段: 鍵か並びの番号
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Seg {
    Key(String),
    Index(usize),
}

/// 1 か所の書き換え: 場所と新しい値（None は鍵を消す）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Op {
    pub path: Vec<Seg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// 組合せの欄の値の書き換え（変わった所だけ。いちばん深い所で値を替え、並びの長さが変わればその並びごと替える）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Edit {
    pub ops: Vec<Op>,
}

impl Edit {
    /// a から b への書き換え（同じなら None）
    pub(crate) fn between(a: &Combo, b: &Combo) -> Option<Edit> {
        if a == b {
            return None;
        }
        let (va, vb) = (to_value(a), to_value(b));
        let mut ops = Vec::new();
        diff(&va, &vb, &mut Vec::new(), &mut ops);
        (!ops.is_empty()).then_some(Edit { ops })
    }

    /// 組合せに当てる（場所が無い・形が合わないときは何も変えずに誤りを返す）
    pub(crate) fn apply(&self, c: &mut Combo) -> Result<(), String> {
        let mut v = to_value(c);
        for op in &self.ops {
            put(&mut v, &op.path, op.value.clone())?;
        }
        *c = serde_json::from_value(v).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// 組合せの保存の形（`gui.combo` と同じ）
fn to_value(c: &Combo) -> Value {
    serde_json::to_value(c).unwrap_or(Value::Null)
}

fn diff(a: &Value, b: &Value, path: &mut Vec<Seg>, ops: &mut Vec<Op>) {
    if a == b {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, va) in x {
                path.push(Seg::Key(k.clone()));
                match y.get(k) {
                    Some(vb) => diff(va, vb, path, ops),
                    None => ops.push(Op {
                        path: path.clone(),
                        value: None,
                    }),
                }
                path.pop();
            }
            for (k, vb) in y.iter().filter(|(k, _)| !x.contains_key(*k)) {
                path.push(Seg::Key(k.clone()));
                ops.push(Op {
                    path: path.clone(),
                    value: Some(vb.clone()),
                });
                path.pop();
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (va, vb)) in x.iter().zip(y).enumerate() {
                path.push(Seg::Index(i));
                diff(va, vb, path, ops);
                path.pop();
            }
        }
        _ => ops.push(Op {
            path: path.clone(),
            value: Some(b.clone()),
        }),
    }
}

/// path の所を value にする（None は鍵を消す）。途中の場所が無ければ誤り
fn put(root: &mut Value, path: &[Seg], value: Option<Value>) -> Result<(), String> {
    let Some((last, head)) = path.split_last() else {
        return match value {
            Some(v) => {
                *root = v;
                Ok(())
            }
            None => Err("cannot remove the whole document".into()),
        };
    };
    let mut cur = root;
    for s in head {
        cur = match (s, cur) {
            (Seg::Key(k), Value::Object(m)) => m.get_mut(k),
            (Seg::Index(i), Value::Array(v)) => v.get_mut(*i),
            _ => None,
        }
        .ok_or_else(|| format!("no place {}", show(path)))?;
    }
    match (last, cur, value) {
        (Seg::Key(k), Value::Object(m), Some(v)) => {
            m.insert(k.clone(), v);
        }
        (Seg::Key(k), Value::Object(m), None) => {
            m.remove(k);
        }
        (Seg::Index(i), Value::Array(a), Some(v)) if *i < a.len() => a[*i] = v,
        _ => return Err(format!("no place {}", show(path))),
    }
    Ok(())
}

fn show(path: &[Seg]) -> String {
    path.iter()
        .map(|s| match s {
            Seg::Key(k) => format!("/{k}"),
            Seg::Index(i) => format!("/{i}"),
        })
        .collect()
}

/// 組合せを書き換えるウィジェット f を描く: f が組合せを変えたら、変えた所を [`Msg::Edit`] として msgs に足し、
/// 組合せは描く前に戻す（当てるのは [`App::update`]）
pub(super) fn stage<R>(
    combo: &mut Combo,
    msgs: &mut Vec<Msg>,
    f: impl FnOnce(&mut Combo) -> R,
) -> R {
    let before = combo.clone();
    let r = f(combo);
    unstage(before, combo, msgs);
    r
}

/// [`stage`] の後半: before から変わっていれば、変えた所を [`Msg::Edit`] として msgs に足し、組合せを before に戻す
/// （組合せの外の状態も一緒に渡すウィジェットのため）
pub(super) fn unstage(before: Combo, combo: &mut Combo, msgs: &mut Vec<Msg>) {
    if let Some(e) = Edit::between(&before, combo) {
        msgs.push(Msg::Edit(e));
        *combo = before;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Combo {
        let mut c = Combo::default();
        c.insert_plane(0);
        c.insert_plane(1);
        c
    }

    /// 書き換えは変わった所だけで、当てると同じ組合せになる
    #[test]
    fn edit_carries_only_the_change_and_reapplies() {
        let a = sample();
        let mut b = a.clone();
        b.epd = 7.5;
        b.fields[1].y = 3.0;
        let e = Edit::between(&a, &b).expect("changed");
        assert_eq!(e.ops.len(), 2, "{e:?}");
        assert!(e
            .ops
            .iter()
            .all(|o| o.value.as_ref().is_some_and(Value::is_number)));
        let mut c = a.clone();
        e.apply(&mut c).unwrap();
        assert_eq!(c, b);
        assert!(Edit::between(&a, &a).is_none());
    }

    /// 並びの長さ・鍵の有無が変わる書き換え（要素を足す・消す、書かない鍵）も当てられ、JSON を通しても同じ
    #[test]
    fn edit_resizes_lists_and_round_trips_through_json() {
        let a = sample();
        let mut b = a.clone();
        b.picks.remove(0);
        b.waves.push(crate::sysset::WavePt::um(0.7));
        b.lead_air = Some(1.25);
        b.floating_clip = false;
        let e = Edit::between(&a, &b).expect("changed");
        let e: Edit = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        let mut c = a.clone();
        e.apply(&mut c).unwrap();
        assert_eq!(c, b);
        // 戻す向きも
        let back = Edit::between(&b, &a).unwrap();
        back.apply(&mut c).unwrap();
        assert_eq!(c, a);
    }

    /// 場所の無い書き換えは誤りで、組合せは変えない
    #[test]
    fn edit_to_a_missing_place_is_refused() {
        let a = sample();
        let e = Edit {
            ops: vec![Op {
                path: vec![
                    Seg::Key("picks".into()),
                    Seg::Index(9),
                    Seg::Key("t".into()),
                ],
                value: Some(json!(1.0)),
            }],
        };
        let mut c = a.clone();
        assert!(e.apply(&mut c).is_err());
        assert_eq!(c, a);
    }

    /// stage: 変えたウィジェットは書き換えを出し、組合せは描く前のまま
    #[test]
    fn stage_turns_widget_changes_into_messages() {
        let mut c = sample();
        let before = c.clone();
        let mut msgs = Vec::new();
        let r = stage(&mut c, &mut msgs, |c| {
            c.stop_gap = 4.0;
            7
        });
        assert_eq!(r, 7);
        assert_eq!(c, before);
        let [Msg::Edit(e)] = msgs.as_slice() else {
            panic!("{msgs:?}");
        };
        e.apply(&mut c).unwrap();
        assert_eq!(c.stop_gap, 4.0);
        // 変えなければ何も出さない
        let mut msgs = Vec::new();
        stage(&mut c, &mut msgs, |_| ());
        assert!(msgs.is_empty());
    }
}
