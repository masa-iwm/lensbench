//! ヘルプの窓（キーの一覧など。app.rs から分けた）

use super::*;

impl Core {
    pub(super) fn help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.help_open;
        // 画面の中央に、高さは画面の 8 割まで（上の帯に重ねず、下が切れない。中はスクロール）
        let screen = ctx.screen_rect();
        egui::Window::new(tr("Keyboard shortcuts"))
            .open(&mut open)
            .default_width(460.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .max_height(screen.height() * 0.8)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("keys").striped(true).show(ui, |ui| {
                    for (k, v) in [
                        ("Ctrl+K", tr("Command palette (find actions, analyses, panels and workspaces by name)")),
                        ("Ctrl+N", tr("New (clear the combination)")),
                        ("Ctrl+O", tr("Open a system file")),
                        ("F5", tr("Recompute all open analyses")),
                        ("Ctrl+R", tr("Reload the system (discard changed values)")),
                        ("Ctrl+Z", tr("Undo a system change (up to 100 steps; inside a text box, its text)")),
                        ("Ctrl+Y / Ctrl+Shift+Z", tr("Redo the undone change")),
                        ("Ctrl+Tab", tr("Next panel (Shift for previous)")),
                        ("Tab / Shift+Tab", tr("Move between fields and buttons in a panel")),
                        ("Enter / Space", tr("Press a button / select")),
                        ("↑ ↓", tr("Step a number field (parameters and analysis settings recompute right away)")),
                        ("Ctrl+A", tr("Next multi-configuration (Shift: previous). In the Lens Data table, selects all rows")),
                        ("Enter", tr("Analysis settings: recompute with the typed value")),
                        (tr("Letters"), tr("Analysis list: type to filter (Enter picks the first match)")),
                        ("F1", tr("This list")),
                        ("Esc", tr("Close the palette or a window")),
                    ] {
                        ui.label(RichText::new(k).monospace().strong());
                        ui.label(v);
                        ui.end_row();
                    }
                });
                for (title, keys) in [
                    (
                        tr("Lens Data"),
                        vec![
                            ("← → ↑ ↓ / Tab", tr("Move between cells (Shift+↑ ↓ / Shift+click select a row range)")),
                            ("Enter / F2", tr("Edit (typing also starts editing). Enter commits, Esc cancels")),
                            ("Insert", tr("Insert a surface before the current one")),
                            ("Alt+Insert", tr("Insert a surface after the current one")),
                            ("Delete", tr("Delete the selected surfaces")),
                            ("Ctrl+C", tr("Copy selected rows as tab-separated text")),
                            ("Ctrl+V", tr("Paste over cells from the current one")),
                            ("Ctrl+Shift+V", tr("Insert as many surfaces as copied rows and paste")),
                            ("Shift+F10", tr("Right-click menu")),
                            ("Ctrl+L", tr("Lock a block")),
                            ("Enter / Backspace", tr("Enter / leave a block")),
                        ],
                    ),
                    (
                        tr("Multi-Config"),
                        vec![
                            ("← → ↑ ↓ / Tab", tr("Move between cells")),
                            ("Enter / F2", tr("Edit (typing also starts editing)")),
                            ("Delete", tr("Delete the selected row (or the configuration if a column header is selected)")),
                            ("Ctrl+C / Ctrl+V", tr("Copy rows / paste over")),
                            ("Shift+F10", tr("Right-click menu")),
                        ],
                    ),
                    (
                        tr("Layout figure (combination)"),
                        vec![
                            ("[ ]", tr("Select the previous / next element")),
                            ("← →", tr("Move along the axis (the selected gap if a dimension is selected). Ctrl ×10, Alt ×1/10")),
                            ("↑ ↓", tr("Decenter (Shift: tilt)")),
                            ("R", tr("Reverse")),
                            ("F", tr("Flip the splitting surface of the BS")),
                            ("Ctrl+L", tr("Lock")),
                            ("Delete", tr("Remove (Ctrl+Z to undo)")),
                            ("Esc", tr("Deselect")),
                        ],
                    ),
                ] {
                    ui.separator();
                    ui.strong(title);
                    egui::Grid::new(("keys", title)).striped(true).show(ui, |ui| {
                        for (k, v) in keys {
                            ui.label(RichText::new(k).monospace().strong());
                            ui.label(v);
                            ui.end_row();
                        }
                    });
                }
                ui.separator();
                ui.label(
                    RichText::new(tr("Drag a tab to reorder, split or float panels. Right-click a tab to move it to a separate window (another monitor).")).small(),
                );
                });
            });
        self.help_open = open;
    }
}
