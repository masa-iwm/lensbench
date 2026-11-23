//! レイアウト図に重ねる構成（[`Overlay`]）の試験（app.rs から分けた）

use super::*;

/// 重ねる構成: configs の結果から欄を作り、選んだものだけ layout の要求に。読み直しても選んだまま
#[test]
fn overlay_params() {
    let mut o = Overlay::default();
    o.update(&json!({ "cfgs": 1, "states": ["A"], "paths": ["TT", "RR"] }));
    assert_eq!(o.items.len(), 2);
    assert_eq!(o.req(), LayoutReq::default());
    o.items[1].2 = true;
    o.offset = [30.0, 0.0];
    assert_eq!(
        o.req(),
        LayoutReq {
            beam_paths: Some("RR".into()),
            offset: vec![0.0, 0.0, 30.0],
            ..Default::default()
        }
    );
    o.update(&json!({ "cfgs": 1, "paths": ["RR", "TT", "TR"] }));
    assert_eq!(o.items.iter().filter(|x| x.2).count(), 1);
    assert_eq!(o.items[0].1, "RR");
    // 光束は 1 本でも選べる
    o.update(&json!({ "cfgs": 1, "beams": ["ref"] }));
    assert_eq!(o.items.len(), 1);
    o.items[0].2 = true;
    assert_eq!(o.req().beams.as_deref(), Some("ref"));
    o.update(&json!({ "cfgs": 3 }));
    o.items[0].2 = true;
    o.items[2].2 = true;
    o.offset = [0.0; 2];
    let cfgs = LayoutReq {
        cfgs: Some("1,3".into()),
        ..Default::default()
    };
    assert_eq!(o.req(), cfgs);
    // xz 面: plane を足し、ずらす量の縦は x
    o.offset = [10.0, 5.0];
    use lensbench_engine::layout::View;
    let r = o.req_in(ViewPlane::Xz);
    assert_eq!(
        (r.offset.as_slice(), r.plane),
        (&[5.0, 0.0, 10.0][..], View::XZ)
    );
    assert_eq!(o.req_in(ViewPlane::Yz), o.req());
    // 展開: yz のまま unfold、3D は plane
    let r = o.req_in(ViewPlane::Unfold);
    assert_eq!(
        (r.offset.as_slice(), r.plane, r.unfold),
        (&[0.0, 5.0, 10.0][..], View::YZ, true)
    );
    assert_eq!(o.req_in(ViewPlane::D3).plane, View::D3);
    // 設定に保存する名前
    assert_eq!(serde_json::to_value(ViewPlane::D3).unwrap(), json!("3d"));
    assert_eq!(
        serde_json::to_value(ViewPlane::Unfold).unwrap(),
        json!("unfold")
    );
}
