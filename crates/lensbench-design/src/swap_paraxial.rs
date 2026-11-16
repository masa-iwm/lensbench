//! 近軸の前ふるい（[`SwapSpec::paraxial`]）: 変数（範囲つきの厚さ）の範囲の角すべてで系の屈折力を求め、`efl` の目標に
//! 届かない組を光線を追わずに落とす。屈折力は厚さのそれぞれについて 1 次なので、範囲の中の最大・最小は角にある。

use super::*;

/// 近軸の前ふるいの設定（[`SwapSpec::paraxial`]）
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Paraxial {
    /// 焦点距離の目標からの許し [mm]（省けば目標の 1 %）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efl_tol: Option<f64>,
}

/// 設定の検査（[`SwapSearch::new`] から）
pub(super) fn check(spec: &SwapSpec) -> lensbench_core::Result<()> {
    let Some(px) = &spec.paraxial else {
        return Ok(());
    };
    let Some(t) = spec.efl else {
        return Err("paraxial needs the efl target".into());
    };
    if !(t.is_finite() && t != 0.0) || px.efl_tol.is_some_and(|x| !(x >= 0.0)) {
        return Err("paraxial: efl must be finite and nonzero, efl_tol 0 or more".into());
    }
    for v in spec.vars.iter().chain(&spec.state_vars) {
        let (name, range) = v.parts()?;
        if !name.ends_with(".t") || range.is_none() {
            return Err(format!(
                "paraxial: variable {name} must be a thickness with a range (e.g. {name}=1:20)"
            )
            .into());
        }
    }
    if spec.vars.len() + spec.state_vars.len() > 12 {
        return Err("paraxial: 12 variables at most (it tries every corner of the ranges)".into());
    }
    Ok(())
}

impl SwapSearch<'_> {
    /// 近軸の前ふるい（[`SwapSpec::paraxial`]）: 状態ごとに、効く変数の範囲の角すべてで系の屈折力を求め、`efl` の目標
    /// （± efl_tol）の屈折力の範囲と重ならなければ誤り。前ふるいが無ければ何もしない
    pub(super) fn efl_reachable(&self, pr: &Problem) -> lensbench_core::Result<()> {
        let (Some(px), Some(t)) = (&self.spec.paraxial, self.spec.efl) else {
            return Ok(());
        };
        let tol = px.efl_tol.unwrap_or(0.01 * t.abs());
        // 目標の屈折力の範囲（焦点距離 t ± tol。0 をまたぐ（|t| ≤ tol）なら屈折力は 1/(|t|+tol) 以下の全部）
        let (f1, f2) = (t - tol, t + tol);
        let (plo, phi) = if f1 * f2 > 0.0 {
            let (a, b) = (1.0 / f1, 1.0 / f2);
            (a.min(b), a.max(b))
        } else {
            (f64::NEG_INFINITY, f64::INFINITY)
        };
        'cfg: for (c, sys) in pr.configs.iter().enumerate() {
            let vs: Vec<&Var> = pr
                .vars
                .iter()
                .filter(|v| v.configs.as_ref().is_none_or(|cs| cs.contains(&c)))
                .collect();
            if vs.iter().any(|v| {
                v.param.kind != lensbench_core::param::Kind::Thick
                    || !(v.lo.is_finite() && v.hi.is_finite())
            }) {
                return Err("paraxial: the variables must be thicknesses with a range".into());
            }
            // ピックアップ・ソルブ・マクロで他の値が追従する系や、屈折率分布の媒質の厚さは 1 次とは限らないので、
            // その状態は確かめない（落とさない）
            if !(sys.pickups.is_empty() && sys.solves.is_empty() && sys.macros.is_empty())
                || vs.iter().any(|v| {
                    sys.surfaces
                        .get(v.param.surf)
                        .is_none_or(|s| s.grin.is_some())
                })
            {
                continue;
            }
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for m in 0..1usize << vs.len() {
                let ps: Vec<(Param, f64)> = vs
                    .iter()
                    .enumerate()
                    .map(|(k, v)| (v.param, if m >> k & 1 == 1 { v.hi } else { v.lo }))
                    .collect();
                let efl = sys.with_params(&ps)?.paraxial().efl;
                let p = if efl.is_infinite() { 0.0 } else { 1.0 / efl };
                // 屈折力の求まらない角があればその状態は確かめない（落とさない）
                if !p.is_finite() {
                    continue 'cfg;
                }
                lo = lo.min(p);
                hi = hi.max(p);
            }
            if hi < plo || lo > phi {
                let f = |p: f64| if p == 0.0 { f64::INFINITY } else { 1.0 / p };
                let (a, b) = (f(lo), f(hi));
                // 屈折力が 0 をまたぐと焦点距離は無限遠を回る（a 以下と b 以上）
                let range = if lo < 0.0 && hi > 0.0 {
                    format!("{a:.4} mm or less, {b:.4} mm or more")
                } else {
                    format!("{:.4} to {:.4} mm", a.min(b), a.max(b))
                };
                return Err(format!(
                    "paraxial: the focal length over the variable ranges ({range}{}) misses the target {t} ± {tol}",
                    if pr.configs.len() > 1 { format!(", state {}", c + 1) } else { String::new() }
                )
                .into());
            }
        }
        Ok(())
    }
}
