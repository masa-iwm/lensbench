//! serve の method の名前・書式の版と、method ごとの鍵の取り方

/// serve の method の名前（`describe` が返す。`handle` の match と試験で突き合わせる、CLI-22）
pub const METHODS: &[&str] = &[
    "ping",
    "progress",
    "cancel",
    "stop",
    "describe",
    "shutdown",
    "echo",
    "load",
    "reload",
    "system",
    "undo",
    "redo",
    "history",
    "get",
    "set",
    "write",
    "edit",
    "surface",
    "surface_data",
    "system_data",
    "config_table",
    "config_diff",
    "save",
    "save_range",
    "analyze",
    "glasses",
    "lenses",
    "catalog_search",
    "trace",
    "plugins",
    "catalog_add",
    "catalog_items",
    "catalog_edit",
    "catalog_fill",
    "cad_step",
    "analysis_args",
    "plugin",
    "layout",
    "layout_export",
    "configs",
    "optimize",
    "swap",
    "tolerance",
    "zoom",
];

/// 書式の版（枠組み・method の約束を壊す変更で上げる）
pub const PROTOCOL: u32 = 1;

/// 要求の鍵の取り方
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Access {
    /// 系を読むだけ（並行に回す）
    Read,
    /// 系や場の外のファイルを変える（1 つずつ）
    Write,
    /// 長い計算（始めたときの系で回し、他の要求を止めない。長い要求どうしは 1 つずつ）
    Long,
}

impl Access {
    /// method の鍵の取り方（知らない method は Write）
    pub fn of(method: &str) -> Access {
        match method {
            "ping" | "describe" | "echo" | "system" | "history" | "get" | "surface"
            | "surface_data" | "system_data" | "config_table" | "configs" | "analyze" | "trace"
            | "plugin" | "plugins" | "analysis_args" | "layout" | "glasses" | "lenses"
            | "catalog_search" => Access::Read,
            "optimize" | "tolerance" | "swap" | "zoom" => Access::Long,
            _ => Access::Write,
        }
    }
}
