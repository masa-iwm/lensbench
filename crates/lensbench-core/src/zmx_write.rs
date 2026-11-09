//! 系を .zmx（OpticStudio のレンズファイル）に書き出す。読込（[`crate::zmx`]）が解釈する項目だけを書く。
//!
//! 厚さのソルブ（[`System::solves`]）は解いた値と MAZH の行を書き、ほかのソルブは値にして書く。ピックアップ（[`System::pickups`]）は解決した値とピックアップの行の両方を書く。モデルガラス（分散なし）は ___BLANK、ほかのガラスは名前とカタログ（GCAT）。

use std::fmt::Write as _;
use std::path::Path;

use crate::error::{Error, Result};
use crate::notimpl;
use crate::param::{PickKind, SolveKind, ThickSolve};
use crate::surface::{After, Aperture, Bire, SurfType, Tilt};
use crate::system::{Aiming, ApertureKind, FieldNorm, FieldType, System};
use crate::zpl_solve::MacroCol;

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
fn col(q: &crate::param::Pickup) -> i64 {
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

/// 系に書いた膜の表（[`System::coat_tables`]）を書き出すときの膜ファイルの名前（[`write`] の既定。[`save`] は
/// `.zmx` の名前から決める）
pub const DEFAULT_COAT_FILE: &str = "lensbench_coat.dat";

/// [`write`] で、系に書いた膜の表を `coat_file` の名前の膜ファイルに書く前提で COFN を書く（膜ファイルの中身は
/// [`coating_dat`]）。表を使う面がなければ `coat_file` は使わない
pub fn write_as(s: &System, coat_file: &str) -> Result<String> {
    if s.units.iter().any(|u| u.moved()) {
        // 玉・群の剛体の動きは座標ブレークにして書く（面の番号が変わる）
        return write_as(&s.flatten_units()?, coat_file);
    }
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
        if let Some(g) = s.glasses.get(&d.mat) {
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
            notimpl!("条件で定める視野（{c:?}）は .zmx に書けない（OpticStudio にない）")
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
    let row = |key: &str, g: &dyn Fn(&crate::system::Field) -> f64| -> String {
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
        .find_map(|d| match d.grin.as_deref().map(|g| &g.kind) {
            Some(crate::grin::GrinKind::Gradium { prof, .. })
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
    for (k, d) in s.surfaces.iter().enumerate() {
        if d.dn != 0.0 || d.disp != 0.0 {
            notimpl!(
                "面 {k}: 屈折率のずれ（dn {}、disp {}）は .zmx に書けない",
                d.dn,
                d.disp
            );
        }
        if d.typ == SurfType::User && !d.ignore {
            notimpl!("面 {k}: 利用者定義の面は .zmx に書けない（OpticStudio にない）");
        }
        w(&mut o, format!("SURF {k}"));
        let typ = match d.typ {
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
            SurfType::User => "STANDARD",
        };
        // 複屈折材の板（形は標準面）: OpticStudio 26.1 の書き方（非公開側の照合データで確認）
        let bire = match (&d.bire, d.typ) {
            (Some(b), SurfType::Standard) => Some(b),
            (Some(_), t) => notimpl!("面 {k}: 複屈折材の板の面が {t:?}（標準面だけ）"),
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
            .find(|q| q.dst == k && q.what == PickKind::Curv)
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
                Some(ThickSolve {
                    kind: SolveKind::MacroCurv(i),
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
                        SolveKind::MarginalAngle => (2, q.height),
                        SolveKind::ChiefAngle => (3, q.height),
                        SolveKind::MarginalNormal => (5, 0.0),
                        SolveKind::ChiefNormal => (6, 0.0),
                        SolveKind::Aplanatic => (7, 0.0),
                        SolveKind::ElementPower => (8, q.height),
                        SolveKind::ConcentricSurface(j) => (9, j as f64),
                        SolveKind::ConcentricRadius(j) => (10, j as f64),
                        SolveKind::FNumber => (11, q.height),
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
        match d.typ {
            SurfType::EvenAsph
            | SurfType::Fresnel
            | SurfType::OddAsph
            | SurfType::TiltSurf
            | SurfType::Irregular => {
                for (i, a) in d.asph.iter().enumerate() {
                    w(&mut o, format!("  PARM {} {}", i + 1, f(*a)));
                }
            }
            SurfType::Binary2 => {
                // PARM 0 は回折次数、1〜8 は r^2〜r^16。XDAT 1 は項の数、2 は正規化半径、k+2 は ρ^{2k} の係数
                let Some(b) = &d.bin2 else {
                    notimpl!("面 {k}: Binary 2 の位相がない");
                };
                w(&mut o, format!("  PARM 0 {}", f(b.order)));
                for (i, a) in d.asph.iter().enumerate() {
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
                let Some(cf) = &d.cylf else {
                    notimpl!("面 {k}: Cylinder Fresnel の面がない");
                };
                for (i, a) in d.asph.iter().enumerate() {
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
                let Some(g) = &d.grin else {
                    notimpl!("面 {k}: GRIN の分布がない");
                };
                match &g.kind {
                    crate::grin::GrinKind::Gradium {
                        prof, dz, dec, cap, ..
                    } => {
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
                    crate::grin::GrinKind::Grad9 { tan, .. } => {
                        w(&mut o, format!("  PARM 1 {}", f(g.dt)));
                        w(&mut o, format!("  PARM 2 {}", f(tan[0])));
                        w(&mut o, format!("  PARM 3 {}", f(tan[1])));
                    }
                }
            }
            SurfType::CoordBrk => {
                for (i, a) in d.cb.iter().enumerate() {
                    w(&mut o, format!("  PARM {} {}", i + 1, f(*a)));
                }
                w(&mut o, format!("  PARM 6 {}", d.cb_order));
            }
            SurfType::Jones => {
                for (i, a) in d.jones.iter().enumerate() {
                    w(&mut o, format!("  PARM {} {}", i + 1, f(*a)));
                }
            }
            SurfType::Paraxial => {
                w(&mut o, format!("  PARM 1 {}", f(d.focal)));
                w(&mut o, format!("  PARM 2 {}", d.opd_mode));
            }
            SurfType::User => {}
            SurfType::Toroidal => {
                let r = if d.cx == 0.0 { 0.0 } else { 1.0 / d.cx };
                w(&mut o, format!("  PARM 1 {}", f(r)));
                for (i, a) in d.asph.iter().enumerate() {
                    w(&mut o, format!("  PARM {} {}", i + 2, f(*a)));
                }
                // Zernike の項の数 0、正規化半径
                w(&mut o, "  XDAT 1 0 0 0 1 0 0 \"\"".into());
                w(&mut o, "  XDAT 2 100 0 0 1 0 0 \"\"".into());
            }
            SurfType::ZernikeStd | SurfType::ZernikeFringe | SurfType::GridSag => {
                // OpticStudio 26.1 の書き方（非公開側の照合データで確認）: PARM 0 は外挿・補間、1〜8 は r^2〜r^16、
                // 9・10 は Zernike の偏心。XDAT 1 は項の数、2 は正規化半径、k+2 は項 k。Grid は GDAT・GARR を埋め込む
                let z = d.zern.as_deref();
                let p0 = match (&d.grid, z) {
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
                if d.typ == SurfType::GridSag && z.is_some_and(|z| !z.extrapolate) {
                    notimpl!("面 {k}: Grid Sag の Zernike の項は外挿しかない");
                }
                if d.asph.len() > 8 {
                    notimpl!("面 {k}: r^18 以上の項");
                }
                w(&mut o, format!("  PARM 0 {}", f(p0)));
                for i in 0..8 {
                    w(
                        &mut o,
                        format!(
                            "  PARM {} {}",
                            i + 1,
                            f(d.asph.get(i).copied().unwrap_or(0.0))
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
                if d.typ == SurfType::GridSag {
                    let Some(g) = &d.grid else {
                        notimpl!("面 {k}: Grid Sag の格子がない");
                    };
                    w(&mut o, format!("  FIMP {}", d.grid_file));
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
                PickKind::Thick => w(
                    &mut o,
                    format!("  PZUP {} {} {} {}", q.src, f(q.scale), f(q.offset), col(q)),
                ),
                // 円錐係数・曲率のピックアップにオフセットはない（読込で 0）
                PickKind::Conic => w(
                    &mut o,
                    format!("  PKUP {} {} {}", q.src, f(q.scale), col(q)),
                ),
                PickKind::Parm(i) => w(
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
                PickKind::Curv
                | PickKind::Dn
                | PickKind::Disp
                | PickKind::Semi
                | PickKind::After(_)
                | PickKind::Radius
                | PickKind::Before(_) => {}
            }
        }
        // パラメータ・円錐係数の ZPL マクロのソルブ: PZPL 番号 "名前"・KZPL "名前"（OpticStudio 26.1.1 の書き方）。
        // 半直径は DIAM の行に書く
        for q in s.macro_parms.iter().filter(|q| q.surf == k) {
            let name = macro_name(s, q.id);
            match q.col {
                MacroCol::Parm => w(&mut o, format!("  PZPL {} \"{name}\"", q.parm)),
                MacroCol::Conic => w(&mut o, format!("  KZPL \"{name}\"")),
                MacroCol::Semi => {}
            }
        }
        w(&mut o, format!("  DISZ {}", f(d.t)));
        // 厚さのソルブ（OpticStudio 2026 R1.01 の書き方）
        for q in s.solves.iter().filter(|q| q.surf == k && !q.kind.is_curv()) {
            let h = f(q.height);
            let line = match q.kind {
                SolveKind::MarginalHeight => format!("  MAZH {h} 0"),
                SolveKind::MarginalHeightZone(z) => format!("  MAZH {h} {}", f(z)),
                SolveKind::ChiefHeight => format!("  CHZH {h} 0"),
                SolveKind::PupilPosition => "  TPUP 0 0".into(),
                SolveKind::Edge(r) => format!("  EDGE {h} {}", f(r)),
                SolveKind::Opd(z) => format!("  OPDZ {h} {}", f(z)),
                SolveKind::Position(j) => format!("  TOLE {j} {h}"),
                SolveKind::Compensator(j) => format!("  TCOM {j} {h}"),
                SolveKind::CenterOfCurvature(j) => format!("  TCOC {j} 0"),
                // ZPL マクロのソルブ（OpticStudio 26.1.1 の書き方）
                SolveKind::MacroThick(i) => {
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
        let mat_pick = s
            .pickups
            .iter()
            .find(|q| q.dst == k && q.what == PickKind::Dn);
        if d.mirror {
            w(&mut o, "  GLAS MIRROR 0 0 1.5 40 0 0 0 0 0 0".into());
        } else if let (Some(q), false) = (mat_pick, d.typ == SurfType::CoordBrk) {
            // 材料のピックアップ（OpticStudio の書き方: GLAS 名前 2 元の面 …。名前は読込で使わない）
            let name = if s.glasses.contains_key(&d.mat) {
                d.mat.as_str()
            } else {
                "___BLANK"
            };
            w(
                &mut o,
                format!("  GLAS {name} 2 {} 1.5 40 0 0 0 0 0 0", q.src),
            );
        } else if d.typ != SurfType::CoordBrk && !d.mat.is_empty() && d.mat != "AIR" {
            if s.glasses.contains_key(&d.mat) || d.grin.is_some() {
                w(&mut o, format!("  GLAS {} 0 0 1.5 40 0 0 0 0 0 0", d.mat));
            } else if let Some((nd, vd, dp)) = model_glass(&d.mat) {
                // 分散ありのモデルガラス（.zmx 読込の名前 "nd,vd[,ΔPgF]"）
                w(
                    &mut o,
                    format!("  GLAS ___BLANK 1 0 {} {} {} 0 0 0 0", f(nd), f(vd), f(dp)),
                );
            } else {
                // モデルガラス（分散なし）: 屈折率は面の後の媒質の値
                let nd = d.n.first().copied().unwrap_or(f64::NAN);
                if d.n.iter().any(|&v| v != nd) {
                    notimpl!("面 {k}: カタログにないガラス {} の書き出し", d.mat);
                }
                w(
                    &mut o,
                    format!("  GLAS ___BLANK 1 0 {} 0 0 0 0 0 0 0", f(nd)),
                );
            }
        }
        if !d.coat.is_empty() {
            w(&mut o, format!("  COAT {}", d.coat));
        }
        match s
            .pickups
            .iter()
            .find(|q| q.dst == k && q.what == PickKind::Semi)
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
                .find(|q| q.surf == k && q.col == MacroCol::Semi)
            {
                // 半直径のマクロのソルブ: DIAM 値 4 0 0 1 "名前"（OpticStudio 26.1.1 の書き方）
                Some(q) => w(
                    &mut o,
                    format!("  DIAM {} 4 0 0 1 \"{}\"", f(d.semi), macro_name(s, q.id)),
                ),
                // 自動は 0、固定は 1
                None => w(
                    &mut o,
                    format!(
                        "  DIAM {} {} 0 0 1 \"\"",
                        f(d.semi),
                        if d.semi_auto { 0 } else { 1 }
                    ),
                ),
            },
        }
        // 機械的な半直径（自動。OpticStudio の保存と同じく書く）。半直径を固定した玉の面で開口のない面は、MEMA の行が
        // あっても OpticStudio・lensbench の読込で浮動開口になる
        w(&mut o, "  MEMA 0 0 0 0 1 \"\"".to_string());
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
            .find(|q| q.dst == k && matches!(q.what, PickKind::After(_)));
        match (&d.after, &d.before) {
            // 面の後の傾き偏心のピックアップ: モード 2·(この面 − 元の面) + 1、反転は + 2
            (a, _) if after_pick.is_some() => {
                let q = after_pick.unwrap();
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
    Ok(o.replace('\n', "\r\n"))
}

/// UTF-16LE（BOM 付き、OpticStudio の保存と同じ）で書く。系に書いた膜の表（[`System::coat_tables`]）を使う面があれば、
/// 表を足した膜ファイル（[`coating_dat`]）を `.zmx` の隣に `名前_coat.dat` で書き、COFN はそれを指す
/// （lensbench は .zmx のフォルダから読む。OpticStudio で開くときは OpticStudio の Coatings フォルダに写す）
pub fn save(path: &Path, s: &System) -> Result<()> {
    let name = coat_file_for(path);
    let txt = write_as(s, &name)?;
    save_coating(path, &name, s)?;
    save_utf16(path, &txt)
}

fn save_utf16(path: &Path, txt: &str) -> Result<()> {
    let mut b = vec![0xffu8, 0xfe];
    for u in txt.encode_utf16() {
        b.extend_from_slice(&u.to_le_bytes());
    }
    crate::vfs::write(path, b).map_err(|e| Error::Io(format!("{}: {e}", path.display())))
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
        crate::vfs::write(&p, txt.as_bytes())
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

/// 系に書いた膜の表（[`System::coat_tables`]）を使う面があれば、その表を足した膜ファイル（COATING.DAT 形式）の中身。
/// 系の膜ファイル（[`System::coating_file`]）があればその中身を先に置き、表を後ろに足す（同じ名前は後ろの表が勝つ。
/// lensbench の読込と同じく系の表が膜ファイルより優先）。表を使う面がなければ None
pub fn coating_dat(s: &System) -> Result<Option<String>> {
    if !uses_coat_tables(s) {
        return Ok(None);
    }
    let mut o = String::new();
    if let Some(p) = &s.coating_file {
        let t =
            crate::glass::read_text(p).map_err(|e| Error::Io(format!("{}: {e}", p.display())))?;
        o.push_str(t.trim_end());
        o.push_str("\r\n\r\n");
    }
    o.push_str("! lensbench: 系に書いた膜の表（プロジェクト・カタログの coatings）\r\n");
    let mut names: Vec<&String> = s.coat_tables.keys().collect();
    names.sort();
    for k in names {
        if k.is_empty() || k.chars().any(|c| c.is_whitespace() || !c.is_ascii()) {
            notimpl!("膜の名前 {k:?}: 空白・ASCII 以外の文字のある名前は膜ファイルに書けない");
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
fn same_wavelengths(t: &crate::coating::Table) -> Vec<(f64, Vec<[f64; 9]>)> {
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
                        r[k] = crate::coating::interp(w, &xs, &cols[k - 1]);
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
        crate::invalid!("書き出す系がない");
    }
    if systems.len() == 1 {
        return Ok((write_as(&systems[0], coat_file)?, systems[0].clone()));
    }
    let mut ss: Vec<System> = Vec::with_capacity(systems.len());
    for (i, s) in systems.iter().enumerate() {
        if s.configs.is_some() {
            notimpl!("構成 {}: 多重構成の表を持つ系はまとめられない", i + 1);
        }
        if let crate::system::FieldType::Solve(_) = s.field_type {
            notimpl!("条件で定める視野は .zmx に書けない");
        }
        let mut t = if s.units.iter().any(|u| u.moved()) {
            s.flatten_units()?
        } else {
            s.clone()
        };
        t.units.clear();
        t.pickups.clear();
        t.solves.clear();
        ss.push(t);
    }
    // 系の値（面の前の行）。名前と使うガラスのカタログ以外は同じでなければならない
    let head = |s: &System| -> Result<Vec<String>> {
        let mut t = s.clone();
        t.name = String::new();
        let txt = write(&t)?;
        Ok(txt
            .split("\r\n")
            .take_while(|l| !l.starts_with("SURF "))
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
                .unwrap_or_else(|| "行の数".into());
            notimpl!("構成 {} の系の値が構成 1 と違う（{d}）", i + 1);
        }
    }
    // 同じ先頭の面（物体面から、どの系でも像面の手前まで）。厚さだけの違いは多重構成の THIC にする
    let key = |d: &crate::surface::Surface| format!("{d:?}");
    let key_t = |d: &crate::surface::Surface| {
        let mut d = d.clone();
        d.t = 0.0;
        // 自動の半直径は構成ごとに光線から求め直すので、値の違いは同じ面とみる
        if d.semi_auto {
            d.semi = 0.0;
            if let Aperture::Floating { semi } = &mut d.ap {
                *semi = 0.0;
            }
        }
        key(&d)
    };
    let min_img = ss.iter().map(|s| s.image()).min().unwrap();
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
        notimpl!("物体面が構成で違う");
    }
    let glrs = ss[0].glrs;
    if ss.iter().any(|s| s.glrs != glrs) || glrs >= np {
        notimpl!("大域座標の基準面が構成ごとの面にある");
    }
    // 像面: 半径のほかは同じ
    let img_key = |s: &System| {
        let mut d = s.surfaces[s.image()].clone();
        d.semi = 0.0;
        key(&d)
    };
    if ss.iter().any(|s| img_key(s) != img_key(&ss[0])) {
        notimpl!("像面（半径のほか）が構成で違う");
    }
    let mut surfs: Vec<crate::surface::Surface> = ss[0].surfaces[..np].to_vec();
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
    let mut m = ss[0].clone();
    m.surfaces = surfs;
    for s in &ss[1..] {
        for (k, g) in &s.glasses {
            m.glasses.entry(k.clone()).or_insert_with(|| g.clone());
        }
        for (k, t) in &s.coat_tables {
            if m.coat_tables.get(k).is_some_and(|u| u != t) {
                notimpl!("膜の表 {k} が構成で違う");
            }
            m.coat_tables.entry(k.clone()).or_insert_with(|| t.clone());
        }
    }
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

/// 分散ありのモデルガラスの名前 "nd,vd[,ΔPgF]"（vd ≠ 0）
fn model_glass(m: &str) -> Option<(f64, f64, f64)> {
    let v: Vec<f64> = m
        .split(',')
        .map(|t| t.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .ok()?;
    match v[..] {
        [nd, vd] if vd != 0.0 => Some((nd, vd, 0.0)),
        [nd, vd, dp] if vd != 0.0 => Some((nd, vd, dp)),
        _ => None,
    }
}

/// マクロの名前（番号がなければ空）
fn macro_name(s: &System, id: u16) -> &str {
    s.macros
        .get(id as usize)
        .map(|m| m.name.as_str())
        .unwrap_or("")
}
