//! 面の範囲の基点の項（[`TermKind::Cardinal`]）: 焦点距離・焦点・主点・反主点・節点・反節点

use super::*;
use lensbench_core::report::{cardinal_span, Cardinal};

/// 面 from から to までの近軸の系の量（[`lensbench_core::report::cardinal_span`]、OpticStudio の CARD の Data 0〜11 と
/// EFLX・EFLY）。位置は物体空間の点が面 from の頂点から、像空間の点が面 to の頂点から [レンズ単位]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardQ {
    /// 空気換算の焦点距離 1/パワー（EFLX・EFLY）
    Efl,
    /// 媒質の中の焦点距離（物体空間は負、CARD の 0・1）
    FObject,
    FImage,
    /// 焦点（CARD の 2・3）
    FocalObject,
    FocalImage,
    /// 主点（4・5）
    PrincipalObject,
    PrincipalImage,
    /// 反主点（6・7）
    AntiPrincipalObject,
    AntiPrincipalImage,
    /// 節点（8・9）
    NodalObject,
    NodalImage,
    /// 反節点（10・11）
    AntiNodalObject,
    AntiNodalImage,
}

impl CardQ {
    pub const ALL: [CardQ; 13] = [
        CardQ::Efl,
        CardQ::FObject,
        CardQ::FImage,
        CardQ::FocalObject,
        CardQ::FocalImage,
        CardQ::PrincipalObject,
        CardQ::PrincipalImage,
        CardQ::AntiPrincipalObject,
        CardQ::AntiPrincipalImage,
        CardQ::NodalObject,
        CardQ::NodalImage,
        CardQ::AntiNodalObject,
        CardQ::AntiNodalImage,
    ];
    pub fn name(self) -> &'static str {
        match self {
            CardQ::Efl => "efl",
            CardQ::FObject => "f_object",
            CardQ::FImage => "f_image",
            CardQ::FocalObject => "focal_object",
            CardQ::FocalImage => "focal_image",
            CardQ::PrincipalObject => "principal_object",
            CardQ::PrincipalImage => "principal_image",
            CardQ::AntiPrincipalObject => "anti_principal_object",
            CardQ::AntiPrincipalImage => "anti_principal_image",
            CardQ::NodalObject => "nodal_object",
            CardQ::NodalImage => "nodal_image",
            CardQ::AntiNodalObject => "anti_nodal_object",
            CardQ::AntiNodalImage => "anti_nodal_image",
        }
    }
    pub fn parse(s: &str) -> Result<CardQ> {
        let t = s.trim().to_ascii_lowercase();
        match CardQ::ALL.iter().find(|q| q.name() == t) {
            Some(q) => Ok(*q),
            None => invalid!(
                "cardinal quantity {s} ({})",
                CardQ::ALL.map(|q| q.name()).join(", ")
            ),
        }
    }
    /// OpticStudio の CARD の Data（Efl は無い）
    pub fn card_data(self) -> Option<usize> {
        CardQ::ALL
            .iter()
            .position(|q| *q == self)
            .and_then(|i| i.checked_sub(1))
    }
    /// CARD の Data から
    pub fn from_card_data(d: usize) -> Option<CardQ> {
        CardQ::ALL.get(d + 1).copied()
    }
    fn of(self, c: &Cardinal) -> f64 {
        match self {
            CardQ::Efl => c.efl,
            CardQ::FObject => c.f_object,
            CardQ::FImage => c.f_image,
            CardQ::FocalObject => c.focal_object,
            CardQ::FocalImage => c.focal_image,
            CardQ::PrincipalObject => c.principal_object,
            CardQ::PrincipalImage => c.principal_image,
            CardQ::AntiPrincipalObject => c.anti_principal_object,
            CardQ::AntiPrincipalImage => c.anti_principal_image,
            CardQ::NodalObject => c.nodal_object,
            CardQ::NodalImage => c.nodal_image,
            CardQ::AntiNodalObject => c.anti_nodal_object,
            CardQ::AntiNodalImage => c.anti_nodal_image,
        }
    }
}

/// 面 from から to までの量 q（波長 w）
pub(super) fn cardinal_value(
    s: &System,
    from: usize,
    to: usize,
    q: CardQ,
    w: usize,
) -> Result<f64> {
    let v = q.of(&cardinal_span(s, from, to, w));
    if !v.is_finite() {
        invalid!("{} of surfaces {from} to {to} is not finite", q.name());
    }
    Ok(v)
}

pub(super) fn check(i: usize, k: &TermKind, s: &System) -> Result<()> {
    let TermKind::Cardinal {
        from,
        to,
        q,
        wave,
        target,
    } = k
    else {
        unreachable!("not a cardinal term");
    };
    let last = s.image() - 1;
    if *from == 0 || from > to || *to > last {
        invalid!("term {i}: cardinal points of surfaces {from} to {to} (1 to {last})");
    }
    if !target.is_finite() {
        invalid!("term {i}: target {target} of {}", q.name());
    }
    if let Some(w) = wave {
        if *w >= s.wavelengths.len() {
            invalid!("term {i}: wavelength {w} (of {})", s.wavelengths.len());
        }
    }
    cardinal_value(s, *from, *to, *q, wave.unwrap_or(s.primary)).map(|_| ())
}

pub(super) fn term(s: &System, k: &TermKind) -> Result<TermOut> {
    let TermKind::Cardinal {
        from,
        to,
        q,
        wave,
        target,
    } = k
    else {
        unreachable!("not a cardinal term");
    };
    let w = wave.unwrap_or(s.primary);
    Ok((vec![cardinal_value(s, *from, *to, *q, w)? - target], None))
}
