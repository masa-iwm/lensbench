//! 光線の表・簡易 POP・機構のケラレ・玉の分け方

use std::f64::consts::PI;
use std::path::{Path, PathBuf};

use lensbench_core::{elements, pop, Error, FieldSel, Result, System};
use lensbench_export::{scene, table};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use lensbench_analysis::defs::{fields, wave};
use lensbench_analysis::{Analysis, Cx, Meta, Saved};

/// 番号の並び（1 始まり）を 0 始まりにする（None はそのまま）
fn indices(v: &Option<Vec<usize>>, n: usize, what: &str) -> Result<Option<Vec<usize>>> {
    v.as_ref()
        .map(|v| {
            v.iter()
                .map(|&i| {
                    if i >= 1 && i <= n {
                        Ok(i - 1)
                    } else {
                        Err(Error::Invalid(format!("{what} {i} はありません（1〜{n}）")))
                    }
                })
                .collect()
        })
        .transpose()
}

// ---------------------------------------------------------------------------------------- rays

/// 光線の並び
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Fan {
    /// y 方向の扇
    Y,
    /// x 方向の扇
    X,
    /// x と y の十字
    Cross,
    /// 正方格子（円の外は除く）
    #[default]
    Grid,
    /// 主光線と瞳の縁の円周
    Ring,
}

impl Fan {
    fn scene(self) -> scene::Fan {
        match self {
            Fan::Y => scene::Fan::Y,
            Fan::X => scene::Fan::X,
            Fan::Cross => scene::Fan::Cross,
            Fan::Grid => scene::Fan::Grid,
            Fan::Ring => scene::Fan::Ring,
        }
    }
}

/// 光線の表の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RaysParams {
    /// 視野の番号（1 始まり、カンマ区切り、省略で全視野）
    pub field: Option<Vec<usize>>,
    /// 波長の番号（1 始まり、カンマ区切り、省略で主波長）
    pub wave: Option<Vec<usize>>,
    /// 全波長（wave より優先）
    pub all_waves: bool,
    /// 光線の並び
    pub fan: Fan,
    /// 光線の本数（扇・十字は片方向の本数、grid は一辺、ring は周の本数で、主光線を足した n+1 本）
    #[schemars(extend("x-min" = 1))]
    pub rays: usize,
    /// 瞳座標を直接（px,py,px,py,…）。fan・rays より優先
    pub pupil: Option<Vec<f64>>,
    /// 書く面（カンマ区切り、省略で全部）
    pub surface: Option<Vec<usize>>,
}

impl Default for RaysParams {
    fn default() -> Self {
        RaysParams {
            field: None,
            wave: None,
            all_waves: false,
            fan: Fan::Grid,
            rays: 5,
            pupil: None,
            surface: None,
        }
    }
}

/// 光線の表（JSON は列ごとの並び。書き出しは拡張子で csv・json・npz）
pub struct RaysOut {
    table: table::Table,
}

impl Serialize for RaysOut {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.table.to_json().serialize(s)
    }
}

impl JsonSchema for RaysOut {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "RaysOut".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "object", "description": "光線の表（rows・columns）" })
    }
}

pub struct Rays;

impl Analysis for Rays {
    type Params = RaysParams;
    type Output = RaysOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "rays",
            about: "光線の面ごとの交点・方向・光路長（1 行 = 1 光線 × 1 面）",
            help: "光線の面ごとの交点・方向・光路長（1 行 = 1 光線 × 1 面）。--save で csv・json・npz に書き出す",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &RaysParams) -> Result<RaysOut> {
        let s = cx.system;
        let (px, py) = match &p.pupil {
            Some(v) => {
                if v.is_empty() || v.len() % 2 != 0 {
                    return Err(Error::Invalid(
                        "pupil は px,py の組の並び（数が偶数）".into(),
                    ));
                }
                v.chunks(2).map(|c| (c[0], c[1])).unzip()
            }
            None => scene::pupil(p.fan.scene(), p.rays),
        };
        let img = s.image();
        if let Some(j) = p.surface.iter().flatten().find(|&&j| j == 0 || j > img) {
            return Err(Error::Invalid(format!("面 {j} は書けない（1〜{img}）")));
        }
        let waves = if p.all_waves {
            Some((0..s.wavelengths.len()).collect())
        } else {
            indices(&p.wave, s.wavelengths.len(), "波長")?
        };
        let o = table::RayOptions {
            fields: indices(&p.field, s.fields.len(), "視野")?,
            waves,
            px,
            py,
            surfaces: p.surface.clone(),
        };
        Ok(RaysOut {
            table: table::rays(s, &o)?,
        })
    }
    fn save(&self, out: &RaysOut, path: &Path) -> Option<Result<Saved>> {
        Some(out.table.save(path).map_err(Error::Io).map(|_| {
            Saved::Written(
                json!({ "written": path.display().to_string(), "rows": out.table.rows() }),
            )
        }))
    }
}

// ----------------------------------------------------------------------------------------- pop

/// 簡易 POP の方法
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PopHow {
    /// 遠ければ fresnel、近ければ angular
    #[default]
    Auto,
    /// Fresnel の積分（近軸）
    Fresnel,
    /// 角スペクトル（FFT）
    Angular,
    /// Rayleigh-Sommerfeld の積分（近似なし・遅い）
    Direct,
}

/// 簡易 POP の設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PopParams {
    /// 場を作る面（1 始まり。その面の後の媒質の中、接平面の上）。省略で、間に平面のダミー面だけを挟んで
    /// 像面まで伝えられる最後の面（dist も to もなければ像面まで伝える）
    pub surf: Option<usize>,
    /// 視野の番号（1 始まり）
    #[schemars(extend("x-min" = 1))]
    pub field: usize,
    /// 波長の番号（1 始まり、省略で主波長）
    pub wave: Option<usize>,
    /// 伝える距離（伝搬の向きに測る）
    #[schemars(extend("x-unit" = "mm"))]
    pub dist: Option<f64>,
    /// 伝える先の面（間は平面のダミー面だけ）。dist も to もなければ次の面
    pub to: Option<usize>,
    /// 方法
    pub method: PopHow,
    /// 瞳の格子の一辺の光線の数
    #[schemars(extend("x-min" = 2))]
    pub rays: usize,
    /// 入力の格子の一辺の標本の数
    #[schemars(extend("x-min" = 2))]
    pub n: usize,
    /// 入力の格子の半幅（省略で光線と開口の範囲）
    #[schemars(extend("x-unit" = "mm"))]
    pub half: Option<f64>,
    /// 面の開口を掛けない
    pub no_aperture: bool,
    /// 出力の格子の一辺の標本の数（省略で入力と同じ）
    pub out_n: Option<usize>,
    /// 出力の格子の半幅（省略で自動）
    #[schemars(extend("x-unit" = "mm"))]
    pub out_half: Option<f64>,
    /// 出力の格子の中心 x,y（省略で主光線の向きに進めた点）
    #[schemars(length(min = 2, max = 2), extend("x-unit" = "mm"))]
    pub center: Option<Vec<f64>>,
    /// 結果に出力の照度の格子も入れる（grid_irradiance、番号 j·nx + i、j は +y の向き）
    pub grid: bool,
}

impl Default for PopParams {
    fn default() -> Self {
        PopParams {
            surf: None,
            field: 1,
            wave: None,
            dist: None,
            to: None,
            method: PopHow::Auto,
            rays: 65,
            n: 256,
            half: None,
            no_aperture: false,
            out_n: None,
            out_half: None,
            center: None,
            grid: false,
        }
    }
}

/// 簡易 POP の結果（JSON の形は鍵の名前で読む。書き出しの格子は保存のときだけ使う）
pub struct PopOut {
    v: Value,
    beam: pop::Beam,
}

impl Serialize for PopOut {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.v.serialize(s)
    }
}

impl JsonSchema for PopOut {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "PopOut".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "object", "description": "入力・出力の場の要約と断面の照度" })
    }
}

/// --surf を省いたときの (場を作る面, 伝える先): 像面まで伝えられる最後の面から像面まで。
/// 無ければ次の面まで伝えられる最後の面から次の面まで
fn pop_surfaces(
    s: &System,
    dist: Option<f64>,
    to: Option<usize>,
) -> Result<(usize, Option<usize>)> {
    let img = s.image();
    let ok = |j: usize, t: usize| {
        [1.0, -1.0]
            .iter()
            .any(|&dir| pop::distance_to(s, j, t, dir).is_ok())
    };
    let (j, t) = match (1..img).rev().find(|&j| ok(j, img)) {
        Some(j) => (j, img),
        None => (1..img)
            .rev()
            .find(|&j| ok(j, j + 1))
            .map(|j| (j, j + 1))
            .ok_or_else(|| {
                Error::Invalid("surf を指定する（次の面まで伝えられる面がない）".into())
            })?,
    };
    Ok((j, to.or(dist.is_none().then_some(t))))
}

pub struct Pop;

impl Analysis for Pop {
    type Params = PopParams;
    type Output = PopOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "pop",
            about: "簡易 POP: 光線から作った場を一様な媒質の中で伝えた照度",
            help: "簡易 POP: 光線から面の接平面の上に場を作り（面の開口を掛けて）、一様な媒質の中を距離 dist \
                   （省略で次の面の頂点面まで）伝えた照度。--save で格子（csv・npz）か全部（json）を書き出す",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &PopParams) -> Result<PopOut> {
        let s = cx.system;
        let (surf, to) = match p.surf {
            Some(j) => (j, p.to),
            None => pop_surfaces(s, p.dist, p.to)?,
        };
        let fi = fields(s, Some(p.field))?[0];
        let w = wave(s, p.wave)?;
        let m = match p.method {
            PopHow::Auto => pop::PopMethod::Auto,
            PopHow::Fresnel => pop::PopMethod::Fresnel,
            PopHow::Angular => pop::PopMethod::Angular,
            PopHow::Direct => pop::PopMethod::Direct,
        };
        let o = pop::RayBeamOpts {
            rays: p.rays,
            n: p.n,
            half: p.half,
            aperture: !p.no_aperture,
            overfill: None,
        };
        let rb = pop::beam_from_rays(s, FieldSel::Index(fi), w, surf, &o)?;
        let b = &rb.beam;
        let d = match (p.dist, to) {
            (Some(d), None) => d,
            (None, t) => pop::distance_to(s, surf, t.unwrap_or(surf + 1), b.dir)?,
            _ => return Err(Error::Invalid("dist と to はどちらか".into())),
        };
        let out = pop::OutGrid {
            n: p.out_n.map(|v| (v, v)),
            half: p.out_half.map(|v| (v, v)),
            center: p.center.as_ref().map(|c| (c[0], c[1])),
        };
        let r = pop::propagate(b, d, m, out)?;
        let pb = &r.beam;
        let mut warnings = Vec::new();
        if r.kernel_step > PI / 2.0 {
            warnings.push(format!(
                "核の位相の標本の間の差が {:.2} rad（π/2 超え）。n を増やすか half・out_half を狭める",
                r.kernel_step
            ));
        }
        if r.fresnel_error > 0.01 && r.method == pop::PopMethod::Fresnel {
            warnings.push(format!(
                "Fresnel 近似で落とす位相が {:.2} rad。method direct を使う",
                r.fresnel_error
            ));
        }
        let summary = |b: &pop::Beam| {
            let (cx, cy) = b.centroid();
            json!({
                "n": [b.nx, b.ny], "dx": b.dx, "dy": b.dy, "center": [b.x0, b.y0],
                "pilot": { "ax": b.pilot.ax, "ay": b.pilot.ay, "curv": b.pilot.curv },
                "power": b.power(), "peak": b.peak(), "centroid": [cx, cy],
                "rms_width": b.rms_width(), "phase_step": b.phase_step(),
            })
        };
        let row =
            |j: usize| -> Vec<f64> { (0..pb.nx).map(|i| pb.u[j * pb.nx + i].norm_sqr()).collect() };
        let colm =
            |i: usize| -> Vec<f64> { (0..pb.ny).map(|j| pb.u[j * pb.nx + i].norm_sqr()).collect() };
        let mut v = json!({
            "unit": "mm, W/mm² (power 1 in the pupil)",
            "surface": surf, "field": p.field, "wavelength_um": b.wl * 1e3, "index": b.index,
            "distance": d, "method": format!("{:?}", r.method).to_lowercase(),
            "rays_used": rb.used, "rays_traced": rb.traced,
            "aperture": rb.aperture.map(|a| format!("{a:?}")),
            "input": summary(b),
            "output": summary(pb),
            "kernel_step": r.kernel_step, "fresnel_error": r.fresnel_error,
            "x": (0..pb.nx).map(|i| pb.x(i)).collect::<Vec<_>>(),
            "irradiance_x": row(pb.ny / 2),
            "y": (0..pb.ny).map(|j| pb.y(j)).collect::<Vec<_>>(),
            "irradiance_y": colm(pb.nx / 2),
            "warnings": warnings,
        });
        if p.grid {
            v["grid_irradiance"] = json!(pb.irradiance());
        }
        Ok(PopOut { v, beam: r.beam })
    }
    fn save(&self, out: &PopOut, path: &Path) -> Option<Result<Saved>> {
        let pb = &out.beam;
        let mut v = out.v.clone();
        if path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        {
            v["grid"] = json!({
                "irradiance": pb.irradiance(),
                "phase": pb.u.iter().map(|c| c.arg()).collect::<Vec<_>>(),
            });
            return Some(Ok(Saved::Json(v)));
        }
        let mut t = table::Table::default();
        let (mut xs, mut ys, mut ir, mut ph) = (vec![], vec![], vec![], vec![]);
        for j in 0..pb.ny {
            for i in 0..pb.nx {
                xs.push(pb.x(i));
                ys.push(pb.y(j));
                ir.push(pb.u[j * pb.nx + i].norm_sqr());
                ph.push(pb.u[j * pb.nx + i].arg());
            }
        }
        t.cols.push(("x".into(), table::Col::F64(xs)));
        t.cols.push(("y".into(), table::Col::F64(ys)));
        t.cols.push(("irradiance".into(), table::Col::F64(ir)));
        t.cols.push(("phase".into(), table::Col::F64(ph)));
        t.meta.insert(
            "note".into(),
            json!("phase はパイロット（output.pilot）を除いた位相 [rad]"),
        );
        t.meta.insert("output".into(), v["output"].clone());
        Some(t.save(path).map_err(Error::Io).map(|_| {
            v["written"] = json!(path.display().to_string());
            Saved::Written(v)
        }))
    }
}

// ---------------------------------------------------------------------------------- mech-check

/// 機構のケラレの設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MechCheckParams {
    /// 視野の番号（1 始まり、カンマ区切り、省略で全視野）
    pub field: Option<Vec<usize>>,
    /// 波長の番号（1 始まり、カンマ区切り、省略で全波長）
    pub wave: Option<Vec<usize>>,
    /// 瞳の格子の一辺の本数（単位円の中の点を使う）
    #[schemars(extend("x-min" = 2))]
    pub grid: usize,
}

impl Default for MechCheckParams {
    fn default() -> Self {
        MechCheckParams {
            field: None,
            wave: None,
            grid: 21,
        }
    }
}

pub struct MechCheck;

impl Analysis for MechCheck {
    type Params = MechCheckParams;
    type Output = Value;
    fn meta(&self) -> Meta {
        Meta {
            name: "mech-check",
            about: "機構部品のケラレ: 部品の穴の外に出た光線の割合",
            help: "機構部品（プロジェクトの mechanics）のケラレ: 瞳の格子の光線を追い、部品の穴（チューブの内径・\
                   ケージ板やマウントのクリア開口）の外に出た割合を部品ごと・視野と波長ごとに出す",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &MechCheckParams) -> Result<Value> {
        let s = cx.system;
        if s.mechanics.is_empty() {
            return Err(Error::Invalid(
                "機構部品がない（プロジェクトの mechanics に書く）".into(),
            ));
        }
        if p.grid < 2 {
            return Err(Error::Invalid("grid は 2 以上".into()));
        }
        let all = |n: usize| (0..n).collect::<Vec<_>>();
        let fields = indices(&p.field, s.fields.len(), "視野")?.unwrap_or(all(s.fields.len()));
        let waves =
            indices(&p.wave, s.wavelengths.len(), "波長")?.unwrap_or(all(s.wavelengths.len()));
        let (px, py) = lensbench_core::mech::pupil_grid(p.grid);
        let v = lensbench_core::mech::vignetting(s, &s.mechanics, &fields, &waves, &px, &py)?;
        let pct = |n: usize, d: usize| {
            if d == 0 {
                0.0
            } else {
                100.0 * n as f64 / d as f64
            }
        };
        let parts: Vec<Value> = s
            .mechanics
            .iter()
            .enumerate()
            .map(|(k, m)| {
                let worst = v.iter().map(|r| pct(r.clipped[k], r.launched)).fold(0.0, f64::max);
                let first = v.iter().map(|r| pct(r.first[k], r.launched)).fold(0.0, f64::max);
                json!({
                    "name": m.name, "surface": m.surface,
                    "clipped_max_pct": worst, "first_max_pct": first,
                    "bores": m.bores.iter().map(|b| json!({"z0": b.z0, "z1": b.z1, "diameter": 2.0 * b.radius})).collect::<Vec<_>>(),
                })
            })
            .collect();
        let rows: Vec<Value> = v
            .iter()
            .map(|r| {
                json!({
                    "field": r.field + 1, "wave": r.wave + 1,
                    "rays": r.launched, "failed": r.failed,
                    "optical_pct": pct(r.optical, r.launched),
                    "passed_pct": pct(r.passed, r.launched),
                    "parts": s.mechanics.iter().enumerate().map(|(k, m)| json!({
                        "name": m.name,
                        "clipped_pct": pct(r.clipped[k], r.launched),
                        "first_pct": pct(r.first[k], r.launched),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        let fit: Vec<Value> = lensbench_core::mech::fit_check(s, &s.mechanics)?
            .iter()
            .map(|f| {
                json!({
                    "kind": f.kind, "error": f.error,
                    "part": s.mechanics[f.part].name,
                    "other": f.other.map(|o| s.mechanics[o].name.clone()),
                    "surface": f.surface, "value": f.value, "message": f.message,
                })
            })
            .collect();
        Ok(json!({ "pupil_rays": px.len(), "parts": parts, "results": rows, "fit": fit }))
    }
}

// ------------------------------------------------------------------------------------ elements

/// 玉モードの設定
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ElementsParams {
    /// 反転する玉の番号（1 始まり、カンマ区切り）
    pub reverse: Option<Vec<usize>>,
    /// 組み直した系を書き出す .zmx
    #[schemars(extend("x-kind" = "path", "x-write" = true))]
    pub out: Option<PathBuf>,
}

pub struct Elements;

impl Analysis for Elements {
    type Params = ElementsParams;
    type Output = Value;
    fn meta(&self) -> Meta {
        Meta {
            name: "elements",
            about: "玉モード: 系を玉・鏡・座標ブレーク・その他の面と空気間隔に分けて示す",
            help: "玉モード: 系を玉・鏡・座標ブレーク・その他の面と空気間隔に分けて示す。reverse で玉を反転し、out で .zmx に書き出す",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &ElementsParams) -> Result<Value> {
        let s = cx.system;
        let mut els = elements::split(s)?;
        let lenses: Vec<usize> = (0..els.len())
            .filter(|&i| els[i].kind == elements::Kind::Lens)
            .collect();
        for &k in p.reverse.iter().flatten() {
            let i = *lenses.get(k.wrapping_sub(1)).ok_or_else(|| {
                Error::Invalid(format!("玉 {k} はありません（1〜{}）", lenses.len()))
            })?;
            els[i] = elements::reverse(&els[i])?;
        }
        let a = elements::assemble(s, &els)?;
        let mut warnings = Vec::new();
        if let Some(path) = &p.out {
            warnings.extend(lensbench_core::zmx_write::save(path, &a)?);
        }
        let w = s.primary;
        let mut nl = 0;
        let v: Vec<Value> = els
            .iter()
            .map(|x| {
                let kind = match x.kind {
                    elements::Kind::Lens => "lens",
                    elements::Kind::Mirror => "mirror",
                    elements::Kind::CoordBreak => "coordinate_break",
                    elements::Kind::Surface => "surface",
                    elements::Kind::Air => "air",
                };
                let mut o = json!({ "kind": kind });
                if x.kind == elements::Kind::Air {
                    o["thickness"] = json!(x.t);
                } else {
                    o["surfaces"] = json!([x.from, x.from + x.surfaces.len() - 1]);
                }
                if x.kind == elements::Kind::Lens {
                    nl += 1;
                    o["lens"] = json!(nl);
                    o["materials"] = json!(x.materials());
                    o["length"] = json!(x.length());
                    o["efl"] = json!(x.efl(w));
                    o["reversed"] = json!(x.reversed);
                }
                o
            })
            .collect();
        Ok(json!({
            "elements": v,
            "efl": a.paraxial().efl,
            "written": p.out.as_ref().map(|p| p.display().to_string()),
            "warnings": warnings,
        }))
    }
}
