//! 減衰最小二乗（DLS、Levenberg–Marquardt）による最適化。
//!
//! 変数は系のパラメータ（[`lensbench_core::param`] の [`Param`](lensbench_core::param::Param): 間隔・傾き・偏心・曲率・玉や群の動きなど）。
//! 評価関数は残差の二乗和 Σ r²。項（[`Term`]）ごとに残差の並びを作り、重みの平方根を掛ける。
//! 評価関数の値（[`Problem::merit`]）は √(Σ r² / Σ 重み)。既定の項（スポット・波面の RMS）では
//! 項ごとの Σ r² が 重み × RMS² になるので、視野の重みで平均した RMS になる（OpticStudio の既定の評価関数と同じ形）。
//!
//! 項:
//! - [`TermKind::Spot`]: RMS スポット半径。光線ごとの (x − 基準, y − 基準) を残差にする（ガウス求積、
//!   [`lensbench_analysis::sweep::rms`] と同じ光線・重み・基準）
//! - [`TermKind::Wavefront`]: RMS 波面収差 [波]。光線ごとの OPD から、ピストン（重心基準なら方向余弦の差に比例する項も）を
//!   除いたもの（[`lensbench_analysis::sweep::rms`] と同じ定義）
//! - [`TermKind::Mtf`]: 1 つの周波数の MTF と目標の差
//! - [`TermKind::Paraxial`]: 近軸量（焦点距離・倍率・瞳・F ナンバーなど、[`ParaxQ`]）と目標の差
//! - [`TermKind::Centroid`]: 瞳の光線の像面の位置の重心（x か y）と目標の差（光線はスポットの項と同じ、解析的な微分）
//! - [`TermKind::Ray`]: 1 本の実光線の、面の局所座標での位置・方向余弦と目標の差（OpticStudio の REAX〜REAC 相当）
//! - [`TermKind::Opd`]: 1 本の光線の OPD [波] と目標の差（参照球は主波長の主光線、OpticStudio の OPDC 相当）
//! - [`TermKind::Zernike`]: OPD の Zernike 係数 [波] と目標の差（[`lensbench_analysis::analysis::zernike`]）
//! - [`TermKind::Coupling`]（単一モードファイバへの結合効率と目標の差、[`lensbench_analysis::fiber::fiber_coupling`]）
//! - [`TermKind::User`]: 利用者が定義する項（[`UserTerm`]。Python の関数など）。系の並びをまとめて評価する
//!
//! 項は JSON でも書ける（`lensbench_design::merit`、CLI の `--term`・Python の `terms=`）。
//!
//! 光線の項の残差は、けられた光線を 0 にする（RMS から除くのと同じ）。追跡が止まった光線（全反射・面に届かない）が
//! あると評価できない（エラー。最適化ではその一歩を捨てて減衰を強める）。[`Problem::stopped_as_vignetted`] なら止まった光線も
//! けられた光線と同じく除く。光線がけられる・戻るたびに評価関数は跳ぶので、項ごとのけられた・止まった光線の数を
//! [`Problem::ray_counts`] で見られる。項の値（[`Problem::term_values`]）は、目標のある項では量そのもの
//! （[`TermValue::Quantity`]）、ない項では √Σr²（[`TermValue::Rms`]）。
//!
//! ヤコビアン（[`Problem::jacobian`]）は、スポット・波面・光線・OPD では解析的な微分を使う: 着地点の微分
//! [`System::land_grad`](lensbench_core::System::land_grad)と光路長の微分 [`System::opl_grad`](lensbench_core::System::opl_grad)。開始の光線を固定した微分なので、
//! 変数で光線の始め方（物体側の開始位置・方向。入射瞳・エイミング・視野の定義で決まる）が変わる列は、
//! 中心差分にする（変数ごとに、少しずらした系の開始の光線を比べて判定する）。波面の参照球の半径（射出瞳の位置）と
//! 像空間の屈折率の変化は、近軸の量の差分で足す。像面が平面でない系の波面・OPD と、光線の z、MTF・近軸量・
//! Zernike・結合効率は差分にする。利用者の項は、微分を返す関数（[`UserTerm::grad`]）があればそれを使い、
//! なければ差分の系をまとめて 1 回で評価する（[`UserTerm::eval`]）。
//! 差分は変数ごとに並列に、項は項ごとに並列に計算する（[`lensbench_core::par::with_threads`] に従う）。
//!
//! 一歩は正規方程式 (JᵀJ + μ diag(JᵀJ)) δ = −Jᵀr を Cholesky で解く（[`DlsOpts::qr`] なら、列の大きさで無次元にした
//! [J D⁻¹; √μ I] の QR で解き、条件数を 2 乗にしない）。止める判定は評価関数の相対的な減り（[`DlsOpts::ftol`]）・一歩の大きさ
//! （[`DlsOpts::xtol`]）・勾配（[`DlsOpts::gtol`]、残差と列のなす角の余弦）。
//!
//! 境界（[`Var::lo`]・[`Var::hi`]）は、一歩ごとに変数を範囲に切り詰め、境界にあって外へ向かう変数はその一歩では動かさない
//! （[`DlsOpts::active_set`] なら、範囲を出る変数を境界に固定して残りを解き直す）。
//!
//! 複数の構成（[`Problem::configs`]、多重構成・ズームの各状態）を同時に扱える。変数ごとに効く構成を選べる
//! （[`Var::configs`]。既定はすべて）。カタログの差し替え探索・ズーム軌跡・モンテカルロは
//! この [`dls`] を繰り返し呼ぶ形で使う。状態を持たないので、一括実行の各タスクの中から呼んでよい。

#![warn(unreachable_pub)]
// NaN を弾く比較（!(x > 0.0)）を core と同じく使う
#![allow(clippy::neg_cmp_op_on_partial_ord)]

mod dls;
mod pupil;
pub mod zoom;
pub mod zplm;

pub use dls::*;
