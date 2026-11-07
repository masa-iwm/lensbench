//! 系を .zmx（OpticStudio のレンズファイル）に書き出す。読込（[`crate::zmx`]）が解釈する項目だけを書く。
//!
//! ソルブやピックアップは値にして書く。モデルガラス（分散なし）は ___BLANK、ほかのガラスは名前とカタログ（GCAT）。

use std::fmt::Write as _;
use std::path::Path;

use crate::error::{Error, Result};
use crate::notimpl;
use crate::surface::{After, Aperture, SurfType, Tilt};
use crate::system::{Aiming, ApertureKind, FieldNorm, FieldType, System};

/// 数値を読み戻しで同じ値になる表記で（Rust の最短表記）
fn f(v: f64) -> String {
    if v.is_infinite() {
        "INFINITY".into()
    } else {
        let s = format!("{v:?}");
        s.strip_suffix(".0").map(|x| x.to_string()).unwrap_or(s)
    }
}

fn tilt_line(out: &mut String, which: u8, mode: u8, t: &Tilt) {
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

/// .zmx の本文（改行は CRLF）
pub fn write(s: &System) -> Result<String> {
    let mut o = String::new();
    let w = |o: &mut String, l: String| {
        o.push_str(&l);
        o.push('\n');
    };
    w(&mut o, "VERS 260327 42 20120530 20120530".into());
    w(&mut o, "MODE SEQ".into());
    w(&mut o, format!("NAME {}", s.name));
    w(&mut o, "UNIT MM X W X CM MR CPMM LU".into());
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
    };
    let nf = s.fields.len();
    w(
        &mut o,
        format!(
            "FTYP {ft} {} {nf} {} {} 0 0",
            if s.telecentric { 1 } else { 0 },
            s.wavelengths.len(),
            if s.field_norm == FieldNorm::Rect {
                1
            } else {
                0
            }
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
    if let Some(c) = s.coating_file.as_ref().and_then(|p| p.file_name()) {
        let c = c.to_string_lossy();
        w(&mut o, format!("COFN QF \"{c}\""));
        w(&mut o, format!("COFN {c}"));
    }
    for (k, d) in s.surfaces.iter().enumerate() {
        w(&mut o, format!("SURF {k}"));
        let typ = match d.typ {
            SurfType::Standard => "STANDARD",
            SurfType::EvenAsph => "EVENASPH",
            SurfType::CoordBrk => "COORDBRK",
            SurfType::Jones => "JONESMAT",
            SurfType::Paraxial => "PARAXIAL",
        };
        w(&mut o, format!("  TYPE {typ}"));
        if !d.comment.is_empty() {
            w(&mut o, format!("  COMM {}", d.comment));
        }
        if d.stop {
            w(&mut o, "  STOP".into());
        }
        if d.ignore {
            w(&mut o, "  IGNR 1".into());
        }
        w(&mut o, format!("  CURV {}", f(d.c)));
        if d.k != 0.0 {
            w(&mut o, format!("  CONI {}", f(d.k)));
        }
        match d.typ {
            SurfType::EvenAsph => {
                for (i, a) in d.asph.iter().enumerate() {
                    w(&mut o, format!("  PARM {} {}", i + 1, f(*a)));
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
            SurfType::Standard => {}
        }
        w(&mut o, format!("  DISZ {}", f(d.t)));
        if d.mirror {
            w(&mut o, "  GLAS MIRROR 0 0 1.5 40 0 0 0 0 0 0".into());
        } else if d.typ != SurfType::CoordBrk && !d.mat.is_empty() && d.mat != "AIR" {
            if s.glasses.contains_key(&d.mat) {
                w(&mut o, format!("  GLAS {} 0 0 1.5 40 0 0 0 0 0 0", d.mat));
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
        w(&mut o, format!("  DIAM {} 0 0 0 1 \"\"", f(d.semi)));
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
            Aperture::Floating { semi } => {
                w(&mut o, format!("  FLAP 0 {} 0", f(semi)));
            }
            Aperture::Obsc { min, max, dx, dy } => {
                w(&mut o, format!("  OBSC {} {} 0", f(min), f(max)));
                w(&mut o, format!("  OBDC {} {}", f(dx), f(dy)));
            }
        }
        if let Some(t) = &d.before {
            tilt_line(&mut o, 1, 0, t);
        }
        match (&d.after, &d.before) {
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

/// UTF-16LE（BOM 付き、OpticStudio の保存と同じ）で書く
pub fn save(path: &Path, s: &System) -> Result<()> {
    let txt = write(s)?;
    let mut b = vec![0xffu8, 0xfe];
    for u in txt.encode_utf16() {
        b.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(path, b).map_err(|e| Error::Io(format!("{}: {e}", path.display())))
}
