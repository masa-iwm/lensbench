//! 同じ処方の玉を裏返して置いた .zmx を、同じ玉の 2 つの置き方（後の玉は前の玉の処方を `reversed`）として分ける

use serde_json::Value;

/// 玉のブロックの面（source.surfaces）を裏返した面: 面の順を逆に、半径の符号を逆に、厚さ・ガラスは間の順を逆に
/// （最後の面は厚さ・ガラスを持たない）。行で書く面・非球面など、裏返しを欄で書けない面があれば None
fn reversed_faces(ss: &[Value]) -> Option<Vec<Value>> {
    let n = ss.len();
    if n < 2 {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut f = ss[n - 1 - i].as_object()?.clone();
        // 裏返すと符号の変わる欄（非球面の係数・行・傾きなど）がある面は確かめない
        let flat = |v: &Value| {
            v.as_array()
                .is_some_and(|a| a.iter().all(|x| x.as_f64() == Some(0.0)))
        };
        if f.iter().any(|(k, v)| match k.as_str() {
            "radius" | "t" | "glass" | "semi" | "semi_auto" | "conic" | "coat" | "comment" => false,
            "asph" => !flat(v),
            _ => true,
        }) {
            return None;
        }
        f.remove("t");
        f.remove("glass");
        if let Some(r) = f.get("radius") {
            f.insert("radius".into(), serde_json::json!(-r.as_f64()?));
        }
        if i + 1 < n {
            let g = ss[n - 2 - i].as_object()?;
            for k in ["t", "glass"] {
                if let Some(v) = g.get(k) {
                    f.insert(k.into(), v.clone());
                }
            }
        }
        out.push(Value::Object(f));
    }
    Some(out)
}

/// blocks のうち、前の玉を裏返した処方の玉（面の欄が前の玉を裏返したものとそろい、面の操作・上書き・置き方の無いもの）を、
/// 前の玉の処方で `reversed` にしたブロックに替える。替えたものがあれば true。値まで同じ系になるかは呼び手が確かめる
pub(super) fn merge_reversed(blocks: &mut [Value]) -> bool {
    let mut changed = false;
    for b in 0..blocks.len() {
        let plain = blocks[b]["type"] == "lens"
            && blocks[b].as_object().is_some_and(|o| {
                o.keys()
                    .all(|k| matches!(k.as_str(), "type" | "id" | "source"))
            });
        let Some(ss) = blocks[b]["source"]["surfaces"].as_array().filter(|_| plain) else {
            continue;
        };
        let ss = ss.clone();
        let src = (0..b).find_map(|a| {
            let x = &blocks[a];
            if x["type"] != "lens" || x["reversed"] == true {
                return None;
            }
            let fs = x["source"]["surfaces"].as_array()?;
            (reversed_faces(fs)? == ss && fs != &ss).then(|| x["source"].clone())
        });
        if let Some(src) = src {
            blocks[b]["source"] = src;
            blocks[b]["reversed"] = Value::Bool(true);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 裏返した面: 面の順と半径の符号が逆、厚さ・ガラスは間の順が逆
    #[test]
    fn faces_reverse_with_gaps_in_between() {
        let a = vec![
            json!({ "radius": 50.0, "t": 4.0, "glass": "N-BK7", "semi": 8.0 }),
            json!({ "radius": -30.0, "t": 2.0, "glass": "F2", "semi": 8.0 }),
            json!({ "radius": -100.0, "semi": 7.0 }),
        ];
        let r = reversed_faces(&a).unwrap();
        assert_eq!(
            r,
            vec![
                json!({ "radius": 100.0, "t": 2.0, "glass": "F2", "semi": 7.0 }),
                json!({ "radius": 30.0, "t": 4.0, "glass": "N-BK7", "semi": 8.0 }),
                json!({ "radius": -50.0, "semi": 8.0 }),
            ]
        );
        assert_eq!(reversed_faces(&r).unwrap(), a);
        // 非球面の係数は裏返すと符号が変わるので確かめない
        let mut b = a.clone();
        b[0]["asph"] = json!([0.0, 1e-5]);
        assert!(reversed_faces(&b).is_none());
    }

    /// 後の玉が前の玉を裏返した処方なら前の玉の処方の reversed に、対称な玉（裏返しても同じ）は替えない
    #[test]
    fn a_later_reversed_lens_takes_the_earlier_prescription() {
        let l1 = json!([{ "radius": 50.0, "t": 4.0, "glass": "N-BK7" }, { "radius": -100.0 }]);
        let l2 = json!([{ "radius": 100.0, "t": 4.0, "glass": "N-BK7" }, { "radius": -50.0 }]);
        let sym = json!([{ "radius": 40.0, "t": 3.0, "glass": "F2" }, { "radius": -40.0 }]);
        let mut blocks = vec![
            json!({ "type": "lens", "id": "L1", "source": { "surfaces": l1 } }),
            json!({ "type": "air", "t": 10.0 }),
            json!({ "type": "lens", "id": "L2", "source": { "surfaces": l2 } }),
            json!({ "type": "lens", "id": "L3", "source": { "surfaces": sym } }),
            json!({ "type": "lens", "id": "L4", "source": { "surfaces": sym } }),
        ];
        assert!(merge_reversed(&mut blocks));
        assert_eq!(blocks[2]["source"]["surfaces"], l1);
        assert_eq!(blocks[2]["reversed"], true);
        assert!(blocks[3].get("reversed").is_none());
        assert!(blocks[4].get("reversed").is_none());
        // 面の操作のある玉は替えない
        let mut b2 = vec![
            json!({ "type": "lens", "id": "L1", "source": { "surfaces": l1 } }),
            json!({ "type": "lens", "id": "L2", "source": { "surfaces": l2 }, "surface_ops": [] }),
        ];
        assert!(!merge_reversed(&mut b2));
    }

    /// 同じ処方の玉を裏返して置いた .zmx は、後の玉が前の玉の処方の裏返し（Reverse）の要素に分かれる
    #[test]
    fn a_zmx_with_a_flipped_copy_opens_as_the_same_lens_reversed() {
        use crate::combo::{Combo, ElemKind};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tmp = std::env::temp_dir().join(format!("lb_gui_rev_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("relay.zmx");
        std::fs::write(
            &f,
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 5\n  DIAM 5 0 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 20\n  DIAM 7 0 0 0 1 \"\"\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.01\n  DISZ 4\n  GLAS N-BK7 0 0 1.5168 64.17 0 0 0 0 0\n  DIAM 7 0 0 0 1 \"\"\n\
SURF 5\n  TYPE STANDARD\n  CURV -0.02\n  DISZ 40\n  DIAM 8 0 0 0 1 \"\"\n\
SURF 6\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        )
        .unwrap();
        let r = Combo::from_zmx(&f, &[root.join("tests/glass")], &[]);
        std::fs::remove_dir_all(&tmp).ok();
        let c = r.unwrap();
        let lenses: Vec<_> = c
            .picks
            .iter()
            .filter(|p| p.kind == ElemKind::Lens)
            .collect();
        assert_eq!(
            lenses.len(),
            2,
            "{:?}",
            c.picks.iter().map(|p| p.kind).collect::<Vec<_>>()
        );
        assert!(!lenses[0].reversed);
        assert!(lenses[1].reversed);
        assert_eq!(lenses[0].surfs, lenses[1].surfs);
        assert_eq!(
            (lenses[0].data.is_some(), lenses[0].semi),
            (lenses[1].data.is_some(), lenses[1].semi)
        );
    }
}
