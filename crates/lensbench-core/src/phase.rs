//! 位相の約束。計算ごとの複素振幅の位相の向きをここに 1 か所で書く（強度・MTF の大きさはどちらでも同じ）。
//!
//! 光路長 L（[mm]、波数 k = 2π/λ）の進み方で 2 つの約束がある:
//!
//! - **exp(−ikL)**（時間因子 exp(+iωt)、平面波 exp(i(ωt − kz))、複素屈折率 N = n − iκ）: 偏光追跡の電場
//!   （[`crate::system::System::try_trace_pol`]、OpticStudio の偏光追跡と同じ）、複屈折の位相、薄膜の特性行列
//!   （[`crate::coating`]）。FFT PSF・回折 MTF の瞳関数 P = exp(+2πiW)（W = 主光線 − 光線の OPD [波]、
//!   [`crate::pupil::PupilFunction`]）もこちら
//! - **exp(+ikL)**（時間因子 exp(−iωt)）: 光線の格子の場（[`crate::raygrid`]）と、それを使う干渉縞・波面の解析、
//!   POP の場と Fresnel の核（[`crate::pop`]）、Huygens PSF の exp(−2πiW)
//!
//! OPD の符号はどちらでも W = (主光線の光路 − 光線の光路) / λ（[`crate::analysis::opd`]）。
//!
//! 2 つの約束の場は複素共役で移る。偏光追跡の電場（伝搬の位相を除いた振幅）を exp(+ikL) の約束の場に掛けるときは
//! [`pol_to_scalar`] を通す（境界はここだけにする）。新しい解析で両方の場を掛け合わせるときも同じ。

use crate::coating::C64;

/// 偏光追跡の電場の成分（exp(−ikL) の約束、伝搬の位相を除いた振幅）を、exp(+ikL) の約束の場の成分にする（複素共役）
#[inline]
pub fn pol_to_scalar(e: C64) -> C64 {
    e.conj()
}
