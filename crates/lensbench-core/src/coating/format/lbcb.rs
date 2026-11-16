//! 特性行列モデル LBCM 3（バイナリ）の符号化・復号と読み書き

use super::super::*;

/// LBCM 3 の透過の値の刻み（この刻みの整数にして持つ。元の値との差は刻みの半分以下）
pub const MODEL_BIN_QUANTUM: f64 = 1e-9;
/// LBCM 3 の鏡の r の刻み。鏡の補間は |r| = 1 からのずれ [`MIRROR_TIR_TOL`] 未満を全反射とみて分岐点を探す（`mirror_branch`）ので、
/// それより十分細かくする
pub const MODEL_BIN_QUANTUM_MIRROR: f64 = 1e-12;

/// 値の表 v（[nw][nb][k] の順に並べたもの）を LBCM 3 の値の塊にする:
/// 刻み q の整数にし、β の方向に 3 階、波長の方向に 2 階の差分を取った残り（端は低い階の差分）を zigzag で u64 にし、
/// [k][nw][nb] の順に並べてバイトの位ごとに分け（8 面）、deflate で縮める。
/// 値は β にも波長にも滑らかなので残りが小さく、上の位のバイトがほとんど 0 になって縮む
pub fn encode_values(v: &[f64], nw: usize, nb: usize, k: usize, q: f64) -> Result<Vec<u8>> {
    debug_assert_eq!(v.len(), nw * nb * k);
    let lim = (1u64 << 60) as f64;
    // [k][nw][nb] の整数
    let mut x = vec![0i64; v.len()];
    for w in 0..nw {
        for j in 0..nb {
            for f in 0..k {
                let y = v[(w * nb + j) * k + f] / q;
                if !(y.abs() < lim) {
                    let v = y * q;
                    invalid!("coating model value {v} doesn't fit LBCM 3 (it must be finite and under 2^60 times the step {q})");
                }
                x[(f * nw + w) * nb + j] = y.round() as i64;
            }
        }
    }
    let lines_b: Vec<(usize, usize, usize)> = (0..k * nw).map(|r| (r * nb, 1, nb)).collect();
    let lines_w: Vec<(usize, usize, usize)> = (0..k)
        .flat_map(|f| (0..nb).map(move |j| (f * nw * nb + j, nb, nw)))
        .collect();
    diff_lines(&mut x, &lines_b, 2);
    diff_lines(&mut x, &lines_b, 1);
    diff_lines(&mut x, &lines_w, 2);
    let n = x.len();
    let mut planes = vec![0u8; 8 * n];
    for (t, d) in x.iter().enumerate() {
        let z = ((d << 1) ^ (d >> 63)) as u64;
        for (bi, byte) in z.to_le_bytes().iter().enumerate() {
            planes[bi * n + t] = *byte;
        }
    }
    Ok(miniz_oxide::deflate::compress_to_vec(&planes, 9))
}

/// 線（先頭, 間隔, 長さ）ごとに差分を取る。order 1: x[i] − x[i−1]（i ≥ 1）、
/// order 2: 1 階の差分 d のうえで d[i] − d[i−1]（i ≥ 2。先頭 2 つは x[0] と d[1]）
pub(in crate::coating) fn diff_lines(x: &mut [i64], lines: &[(usize, usize, usize)], order: usize) {
    for &(s, st, len) in lines {
        for i in (1..len).rev() {
            x[s + i * st] = x[s + i * st].wrapping_sub(x[s + (i - 1) * st]);
        }
        if order == 2 {
            for i in (2..len).rev() {
                x[s + i * st] = x[s + i * st].wrapping_sub(x[s + (i - 1) * st]);
            }
        }
    }
}

/// [`diff_lines`] の逆
pub(in crate::coating) fn undiff_lines(
    x: &mut [i64],
    lines: &[(usize, usize, usize)],
    order: usize,
) {
    for &(s, st, len) in lines {
        if order == 2 {
            for i in 2..len {
                x[s + i * st] = x[s + i * st].wrapping_add(x[s + (i - 1) * st]);
            }
        }
        for i in 1..len {
            x[s + i * st] = x[s + i * st].wrapping_add(x[s + (i - 1) * st]);
        }
    }
}

/// [`encode_values`] の逆（[nw][nb][k] の順の値）
pub(in crate::coating) fn decode_values(
    c: &[u8],
    nw: usize,
    nb: usize,
    k: usize,
    q: f64,
) -> Result<Vec<f64>> {
    let n = match nw.checked_mul(nb).and_then(|x| x.checked_mul(k)) {
        Some(n) if n.checked_mul(8).is_some() => n,
        _ => invalid!("too many wavelengths or β values in the coating model (LBCM 3)"),
    };
    let planes = miniz_oxide::inflate::decompress_to_vec_with_limit(c, 8 * n).map_err(|_| {
        Error::Invalid("the value block of the coating model (LBCM 3) can't be inflated".into())
    })?;
    if planes.len() != 8 * n {
        invalid!("the coating model (LBCM 3) has the wrong number of values");
    }
    let mut x: Vec<i64> = (0..n)
        .map(|t| {
            let mut b = [0u8; 8];
            for (bi, byte) in b.iter_mut().enumerate() {
                *byte = planes[bi * n + t];
            }
            let u = u64::from_le_bytes(b);
            ((u >> 1) as i64) ^ -((u & 1) as i64)
        })
        .collect();
    let lines_b: Vec<(usize, usize, usize)> = (0..k * nw).map(|r| (r * nb, 1, nb)).collect();
    let lines_w: Vec<(usize, usize, usize)> = (0..k)
        .flat_map(|f| (0..nb).map(move |j| (f * nw * nb + j, nb, nw)))
        .collect();
    undiff_lines(&mut x, &lines_w, 2);
    undiff_lines(&mut x, &lines_b, 1);
    undiff_lines(&mut x, &lines_b, 2);
    let mut v = vec![0f64; n];
    for w in 0..nw {
        for j in 0..nb {
            for f in 0..k {
                v[(w * nb + j) * k + f] = x[(f * nw + w) * nb + j] as f64 * q;
            }
        }
    }
    Ok(v)
}

/// 膜のモデルのバイナリ形式（LBCM 3）を読む。中身は [`parse_models`] と同じで、値は透過を刻み
/// [`MODEL_BIN_QUANTUM`]（1e-9）、鏡を [`MODEL_BIN_QUANTUM_MIRROR`]（1e-12）の整数にして縮めて持つ（`encode_values`）。
/// f32 にしないのは、M̃ の成分が大きい（20 を超える）高い β で丸めだけで係数が 4e-6 ずれ、鏡は r から求める分岐点が丸めで動いて
/// 1e-5 ほどずれるため。
///
/// リトルエンディアン。先頭 `LBCM 3\n`、u32 膜の数、膜ごとに:
/// u16 名前の長さ・名前（UTF-8）、f64 D、f64 BMAX、u32 折れ目の数・f64 折れ目、
/// u32 波長の数 nw・u32 β の数 nb・f64 波長・f64 β・値の塊 [nw][nb][偏光 s, p][(Re Im)×(M̃00 M̃10 M̃01 M̃11), ψ]（k = 18。透過がなければ nw = 0 で塊もない）、
/// u8 鏡の有無、鏡があれば f64 基準の屈折率・f64 D_m・u32 nw・u32 nb・f64 波長・f64 β・値の塊 [nw][nb][Re r_s, Im r_s, Re r_p, Im r_p]（k = 4）。
/// 値の塊は f64 刻み q・u32 バイト数・deflate したバイト列（中身は `encode_values`）。
pub fn parse_models_bin(b: &[u8]) -> Result<Vec<(String, CoatModel)>> {
    struct Rd<'a> {
        b: &'a [u8],
        p: usize,
    }
    impl Rd<'_> {
        fn take(&mut self, n: usize) -> Result<&[u8]> {
            if self.p + n > self.b.len() {
                invalid!("the coating model (LBCM 3) is cut short");
            }
            self.p += n;
            Ok(&self.b[self.p - n..self.p])
        }
        fn u8(&mut self) -> Result<u8> {
            Ok(self.take(1)?[0])
        }
        fn u16(&mut self) -> Result<u16> {
            Ok(u16::from_le_bytes(
                self.take(2)?.try_into().expect("2 bytes"),
            ))
        }
        fn u32(&mut self) -> Result<usize> {
            Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")) as usize)
        }
        fn f64s(&mut self, n: usize) -> Result<Vec<f64>> {
            let (c, _) = self.take(8 * n)?.as_chunks::<8>();
            Ok(c.iter().map(|c| f64::from_le_bytes(*c)).collect())
        }
        fn values(&mut self, nw: usize, nb: usize, k: usize) -> Result<Vec<f64>> {
            let q = self.f64s(1)?[0];
            if !(q > 0.0 && q.is_finite()) {
                invalid!("the value step of the coating model (LBCM 3) is not positive");
            }
            let len = self.u32()?;
            decode_values(self.take(len)?, nw, nb, k, q)
        }
    }
    fn grid(name: &str, w: &[f64], b: &[f64]) -> Result<()> {
        if b.len() < 4
            || w.windows(2).any(|x| !(x[0] < x[1]))
            || b.windows(2).any(|x| !(x[0] < x[1]))
        {
            invalid!("coating model {name}: the wavelengths or β are not ascending, or there are fewer than 4 β points");
        }
        Ok(())
    }
    if !b.starts_with(MODEL_BIN_HEADER) {
        invalid!("the coating model doesn't start with LBCM 3");
    }
    let mut r = Rd {
        b,
        p: MODEL_BIN_HEADER.len(),
    };
    let nc = r.u32()?;
    // 膜の数はファイルの値なので、それで先に確保しない（壊れたファイルで巨大な確保にならないように）
    let mut out: Vec<(String, CoatModel)> = Vec::new();
    for _ in 0..nc {
        let ln = r.u16()? as usize;
        let name = String::from_utf8(r.take(ln)?.to_vec()).map_err(|_| {
            Error::Invalid("a name in the coating model (LBCM 3) is not UTF-8".into())
        })?;
        let mut m = CoatModel {
            d: r.f64s(1)?[0],
            bmax: r.f64s(1)?[0],
            ..Default::default()
        };
        if !(m.d.is_finite() && m.bmax.is_finite()) {
            invalid!("coating model {name}: D or BMAX is not finite");
        }
        let nk = r.u32()?;
        m.kinks = r.f64s(nk)?;
        let (nw, nb) = (r.u32()?, r.u32()?);
        if nw > 0 {
            m.wavelengths = r.f64s(nw)?;
            m.betas = r.f64s(nb)?;
            grid(&name, &m.wavelengths, &m.betas)?;
            let v = r.values(nw, nb, 18)?;
            let z = |i: usize| C64::new(v[i], v[i + 1]);
            m.m = (0..nw)
                .map(|w| {
                    (0..nb)
                        .map(|j| {
                            let o = (w * nb + j) * 18;
                            [0, 9].map(|q| [z(o + q), z(o + q + 2), z(o + q + 4), z(o + q + 6)])
                        })
                        .collect()
                })
                .collect();
            m.psi = (0..nw)
                .map(|w| {
                    (0..nb)
                        .map(|j| {
                            let o = (w * nb + j) * 18;
                            [v[o + 8], v[o + 17]]
                        })
                        .collect()
                })
                .collect();
        }
        let has_mirror = match r.u8()? {
            0 => false,
            1 => true,
            x => invalid!("coating model {name}: the mirror flag {x} is not 0 or 1"),
        };
        if has_mirror {
            let h = r.f64s(2)?;
            if !(h[0].is_finite() && h[1].is_finite()) {
                invalid!("coating model {name}: the mirror reference index or D_m is not finite");
            }
            let (nw, nb) = (r.u32()?, r.u32()?);
            let wavelengths = r.f64s(nw)?;
            let betas = r.f64s(nb)?;
            grid(&name, &wavelengths, &betas)?;
            let v = r.values(nw, nb, 4)?;
            let rr = (0..nw)
                .map(|w| {
                    (0..nb)
                        .map(|j| {
                            let o = (w * nb + j) * 4;
                            [C64::new(v[o], v[o + 1]), C64::new(v[o + 2], v[o + 3])]
                        })
                        .collect()
                })
                .collect();
            m.mirror = Some(MirrorModel {
                n_ref: h[0],
                d: h[1],
                wavelengths,
                betas,
                r: rr,
                ..Default::default()
            });
        } else if nw == 0 {
            invalid!("coating model {name} has no data");
        }
        if out.iter().any(|(k, _)| *k == name) {
            invalid!("coating model {name} appears twice");
        }
        out.push((name, m));
    }
    // 末尾の注記（なくてもよい）: NOTES\n、数、（名前の長さ u16、名前、注記の長さ u32、注記）× 数
    if b.len() > r.p && b[r.p..].starts_with(MODEL_BIN_NOTES) {
        r.p += MODEL_BIN_NOTES.len();
        let nn = r.u32()?;
        for _ in 0..nn {
            let ln = r.u16()? as usize;
            let name = String::from_utf8_lossy(r.take(ln)?).to_string();
            let ln = r.u32()?;
            let note = String::from_utf8(r.take(ln)?.to_vec()).map_err(|_| {
                Error::Invalid("the notes of the coating model (LBCM 3) are not UTF-8".into())
            })?;
            match out.iter_mut().find(|(k, _)| *k == name) {
                Some((_, m)) => m.note = note,
                None => invalid!("no coating {name} for the notes of the coating model (LBCM 3)"),
            }
        }
    }
    if r.p != b.len() {
        invalid!("extra data after the coating model (LBCM 3)");
    }
    Ok(out)
}

/// 膜のモデルをバイナリの形式（[`parse_models_bin`]）にする。値は 1e-9 刻みの整数にして縮める（元の値との差は 5e-10 以下）
pub fn format_models_bin(entries: &[(&str, &CoatModel)]) -> Result<Vec<u8>> {
    let mut o = MODEL_BIN_HEADER.to_vec();
    let block =
        |o: &mut Vec<u8>, v: &[f64], nw: usize, nb: usize, k: usize, q: f64| -> Result<()> {
            let c = encode_values(v, nw, nb, k, q)?;
            o.extend_from_slice(&q.to_le_bytes());
            o.extend_from_slice(&(c.len() as u32).to_le_bytes());
            o.extend_from_slice(&c);
            Ok(())
        };
    let u32 = |o: &mut Vec<u8>, v: usize| o.extend_from_slice(&(v as u32).to_le_bytes());
    let f64s =
        |o: &mut Vec<u8>, v: &[f64]| v.iter().for_each(|x| o.extend_from_slice(&x.to_le_bytes()));
    u32(&mut o, entries.len());
    for (name, m) in entries {
        o.extend_from_slice(&(name.len() as u16).to_le_bytes());
        o.extend_from_slice(name.as_bytes());
        f64s(&mut o, &[m.d, m.bmax]);
        u32(&mut o, m.kinks.len());
        f64s(&mut o, &m.kinks);
        u32(&mut o, m.wavelengths.len());
        u32(
            &mut o,
            if m.wavelengths.is_empty() {
                0
            } else {
                m.betas.len()
            },
        );
        if !m.wavelengths.is_empty() {
            f64s(&mut o, &m.wavelengths);
            f64s(&mut o, &m.betas);
            let mut v = Vec::with_capacity(m.wavelengths.len() * m.betas.len() * 18);
            for (mw, pw) in m.m.iter().zip(&m.psi) {
                for (mj, pj) in mw.iter().zip(pw) {
                    for pol in 0..2 {
                        for z in mj[pol] {
                            v.extend([z.re, z.im]);
                        }
                        v.push(pj[pol]);
                    }
                }
            }
            block(
                &mut o,
                &v,
                m.wavelengths.len(),
                m.betas.len(),
                18,
                MODEL_BIN_QUANTUM,
            )?;
        }
        match &m.mirror {
            None => o.push(0),
            Some(r) => {
                o.push(1);
                f64s(&mut o, &[r.n_ref, r.d]);
                u32(&mut o, r.wavelengths.len());
                u32(&mut o, r.betas.len());
                f64s(&mut o, &r.wavelengths);
                f64s(&mut o, &r.betas);
                let v: Vec<f64> =
                    r.r.iter()
                        .flatten()
                        .flat_map(|p| [p[0].re, p[0].im, p[1].re, p[1].im])
                        .collect();
                block(
                    &mut o,
                    &v,
                    r.wavelengths.len(),
                    r.betas.len(),
                    4,
                    MODEL_BIN_QUANTUM_MIRROR,
                )?;
            }
        }
    }
    // 注記のある膜があれば末尾に（parse_models_bin の NOTES）
    let notes: Vec<&(&str, &CoatModel)> =
        entries.iter().filter(|(_, m)| !m.note.is_empty()).collect();
    if !notes.is_empty() {
        o.extend_from_slice(MODEL_BIN_NOTES);
        u32(&mut o, notes.len());
        for (name, m) in notes {
            o.extend_from_slice(&(name.len() as u16).to_le_bytes());
            o.extend_from_slice(name.as_bytes());
            u32(&mut o, m.note.len());
            o.extend_from_slice(m.note.as_bytes());
        }
    }
    Ok(o)
}
