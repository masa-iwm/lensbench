//! L 本の光線の値の組（[`F`]）と真偽の組（[`B`]）。まとめ追跡の演算はこの型の演算子・関数で書く。
//!
//! 光線ごとの演算は 1 本ずつの追跡と同じ IEEE の演算（四則・平方根・比較・符号）だけで、光線の間で値を
//! 混ぜないので、結果は光線ごとに 1 本ずつの追跡とビット単位で同じ。
//!
//! - wasm32 ＋ simd128: 2 本ずつ `f64x2` の命令で計算する（LLVM は wasm ではこの形のループを
//!   ベクトル化しないので、命令を名指しする。公開 #99）
//! - それ以外: 光線の配列への素直なループ（コンパイラのベクトル化に任せる。x86_64 の AVX 版も同じ）

use super::L;
use std::ops::{Add, AddAssign, BitAnd, BitOr, Div, Index, IndexMut, Mul, Neg, Not, Sub};

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
pub use simd::*;

#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
pub use plain::*;

/// [`F`] の光線ごとの値（配列として）
impl Index<usize> for F {
    type Output = f64;
    #[inline(always)]
    fn index(&self, i: usize) -> &f64 {
        &self.arr()[i]
    }
}

impl IndexMut<usize> for F {
    #[inline(always)]
    fn index_mut(&mut self, i: usize) -> &mut f64 {
        &mut self.arr_mut()[i]
    }
}

macro_rules! binop_scalar {
    ($tr:ident, $m:ident) => {
        impl $tr<f64> for F {
            type Output = F;
            #[inline(always)]
            fn $m(self, o: f64) -> F {
                $tr::$m(self, F::splat(o))
            }
        }
        impl $tr<F> for f64 {
            type Output = F;
            #[inline(always)]
            fn $m(self, o: F) -> F {
                $tr::$m(F::splat(self), o)
            }
        }
    };
}
binop_scalar!(Add, add);
binop_scalar!(Sub, sub);
binop_scalar!(Mul, mul);
binop_scalar!(Div, div);

impl AddAssign for F {
    #[inline(always)]
    fn add_assign(&mut self, o: F) {
        *self = *self + o;
    }
}

impl F {
    #[inline(always)]
    pub fn from_fn(mut f: impl FnMut(usize) -> f64) -> F {
        let mut o = F::splat(0.0);
        for i in 0..L {
            o[i] = f(i);
        }
        o
    }

    /// `x.max(0.0)`（NaN は 0）。この追跡で -0 は max に来ない（1 − x・a² − b·c の形で、-0 にならない）
    #[inline(always)]
    pub fn max0(self) -> F {
        sel(self.gt(0.0), self, F::splat(0.0))
    }

    /// 有限か（NaN・無限大は false）
    #[inline(always)]
    pub fn is_finite(self) -> B {
        self.abs().lt(f64::INFINITY)
    }
}

/// 2 つの組の比較（光線ごと）。右は組でも数でもよい
pub trait Rhs: Copy {
    fn lanes(self) -> F;
}
impl Rhs for F {
    #[inline(always)]
    fn lanes(self) -> F {
        self
    }
}
impl Rhs for f64 {
    #[inline(always)]
    fn lanes(self) -> F {
        F::splat(self)
    }
}

impl B {
    #[inline(always)]
    pub fn from_fn(f: impl Fn(usize) -> bool) -> B {
        let mut o = B::splat(false);
        for i in 0..L {
            o.set(i, f(i));
        }
        o
    }
}

/// 光線ごとに m なら a、でなければ b
#[inline(always)]
pub fn sel(m: B, a: F, b: F) -> F {
    select(m, a, b)
}

#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
mod plain {
    use super::*;

    /// L 本の光線の値
    #[derive(Clone, Copy)]
    #[repr(C, align(32))]
    pub struct F([f64; L]);

    /// L 本の光線の真偽
    #[derive(Clone, Copy)]
    pub struct B([bool; L]);

    macro_rules! lanes {
        ($i:ident => $e:expr) => {{
            let mut o = [0.0f64; L];
            for $i in 0..L {
                o[$i] = $e;
            }
            F(o)
        }};
    }
    macro_rules! lanes_b {
        ($i:ident => $e:expr) => {{
            let mut o = [false; L];
            for $i in 0..L {
                o[$i] = $e;
            }
            B(o)
        }};
    }

    macro_rules! binop {
        ($tr:ident, $m:ident, $op:tt) => {
            impl $tr for F {
                type Output = F;
                #[inline(always)]
                fn $m(self, o: F) -> F {
                    lanes!(i => self.0[i] $op o.0[i])
                }
            }
        };
    }
    binop!(Add, add, +);
    binop!(Sub, sub, -);
    binop!(Mul, mul, *);
    binop!(Div, div, /);

    impl Neg for F {
        type Output = F;
        #[inline(always)]
        fn neg(self) -> F {
            lanes!(i => -self.0[i])
        }
    }

    macro_rules! cmp {
        ($m:ident, $op:tt) => {
            #[inline(always)]
            pub fn $m(self, o: impl Rhs) -> B {
                let o = o.lanes();
                lanes_b!(i => self.0[i] $op o.0[i])
            }
        };
    }

    impl F {
        #[inline(always)]
        pub fn splat(x: f64) -> F {
            F([x; L])
        }
        #[inline(always)]
        pub(super) fn arr(&self) -> &[f64; L] {
            &self.0
        }
        #[inline(always)]
        pub(super) fn arr_mut(&mut self) -> &mut [f64; L] {
            &mut self.0
        }
        #[inline(always)]
        pub fn sqrt(self) -> F {
            lanes!(i => self.0[i].sqrt())
        }
        #[inline(always)]
        pub fn abs(self) -> F {
            lanes!(i => self.0[i].abs())
        }
        #[inline(always)]
        pub fn signum(self) -> F {
            lanes!(i => self.0[i].signum())
        }
        cmp!(lt, <);
        cmp!(gt, >);
        cmp!(le, <=);
        cmp!(ge, >=);
        cmp!(eq, ==);
        cmp!(ne, !=);
    }

    impl B {
        #[inline(always)]
        pub fn splat(b: bool) -> B {
            B([b; L])
        }
        #[inline(always)]
        pub fn at(self, i: usize) -> bool {
            self.0[i]
        }
        #[inline(always)]
        pub fn set(&mut self, i: usize, b: bool) {
            self.0[i] = b;
        }
        #[inline(always)]
        pub fn any(self) -> bool {
            self.0.iter().any(|&b| b)
        }
    }

    impl BitAnd for B {
        type Output = B;
        #[inline(always)]
        fn bitand(self, o: B) -> B {
            lanes_b!(i => self.0[i] & o.0[i])
        }
    }
    impl BitOr for B {
        type Output = B;
        #[inline(always)]
        fn bitor(self, o: B) -> B {
            lanes_b!(i => self.0[i] | o.0[i])
        }
    }
    impl Not for B {
        type Output = B;
        #[inline(always)]
        fn not(self) -> B {
            lanes_b!(i => !self.0[i])
        }
    }

    #[inline(always)]
    pub(super) fn select(m: B, a: F, b: F) -> F {
        lanes!(i => if m.0[i] { a.0[i] } else { b.0[i] })
    }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod simd {
    use super::*;
    use std::arch::wasm32::*;

    /// 2 本ずつの組の数
    const N: usize = L / 2;
    const _: () = assert!(L % 2 == 0);

    /// L 本の光線の値（2 本ずつ f64x2）
    #[derive(Clone, Copy)]
    pub struct F([v128; N]);

    /// L 本の光線の真偽（2 本ずつ、各 64 ビットが全部 1 か 0）
    #[derive(Clone, Copy)]
    pub struct B([v128; N]);

    #[inline(always)]
    fn map1(a: [v128; N], f: impl Fn(v128) -> v128) -> [v128; N] {
        let mut o = a;
        for j in 0..N {
            o[j] = f(a[j]);
        }
        o
    }

    #[inline(always)]
    fn map2(a: [v128; N], b: [v128; N], f: impl Fn(v128, v128) -> v128) -> [v128; N] {
        let mut o = a;
        for j in 0..N {
            o[j] = f(a[j], b[j]);
        }
        o
    }

    macro_rules! binop {
        ($tr:ident, $m:ident, $f:ident) => {
            impl $tr for F {
                type Output = F;
                #[inline(always)]
                fn $m(self, o: F) -> F {
                    F(map2(self.0, o.0, |a, b| $f(a, b)))
                }
            }
        };
    }
    binop!(Add, add, f64x2_add);
    binop!(Sub, sub, f64x2_sub);
    binop!(Mul, mul, f64x2_mul);
    binop!(Div, div, f64x2_div);

    impl Neg for F {
        type Output = F;
        #[inline(always)]
        fn neg(self) -> F {
            F(map1(self.0, |x| f64x2_neg(x)))
        }
    }

    macro_rules! cmp {
        ($m:ident, $f:ident) => {
            #[inline(always)]
            pub fn $m(self, o: impl Rhs) -> B {
                B(map2(self.0, o.lanes().0, |a, b| $f(a, b)))
            }
        };
    }

    impl F {
        #[inline(always)]
        pub fn splat(x: f64) -> F {
            F([f64x2_splat(x); N])
        }
        #[inline(always)]
        pub(super) fn arr(&self) -> &[f64; L] {
            // 安全: [v128; N] と [f64; L] は大きさが同じで、v128 の並びの方が揃え（16）が厳しい
            unsafe { &*(self as *const F as *const [f64; L]) }
        }
        #[inline(always)]
        pub(super) fn arr_mut(&mut self) -> &mut [f64; L] {
            // 安全: arr と同じ。どのビット列も f64x2 として正しい
            unsafe { &mut *(self as *mut F as *mut [f64; L]) }
        }
        #[inline(always)]
        pub fn sqrt(self) -> F {
            F(map1(self.0, |x| f64x2_sqrt(x)))
        }
        #[inline(always)]
        pub fn abs(self) -> F {
            F(map1(self.0, |x| f64x2_abs(x)))
        }
        /// [`f64::signum`]（NaN は NaN、±0 は ±1）
        #[inline(always)]
        pub fn signum(self) -> F {
            let one = f64x2_splat(1.0);
            let sign = f64x2_splat(-0.0);
            F(map1(self.0, |x| {
                let s = v128_or(one, v128_and(x, sign));
                // x が NaN なら x（x != x が真）
                v128_bitselect(x, s, f64x2_ne(x, x))
            }))
        }
        cmp!(lt, f64x2_lt);
        cmp!(gt, f64x2_gt);
        cmp!(le, f64x2_le);
        cmp!(ge, f64x2_ge);
        cmp!(eq, f64x2_eq);
        cmp!(ne, f64x2_ne);
    }

    impl B {
        #[inline(always)]
        pub fn splat(b: bool) -> B {
            B([i64x2_splat(-(b as i64)); N])
        }
        #[inline(always)]
        fn words(&self) -> &[u64; L] {
            // 安全: F::arr と同じ
            unsafe { &*(self as *const B as *const [u64; L]) }
        }
        #[inline(always)]
        pub fn at(self, i: usize) -> bool {
            self.words()[i] != 0
        }
        #[inline(always)]
        pub fn set(&mut self, i: usize, b: bool) {
            // 安全: F::arr_mut と同じ
            let w = unsafe { &mut *(self as *mut B as *mut [u64; L]) };
            w[i] = if b { u64::MAX } else { 0 };
        }
        #[inline(always)]
        pub fn any(self) -> bool {
            let mut a = self.0[0];
            for j in 1..N {
                a = v128_or(a, self.0[j]);
            }
            v128_any_true(a)
        }
    }

    impl BitAnd for B {
        type Output = B;
        #[inline(always)]
        fn bitand(self, o: B) -> B {
            B(map2(self.0, o.0, |a, b| v128_and(a, b)))
        }
    }
    impl BitOr for B {
        type Output = B;
        #[inline(always)]
        fn bitor(self, o: B) -> B {
            B(map2(self.0, o.0, |a, b| v128_or(a, b)))
        }
    }
    impl Not for B {
        type Output = B;
        #[inline(always)]
        fn not(self) -> B {
            B(map1(self.0, |x| v128_not(x)))
        }
    }

    #[inline(always)]
    pub(super) fn select(m: B, a: F, b: F) -> F {
        let mut o = a.0;
        for j in 0..N {
            o[j] = v128_bitselect(a.0[j], b.0[j], m.0[j]);
        }
        F(o)
    }
}
