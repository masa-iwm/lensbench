//! 読み上げ（アクセシビリティ）の木の手直し。部品の外で作られ、こちらで名前を付けられない節に名前を足す

use egui::accesskit::Role;

use crate::i18n::tr;

/// egui_dock の仕切り（パネルの境目のドラッグできる線）は名前を持たない。読み上げでも、試験の
/// 「名前の無い入力欄」の検査でも引っかかるので、出力の木で名前の無い仕切りに名前を付ける
struct SplitterNames;

impl egui::Plugin for SplitterNames {
    fn debug_name(&self) -> &'static str {
        "lensbench-splitter-names"
    }

    fn output_hook(&mut self, _ctx: &egui::Context, output: &mut egui::FullOutput) {
        let Some(tree) = output.platform_output.accesskit_update.as_mut() else {
            return;
        };
        for (_, node) in &mut tree.nodes {
            if node.role() == Role::Splitter && node.label().is_none() {
                node.set_label(tr("Resize panels"));
            }
        }
    }
}

/// App の初めに一度呼ぶ（同じ型の plugin は二度目以降は足されない）
pub(crate) fn install(ctx: &egui::Context) {
    ctx.add_plugin(SplitterNames);
}
