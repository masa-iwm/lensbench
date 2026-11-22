//! 系を .zmx（OpticStudio のレンズファイル）に書き出す。読込（[`crate::zmx`]）が解釈する項目だけを書く。
//!
//! 厚さのソルブ（[`System::solves`](lensbench_core::system::Prescription::solves)）は解いた値と MAZH の行を書き、ほかのソルブは値にして書く。ピックアップ（[`System::pickups`](lensbench_core::system::Prescription::pickups)）は解決した値とピックアップの行の両方を書く。モデルガラス（分散なし）は ___BLANK、ほかのガラスは名前とカタログ（GCAT）。

use std::fmt::Write as _;
use std::path::Path;

use lensbench_core::error::{Error, Result};
use lensbench_core::notimpl;
use lensbench_core::param::Kind;
use lensbench_core::solves::{Solve, SolveKind};
use lensbench_core::surface::{After, Aperture, Bire, Material, SurfType, Tilt};
use lensbench_core::system::{Aiming, ApertureKind, FieldNorm, FieldType, System};

mod configs;
use configs::{config_notes, config_op, config_pick_unwritable, config_row_numbers};

/// 数値を読み戻しで同じ値になる表記で（Rust の最短表記）
fn f(v: f64) -> String {
    if v.is_infinite() {
        "INFINITY".into()
    } else {
        let s = format!("{v:?}");
        s.strip_suffix(".0").map(|x| x.to_string()).unwrap_or(s)
    }
}

/// ピックアップの元の列のコード（0 = 同じ列）
fn col(q: &lensbench_core::pickup::Pickup) -> i64 {
    q.from.and_then(|k| k.column()).unwrap_or(0)
}

fn tilt_line(out: &mut String, which: u8, mode: usize, t: &Tilt) {
    let _ = writeln!(
        out,
        "  SCBD {which} {} {mode} {} {} {} {} {}",
        t.order,
        f(t.d[0]),
        f(t.d[1]),
        f(t.d[2]),
        f(t.d[3]),
        f(t.d[4])
    );
}

/// .zmx の本文（改行は CRLF）。GRIN の面の直後に座標ブレークがある系（GRIN の面だけを動かす玉・群を含む）も
/// そのまま書くが、OpticStudio 26.1 はその系を「GRIN の面の後ろに置けない面」としてエラーにする
pub fn write(s: &System) -> Result<String> {
    write_as(s, DEFAULT_COAT_FILE)
}

/// 系に書いた膜の表（[`System::coat_tables`](lensbench_core::system::Prescription::coat_tables)）を書き出すときの膜ファイルの名前（[`write`] の既定。[`save`] は
/// `.zmx` の名前から決める）
pub(crate) const DEFAULT_COAT_FILE: &str = "lensbench_coat.dat";

/// [`write`] で、系に書いた膜の表を `coat_file` の名前の膜ファイルに書く前提で COFN を書く（膜ファイルの中身は
/// [`coating_dat`]）。表を使う面がなければ `coat_file` は使わない
/// 面の前の行（系の値: 版・名前・単位・開口・視野・波長・膜ファイルなど）。系の値が同じかを面を書かずに比べるのにも使う
fn write_header(s: &System, coat_file: &str) -> Result<String> {
    let mut o = String::new();
    let w = |o: &mut String, l: String| {
        o.push_str(&l);
        o.push('\n');
    };
    w(&mut o, "VERS 260327 42 20120530 20120530".into());
    w(&mut o, "MODE SEQ".into());
    w(&mut o, format!("NAME {}", s.name));
    w(
        &mut o,
        format!("UNIT MM X W X CM {} CPMM LU", s.afocal_unit.code()),
    );
    match s.aperture.0 {
        ApertureKind::Epd => w(&mut o, format!("ENPD {}", f(s.aperture.1))),
        ApertureKind::FNum => w(&mut o, format!("FNUM {} 0", f(s.aperture.1))),
        ApertureKind::Pwfn => w(&mut o, format!("FNUM {} 1", f(s.aperture.1))),
        ApertureKind::Na => w(&mut o, format!("OBNA {} 0", f(s.aperture.1))),
        ApertureKind::Cone => w(&mut o, format!("OBNA {} 1", f(s.aperture.1))),
        ApertureKind::Float => w(&mut o, "FLOA".into()),
    }
    w(
        &mut o,
        format!(
            "ENVD {} {} {}",
            f(s.temperature),
            f(s.pressure),
            if s.adjust_index { 1 } else { 0 }
        ),
    );
    // 使っているガラスのカタログ（出てきた順）
    let mut cats: Vec<String> = Vec::new();
    for d in &s.surfaces {
        if let Some(g) = d.mat.glass().and_then(|m| s.glasses.get(m)) {
            if !g.catalog.is_empty() && !cats.contains(&g.catalog) {
                cats.push(g.catalog.clone());
            }
        }
    }
    w(&mut o, format!("GCAT {}", cats.join(" ")));
    let aim = match s.aiming {
        Aiming::Off => 0,
        Aiming::Paraxial => 1,
        Aiming::Real => 2,
    };
    w(&mut o, format!("RAIM 0 {aim} 1 1 0 0 0 0 0 1"));
    let ft = match s.field_type {
        FieldType::Angle => 0,
        FieldType::ObjH => 1,
        FieldType::ParH => 2,
        FieldType::RealH => 3,
        FieldType::Solve(c) => {
            notimpl!("a field set by a condition ({c:?}) can't be written to .zmx (OpticStudio has none)")
        }
    };
    let nf = s.fields.len();
    w(
        &mut o,
        format!(
            // FTYP 種類 テレセントリック 視野の数 波長の数 正規化 Iterate Solves Afocal 視野の数（OpticStudio 2026 R1.01 の書き方。
            // 8 番目は 3 番目と同じ数で、なくても読みは変わらない）
            "FTYP {ft} {} {nf} {} {} 0 {} {nf}",
            if s.telecentric { 1 } else { 0 },
            s.wavelengths.len(),
            if s.field_norm == FieldNorm::Rect {
                1
            } else {
                0
            },
            if s.afocal { 1 } else { 0 }
        ),
    );
    w(&mut o, format!("ROPD {}", s.ropd));
    // 瞳のアポダイゼーション GFAC 係数 種類（OpticStudio は一様でも GFAC 0 0 と書くが、読む側の既定も一様なので
    // 一様なら書かない）
    match s.apodization {
        lensbench_core::pupil::Apodization::Uniform => {}
        lensbench_core::pupil::Apodization::Gaussian(g) => w(&mut o, format!("GFAC {} 1", f(g))),
        lensbench_core::pupil::Apodization::CosineCubed => w(&mut o, "GFAC 0 2".to_string()),
    }
    let row = |key: &str, g: &dyn Fn(&lensbench_core::system::Field) -> f64| -> String {
        let v: Vec<String> = s.fields.iter().map(|x| f(g(x))).collect();
        format!("{key} {}", v.join(" "))
    };
    w(&mut o, row("XFLN", &|x| x.x));
    w(&mut o, row("YFLN", &|x| x.y));
    w(&mut o, row("FWGN", &|x| x.w));
    w(&mut o, row("VDXN", &|x| x.vdx));
    w(&mut o, row("VDYN", &|x| x.vdy));
    w(&mut o, row("VCXN", &|x| x.vcx));
    w(&mut o, row("VCYN", &|x| x.vcy));
    w(&mut o, row("VANN", &|x| x.van));
    for (i, (l, q)) in s.wavelengths.iter().zip(&s.weights).enumerate() {
        w(&mut o, format!("WAVM {} {} {}", i + 1, f(*l), f(*q)));
    }
    w(&mut o, format!("PWAV {}", s.primary + 1));
    w(&mut o, format!("GLRS {} 0", s.glrs));
    // Gradium の分布のファイル（COFN の 4 番目）。既定の PROFILE.GRD 以外なら書く
    let grd = s
        .surfaces
        .iter()
        .find_map(|d| match d.grin().map(|g| &g.kind) {
            Some(lensbench_core::grin::GrinKind::Gradium { prof, .. })
                if !prof.file.eq_ignore_ascii_case("PROFILE.GRD") =>
            {
                Some(prof.file.clone())
            }
            _ => None,
        });
    // 系に書いた膜の表（プロジェクト・カタログの coatings）を使う面があれば、表を足した膜ファイル（coat_file）を指す
    let coat = if uses_coat_tables(s) {
        Some(coat_file.to_string())
    } else {
        s.coating_file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|c| c.to_string_lossy().to_string())
    };
    match (coat, grd) {
        (Some(c), None) => {
            w(&mut o, format!("COFN QF \"{c}\""));
            w(&mut o, format!("COFN {c}"));
        }
        (c, Some(g)) => {
            let c = c.unwrap_or_else(|| "COATING.DAT".into());
            w(
                &mut o,
                format!("COFN QF \"{c}\" \"SCATTER_PROFILE.DAT\" \"ABG_DATA.DAT\" \"{g}\""),
            );
            w(
                &mut o,
                format!("COFN {c} SCATTER_PROFILE.DAT ABG_DATA.DAT {g}"),
            );
        }
        (None, None) => {}
    }
    Ok(o)
}

pub(crate) fn write_as(s: &System, coat_file: &str) -> Result<String> {
    if s.units.iter().any(|u| u.moved()) {
        // 玉・群の剛体の動きは座標ブレークにして書く（面の番号が変わる）
        // 面の番号が変わるので多重構成の表は書かない（[`write_notes`] に注意）
        let mut f = s.flatten_units()?.into_prescription();
        f.configs = None;
        return write_as(&f.build_raw()?, coat_file);
    }
    let mut o = write_header(s, coat_file)?;
    let w = |o: &mut String, l: String| {
        o.push_str(&l);
        o.push('\n');
    };
    for (k, d) in s.surfaces.iter().enumerate() {
        if d.dn != 0.0 || d.disp != 0.0 {
            notimpl!(
                "surface {k}: an index offset (dn {}, disp {}) can't be written to .zmx",
                d.dn,
                d.disp
            );
        }
        if d.typ() == SurfType::User && !d.ignore {
            notimpl!("surface {k}: a user-defined surface can't be written to .zmx (OpticStudio has none)");
        }
        if d.typ() == SurfType::UserShape && !d.ignore {
            notimpl!(
                "surface {k}: a user-defined shape can't be written to .zmx (OpticStudio has none)"
            );
        }
        w(&mut o, format!("SURF {k}"));
        let typ = match d.typ() {
            SurfType::Standard => "STANDARD",
            SurfType::EvenAsph => "EVENASPH",
            SurfType::CoordBrk => "COORDBRK",
            SurfType::Jones => "JONESMAT",
            SurfType::Paraxial => "PARAXIAL",
            SurfType::Toroidal => "TOROIDAL",
            SurfType::ZernikeStd => "SZERNSAG",
            SurfType::ZernikeFringe => "FZERNSAG",
            SurfType::GridSag => "GRID_SAG",
            SurfType::Fresnel => "FRESNELS",
            SurfType::CylFresnel => "CFRESNEL",
            SurfType::Binary2 => "BINARY_2",
            SurfType::Gradium => "GRINSUR8",
            SurfType::Gradient9 => "GRINSUR9",
            SurfType::OddAsph => "ODDASPHE",
            SurfType::TiltSurf => "TILTSURF",
            SurfType::Irregular => "IRREGULA",
            // 無視する面は標準面として書く
            SurfType::User | SurfType::UserShape => "STANDARD",
        };
        // 複屈折材の板（形は標準面）: OpticStudio 26.1 の書き方（OpticStudio で書き出した .zmx で確認）
        let bire = match (&d.bire, d.typ()) {
            (Some(b), SurfType::Standard) => Some(b),
            (Some(_), t) => {
                notimpl!("surface {k}: the birefringent plate surface is {t:?} (standard only)")
            }
            (None, _) => None,
        };
        match bire {
            Some(Bire::In { .. }) => w(&mut o, "  TYPE BIRE__IN \"\"".into()),
            Some(Bire::Out) => w(&mut o, "  TYPE BIRE_OUT".into()),
            None => w(&mut o, format!("  TYPE {typ}")),
        }
        if !d.comment.is_empty() {
            w(&mut o, format!("  COMM {}", d.comment));
        }
        if d.stop {
            w(&mut o, "  STOP".into());
        }
        if d.ignore {
            // OpticStudio の書き方: HIDE の 6 番目の値が面の無視
            w(&mut o, "  HIDE 0 0 0 0 0 1 0 0 0 0 0 0 0".into());
        }
        match s
            .pickups
            .iter()
            .find(|q| q.dst == k && q.what == Kind::Curv)
        {
            Some(q) => w(
                &mut o,
                format!(
                    "  CURV {} 4 {} {} {} \"\"",
                    f(d.c),
                    q.src,
                    f(q.scale),
                    col(q)
                ),
            ),
            None => match s.solves.iter().find(|q| q.surf == k && q.kind.is_curv()) {
                // ZPL マクロのソルブ: CURV 曲率 12 0 0 0 "名前"
                Some(Solve {
                    kind: SolveKind::MacroCurv { index: i },
                    ..
                }) => {
                    let name = s
                        .macros
                        .get(*i as usize)
                        .map(|m| m.name.as_str())
                        .unwrap_or("");
                    w(&mut o, format!("  CURV {} 12 0 0 0 \"{name}\"", f(d.c)))
                }
                // 曲率のソルブ: CURV 曲率 種類 値 0 0 ""（OpticStudio 2026 R1.01 の書き方）
                Some(q) => {
                    let (code, v) = match q.kind {
                        SolveKind::MarginalAngle { .. } => (2, q.kind.value()),
                        SolveKind::ChiefAngle { .. } => (3, q.kind.value()),
                        SolveKind::MarginalNormal => (5, 0.0),
                        SolveKind::ChiefNormal => (6, 0.0),
                        SolveKind::Aplanatic => (7, 0.0),
                        SolveKind::ElementPower { .. } => (8, q.kind.value()),
                        SolveKind::ConcentricSurface { other: j } => (9, j as f64),
                        SolveKind::ConcentricRadius { other: j } => (10, j as f64),
                        SolveKind::FNumber { .. } => (11, q.kind.value()),
                        _ => unreachable!(),
                    };
                    w(
                        &mut o,
                        format!("  CURV {} {code} {} 0 0 \"\"", f(d.c), f(v)),
                    )
                }
                None => w(&mut o, format!("  CURV {}", f(d.c))),
            },
        }
        if d.k != 0.0 {
            w(&mut o, format!("  CONI {}", f(d.k)));
        }
        match bire {
            Some(Bire::In { mode, axis, .. }) => {
                // Draw Axis・Mode・結晶軸・Parax Ignore（OS の既定 1）・Shape、Gradient Mode は XDAT 1
                let mut v = vec![0.0, *mode as f64, axis[0], axis[1], axis[2], 1.0];
                v.resize(13, 0.0);
                for (i, a) in v.iter().enumerate() {
                    w(&mut o, format!("  PARM {i} {}", f(*a)));
                }
                w(&mut o, "  XDAT 1 0 0 0 1 0 0 \"\"".into());
            }
            Some(Bire::Out) => {
                for i in 1..=12 {
                    w(&mut o, format!("  PARM {i} 0"));
                }
            }
            None => {}
        }
        match d.typ() {
            SurfType::EvenAsph
            | SurfType::Fresnel
            | SurfType::OddAsph
            | SurfType::TiltSurf
            | SurfType::Irregular => {
                for (i, a) in d.asph().iter().enumerate() {
                    w(&mut o, format!("  PARM {} {}", i + 1, f(*a)));
                }
            }
            SurfType::Binary2 => {
                // PARM 0 は回折次数、1〜8 は r^2〜r^16。XDAT 1 は項の数、2 は正規化半径、k+2 は ρ^{2k} の係数
                let Some(b) = &d.bin2() else {
                    notimpl!("surface {k}: no Binary 2 phase");
                };
                w(&mut o, format!("  PARM 0 {}", f(b.order)));
                for (i, a) in d.asph().iter().enumerate() {
                    w(&mut o, format!("  PARM {} {}", i + 1, f(*a)));
                }
                w(&mut o, format!("  XDAT 1 {} 0 0 1 0 0 \"\"", b.coef.len()));
                w(&mut o, format!("  XDAT 2 {} 0 0 1 0 0 \"\"", f(b.norm)));
                for (i, a) in b.coef.iter().enumerate() {
                    w(&mut o, format!("  XDAT {} {} 0 0 1 0 0 \"\"", i + 3, f(*a)));
                }
            }
            SurfType::CylFresnel => {
                // PARM 1〜8 は基板の y^2〜y^16。XDAT 1 は項の数、2・3 は屈折の面の曲率・円錐係数、k+3 は y^{2k} の係数
                let Some(cf) = &d.cylf() else {
                    notimpl!("surface {k}: no Cylinder Fresnel surface");
                };
                for (i, a) in d.asph().iter().enumerate() {
                    w(&mut o, format!("  PARM {} {}", i + 1, f(*a)));
                }
                w(&mut o, format!("  XDAT 1 {} 0 0 1 0 0 \"\"", cf.coef.len()));
                w(&mut o, format!("  XDAT 2 {} 0 0 1 0 0 \"\"", f(cf.c)));
                w(&mut o, format!("  XDAT 3 {} 0 0 1 0 0 \"\"", f(cf.k)));
                for (i, a) in cf.coef.iter().enumerate() {
                    w(&mut o, format!("  XDAT {} {} 0 0 1 0 0 \"\"", i + 4, f(*a)));
                }
            }
            SurfType::Gradium | SurfType::Gradient9 => {
                // GRINSUR8: PARM 0 素材の厚さ、1 Δt、2 Δz、3 頂点の基準の屈折率（表示用）、4〜7 dx dy tx ty、8 端の扱い。
                // GRINSUR9: PARM 1 Δt、2 tan α、3 tan β。分布・材料の名前は GLAS
                let Some(g) = &d.grin() else {
                    notimpl!("surface {k}: no GRIN profile");
                };
                match &g.kind {
                    lensbench_core::grin::GrinKind::Gradium {
                        prof, dz, dec, cap, ..
                    } => {
                        if prof.flipped {
                            let name = &prof.name;
                            notimpl!("surface {k}: the Gradium profile {name} is flipped (after a mirror or in a reversed lens) and differs from the profile file, so it can't be written to .zmx");
                        }
                        w(&mut o, format!("  PARM 0 {}", f(prof.zmax)));
                        w(&mut o, format!("  PARM 1 {}", f(g.dt)));
                        w(&mut o, format!("  PARM 2 {}", f(*dz)));
                        w(
                            &mut o,
                            format!("  PARM 3 {}", f(g.ref_index().unwrap_or(0.0))),
                        );
                        for (i, a) in dec.iter().enumerate() {
                            w(&mut o, format!("  PARM {} {}", i + 4, f(*a)));
                        }
                        w(&mut o, format!("  PARM 8 {cap}"));
                    }
                    lensbench_core::grin::GrinKind::Grad9 { tan, .. } => {
                        w(&mut o, format!("  PARM 1 {}", f(g.dt)));
                        w(&mut o, format!("  PARM 2 {}", f(tan[0])));
                        w(&mut o, format!("  PARM 3 {}", f(tan[1])));
                    }
                }
            }
            // 番号と値の対応は Surface::parm（整数の欄は整数で書く）
            SurfType::CoordBrk => {
                for i in 1..=5 {
                    w(
                        &mut o,
                        format!("  PARM {i} {}", f(d.parm(i).unwrap_or(0.0))),
                    );
                }
                w(&mut o, format!("  PARM 6 {}", d.cb_order()));
            }
            SurfType::Jones => {
                for i in 1..=8 {
                    w(
                        &mut o,
                        format!("  PARM {i} {}", f(d.parm(i).unwrap_or(0.0))),
                    );
                }
            }
            SurfType::Paraxial => {
                w(&mut o, format!("  PARM 1 {}", f(d.parm(1).unwrap_or(0.0))));
                w(&mut o, format!("  PARM 2 {}", d.opd_mode()));
            }
            SurfType::User | SurfType::UserShape => {}
            SurfType::Toroidal => {
                w(&mut o, format!("  PARM 1 {}", f(d.parm(1).unwrap_or(0.0))));
                for (i, a) in d.asph().iter().enumerate() {
                    w(&mut o, format!("  PARM {} {}", i + 2, f(*a)));
                }
                // Zernike の項の数 0、正規化半径
                w(&mut o, "  XDAT 1 0 0 0 1 0 0 \"\"".into());
                w(&mut o, "  XDAT 2 100 0 0 1 0 0 \"\"".into());
            }
            SurfType::ZernikeStd | SurfType::ZernikeFringe | SurfType::GridSag => {
                // OpticStudio 26.1 の書き方（OpticStudio で書き出した .zmx で確認）: PARM 0 は外挿・補間、1〜8 は r^2〜r^16、
                // 9・10 は Zernike の偏心。XDAT 1 は項の数、2 は正規化半径、k+2 は項 k。Grid は GDAT・GARR を埋め込む
                let z = d.zern();
                let p0 = match (&d.grid(), z) {
                    (Some(g), _) => {
                        if g.linear {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    (None, Some(z)) => {
                        if z.extrapolate {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    (None, None) => 1.0,
                };
                if d.typ() == SurfType::GridSag && z.is_some_and(|z| !z.extrapolate) {
                    notimpl!("surface {k}: the Grid Sag Zernike terms can only be extrapolated");
                }
                if d.asph().len() > 8 {
                    notimpl!("surface {k}: terms of r^18 or higher");
                }
                w(&mut o, format!("  PARM 0 {}", f(p0)));
                for i in 0..8 {
                    w(
                        &mut o,
                        format!(
                            "  PARM {} {}",
                            i + 1,
                            f(d.asph().get(i).copied().unwrap_or(0.0))
                        ),
                    );
                }
                w(&mut o, format!("  PARM 9 {}", f(z.map_or(0.0, |z| z.dx))));
                w(&mut o, format!("  PARM 10 {}", f(z.map_or(0.0, |z| z.dy))));
                let (nt, norm) = z.map_or((0, 100.0), |z| (z.coef.len(), z.norm));
                w(&mut o, format!("  XDAT 1 {nt} 0 0 1 0 0 \"\""));
                w(&mut o, format!("  XDAT 2 {} 0 0 1 0 0 \"\"", f(norm)));
                if let Some(z) = z {
                    for (i, a) in z.coef.iter().enumerate() {
                        w(&mut o, format!("  XDAT {} {} 0 0 1 0 0 \"\"", i + 3, f(*a)));
                    }
                }
                if d.typ() == SurfType::GridSag {
                    let Some(g) = &d.grid() else {
                        notimpl!("surface {k}: no Grid Sag grid");
                    };
                    w(&mut o, format!("  FIMP {}", d.grid_file()));
                    w(
                        &mut o,
                        format!(
                            "  GDAT {} {} {} {} {} {} 0 0 0 1",
                            g.nx,
                            g.ny,
                            f(g.dx),
                            f(g.dy),
                            f(g.xdec),
                            f(g.ydec)
                        ),
                    );
                    // .DAT の行の順（+y の行から、行の中は −x から）
                    let mut i = 0;
                    for row in (0..g.ny).rev() {
                        for ix in 0..g.nx {
                            let kk = row * g.nx + ix;
                            let v = g.z[kk];
                            w(
                                &mut o,
                                format!(
                                    "  GARR {i} {} {} {} {} {}",
                                    f(v[0]),
                                    f(v[1]),
                                    f(v[2]),
                                    f(v[3]),
                                    u8::from(g.nodata[kk])
                                ),
                            );
                            i += 1;
                        }
                    }
                }
            }
            SurfType::Standard => {}
        }
        for q in s.pickups.iter().filter(|q| q.dst == k) {
            match q.what {
                Kind::Thick => w(
                    &mut o,
                    format!("  PZUP {} {} {} {}", q.src, f(q.scale), f(q.offset), col(q)),
                ),
                // 円錐係数・曲率のピックアップにオフセットはない（読込で 0）
                Kind::Conic => w(
                    &mut o,
                    format!("  PKUP {} {} {}", q.src, f(q.scale), col(q)),
                ),
                Kind::Parm(i) => w(
                    &mut o,
                    format!(
                        "  PPAR {i} {} {} {} {}",
                        q.src,
                        f(q.scale),
                        f(q.offset),
                        col(q)
                    ),
                ),
                // 曲率は CURV、材料（dn・disp）は GLAS、半直径は DIAM、面の後の傾き偏心は SCBD の行に書く
                Kind::Curv
                | Kind::Dn
                | Kind::Disp
                | Kind::Semi
                | Kind::After(_)
                | Kind::Radius
                | Kind::Before(_)
                | Kind::Coating => {}
                // ピックアップの量でない種類（Pickup::what に入らない）
                _ => {}
            }
        }
        // パラメータ・円錐係数の ZPL マクロのソルブ: PZPL 番号 "名前"・KZPL "名前"（OpticStudio 26.1.1 の書き方）。
        // 半直径は DIAM の行に書く
        for q in s.macro_parms.iter().filter(|q| q.surf == k) {
            let name = macro_name(s, q.id);
            match q.what {
                Kind::Parm(i) => w(&mut o, format!("  PZPL {i} \"{name}\"")),
                Kind::Conic => w(&mut o, format!("  KZPL \"{name}\"")),
                _ => {}
            }
        }
        if d.tce != 0.0 {
            w(&mut o, format!("  TCED {} 0 0", f(d.tce)));
        }
        w(&mut o, format!("  DISZ {}", f(d.t)));
        // 厚さのソルブ（OpticStudio 2026 R1.01 の書き方）
        for q in s.solves.iter().filter(|q| q.surf == k && !q.kind.is_curv()) {
            let h = f(q.kind.value());
            let line = match q.kind {
                SolveKind::MarginalHeight { .. } => format!("  MAZH {h} 0"),
                SolveKind::MarginalHeightZone { zone: z, .. } => format!("  MAZH {h} {}", f(z)),
                SolveKind::ChiefHeight { .. } => format!("  CHZH {h} 0"),
                SolveKind::PupilPosition => "  TPUP 0 0".into(),
                SolveKind::Edge { radius: r, .. } => format!("  EDGE {h} {}", f(r)),
                SolveKind::Opd { zone: z, .. } => format!("  OPDZ {h} {}", f(z)),
                SolveKind::Position { other: j, .. } => format!("  TOLE {j} {h}"),
                SolveKind::Compensator { other: j, .. } => format!("  TCOM {j} {h}"),
                SolveKind::CenterOfCurvature { other: j } => format!("  TCOC {j} 0"),
                // ZPL マクロのソルブ（OpticStudio 26.1.1 の書き方）
                SolveKind::MacroThick { index: i } => {
                    let name = s
                        .macros
                        .get(i as usize)
                        .map(|m| m.name.as_str())
                        .unwrap_or("");
                    format!("  TZPL \"{name}\"")
                }
                _ => unreachable!(),
            };
            w(&mut o, line);
        }
        let mat_pick = s.pickups.iter().find(|q| q.dst == k && q.what == Kind::Dn);
        if d.is_mirror() {
            w(&mut o, "  GLAS MIRROR 0 0 1.5 40 0 0 0 0 0 0".into());
        } else if let (Some(q), false) = (mat_pick, d.typ() == SurfType::CoordBrk) {
            // 材料のピックアップ（OpticStudio の書き方: GLAS 名前 2 元の面 …。名前は読込で使わない）
            let name = match d.mat.glass() {
                Some(m) if s.glasses.contains_key(m) => glass_name(m),
                _ => "___BLANK",
            };
            w(
                &mut o,
                format!("  GLAS {name} 2 {} 1.5 40 0 0 0 0 0 0", q.src),
            );
        } else if d.typ() != SurfType::CoordBrk {
            match &d.mat {
                Material::Air | Material::Mirror | Material::Inherit => {}
                Material::Glass(m) if s.glasses.contains_key(m) || d.grin().is_some() => {
                    // 名前を付け替えたガラス（取り込んだ .zmx の GCAT の、名前@カタログ）はカタログの名前で
                    let m = glass_name(m);
                    w(&mut o, format!("  GLAS {m} 0 0 1.5 40 0 0 0 0 0 0"));
                }
                Material::Glass(m) => {
                    notimpl!("surface {k}: writing glass {m} that is not in the catalogs");
                }
                // モデルガラス（分散あり）
                Material::Model { nd, vd, dpgf } => w(
                    &mut o,
                    format!(
                        "  GLAS ___BLANK 1 0 {} {} {} 0 0 0 0",
                        f(*nd),
                        f(*vd),
                        f(*dpgf)
                    ),
                ),
                // モデルガラス（分散なし）
                Material::Index(nd) => w(
                    &mut o,
                    format!("  GLAS ___BLANK 1 0 {} 0 0 0 0 0 0 0", f(*nd)),
                ),
            }
        }
        if !d.coat.is_empty() {
            w(&mut o, format!("  COAT {}", d.coat));
        }
        match s
            .pickups
            .iter()
            .find(|q| q.dst == k && q.what == Kind::Semi)
        {
            // DIAM 値 2 元の面 列 倍率（OpticStudio 2026 R1.01 の書き方）
            Some(q) => w(
                &mut o,
                format!(
                    "  DIAM {} 2 {} {} {} \"\"",
                    f(d.semi),
                    q.src,
                    col(q),
                    f(q.scale)
                ),
            ),
            None => match s
                .macro_parms
                .iter()
                .find(|q| q.surf == k && q.what == Kind::Semi)
            {
                // 半直径のマクロのソルブ: DIAM 値 4 0 0 1 "名前"（OpticStudio 26.1.1 の書き方）
                Some(q) => w(
                    &mut o,
                    format!("  DIAM {} 4 0 0 1 \"{}\"", f(d.semi), macro_name(s, q.id)),
                ),
                // 自動は 0、固定は 1。座標ブレークは自動で（半直径は使わない。固定で書くとガラスの中の座標ブレークは
                // 読込で半直径 0 の浮動開口になり、光線が全部けられる）。開口の無い半直径 0 の面（プロジェクトで径を
                // 書かない玉）も同じ理由で自動で書く（半直径 0 の面は lensbench ではけない）
                None => w(
                    &mut o,
                    format!(
                        "  DIAM {} {} 0 0 1 \"\"",
                        f(d.semi),
                        if d.semi_auto
                            || d.typ() == SurfType::CoordBrk
                            || (d.semi == 0.0 && d.ap == Aperture::None)
                        {
                            0
                        } else {
                            1
                        }
                    ),
                ),
            },
        }
        // チップゾーン（0 なら書かない）と機械的な半直径（固定は 1、自動は 0 で OpticStudio が計算し直す）。半直径を
        // 固定した玉の面で開口のない面は、MEMA の行があっても OpticStudio・lensbench の読込で浮動開口になる
        if d.chip_zone != 0.0 {
            w(&mut o, format!("  OEMA {} 0 0 0 1 \"\"", f(d.chip_zone)));
        }
        match (
            s.pickups
                .iter()
                .find(|q| q.dst == k && q.what == Kind::MechSemi),
            d.mech_semi,
        ) {
            // MEMA 値 2 元の面 0 倍率（OpticStudio 2026 R1.01 の書き方。オフセットは書けない）
            (Some(q), _) if q.offset != 0.0 => notimpl!(
                "surface {k}: the mechanical semi-diameter pickup has an offset ({}), which can't be written to .zmx",
                q.offset
            ),
            (Some(q), m) => w(
                &mut o,
                format!(
                    "  MEMA {} 2 {} 0 {} \"\"",
                    f(m.unwrap_or(0.0)),
                    q.src,
                    f(q.scale)
                ),
            ),
            (None, Some(m)) => match s
                .macro_parms
                .iter()
                .find(|q| q.surf == k && q.what == Kind::MechSemi)
            {
                // 機械半径のマクロのソルブ: MEMA 値 3 0 0 1 "名前"（OpticStudio 2026 R1.01 の書き方）
                Some(q) => w(
                    &mut o,
                    format!("  MEMA {} 3 0 0 1 \"{}\"", f(m), macro_name(s, q.id)),
                ),
                None => w(&mut o, format!("  MEMA {} 1 0 0 1 \"\"", f(m))),
            },
            // 自動も今の値を書く（OpticStudio と同じ。OpticStudio 2026 R1.01 の読込は、機械半径のピックアップを元の面の
            // MEMA の値 × 倍率とクリア半径で確かめ、0 ならピックアップを外す）
            (None, None) => w(&mut o, format!("  MEMA {} 0 0 0 1 \"\"", f(s.lens_radius(k)))),
        }
        match d.ap {
            Aperture::None => {}
            Aperture::Circular { min, max, dx, dy } => {
                w(&mut o, format!("  CLAP {} {} 0", f(min), f(max)));
                w(&mut o, format!("  OBDC {} {}", f(dx), f(dy)));
            }
            Aperture::Rect { hx, hy, dx, dy } => {
                w(&mut o, format!("  SQAP {} {} 0", f(hx), f(hy)));
                w(&mut o, format!("  OBDC {} {}", f(dx), f(dy)));
            }
            Aperture::Ellipse { hx, hy, dx, dy } => {
                w(&mut o, format!("  ELAP {} {} 0", f(hx), f(hy)));
                w(&mut o, format!("  OBDC {} {}", f(dx), f(dy)));
            }
            Aperture::Floating { semi } => {
                w(&mut o, format!("  FLAP 0 {} 0", f(semi)));
            }
            Aperture::Obsc { min, max, dx, dy } => {
                w(&mut o, format!("  OBSC {} {} 0", f(min), f(max)));
                w(&mut o, format!("  OBDC {} {}", f(dx), f(dy)));
            }
            Aperture::Spider {
                width,
                arms,
                dx,
                dy,
            } => {
                w(&mut o, format!("  SPID {} {arms} 0", f(width)));
                w(&mut o, format!("  OBDC {} {}", f(dx), f(dy)));
            }
        }
        if let Some(t) = &d.before {
            tilt_line(&mut o, 1, 0, t);
        }
        let after_pick = s
            .pickups
            .iter()
            .find(|q| q.dst == k && matches!(q.what, Kind::After(_)));
        match (&d.after, &d.before) {
            // 面の後の傾き偏心のピックアップ: モード 2·(この面 − 元の面) + 1、反転は + 2
            (a, _) if after_pick.is_some() => {
                let q = after_pick.expect("checked by the match guard");
                let mode = 2 * (k - q.src) + if q.scale < 0.0 { 2 } else { 1 };
                let t = match a {
                    After::Tilt(t) => *t,
                    _ => Tilt {
                        d: [0.0; 5],
                        order: 0,
                    },
                };
                tilt_line(&mut o, 2, mode, &t);
            }
            (After::None, _) => {}
            (After::Reverse, Some(b)) => {
                let r = Tilt {
                    d: b.d.map(|v| -v),
                    order: 1 - b.order.min(1),
                };
                tilt_line(&mut o, 2, 2, &r);
            }
            (After::Reverse, None) => {}
            (After::Tilt(t), _) => tilt_line(&mut o, 2, 0, t),
        }
    }
    // 多重構成の表（MCE）。書ける行だけ（[`config_op`]）
    if let Some(c) = &s.configs {
        w(&mut o, format!("MNUM {} {}", c.n, c.active));
        // 行の番号（1 始まり、構成のピックアップの元の行の欄）。書かない行は数えない
        let row_no = config_row_numbers(s, c);
        let mut row = 0;
        for r in &c.rows {
            row += 1;
            if r.is_material() {
                for k in 1..=c.n {
                    let name = r.name_at(k).map_err(|e| e.to_string())?;
                    let p1 = r.param.index();
                    w(
                        &mut o,
                        format!("GLSS {p1} {k} \"{name}\" 0 0 0 1 1 0 0 \"\" 0"),
                    );
                }
                continue;
            }
            if r.is_coating() {
                for k in 1..=c.n {
                    let name = r.name_at(k).map_err(|e| e.to_string())?;
                    let p1 = r.param.index();
                    w(
                        &mut o,
                        format!("COTN {p1} {k} \"{name}\" 0 0 0 1 1 0 0.0 \"\" 0"),
                    );
                }
                continue;
            }
            let Some((op, p1, parm, conv)) = config_op(s, r.param) else {
                row -= 1;
                continue;
            };
            let p = parm.unwrap_or(0);
            for (k, &v) in r.values.iter().enumerate() {
                let v = conv(v);
                // 構成のピックアップ: 種類（元の構成 + 1）・元の構成・元の行・倍率・オフセット（半径の行は曲率で書けないので値だけ）
                // 別の行からのものは上の行・前か同じ構成からだけ（OpticStudio 2026 R1.01 はほかを固定に戻す。値だけ書く）
                let src_row = |q: &lensbench_core::configs::ConfigPick| match q.row {
                    None => Some(row),
                    Some(p) => row_no
                        .iter()
                        .find(|(x, _)| *x == p)
                        .map(|(_, n)| *n)
                        .filter(|&n| n < row && q.from <= q.k),
                };
                let (t, from, src, sc, off) = match r.pick(k + 1) {
                    Some(q) if !config_pick_unwritable(r) && src_row(q).is_some() => (
                        q.from + 1,
                        q.from,
                        src_row(q).unwrap_or(row),
                        q.scale,
                        conv(q.offset),
                    ),
                    _ => (0, 1, 1, 1.0, 0.0),
                };
                w(
                    &mut o,
                    format!(
                        "{op} {p1} {} {} {t} {p} 0 {from} {src} {} {} 0 \"\" 0",
                        k + 1,
                        f(v),
                        f(sc),
                        f(off)
                    ),
                );
            }
        }
    }
    Ok(o.replace('\n', "\r\n"))
}

/// UTF-16LE（BOM 付き、OpticStudio の保存と同じ）で書く。系に書いた膜の表（[`System::coat_tables`](lensbench_core::system::Prescription::coat_tables)）を使う面があれば、
/// 表を足した膜ファイル（[`coating_dat`]）を `.zmx` の隣に `名前_coat.dat` で書き、COFN はそれを指す
/// （lensbench は .zmx のフォルダから読む。OpticStudio で開くときは OpticStudio の Coatings フォルダに写す）
/// 書いたファイルに入らなかったもの・変えたものの注意を返す（[`write_notes`]）
pub fn save(path: &Path, s: &System) -> Result<Vec<String>> {
    let name = coat_file_for(path);
    let txt = write_as(s, &name)?;
    save_coating(path, &name, s)?;
    save_utf16(path, &txt)?;
    Ok(write_notes(s))
}

/// [`save`] で、メリットファンクションの行（[`crate::zmx::merit::MeritRow`]）も面の後に書く（OpticStudio と
/// 同じ位置: 面の後、多重構成の表の前）。玉・群を動かした系は面の番号が変わるので、行は書かずに注意を返す
pub fn save_with_merit(
    path: &Path,
    s: &System,
    rows: &[crate::zmx::merit::MeritRow],
) -> Result<Vec<String>> {
    let name = coat_file_for(path);
    let mut txt = write_as(s, &name)?;
    let mut notes = write_notes(s);
    if !rows.is_empty() {
        if s.units.iter().any(|u| u.moved()) {
            notes.push(
                "the merit function was not written to the .zmx (moving lenses or groups renumbers the surfaces)"
                    .into(),
            );
        } else {
            // 多重構成の表を書けない系（表の無い系・書けない構成の量）では、構成 2 以降の行（最初の CONF n > 1 から後）を書かない
            let has_mce = txt.starts_with("MNUM ") || txt.contains("\r\nMNUM ");
            let later = rows
                .iter()
                .position(|r| r.op == "CONF" && r.int(0).is_ok_and(|c| c > 1));
            let cut = later.filter(|_| !has_mce);
            if later.is_some() && has_mce {
                notes.push(
                    "the .zmx is written from configuration 1; the multi-configuration table keeps the values read"
                        .into(),
                );
            }
            if cut.is_some() {
                notes.push(
                    "the merit function rows of configurations 2 and later were not written (the .zmx has no multi-configuration table)"
                        .into(),
                );
            }
            txt = insert_merit(&txt, &rows[..cut.unwrap_or(rows.len())]);
        }
    }
    save_coating(path, &name, s)?;
    save_utf16(path, &txt)?;
    Ok(notes)
}

/// .zmx の本文（CRLF）の面の後・MNUM の前に行を入れる
fn insert_merit(txt: &str, rows: &[crate::zmx::merit::MeritRow]) -> String {
    let mut block = String::new();
    for r in rows {
        block.push_str(&r.text());
        block.push_str("\r\n");
    }
    let at = if txt.starts_with("MNUM ") {
        Some(0)
    } else {
        txt.find("\r\nMNUM ").map(|i| i + 2)
    };
    match at {
        Some(i) => format!("{}{block}{}", &txt[..i], &txt[i..]),
        None => format!("{txt}{block}"),
    }
}

/// .zmx に書くと落ちる・変わるもの（以前は黙って落としていた）。空なら系の全部を書ける。欠けなく残すなら系ファイル（.json）
pub fn write_notes(s: &System) -> Vec<String> {
    let mut out = Vec::new();
    if s.units.iter().any(|u| u.moved()) {
        out.push(
            "lens and group moves were written as coordinate breaks (the surface numbers change)"
                .to_string(),
        );
    }
    for q in &s.pickups {
        let what = match q.what {
            Kind::Before(_) => "tilt and decenter before the surface",
            Kind::Coating => "coating",
            Kind::Radius => "radius of curvature",
            _ => continue,
        };
        out.push(format!(
            "the {what} pickup of surface {} (from surface {}) can't be written to .zmx, so only the value was written",
            q.dst, q.src
        ));
    }
    if !s.mechanics.is_empty() {
        out.push(format!(
            "{} mechanical parts can't be written to .zmx",
            s.mechanics.len()
        ));
    }
    if !s.floating_clip {
        out.push(
            "the setting that floating apertures don't clip (floating_clip: false) can't be written to .zmx (they clip when read back)"
                .to_string(),
        );
    }
    // 利用者の材料（拡張の表のモデル）は名前だけ書く。OpticStudio のカタログには無い
    let user: std::collections::BTreeSet<String> = s
        .surfaces
        .iter()
        .map(|d| d.mat.name())
        .filter(|n| {
            s.glasses
                .get(n.trim())
                .is_some_and(|g| g.formula == lensbench_core::usermaterial::USER_FORMULA)
        })
        .collect();
    if !user.is_empty() {
        out.push(format!(
            "user-defined materials ({}) were written by name only; OpticStudio has no catalog for them",
            user.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    out.extend(config_notes(s));
    if !out.is_empty() {
        out.push("write a system file (.json) to keep everything".to_string());
    }
    out
}

fn save_utf16(path: &Path, txt: &str) -> Result<()> {
    let mut b = vec![0xffu8, 0xfe];
    for u in txt.encode_utf16() {
        b.extend_from_slice(&u.to_le_bytes());
    }
    lensbench_core::vfs::write(path, b).map_err(|e| Error::Io(format!("{}: {e}", path.display())))
}

/// `.zmx` の隣に書く膜ファイルの名前
fn coat_file_for(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|x| x.to_string_lossy().to_string())
        .unwrap_or_else(|| "lensbench".into());
    format!("{stem}_coat.dat")
}

fn save_coating(path: &Path, name: &str, s: &System) -> Result<()> {
    if let Some(txt) = coating_dat(s)? {
        let p = path.with_file_name(name);
        lensbench_core::vfs::write(&p, txt.as_bytes())
            .map_err(|e| Error::Io(format!("{}: {e}", p.display())))?;
    }
    Ok(())
}

/// 系に書いた膜の表を使う面があるか
fn uses_coat_tables(s: &System) -> bool {
    s.surfaces
        .iter()
        .any(|d| !d.coat.is_empty() && s.coat_tables.contains_key(&d.coat))
}

/// 系に書いた膜の表（[`System::coat_tables`](lensbench_core::system::Prescription::coat_tables)）を使う面があれば、その表を足した膜ファイル（COATING.DAT 形式）の中身。
/// 系の膜ファイル（[`System::coating_file`](lensbench_core::system::Prescription::coating_file)）があればその中身を先に置き、表を後ろに足す（同じ名前は後ろの表が勝つ。
/// lensbench の読込と同じく系の表が膜ファイルより優先）。表を使う面がなければ None
pub fn coating_dat(s: &System) -> Result<Option<String>> {
    if !uses_coat_tables(s) {
        return Ok(None);
    }
    let mut o = String::new();
    if let Some(p) = &s.coating_file {
        let t = lensbench_core::glass::read_text(p)
            .map_err(|e| Error::Io(format!("{}: {e}", p.display())))?;
        o.push_str(t.trim_end());
        o.push_str("\r\n\r\n");
    }
    o.push_str("! lensbench: 系に書いた膜の表（プロジェクト・カタログの coatings）\r\n");
    let mut names: Vec<&String> = s.coat_tables.keys().collect();
    names.sort();
    for k in names {
        if k.is_empty() || k.chars().any(|c| c.is_whitespace() || !c.is_ascii()) {
            notimpl!("coating name {k:?}: a name with spaces or non-ASCII characters can't go in a coating file");
        }
        let _ = write!(o, "TABLE {k}\r\n");
        for (a, rows) in same_wavelengths(&s.coat_tables[k]) {
            let _ = write!(o, "ANGL {}\r\n", f(a));
            for r in rows {
                let v: Vec<String> = r.iter().map(|x| f(*x)).collect();
                let _ = write!(o, "WAVE {}\r\n", v.join(" "));
            }
        }
    }
    Ok(Some(o))
}

/// OpticStudio の TABLE 膜は全部の角度で同じ波長の並び（同じ値・同じ順・同じ数）でなければならない（違うと
/// エラーを出さずに表全体を R = T = 0 として読む。OpticStudio 26.1 の実測）。角度ごとに波長が違う表は、全角度の
/// 波長の和集合に、角度ごとに波長の線形補間（範囲の外は端の値）で並べ直す。lensbench の表の読み方（角度ごとに
/// 波長で線形補間、範囲の外は端の値）では同じ値の表
fn same_wavelengths(t: &lensbench_core::coating::Table) -> Vec<(f64, Vec<[f64; 9]>)> {
    let mut wl: Vec<f64> = t.iter().flat_map(|(_, r)| r.iter().map(|x| x[0])).collect();
    wl.sort_by(f64::total_cmp);
    wl.dedup();
    t.iter()
        .map(|(a, rows)| {
            if rows.len() == wl.len() && rows.iter().zip(&wl).all(|(r, w)| r[0] == *w) {
                return (*a, rows.clone());
            }
            let xs: Vec<f64> = rows.iter().map(|r| r[0]).collect();
            let cols: Vec<Vec<f64>> = (1..9)
                .map(|k| rows.iter().map(|r| r[k]).collect())
                .collect();
            let out = wl
                .iter()
                .map(|&w| {
                    let mut r = [w; 9];
                    for k in 1..9 {
                        r[k] = lensbench_core::coating::interp(w, &xs, &cols[k - 1]);
                    }
                    r
                })
                .collect();
            (*a, out)
        })
        .collect()
}

/// 面の並びの違う系（BS の経路・プロジェクトの状態など）を、系ごとに 1 構成の多重構成の .zmx にまとめる。
///
/// 面は「全部の系で同じ先頭の面」＋「系ごとの残りの面」＋「像面」の順に並べ、構成 k では系 k 以外の残りの面を
/// 無視する（多重構成の IGNR。無視した面は厚さ・座標ブレークも効かない）。先頭の面で厚さだけが違えば THIC、
/// 絞りが構成で違えば STPS、像面の半径が違えば SDIA の行を足す。系の値（視野・波長・開口など）は全部の系で同じでなければならない。
/// ピックアップ・ソルブは値にして書く（構成ごとに面の番号が違うため）。玉・群の動きは座標ブレークにする
pub fn write_configs(systems: &[System]) -> Result<String> {
    Ok(write_configs_as(systems, DEFAULT_COAT_FILE)?.0)
}

/// [`write_configs`] の本体。まとめた系（膜の表は全部の系の表を合わせる）も返す
fn write_configs_as(systems: &[System], coat_file: &str) -> Result<(String, System)> {
    if systems.is_empty() {
        lensbench_core::invalid!("no system to write");
    }
    if systems.len() == 1 {
        return Ok((write_as(&systems[0], coat_file)?, systems[0].clone()));
    }
    let mut ss: Vec<System> = Vec::with_capacity(systems.len());
    for (i, s) in systems.iter().enumerate() {
        if s.configs.is_some() {
            notimpl!(
                "configuration {}: systems with a multi-configuration table can't be merged",
                i + 1
            );
        }
        if let lensbench_core::system::FieldType::Solve(_) = s.field_type {
            notimpl!("a field set by a condition can't be written to .zmx");
        }
        let t = if s.units.iter().any(|u| u.moved()) {
            s.flatten_units()?
        } else {
            s.clone()
        };
        let mut t = t.into_prescription();
        t.units.clear();
        t.pickups.clear();
        t.solves.clear();
        ss.push(t.build_raw()?);
    }
    // 系の値（面の前の行）。名前と使うガラスのカタログ以外は同じでなければならない
    let head = |s: &System| -> Result<Vec<String>> {
        let mut t = s.prescription().clone();
        t.name = String::new();
        Ok(write_header(&t.build_raw()?, coat_file)?
            .lines()
            .filter(|l| !l.starts_with("GCAT") && !l.starts_with("NAME"))
            .map(|l| l.to_string())
            .collect())
    };
    let h0 = head(&ss[0])?;
    for (i, s) in ss.iter().enumerate().skip(1) {
        let h = head(s)?;
        if h != h0 {
            let d = h
                .iter()
                .zip(&h0)
                .find(|(a, b)| a != b)
                .map(|(a, _)| a.clone())
                .unwrap_or_else(|| "the number of lines".into());
            notimpl!(
                "configuration {} system values differ from configuration 1 ({d})",
                i + 1
            );
        }
    }
    // 同じ先頭の面（物体面から、どの系でも像面の手前まで）。厚さだけの違いは多重構成の THIC にする
    let key_t = |d: &lensbench_core::surface::Surface| {
        let mut d = d.clone();
        d.t = 0.0;
        // 自動の半直径は構成ごとに光線から求め直すので、値の違いは同じ面とみる
        if d.semi_auto {
            d.semi = 0.0;
            if let Aperture::Floating { semi } = &mut d.ap {
                *semi = 0.0;
            }
        }
        d
    };
    let min_img = ss
        .iter()
        .map(|s| s.image())
        .min()
        .expect("a system has at least one configuration");
    let mut np = 0;
    while np < min_img
        && ss
            .iter()
            .all(|s| key_t(&s.surfaces[np]) == key_t(&ss[0].surfaces[np]))
    {
        np += 1;
    }
    let thick: Vec<(usize, Vec<f64>)> = (0..np)
        .filter_map(|j| {
            let v: Vec<f64> = ss.iter().map(|s| s.surfaces[j].t).collect();
            v.iter().any(|&t| t != v[0]).then_some((j, v))
        })
        .collect();
    if np == 0 {
        notimpl!("the object surface differs between configurations");
    }
    let glrs = ss[0].glrs;
    if ss.iter().any(|s| s.glrs != glrs) || glrs >= np {
        notimpl!("the global coordinate reference surface is on a per-configuration surface");
    }
    // 像面: 半径のほかは同じ
    let img_key = |s: &System| {
        let mut d = s.surfaces[s.image()].clone();
        d.semi = 0.0;
        d
    };
    if ss.iter().any(|s| img_key(s) != img_key(&ss[0])) {
        notimpl!("the image surface (other than its radius) differs between configurations");
    }
    let mut surfs: Vec<lensbench_core::surface::Surface> = ss[0].surfaces[..np].to_vec();
    // 系ごとの残りの面の番号（まとめた系の番号）と、絞りの面
    let mut tails: Vec<Vec<usize>> = Vec::new();
    let mut stops: Vec<usize> = Vec::new();
    let mut ign0: Vec<bool> = Vec::new();
    for (i, s) in ss.iter().enumerate() {
        let mut v = Vec::new();
        let st = s.stop();
        let mut stop_m = if st < np { st } else { 0 };
        for j in np..s.image() {
            let mut d = s.surfaces[j].clone();
            if j == st {
                stop_m = surfs.len();
            }
            d.stop = false;
            ign0.push(d.ignore);
            if i > 0 {
                d.ignore = true;
            }
            v.push(surfs.len());
            surfs.push(d);
        }
        tails.push(v);
        stops.push(stop_m);
    }
    for d in surfs.iter_mut() {
        d.stop = false;
    }
    surfs[stops[0]].stop = stops[0] > 0;
    let mut im = ss[0].surfaces[ss[0].image()].clone();
    im.stop = false;
    surfs.push(im);
    let nimg = surfs.len() - 1;
    let mut m = ss[0].prescription().clone();
    m.surfaces = surfs;
    for s in &ss[1..] {
        if !std::sync::Arc::ptr_eq(&m.glasses, &s.glasses) {
            let mg = std::sync::Arc::make_mut(&mut m.glasses);
            for (k, g) in s.glasses.iter() {
                mg.entry(k.clone()).or_insert_with(|| g.clone());
            }
        }
        for (k, t) in &s.coat_tables {
            if m.coat_tables.get(k).is_some_and(|u| u != t) {
                notimpl!("coating table {k} differs between configurations");
            }
            m.coat_tables.entry(k.clone()).or_insert_with(|| t.clone());
        }
    }
    let m = m.build_raw()?;
    let mut o = write_as(&m, coat_file)?;
    // 多重構成の表（OpticStudio 26.1 が書く形: 演算子 番号1 構成 値 0 0 0 1 1 倍率 オフセット 0 "" 0。
    // 倍率を落とすと OpticStudio は黙って値を読み違える）
    let n = ss.len();
    let row = |o: &mut String, op: &str, p1: usize, vals: &[f64]| {
        for (c, v) in vals.iter().enumerate() {
            o.push_str(&format!(
                "{op} {p1} {} {} 0 0 0 1 1 1 0 0 \"\" 0\r\n",
                c + 1,
                f(*v)
            ));
        }
    };
    o.push_str(&format!("MNUM {n} 1\r\n"));
    for (j, v) in &thick {
        row(&mut o, "THIC", *j, v);
    }
    let mut t0 = 0;
    for (i, v) in tails.iter().enumerate() {
        for &j in v {
            let own = ign0[t0];
            t0 += 1;
            let vals: Vec<f64> = (0..n)
                .map(|c| {
                    if c == i {
                        f64::from(u8::from(own))
                    } else {
                        1.0
                    }
                })
                .collect();
            row(&mut o, "IGNR", j, &vals);
        }
    }
    if stops.iter().any(|&k| k != stops[0]) {
        let vals: Vec<f64> = stops.iter().map(|&k| k as f64).collect();
        row(&mut o, "STPS", 0, &vals);
    }
    let semis: Vec<f64> = ss.iter().map(|s| s.surfaces[s.image()].semi).collect();
    if semis.iter().any(|&v| v != semis[0]) {
        row(&mut o, "SDIA", nimg, &semis);
    }
    Ok((o, m))
}

/// [`write_configs`] を UTF-16LE で書く（膜の表は [`save`] と同じく隣の膜ファイルに）
pub fn save_configs(path: &Path, systems: &[System]) -> Result<()> {
    let name = coat_file_for(path);
    let (txt, m) = write_configs_as(systems, &name)?;
    save_coating(path, &name, &m)?;
    save_utf16(path, &txt)
}

/// マクロの名前（番号がなければ空）
fn macro_name(s: &System, id: u16) -> &str {
    s.macros
        .get(id as usize)
        .map(|m| m.name.as_str())
        .unwrap_or("")
}

/// .zmx に書くガラスの名前: 名前@カタログ（[`lensbench_core::glass::qualified`]）は @ の前（カタログの中の名前）
fn glass_name(m: &str) -> &str {
    lensbench_core::glass::base_name(m)
}

#[cfg(test)]
mod merit_tests {
    use super::insert_merit;
    use crate::zmx::merit::MeritRow;

    #[test]
    fn merit_before_mnum() {
        let rows = [
            MeritRow::blank("a\nb"),
            MeritRow::new("EFFL", [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 50.0, 0.5),
        ];
        let block = "BLNK a b\r\nEFFL 0 1 0 0 0 0 50 0.5 0 0\r\n";
        let t = "SURF 0\r\n  DISZ 0\r\nMNUM 2 1\r\nTHIC 1 1 5 0 0 0 1 1 1 0 0 \"\" 0\r\n";
        let got = insert_merit(t, &rows);
        assert_eq!(
            got,
            format!(
                "SURF 0\r\n  DISZ 0\r\n{block}MNUM 2 1\r\nTHIC 1 1 5 0 0 0 1 1 1 0 0 \"\" 0\r\n"
            )
        );
        assert_eq!(
            insert_merit("SURF 0\r\n", &rows),
            format!("SURF 0\r\n{block}")
        );
    }

    /// 多重構成の表の無い系では、構成 2 以降の行（最初の CONF n > 1 から後）を書かずに注意を返す
    #[test]
    fn merit_without_mce_drops_later_configs() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let s = crate::zmx::load_with(
            &root.join("tests/refs/A0/system.zmx"),
            &crate::zmx::LoadOptions {
                cfg: 1,
                catalog_dirs: vec![root.join("tests/glass")],
                ..Default::default()
            },
        )
        .unwrap();
        let z = [0.0; 8];
        let rows = [
            MeritRow::new("EFFL", [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 50.0, 1.0),
            MeritRow::new("CONF", [2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.0, 0.0),
            MeritRow::new("TOTR", z, 80.0, 1.0),
        ];
        let dir = std::env::temp_dir().join(format!("lb_merit_mce_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.zmx");
        let notes = super::save_with_merit(&path, &s, &rows).unwrap();
        assert!(
            notes
                .iter()
                .any(|n| n.contains("configurations 2 and later")),
            "{notes:?}"
        );
        let mf = crate::zmx::merit::merit_function(&path).unwrap();
        let ops: Vec<&str> = mf.rows.iter().map(|r| r.op.as_str()).collect();
        assert_eq!(ops, ["EFFL"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
