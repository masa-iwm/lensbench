//! 系の設定: 組合せの開口・視野・波長と重み・エイミング・物体距離・絞りの位置・アフォーカルなど。
//!
//! 値は [`Combo`] が持ち、[`Combo::project`] でプロジェクトファイル（lensbench-project）の項目にする。
//! 名前（serde）はプロジェクトファイルと同じ。

use egui::{Color32, RichText, Ui};
use lensbench_core::system::AfocalUnit;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::combo::{Combo, ElemKind, Pick};
use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::theme;

pub(crate) mod form;
mod pts;
pub use pts::{FieldPt, WavePt};

/// 開口の種類（プロジェクトの `aperture.type`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApKind {
    /// 入射瞳径
    #[default]
    Epd,
    /// 像空間 F 値
    Fnum,
    /// 物体空間 NA
    Na,
    /// 絞りの径で決める
    Float,
    /// 近軸作動 F 値
    Pwfn,
    /// 物体の円錐角
    Cone,
}

impl ApKind {
    pub const ALL: [ApKind; 6] = [
        ApKind::Epd,
        ApKind::Fnum,
        ApKind::Na,
        ApKind::Float,
        ApKind::Pwfn,
        ApKind::Cone,
    ];
    pub fn label(self) -> &'static str {
        match self {
            ApKind::Epd => tr("Entrance pupil diameter"),
            ApKind::Fnum => tr("Image space F/#"),
            ApKind::Na => tr("Object space NA"),
            ApKind::Float => tr("Float by stop size"),
            ApKind::Pwfn => tr("Paraxial working F/#"),
            ApKind::Cone => tr("Object cone angle"),
        }
    }
    fn key(self) -> &'static str {
        match self {
            ApKind::Epd => "epd",
            ApKind::Fnum => "fnum",
            ApKind::Na => "na",
            ApKind::Float => "float",
            ApKind::Pwfn => "pwfn",
            ApKind::Cone => "cone",
        }
    }
    /// 値を入れる種類か（EPD は入射瞳径、Float は絞りの径で、どちらも「絞りの径」の欄）
    pub fn has_value(self) -> bool {
        !matches!(self, ApKind::Epd | ApKind::Float)
    }
    /// 種類を変えたときの初めの値
    fn initial(self) -> f64 {
        match self {
            ApKind::Fnum | ApKind::Pwfn => 5.0,
            ApKind::Na => 0.1,
            ApKind::Cone => 5.0,
            ApKind::Epd | ApKind::Float => 0.0,
        }
    }
    fn unit(self) -> &'static str {
        match self {
            ApKind::Cone => "°",
            _ => "",
        }
    }
    /// 物体が無限遠では使えない
    fn needs_finite(self) -> bool {
        matches!(self, ApKind::Na | ApKind::Cone)
    }
}

/// 視野の種類（プロジェクトの `fields.type`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    #[default]
    Angle,
    ObjectHeight,
    ParaxialImageHeight,
    RealImageHeight,
    Solve,
}

impl FieldKind {
    pub const ALL: [FieldKind; 5] = [
        FieldKind::Angle,
        FieldKind::ObjectHeight,
        FieldKind::ParaxialImageHeight,
        FieldKind::RealImageHeight,
        FieldKind::Solve,
    ];
    pub fn label(self) -> &'static str {
        match self {
            FieldKind::Angle => tr("Angle"),
            FieldKind::ObjectHeight => tr("Object height"),
            FieldKind::ParaxialImageHeight => tr("Paraxial image height"),
            FieldKind::RealImageHeight => tr("Real image height"),
            FieldKind::Solve => tr("Field by condition (solve)"),
        }
    }
    fn key(self) -> &'static str {
        match self {
            FieldKind::Angle => "angle",
            FieldKind::ObjectHeight => "object_height",
            FieldKind::ParaxialImageHeight => "paraxial_image_height",
            FieldKind::RealImageHeight => "real_image_height",
            FieldKind::Solve => "solve",
        }
    }
    fn unit(self) -> &'static str {
        match self {
            FieldKind::Angle => "°",
            FieldKind::Solve => "",
            _ => " mm",
        }
    }
}

/// 条件で定める視野の条件（プロジェクトの `fields.solve`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SolveCond {
    /// 面での主光線の位置
    #[default]
    Position,
    /// 2 面での方向余弦の和
    DirSum,
    /// 2 面での方向余弦の差
    DirDiff,
}

impl SolveCond {
    const ALL: [SolveCond; 3] = [SolveCond::Position, SolveCond::DirSum, SolveCond::DirDiff];
    fn label(self) -> &'static str {
        match self {
            SolveCond::Position => tr("Position at a surface"),
            SolveCond::DirSum => tr("Sum of direction cosines at 2 surfaces"),
            SolveCond::DirDiff => tr("Difference of direction cosines at 2 surfaces"),
        }
    }
}

/// 条件で定める視野の条件と面（番号か名前）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FieldSolve {
    pub cond: SolveCond,
    pub s1: String,
    pub s2: String,
}

impl FieldSolve {
    fn json(&self) -> Value {
        match self.cond {
            SolveCond::Position => json!({ "cond": "position", "surface": self.s1 }),
            SolveCond::DirSum => json!({ "cond": "dir_sum", "s1": self.s1, "s2": self.s2 }),
            SolveCond::DirDiff => json!({ "cond": "dir_diff", "s1": self.s1, "s2": self.s2 }),
        }
    }
}

/// 光線エイミング（プロジェクトの `aiming`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Aiming {
    #[default]
    Off,
    Paraxial,
    Real,
}

impl Aiming {
    const ALL: [Aiming; 3] = [Aiming::Off, Aiming::Paraxial, Aiming::Real];
    fn label(self) -> &'static str {
        match self {
            Aiming::Off => tr("Off"),
            Aiming::Paraxial => tr("Paraxial"),
            Aiming::Real => tr("Real"),
        }
    }
    fn key(self) -> &'static str {
        match self {
            Aiming::Off => "off",
            Aiming::Paraxial => "paraxial",
            Aiming::Real => "real",
        }
    }
}

const AFOCAL_UNITS: [AfocalUnit; 6] = [
    AfocalUnit::Microradians,
    AfocalUnit::Milliradians,
    AfocalUnit::Radians,
    AfocalUnit::ArcSeconds,
    AfocalUnit::ArcMinutes,
    AfocalUnit::Degrees,
];

/// 浮動開口の欄の説明
pub const FLOATING_CLIP_HINT: &str = "Floating apertures (FLAP in .zmx etc.). Uncheck to ignore semi-diameters (to compare with models that do not clip at semi-diameters)";

/// 状態ごとに変えられる系の値（プロジェクトの状態の `project`）。絞りの位置・硝材カタログは全部の状態で共通
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SysSnap {
    pub aperture: ApKind,
    pub ap_value: f64,
    pub epd: f64,
    pub field_kind: FieldKind,
    pub field_solve: FieldSolve,
    pub fields: Vec<FieldPt>,
    pub waves: Vec<WavePt>,
    pub primary: usize,
    pub object_distance: Option<f64>,
    pub telecentric: bool,
    pub aiming: Aiming,
    pub afocal: Option<String>,
}

impl Default for SysSnap {
    fn default() -> Self {
        Combo::default().snap()
    }
}

impl Combo {
    /// 状態ごとに変えられる系の値
    pub fn snap(&self) -> SysSnap {
        SysSnap {
            aperture: self.aperture,
            ap_value: self.ap_value,
            epd: self.epd,
            field_kind: self.field_kind,
            field_solve: self.field_solve.clone(),
            fields: self.fields.clone(),
            waves: self.waves.clone(),
            primary: self.primary,
            object_distance: self.object_distance,
            telecentric: self.telecentric,
            aiming: self.aiming,
            afocal: self.afocal.clone(),
        }
    }

    /// [`Self::snap`] の値を当てる
    /// 開いた系の設定（serve の `system_data` の data）を組合せの設定にする。obj_t は物体面の厚さ（無限遠は inf）。
    /// 条件で定める視野は組合せの欄に移せないので断る（#135）
    pub fn take_system_data(&mut self, d: &Value, obj_t: f64) -> Result<(), String> {
        let empty = serde_json::Map::new();
        let f = form::SysForm::from_data(d.as_object().unwrap_or(&empty))?;
        if f.field_cond.is_some() {
            return Err(
                tr("A system with solved fields cannot be moved to combination settings").into(),
            );
        }
        f.apply_to(self);
        self.object_distance = obj_t.is_finite().then_some(obj_t);
        Ok(())
    }

    pub fn apply_snap(&mut self, s: &SysSnap) {
        self.aperture = s.aperture;
        self.ap_value = s.ap_value;
        self.epd = s.epd;
        self.field_kind = s.field_kind;
        self.field_solve = s.field_solve.clone();
        self.fields = s.fields.clone();
        self.waves = s.waves.clone();
        self.primary = s.primary;
        self.object_distance = s.object_distance;
        self.telecentric = s.telecentric;
        self.aiming = s.aiming;
        self.afocal = s.afocal.clone();
    }

    /// 状態 n の系の値を基本と比べ、違う項目をプロジェクトの状態の `project` の形で返す
    pub fn state_project(&self, n: &str) -> serde_json::Map<String, Value> {
        let mut out = serde_json::Map::new();
        if let Some(s) = self.state_sys.get(n) {
            let mut a = json!({});
            self.put_settings(&mut a);
            let mut t = self.clone();
            t.apply_snap(s);
            let mut b = json!({});
            t.put_settings(&mut b);
            for k in [
                "wavelengths",
                "primary",
                "aperture",
                "fields",
                "object_distance",
                "telecentric",
                "aiming",
                "afocal",
            ] {
                if a.get(k) == b.get(k) {
                    continue;
                }
                // 書かない項目は既定（無限遠・テレセントリックなし・エイミングなし・アフォーカルなし）
                let x = b.get(k).cloned().unwrap_or(match k {
                    "telecentric" => json!(false),
                    "aiming" => json!("off"),
                    _ => Value::Null,
                });
                out.insert(k.into(), x);
            }
        }
        if let Some(p) = self.state_path.get(n) {
            let p: String = p
                .chars()
                .map(|c| c.to_ascii_uppercase())
                .filter(|c| matches!(c, 'T' | 'R'))
                .collect();
            let base: String = self.path.chars().map(|c| c.to_ascii_uppercase()).collect();
            if p != base {
                out.insert(
                    "path".into(),
                    if p.is_empty() { Value::Null } else { json!(p) },
                );
            }
        }
        out
    }
}

/// 絞りがどこにあるか
#[derive(Debug, Clone, PartialEq)]
pub enum StopAt {
    /// 絞りの要素が無く、最初の要素の前に置く
    Lead,
    /// k 番目（0 始まり）の絞りの要素
    Pick(usize),
    /// 絞りの要素が 2 つ以上（系を作れない）
    Many(Vec<usize>),
}

impl Combo {
    pub fn stop_at(&self) -> StopAt {
        // 分岐の先も含めた全部の要素（[`Combo::all`] の順。分岐が無ければ並びの番号）
        let ks: Vec<usize> = self
            .all()
            .iter()
            .enumerate()
            .filter(|(_, l)| self.get(l).is_some_and(Pick::is_stop))
            .map(|(k, _)| k)
            .collect();
        match ks.as_slice() {
            [] => StopAt::Lead,
            [k] => StopAt::Pick(*k),
            _ => StopAt::Many(ks),
        }
    }

    /// 前に自動で置いている絞りを、並びの先頭の「絞り」の要素にする（間隔・径はそのまま）
    pub fn make_stop_element(&mut self) -> Option<usize> {
        if !self.lead_stop() {
            return None;
        }
        let mut p = Pick::new(ElemKind::Stop);
        p.semi = self.epd / 2.0;
        p.air = self.stop_gap;
        self.picks.insert(0, p);
        Some(0)
    }

    /// プロジェクトの `aperture`
    pub fn aperture_json(&self) -> Value {
        let value = match self.aperture {
            ApKind::Epd => self.epd,
            ApKind::Float => 0.0,
            _ => self.ap_value,
        };
        json!({ "type": self.aperture.key(), "value": value })
    }

    /// プロジェクトの `fields`
    pub fn fields_json(&self) -> Value {
        let mut f = json!({
            "type": self.field_kind.key(),
            "points": self.fields.iter().map(|p| json!([p.x, p.y])).collect::<Vec<_>>(),
        });
        if self.fields.iter().any(|p| p.w != 1.0) {
            f["weights"] = json!(self.fields.iter().map(|p| p.w).collect::<Vec<_>>());
        }
        if self.fields.iter().any(FieldPt::has_vig) {
            f["vignetting"] = json!(self.fields.iter().map(|p| p.vig).collect::<Vec<_>>());
        }
        if self.field_kind == FieldKind::Solve {
            f["solve"] = self.field_solve.json();
        }
        f
    }

    /// プロジェクトの系の値（波長・開口・視野・物体距離・エイミング・アフォーカル）を足す
    pub fn put_settings(&self, v: &mut Value) {
        v["wavelengths"] = json!(self
            .waves
            .iter()
            .map(|w| if w.w == 1.0 {
                json!({ "um": w.um })
            } else {
                json!({ "um": w.um, "weight": w.w })
            })
            .collect::<Vec<_>>());
        v["primary"] = json!(self.primary.clamp(1, self.waves.len().max(1)));
        v["aperture"] = self.aperture_json();
        v["fields"] = self.fields_json();
        self.extra.put(v);
        if let Some(d) = self.object_distance {
            v["object_distance"] = json!(d);
            if self.telecentric {
                v["telecentric"] = json!(true);
            }
        }
        if self.aiming != Aiming::Off {
            v["aiming"] = json!(self.aiming.key());
        }
        if let Some(u) = &self.afocal {
            v["afocal"] = json!(u.to_ascii_lowercase());
        }
        if !self.floating_clip {
            v["floating_clip"] = json!(false);
        }
        let cats: Vec<&str> = self
            .glass_catalogs
            .iter()
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .collect();
        if !cats.is_empty() {
            v["glass_catalogs"] = json!(cats);
        }
    }

    /// 設定の一行の要約（開口・視野・波長・物体）
    pub fn settings_summary(&self) -> String {
        let ap = match self.aperture {
            ApKind::Epd => format!("EPD {} mm", fmt_num(self.epd)),
            ApKind::Float => format!("{} {} mm", tr("Stop diameter"), fmt_num(self.epd)),
            k => format!("{} {}{}", k.label(), fmt_num(self.ap_value), k.unit()),
        };
        let obj = match self.object_distance {
            None => tr("Infinity").to_string(),
            Some(d) => format!("{} {} mm", tr("Object"), fmt_num(d)),
        };
        trf(
            "{} · {} {} points · {} wavelengths · {}",
            &[
                &ap,
                &self.field_kind.label(),
                &self.fields.len(),
                &self.waves.len(),
                &obj,
            ],
        )
    }

    /// 系を作れない設定の説明（作る前に画面で示す）
    pub fn settings_problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(Err(e)) = self.detector.as_ref().map(|d| d.block()) {
            out.push(trf("Detector settings: {}", &[&e]));
        }
        if self.waves.is_empty() {
            out.push(tr("No wavelengths").to_string());
        }
        if self.fields.is_empty() {
            out.push(tr("No fields").to_string());
        }
        if self.object_distance.is_none() {
            if self.aperture.needs_finite() {
                out.push(tr("This aperture type needs a finite object").to_string());
            }
            if self.field_kind == FieldKind::ObjectHeight {
                out.push(tr("Object-height fields need a finite object").to_string());
            }
        }
        if self.telecentric && self.object_distance.is_some() {
            if self.aiming != Aiming::Off {
                out.push(
                    tr("Ray aiming cannot be used with object-space telecentricity").to_string(),
                );
            }
            if !self.aperture.needs_finite() {
                out.push(
                    tr("Object-space telecentricity needs an object space NA or object cone angle aperture").to_string(),
                );
            }
        }
        if self.field_kind == FieldKind::Solve
            && (self.field_solve.s1.trim().is_empty()
                || (self.field_solve.cond != SolveCond::Position
                    && self.field_solve.s2.trim().is_empty()))
        {
            out.push(tr("Enter the surfaces for the field condition").to_string());
        }
        if let StopAt::Many(ks) = self.stop_at() {
            out.push(trf("{} stop elements (keep only one)", &[&ks.len()]));
        }
        out
    }
}

/// 系の設定の欄。cats は硝材のフォルダにある AGF の名前（硝材カタログの候補）。状態を選んでいれば、
/// その状態だけの値（開口・視野・波長・物体距離・エイミング・アフォーカル）にもできる
/// 最後に置く検出器（拡張が足した検出器のブロック。無ければ出さない）
fn detector_ui(ui: &mut Ui, c: &mut Combo, id: egui::Id) {
    let kinds = crate::ext::detectors();
    if kinds.is_empty() && c.detector.is_none() {
        return;
    }
    ui.add_space(6.0);
    let head = match c
        .detector
        .as_ref()
        .and_then(|d| crate::ext::detector(&d.kind))
    {
        Some(d) => format!("{} ({})", tr("Detector"), tr(d.object.title)),
        None => tr("Detector").to_string(),
    };
    egui::CollapsingHeader::new(RichText::new(head).strong())
        .id_salt(id.with("detector"))
        .show(ui, |ui| {
            let weak = theme::weak_text(ui.visuals());
            let cur = c.detector.as_ref().map(|d| d.kind.clone());
            ui.horizontal_wrapped(|ui| {
                if ui.selectable_label(cur.is_none(), tr("Off")).clicked() {
                    c.detector = None;
                }
                for d in &kinds {
                    if ui
                        .selectable_label(cur.as_deref() == Some(d.kind), tr(d.object.title))
                        .clicked()
                        && cur.as_deref() != Some(d.kind)
                    {
                        c.detector = Some(crate::combo::DetectorPick {
                            kind: d.kind.into(),
                            ..Default::default()
                        });
                    }
                }
            });
            ui.label(
                RichText::new(tr(
                    "Detector block at the image surface (the last block of the project; the last air is the distance to the detector). Empty fields use the defaults",
                ))
                .small()
                .color(weak),
            );
            let Some(d) = c.detector.as_mut() else {
                return;
            };
            let Some(kind) = crate::ext::detector(&d.kind) else {
                ui.label(trf("Detector {} is not available in this build (its settings are kept)", &[&d.kind]));
                return;
            };
            let specs = crate::argform::object_specs(&kind.object);
            let mut done = vec![false; specs.len()];
            let mut groups: Vec<(&str, Vec<&crate::argform::ArgSpec>)> = Vec::new();
            for (prefix, title) in kind.object.groups {
                let g: Vec<usize> = (0..specs.len())
                    .filter(|&k| !done[k] && specs[k].name.starts_with(prefix))
                    .collect();
                for &k in &g {
                    done[k] = true;
                }
                groups.push((title, g.into_iter().map(|k| &specs[k]).collect()));
            }
            let rest: Vec<&crate::argform::ArgSpec> =
                (0..specs.len()).filter(|&k| !done[k]).map(|k| &specs[k]).collect();
            if !rest.is_empty() {
                groups.push(("", rest));
            }
            for (title, g) in groups {
                if g.is_empty() {
                    continue;
                }
                egui::CollapsingHeader::new(if title.is_empty() { tr("Others") } else { tr(title) })
                    .id_salt(id.with(("detector_group", title)))
                    .show(ui, |ui| {
                        crate::argform::form(ui, ("detector", title), &g, &mut d.vals);
                    });
            }
            // 空の欄は持たない（組合せの比べ・保存を軽く）
            d.vals.retain(|_, t| !t.trim().is_empty());
        });
}

pub fn ui(ui: &mut Ui, c: &mut Combo, cats: &[String]) {
    let Some(n) = c.active_state().map(str::to_string) else {
        body(ui, c, cats);
        return;
    };
    let weak = theme::weak_text(ui.visuals());
    let mut own = c.state_sys.contains_key(&n);
    let r = ui
        .checkbox(&mut own, trf("Use values for state {} only", &[&n]))
        .on_hover_text(tr(
            "Change the aperture, fields, wavelengths, object distance, telecentricity, aiming and afocal mode for this state only (stop position and glass catalogs are shared by all states)",
        ));
    if r.changed() {
        if own {
            c.state_sys.insert(n.clone(), c.snap());
        } else {
            c.state_sys.remove(&n);
        }
    }
    let Some(snap) = c.state_sys.get(&n).cloned() else {
        ui.label(
            RichText::new(trf("State {} uses the base values too", &[&n]))
                .small()
                .color(weak),
        );
        ui.add_space(4.0);
        body(ui, c, cats);
        return;
    };
    // 状態の値を当てた写しで編集し、状態の値と、ほかの（共通の）項目の変更を戻す
    let base = c.snap();
    let mut t = c.clone();
    t.apply_snap(&snap);
    body(ui, &mut t, cats);
    let edited = t.snap();
    t.apply_snap(&base);
    t.state_sys.insert(n, edited);
    if t != *c {
        *c = t;
    }
}

/// 組合せだけの項目（絞りの径と位置・物体・最初の空気）を共通の欄に差し込む
struct ComboExtra<'a>(&'a mut Combo);

impl form::Extra for ComboExtra<'_> {
    fn aperture_rows(&mut self, ui: &mut Ui, f: &mut form::SysForm) {
        let c = &*self.0;
        // 前に自動で置く絞りの径（EPD は入射瞳径の欄が兼ねる。絞りが要素なら要素の半径）
        if f.aperture != ApKind::Epd && c.lead_stop() {
            ui.label(if f.aperture == ApKind::Float {
                tr("Stop diameter [mm]")
            } else {
                tr("Stop diameter (drawing only) [mm]")
            });
            // 欄の値（f）を変える。c を直接変えると後の apply_to で戻る
            ui.add(
                egui::DragValue::new(&mut f.epd)
                    .speed(0.1)
                    .range(1e-3..=1e4)
                    .max_decimals(4),
            );
            ui.end_row();
        }
    }

    fn after_aperture(&mut self, ui: &mut Ui, f: &form::SysForm) {
        let c = &mut *self.0;
        let weak = theme::weak_text(ui.visuals());
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("Aperture stop")).strong());
            match c.stop_at() {
                StopAt::Lead => {
                    ui.label(tr("Before the first element (automatic)"));
                    ui.label(tr("Gap"));
                    ui.add(
                        egui::DragValue::new(&mut c.stop_gap)
                            .speed(0.1)
                            .range(0.0..=1e5)
                            .suffix(" mm"),
                    );
                    if ui
                        .button(tr("Make the stop an element"))
                        .on_hover_text(tr(
                            "Put a \"Stop\" element at the head of the sequence. Drag it in the figure to move it anywhere",
                        ))
                        .clicked()
                    {
                        c.make_stop_element();
                    }
                }
                StopAt::Pick(k) => {
                    let label = c
                        .all()
                        .get(k)
                        .and_then(|l| c.get(l))
                        .map(|p| p.label())
                        .unwrap_or_default();
                    ui.label(trf("Element {}: \"{}\"", &[&(k + 1), &label]));
                }
                StopAt::Many(ks) => {
                    let s: Vec<String> = ks.iter().map(|k| (k + 1).to_string()).collect();
                    ui.label(
                        RichText::new(trf("Elements {} (keep only one)", &[&s.join(", ")]))
                            .color(Color32::RED),
                    );
                }
            }
        });
        ui.label(
            RichText::new(tr(
                "Place a \"Stop\" from the element palette anywhere in the sequence to set the stop there. Without one, a stop is placed automatically before the first element.",
            ))
            .small()
            .color(weak),
        );
        if f.aperture == ApKind::Float && !c.lead_stop() {
            ui.label(
                RichText::new(tr("The aperture is set by the stop element's radius"))
                    .small()
                    .color(weak),
            );
        }
    }

    fn option_rows(&mut self, ui: &mut Ui) {
        let c = &mut *self.0;
        ui.label(tr("Object"));
        ui.horizontal(|ui| {
            let mut finite = c.object_distance.is_some();
            ui.radio_value(&mut finite, false, tr("Infinity"));
            ui.radio_value(&mut finite, true, tr("Finite"));
            match (finite, c.object_distance) {
                (true, None) => c.object_distance = Some(100.0),
                (false, Some(_)) => c.object_distance = None,
                _ => {}
            }
            if let Some(d) = c.object_distance.as_mut() {
                ui.add(
                    egui::DragValue::new(d)
                        .speed(0.5)
                        .range(0.0..=1e9)
                        .max_decimals(4)
                        .suffix(" mm"),
                )
                .on_hover_text(tr(
                    "From the object surface to the first element (or the leading stop)",
                ));
            }
        });
        ui.end_row();
        crate::combo_tree::lead_air_row(ui, c);
    }

    fn telecentric(&self) -> bool {
        self.0.object_distance.is_some()
    }
}

fn body(ui: &mut Ui, c: &mut Combo, cats: &[String]) {
    let weak = theme::weak_text(ui.visuals());
    let id = ui.id().with("sysset");
    let mut f = form::SysForm::of_combo(c);
    form::ui(ui, &mut f, &mut ComboExtra(c));
    f.apply_to(c);
    ui.separator();
    glass_catalogs_ui(ui, c, cats, weak);
    let probs = c.settings_problems();
    if !probs.is_empty() {
        ui.separator();
        for p in probs {
            ui.label(RichText::new(format!("⚠ {p}")).color(Color32::from_rgb(220, 80, 60)));
        }
    }
    // 膜（フィルタ）の定義（プロジェクトの coatings）
    ui.add_space(6.0);
    let head = if c.coatings.is_empty() {
        tr("Coating (filter) definitions").to_string()
    } else {
        format!(
            "{} ({})",
            tr("Coating (filter) definitions"),
            c.coatings.len()
        )
    };
    // 通し確認では開いて見せる（[`crate::coat_defs::focus`]）
    let focus = crate::coat_defs::take_focus(ui.ctx());
    let r = egui::CollapsingHeader::new(RichText::new(head).strong())
        .id_salt(id.with("coat_defs"))
        .open(focus.then_some(true))
        .show(ui, |ui| {
            crate::coat_defs::ui(ui, &mut c.coatings);
        });
    if focus {
        r.header_response.scroll_to_me(Some(egui::Align::Min));
    }
    detector_ui(ui, c, id);
}

/// 硝材カタログ: 前ほど優先。硝材のフォルダの AGF から足す・外す・順を変える
fn glass_catalogs_ui(ui: &mut Ui, c: &mut Combo, cats: &[String], weak: Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("Glass catalogs")).strong());
        let n = c.glass_catalogs.len();
        let (mut up, mut del) = (None, None);
        for (i, name) in c.glass_catalogs.iter().enumerate() {
            let missing = !cats.is_empty() && !cats.iter().any(|x| x.eq_ignore_ascii_case(name));
            let t = RichText::new(name.as_str());
            ui.label(if missing {
                t.color(Color32::from_rgb(220, 80, 60))
            } else {
                t
            })
            .on_hover_text(if missing {
                tr("Not in the glass folders")
            } else {
                tr("Earlier ones take precedence")
            });
            if i > 0 && ui.small_button("◀").on_hover_text(tr("Earlier")).clicked() {
                up = Some(i);
            }
            if ui.small_button("✖").on_hover_text(tr("Remove")).clicked() {
                del = Some(i);
            }
            if i + 1 < n {
                ui.weak("·");
            }
        }
        if let Some(i) = up {
            c.glass_catalogs.swap(i - 1, i);
        }
        if let Some(i) = del {
            c.glass_catalogs.remove(i);
        }
        let rest: Vec<&String> = cats
            .iter()
            .filter(|x| !c.glass_catalogs.iter().any(|y| y.eq_ignore_ascii_case(x)))
            .collect();
        if !rest.is_empty() {
            ui.menu_button(tr("+ Add"), |ui| {
                for x in rest {
                    if ui.button(x.as_str()).clicked() {
                        c.glass_catalogs.push(x.clone());
                        ui.close_menu();
                    }
                }
            });
        }
    });
    ui.label(
        RichText::new(tr(
            "Catalogs for the glass of custom lenses, plates and beam splitters (earlier ones first; SCHOTT if empty). Choices are the AGF files in the glass folders",
        ))
        .small()
        .color(weak),
    );
}

/// 波長のよく使う組（F・d・C、e、He-Ne）
const PRESETS: &[(&str, &[f64])] = &[
    ("F d C", &[0.4861327, 0.5875618, 0.6562725]),
    ("F' e C'", &[0.4799914, 0.5460740, 0.6438470]),
    ("d", &[0.5875618]),
    ("He-Ne", &[0.6328]),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// 開いた系（.zmx）を組合せ（系の取り込み 1 つ＋系の設定）にすると、同じ系になる（#135）
    #[test]
    fn open_system_becomes_a_combo() {
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let glass = vec![root.join("tests/glass")];
        let zmx = root.join("tests/refs/A0/system.zmx");
        let opt = lensbench_core::zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: glass.clone(),
            ..Default::default()
        };
        let a = lensbench_core::zmx::load_with(&zmx, &opt).unwrap();
        let d = lensbench_project::edit_ops::system_data(&a).unwrap();
        let mut c = Combo::default();
        c.take_system_data(&d, a.surfaces[0].t).unwrap();
        let mut z = Pick::new(ElemKind::ZmxRange);
        z.path = zmx.display().to_string();
        z.air = 0.0;
        c.picks.push(z);
        assert!(
            c.settings_problems().is_empty(),
            "{:?}",
            c.settings_problems()
        );
        let p = lensbench_project::Project::from_json(&c.project().to_string()).unwrap();
        let env = lensbench_project::Env {
            step_files: None,
            base: root.clone(),
            glass_dirs: glass,
            catalog: None,
        };
        let b = lensbench_project::expand(&p, &env, &lensbench_project::ExpandOpts::default())
            .map(|x| x.system)
            .unwrap();
        assert_eq!(b.surfaces.len(), a.surfaces.len());
        for (x, y) in a.surfaces.iter().zip(&b.surfaces) {
            assert_eq!(x.c, y.c);
            assert!((x.t - y.t).abs() < 1e-12 || x.t == y.t, "{} {}", x.t, y.t);
        }
        assert_eq!(b.wavelengths, a.wavelengths);
        assert_eq!(b.primary, a.primary);
        assert_eq!(b.fields.len(), a.fields.len());
        assert_eq!(b.field_type, a.field_type);
        assert_eq!(c.aperture, ApKind::Epd);
        assert_eq!(c.object_distance, None);
        // 条件で定める視野は断る
        let mut d2 = d.clone();
        d2["field_type"] = json!({ "Solve": {} });
        assert!(c.take_system_data(&d2, 0.0).is_err());
    }

    #[test]
    fn settings_go_to_the_project() {
        let mut c = Combo {
            aperture: ApKind::Fnum,
            ap_value: 4.0,
            field_kind: FieldKind::ObjectHeight,
            fields: vec![FieldPt::xyw(1.0, 2.0, 0.5)],
            waves: vec![WavePt { um: 0.6, w: 2.0 }],
            primary: 3,
            object_distance: Some(50.0),
            telecentric: true,
            aiming: Aiming::Real,
            afocal: Some("as".into()),
            ..Default::default()
        };
        c.picks.push(Pick::new(ElemKind::Lens));
        let p = c.project();
        assert_eq!(p["aperture"], json!({"type": "fnum", "value": 4.0}));
        assert_eq!(p["fields"]["type"], "object_height");
        assert_eq!(p["fields"]["points"], json!([[1.0, 2.0]]));
        assert_eq!(p["fields"]["weights"], json!([0.5]));
        assert_eq!(p["wavelengths"], json!([{"um": 0.6, "weight": 2.0}]));
        assert_eq!(p["primary"], 1);
        assert_eq!(p["object_distance"], 50.0);
        assert_eq!(p["telecentric"], true);
        assert_eq!(p["aiming"], "real");
        assert_eq!(p["afocal"], "as");
        assert!(c
            .settings_problems()
            .iter()
            .any(|s| s.contains("telecentric")));
        // 既定の組合せは項目を増やさない
        let d = Combo::default().project();
        for k in ["object_distance", "telecentric", "aiming", "afocal"] {
            assert!(d.get(k).is_none(), "{k}");
        }
        assert!(d["fields"].get("weights").is_none());
        assert_eq!(d["aperture"], json!({"type": "epd", "value": 5.0}));
    }

    #[test]
    fn solve_field_and_float() {
        let c = Combo {
            aperture: ApKind::Float,
            field_kind: FieldKind::Solve,
            field_solve: FieldSolve {
                cond: SolveCond::DirSum,
                s1: "1".into(),
                s2: "6".into(),
            },
            ..Default::default()
        };
        let p = c.project();
        assert_eq!(p["aperture"]["type"], "float");
        assert_eq!(p["blocks"][0]["semi"], 2.5);
        assert_eq!(
            p["fields"]["solve"],
            json!({"cond": "dir_sum", "s1": "1", "s2": "6"})
        );
    }

    /// 先頭に自動で置く絞りの径（入射瞳径以外の種類）は欄の値 f.epd で変え、apply_to で組合せに残る。
    /// 何も変えないフレームでは組合せは変わらない
    #[test]
    fn stop_diameter_goes_through_the_form() {
        let mut c = Combo {
            aperture: ApKind::Fnum,
            ap_value: 4.0,
            epd: 8.0,
            picks: vec![Pick::new(ElemKind::Lens)],
            ..Default::default()
        };
        assert!(c.lead_stop());
        let before = c.clone();
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| body(ui, &mut c, &[]));
        });
        assert_eq!(c, before);
        let mut f = form::SysForm::of_combo(&c);
        f.epd = 6.0;
        f.apply_to(&mut c);
        assert_eq!((c.epd, c.ap_value, c.aperture), (6.0, 4.0, ApKind::Fnum));
    }

    #[test]
    fn lead_stop_becomes_an_element() {
        let mut c = Combo {
            epd: 6.0,
            stop_gap: 3.0,
            picks: vec![Pick::new(ElemKind::Lens)],
            ..Default::default()
        };
        assert_eq!(c.stop_at(), StopAt::Lead);
        let before = c.project()["blocks"].clone();
        assert_eq!(c.make_stop_element(), Some(0));
        assert_eq!(c.stop_at(), StopAt::Pick(0));
        assert_eq!(c.make_stop_element(), None);
        // 作る系は同じ（絞り・空気・玉の並び）
        let after = c.project()["blocks"].clone();
        assert_eq!(before[0]["semi"], after[0]["semi"]);
        assert_eq!(before[1], after[1]);
        assert_eq!(
            before.as_array().unwrap().len(),
            after.as_array().unwrap().len()
        );
        c.picks.push(Pick::new(ElemKind::Stop));
        assert!(matches!(c.stop_at(), StopAt::Many(_)));
        assert!(!c.settings_problems().is_empty());
    }

    /// 玉の前面を絞りに・硝材カタログ: 絞りは玉の前面で、前に自動の絞りを置かない。硝材カタログは
    /// プロジェクトの glass_catalogs になり、OHARA のガラスも引ける
    #[test]
    fn lens_front_stop_and_glass_catalogs() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let glass = root.join("tests/glass").display().to_string();
        let dir = std::env::temp_dir().join(format!("lensbench-stopfront-{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&dir).unwrap();
        let f = dir.join("combo.json");
        let mut c = Combo::default();
        let mut a = Pick::new(ElemKind::Lens);
        a.air = 10.0;
        let mut b = Pick::new(ElemKind::Lens);
        b.glass = "S-FSL5".into();
        b.stop_front = true;
        b.air = 40.0;
        c.picks = vec![a, b];
        assert_eq!(c.stop_at(), StopAt::Pick(1));
        assert!(!c.lead_stop());
        c.glass_catalogs = vec!["OHARA".into(), "SCHOTT".into()];
        let p = c.project();
        assert_eq!(p["glass_catalogs"], json!(["OHARA", "SCHOTT"]));
        let bl = p["blocks"].as_array().unwrap();
        assert_eq!(bl[0]["type"], "lens");
        assert_eq!(bl[2]["stop"], true);
        lensbench_core::vfs::write(&f, p.to_string()).unwrap();
        let load = json!({ "file": f.display().to_string(), "glass_dirs": [glass] });
        let v = lensbench_engine::preview_layout(&load, &json!({})).unwrap();
        let surfs = v["surfaces"].as_array().unwrap();
        let st = surfs.iter().position(|x| x["stop"] == true).unwrap();
        // 物体・玉 1 の 2 面のあと、玉 2 の前面
        assert_eq!(st, 3, "{v}");
        // ユーザーカタログの項目（群）は前面を絞りにできない
        let mut u = Pick::new(ElemKind::Catalog);
        u.path = "x.json".into();
        u.stop_front = true;
        assert!(!u.is_stop() && u.block("U")["stop"].is_null());
        // .zmx の玉の構成番号
        let mut z = Pick::new(ElemKind::Zmx);
        z.path = "a.zmx".into();
        assert!(z.block("Z")["source"]["config"].is_null());
        z.config = 2;
        assert_eq!(z.block("Z")["source"]["config"], 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 画面で問題が無いと言う設定は、serve で系になる
    #[test]
    fn settings_build_through_serve() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let glass = root.join("tests/glass").display().to_string();
        let dir = std::env::temp_dir().join(format!("lensbench-sysset-{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&dir).unwrap();
        let f = dir.join("combo.json");
        let build = |c: &Combo| {
            lensbench_core::vfs::write(&f, c.project().to_string()).unwrap();
            let load = json!({ "file": f.display().to_string(), "glass_dirs": [glass] });
            lensbench_engine::preview_layout(&load, &json!({}))
        };
        let mut base = Combo::default();
        let mut lens = Pick::new(ElemKind::Lens);
        lens.air = 45.0;
        base.picks.push(lens);
        let mut n = 0;
        for ap in ApKind::ALL {
            for fk in [
                FieldKind::Angle,
                FieldKind::ObjectHeight,
                FieldKind::ParaxialImageHeight,
                FieldKind::RealImageHeight,
            ] {
                for obj in [None, Some(80.0)] {
                    for aim in Aiming::ALL {
                        let mut c = base.clone();
                        c.aperture = ap;
                        c.ap_value = ap.initial();
                        c.field_kind = fk;
                        c.object_distance = obj;
                        c.aiming = aim;
                        if fk != FieldKind::Angle {
                            c.fields = [0.0, 0.5, 1.0].map(FieldPt::y).to_vec();
                        }
                        let r = build(&c);
                        // 問題があると言う設定の中には、系になっても瞳が無限大になるもの（無限遠物体の NA）もある
                        if c.settings_problems().is_empty() {
                            assert!(r.is_ok(), "{ap:?} {fk:?} {obj:?} {aim:?}: {:?}", r.err());
                            n += 1;
                        }
                    }
                }
            }
        }
        assert!(n > 40, "{n}");
        // 重み・アフォーカル・物体側テレセントリック
        let mut c = base.clone();
        c.fields[1].w = 2.0;
        c.waves[0].w = 0.5;
        c.afocal = Some("mr".into());
        assert!(build(&c).is_ok());
        c.afocal = None;
        c.object_distance = Some(80.0);
        c.telecentric = true;
        c.aperture = ApKind::Na;
        c.ap_value = 0.05;
        c.field_kind = FieldKind::ObjectHeight;
        assert!(c.settings_problems().is_empty());
        assert!(build(&c).is_ok(), "{:?}", build(&c).err());
        c.aperture = ApKind::Fnum;
        assert!(!c.settings_problems().is_empty());
        assert!(build(&c).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
