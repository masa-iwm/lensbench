//! 要素の種類ごとの欄（[`KindData`]）。種類ごとに 1 つだけ持つ（BS の欄と鏡の欄を同時に持つことはない）

use serde::{Deserialize, Serialize};

use super::{ApData, Bs, CustomLens, MirrorData, Pick, ZmxSrc};

/// 要素の種類だけが持つ欄（[`Pick::data`]）。書き出すと `data` の下の種類の鍵（`custom`・`mirror`・`bs`・`ap`・`zmx`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KindData {
    /// 自作の玉（2 面の単玉）・平板
    Custom(CustomLens),
    /// 鏡
    Mirror(MirrorData),
    /// ビームスプリッター
    Bs(Bs),
    /// 開口・遮蔽
    Ap(ApData),
    /// .zmx の玉・取り込み
    Zmx(ZmxSrc),
}

/// 種類の欄を読む・変える口。読むと、その種類でなければ既定。変えると、その種類の欄でなければ既定で作り直す
/// （ほかの種類の欄は消える。その種類の要素だけで呼ぶ）
macro_rules! kind_access {
    ($get:ident, $get_mut:ident, $variant:ident, $ty:ty, $none:expr, $new:expr, $doc:literal) => {
        impl Pick {
            #[doc = concat!($doc, "の欄（ほかの種類では既定）")]
            pub fn $get(&self) -> &$ty {
                static NONE: $ty = $none;
                match self.data.as_deref() {
                    Some(KindData::$variant(x)) => x,
                    _ => &NONE,
                }
            }

            #[doc = concat!($doc, "の欄を変える（無ければ作る。", $doc, "の要素だけで呼ぶ）")]
            pub fn $get_mut(&mut self) -> &mut $ty {
                if !matches!(self.data.as_deref(), Some(KindData::$variant(_))) {
                    self.data = Some(Box::new(KindData::$variant($new)));
                }
                match self.data.as_deref_mut() {
                    Some(KindData::$variant(x)) => x,
                    _ => unreachable!(),
                }
            }
        }
    };
}

kind_access!(
    custom,
    custom_mut,
    Custom,
    CustomLens,
    CustomLens::NONE,
    CustomLens::default(),
    "自作の玉・平板"
);
kind_access!(
    mirror,
    mirror_mut,
    Mirror,
    MirrorData,
    MirrorData::DEFAULT,
    MirrorData::DEFAULT,
    "鏡"
);
kind_access!(bs, bs_mut, Bs, Bs, Bs::DEFAULT, Bs::DEFAULT, "BS");
kind_access!(
    ap,
    ap_mut,
    Ap,
    ApData,
    ApData::DEFAULT,
    ApData::DEFAULT,
    "開口"
);
kind_access!(
    zmx,
    zmx_mut,
    Zmx,
    ZmxSrc,
    ZmxSrc::DEFAULT,
    ZmxSrc::DEFAULT,
    ".zmx の玉・取り込み"
);
