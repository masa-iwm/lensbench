//! 光線の値の組（[`Lanes`]）と真偽の組（[`Mask`]）。追跡のカーネルはこの型の演算子・関数で書き、
//! 組の幅ごとに展開する（[`Wide`]: まとめ追跡の [`super::L`] 本、[`One`]: 1 本）。
//!
//! 光線ごとの演算は IEEE の演算（四則・平方根・比較・符号）だけで、光線の間で値を混ぜないので、
//! 結果は組の幅によらず光線ごとに同じビット。
//!
//! - wasm32 ＋ simd128: [`Wide`] は 2 本ずつ `f64x2` の命令で計算する（LLVM は wasm ではこの形のループを
//!   ベクトル化しないので、命令を名指しする）
//! - それ以外: 光線の配列への素直なループ（コンパイラのベクトル化に任せる。x86_64 の AVX 版も同じ）

use std::ops::{Add, AddAssign, BitAnd, BitOr, Div, Index, IndexMut, Mul, Neg, Not, Sub};

/// 光線の値の組。四則は組どうしと、右が数のもの（左が数の式は `V::splat(a) - x` の形で書く）
pub(crate) trait Lanes:
    Copy
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
    + Add<f64, Output = Self>
    + Sub<f64, Output = Self>
    + Mul<f64, Output = Self>
    + Div<f64, Output = Self>
    + AddAssign
    + Index<usize, Output = f64>
    + IndexMut<usize>
    + Rhs<Self>
{
    /// 真偽の組
    type B: Mask;
    /// 光線の数
    const N: usize;
    fn splat(x: f64) -> Self;
    fn sqrt(self) -> Self;
    fn abs(self) -> Self;
    /// [`f64::signum`]（NaN は NaN、±0 は ±1）
    fn signum(self) -> Self;
    fn lt(self, o: impl Rhs<Self>) -> Self::B;
    fn gt(self, o: impl Rhs<Self>) -> Self::B;
    fn le(self, o: impl Rhs<Self>) -> Self::B;
    fn ge(self, o: impl Rhs<Self>) -> Self::B;
    fn eq(self, o: impl Rhs<Self>) -> Self::B;
    fn ne(self, o: impl Rhs<Self>) -> Self::B;
    /// 光線ごとに m なら a、でなければ b
    fn select(m: Self::B, a: Self, b: Self) -> Self;

    #[inline(always)]
    fn from_fn(mut f: impl FnMut(usize) -> f64) -> Self {
        let mut o = Self::splat(0.0);
        for i in 0..Self::N {
            o[i] = f(i);
        }
        o
    }

    /// `x.max(0.0)`（NaN は 0）。この追跡で -0 は max に来ない（1 − x・a² − b·c の形で、-0 にならない）
    #[inline(always)]
    fn max0(self) -> Self {
        Self::select(self.gt(0.0), self, Self::splat(0.0))
    }

    /// 有限か（NaN・無限大は false）
    #[inline(always)]
    fn is_finite(self) -> Self::B {
        self.abs().lt(f64::INFINITY)
    }
}

/// 真偽の組
pub(crate) trait Mask:
    Copy + BitAnd<Output = Self> + BitOr<Output = Self> + Not<Output = Self>
{
    const N: usize;
    fn splat(b: bool) -> Self;
    fn at(self, i: usize) -> bool;
    fn set(&mut self, i: usize, b: bool);
    fn any(self) -> bool;

    #[inline(always)]
    fn from_fn(f: impl Fn(usize) -> bool) -> Self {
        let mut o = Self::splat(false);
        for i in 0..Self::N {
            o.set(i, f(i));
        }
        o
    }
}

/// 比較の右辺（組か数）
pub(crate) trait Rhs<V>: Copy {
    fn lanes(self) -> V;
}
impl<V: Lanes> Rhs<V> for f64 {
    #[inline(always)]
    fn lanes(self) -> V {
        V::splat(self)
    }
}

/// 光線ごとに m なら a、でなければ b
#[inline(always)]
pub(crate) fn sel<V: Lanes>(m: V::B, a: V, b: V) -> V {
    V::select(m, a, b)
}

/// 1 本（1 本ずつの追跡）
pub(crate) type One = plain::F<1>;

/// まとめ追跡の L 本
#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
pub(crate) type Wide = plain::F<{ super::L }>;
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
pub(crate) type Wide = simd::F;

pub(crate) mod plain {
    use super::*;

    /// N 本の光線の値
    #[derive(Clone, Copy)]
    #[repr(C, align(32))]
    pub(crate) struct F<const N: usize>([f64; N]);

    /// N 本の光線の真偽
    #[derive(Clone, Copy)]
    pub(crate) struct B<const N: usize>([bool; N]);

    impl<const N: usize> Rhs<F<N>> for F<N> {
        #[inline(always)]
        fn lanes(self) -> F<N> {
            self
        }
    }

    macro_rules! lanes {
        ($i:ident => $e:expr) => {{
            let mut o = [0.0f64; N];
            for $i in 0..N {
                o[$i] = $e;
            }
            F(o)
        }};
    }
    macro_rules! lanes_b {
        ($i:ident => $e:expr) => {{
            let mut o = [false; N];
            for $i in 0..N {
                o[$i] = $e;
            }
            B(o)
        }};
    }

    macro_rules! binop {
        ($tr:ident, $m:ident, $op:tt) => {
            impl<const N: usize> $tr for F<N> {
                type Output = F<N>;
                #[inline(always)]
                fn $m(self, o: F<N>) -> F<N> {
                    lanes!(i => self.0[i] $op o.0[i])
                }
            }
            impl<const N: usize> $tr<f64> for F<N> {
                type Output = F<N>;
                #[inline(always)]
                fn $m(self, o: f64) -> F<N> {
                    lanes!(i => self.0[i] $op o)
                }
            }
        };
    }
    binop!(Add, add, +);
    binop!(Sub, sub, -);
    binop!(Mul, mul, *);
    binop!(Div, div, /);

    impl<const N: usize> Neg for F<N> {
        type Output = F<N>;
        #[inline(always)]
        fn neg(self) -> F<N> {
            lanes!(i => -self.0[i])
        }
    }

    impl<const N: usize> AddAssign for F<N> {
        #[inline(always)]
        fn add_assign(&mut self, o: F<N>) {
            *self = *self + o;
        }
    }

    impl<const N: usize> Index<usize> for F<N> {
        type Output = f64;
        #[inline(always)]
        fn index(&self, i: usize) -> &f64 {
            &self.0[i]
        }
    }
    impl<const N: usize> IndexMut<usize> for F<N> {
        #[inline(always)]
        fn index_mut(&mut self, i: usize) -> &mut f64 {
            &mut self.0[i]
        }
    }

    macro_rules! cmp {
        ($m:ident, $op:tt) => {
            #[inline(always)]
            fn $m(self, o: impl Rhs<Self>) -> B<N> {
                let o = o.lanes();
                lanes_b!(i => self.0[i] $op o.0[i])
            }
        };
    }

    impl<const N: usize> Lanes for F<N> {
        type B = B<N>;
        const N: usize = N;
        #[inline(always)]
        fn splat(x: f64) -> F<N> {
            F([x; N])
        }
        #[inline(always)]
        fn sqrt(self) -> F<N> {
            lanes!(i => self.0[i].sqrt())
        }
        #[inline(always)]
        fn abs(self) -> F<N> {
            lanes!(i => self.0[i].abs())
        }
        #[inline(always)]
        fn signum(self) -> F<N> {
            lanes!(i => self.0[i].signum())
        }
        cmp!(lt, <);
        cmp!(gt, >);
        cmp!(le, <=);
        cmp!(ge, >=);
        cmp!(eq, ==);
        cmp!(ne, !=);
        #[inline(always)]
        fn select(m: B<N>, a: F<N>, b: F<N>) -> F<N> {
            lanes!(i => if m.0[i] { a.0[i] } else { b.0[i] })
        }
    }

    impl<const N: usize> Mask for B<N> {
        const N: usize = N;
        #[inline(always)]
        fn splat(b: bool) -> B<N> {
            B([b; N])
        }
        #[inline(always)]
        fn at(self, i: usize) -> bool {
            self.0[i]
        }
        #[inline(always)]
        fn set(&mut self, i: usize, b: bool) {
            self.0[i] = b;
        }
        #[inline(always)]
        fn any(self) -> bool {
            self.0.iter().any(|&b| b)
        }
    }

    impl<const N: usize> BitAnd for B<N> {
        type Output = B<N>;
        #[inline(always)]
        fn bitand(self, o: B<N>) -> B<N> {
            lanes_b!(i => self.0[i] & o.0[i])
        }
    }
    impl<const N: usize> BitOr for B<N> {
        type Output = B<N>;
        #[inline(always)]
        fn bitor(self, o: B<N>) -> B<N> {
            lanes_b!(i => self.0[i] | o.0[i])
        }
    }
    impl<const N: usize> Not for B<N> {
        type Output = B<N>;
        #[inline(always)]
        fn not(self) -> B<N> {
            lanes_b!(i => !self.0[i])
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
pub(crate) mod simd {
    use super::*;
    use crate::batch::L;
    use std::arch::wasm32::*;

    /// 2 本ずつの組の数
    const NW: usize = L / 2;
    const _: () = assert!(L.is_multiple_of(2));

    /// L 本の光線の値（2 本ずつ f64x2）
    #[derive(Clone, Copy)]
    pub(crate) struct F([v128; NW]);

    /// L 本の光線の真偽（2 本ずつ、各 64 ビットが全部 1 か 0）
    #[derive(Clone, Copy)]
    pub(crate) struct B([v128; NW]);

    impl Rhs<F> for F {
        #[inline(always)]
        fn lanes(self) -> F {
            self
        }
    }

    #[inline(always)]
    fn map1(a: [v128; NW], f: impl Fn(v128) -> v128) -> [v128; NW] {
        let mut o = a;
        for j in 0..NW {
            o[j] = f(a[j]);
        }
        o
    }

    #[inline(always)]
    fn map2(a: [v128; NW], b: [v128; NW], f: impl Fn(v128, v128) -> v128) -> [v128; NW] {
        let mut o = a;
        for j in 0..NW {
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
            impl $tr<f64> for F {
                type Output = F;
                #[inline(always)]
                fn $m(self, o: f64) -> F {
                    $tr::$m(self, F::splat(o))
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

    impl AddAssign for F {
        #[inline(always)]
        fn add_assign(&mut self, o: F) {
            *self = *self + o;
        }
    }

    impl Index<usize> for F {
        type Output = f64;
        #[inline(always)]
        fn index(&self, i: usize) -> &f64 {
            // 安全: [v128; NW] と [f64; L] は大きさが同じで、v128 の並びの方が揃え（16）が厳しい
            let a = unsafe { &*(self as *const F as *const [f64; L]) };
            &a[i]
        }
    }
    impl IndexMut<usize> for F {
        #[inline(always)]
        fn index_mut(&mut self, i: usize) -> &mut f64 {
            // 安全: index と同じ。どのビット列も f64x2 として正しい
            let a = unsafe { &mut *(self as *mut F as *mut [f64; L]) };
            &mut a[i]
        }
    }

    macro_rules! cmp {
        ($m:ident, $f:ident) => {
            #[inline(always)]
            fn $m(self, o: impl Rhs<Self>) -> B {
                B(map2(self.0, o.lanes().0, |a, b| $f(a, b)))
            }
        };
    }

    impl Lanes for F {
        type B = B;
        const N: usize = L;
        #[inline(always)]
        fn splat(x: f64) -> F {
            F([f64x2_splat(x); NW])
        }
        #[inline(always)]
        fn sqrt(self) -> F {
            F(map1(self.0, |x| f64x2_sqrt(x)))
        }
        #[inline(always)]
        fn abs(self) -> F {
            F(map1(self.0, |x| f64x2_abs(x)))
        }
        #[inline(always)]
        fn signum(self) -> F {
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
        #[inline(always)]
        fn select(m: B, a: F, b: F) -> F {
            let mut o = a.0;
            for j in 0..NW {
                o[j] = v128_bitselect(a.0[j], b.0[j], m.0[j]);
            }
            F(o)
        }
    }

    impl B {
        #[inline(always)]
        fn words(&self) -> &[u64; L] {
            // 安全: F の index と同じ
            unsafe { &*(self as *const B as *const [u64; L]) }
        }
    }

    impl Mask for B {
        const N: usize = L;
        #[inline(always)]
        fn splat(b: bool) -> B {
            B([i64x2_splat(-(b as i64)); NW])
        }
        #[inline(always)]
        fn at(self, i: usize) -> bool {
            self.words()[i] != 0
        }
        #[inline(always)]
        fn set(&mut self, i: usize, b: bool) {
            // 安全: F の index_mut と同じ
            let w = unsafe { &mut *(self as *mut B as *mut [u64; L]) };
            w[i] = if b { u64::MAX } else { 0 };
        }
        #[inline(always)]
        fn any(self) -> bool {
            let mut a = self.0[0];
            for j in 1..NW {
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
}
