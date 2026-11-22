//! GPU（wgpu の compute shader）の計算。今は Huygens PSF の和（像の点 × 瞳の 2 次波源）だけ。
//!
//! 単精度で足すので、位相が大きくならない書き方にする（式をそのまま単精度に移すと位相が 1e6 rad 級になり、強度が
//! 1e-3 級ずれる）:
//!
//! - 参照球: 像の点は球の中心 C からの差 d = P − C、光源は a = C − Q（|a| = R）で渡し、
//!   |P − Q| − R = (2 a·d + d²) / (|P − Q| + R) から位相 φ − kσ(|P − Q| − R) を求める（定数 kσR は落とす）
//! - 平面: 像の点は主光線の像点 P₀ からの差 d、光源は主光線の向きとの差 k − k₀ で渡し、位相 φ − kn (k − k₀)·d
//!   （全部の光源に共通な kn k₀·d は強度に効かないので落とす）
//!
//! 光源の位相の定数 φ は倍精度で [−π, π) に簡約してから単精度にする。差は照合の系で収差のない中心の値の
//! 5e-7 倍以下（A0・D0 の瞳 256・像 256 を llvmpipe で）。光源は何回かに分けて投げる（Windows は 1 回の投入が 2 s を超えると GPU が止められる）。
//!
//! wasm では使えない（[`gpu`] がいつも None）。

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Gpu;

/// 参照の面
#[derive(Debug, Clone, Copy)]
pub enum Reference {
    /// 参照球: 半径 R と k·σ（k = 2πn/λ、σ は球が像点の前なら +1）
    Sphere { radius: f64, ksg: f64 },
    /// 主光線に垂直な平面（または像面）: k = 2πn/λ
    Plane { kn: f64 },
}

/// 2 次波源（成分ごとの配列。長さは同じ）
#[derive(Debug, Clone, Copy)]
pub struct Sources<'a> {
    /// 球: a = C − Q（光源から球の中心へ）、平面: k − k₀（光線の向きと主光線の向きの差）
    pub dir: &'a [[f64; 3]],
    /// 振幅（面積の割合 × 瞳の振幅）
    pub amp: &'a [f64],
    /// 位相の定数 [rad]（球: 2πW、平面: 2πW + kn k·(Q − P₀)）。大きくてもよい（倍精度で簡約する）
    pub phase: &'a [f64],
}

/// 和をやめた理由
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// cancel が true を返した
    Cancelled,
    /// GPU の誤り（装置を失った・検証の誤りなど）。呼ぶ側は CPU で計算し直す
    Gpu(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Cancelled => write!(f, "cancelled"),
            Error::Gpu(e) => write!(f, "GPU error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// 使える GPU（初回に一度だけ探す）。software が false なら CPU で動くアダプタ（WARP・llvmpipe）は使わない。
/// 環境変数 WGPU_BACKEND（vulkan・dx12・metal）でバックエンドを選べる
pub fn gpu(software: bool) -> Option<&'static Gpu> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        native::get().filter(|g| software || !g.software())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = software;
        None
    }
}

/// wasm の代わりの型（作れない）
#[cfg(target_arch = "wasm32")]
pub struct Gpu(());

#[cfg(target_arch = "wasm32")]
impl Gpu {
    pub fn name(&self) -> &str {
        ""
    }
    pub fn software(&self) -> bool {
        true
    }
    pub fn huygens(
        &self,
        _r: Reference,
        _s: Sources<'_>,
        _pts: &[[f64; 3]],
        _cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<f64>, Error> {
        Err(Error::Gpu("no GPU on wasm".into()))
    }
}
