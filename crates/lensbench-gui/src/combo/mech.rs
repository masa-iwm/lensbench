//! 組合せに置く機構部品（[`MechPick`]）

use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use lensbench_project::{MechOutline, UserItem};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 組合せに置く機構部品: カタログの品を、要素（ブロック id）の面に置く
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct MechPick {
    pub name: String,
    /// ユーザーカタログのファイル（絶対パス）と品の名前
    pub catalog: String,
    pub item: String,
    /// 置く要素のブロック id（空なら系の面 1）と、その要素の面（0 始まり）
    pub at: String,
    pub face: usize,
    /// 置いた面の局所座標でのずれ [mm] と傾き [deg]
    pub offset: [f64; 3],
    pub tilt: [f64; 2],
    /// 保持する要素のブロック id
    pub holds: Vec<String>,
    /// 部品の座を保持する要素の前面に合わせて置く（置く要素・面の代わり）
    pub seat: bool,
    /// 前の機構部品（名前）の後ろの端につなぐ（置く要素・面・座の代わり。z はそこからのずれ）
    pub after: String,
    /// 裏返して置く（光が部品の z の大きい端から入る）
    pub reversed: bool,
}

impl MechPick {
    /// プロジェクトの mechanics の項目
    pub(crate) fn json(&self) -> Value {
        let at = if self.at.is_empty() {
            json!({ "surface": 1 })
        } else {
            json!({ "block": self.at, "surface": self.face })
        };
        if !self.after.is_empty() {
            let mut v = json!({ "name": self.name, "catalog": self.catalog, "item": self.item,
                "after": self.after, "offset": self.offset });
            if !self.holds.is_empty() {
                v["holds"] = json!(self.holds);
            }
            if self.reversed {
                v["reversed"] = json!(true);
            }
            return v;
        }
        let mut v = json!({ "name": self.name, "catalog": self.catalog, "item": self.item, "at": at,
            "offset": self.offset, "tilt": self.tilt });
        if !self.holds.is_empty() {
            v["holds"] = json!(self.holds);
            if self.seat {
                v["seat"] = json!(true);
            }
        }
        if self.reversed {
            v["reversed"] = json!(true);
        }
        v
    }
}

impl MechPick {
    /// 部品の品をカタログ（`catalog` のファイル。更新時刻で覚えて読み直さない）から引く。Err は見つからない理由
    pub(crate) fn lookup(&self) -> Result<UserItem, String> {
        let c = lensbench_project::UserCatalog::load_shared(std::path::Path::new(&self.catalog))
            .map_err(|e| trf("Cannot read the catalog {} ({})", &[&self.catalog, &e]))?;
        c.items
            .get(&self.item)
            .cloned()
            .ok_or_else(|| trf("Item {} is not in {}", &[&self.item, &self.catalog]))
    }

    /// 置いた所の 1 行（要素・面・z、つなぎ・座・反転）
    pub(crate) fn placement(&self) -> String {
        let z = fmt_num(self.offset[2]);
        let mut t = if !self.after.is_empty() {
            trf("Connected after {}, z {} mm", &[&self.after, &z])
        } else if self.seat && !self.holds.is_empty() {
            trf("Seated on {}, z {} mm", &[&self.holds[0], &z])
        } else if self.at.is_empty() {
            trf("At surface 1, z {} mm", &[&z])
        } else {
            trf("At {} surface {}, z {} mm", &[&self.at, &self.face, &z])
        };
        if self.reversed {
            t.push_str(tr(", reversed"));
        }
        t
    }

    /// レイアウト図で選んだ部品の説明の行（名前と品番、種類と寸法、品の説明、穴・保持、置いた所、保持する要素）
    pub(crate) fn card(&self) -> Vec<String> {
        let mut v = vec![format!("{}  {}", self.name, self.item)];
        match self.lookup() {
            Ok(it) => {
                v.push(mech_info(&it));
                if !it.description.is_empty() {
                    v.push(it.description.clone());
                }
                if let Some(m) = it.mech() {
                    let bores: Vec<String> = m
                        .bores
                        .iter()
                        .map(|b| {
                            trf(
                                "⌀{} mm (z {}–{} mm)",
                                &[&fmt_num(b.diameter), &fmt_num(b.z0), &fmt_num(b.z1)],
                            )
                        })
                        .collect();
                    if !bores.is_empty() {
                        v.push(trf("Bores {}", &[&bores.join(", ")]));
                    }
                    if let Some(h) = &m.holds {
                        v.push(trf(
                            "Holding diameter {} mm, seat at z {} mm",
                            &[&fmt_num(h.diameter), &fmt_num(h.seat_z)],
                        ));
                    }
                }
            }
            Err(e) => v.push(format!("⚠ {e}")),
        }
        v.push(self.placement());
        if !self.holds.is_empty() {
            v.push(trf("Holds {}", &[&self.holds.join(", ")]));
        }
        v
    }
}

/// レイアウト図と組合せの機構部品の欄で選んだ部品の名前（egui のメモリ。どちらで選んでも両方で目立つ）
pub(crate) fn sel_id() -> egui::Id {
    egui::Id::unique("lensbench-mech-sel")
}

/// 図で選んだので、欄のその部品の行まで送る（欄が受け取ったら消す）
pub(crate) fn scroll_id() -> egui::Id {
    egui::Id::unique("lensbench-mech-sel-scroll")
}

/// 選んでいる部品の名前
pub(crate) fn selected(ctx: &egui::Context) -> Option<String> {
    ctx.data(|d| d.get_temp::<String>(sel_id()))
}

/// 部品を選ぶ（None で外す）
pub(crate) fn select(ctx: &egui::Context, name: Option<&str>) {
    ctx.data_mut(|d| match name {
        Some(n) => {
            d.insert_temp(sel_id(), n.to_string());
        }
        None => {
            d.remove::<String>(sel_id());
        }
    });
}

/// 品の種類と主な寸法の 1 行（例「Tube: length 30 mm, bore 23 mm, outer diameter 30.5 mm, thread M25」）
pub(crate) fn mech_info(it: &UserItem) -> String {
    let kind = it
        .kind
        .and_then(|k| {
            crate::catalog::CATEGORIES
                .iter()
                .find(|(c, _)| *c == k.as_str())
                .map(|(_, n)| tr(n))
        })
        .unwrap_or_else(|| tr("Mechanical part"));
    let Some(m) = it.mech() else {
        return kind.to_string();
    };
    let mut t = trf("{}: length {} mm", &[&kind, &fmt_num(m.length)]);
    let bore = m
        .bores
        .iter()
        .map(|b| b.diameter)
        .fold(f64::INFINITY, f64::min);
    if bore.is_finite() {
        t.push_str(&trf(", bore {} mm", &[&fmt_num(bore)]));
    }
    if m.outer_diameter > 0.0 {
        t.push_str(&if m.outline == MechOutline::Square {
            trf(", square side {} mm", &[&fmt_num(m.outer_diameter)])
        } else {
            trf(", outer diameter {} mm", &[&fmt_num(m.outer_diameter)])
        });
    }
    if m.rod_spacing > 0.0 {
        t.push_str(&trf(", cage rods {} mm apart", &[&fmt_num(m.rod_spacing)]));
    }
    if !m.thread.is_empty() {
        t.push_str(&trf(", thread {}", &[&m.thread]));
    }
    t
}

/// 選んだ部品の札に出す mech-check の結果（app が毎フレーム置く。値が Null なら計算中）
fn check_id() -> egui::Id {
    egui::Id::unique("lensbench-mech-check")
}

/// 札に出す mech-check の結果を置く（None は計算中）
pub(crate) fn put_check(ctx: &egui::Context, v: Option<std::sync::Arc<Value>>) {
    let v = v.unwrap_or_else(|| std::sync::Arc::new(Value::Null));
    ctx.data_mut(|d| d.insert_temp(check_id(), v));
}

/// 札に出す mech-check の結果（まだ置かれていなければ None）
pub(crate) fn check(ctx: &egui::Context) -> Option<std::sync::Arc<Value>> {
    ctx.data(|d| d.get_temp(check_id()))
}

/// 札の行の数の上限（視野×波長が多いとき）
const CHECK_ROWS: usize = 12;

/// mech-check の結果 v のうち、部品 name の分の札の行: 視野・波長ごとの通る割合とこの部品が最初に切る割合
/// （解析「機構のケラレ」の表の行と列）、収まりの注意。v が Null なら計算中
pub(crate) fn check_lines(name: &str, v: &Value) -> Vec<String> {
    if v.is_null() {
        return vec![tr("Mechanical vignetting: computing…").to_string()];
    }
    if let Some(e) = v["error"].as_str() {
        return vec![trf("Mechanical vignetting: {}", &[&e])];
    }
    let Some(k) = v["parts"]
        .as_array()
        .and_then(|a| a.iter().position(|p| p["name"] == name))
    else {
        return Vec::new();
    };
    let pct = |x: &Value| format!("{:.1}", x.as_f64().unwrap_or(f64::NAN));
    let rows = v["results"].as_array().cloned().unwrap_or_default();
    let mut out = vec![tr("Field, wave: passes / this part first (all) [%]").to_string()];
    for r in rows.iter().take(CHECK_ROWS) {
        let q = &r["parts"][k];
        let cut = r["passed_pct"].as_f64().is_some_and(|x| x < 100.0 - 1e-9);
        out.push(format!(
            "{}{}, {}: {} / {} ({})",
            if cut { "⚠ " } else { "" },
            r["field"],
            r["wave"],
            pct(&r["passed_pct"]),
            pct(&q["first_pct"]),
            pct(&q["clipped_pct"]),
        ));
    }
    if rows.len() > CHECK_ROWS {
        out.push(trf(
            "… {} more rows in the \"Mechanical vignetting\" analysis",
            &[&(rows.len() - CHECK_ROWS)],
        ));
    }
    for f in v["fit"].as_array().into_iter().flatten() {
        if f["part"] == name || f["other"] == name {
            let m = f["message"].as_str().unwrap_or_default();
            out.push(if f["error"] == true {
                format!("⚠ {m}")
            } else {
                m.to_string()
            });
        }
    }
    out
}
