//! 元に戻す段の名前（serve の undo_list・redo_list の 1 つを GUI の言葉に。app.rs から分けた）
use serde_json::Value;

use crate::i18n::{tr, trf};

/// serve の段の変更の名前（set はパラメータの説明、edit は操作）
pub fn hist_label(v: &Value) -> String {
    if let Some(f) = v["open"].as_str() {
        return trf("Open {}", &[&f]);
    }
    if let Some(f) = v["reload"].as_str() {
        return trf("Reload {}", &[&f]);
    }
    if let Some(names) = v["set"].as_array() {
        let d: Vec<String> = names
            .iter()
            .filter_map(Value::as_str)
            .map(|n| {
                let d = crate::param_pick::param_desc(n);
                if d.is_empty() {
                    n.to_string()
                } else {
                    d
                }
            })
            .collect();
        return match d.len() {
            0 => tr("Value change").into(),
            1..=2 => d.join(crate::i18n::tr(", ")),
            n => trf("{} and {} more", &[&d[0], &(n - 1)]),
        };
    }
    let ops = v["edit"].as_array().cloned().unwrap_or_default();
    let one = |o: &Value| -> String {
        let s = o["surface"].as_u64().map(|x| x.to_string());
        let op = o["op"].as_str().unwrap_or_default();
        let what = match op {
            "insert_surface" => tr("Insert surface"),
            "delete_surface" => tr("Delete surface"),
            "set_surface" | "set_surface_data" => tr("Surface values"),
            "set_surface_type" => tr("Surface type"),
            "set_material" => tr("Material"),
            "set_stop" => tr("Aperture stop"),
            "add_pickup" | "remove_pickup" => tr("Pickups"),
            "add_solve" | "remove_solve" => tr("Solves"),
            "set_system_data" => tr("System settings"),
            "set_param" => tr("Value change"),
            "define_unit" | "set_unit" | "remove_unit" => tr("Lenses / groups"),
            "add_mech" | "set_mech" | "remove_mech" => tr("Mechanical parts"),
            "set_config" => tr("Switch configuration"),
            o if o.contains("config") => tr("Multi-Config"),
            _ => tr("Edit"),
        };
        // 面の値の 1 項目なら、その項目の名前（面 8 の厚さ など）
        if let (Some(s), Some([f])) = (&s, o["fields"].as_array().map(Vec::as_slice)) {
            let short = match f.as_str().unwrap_or_default() {
                "thickness" | "t" => "t",
                "radius" | "r" => "r",
                "curv" | "c" => "c",
                "conic" | "k" => "k",
                _ => "",
            };
            if !short.is_empty() {
                return crate::param_pick::param_desc(&format!("{s}.{short}"));
            }
        }
        match s {
            Some(s) => trf("{} (surface {})", &[&what, &s]),
            None => what.to_string(),
        }
    };
    match ops.len() {
        0 => tr("Edit").into(),
        1 => one(&ops[0]),
        n => trf("{} and {} more", &[&one(&ops[0]), &(n - 1)]),
    }
}
