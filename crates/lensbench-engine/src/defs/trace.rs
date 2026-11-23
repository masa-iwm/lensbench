//! 光線の表・簡易 POP・機構のケラレ・玉の分け方

use std::f64::consts::PI;
use std::path::{Path, PathBuf};

use lensbench_analysis::pop;
use lensbench_core::{elements, Error, FieldSel, Result, System};
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
                        Err(Error::Invalid(format!("no {what} {i} (1 to {n})")))
                    }
                })
                .collect()
        })
        .transpose()
}

// ---------------------------------------------------------------------------------------- rays

/// Ray pattern
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum Fan {
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
pub(super) struct RaysParams {
    /// Field numbers (from 1, comma-separated; all fields if omitted)
    pub fields: Option<Vec<usize>>,
    /// Wavelength numbers (from 1, comma-separated; the primary wavelength if omitted)
    pub waves: Option<Vec<usize>>,
    /// All wavelengths (takes precedence over waves)
    pub all_waves: bool,
    /// Ray pattern
    pub fan: Fan,
    /// Number of rays (rays per direction for a fan or cross, the side for grid, the rays around for ring, plus the chief ray: n+1)
    #[schemars(extend("x-min" = 1))]
    pub rays: usize,
    /// Pupil coordinates directly (px,py,px,py,…). Takes precedence over fan and rays
    pub pupil: Option<Vec<f64>>,
    /// Surfaces to write (comma-separated; all if omitted)
    pub surface: Option<Vec<usize>>,
}

impl Default for RaysParams {
    fn default() -> Self {
        RaysParams {
            fields: None,
            waves: None,
            all_waves: false,
            fan: Fan::Grid,
            rays: 5,
            pupil: None,
            surface: None,
        }
    }
}

/// 光線の表（JSON は列ごとの並び。書き出しは拡張子で csv・json・npz）
pub(super) struct RaysOut {
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

pub(super) struct Rays;

impl Analysis for Rays {
    type Params = RaysParams;
    type Output = RaysOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "rays",
            about: "Intersections, directions and optical path lengths of rays at each surface (1 row = 1 ray × 1 surface)",
            help: "Intersections, directions and optical path lengths of rays at each surface (1 row = 1 ray × 1 surface). --save writes csv, json or npz",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &RaysParams) -> Result<RaysOut> {
        Ok(RaysOut {
            table: table::rays(cx.system, &ray_options(cx.system, p)?)?,
        })
    }
    fn save(&self, out: &RaysOut, path: &Path) -> Option<Result<Saved>> {
        Some(out.table.save(path).map(|_| {
            Saved::Written(
                json!({ "written": path.display().to_string(), "rows": out.table.rows() }),
            )
        }))
    }
    /// 書き出すときは表を全部作らずに書く（CSV・npz）
    fn run_to_file(&self, cx: &Cx<'_>, p: &RaysParams, path: &Path) -> Option<Result<Saved>> {
        Some(
            ray_options(cx.system, p)
                .and_then(|o| table::save_rays(cx.system, &o, path))
                .map(|rows| {
                    Saved::Written(json!({ "written": path.display().to_string(), "rows": rows }))
                }),
        )
    }
}

/// 光線の表の設定から追跡の条件を作る
fn ray_options(s: &lensbench_core::System, p: &RaysParams) -> Result<table::RayOptions> {
    let (px, py) = match &p.pupil {
        Some(v) => {
            if v.is_empty() || v.len() % 2 != 0 {
                return Err(Error::Invalid(
                    "pupil is a list of px,py pairs (an even count)".into(),
                ));
            }
            v.chunks(2).map(|c| (c[0], c[1])).unzip()
        }
        None => scene::pupil(p.fan.scene(), p.rays),
    };
    let img = s.image();
    if let Some(j) = p.surface.iter().flatten().find(|&&j| j == 0 || j > img) {
        return Err(Error::Invalid(format!(
            "surface {j} can't be written (1 to {img})"
        )));
    }
    let waves = if p.all_waves {
        Some((0..s.wavelengths.len()).collect())
    } else {
        indices(&p.waves, s.wavelengths.len(), "wavelength")?
    };
    Ok(table::RayOptions {
        fields: indices(&p.fields, s.fields.len(), "field")?,
        waves,
        px,
        py,
        surfaces: p.surface.clone(),
    })
}

// ----------------------------------------------------------------------------------------- pop

/// 簡易 POP の方法
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum PopHow {
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
pub(super) struct PopParams {
    /// Surface to build the field at (from 1; in the medium after it, on the tangent plane). If omitted, with only plane dummy surfaces in between
    /// The last surface that can be propagated to the image (propagates to the image if neither dist nor to is given)
    pub surf: Option<usize>,
    /// Field number (from 1)
    #[schemars(extend("x-min" = 1))]
    pub field: usize,
    /// Wavelength number (from 1; the primary wavelength if omitted)
    pub wave: Option<usize>,
    /// Distance to propagate (measured along the propagation direction)
    #[schemars(extend("x-unit" = "mm"))]
    pub dist: Option<f64>,
    /// Surface to propagate to (only plane dummy surfaces in between). The next surface if neither dist nor to is given
    pub to: Option<usize>,
    /// Method
    pub method: PopHow,
    /// Number of rays on a side of the pupil grid
    #[schemars(extend("x-min" = 2))]
    pub rays: usize,
    /// Number of samples on a side of the input grid
    #[schemars(extend("x-min" = 2))]
    pub n: usize,
    /// Half width of the input grid (the extent of the rays and aperture if omitted)
    #[schemars(extend("x-unit" = "mm"))]
    pub half: Option<f64>,
    /// Don't apply the surface aperture
    pub no_aperture: bool,
    /// Number of samples on a side of the output grid (same as the input if omitted)
    pub out_n: Option<usize>,
    /// Half width of the output grid (automatic if omitted)
    #[schemars(extend("x-unit" = "mm"))]
    pub out_half: Option<f64>,
    /// Center x,y of the output grid (the point advanced along the chief ray if omitted)
    #[schemars(length(min = 2, max = 2), extend("x-unit" = "mm"))]
    pub center: Option<Vec<f64>>,
    /// Also include the output irradiance grid in the result (grid_irradiance, index j·nx + i, j along +y)
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
pub(super) struct PopOut {
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
                Error::Invalid("give surf (no surface to propagate to the next one)".into())
            })?,
    };
    Ok((j, to.or(dist.is_none().then_some(t))))
}

pub(super) struct Pop;

impl Analysis for Pop {
    type Params = PopParams;
    type Output = PopOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "pop",
            about: "Simple POP: irradiance of a field built from rays, propagated in a uniform medium",
            help: "Simple POP: build a field from rays on the tangent plane of a surface (with the surface aperture applied) and propagate it in a uniform medium over the distance dist (to the vertex plane of the next surface if omitted); the irradiance. --save writes the grid (csv, npz) or everything (json)",
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
            _ => return Err(Error::Invalid("give only one of dist and to".into())),
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
                "the kernel phase changes by {:.2} rad between samples (more than π/2); increase n or narrow half and out_half",
                r.kernel_step
            ));
        }
        if r.fresnel_error > 0.01 && r.method == pop::PopMethod::Fresnel {
            warnings.push(format!(
                "the Fresnel approximation drops {:.2} rad of phase; use method direct",
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
        let ig = pb.irradiance_grid();
        let colm = |i: usize| -> Vec<f64> { ig.rows().map(|r| r[i]).collect() };
        let mut v = json!({
            "unit": "mm, W/mm² (power 1 in the pupil)",
            "surface": surf, "field": p.field, "wavelength_um": b.wl * 1e3, "index": b.index,
            "distance": d, "method": format!("{:?}", r.method).to_lowercase(),
            "rays_used": rb.used, "rays_traced": rb.traced,
            "aperture": rb.aperture.map(|a| format!("{a:?}")),
            "input": summary(b),
            "output": summary(pb),
            "kernel_step": r.kernel_step, "fresnel_error": r.fresnel_error,
            "x": ig.x(),
            "irradiance_x": ig.row(pb.ny / 2),
            "y": ig.y(),
            "irradiance_y": colm(pb.nx / 2),
            "warnings": warnings,
        });
        if p.grid {
            v["grid_irradiance"] = json!(ig.data());
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
        let g = pb.grid();
        let (mut xs, mut ys, mut ir, mut ph) = (vec![], vec![], vec![], vec![]);
        for (&y, r) in g.y().iter().zip(g.rows()) {
            for (&x, u) in g.x().iter().zip(r) {
                xs.push(x);
                ys.push(y);
                ir.push(u.norm_sqr());
                ph.push(u.arg());
            }
        }
        t.cols.push(("x".into(), table::Col::F64(xs)));
        t.cols.push(("y".into(), table::Col::F64(ys)));
        t.cols.push(("irradiance".into(), table::Col::F64(ir)));
        t.cols.push(("phase".into(), table::Col::F64(ph)));
        t.meta.insert(
            "note".into(),
            json!("phase is the phase without the pilot beam (output.pilot) [rad]"),
        );
        t.meta.insert("output".into(), v["output"].clone());
        Some(t.save(path).map(|_| {
            v["written"] = json!(path.display().to_string());
            Saved::Written(v)
        }))
    }
}

// ---------------------------------------------------------------------------------- mech-check

/// 機構のケラレの設定
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct MechCheckParams {
    /// Field numbers (from 1, comma-separated; all fields if omitted)
    pub fields: Option<Vec<usize>>,
    /// Wavelength numbers (from 1, comma-separated; all wavelengths if omitted)
    pub waves: Option<Vec<usize>>,
    /// Rays on a side of the pupil grid (points inside the unit circle are used)
    #[schemars(extend("x-min" = 2))]
    pub grid: usize,
}

impl Default for MechCheckParams {
    fn default() -> Self {
        MechCheckParams {
            fields: None,
            waves: None,
            grid: 21,
        }
    }
}

pub(super) struct MechCheck;

impl Analysis for MechCheck {
    type Params = MechCheckParams;
    type Output = Value;
    fn meta(&self) -> Meta {
        Meta {
            name: "mech-check",
            about: "Mechanical part vignetting: fraction of rays outside the part bores",
            help: "Vignetting by mechanical parts (the project's mechanics): trace the rays of the pupil grid and give the fraction outside the part bores (tube inner diameter, clear aperture of cage plates and mounts) per part, field and wavelength. Per field and wavelength, passed + optical + failed + the first clip of each part add up to 100 %",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &MechCheckParams) -> Result<Value> {
        let s = cx.system;
        if s.mechanics.is_empty() {
            return Err(Error::Invalid(
                "no mechanical parts (write them in the project's mechanics)".into(),
            ));
        }
        if p.grid < 2 {
            return Err(Error::Invalid("grid must be 2 or more".into()));
        }
        let all = |n: usize| (0..n).collect::<Vec<_>>();
        let fields = indices(&p.fields, s.fields.len(), "field")?.unwrap_or(all(s.fields.len()));
        let waves = indices(&p.waves, s.wavelengths.len(), "wavelength")?
            .unwrap_or(all(s.wavelengths.len()));
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
                    "failed_pct": pct(r.failed, r.launched),
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
        // mech_clip: ほかの解析でも部品で光線をけるか（系の設定）。各行の passed・optical・failed と部品ごとの first の
        // 割合を足すと 100 %
        Ok(json!({
            "pupil_rays": px.len(), "mech_clip": s.mech_clip,
            "parts": parts, "results": rows, "fit": fit,
        }))
    }
}

// ------------------------------------------------------------------------------------ elements

/// 玉モードの設定
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ElementsParams {
    /// Lens numbers to flip (from 1, comma-separated)
    pub reverse: Option<Vec<usize>>,
    /// .zmx to write the rebuilt system to
    #[schemars(extend("x-kind" = "path", "x-write" = true))]
    pub out: Option<PathBuf>,
}

pub(super) struct Elements;

impl Analysis for Elements {
    type Params = ElementsParams;
    type Output = Value;
    fn meta(&self) -> Meta {
        Meta {
            name: "elements",
            about: "Lens mode: show the system split into lenses, mirrors, coordinate breaks, other surfaces and air spaces",
            help: "Lens mode: show the system split into lenses, mirrors, coordinate breaks, other surfaces and air spaces. reverse flips lenses and out writes .zmx",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &ElementsParams) -> Result<Value> {
        let s = cx.system;
        let mut els = elements::split(s)?;
        let lenses: Vec<usize> = (0..els.len())
            .filter(|&i| els[i].kind == elements::Kind::Lens)
            .collect();
        for &k in p.reverse.iter().flatten() {
            let i = *lenses
                .get(k.wrapping_sub(1))
                .ok_or_else(|| Error::Invalid(format!("no lens {k} (1 to {})", lenses.len())))?;
            els[i] = elements::reverse(&els[i])?;
        }
        let a = elements::assemble(s, &els)?;
        let mut warnings = Vec::new();
        if let Some(path) = &p.out {
            warnings.extend(lensbench_formats::zmx_write::save(path, &a)?);
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
