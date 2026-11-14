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
            ApKind::Epd => tr("入射瞳径"),
            ApKind::Fnum => tr("像空間 F 値"),
            ApKind::Na => tr("物体空間 NA"),
            ApKind::Float => tr("絞りの径で決める"),
            ApKind::Pwfn => tr("近軸作動 F 値"),
            ApKind::Cone => tr("物体の円錐角"),
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
            FieldKind::Angle => tr("角度"),
            FieldKind::ObjectHeight => tr("物体高"),
            FieldKind::ParaxialImageHeight => tr("近軸像高"),
            FieldKind::RealImageHeight => tr("実像高"),
            FieldKind::Solve => tr("条件で定める視野"),
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
            SolveCond::Position => tr("面での位置"),
            SolveCond::DirSum => tr("2 面の方向余弦の和"),
            SolveCond::DirDiff => tr("2 面の方向余弦の差"),
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
            Aiming::Off => tr("なし"),
            Aiming::Paraxial => tr("近軸"),
            Aiming::Real => tr("実光線"),
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

/// 視野の点（x, y と重み）。古い保存の数（視野角 y）も読む
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(from = "FieldIn")]
pub struct FieldPt {
    pub x: f64,
    pub y: f64,
    pub w: f64,
}

impl FieldPt {
    pub fn y(y: f64) -> FieldPt {
        FieldPt { x: 0.0, y, w: 1.0 }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FieldIn {
    Y(f64),
    Pt {
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        #[serde(default = "one")]
        w: f64,
    },
}

impl From<FieldIn> for FieldPt {
    fn from(v: FieldIn) -> FieldPt {
        match v {
            FieldIn::Y(y) => FieldPt::y(y),
            FieldIn::Pt { x, y, w } => FieldPt { x, y, w },
        }
    }
}

/// 波長 [µm] と重み。古い保存の数（波長だけ）も読む
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(from = "WaveIn")]
pub struct WavePt {
    pub um: f64,
    pub w: f64,
}

impl WavePt {
    pub fn um(um: f64) -> WavePt {
        WavePt { um, w: 1.0 }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WaveIn {
    Um(f64),
    Pt {
        um: f64,
        #[serde(default = "one")]
        w: f64,
    },
}

impl From<WaveIn> for WavePt {
    fn from(v: WaveIn) -> WavePt {
        match v {
            WaveIn::Um(um) => WavePt::um(um),
            WaveIn::Pt { um, w } => WavePt { um, w },
        }
    }
}

fn one() -> f64 {
    1.0
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
pub const FLOATING_CLIP_HINT: &str = "浮動開口（.zmx の FLAP など）。外すと半直径でけらない（半直径でけらない従来のモデルと比べるとき）";

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
        let wl: Vec<f64> = d["wavelengths"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default();
        if wl.is_empty() {
            return Err(tr("系の波長が読めない").into());
        }
        let ws: Vec<f64> = d["weights"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default();
        self.waves = wl
            .iter()
            .enumerate()
            .map(|(i, &um)| WavePt {
                um,
                w: ws.get(i).copied().unwrap_or(1.0),
            })
            .collect();
        self.primary = d["primary"].as_u64().map_or(1, |p| p as usize + 1);
        self.field_kind = match d["field_type"].as_str() {
            Some("Angle") => FieldKind::Angle,
            Some("ObjH") => FieldKind::ObjectHeight,
            Some("ParH") => FieldKind::ParaxialImageHeight,
            Some("RealH") => FieldKind::RealImageHeight,
            _ => return Err(tr("条件で定める視野の系は、組合せの設定に移せない").into()),
        };
        self.fields = d["fields"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|f| FieldPt {
                        x: f["x"].as_f64().unwrap_or(0.0),
                        y: f["y"].as_f64().unwrap_or(0.0),
                        w: f["w"].as_f64().unwrap_or(1.0),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let (kind, val) = (d["aperture"][0].as_str(), d["aperture"][1].as_f64());
        let val = val.unwrap_or(0.0);
        self.aperture = match kind {
            Some("Epd") => ApKind::Epd,
            Some("FNum") => ApKind::Fnum,
            Some("Na") => ApKind::Na,
            Some("Float") => ApKind::Float,
            Some("Pwfn") => ApKind::Pwfn,
            Some("Cone") => ApKind::Cone,
            _ => return Err(tr("系の開口が読めない").into()),
        };
        if self.aperture == ApKind::Epd {
            self.epd = val;
        } else {
            self.ap_value = val;
        }
        self.object_distance = obj_t.is_finite().then_some(obj_t);
        self.telecentric = d["telecentric"].as_bool().unwrap_or(false);
        self.floating_clip = d["floating_clip"].as_bool().unwrap_or(true);
        self.aiming = match d["aiming"].as_str() {
            Some("Paraxial") => Aiming::Paraxial,
            Some("Real") => Aiming::Real,
            _ => Aiming::Off,
        };
        self.afocal = if d["afocal"].as_bool() == Some(true) {
            Some(
                match d["afocal_unit"].as_str() {
                    Some("Microradians") => "ur",
                    Some("Radians") => "rr",
                    Some("ArcSeconds") => "as",
                    Some("ArcMinutes") => "am",
                    Some("Degrees") => "dg",
                    _ => "mr",
                }
                .to_string(),
            )
        } else {
            None
        };
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
            ApKind::Float => format!("{} {} mm", tr("絞りの径"), fmt_num(self.epd)),
            k => format!("{} {}{}", k.label(), fmt_num(self.ap_value), k.unit()),
        };
        let obj = match self.object_distance {
            None => tr("無限遠").to_string(),
            Some(d) => format!("{} {} mm", tr("物体"), fmt_num(d)),
        };
        trf(
            "{} · {} {} 点 · 波長 {} · {}",
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
            out.push(trf("検出器の設定: {}", &[&e]));
        }
        if self.waves.is_empty() {
            out.push(tr("波長が無い").to_string());
        }
        if self.fields.is_empty() {
            out.push(tr("視野が無い").to_string());
        }
        if self.object_distance.is_none() {
            if self.aperture.needs_finite() {
                out.push(tr("この開口の種類は物体が有限のときだけ").to_string());
            }
            if self.field_kind == FieldKind::ObjectHeight {
                out.push(tr("物体高の視野は物体が有限のときだけ").to_string());
            }
        }
        if self.telecentric && self.object_distance.is_some() {
            if self.aiming != Aiming::Off {
                out.push(tr("物体側テレセントリックではエイミングを使えない").to_string());
            }
            if !self.aperture.needs_finite() {
                out.push(
                    tr("物体側テレセントリックの開口は物体空間 NA か物体の円錐角").to_string(),
                );
            }
        }
        if self.field_kind == FieldKind::Solve
            && (self.field_solve.s1.trim().is_empty()
                || (self.field_solve.cond != SolveCond::Position
                    && self.field_solve.s2.trim().is_empty()))
        {
            out.push(tr("条件で定める視野の面を入れる").to_string());
        }
        if let StopAt::Many(ks) = self.stop_at() {
            out.push(trf("絞りの要素が {} 個（1 つだけにする）", &[&ks.len()]));
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
        Some(d) => format!("{} ({})", tr("検出器"), tr(d.object.title)),
        None => tr("検出器").to_string(),
    };
    egui::CollapsingHeader::new(RichText::new(head).strong())
        .id_salt(id.with("detector"))
        .show(ui, |ui| {
            let weak = theme::weak_text(ui.visuals());
            let cur = c.detector.as_ref().map(|d| d.kind.clone());
            ui.horizontal_wrapped(|ui| {
                if ui.selectable_label(cur.is_none(), tr("なし")).clicked() {
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
                    "像面に置く検出器のブロック（プロジェクトの最後のブロック、最後の空気が検出器までの距離）。空の欄は既定値",
                ))
                .small()
                .color(weak),
            );
            let Some(d) = c.detector.as_mut() else {
                return;
            };
            let Some(kind) = crate::ext::detector(&d.kind) else {
                ui.label(trf("検出器 {} はこの版で使えない（設定はそのまま残す）", &[&d.kind]));
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
                egui::CollapsingHeader::new(if title.is_empty() { tr("そのほか") } else { tr(title) })
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
        .checkbox(&mut own, trf("状態 {} だけの値にする", &[&n]))
        .on_hover_text(tr(
            "開口・視野・波長・物体距離・テレセントリック・エイミング・アフォーカルを、この状態だけ変える（絞りの位置・硝材カタログは全部の状態で共通）",
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
            RichText::new(trf("状態 {} も基本の値を使う", &[&n]))
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

fn body(ui: &mut Ui, c: &mut Combo, cats: &[String]) {
    let weak = theme::weak_text(ui.visuals());
    let id = ui.id().with("sysset");
    // 開口
    egui::Grid::new(id.with("ap"))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label(tr("開口の種類"));
            let before = c.aperture;
            egui::ComboBox::from_id_salt(id.with("apk"))
                .selected_text(c.aperture.label())
                .show_ui(ui, |ui| {
                    for k in ApKind::ALL {
                        ui.selectable_value(&mut c.aperture, k, k.label());
                    }
                });
            if c.aperture != before && c.aperture.has_value() {
                c.ap_value = c.aperture.initial();
            }
            ui.end_row();
            if c.aperture.has_value() {
                ui.label(c.aperture.label());
                ui.add(
                    egui::DragValue::new(&mut c.ap_value)
                        .speed(0.01)
                        .range(1e-6..=1e6)
                        .max_decimals(6)
                        .suffix(c.aperture.unit()),
                );
                ui.end_row();
            }
            let lead = c.lead_stop();
            if c.aperture == ApKind::Epd || lead {
                ui.label(match c.aperture {
                    ApKind::Epd => tr("入射瞳径 [mm]"),
                    ApKind::Float => tr("絞りの径 [mm]"),
                    _ => tr("絞りの径（図だけ）[mm]"),
                });
                ui.add(
                    egui::DragValue::new(&mut c.epd)
                        .speed(0.1)
                        .range(1e-3..=1e4)
                        .max_decimals(4),
                );
                ui.end_row();
            }
        });
    // 絞りの位置
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("絞り")).strong());
        match c.stop_at() {
            StopAt::Lead => {
                ui.label(tr("最初の要素の前（自動）"));
                ui.label(tr("間隔"));
                ui.add(
                    egui::DragValue::new(&mut c.stop_gap)
                        .speed(0.1)
                        .range(0.0..=1e5)
                        .suffix(" mm"),
                );
                if ui
                    .button(tr("絞りを要素にする"))
                    .on_hover_text(tr(
                        "並びの先頭に「絞り」の要素を置く。図でドラッグして好きな位置へ動かせる",
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
                ui.label(trf("{} 番目の要素「{}」", &[&(k + 1), &label]));
            }
            StopAt::Many(ks) => {
                let s: Vec<String> = ks.iter().map(|k| (k + 1).to_string()).collect();
                ui.label(
                    RichText::new(trf("{} 番目の要素（1 つだけにする）", &[&s.join(", ")]))
                        .color(Color32::RED),
                );
            }
        }
    });
    ui.label(
        RichText::new(tr(
            "絞りは要素パレットの「絞り」を並びの好きな所に置くとそこになる。置かなければ最初の要素の前に自動で置く。",
        ))
        .small()
        .color(weak),
    );
    if c.aperture == ApKind::Float && !c.lead_stop() {
        ui.label(
            RichText::new(tr("開口は絞りの要素の半径で決まる"))
                .small()
                .color(weak),
        );
    }
    ui.separator();

    // 物体・エイミング・像空間
    egui::Grid::new(id.with("obj"))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label(tr("物体"));
            ui.horizontal(|ui| {
                let mut finite = c.object_distance.is_some();
                ui.radio_value(&mut finite, false, tr("無限遠"));
                ui.radio_value(&mut finite, true, tr("有限"));
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
                    .on_hover_text(tr("物体面から最初の要素（前の絞り）まで"));
                }
            });
            ui.end_row();
            if c.object_distance.is_some() {
                ui.label("");
                ui.checkbox(&mut c.telecentric, tr("物体側テレセントリック"));
                ui.end_row();
            }
            ui.label(tr("光線エイミング"));
            ui.horizontal(|ui| {
                for a in Aiming::ALL {
                    ui.radio_value(&mut c.aiming, a, a.label());
                }
            });
            ui.end_row();
            ui.label(tr("像空間"));
            ui.horizontal(|ui| {
                let mut on = c.afocal.is_some();
                if ui.checkbox(&mut on, tr("アフォーカル")).changed() {
                    c.afocal = on.then(|| AfocalUnit::default().code().to_ascii_lowercase());
                }
                if let Some(code) = c.afocal.as_mut() {
                    let mut u = AfocalUnit::from_code(code).unwrap_or_default();
                    egui::ComboBox::from_id_salt(id.with("afu"))
                        .selected_text(u.label())
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for x in AFOCAL_UNITS {
                                ui.selectable_value(&mut u, x, x.label());
                            }
                        });
                    *code = u.code().to_ascii_lowercase();
                }
            });
            ui.end_row();
            ui.label("");
            ui.checkbox(
                &mut c.floating_clip,
                tr("半直径を固定した面の外の光線をける"),
            )
            .on_hover_text(tr(FLOATING_CLIP_HINT));
            ui.end_row();
        });
    ui.separator();

    // 視野
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("視野")).strong());
        egui::ComboBox::from_id_salt(id.with("fk"))
            .selected_text(c.field_kind.label())
            .show_ui(ui, |ui| {
                for k in FieldKind::ALL {
                    ui.selectable_value(&mut c.field_kind, k, k.label());
                }
            });
    });
    if c.field_kind == FieldKind::Solve {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(id.with("sc"))
                .selected_text(c.field_solve.cond.label())
                .show_ui(ui, |ui| {
                    for k in SolveCond::ALL {
                        ui.selectable_value(&mut c.field_solve.cond, k, k.label());
                    }
                });
            let two = c.field_solve.cond != SolveCond::Position;
            ui.label(if two { tr("面 1") } else { tr("面") });
            ui.add(
                egui::TextEdit::singleline(&mut c.field_solve.s1)
                    .hint_text(tr("番号か名前"))
                    .desired_width(70.0),
            );
            if two {
                ui.label(tr("面 2"));
                ui.add(
                    egui::TextEdit::singleline(&mut c.field_solve.s2)
                        .hint_text(tr("番号か名前"))
                        .desired_width(70.0),
                );
            }
        });
        ui.label(
            RichText::new(tr(
                "主波長の主光線が条件の値（下の x・y）になる物体を解く。.zmx には書けない",
            ))
            .small()
            .color(weak),
        );
    }
    let unit = c.field_kind.unit();
    let mut del = None;
    egui::Grid::new(id.with("fields"))
        .num_columns(5)
        .striped(true)
        .spacing([6.0, 2.0])
        .show(ui, |ui| {
            ui.label(RichText::new("#").color(weak));
            ui.label(RichText::new("x").color(weak));
            ui.label(RichText::new("y").color(weak));
            ui.label(RichText::new(tr("重み")).color(weak));
            ui.label("");
            ui.end_row();
            for (i, f) in c.fields.iter_mut().enumerate() {
                ui.label((i + 1).to_string());
                ui.add(
                    egui::DragValue::new(&mut f.x)
                        .speed(0.05)
                        .max_decimals(6)
                        .suffix(unit),
                );
                ui.add(
                    egui::DragValue::new(&mut f.y)
                        .speed(0.05)
                        .max_decimals(6)
                        .suffix(unit),
                );
                ui.add(
                    egui::DragValue::new(&mut f.w)
                        .speed(0.05)
                        .range(0.0..=1e6)
                        .max_decimals(4),
                );
                if ui.small_button("✖").on_hover_text(tr("外す")).clicked() {
                    del = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = del {
        c.fields.remove(i);
    }
    if ui.small_button(tr("＋ 視野を足す")).clicked() {
        let next = c
            .fields
            .last()
            .map_or(FieldPt::y(0.0), |f| FieldPt { y: f.y + 1.0, ..*f });
        c.fields.push(next);
    }
    ui.separator();

    // 波長
    ui.label(RichText::new(tr("波長")).strong());
    let mut del = None;
    egui::Grid::new(id.with("waves"))
        .num_columns(5)
        .striped(true)
        .spacing([6.0, 2.0])
        .show(ui, |ui| {
            ui.label(RichText::new("#").color(weak));
            ui.label(RichText::new(tr("波長 [µm]")).color(weak));
            ui.label(RichText::new(tr("重み")).color(weak));
            ui.label(RichText::new(tr("主")).color(weak));
            ui.label("");
            ui.end_row();
            for (i, w) in c.waves.iter_mut().enumerate() {
                ui.label((i + 1).to_string());
                ui.add(
                    egui::DragValue::new(&mut w.um)
                        .speed(0.001)
                        .range(0.1..=30.0)
                        .max_decimals(7),
                );
                ui.add(
                    egui::DragValue::new(&mut w.w)
                        .speed(0.05)
                        .range(0.0..=1e6)
                        .max_decimals(4),
                );
                ui.radio_value(&mut c.primary, i + 1, "");
                if ui.small_button("✖").on_hover_text(tr("外す")).clicked() {
                    del = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = del {
        c.waves.remove(i);
        if c.primary > i + 1 || c.primary > c.waves.len() {
            c.primary = c.primary.saturating_sub(1).max(1);
        }
    }
    ui.horizontal_wrapped(|ui| {
        if ui.small_button(tr("＋ 波長を足す")).clicked() {
            let next = c.waves.last().map_or(0.5875618, |w| w.um + 0.05);
            c.waves.push(WavePt::um(next));
        }
        ui.label(RichText::new(tr("よく使う組:")).small().color(weak));
        for (name, set) in PRESETS {
            if ui.small_button(*name).clicked() {
                c.waves = set.iter().map(|&u| WavePt::um(u)).collect();
                c.primary = if set.len() == 3 { 2 } else { 1 };
            }
        }
    });
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
        tr("膜（フィルタ）の定義").to_string()
    } else {
        format!("{} ({})", tr("膜（フィルタ）の定義"), c.coatings.len())
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
        ui.label(RichText::new(tr("硝材カタログ")).strong());
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
                tr("硝材のフォルダに無い")
            } else {
                tr("前ほど優先")
            });
            if i > 0 && ui.small_button("◀").on_hover_text(tr("前へ")).clicked() {
                up = Some(i);
            }
            if ui.small_button("✖").on_hover_text(tr("外す")).clicked() {
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
            ui.menu_button(tr("＋ 足す"), |ui| {
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
            "自作の玉・平板・BS のガラスを引くカタログ（前ほど優先）。空なら SCHOTT。候補は設定の硝材のフォルダの AGF",
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
            base: root.clone(),
            glass_dirs: glass,
            catalog: None,
        };
        let b = lensbench_project::expand(&p, &env).unwrap();
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

    /// 古い保存（視野角 y・波長の数の並び）も読める
    #[test]
    fn old_combo_is_read() {
        let c: Combo = serde_json::from_str(
            r#"{"epd": 4, "fields": [0, 1.5], "waves": [0.55, 0.65], "primary": 1}"#,
        )
        .unwrap();
        assert_eq!(c.fields, vec![FieldPt::y(0.0), FieldPt::y(1.5)]);
        assert_eq!(c.waves, vec![WavePt::um(0.55), WavePt::um(0.65)]);
        assert_eq!(c.aperture, ApKind::Epd);
        assert_eq!(c.object_distance, None);
        // 書いて読み直すと同じ
        let back: Combo = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn settings_go_to_the_project() {
        let mut c = Combo {
            aperture: ApKind::Fnum,
            ap_value: 4.0,
            field_kind: FieldKind::ObjectHeight,
            fields: vec![FieldPt {
                x: 1.0,
                y: 2.0,
                w: 0.5,
            }],
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
            .any(|s| s.contains("テレセントリック")));
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
