//! カウンタ式の乱数: 種・光線の番号・出来事の回数・その中の何個目かだけで決まる数。
//!
//! 状態を持たないので、光線をどの順・どのスレッドで追っても同じ光線には同じ数が出る（非順次の追跡の分岐・散乱で
//! 使う。スレッド数で結果が変わらない）。GPU でも同じ式で同じ列が出せる（64 ビットの整数の掛け算・xor・シフトだけ）。
//! 混ぜ方は SplitMix64 の仕上げの関数（暗号用ではない）。

/// 種を持つカウンタ式の乱数
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CounterRng {
    seed: u64,
}

/// SplitMix64 の仕上げ（全単射。1 ビットの違いが全体に広がる）
const fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// 黄金比の 64 ビットの数（SplitMix64 の歩み）
const GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

impl CounterRng {
    pub const fn new(seed: u64) -> Self {
        CounterRng { seed }
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// 光線 ray の event 回目の出来事の k 個目の 64 ビットの数
    pub const fn bits(&self, ray: u64, event: u32, k: u32) -> u64 {
        let h = mix(self
            .seed
            .wrapping_add(GAMMA.wrapping_mul(ray.wrapping_add(1))));
        mix(h ^ mix(((event as u64) << 32 | k as u64).wrapping_add(GAMMA)))
    }

    /// 光線 ray の event 回目の出来事の k 個目の [0, 1) の一様な数（53 ビット）
    pub fn uniform(&self, ray: u64, event: u32, k: u32) -> f64 {
        (self.bits(ray, event, k) >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_counter_gives_same_number() {
        let r = CounterRng::new(7);
        assert_eq!(r.bits(3, 1, 0), r.bits(3, 1, 0));
        assert_eq!(r.uniform(3, 1, 0), CounterRng::new(7).uniform(3, 1, 0));
        // どの数を変えても別の数
        let b = r.bits(3, 1, 0);
        for o in [
            r.bits(4, 1, 0),
            r.bits(3, 2, 0),
            r.bits(3, 1, 1),
            CounterRng::new(8).bits(3, 1, 0),
        ] {
            assert_ne!(b, o);
        }
    }

    #[test]
    fn uniform_in_unit_interval_and_flat() {
        let r = CounterRng::new(42);
        let n = 200_000u64;
        let mut bins = [0u32; 16];
        let (mut s, mut s2) = (0.0, 0.0);
        for i in 0..n {
            let u = r.uniform(i, (i % 5) as u32, 0);
            assert!((0.0..1.0).contains(&u));
            bins[(u * 16.0) as usize] += 1;
            s += u;
            s2 += u * u;
        }
        let nf = n as f64;
        // 平均 1/2・分散 1/12（標準誤差の 5 倍以内）
        assert!((s / nf - 0.5).abs() < 5.0 * (1.0 / 12.0f64 / nf).sqrt());
        assert!((s2 / nf - s * s / nf / nf - 1.0 / 12.0).abs() < 1e-3);
        // 16 区間のカイ二乗（自由度 15、0.1 % 点 ≈ 37.7）
        let e = nf / 16.0;
        let chi: f64 = bins.iter().map(|&b| (b as f64 - e).powi(2) / e).sum();
        assert!(chi < 37.7, "chi2 {chi}");
    }

    #[test]
    fn neighbouring_rays_are_uncorrelated() {
        // 隣の光線の数の相関（光線の番号が連続でも並ばない）
        let r = CounterRng::new(1);
        let n = 100_000u64;
        let (mut sxy, mut sx, mut sy) = (0.0, 0.0, 0.0);
        for i in 0..n {
            let (x, y) = (r.uniform(i, 0, 0), r.uniform(i + 1, 0, 0));
            sxy += x * y;
            sx += x;
            sy += y;
        }
        let nf = n as f64;
        let c = (sxy / nf - sx * sy / nf / nf) * 12.0;
        assert!(c.abs() < 0.02, "corr {c}");
    }
}
