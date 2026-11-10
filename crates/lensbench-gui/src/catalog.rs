//! カタログブラウザ: ガラスとカタログレンズを探し、絞り込み、レンズを組み合わせて系にする。
//!
//! 一覧は serve の `glasses`・`lenses` で取る。組み合わせたレンズはプロジェクトファイル（ブロックで組む系、
//! `"type": "lens"` の `source` に vendor・part）にして `load` で読む。系の中の面を直す編集は
//! 系の編集の上に作る。

use egui::{Color32, RichText, Sense, Ui};
use egui_extras::{Column, Size, StripBuilder, TableBuilder};
use serde_json::{json, Value};

use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GlassRow {
    pub name: String,
    pub catalog: String,
    pub nd: f64,
    pub vd: f64,
    pub range: Option<[f64; 2]>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LensRow {
    /// 最初の玉の前・後ろの面の曲率 [1/mm]（カタログに無ければ None）
    pub curv: Option<[f64; 2]>,
    pub vendor: String,
    pub part: String,
    pub efl: f64,
    pub diameter: f64,
    pub thickness: f64,
    pub glasses: Vec<String>,
    pub error: Option<String>,
    /// 玉の無い鏡（曲率半径・コーニック。0 は平面）
    pub mirror: Option<(f64, f64)>,
    /// ガラスの枚数（貼り合わせは 2 枚以上、鏡は 0）
    pub pieces: usize,
    /// 品の種類
    pub kind: RowKind,
    /// lensbench 形式のカタログの品: カタログのファイル（絶対パス）
    pub source: Option<String>,
    /// 鏡・BS の膜の名前
    pub coat: String,
    /// BS の中身（serve の `splitter`: form・size・glass・ratio・ar など）
    pub splitter: Option<Value>,
    /// 品の種類（serve の `category`: lens・mirror・splitter・polarizer・waveplate・window・filter・prism・other）
    pub category: String,
    /// 製品ページ
    pub url: String,
    /// 説明
    pub description: String,
    /// 仕様の出典・確かめた日
    pub spec_source: String,
    pub checked: String,
    /// serve の行のまま（仕様 `specs`・タグ `tags` と、仕様での絞り込みに使う）
    pub raw: Value,
}

/// 品の種類の選択肢（serve の `category`、表示名）
pub const CATEGORIES: &[(&str, &str)] = &[
    ("lens", "レンズ"),
    ("mirror", "鏡"),
    ("splitter", "BS"),
    ("polarizer", "偏光子"),
    ("waveplate", "波長板"),
    ("window", "ウィンドウ"),
    ("filter", "フィルター"),
    ("prism", "プリズム"),
    ("tube", "チューブ"),
    ("cage_plate", "ケージ板"),
    ("mount", "マウント"),
    ("rod", "ロッド"),
    ("adapter", "アダプタ"),
    ("other", "その他"),
];

/// 仕様の値を短く（範囲は a–b、単位つき）
pub fn spec_text(name: &str, v: &Value) -> String {
    let unit = lensbench_project::catalog_spec::spec_unit(name);
    let n = |x: &Value| match x {
        Value::Number(_) => fmt_num(x.as_f64().unwrap_or(f64::NAN)),
        Value::String(t) => t.clone(),
        o => o.to_string(),
    };
    let body = match v {
        Value::Array(a) if a.len() == 2 && a.iter().all(Value::is_number) => {
            format!("{}–{}", n(&a[0]), n(&a[1]))
        }
        Value::Array(a) => a.iter().map(n).collect::<Vec<_>>().join(", "),
        o => n(o),
    };
    if unit.is_empty() || !v.is_number() && !v.is_array() {
        body
    } else {
        format!("{body} {unit}")
    }
}

impl LensRow {
    /// 仕様（名前と値）
    /// 未完の品（厚さ・径が足りず、補うまで配置できない）か
    pub fn is_incomplete(&self) -> bool {
        self.raw["incomplete"] == true
    }
    /// 未完の品の足りない値の名前（thickness・diameter）
    pub fn missing(&self) -> Vec<String> {
        self.raw["missing"]
            .as_array()
            .map(|a| a.iter().map(s).collect())
            .unwrap_or_default()
    }
    /// 足りない値の表示名（「厚さ・径」）
    pub fn missing_text(&self) -> String {
        self.missing()
            .iter()
            .map(|m| match m.as_str() {
                "thickness" => tr("厚さ"),
                "diameter" => tr("径"),
                x => x,
            })
            .collect::<Vec<_>>()
            .join(tr("・"))
    }
    /// 置けない理由（誤り・未完の品）
    pub fn placeable(&self) -> bool {
        self.error.is_none() && !self.is_incomplete()
    }
    pub fn specs(&self) -> Vec<(&str, &Value)> {
        self.raw["specs"]
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.as_str(), v)).collect())
            .unwrap_or_default()
    }
    /// 表の「仕様」の列: 種類ごとに要の仕様を先に、ほかは後ろ
    pub fn spec_summary(&self) -> String {
        let first: &[&str] = match self.category.as_str() {
            "splitter" => &[
                "split_ratio",
                "extinction_ratio",
                "wavelength_nm",
                "coating",
            ],
            "mirror" => &["coating", "reflectance", "wavelength_nm", "flatness"],
            "polarizer" => &["extinction_ratio", "wavelength_nm", "transmittance"],
            "waveplate" => &["retardance", "wavelength_nm", "order"],
            "window" => &["material", "wavelength_nm", "coating", "wedge"],
            _ => &["coating", "wavelength_nm"],
        };
        let specs = self.specs();
        let mut out: Vec<String> = Vec::new();
        for k in first {
            if let Some((_, v)) = specs.iter().find(|(n, _)| n == k) {
                out.push(spec_text(k, v));
            }
        }
        for (k, v) in &specs {
            if !first.contains(k) && !matches!(*k, "diameter" | "thickness" | "efl") {
                out.push(format!("{k} {}", spec_text(k, v)));
            }
        }
        out.join("、")
    }
}

/// カタログの品の種類
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RowKind {
    #[default]
    Lens,
    Mirror,
    Splitter,
    /// 機構部品（チューブ・ケージ板など）。組合せの「機構部品」に足す
    Mech,
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// serve の `glasses` の結果
pub fn parse_glasses(v: &Value) -> Vec<GlassRow> {
    v["glasses"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|g| GlassRow {
                    name: s(&g["name"]),
                    catalog: s(&g["catalog"]),
                    nd: f(&g["nd"]),
                    vd: f(&g["vd"]),
                    range: g["range_um"]
                        .as_array()
                        .filter(|r| r.len() == 2)
                        .map(|r| [f(&r[0]), f(&r[1])]),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// serve の `lenses` の結果
pub fn parse_lenses(v: &Value) -> Vec<LensRow> {
    v["lenses"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|l| LensRow {
                    curv: l["curv"].as_array().and_then(|a| match a.as_slice() {
                        [a, b] => Some([a.as_f64()?, b.as_f64()?]),
                        _ => None,
                    }),
                    vendor: s(&l["vendor"]),
                    part: s(&l["part"]),
                    efl: f(&l["efl"]),
                    diameter: f(&l["diameter"]),
                    thickness: f(&l["thickness"]),
                    glasses: l["glasses"]
                        .as_array()
                        .map(|g| g.iter().map(s).collect())
                        .unwrap_or_default(),
                    error: l["error"].as_str().map(str::to_string),
                    pieces: l["pieces"].as_u64().unwrap_or(0) as usize,
                    mirror: (l["mirror"] == true && l["radius"].is_number()).then(|| {
                        (
                            l["radius"].as_f64().unwrap_or(0.0),
                            l["conic"].as_f64().unwrap_or(0.0),
                        )
                    }),
                    kind: match l["kind"].as_str() {
                        Some("splitter") => RowKind::Splitter,
                        Some("mirror") => RowKind::Mirror,
                        Some("mech") => RowKind::Mech,
                        _ if l["mirror"] == true && l["radius"].is_number() => RowKind::Mirror,
                        _ => RowKind::Lens,
                    },
                    source: l["source"]["catalog"].as_str().map(str::to_string),
                    coat: s(&l["coat"]),
                    splitter: l["splitter"].is_object().then(|| l["splitter"].clone()),
                    category: l["category"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            lensbench_project::catalog_spec::row_category(l).to_string()
                        }),
                    url: s(&l["url"]),
                    description: s(&l["description"]),
                    spec_source: s(&l["spec_source"]),
                    checked: s(&l["checked"]),
                    raw: l.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 製品ページを開くボタン（既定のブラウザ。右クリックでリンクを写す）
fn link_button(ui: &mut Ui, url: &str) {
    if url.is_empty() {
        return;
    }
    let b = ui.button(tr("🔗 製品ページ")).on_hover_text(url);
    if b.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
    b.context_menu(|ui| {
        if ui.button(tr("リンクを写す")).clicked() {
            ui.ctx().copy_text(url.to_string());
            ui.close_menu();
        }
    });
}

/// 品の説明・仕様・出典の表
fn spec_table(ui: &mut Ui, r: &LensRow) {
    ui.label(RichText::new(format!("{} {}", r.vendor, r.part)).strong());
    if !r.description.is_empty() {
        ui.label(&r.description);
    }
    egui::Grid::new(("specs", &r.vendor, &r.part))
        .striped(true)
        .show(ui, |ui| {
            for (k, v) in r.specs() {
                ui.label(k);
                ui.label(spec_text(k, v));
                ui.end_row();
            }
        });
    if let Some(t) = r.raw["tags"].as_array().filter(|a| !a.is_empty()) {
        let t: Vec<&str> = t.iter().filter_map(Value::as_str).collect();
        ui.label(trf("タグ {}", &[&t.join(", ")]));
    }
    // 枠付きの品: 置くと枠が機構部品として付く
    if let Some(f) = r.raw.get("frame").filter(|f| f.is_object()) {
        let num = |v: &Value| v.as_f64().map_or("—".to_string(), fmt_num);
        let clear = f["bores"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|b| b["diameter"].as_f64())
                    .fold(f64::INFINITY, f64::min)
            })
            .filter(|x| x.is_finite());
        ui.label(trf(
            "枠付き: 枠の長さ {} mm、外径 {} mm、最小の内径 {} mm（置くと機構部品として付きます）",
            &[
                &num(&f["length"]),
                &num(&f["outer_diameter"]),
                &clear.map_or("—".to_string(), fmt_num),
            ],
        ));
    }
    if !r.spec_source.is_empty() || !r.checked.is_empty() {
        ui.weak(trf("出典 {}（{}）", &[&r.spec_source, &r.checked]));
    }
    if !r.url.is_empty() {
        ui.hyperlink_to(tr("製品ページ"), &r.url);
    }
}

/// 下限・上限（None は制限なし）。NaN の値は制限があれば外す
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Range {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl Range {
    pub fn contains(&self, x: f64) -> bool {
        if self.min.is_none() && self.max.is_none() {
            return true;
        }
        !x.is_nan() && self.min.map_or(true, |m| x >= m) && self.max.map_or(true, |m| x <= m)
    }
}

/// 文字列の絞り込み: 空白で区切った語がすべて（大文字・小文字を区別せず）どこかに含まれる
fn text_match(query: &str, fields: &[&str]) -> bool {
    let hay: Vec<String> = fields.iter().map(|x| x.to_lowercase()).collect();
    query.split_whitespace().all(|w| {
        let w = w.to_lowercase();
        hay.iter().any(|h| h.contains(&w))
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LensFilter {
    pub text: String,
    /// 空ならすべてのメーカー
    pub vendor: String,
    pub efl: Range,
    pub diameter: Range,
    /// 正のレンズだけ・負のレンズだけ
    pub sign: Sign,
    /// ガラスの枚数（0 はすべて、3 は 3 枚以上）
    pub pieces: usize,
    /// 種類（None はすべて）
    pub kind: Option<RowKind>,
    /// 品の種類（serve の `category`、None はすべて）
    pub category: Option<String>,
    /// 仕様の条件（空白で区切る。CLI catalog-search --spec と同じ書き方: `wavelength_nm=633`・`extinction_ratio>=1000`）
    pub spec: String,
}

/// 仕様の条件を読む（読めない条件は誤りの文に）
pub fn parse_spec_filters(
    q: &str,
) -> Result<Vec<lensbench_project::catalog_spec::SpecFilter>, String> {
    q.split_whitespace()
        .map(lensbench_project::catalog_spec::SpecFilter::parse)
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Sign {
    #[default]
    Any,
    Positive,
    Negative,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LensSort {
    #[default]
    Vendor,
    Part,
    Efl,
    Diameter,
    Thickness,
}

pub fn filter_lenses(rows: &[LensRow], q: &LensFilter, sort: LensSort, desc: bool) -> Vec<usize> {
    // 読めない仕様の条件は無視する（欄の横に誤りを出す）
    let specs = parse_spec_filters(&q.spec).unwrap_or_default();
    let mut v: Vec<usize> = (0..rows.len())
        .filter(|&i| {
            let r = &rows[i];
            let g = r.glasses.join(" ");
            let tags = r.raw["tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let sp = r
                .specs()
                .iter()
                .map(|(_, v)| spec_text("", v))
                .collect::<Vec<_>>()
                .join(" ");
            (q.vendor.is_empty() || r.vendor == q.vendor)
                && text_match(
                    &q.text,
                    &[&r.vendor, &r.part, &g, &r.description, &tags, &sp],
                )
                && q.category.as_ref().map_or(true, |c| &r.category == c)
                && specs.iter().all(|f| f.matches(&r.raw))
                && q.efl.contains(r.efl)
                && q.diameter.contains(r.diameter)
                && match q.sign {
                    Sign::Any => true,
                    Sign::Positive => r.efl > 0.0,
                    Sign::Negative => r.efl < 0.0,
                }
                && match q.pieces {
                    0 => true,
                    3 => r.pieces >= 3,
                    n => r.pieces == n,
                }
                && q.kind.map_or(true, |k| r.kind == k)
        })
        .collect();
    let key = |i: &usize| {
        let r = &rows[*i];
        match sort {
            LensSort::Vendor | LensSort::Part => 0.0,
            LensSort::Efl => r.efl,
            LensSort::Diameter => r.diameter,
            LensSort::Thickness => r.thickness,
        }
    };
    v.sort_by(|a, b| {
        let o = match sort {
            LensSort::Vendor => {
                (&rows[*a].vendor, &rows[*a].part).cmp(&(&rows[*b].vendor, &rows[*b].part))
            }
            LensSort::Part => rows[*a].part.cmp(&rows[*b].part),
            // NaN は最後
            _ => key(a).total_cmp(&key(b)),
        };
        if desc {
            o.reverse()
        } else {
            o
        }
    });
    v
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GlassFilter {
    pub text: String,
    pub catalog: String,
    pub nd: Range,
    pub vd: Range,
}

pub fn filter_glasses(rows: &[GlassRow], q: &GlassFilter) -> Vec<usize> {
    (0..rows.len())
        .filter(|&i| {
            let r = &rows[i];
            (q.catalog.is_empty() || r.catalog == q.catalog)
                && text_match(&q.text, &[&r.name, &r.catalog])
                && q.nd.contains(r.nd)
                && q.vd.contains(r.vd)
        })
        .collect()
}

pub use crate::combo::{Combo, Pick};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Lenses,
    Glasses,
}

/// ブラウザの操作で GUI に頼むこと
pub enum Action {
    /// 一覧を読み直す
    Refresh,
    /// 組合せを系にして読む
    Build(Value),
    /// 設定（フォルダ）を開く
    Settings,
    /// 系の設定のパネルを開く
    SysSettings,
}

#[derive(Default)]
pub struct Browser {
    pub tab: Tab,
    pub glasses: Vec<GlassRow>,
    pub lenses: Vec<LensRow>,
    pub glass_q: GlassFilter,
    pub lens_q: LensFilter,
    pub lens_sort: LensSort,
    pub lens_desc: bool,
    pub lens_sel: Option<usize>,
    pub glass_sel: Option<usize>,

    /// ガラスマップの見えている範囲
    pub glass_view: MapView,
    pub combo: Combo,
    /// 一覧を一度でも読んだか
    pub loaded: bool,
    /// 読み込み中の一覧（頼んで返事を待っている。0 件と見分ける）
    pub lenses_pending: bool,
    pub glasses_pending: bool,
    /// 足す玉をこの要素の直後へ（組合せの図で選んでいる要素。無ければ後ろ）
    pub insert_after: Option<usize>,
    /// 足した玉の番号（図で選ぶ）
    pub added: Option<usize>,
    /// 選んだ玉の断面（行の番号と serve の layout）
    pub preview: Option<(usize, crate::combo::LayoutData)>,
    /// 補う窓で開く未完の品（足そうとした行）
    pub fill_req: Option<LensRow>,
    /// 一覧で隠す列（[`LensCol::key`]、設定に残す）
    pub lens_hide: Vec<String>,
    /// 品の STEP の取得（選んだ品の 1 件だけ）
    pub cad: crate::cad_fetch::CadFetcher,
}

/// カタログレンズの一覧の列
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LensCol {
    Vendor,
    Part,
    Efl,
    Diameter,
    Thickness,
    Glass,
    Spec,
}

impl LensCol {
    pub const ALL: [LensCol; 7] = [
        LensCol::Vendor,
        LensCol::Part,
        LensCol::Efl,
        LensCol::Diameter,
        LensCol::Thickness,
        LensCol::Glass,
        LensCol::Spec,
    ];

    /// 設定に書く名前
    pub fn key(self) -> &'static str {
        match self {
            LensCol::Vendor => "vendor",
            LensCol::Part => "part",
            LensCol::Efl => "efl",
            LensCol::Diameter => "diameter",
            LensCol::Thickness => "thickness",
            LensCol::Glass => "glass",
            LensCol::Spec => "spec",
        }
    }

    fn label(self) -> &'static str {
        match self {
            LensCol::Vendor => tr("メーカー"),
            LensCol::Part => tr("型番"),
            LensCol::Efl => tr("焦点距離"),
            LensCol::Diameter => tr("径"),
            LensCol::Thickness => tr("厚さ"),
            LensCol::Glass => tr("ガラス"),
            LensCol::Spec => tr("仕様"),
        }
    }

    fn sort(self) -> Option<LensSort> {
        match self {
            LensCol::Vendor => Some(LensSort::Vendor),
            LensCol::Part => Some(LensSort::Part),
            LensCol::Efl => Some(LensSort::Efl),
            LensCol::Diameter => Some(LensSort::Diameter),
            LensCol::Thickness => Some(LensSort::Thickness),
            LensCol::Glass | LensCol::Spec => None,
        }
    }

    /// 列の幅（初め・最小）
    fn column(self) -> Column {
        let c = match self {
            LensCol::Vendor => Column::initial(110.0),
            LensCol::Part => Column::initial(120.0),
            LensCol::Efl => Column::initial(70.0),
            LensCol::Diameter | LensCol::Thickness => Column::initial(60.0),
            LensCol::Glass => Column::initial(130.0),
            LensCol::Spec => Column::initial(220.0),
        };
        c.at_least(36.0).clip(true)
    }
}

/// 一覧に出す列（型番は必ず出す）
pub fn shown_cols(hide: &[String]) -> Vec<LensCol> {
    LensCol::ALL
        .into_iter()
        .filter(|c| *c == LensCol::Part || !hide.iter().any(|h| h == c.key()))
        .collect()
}

/// プレビューで向きを変える負の単レンズか（貼り合わせ・多枚玉は無限共役を前側に想定した設計が多いので今の向き）
fn negative_singlet(r: &LensRow) -> bool {
    r.kind == RowKind::Lens
        && r.mirror.is_none()
        && r.pieces == 1
        && r.efl.is_finite()
        && r.efl < -1e-3
}

/// 負の単レンズの断面: 玉を反転するか（曲率の強い面＝無限共役の側を後ろへ）と、前側焦点の位置（前の面の頂点から後ろへ [mm]）。
/// ガラスの屈折率は分からないので 1.55 とみた厚肉の近軸の式（向きの分かる図にするだけ）。曲率が無ければ主点を玉の中ほどとみる
fn negative_preview_focus(efl: f64, t: f64, curv: Option<[f64; 2]>) -> (bool, f64) {
    let Some([c1, c2]) = curv else {
        return (false, efl.abs() + 0.5 * t);
    };
    // 前の面の方が強く曲がっていれば反転（反転すると面の順と曲率の符号が入れ替わる）
    let reversed = c1.abs() > c2.abs() + 1e-9;
    let c_back = if reversed { -c1 } else { c2 };
    let n = 1.55;
    let phi = 1.0 / efl;
    let phi2 = (1.0 - n) * c_back;
    // 前側焦点: 頂点から z_F = -(1 - (t/n)·φ2)/φ（φ < 0 なので後ろ）
    let z = -(1.0 - t / n * phi2) / phi;
    (
        reversed,
        if z.is_finite() && z > 0.0 {
            z
        } else {
            efl.abs() + 0.5 * t
        },
    )
}

/// カタログの玉 1 枚の断面を見るためのプロジェクト（玉の径の 6 割の平行光、像面は焦点距離の所）
pub fn preview_project(r: &LensRow) -> serde_json::Value {
    // 機構部品: 絞りの後に部品を置き、外径の 45 % の平行光を通す
    if r.kind == RowKind::Mech {
        // diameter は外径。光束は外径の 45 %（内径を少しはみ出すと切れて見える）
        let outer = if r.diameter.is_finite() && r.diameter > 0.0 {
            r.diameter
        } else {
            10.0
        };
        let len = if r.thickness.is_finite() {
            r.thickness.max(1.0)
        } else {
            10.0
        };
        let mut v = json!({
            "name": format!("{} {}", r.vendor, r.part),
            "wavelengths": [{"um": 0.5876}],
            "aperture": {"type": "epd", "value": outer * 0.45},
            "fields": {"type": "angle", "points": [[0, 0]]},
            "blocks": [{"type": "stop"}, {"type": "air", "t": len + 10.0}],
        });
        if let Some(src) = &r.source {
            v["mechanics"] = json!([{ "catalog": src, "item": r.part, "at": {"surface": 1}, "offset": [0, 0, 5] }]);
        }
        return v;
    }
    let mut p = pick_from(r);
    p.air = if r.efl.is_finite() && r.efl.abs() > 1e-3 {
        r.efl.abs()
    } else {
        20.0
    };
    if negative_singlet(r) {
        // 負の単レンズ: 無限共役の側（曲率の強い面）を後ろに向け、絞りは前。前側焦点（玉の後ろ）へ集まる光を
        // 入れて（物体は負の距離）、平行光で出す
        let f = r.efl.abs();
        let t = if r.thickness.is_finite() {
            r.thickness.max(0.0)
        } else {
            0.0
        };
        let (reversed, z) = negative_preview_focus(r.efl, t, r.curv);
        p.reversed = reversed != p.reversed;
        p.air = f;
        let stop_gap = 2.0;
        let c = Combo {
            name: format!("{} {}", r.vendor, r.part),
            epd: (r.diameter * 0.6).max(0.5),
            stop_gap,
            object_distance: Some(-(stop_gap + z)),
            picks: vec![p],
            ..Default::default()
        };
        return c.project();
    }
    // 傾けた素子（45° の鏡など）は光軸方向にも広がるので、絞りを素子の手前の端より前に置き、
    // 像面も素子の端より外に置く（絞りが素子に重なり、光線が絞りより前で折れて見えるのを避ける）
    let tilt = p.tilt[0].abs().max(p.tilt[1].abs()).min(89.0).to_radians();
    let half = if p.semi.is_finite() {
        p.semi.max(0.0)
    } else {
        0.0
    };
    let stop_gap = 2.0 + half * tilt.sin();
    if tilt > 0.0 {
        p.air = p.air.max(half * tilt.cos() + 10.0);
    }
    let c = Combo {
        name: format!("{} {}", r.vendor, r.part),
        epd: (r.diameter * 0.6).max(0.5),
        stop_gap,
        picks: vec![p],
        ..Default::default()
    };
    c.project()
}

/// 機構部品の置く要素・保持する要素の id が今の要素に無いか（空は面 1・保持なしで、無いとは言わない）
fn stale_id(id: &str, ids: &[String]) -> bool {
    !id.is_empty() && !ids.iter().any(|x| x == id)
}

/// 組合せの機構部品: 置く要素・面・ずれ・保持する要素。部品はカタログの機構部品の行から足す
fn mech_ui(ui: &mut Ui, ms: &mut Vec<crate::combo::MechPick>, ids: &[String]) {
    ui.label(RichText::new(tr("機構部品")).strong()).on_hover_text(tr(
        "チューブ・ケージ板など。カタログの機構部品の行を足すと、選んでいる要素に置きます。ケラレは解析の「機構のケラレ」で見ます",
    ));
    let mut remove = None;
    let names: Vec<String> = ms.iter().map(|m| m.name.clone()).collect();
    egui::Grid::new("mechs")
        .striped(true)
        .num_columns(6)
        .show(ui, |ui| {
            for (k, m) in ms.iter_mut().enumerate() {
                ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(90.0))
                    .on_hover_text(&m.item);
                let none = tr("（面 1）").to_string();
                // 消した・名前の変わった要素は ⚠ を付けて選び直してもらう
                let show = |id: &str| {
                    if stale_id(id, ids) {
                        format!("⚠ {id}")
                    } else {
                        id.to_string()
                    }
                };
                let gone = tr("要素が無い（消したか名前が変わった）。選び直してください");
                // 置く所: 要素か、前の機構部品の後ろ（つなぐ）
                let after_text = |n: &str| trf("{} の後ろにつなぐ", &[&n]);
                let prev = &names[..k];
                let stale_after = !m.after.is_empty() && !prev.contains(&m.after);
                egui::ComboBox::from_id_salt(("mech-at", k))
                    .selected_text(if !m.after.is_empty() {
                        if stale_after {
                            format!("⚠ {}", after_text(&m.after))
                        } else {
                            after_text(&m.after)
                        }
                    } else if m.at.is_empty() {
                        none.clone()
                    } else {
                        show(&m.at)
                    })
                    .show_ui(ui, |ui| {
                        let mut pick = |ui: &mut Ui, at: &str, after: &str, label: String| {
                            let sel = m.at == at && m.after == after;
                            if ui.selectable_label(sel, label).clicked() {
                                m.at = at.to_string();
                                m.after = after.to_string();
                                if !after.is_empty() {
                                    m.seat = false;
                                }
                            }
                        };
                        pick(ui, "", "", none.clone());
                        for id in ids {
                            pick(ui, id, "", id.clone());
                        }
                        for n in prev {
                            pick(ui, "", n, after_text(n));
                        }
                    })
                    .response
                    .on_hover_text(if stale_id(&m.at, ids) || stale_after {
                        gone
                    } else if !m.after.is_empty() {
                        tr("前の部品の後ろの端につなぎます。両方の端にねじがあれば、合うか確かめ、オスのねじの分だけ重ねます")
                    } else {
                        tr("置く要素")
                    });
                ui.horizontal(|ui| {
                    ui.label(tr("面"));
                    ui.add(egui::DragValue::new(&mut m.face).range(0..=99))
                        .on_hover_text(tr("要素の何番目の面に置くか（0 始まり）"));
                    ui.label("z");
                    ui.add(
                        egui::DragValue::new(&mut m.offset[2])
                            .speed(0.1)
                            .suffix(" mm"),
                    )
                    .on_hover_text(tr("置いた面から光の進む向きへのずれ"));
                });
                let held = m.holds.first().cloned().unwrap_or_default();
                let mut sel = held.clone();
                egui::ComboBox::from_id_salt(("mech-holds", k))
                    .selected_text(if sel.is_empty() {
                        tr("保持なし").to_string()
                    } else {
                        show(&sel)
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut sel, String::new(), tr("保持なし"));
                        for id in ids {
                            ui.selectable_value(&mut sel, id.clone(), id);
                        }
                    })
                    .response
                    .on_hover_text(if stale_id(&held, ids) {
                        gone
                    } else {
                        tr("部品が保持する要素（はみ出し・保持径を確かめる）")
                    });
                if sel != held {
                    m.holds = if sel.is_empty() {
                        Vec::new()
                    } else {
                        vec![sel]
                    };
                }
                ui.vertical(|ui| {
                    ui.add_enabled(!m.holds.is_empty() && m.after.is_empty(), egui::Checkbox::new(&mut m.seat, tr("座に合わせる")))
                        .on_hover_text(tr("部品の座（保持する素子を当てる面）を、保持する要素の前面に合わせて置きます（置く要素・面の代わり。z はそこからのずれ）"));
                    ui.checkbox(&mut m.reversed, tr("裏返す"))
                        .on_hover_text(tr("部品を裏返して置きます（光が部品の後ろの端から入る。穴・座・両端のねじも入れ替わる）"));
                });
                if ui.button("✖").on_hover_text(tr("外す")).clicked() {
                    remove = Some(k);
                }
                ui.end_row();
            }
        });
    if let Some(k) = remove {
        ms.remove(k);
    }
}

/// 範囲の入力（チェックで有効にする）
fn range_ui(ui: &mut Ui, label: &str, r: &mut Range, default: (f64, f64), speed: f64) {
    ui.label(label);
    let mut on = r.min.is_some();
    if ui.checkbox(&mut on, tr("下限")).changed() {
        r.min = on.then_some(default.0);
    }
    if let Some(m) = r.min.as_mut() {
        ui.add(egui::DragValue::new(m).speed(speed));
    }
    let mut on = r.max.is_some();
    if ui.checkbox(&mut on, tr("上限")).changed() {
        r.max = on.then_some(default.1);
    }
    if let Some(m) = r.max.as_mut() {
        ui.add(egui::DragValue::new(m).speed(speed));
    }
}

/// 一覧の 1 マス
fn lens_cell(ui: &mut Ui, c: LensCol, r: &LensRow) {
    // 切れた文字はマウスを載せると全体が出る（egui の Label が切ったときに出す）
    let text = |ui: &mut Ui, t: &str| {
        ui.label(t);
    };
    match c {
        LensCol::Vendor => text(ui, &r.vendor),
        LensCol::Part => text(ui, &r.part),
        LensCol::Efl => num_cell(ui, r.efl),
        LensCol::Diameter => num_cell(ui, r.diameter),
        LensCol::Thickness => num_cell(ui, r.thickness),
        LensCol::Glass => {
            if let Some(e) = &r.error {
                ui.colored_label(Color32::RED, e).on_hover_text(e);
            } else if r.is_incomplete() {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    trf("⚠ 未完（{}）", &[&r.missing_text()]),
                )
                .on_hover_text(tr("ダブルクリックで品ページから補います"));
            } else if r.kind == RowKind::Splitter {
                text(ui, &format!("BS {}", r.glasses.join(" / ")));
            } else if r.mirror.is_some() {
                ui.label(tr("鏡"));
            } else {
                let k = CATEGORIES
                    .iter()
                    .find(|(c, _)| *c == r.category && !matches!(*c, "lens"));
                let t = match k {
                    Some((_, n)) if r.glasses.is_empty() => tr(n).to_string(),
                    Some((_, n)) => format!("{} {}", tr(n), r.glasses.join(" / ")),
                    None => r.glasses.join(" / "),
                };
                text(ui, &t);
            }
        }
        LensCol::Spec => {
            let t = r.spec_summary();
            if !t.is_empty() {
                ui.label(&t).on_hover_ui(|ui| spec_table(ui, r));
            }
        }
    }
}

fn num_cell(ui: &mut Ui, x: f64) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(if x.is_finite() {
            fmt_num((x * 1e3).round() / 1e3)
        } else {
            "—".into()
        });
    });
}

impl Browser {
    pub fn ui(&mut self, ui: &mut Ui, n_glass_dirs: usize, n_lens_dirs: usize) -> Option<Action> {
        let mut act = None;
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.tab,
                Tab::Lenses,
                if self.lenses_pending {
                    tr("カタログレンズ（読み込み中…）").to_string()
                } else {
                    trf("カタログレンズ（{}）", &[&self.lenses.len()])
                },
            );
            ui.selectable_value(
                &mut self.tab,
                Tab::Glasses,
                if self.glasses_pending {
                    tr("ガラス（読み込み中…）").to_string()
                } else {
                    trf("ガラス（{}）", &[&self.glasses.len()])
                },
            );
            ui.separator();
            if ui.button(tr("⟳ 読み直す")).clicked() {
                act = Some(Action::Refresh);
            }
            if ui
                .button(trf(
                    "フォルダ…（ガラス {}・レンズ {}）",
                    &[&n_glass_dirs, &n_lens_dirs],
                ))
                .clicked()
            {
                act = Some(Action::Settings);
            }
        });
        ui.separator();
        match self.tab {
            Tab::Lenses => {
                if self.lenses_pending {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr(
                            "カタログレンズを読み込んでいます（数万枚あると数十秒かかります）",
                        ));
                    });
                } else if n_lens_dirs == 0 && self.lenses.is_empty() {
                    ui.label(tr("レンズカタログのフォルダ（.zmx を入れたフォルダ。メーカーはフォルダの名前、型番はファイル名）を「フォルダ…」で足してください。"));
                }
                if let Some(a) = self.lens_ui(ui) {
                    act = Some(a);
                }
            }
            Tab::Glasses => {
                if self.glasses_pending {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr("ガラスカタログを読み込んでいます"));
                    });
                } else if n_glass_dirs == 0 && self.glasses.is_empty() {
                    ui.label(tr("ガラスカタログ（.AGF）のフォルダを「フォルダ…」で足してください（環境変数 LENSBENCH_GLASSCAT も使います）。"));
                }
                self.glass_ui(ui);
            }
        }
        act
    }

    fn lens_ui(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut act = None;
        // 右に組合せ、左に一覧（狭ければ上に一覧、下に組合せ）
        let w = ui.available_width();
        let wide = w >= 640.0;
        let builder = StripBuilder::new(ui).size(Size::remainder().at_least(160.0));
        let builder = if wide {
            builder.size(Size::exact((w * 0.38).clamp(260.0, 400.0)))
        } else {
            builder.size(Size::relative(0.45))
        };
        let mut cells = |mut strip: egui_extras::Strip| {
            strip.cell(|ui| self.lens_list(ui));
            strip.cell(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt("combo_scroll")
                    .show(ui, |ui| {
                        ui.add_space(2.0);
                        if let Some(a) = self.combo_ui(ui) {
                            act = Some(a);
                        }
                    });
            });
        };
        if wide {
            builder.horizontal(&mut cells);
        } else {
            builder.vertical(cells);
        }
        act
    }

    fn lens_list(&mut self, ui: &mut Ui) {
        let q = &mut self.lens_q;
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("検索"));
            ui.add(
                egui::TextEdit::singleline(&mut q.text)
                    .hint_text(tr("型番・メーカー・ガラス"))
                    .desired_width(160.0),
            );
            let mut vendors: Vec<&str> = self.lenses.iter().map(|l| l.vendor.as_str()).collect();
            vendors.sort();
            vendors.dedup();
            egui::ComboBox::from_id_salt("vendor")
                .selected_text(if q.vendor.is_empty() {
                    tr("すべてのメーカー")
                } else {
                    &q.vendor
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut q.vendor, String::new(), tr("すべてのメーカー"));
                    for v in vendors {
                        ui.selectable_value(&mut q.vendor, v.to_string(), v);
                    }
                });
            ui.selectable_value(&mut q.sign, Sign::Any, tr("正負とも"));
            ui.selectable_value(&mut q.sign, Sign::Positive, tr("正"));
            ui.selectable_value(&mut q.sign, Sign::Negative, tr("負"));
            ui.separator();
            ui.label(tr("枚数"));
            ui.selectable_value(&mut q.pieces, 0, tr("すべて"));
            ui.selectable_value(&mut q.pieces, 1, "1");
            ui.selectable_value(&mut q.pieces, 2, "2");
            ui.selectable_value(&mut q.pieces, 3, "3+");
            ui.separator();
            ui.label(tr("種類"));
            // 一覧にある種類だけ
            let have: Vec<&str> = CATEGORIES
                .iter()
                .map(|(k, _)| *k)
                .filter(|k| self.lenses.iter().any(|l| l.category == *k))
                .collect();
            let name = |k: &str| {
                CATEGORIES
                    .iter()
                    .find(|(c, _)| *c == k)
                    .map_or(k.to_string(), |(_, n)| tr(n).to_string())
            };
            egui::ComboBox::from_id_salt("category")
                .selected_text(q.category.as_deref().map_or(tr("すべて").to_string(), name))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut q.category, None, tr("すべて"));
                    for k in have {
                        ui.selectable_value(&mut q.category, Some(k.to_string()), name(k));
                    }
                });
        });
        ui.horizontal_wrapped(|ui| {
            range_ui(ui, tr("焦点距離 [mm]"), &mut q.efl, (10.0, 100.0), 1.0);
            ui.separator();
            range_ui(ui, tr("径 [mm]"), &mut q.diameter, (5.0, 25.4), 0.5);
            ui.separator();
            ui.label(tr("仕様"));
            ui.add(
                egui::TextEdit::singleline(&mut q.spec)
                    .hint_text("wavelength_nm=633 extinction_ratio>=1000")
                    .desired_width(220.0),
            )
            .on_hover_text(tr(
                "仕様の条件（空白で区切り、すべて満たす）: 名前（ある）、名前=値（数は一致、範囲の仕様は含む、文字列は含む）、名前=a..b、名前>=値・<=・>・<。efl・diameter・thickness は一覧の値でも比べる",
            ));
            if let Err(e) = parse_spec_filters(&q.spec) {
                ui.colored_label(Color32::RED, e);
            }
        });
        let idx = filter_lenses(&self.lenses, &self.lens_q, self.lens_sort, self.lens_desc);
        // 件数と、選んだレンズの詳細（1 行。選んでも表が動かないよう高さを決めておく）
        let picked = self.lens_sel.and_then(|i| self.lenses.get(i)).cloned();
        // 1 行に収め、長ければ横にスクロール（右の組合せ・図に重ならない）
        egui::ScrollArea::horizontal()
            .id_salt("lens_detail")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(24.0);
                    ui.label(trf("{} / {} 枚", &[&idx.len(), &self.lenses.len()]));
                    let hide = &mut self.lens_hide;
                    ui.menu_button(tr("列…"), |ui| {
                        for c in LensCol::ALL {
                            let mut on = !hide.iter().any(|h| h == c.key());
                            let resp = ui.add_enabled(
                                c != LensCol::Part,
                                egui::Checkbox::new(&mut on, c.label()),
                            );
                            if resp.changed() {
                                hide.retain(|h| h != c.key());
                                if !on {
                                    hide.push(c.key().to_string());
                                }
                            }
                        }
                    })
                    .response
                    .on_hover_text(tr("一覧に出す列を選びます"));
                    ui.separator();
                    let Some(r) = picked else {
                        ui.weak(tr("行をクリックで選び、ダブルクリックで組合せに足します"));
                        return;
                    };
                    ui.label(RichText::new(format!("{} {}", r.vendor, r.part)).strong());
                    if let Some(e) = &r.error {
                        ui.colored_label(Color32::RED, e);
                    } else if r.is_incomplete() {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            trf("未完の品: {} が無いので置けません", &[&r.missing_text()]),
                        );
                    } else if r.kind == RowKind::Splitter {
                        ui.label(splitter_detail(&r));
                    } else if r.kind == RowKind::Mech {
                        // 簡易外形（丸は外径、角は一辺）
                        let m = &r.raw["mech"];
                        let od = fmt_num(m["outer_diameter"].as_f64().unwrap_or(0.0));
                        ui.label(if m["outline"] == "square" {
                            trf("機構部品: 長さ {} mm、角（一辺 {} mm）", &[&fmt_num(r.thickness), &od])
                        } else {
                            trf("機構部品: 長さ {} mm、丸（外径 {} mm）", &[&fmt_num(r.thickness), &od])
                        });
                    } else if let Some((radius, _)) = r.mirror {
                        if radius == 0.0 {
                            ui.label(trf("平面鏡: 径 {} mm", &[&fmt_num(r.diameter)]));
                        } else {
                            ui.label(trf(
                                "鏡: R = {} mm、f = {} mm、径 {} mm",
                                &[&fmt_num(radius), &fmt_num(r.efl), &fmt_num(r.diameter)],
                            ));
                        }
                        if !r.coat.is_empty() {
                            ui.label(trf("膜 {}", &[&r.coat]));
                        }
                    } else {
                        ui.label(trf(
                            "f = {} mm、径 {} mm、厚さ {} mm、{} 枚、ガラス {}",
                            &[
                                &fmt_num(r.efl),
                                &fmt_num(r.diameter),
                                &fmt_num(r.thickness),
                                &r.pieces,
                                &r.glasses.join(" / "),
                            ],
                        ));
                    }
                    if r.is_incomplete() {
                        if ui
                            .button(tr("🌐 品ページから補う…"))
                            .on_hover_text(tr("品ページを開き、仕様表か STEP から足りない値を入れます（ダブルクリックでも）"))
                            .clicked()
                        {
                            self.add_pick(&r);
                        }
                    } else if ui.button(tr("➕ 組合せに足す")).clicked() {
                        self.add_pick(&r);
                    }
                    link_button(ui, &r.url);
                    self.cad.poll();
                    self.cad.ui(ui, &r);
                    let specs = r.specs();
                    if !specs.is_empty() || !r.description.is_empty() {
                        ui.menu_button(tr("仕様…"), |ui| spec_table(ui, &r));
                    }
                    // 素子が入る機構部品（保持径が径以上で、少しだけ太いもの）
                    if r.kind != RowKind::Mech && !r.is_incomplete() && r.diameter > 0.0 {
                        let fit: Vec<&LensRow> = self
                            .lenses
                            .iter()
                            .filter(|m| {
                                m.kind == RowKind::Mech
                                    && lensbench_project::catalog_spec::fits_row(&m.raw, r.diameter)
                            })
                            .collect();
                        let mut add = None;
                        if !fit.is_empty() {
                            ui.menu_button(trf("入る機構部品（{}）…", &[&fit.len()]), |ui| {
                                for m in &fit {
                                    if ui
                                        .button(format!("{} {}", m.vendor, m.part))
                                        .on_hover_text(&m.description)
                                        .clicked()
                                    {
                                        add = Some((*m).clone());
                                        ui.close_menu();
                                    }
                                }
                            })
                            .response
                            .on_hover_text(tr("この品の径が入るチューブ・マウント・ケージ板。選ぶと組合せの機構部品に足します"));
                        }
                        if let Some(m) = add {
                            self.add_pick(&m);
                        }
                    }
                });
            });
        ui.separator();
        // 表の文字を選べないようにする（行のクリック・ダブルクリックを文字が奪わない）
        ui.style_mut().interaction.selectable_labels = false;
        self.preview_ui(ui);
        let mut clicked = None;
        let mut add = None;
        let sort = &mut self.lens_sort;
        let desc = &mut self.lens_desc;
        let rows = &self.lenses;
        let sel = self.lens_sel;
        let header =
            |ui: &mut Ui, label: &str, key: LensSort, sort: &mut LensSort, desc: &mut bool| {
                let mark = if *sort == key {
                    if *desc {
                        " ▼"
                    } else {
                        " ▲"
                    }
                } else {
                    ""
                };
                if ui.button(format!("{label}{mark}")).clicked() {
                    if *sort == key {
                        *desc = !*desc;
                    } else {
                        *sort = key;
                        *desc = false;
                    }
                }
            };
        let cols = shown_cols(&self.lens_hide);
        // 長い項目は折り返さず列の幅で切る（全体はマウスを載せると出る）。列が収まらなければ横にスクロール
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        egui::ScrollArea::horizontal()
            .id_salt("lens_table_h")
            .show(ui, |ui| {
                let mut tb = TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .sense(Sense::click_and_drag());
                for c in &cols {
                    tb = tb.column(c.column());
                }
                tb.header(22.0, |mut h| {
                    for &c in &cols {
                        h.col(|ui| match c.sort() {
                            Some(k) => header(ui, c.label(), k, sort, desc),
                            None => {
                                ui.strong(c.label());
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(20.0, idx.len(), |mut row| {
                        let i = idx[row.index()];
                        let r = &rows[i];
                        row.set_selected(sel == Some(i));
                        for &c in &cols {
                            row.col(|ui| lens_cell(ui, c, r));
                        }
                        let resp = row.response();
                        if resp.clicked() {
                            clicked = Some(i);
                        }
                        if resp.double_clicked() {
                            add = Some(i);
                        }
                        // レイアウト図へドラッグして差し込む
                        if resp.drag_started() && r.placeable() {
                            egui::DragAndDrop::set_payload(
                                &resp.ctx,
                                crate::combo::DropPick(pick_from(r)),
                            );
                            clicked = Some(i);
                        }
                    });
                });
            });
        if let Some(i) = clicked {
            self.lens_sel = Some(i);
        }
        if let Some(i) = add {
            let r = self.lenses[i].clone();
            self.add_pick(&r);
        }
    }

    /// 選んだ玉の断面（平行光が集まる様子）
    fn preview_ui(&self, ui: &mut Ui) {
        let Some(i) = self.lens_sel else { return };
        let h = 120.0;
        match &self.preview {
            Some((j, d)) if *j == i && !d.solids.is_empty() => {
                let accent = ui.visuals().selection.stroke.color;
                crate::combo::view_only(ui, d, h, accent, None);
            }
            Some((j, _)) if *j == i => {
                ui.allocate_space(egui::vec2(ui.available_width(), h));
            }
            _ => {
                ui.allocate_ui(egui::vec2(ui.available_width(), h), |ui| {
                    ui.centered_and_justified(|ui| ui.weak(tr("断面を作っています…")));
                });
            }
        }
    }

    pub fn add_pick(&mut self, r: &LensRow) {
        // 未完の品は置かず、補う窓を開く
        if r.is_incomplete() {
            self.fill_req = Some(r.clone());
            return;
        }
        // 機構部品は要素の並びでなく「機構部品」へ（選んでいる要素に置く）
        if r.kind == RowKind::Mech {
            let ids = self.combo.ids();
            let at = self
                .insert_after
                .and_then(|k| ids.get(k).cloned())
                .or_else(|| ids.iter().find(|x| !x.is_empty()).cloned())
                .unwrap_or_default();
            // 名前は組合せの中で別々に（つなぐ相手を名前で選ぶので）
            let mut name = r.part.clone();
            let mut n = 2;
            while self.combo.mechanics.iter().any(|m| m.name == name) {
                name = format!("{} {n}", r.part);
                n += 1;
            }
            self.combo.mechanics.push(crate::combo::MechPick {
                name,
                catalog: r.source.clone().unwrap_or_default(),
                item: r.part.clone(),
                at,
                ..Default::default()
            });
            return;
        }
        let at = self.insert_after.map_or(self.combo.len(), |k| k + 1);
        let at = crate::combo::insert_pick(&mut self.combo, at, pick_from(r));
        // 続けて足すと、足した玉の後ろへ
        self.insert_after = Some(at);
        self.added = Some(at);
    }

    fn combo_ui(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut act = None;
        let c = &mut self.combo;
        ui.heading(tr("組合せ"));
        ui.label(tr("一覧の行をダブルクリック（または「組合せに足す」）で、レイアウト図で選んでいる要素の直後（無ければ後ろ）に足します。光は上から下へ進みます。"));
        ui.horizontal(|ui| {
            ui.label(tr("名前"));
            ui.text_edit_singleline(&mut c.name);
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(tr("⚙ 系の設定"))
                .on_hover_text(tr(
                    "開口・視野・波長と重み・エイミング・物体距離・絞りの位置など",
                ))
                .clicked()
            {
                act = Some(Action::SysSettings);
            }
            ui.label(RichText::new(c.settings_summary()).small().weak());
        });
        for p in c.settings_problems() {
            ui.label(RichText::new(format!("⚠ {p}")).color(Color32::from_rgb(220, 80, 60)));
        }
        ui.separator();
        let ids: Vec<String> = c.ids().into_iter().filter(|x| !x.is_empty()).collect();
        let n = c.picks.len();
        let mut op: Option<(usize, i32)> = None; // (行, -1 上へ / +1 下へ / 0 消す)
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 40.0)
            .show(ui, |ui| {
                egui::Grid::new("picks")
                    .striped(true)
                    .num_columns(4)
                    .show(ui, |ui| {
                        for (i, p) in c.picks.iter_mut().enumerate() {
                            ui.label(if p.is_branched() {
                                // 分岐の先は組合せの図・要素の設定で
                                trf("{}. {}（分岐）", &[&(i + 1), &p.label()])
                            } else {
                                format!("{}. {} {}", i + 1, p.vendor, p.part)
                            });
                            ui.checkbox(&mut p.reversed, tr("反転"))
                                .on_hover_text(tr("レンズを裏返して置く"));
                            ui.horizontal(|ui| {
                                ui.label(if i + 1 == n {
                                    tr("像面まで")
                                } else {
                                    tr("後の空気")
                                });
                                ui.add(egui::DragValue::new(&mut p.air).speed(0.1).suffix(" mm"));
                            });
                            ui.horizontal(|ui| {
                                if ui.add_enabled(i > 0, egui::Button::new("↑")).clicked() {
                                    op = Some((i, -1));
                                }
                                if ui.add_enabled(i + 1 < n, egui::Button::new("↓")).clicked() {
                                    op = Some((i, 1));
                                }
                                if ui.button("✖").on_hover_text(tr("外す")).clicked() {
                                    op = Some((i, 0));
                                }
                            });
                            ui.end_row();
                        }
                    });
                if !c.mechanics.is_empty() {
                    ui.separator();
                    mech_ui(ui, &mut c.mechanics, &ids);
                }
            });
        match op {
            Some((i, 0)) => {
                c.picks.remove(i);
            }
            Some((i, -1)) => c.picks.swap(i, i - 1),
            Some((i, _)) => c.picks.swap(i, i + 1),
            None => {}
        }
        ui.horizontal(|ui| {
            let ok = !c.picks.is_empty() && c.settings_problems().is_empty();
            if ui
                .add_enabled(
                    ok,
                    egui::Button::new(RichText::new(tr("▶ この組合せで系を作る")).strong()),
                )
                .on_hover_text(tr("プロジェクトファイル（.json）に書いて読み込みます"))
                .clicked()
            {
                act = Some(Action::Build(c.project()));
            }
            if ui
                .add_enabled(!c.picks.is_empty(), egui::Button::new(tr("空にする")))
                .clicked()
            {
                c.picks.clear();
            }
        });
        act
    }

    fn glass_ui(&mut self, ui: &mut Ui) {
        let q = &mut self.glass_q;
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("検索"));
            ui.add(
                egui::TextEdit::singleline(&mut q.text)
                    .hint_text(tr("ガラス名"))
                    .desired_width(140.0),
            );
            let mut cats: Vec<&str> = self.glasses.iter().map(|g| g.catalog.as_str()).collect();
            cats.sort();
            cats.dedup();
            egui::ComboBox::from_id_salt("glass_cat")
                .selected_text(if q.catalog.is_empty() {
                    tr("すべてのカタログ")
                } else {
                    &q.catalog
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut q.catalog, String::new(), tr("すべてのカタログ"));
                    for v in cats {
                        ui.selectable_value(&mut q.catalog, v.to_string(), v);
                    }
                });
        });
        ui.horizontal_wrapped(|ui| {
            range_ui(ui, "nd", &mut q.nd, (1.5, 1.8), 0.001);
            ui.separator();
            range_ui(ui, "νd", &mut q.vd, (30.0, 60.0), 0.1);
        });
        let idx = filter_glasses(&self.glasses, &self.glass_q);
        let picked = self.glass_sel.and_then(|i| self.glasses.get(i));
        // 1 行に収め、長ければ横にスクロール（右の組合せ・図に重ならない）
        egui::ScrollArea::horizontal()
            .id_salt("glass_detail")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(24.0);
                    ui.label(trf("{} / {} 種", &[&idx.len(), &self.glasses.len()]));
                    ui.separator();
                    let Some(g) = picked else {
                        ui.weak(tr(tr(
                            "nd・νd はカタログ温度の分散式から。表の行か図の点をクリックで選びます",
                        )));
                        return;
                    };
                    ui.label(RichText::new(format!("{} ({})", g.name, g.catalog)).strong());
                    ui.label(format!(
                        "nd {}, νd {}{}",
                        fmt_num((g.nd * 1e5).round() / 1e5),
                        fmt_num((g.vd * 1e2).round() / 1e2),
                        g.range
                            .map(|r| trf("、波長 {}–{} µm", &[&fmt_num(r[0]), &fmt_num(r[1])]))
                            .unwrap_or_default()
                    ));
                    if ui.button(tr("名前をコピー")).clicked() {
                        ui.ctx().copy_text(g.name.clone());
                    }
                });
            });
        ui.separator();
        // 左に表、右にガラスマップ（狭ければ下に）
        let w = ui.available_width();
        let wide = w >= 640.0;
        let builder = StripBuilder::new(ui).size(Size::remainder().at_least(160.0));
        let builder = if wide {
            builder.size(Size::exact((w * 0.4).clamp(240.0, 420.0)))
        } else {
            builder.size(Size::relative(0.45))
        };
        let mut cells = |mut strip: egui_extras::Strip| {
            strip.cell(|ui| self.glass_table(ui, &idx));
            strip.cell(|ui| {
                if let Some(i) = glass_map(
                    ui,
                    &mut self.glass_view,
                    &self.glasses,
                    &idx,
                    self.glass_sel,
                ) {
                    self.glass_sel = Some(i);
                }
            });
        };
        if wide {
            builder.horizontal(&mut cells);
        } else {
            builder.vertical(cells);
        }
    }

    fn glass_table(&mut self, ui: &mut Ui, idx: &[usize]) {
        let rows = &self.glasses;
        let sel = self.glass_sel;
        let mut clicked = None;
        ui.style_mut().interaction.selectable_labels = false;
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(Sense::click())
            .column(Column::initial(110.0))
            .column(Column::initial(80.0))
            .column(Column::initial(70.0))
            .column(Column::initial(60.0))
            .column(Column::remainder())
            .header(22.0, |mut h| {
                for t in [tr("ガラス"), tr("カタログ"), "nd", "νd", tr("波長 [µm]")] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(20.0, idx.len(), |mut row| {
                    let i = idx[row.index()];
                    let g = &rows[i];
                    row.set_selected(sel == Some(i));
                    row.col(|ui| {
                        ui.label(&g.name);
                    });
                    row.col(|ui| {
                        ui.label(&g.catalog);
                    });
                    row.col(|ui| num_cell(ui, (g.nd * 1e5).round() / 1e5 + 0.0));
                    row.col(|ui| num_cell(ui, (g.vd * 1e2).round() / 1e2));
                    row.col(|ui| {
                        if let Some(r) = g.range {
                            ui.label(format!("{}–{}", fmt_num(r[0]), fmt_num(r[1])));
                        }
                    });
                    if row.response().clicked() {
                        clicked = Some(i);
                    }
                });
            });
        if let Some(i) = clicked {
            self.glass_sel = Some(i);
        }
    }
}

/// カタログの行を組合せの要素にする
pub fn pick_from(r: &LensRow) -> Pick {
    // 後の空気の初めの値: 正のレンズなら焦点距離（平行光がおおよそ集まる所）、ほかは 5 mm
    let air = if r.efl.is_finite() && r.efl > 0.0 {
        (r.efl * 1e3).round() / 1e3
    } else {
        5.0
    };
    // カタログの BS は BS のブロック（中身を写す。組合せで向き・経路・寸法を変えられる）
    if let (RowKind::Splitter, Some(sp)) = (r.kind, &r.splitter) {
        let mut p = Pick::new(crate::combo::ElemKind::Splitter);
        p.vendor = r.vendor.clone();
        p.part = r.part.clone();
        if let Some(x) = sp["size"].as_f64().filter(|x| *x > 0.0) {
            p.t = x;
        }
        if let Some(g) = sp["glass"].as_str().filter(|g| !g.is_empty()) {
            p.glass = g.to_string();
        }
        if let Some(x) = sp["ratio"].as_f64() {
            p.ratio = x;
        }
        p.coat = r.coat.clone();
        let mut extra = sp.clone();
        if let Some(m) = extra.as_object_mut() {
            for k in ["size", "glass", "ratio", "coat"] {
                m.remove(k);
            }
        }
        p.extra = extra;
        p.absorb_extra();
        return p;
    }
    // カタログの鏡は鏡のブロック（傾けて光軸を折れる）
    if let Some((radius, conic)) = r.mirror {
        let mut p = Pick::new(crate::combo::ElemKind::Mirror);
        p.vendor = r.vendor.clone();
        p.part = r.part.clone();
        p.r1 = radius;
        p.conic = conic;
        p.coat = r.coat.clone();
        if r.diameter.is_finite() && r.diameter > 0.0 {
            p.semi = r.diameter / 2.0;
        }
        return p;
    }
    Pick {
        vendor: r.vendor.clone(),
        part: r.part.clone(),
        // lensbench 形式のカタログの玉はカタログのファイルを参照する
        path: r.source.clone().unwrap_or_default(),
        air,
        // カタログの玉は中身を変えない前提なので、既定でロック（位置・後の空気は変えられる）
        locked: true,
        ..Pick::default()
    }
}

/// BS の詳細の行
fn splitter_detail(r: &LensRow) -> String {
    let sp = r.splitter.clone().unwrap_or_default();
    let form = sp["form"].as_str().unwrap_or("cube");
    let size = if form == "cube" {
        trf("キューブ 一辺 {} mm", &[&fmt_num(r.diameter)])
    } else if form == "plate" {
        trf(
            "平板 厚さ {} mm、径 {} mm",
            &[&fmt_num(r.thickness), &fmt_num(r.diameter)],
        )
    } else {
        tr("理想の面").to_string()
    };
    let split = if !r.coat.is_empty() {
        trf("膜 {}", &[&r.coat])
    } else if sp["pbs"].is_object() {
        "PBS".to_string()
    } else {
        trf("T {}", &[&fmt_num(sp["ratio"].as_f64().unwrap_or(0.5))])
    };
    let mut v = vec![size, split];
    if !r.glasses.is_empty() {
        v.push(trf("ガラス {}", &[&r.glasses.join(" / ")]));
    }
    if let Some(ar) = sp["ar"].as_str().filter(|a| !a.is_empty()) {
        v.push(trf("AR {}", &[&ar]));
    }
    format!("BS: {}", v.join(tr("、")))
}

/// ガラスマップの見えている範囲（νd は vd0 が左。左ほど大きい）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapView {
    pub vd0: f64,
    pub vd1: f64,
    pub nd0: f64,
    pub nd1: f64,
}

impl MapView {
    /// 全体表示
    pub const FULL: MapView = MapView {
        vd0: 95.0,
        vd1: 15.0,
        nd0: 1.4,
        nd1: 2.1,
    };
    /// 拡大の限り（幅の最小）
    const MIN_VD: f64 = 0.5;
    const MIN_ND: f64 = 0.005;

    pub fn is_full(&self) -> bool {
        *self == Self::FULL
    }

    /// (vd, nd) を中心に factor 倍に拡大する（1 より小さければ縮小）。全体より広くはしない
    pub fn zoom(&mut self, vd: f64, nd: f64, factor: f64) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let f = Self::FULL;
        let (fw, fh) = (f.vd0 - f.vd1, f.nd1 - f.nd0);
        let w = ((self.vd0 - self.vd1) / factor).clamp(Self::MIN_VD, fw);
        let h = ((self.nd1 - self.nd0) / factor).clamp(Self::MIN_ND, fh);
        // 指した点が画面上で動かないように
        let a = (self.vd0 - vd) / (self.vd0 - self.vd1);
        let b = (nd - self.nd0) / (self.nd1 - self.nd0);
        self.vd0 = vd + a * w;
        self.vd1 = self.vd0 - w;
        self.nd0 = nd - b * h;
        self.nd1 = self.nd0 + h;
        self.keep_inside();
    }

    /// 範囲を (dvd, dnd) だけずらす（全体の外へは出さない）
    pub fn pan(&mut self, dvd: f64, dnd: f64) {
        self.vd0 += dvd;
        self.vd1 += dvd;
        self.nd0 += dnd;
        self.nd1 += dnd;
        self.keep_inside();
    }

    fn keep_inside(&mut self) {
        let f = Self::FULL;
        if self.vd0 > f.vd0 {
            let d = self.vd0 - f.vd0;
            self.vd0 -= d;
            self.vd1 -= d;
        }
        if self.vd1 < f.vd1 {
            let d = f.vd1 - self.vd1;
            self.vd0 += d;
            self.vd1 += d;
        }
        if self.nd0 < f.nd0 {
            let d = f.nd0 - self.nd0;
            self.nd0 += d;
            self.nd1 += d;
        }
        if self.nd1 > f.nd1 {
            let d = self.nd1 - f.nd1;
            self.nd0 -= d;
            self.nd1 -= d;
        }
        // 全体と同じ幅なら丸めの誤差を残さない
        if (self.vd0 - self.vd1 - (f.vd0 - f.vd1)).abs() < 1e-9 {
            self.vd0 = f.vd0;
            self.vd1 = f.vd1;
        }
        if (self.nd1 - self.nd0 - (f.nd1 - f.nd0)).abs() < 1e-9 {
            self.nd0 = f.nd0;
            self.nd1 = f.nd1;
        }
    }
}

impl Default for MapView {
    fn default() -> Self {
        Self::FULL
    }
}

/// 目盛り（lo..=hi の中の、1・2・5 × 10^k 刻みの値と小数の桁数）。おおよそ n 本
fn ticks(lo: f64, hi: f64, n: usize) -> (Vec<f64>, usize) {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    let raw = (hi - lo) / n.max(1) as f64;
    if !(raw.is_finite() && raw > 0.0) {
        return (vec![], 0);
    }
    let p = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * p)
        .find(|&s| s >= raw * (1.0 - 1e-9))
        .unwrap_or(10.0 * p);
    let digits = (-step.log10().floor()).max(0.0) as usize;
    let mut v = vec![];
    let mut k = (lo / step).ceil();
    let q = 10f64.powi(digits as i32);
    while k * step <= hi + step * 1e-9 {
        v.push((k * step * q).round() / q);
        k += 1.0;
    }
    (v, digits)
}

/// ガラスマップ（横 νd（右ほど小さい）、縦 nd）。クリックした点に一番近いガラスを返す。
/// ホイールで拡大・縮小、ドラッグで移動、ダブルクリックで全体表示
fn glass_map(
    ui: &mut Ui,
    view: &mut MapView,
    rows: &[GlassRow],
    idx: &[usize],
    sel: Option<usize>,
) -> Option<usize> {
    let size = egui::vec2(
        ui.available_width(),
        ui.available_width().min(ui.available_height()) * 0.8,
    );
    let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let r = resp.rect;
    let rect = egui::Rect::from_min_max(
        r.min + egui::vec2(40.0, 22.0),
        r.max - egui::vec2(12.0, 32.0),
    );
    // 操作（描く前に範囲を決める）
    if resp.double_clicked() {
        *view = MapView::FULL;
    } else {
        if resp.dragged() {
            let d = resp.drag_delta();
            view.pan(
                -(d.x / rect.width()) as f64 * (view.vd1 - view.vd0),
                (d.y / rect.height()) as f64 * (view.nd1 - view.nd0),
            );
        }
        if let Some(p) = resp.hover_pos() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (scroll as f64 * 0.003).exp() * zoom as f64;
            if (factor - 1.0).abs() > 1e-6 {
                let vd =
                    view.vd0 + ((p.x - rect.left()) / rect.width()) as f64 * (view.vd1 - view.vd0);
                let nd = view.nd0
                    + ((rect.bottom() - p.y) / rect.height()) as f64 * (view.nd1 - view.nd0);
                view.zoom(vd, nd, factor);
                // 外側の表などをスクロールさせない
                ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
            }
        }
    }
    let MapView { vd0, vd1, nd0, nd1 } = *view;
    let vis = ui.visuals();
    painter.rect_filled(resp.rect, 2.0, vis.extreme_bg_color);
    let to = |vd: f64, nd: f64| {
        egui::pos2(
            rect.left() + ((vd - vd0) / (vd1 - vd0)) as f32 * rect.width(),
            rect.bottom() - ((nd - nd0) / (nd1 - nd0)) as f32 * rect.height(),
        )
    };
    let axis = vis.weak_text_color();
    let font = egui::FontId::proportional(10.0);
    let (vts, vdig) = ticks(vd1, vd0, 8);
    for vd in vts {
        let p = to(vd, nd0);
        painter.line_segment([p, to(vd, nd1)], (0.5, axis.gamma_multiply(0.3)));
        painter.text(
            p + egui::vec2(0.0, 2.0),
            egui::Align2::CENTER_TOP,
            format!("{vd:.vdig$}"),
            font.clone(),
            axis,
        );
    }
    let (nts, ndig) = ticks(nd0, nd1, 7);
    for nd in nts {
        let p = to(vd0, nd);
        painter.line_segment([p, to(vd1, nd)], (0.5, axis.gamma_multiply(0.3)));
        painter.text(
            p - egui::vec2(3.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            format!("{nd:.ndig$}"),
            font.clone(),
            axis,
        );
    }
    painter.text(
        rect.center_bottom() + egui::vec2(0.0, 16.0),
        egui::Align2::CENTER_TOP,
        "νd",
        font.clone(),
        axis,
    );
    painter.text(
        rect.left_top() - egui::vec2(0.0, 6.0),
        egui::Align2::CENTER_BOTTOM,
        "nd",
        font.clone(),
        axis,
    );
    if !view.is_full() {
        painter.text(
            rect.left_top() + egui::vec2(16.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            tr("ダブルクリックで全体表示"),
            font,
            axis,
        );
    }
    let dot = vis.selection.bg_fill;
    // 点は軸の内側だけに描く
    let painter = painter.with_clip_rect(rect.expand(3.0));
    for &i in idx {
        let g = &rows[i];
        if g.nd.is_finite() && g.vd.is_finite() {
            painter.circle_filled(to(g.vd, g.nd), 2.0, dot);
        }
    }
    if let Some(g) = sel.and_then(|i| rows.get(i)) {
        if g.nd.is_finite() && g.vd.is_finite() {
            painter.circle_stroke(to(g.vd, g.nd), 5.0, (2.0, Color32::from_rgb(230, 80, 40)));
        }
    }
    let near = |p: egui::Pos2| {
        if !rect.expand(3.0).contains(p) {
            return None;
        }
        idx.iter()
            .copied()
            .filter(|&i| rows[i].nd.is_finite() && rows[i].vd.is_finite())
            .map(|i| (i, to(rows[i].vd, rows[i].nd).distance(p)))
            .filter(|&(_, d)| d < 12.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    if let Some(p) = resp.hover_pos() {
        if !resp.dragged() {
            if let Some(i) = near(p) {
                let g = &rows[i];
                resp.clone().on_hover_text(format!(
                    "{} ({})\nnd {:.5}, νd {:.2}",
                    g.name, g.catalog, g.nd, g.vd
                ));
            }
        }
    }
    if resp.clicked() {
        return resp.interact_pointer_pos().and_then(near);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// アッベ図の拡大は指した点を動かさず、縮小は全体まで、移動は全体の外へ出ない
    #[test]
    fn map_view_zoom_pan() {
        let mut v = MapView::default();
        assert!(v.is_full());
        // (60, 1.6) を中心に 4 倍
        let a = (v.vd0 - 60.0) / (v.vd0 - v.vd1);
        let b = (1.6 - v.nd0) / (v.nd1 - v.nd0);
        v.zoom(60.0, 1.6, 4.0);
        assert!((v.vd0 - v.vd1 - 20.0).abs() < 1e-9);
        assert!((v.nd1 - v.nd0 - 0.175).abs() < 1e-9);
        assert!(((v.vd0 - 60.0) / (v.vd0 - v.vd1) - a).abs() < 1e-9);
        assert!(((1.6 - v.nd0) / (v.nd1 - v.nd0) - b).abs() < 1e-9);
        // 右下へ大きく動かしても全体の内側
        v.pan(-1000.0, -10.0);
        assert!((v.vd1 - 15.0).abs() < 1e-9 && (v.nd0 - 1.4).abs() < 1e-9);
        assert!((v.vd0 - v.vd1 - 20.0).abs() < 1e-9);
        // 縮小しすぎても全体まで
        v.zoom(30.0, 1.5, 1e-3);
        assert!(v.is_full());
        // 拡大の限り
        v.zoom(50.0, 1.7, 1e9);
        assert!((v.vd0 - v.vd1 - MapView::MIN_VD).abs() < 1e-9);
        assert!((v.nd1 - v.nd0 - MapView::MIN_ND).abs() < 1e-9);
    }

    #[test]
    fn map_ticks() {
        let (t, d) = ticks(15.0, 95.0, 8);
        assert_eq!(t, vec![20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0]);
        assert_eq!(d, 0);
        let (t, d) = ticks(1.4, 2.1, 7);
        assert_eq!(t.len(), 8);
        assert_eq!(d, 1);
        let (t, d) = ticks(1.512, 1.53, 7);
        assert_eq!(d, 3);
        assert!(t.iter().all(|&x| (1.512..=1.53).contains(&x)));
    }

    fn lens(v: &str, p: &str, efl: f64, d: f64) -> LensRow {
        LensRow {
            vendor: v.into(),
            part: p.into(),
            efl,
            diameter: d,
            thickness: 3.0,
            glasses: vec!["N-BK7".into()],
            error: None,
            mirror: None,
            pieces: 1,
            ..Default::default()
        }
    }

    /// ダブルクリックで足す玉は、図で選んでいる要素の直後へ（続けて足すとその後ろへ）
    #[test]
    fn add_pick_goes_after_selected() {
        let mut b = Browser::default();
        for p in ["A", "B", "C"] {
            b.add_pick(&lens("LB", p, 50.0, 25.4));
        }
        b.insert_after = None;
        let order = |b: &Browser| {
            b.combo
                .picks
                .iter()
                .map(|p| p.part.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&b), ["A", "B", "C"]);
        b.insert_after = Some(0);
        b.add_pick(&lens("LB", "X", 50.0, 25.4));
        b.add_pick(&lens("LB", "Y", 50.0, 25.4));
        assert_eq!(order(&b), ["A", "X", "Y", "B", "C"]);
        assert_eq!(b.added, Some(2));
    }

    /// カタログの鏡は鏡のブロック（型番・曲率半径・径）として組合せに入る
    #[test]
    fn catalog_mirror_becomes_mirror_block() {
        let v = json!({ "lenses": [
            { "vendor": "TL", "part": "CM254-100", "efl": 100.0, "diameter": 25.4, "thickness": 0.0,
              "glasses": [], "mirror": true, "radius": -200.0, "conic": -1.0 },
        ] });
        let rows = parse_lenses(&v);
        assert_eq!(rows[0].mirror, Some((-200.0, -1.0)));
        let mut b = Browser::default();
        b.add_pick(&rows[0]);
        let p = &b.combo.picks[0];
        assert_eq!(p.kind, crate::combo::ElemKind::Mirror);
        assert_eq!(p.label(), "TL CM254-100");
        let blocks = b.combo.project()["blocks"].clone();
        let m = blocks
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["type"] == "mirror")
            .unwrap()
            .clone();
        assert_eq!(m["radius"], -200.0);
        assert_eq!(m["conic"], -1.0);
        assert_eq!(m["semi"], 12.7);
    }

    #[test]
    fn lens_filter_and_sort() {
        let rows = vec![
            lens("A", "P50", 50.0, 25.4),
            lens("A", "N25", -25.0, 12.7),
            lens("B", "P100", 100.0, 25.4),
            lens("B", "BAD", f64::NAN, 10.0),
        ];
        let mut q = LensFilter::default();
        assert_eq!(
            filter_lenses(&rows, &q, LensSort::Efl, false),
            vec![1, 0, 2, 3]
        );
        assert_eq!(
            filter_lenses(&rows, &q, LensSort::Efl, true),
            vec![3, 2, 0, 1]
        );
        q.sign = Sign::Positive;
        assert_eq!(
            filter_lenses(&rows, &q, LensSort::Vendor, false),
            vec![0, 2]
        );
        q.sign = Sign::Any;
        q.efl = Range {
            min: Some(40.0),
            max: None,
        };
        assert_eq!(
            filter_lenses(&rows, &q, LensSort::Vendor, false),
            vec![0, 2]
        );
        q.efl = Range::default();
        q.text = "b p1".into();
        assert_eq!(filter_lenses(&rows, &q, LensSort::Vendor, false), vec![2]);
        q.text = "bk7".into();
        q.vendor = "A".into();
        q.diameter = Range {
            min: None,
            max: Some(20.0),
        };
        assert_eq!(filter_lenses(&rows, &q, LensSort::Vendor, false), vec![1]);
    }

    /// 枚数で絞る（3 は 3 枚以上）
    #[test]
    fn lens_filter_by_pieces() {
        let mut rows = vec![
            lens("A", "S", 50.0, 25.4),
            lens("A", "D", 50.0, 25.4),
            lens("A", "T", 50.0, 25.4),
        ];
        rows[1].pieces = 2;
        rows[2].pieces = 4;
        let mut q = LensFilter::default();
        let f = |q: &LensFilter| filter_lenses(&rows, q, LensSort::Part, false);
        assert_eq!(f(&q).len(), 3);
        q.pieces = 2;
        assert_eq!(f(&q), vec![1]);
        q.pieces = 3;
        assert_eq!(f(&q), vec![2]);
    }

    /// カタログの機構部品は要素でなく機構部品として、選んでいる要素に置く
    #[test]
    fn catalog_mech_becomes_mechanics() {
        let v = json!({ "lenses": [
            { "vendor": "TL", "part": "SM1L10", "efl": null, "diameter": 30.5, "thickness": 25.4,
              "glasses": [], "kind": "mech", "source": { "catalog": "/c/items/tl_mech.json" } },
        ] });
        let rows = parse_lenses(&v);
        assert_eq!(rows[0].kind, RowKind::Mech);
        let mut b = Browser::default();
        b.add_pick(&lens("LB", "A", 50.0, 25.4));
        b.add_pick(&lens("LB", "B", 50.0, 25.4));
        b.insert_after = Some(1);
        b.add_pick(&rows[0]);
        assert_eq!(b.combo.picks.len(), 2);
        let m = &mut b.combo.mechanics[0];
        assert_eq!((m.item.as_str(), m.at.as_str()), ("SM1L10", "B"));
        m.holds = vec!["A".into()];
        let p = b.combo.project();
        let j = &p["mechanics"][0];
        assert_eq!(j["catalog"], "/c/items/tl_mech.json");
        assert_eq!(j["at"], json!({ "block": "B", "surface": 0 }));
        assert_eq!(j["holds"], json!(["A"]));
        // 置く要素が空なら系の面 1、保持なしなら holds は出さない
        b.combo.mechanics[0].at.clear();
        b.combo.mechanics[0].holds.clear();
        let j = &b.combo.project()["mechanics"][0];
        assert_eq!(j["at"], json!({ "surface": 1 }));
        assert!(j.get("holds").is_none());
        // 消した要素の id は古い id
        assert!(
            stale_id("B", &["A".to_string()])
                && !stale_id("", &[])
                && !stale_id("A", &["A".to_string()])
        );
        // 断面のプレビューにも部品が入る
        assert_eq!(preview_project(&rows[0])["mechanics"][0]["item"], "SM1L10");
    }

    /// 選んだ玉の断面を見るプロジェクト: 玉 1 枚、像面は焦点距離の所
    #[test]
    fn preview_project_has_one_lens() {
        let mut r = lens("LB", "N", -25.0, 12.7);
        let p = preview_project(&r);
        let b = p["blocks"].as_array().unwrap();
        assert_eq!(b.iter().filter(|x| x["type"] == "lens").count(), 1);
        assert_eq!(b.last().unwrap()["t"], 25.0);
        r.efl = f64::NAN;
        assert_eq!(
            preview_project(&r)["blocks"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["t"],
            20.0
        );
    }

    #[test]
    fn glass_filter() {
        let g = |n: &str, nd: f64, vd: f64| GlassRow {
            name: n.into(),
            catalog: "SCHOTT".into(),
            nd,
            vd,
            range: None,
        };
        let rows = vec![
            g("N-BK7", 1.5168, 64.17),
            g("F2", 1.62, 36.37),
            g("N-SF11", 1.785, 25.68),
        ];
        let mut q = GlassFilter::default();
        q.nd.min = Some(1.6);
        q.vd.min = Some(30.0);
        assert_eq!(filter_glasses(&rows, &q), vec![1]);
        q = GlassFilter {
            text: "n-".into(),
            ..Default::default()
        };
        assert_eq!(filter_glasses(&rows, &q), vec![0, 2]);
    }

    #[test]
    fn project_ids_are_unique() {
        let mut c = Combo::default();
        for _ in 0..2 {
            c.picks.push(Pick {
                vendor: "LB".into(),
                part: "PCX-100".into(),
                air: 10.0,
                ..Pick::default()
            });
        }
        c.picks[1].reversed = true;
        let p = c.project();
        let b = p["blocks"].as_array().unwrap();
        assert_eq!(b.len(), 6);
        assert_eq!(b[2]["id"], "PCX-100");
        assert_eq!(b[4]["id"], "PCX-100#2");
        assert_eq!(b[4]["reversed"], true);
        assert_eq!(b[0]["semi"], 2.5);
    }

    /// serve と同じ API で、カタログの一覧を取り、組合せのプロジェクトを読んで解析できる
    #[test]
    fn combo_loads_through_serve_api() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let glass = root.join("tests/glass").display().to_string();
        let lb = root.join("examples/lenses/LB").display().to_string();
        let call = |m: &str, p: Value| lensbench_cli::serve::call(m, &p, Some(1)).unwrap();
        let g = parse_glasses(&call("glasses", json!({ "glass_dir": [glass] })));
        assert!(g.iter().any(|x| x.name == "N-BK7"));
        let l = parse_lenses(&call(
            "lenses",
            json!({ "lens_catalog": [lb], "glass_dir": [glass] }),
        ));
        assert_eq!(l.len(), 5);
        assert!(l.iter().all(|x| x.error.is_none()), "{l:?}");
        let mut b = Browser {
            lenses: l,
            ..Default::default()
        };
        let pcx = b
            .lenses
            .iter()
            .find(|x| x.part == "PCX-100")
            .unwrap()
            .clone();
        b.add_pick(&pcx);
        assert_eq!(b.combo.picks[0].air, (pcx.efl * 1e3).round() / 1e3);
        let dir = std::env::temp_dir().join(format!("lensbench-gui-test-{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&dir).unwrap();
        let f = dir.join("combo.json");
        lensbench_core::vfs::write(&f, b.combo.project().to_string()).unwrap();
        let s = call(
            "load",
            json!({ "file": f.display().to_string(), "glass_dir": [glass], "lens_catalog": [lb] }),
        );
        assert_eq!(s["fields"].as_array().unwrap().len(), 3);
        let info = call("analyze", json!({ "cmd": "info" }));
        let efl = info["effl"].as_f64().unwrap();
        assert!((efl - pcx.efl).abs() < 1e-6 * pcx.efl, "{efl} {}", pcx.efl);
        let spot = call("analyze", json!({ "cmd": "spot", "field": 1 }));
        assert!(spot.is_object());
        // レイアウト図の像面から、像面を近軸の焦点へ（GUI と同じ手順）
        let lay = crate::combo::LayoutData::parse(&call("layout", json!({})));
        let img = lay.image_surface().unwrap();
        let ray = |last: usize| {
            let p = json!({ "px": [0.0], "py": [1e-3], "hx": 0.0, "hy": 0.0, "last": last });
            let (_, a) = lensbench_cli::serve::call_arrays("trace", &p, Some(1)).unwrap();
            let g = |k: &str| a.iter().find(|x| x.0 == k).unwrap().1[0];
            (g("y"), g("z"), g("m"), g("n"))
        };
        let (y, z, m, n) = ray(img - 1);
        let t = crate::combo::paraxial_focus(y, z, m, n).unwrap();
        // 平凸の後ろの焦点はほぼ焦点距離（主点のずれの分だけ短い）
        assert!(t > 0.9 * pcx.efl && t < pcx.efl, "{t}");
        b.combo.picks[0].air = t;
        lensbench_core::vfs::write(&f, b.combo.project().to_string()).unwrap();
        call(
            "load",
            json!({ "file": f.display().to_string(), "glass_dir": [glass], "lens_catalog": [lb] }),
        );
        let (y_img, ..) = ray(img);
        assert!(y_img.abs() < 1e-9, "{y_img} (入射 {y})");
        std::fs::remove_dir_all(&dir).ok();
    }
    /// lensbench 形式のカタログの BS・鏡・玉を組合せに足し、serve で系にできる
    #[test]
    fn json_catalog_parts_build() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let glass = root.join("tests/glass").display().to_string();
        let lbp = root.join("examples/lenses/LBP").display().to_string();
        let call = |m: &str, p: Value| lensbench_cli::serve::call(m, &p, Some(1));
        let l = parse_lenses(
            &call(
                "lenses",
                json!({ "lens_catalog": [lbp], "glass_dir": [glass] }),
            )
            .unwrap(),
        );
        assert!(l.iter().all(|x| x.error.is_none()), "{l:?}");
        let row = |p: &str| l.iter().find(|x| x.part == p).unwrap().clone();
        assert_eq!(row("BS-127").kind, RowKind::Splitter);
        assert_eq!(row("PM-254").kind, RowKind::Mirror);
        assert_eq!(row("SL-254-50").kind, RowKind::Lens);
        // 種類で絞り込む
        let q = LensFilter {
            kind: Some(RowKind::Splitter),
            ..Default::default()
        };
        assert_eq!(filter_lenses(&l, &q, LensSort::Vendor, false).len(), 3);
        // 品の種類・仕様の条件・語（説明・タグ・仕様）で絞り込む
        let parts = |q: &LensFilter| -> Vec<String> {
            filter_lenses(&l, q, LensSort::Part, false)
                .into_iter()
                .map(|i| l[i].part.clone())
                .collect()
        };
        let q = LensFilter {
            category: Some("waveplate".into()),
            ..Default::default()
        };
        assert_eq!(parts(&q), ["QWP-254-633"]);
        let q = LensFilter {
            spec: "extinction_ratio>=1000 wavelength_nm=650".into(),
            ..Default::default()
        };
        assert_eq!(parts(&q), ["PBS-127", "POL-254"]);
        let q = LensFilter {
            text: "dielectric".into(),
            ..Default::default()
        };
        assert_eq!(parts(&q), ["PM-254"]);
        assert!(parse_spec_filters("efl>=x").is_err());
        let bs = row("BS-127");
        assert_eq!(bs.url, "https://example.com/lb/bs-127");
        assert_eq!(bs.category, "splitter");
        assert!(
            bs.spec_summary().starts_with("50:50、400–700 nm"),
            "{}",
            bs.spec_summary()
        );
        assert_eq!(row("PM-254").url, "https://example.com/lb/PM-254");
        let dir = std::env::temp_dir().join(format!("lensbench-gui-parts-{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&dir).unwrap();
        let f = dir.join("combo.json");
        for parts in [
            vec!["SL-254-50", "BS-127"],
            vec!["SL-254-50", "BSP-254"],
            vec!["SL-254-50", "PM-254"],
            vec!["CM-254-100"],
        ] {
            let mut c = Combo {
                epd: 5.0,
                ..Default::default()
            };
            for p in &parts {
                c.picks.push(pick_from(&row(p)));
            }
            let pr = c.project();
            let b = pr["blocks"].as_array().unwrap();
            if parts[1..].contains(&"BSP-254") {
                let bs = b.iter().find(|x| x["type"] == "splitter").unwrap();
                assert_eq!(bs["form"], "plate");
                assert_eq!(bs["thickness"], 1.0);
            }
            if parts[0] == "SL-254-50" {
                assert!(b
                    .iter()
                    .any(|x| x["type"] == "user" && x["item"] == "SL-254-50"));
            }
            lensbench_core::vfs::write(&f, pr.to_string()).unwrap();
            // serve の load は系を差し替える（ほかのテストと並ぶ）ので、系を変えない下書きの断面で確かめる
            let lay = lensbench_cli::serve::preview_layout(
                &json!({ "file": f.display().to_string(), "glass_dir": [glass] }),
                &json!({}),
            )
            .unwrap_or_else(|e| panic!("{parts:?}: {e}"));
            assert!(!lay["solids"].as_array().unwrap().is_empty(), "{parts:?}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 負の単レンズのプレビューは、集まる光を入れて平行光で出し、絞りは玉の前
    #[test]
    fn negative_singlet_preview_exits_parallel() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let glass = root.join("tests/glass").display().to_string();
        let lb = root.join("examples/lenses/LB").display().to_string();
        let call = |m: &str, p: Value| lensbench_cli::serve::call(m, &p, Some(1));
        let l = parse_lenses(
            &call(
                "lenses",
                json!({ "lens_catalog": [lb.clone()], "glass_dir": [glass.clone()] }),
            )
            .unwrap(),
        );
        let dir =
            std::env::temp_dir().join(format!("lensbench-gui-negprev-{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&dir).unwrap();
        let f = dir.join("p.json");
        for part in ["BCC-32", "PCX-100", "ACH-45"] {
            let r = l.iter().find(|x| x.part == part).unwrap();
            let p = preview_project(r);
            let neg = part == "BCC-32";
            assert_eq!(negative_singlet(r), neg, "{part}: {} {}", r.efl, r.pieces);
            lensbench_core::vfs::write(&f, p.to_string()).unwrap();
            let mut lay = crate::combo::LayoutData::parse(
                &lensbench_cli::serve::preview_layout(
                    &json!({ "file": f.display().to_string(), "glass_dir": [glass.clone()], "lens_catalog": [lb.clone()] }),
                    &json!({}),
                )
                .unwrap_or_else(|e| panic!("{part}: {e}")),
            );
            assert!(!lay.solids.is_empty() && !lay.rays.is_empty(), "{part}");
            lay.virtual_object_from_front();
            // 光線は玉の前から始まる
            assert!(
                !neg || lay.rays.iter().all(|(_, p)| p[0][0] < -1.0),
                "{part}"
            );
            // 縁の光線の最後の区間の傾き
            let slope = |pts: &[[f64; 2]]| {
                let a = pts[pts.len() - 2];
                let b = pts[pts.len() - 1];
                (b[1] - a[1]) / (b[0] - a[0])
            };
            // 軸上の視野の光線
            let max = lay
                .rays
                .iter()
                .filter(|(c, _)| *c == 1)
                .map(|(_, p)| slope(p).abs())
                .fold(0.0, f64::max);
            if neg {
                // 平行に近い（前側焦点は近似の位置、縁の光線は球面収差の分だけ開く）。入る光の傾きの 1/4 未満
                let first = |pts: &[[f64; 2]]| (pts[1][1] - pts[0][1]) / (pts[1][0] - pts[0][0]);
                let max_in = lay
                    .rays
                    .iter()
                    .map(|(_, p)| first(p).abs())
                    .fold(0.0, f64::max);
                assert!(max < 0.1 * max_in, "{part}: {max} / {max_in}");
                // 絞りは玉の前
                let stop = lay.stop.unwrap();
                let lens_first = lay.units[0].1[0][0];
                assert!(stop < lens_first, "{part}: stop {stop} {:?}", lay.units);
            } else {
                assert!(max > 0.02, "{part}: {max}");
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 負の単レンズは曲率の強い面を後ろ（平行光の出る側）へ向ける
    #[test]
    fn negative_preview_orientation() {
        // 平凹で凹面が前 → 反転、凹面が後ろ → そのまま
        let (rev, z) = negative_preview_focus(-50.0, 3.0, Some([-0.0388, 0.0]));
        assert!(rev);
        // 反転後の後ろの面は凹（c = 0.0388）、前側焦点は |f| より少し後ろ
        assert!(z > 50.0 && z < 53.0, "{z}");
        let (rev, z) = negative_preview_focus(-50.0, 3.0, Some([0.0, 0.0388]));
        assert!(!rev);
        assert!(z > 50.0 && z < 53.0, "{z}");
        // 曲率が無ければ反転せず、主点は玉の中ほど
        assert_eq!(negative_preview_focus(-50.0, 3.0, None), (false, 51.5));
    }

    /// カタログの鏡・BS の膜は組合せのブロックに写る
    #[test]
    fn coat_goes_to_block() {
        let mut r = lens("X", "M", f64::NAN, 25.4);
        r.kind = RowKind::Mirror;
        r.mirror = Some((0.0, 0.0));
        r.coat = "HR".into();
        let p = pick_from(&r);
        assert_eq!(p.block("M1")["coat"], "HR");
        r.kind = RowKind::Splitter;
        r.coat = "BS3".into();
        r.splitter = Some(json!({ "form": "cube", "size": 10.0, "glass": "N-SF11", "ar": "AR2" }));
        let p = pick_from(&r);
        assert_eq!((p.t, p.glass.as_str()), (10.0, "N-SF11"));
        let b = p.block("BS1");
        assert_eq!(
            (b["coat"].as_str(), b["ar"].as_str()),
            (Some("BS3"), Some("AR2"))
        );
        assert_eq!(b["size"], 10.0);
    }
}

#[cfg(test)]
mod preview_tilt_tests {
    use super::*;
    use serde_json::json;

    /// 45° の平面鏡のプレビュー: 絞りは鏡の手前の端より前、光線は絞りを通ってから鏡で折れる（#130）
    #[test]
    fn tilted_mirror_preview_stop_before_mirror() {
        let r = LensRow {
            vendor: "V".into(),
            part: "M".into(),
            efl: f64::INFINITY,
            diameter: 25.4,
            thickness: 6.35,
            mirror: Some((f64::INFINITY, 0.0)),
            ..Default::default()
        };
        let p = preview_project(&r);
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let glass = root.join("tests/glass").display().to_string();
        let dir =
            std::env::temp_dir().join(format!("lensbench-gui-mirprev-{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&dir).unwrap();
        let f = dir.join("p.json");
        lensbench_core::vfs::write(&f, p.to_string()).unwrap();
        let lay = crate::combo::LayoutData::parse(
            &lensbench_cli::serve::preview_layout(
                &json!({ "file": f.display().to_string(), "glass_dir": [glass] }),
                &json!({ "rays": 5 }),
            )
            .unwrap(),
        );
        std::fs::remove_dir_all(&dir).ok();
        assert!(!lay.rays.is_empty());
        // 光線は z = 0（絞り）から +z へ進んでから折れる（絞りより前で鏡に当たらない）
        for (_, pts) in &lay.rays {
            assert!(pts[1][0] > 0.5, "{pts:?}");
        }
    }

    /// カタログの玉は既定でロック、鏡・BS はロックしない（#123）
    #[test]
    fn catalog_lens_locked_by_default() {
        let lens = LensRow {
            vendor: "V".into(),
            part: "L".into(),
            efl: 50.0,
            diameter: 25.4,
            thickness: 3.0,
            pieces: 1,
            ..Default::default()
        };
        assert!(pick_from(&lens).locked);
        let mirror = LensRow {
            mirror: Some((f64::INFINITY, 0.0)),
            ..lens
        };
        assert!(!pick_from(&mirror).locked);
    }
}
