//! 要求の印と method の対応の試験

use super::*;

/// 印の method は、serve（[`lensbench_engine::session::METHODS`]）か下書きの Worker が受けるものだけ
#[test]
fn every_tag_method_is_answered() {
    let draft = [
        "preview",
        "glasses",
        "lenses",
        "catalog_items",
        "catalog_edit",
        "catalog_fill",
        "folders",
        "cad_measure",
        "doc_peek",
    ];
    let tags = [
        Tag::Load,
        Tag::Info,
        Tag::Analyze(0),
        Tag::Rays(0),
        Tag::Glasses,
        Tag::Lenses,
        Tag::Get,
        Tag::Gets,
        Tag::Set,
        Tag::Write,
        Tag::Layout,
        Tag::Preview,
        Tag::LayoutExport,
        Tag::LensPreview(0),
        Tag::Focus,
        Tag::Surfaces,
        Tag::WriteBack,
        Tag::SaveRange,
        Tag::SurfData(0),
        Tag::Configs,
        Tag::UserCatalog(CatOp::Add),
        Tag::UserCatalog(CatOp::Items),
        Tag::UserCatalog(CatOp::Edit),
        Tag::CatalogFill,
        Tag::Optimize,
        Tag::Swap,
        Tag::Tolerance,
        Tag::Zoom,
        Tag::Folders,
        Tag::CadMeasure,
        Tag::OpenDoc,
        Tag::PopupDoc,
        Tag::Coatings,
        Tag::MechCheck,
    ];
    for t in &tags {
        let m = t.method().unwrap();
        assert!(
            lensbench_engine::session::METHODS.contains(&m) || draft.contains(&m),
            "{t:?}: {m}"
        );
    }
    assert_eq!(Tag::Refresh.method(), None);
}
