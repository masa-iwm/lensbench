//! 位相の約束。複素振幅の位相の向きをここに書く（強度・MTF の大きさは約束に依らない）。
//!
//! すべての計算で **exp(−ikL)**（光路長 L [mm]、波数 k = 2π/λ、時間因子 exp(+iωt)、平面波 exp(i(ωt − kz))、
//! 複素屈折率 N = n − iκ）にそろえる（ANA-13）: 偏光追跡の電場（[`crate::system::System::try_trace_pol`]、
//! OpticStudio の偏光追跡と同じ）、複屈折の位相、薄膜の特性行列（[`crate::coating`]）、FFT PSF・回折 MTF の瞳関数
//! P = exp(+2πiW)（W = 主光線 − 光線の OPD [波]、[`crate::pupil::PupilFunction`]）、Huygens PSF、POP の場と
//! Fresnel の核（[`crate::pop`]）、光線の格子の場と入射の偏光（[`crate::raygrid`]、円偏光 + は (1, −i)/√2）、
//! それを使う干渉縞・拡張の解析。
//!
//! OPD の符号は W = (主光線の光路 − 光線の光路) / λ（[`crate::analysis::opd`]）。
