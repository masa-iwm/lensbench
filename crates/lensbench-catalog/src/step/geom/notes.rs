//! STEP の測りの説明（品の notes に書く）。英語が元で、文は「. 」でつなぐ（日本語の表示では文ごとに表で訳して
//! 「。」でつなぐ）

use super::fmt;
use crate::step::RetainingRing;

/// ガラスと枠を組立の置き方どおりに測った品。`n` は立体・ガラス・枠の数、`d` はガラスの厚さ・径、枠の長さ・外径・
/// 最小の内径、枠の端からガラスの前面まで [mm]。`merged` はガラスと枠が 1 つの立体のとき
pub(super) fn glass_mount(n: [usize; 3], d: [f64; 6], merged: bool) -> String {
    let mut v = vec![
        format!(
            "measured {} solids ({} glass, {} mount) placed as in the assembly",
            n[0], n[1], n[2]
        ),
        format!(
            "glass thickness {} mm and diameter {} mm, mount length {} mm, outer diameter {} mm and smallest inner diameter (clear aperture) {} mm, glass front {} mm from the mount end",
            fmt(d[0]),
            fmt(d[1]),
            fmt(d[2]),
            fmt(d[3]),
            fmt(d[4]),
            fmt(d[5]),
        ),
    ];
    if merged {
        v.push("the glass and mount are one solid, so the glass is taken between the faces that block the optical axis, and its diameter is the visible one (the part hidden by the mount is unknown)".into());
    }
    v.join(". ")
}

/// 機構部品の座の位置
pub(super) enum Seat {
    /// 枠の端（STEP から分からない）
    End,
    /// 保持する素子より細い穴の段
    Step,
    /// 止め輪の奥の面
    Ring,
}

/// ガラスの無い機構部品として測った枠。`n` は立体・穴のある立体の数、`cage` はケージ板（外径が一辺）
pub(super) fn mech(
    n: (usize, usize),
    frame: &lensbench_project::MechSpec,
    cage: bool,
    clear: f64,
    thread: &str,
    seat: Seat,
    retaining: &[RetainingRing],
) -> String {
    let mut v = vec![format!(
        "measured {} solids ({} with a bore) as the mount of a mechanical part without glass",
        n.0, n.1
    )];
    v.push(if cage {
        format!(
            "length {} mm, outer diameter {} mm (side of the cage plate), rod spacing {} mm, smallest inner diameter (clear aperture) {} mm",
            fmt(frame.length),
            fmt(frame.outer_diameter),
            fmt(frame.rod_spacing),
            fmt(clear)
        )
    } else {
        format!(
            "length {} mm, outer diameter {} mm, smallest inner diameter (clear aperture) {} mm",
            fmt(frame.length),
            fmt(frame.outer_diameter),
            fmt(clear)
        )
    });
    if !thread.is_empty() {
        v.push(format!(
            "inner thread {thread} (for elements of diameter {} mm)",
            fmt(frame.holds.map_or(0.0, |h| h.diameter))
        ));
    }
    v.push(
        match seat {
            Seat::End => "the seat is the mount end (not known from the STEP)",
            Seat::Step => "the seat is the bore step narrower than the held element",
            Seat::Ring => "the seat is the face behind the retaining ring",
        }
        .to_string(),
    );
    let ds = retaining
        .iter()
        .map(|r| fmt(r.inner_diameter))
        .collect::<Vec<_>>()
        .join(" / ");
    match retaining.len() {
        0 => {}
        1 => v.push(format!(
            "the supplied retaining ring (inner diameter {ds} mm) moves with the element, so it isn't part of the mount"
        )),
        n => v.push(format!(
            "the {n} supplied retaining rings (inner diameter {ds} mm) move with the element, so they aren't part of the mount"
        )),
    }
    v.join(". ")
}
