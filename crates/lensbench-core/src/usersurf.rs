//! 利用者が定義する面の口。
//!
//! 面の形は頂点の平面（z = 0、理想レンズと同じ）に限り、光線の曲げ方と光路長（位相）だけを利用者が決める。
//! [`SurfaceModel`] を実装して [`register`] で名前を登録し、面の種類 [`crate::surface::SurfType::User`] の面に
//! `user = {"model": 名前, "config": {…}}`（[`UserSurf`]）を与える。設定は検出器・解析のプラグインと同じ
//! 数・文字列・数の並びの小さな辞書（[`crate::plugin::Config`]）。
//!
//! 約束:
//! - [`SurfaceInstance::deflect`] は面の局所座標の交点（z = 0）と単位の入射方向から、単位の出射方向と
//!   OPD に足す光路長 [mm]（理想レンズの位相と同じ扱い。幾何的な光路長 `opl` には入れない）を返す。
//!   状態を持たず、どのスレッドから呼んでもよい。光線を落とすときは None（その面で err）
//! - 近軸の追跡（EFL・瞳・近軸像面）には [`SurfaceInstance::paraxial_power`]（1 / 焦点距離）を使う
//! - 偏光の追跡では、電場を方向の変化と同じ最小の回転で回す（振幅は変えない。理想レンズと同じ）
//! - dOPL/dp（[`crate::fermat`]）・着地点の微分（[`crate::landing`]）は、この面を通る光線では未対応（誤り）
//! - .zmx には書けない（系ファイル・プロジェクトには書ける）。前後の媒質は空気に限る（理想レンズと同じ）
//!
//! 同梱のモデル `ideal_mapping`（[`IdealMapping`]）は、画角 θ → 像高 h の関係を与える理想レンズ。

use crate::RwLockExt;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

use crate::math::{dot, norm, scale, sub, V3};
use crate::plugin::{Config, ConfigReader, ParamSpec, ParamType, Value};
use crate::{invalid, Result};

/// 利用者定義の面のモデル（設定から [`SurfaceInstance`] を作る）
pub trait SurfaceModel: Send + Sync {
    /// 登録の名前
    fn name(&self) -> &str;
    /// 1 行の説明
    fn doc(&self) -> String {
        String::new()
    }
    /// 設定の項目（GUI の欄・一覧用）
    fn params(&self) -> Vec<ParamSpec> {
        Vec::new()
    }
    /// 設定を確かめて面を作る
    fn prepare(&self, config: &Config) -> Result<Arc<dyn SurfaceInstance>>;
}

/// 設定の決まった利用者定義の面
pub trait SurfaceInstance: Send + Sync {
    /// 局所座標の交点 `p`（z = 0）・単位の入射方向 `k`（k[2] の符号が進む向き）・前後の屈折率・波長 [µm] から、
    /// 単位の出射方向と OPD に足す光路長 [mm]。None で光線を落とす
    fn deflect(&self, p: V3, k: V3, n_in: f64, n_out: f64, wl_um: f64) -> Option<(V3, f64)>;
    /// 近軸の屈折力 1 / f [1/mm]（近軸の追跡で使う。0 なら素通し）
    fn paraxial_power(&self) -> f64 {
        0.0
    }
}

/// 面に付ける利用者定義の面（モデルの名前・設定と、準備した面）。系ファイルには名前と設定だけを書き、
/// 読むときに登録から作り直す
#[derive(Clone)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "UserSurfSpec", into = "UserSurfSpec")
)]
pub struct UserSurf {
    pub model: String,
    pub config: Config,
    inst: Arc<dyn SurfaceInstance>,
}

/// [`UserSurf`] の書き出しの形
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct UserSurfSpec {
    /// モデルの名前（同梱は ideal_mapping）
    pub model: String,
    /// 設定（数・文字列・数の並び）
    #[cfg_attr(feature = "serde", serde(default))]
    pub config: Config,
}

impl TryFrom<UserSurfSpec> for UserSurf {
    type Error = crate::Error;
    fn try_from(s: UserSurfSpec) -> Result<Self> {
        UserSurf::new(&s.model, s.config)
    }
}

impl From<UserSurf> for UserSurfSpec {
    fn from(u: UserSurf) -> Self {
        UserSurfSpec {
            model: u.model,
            config: u.config,
        }
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for UserSurf {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        UserSurfSpec::schema_name()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        UserSurfSpec::json_schema(g)
    }
}

impl std::fmt::Debug for UserSurf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserSurf")
            .field("model", &self.model)
            .field("config", &self.config)
            .finish()
    }
}

impl PartialEq for UserSurf {
    fn eq(&self, o: &Self) -> bool {
        self.model == o.model && self.config == o.config
    }
}

impl UserSurf {
    /// 登録のモデル `model` を設定 `config` で準備する
    pub fn new(model: &str, config: Config) -> Result<UserSurf> {
        let m = get(model)?;
        let inst = m.prepare(&config)?;
        Ok(UserSurf {
            model: model.to_string(),
            config,
            inst,
        })
    }

    pub fn instance(&self) -> &dyn SurfaceInstance {
        self.inst.as_ref()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 登録

fn table() -> &'static RwLock<BTreeMap<String, Arc<dyn SurfaceModel>>> {
    static T: OnceLock<RwLock<BTreeMap<String, Arc<dyn SurfaceModel>>>> = OnceLock::new();
    T.get_or_init(|| {
        let mut m: BTreeMap<String, Arc<dyn SurfaceModel>> = BTreeMap::new();
        m.insert("ideal_mapping".into(), Arc::new(IdealMapping));
        RwLock::new(m)
    })
}

/// モデルを登録する（同じ名前は差し替える）。プロセスで共通
pub fn register(m: Arc<dyn SurfaceModel>) {
    let mut t = table().write_ok();
    t.insert(m.name().to_string(), m);
}

/// 登録を外す（あれば true）
pub fn unregister(name: &str) -> bool {
    let mut t = table().write_ok();
    t.remove(name).is_some()
}

/// 名前のモデル
pub fn get(name: &str) -> Result<Arc<dyn SurfaceModel>> {
    let t = table().read_ok();
    match t.get(name) {
        Some(m) => Ok(m.clone()),
        None => invalid!(
            "利用者定義の面のモデル {name} はない（登録: {}）",
            t.keys().cloned().collect::<Vec<_>>().join(", ")
        ),
    }
}

/// 登録のモデルの名前（名前の順）
pub fn models() -> Vec<String> {
    let t = table().read_ok();
    t.keys().cloned().collect()
}

// ---------------------------------------------------------------------------------------------------------------
// 同梱: 画角 → 像高を与える理想レンズ

/// 画角 θ → 像高 h の関係を与える理想レンズ（焦点距離 f の薄肉、頂点の平面）。
///
/// 入射方向 k の光線（θ = 光軸からの角、方位は k の xy の向き）は、焦点面（レンズから f）の点
/// P = (h(θ)·方位, f) に集まる。出射方向は交点から P への向き、OPD に足す光路長は
/// sign(f)·(|P| − |P − r| − k·r)（平行光の波面から P までの光路長を一定にする。同じ方向の光線はどれも P で
/// 無収差）。`tan`（h = f tan θ）は理想レンズ（`PARAXIAL`、物体が無限遠）とビット同じ。
///
/// 物体は無限遠を想定する（有限の物体でも光線の方向ごとに上の式で曲げる。`tan` 以外では有限の物点の像は
/// 無収差にならない）。
///
/// 設定: `f`（焦点距離 [mm]、必須、0 以外）、`mapping`（`tan`・`theta`（f θ、等距離射影）・`sin`（f sin θ、正射影）・
/// `equisolid`（2 f sin(θ/2)、等立体角）・`stereographic`（2 f tan(θ/2)、立体射影）・`poly`・`table`、既定 `tan`）、
/// `coef`（poly: h = f (θ + c₁ θ³ + c₂ θ⁵ + …) の c₁, c₂, …）、`theta_deg`・`height`（table: θ [deg]（0 から増える）と
/// h [mm] の表、区分 3 次エルミート（PCHIP）で補間、範囲の外は光線を落とす）。
pub struct IdealMapping;

#[derive(Debug, Clone)]
enum Mapping {
    Tan,
    Theta,
    Sin,
    Equisolid,
    Stereo,
    Poly(Vec<f64>),
    Table(Pchip),
}

/// 単調な区分 3 次エルミート（Fritsch–Carlson）
#[derive(Debug, Clone)]
struct Pchip {
    x: Vec<f64>,
    y: Vec<f64>,
    d: Vec<f64>,
}

impl Pchip {
    fn new(x: Vec<f64>, y: Vec<f64>) -> Result<Pchip> {
        let n = x.len();
        if n < 2 || y.len() != n {
            invalid!("表の theta_deg・height は同じ長さの 2 点以上に");
        }
        if x.windows(2).any(|w| !(w[1] > w[0])) || !x.iter().chain(&y).all(|v| v.is_finite()) {
            invalid!("表の theta_deg は増える順の有限の値に");
        }
        let h: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
        let s: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / h[i]).collect();
        let mut d = vec![0.0; n];
        if n == 2 {
            d = vec![s[0]; 2];
        } else {
            for i in 1..n - 1 {
                d[i] = if s[i - 1] * s[i] <= 0.0 {
                    0.0
                } else {
                    let (w1, w2) = (2.0 * h[i] + h[i - 1], h[i] + 2.0 * h[i - 1]);
                    (w1 + w2) / (w1 / s[i - 1] + w2 / s[i])
                };
            }
            let end = |h0: f64, h1: f64, s0: f64, s1: f64| {
                let e = ((2.0 * h0 + h1) * s0 - h0 * s1) / (h0 + h1);
                if e * s0 <= 0.0 {
                    0.0
                } else if s0 * s1 <= 0.0 && e.abs() > 3.0 * s0.abs() {
                    3.0 * s0
                } else {
                    e
                }
            };
            d[0] = end(h[0], h[1], s[0], s[1]);
            d[n - 1] = end(h[n - 2], h[n - 3], s[n - 2], s[n - 3]);
        }
        Ok(Pchip { x, y, d })
    }

    fn eval(&self, t: f64) -> Option<f64> {
        let n = self.x.len();
        if !(t >= self.x[0] && t <= self.x[n - 1]) {
            return None;
        }
        let i = match self.x.partition_point(|&v| v <= t) {
            0 => 0,
            k => (k - 1).min(n - 2),
        };
        let h = self.x[i + 1] - self.x[i];
        let u = (t - self.x[i]) / h;
        let (u2, u3) = (u * u, u * u * u);
        Some(
            (2.0 * u3 - 3.0 * u2 + 1.0) * self.y[i]
                + (u3 - 2.0 * u2 + u) * h * self.d[i]
                + (-2.0 * u3 + 3.0 * u2) * self.y[i + 1]
                + (u3 - u2) * h * self.d[i + 1],
        )
    }
}

/// 設定の決まった [`IdealMapping`]
#[derive(Debug, Clone)]
pub struct IdealMappingInst {
    f: f64,
    map: Mapping,
}

impl IdealMappingInst {
    /// 画角 θ [rad] の像高 [mm]（表の範囲の外は None）
    pub fn height(&self, th: f64) -> Option<f64> {
        let f = self.f;
        Some(match &self.map {
            Mapping::Tan => f * th.tan(),
            Mapping::Theta => f * th,
            Mapping::Sin => f * th.sin(),
            Mapping::Equisolid => 2.0 * f * (th / 2.0).sin(),
            Mapping::Stereo => 2.0 * f * (th / 2.0).tan(),
            Mapping::Poly(c) => {
                let t2 = th * th;
                let (mut s, mut p) = (th, th);
                for a in c {
                    p *= t2;
                    s += a * p;
                }
                f * s
            }
            Mapping::Table(t) => t.eval(th.to_degrees())?,
        })
    }

    /// h′(0) [mm/rad]（近軸の焦点距離）
    fn slope0(&self) -> f64 {
        match &self.map {
            Mapping::Table(t) => t.d[0].to_degrees(),
            _ => self.f,
        }
    }
}

impl SurfaceModel for IdealMapping {
    fn name(&self) -> &str {
        "ideal_mapping"
    }

    fn doc(&self) -> String {
        "画角 θ → 像高 h の関係を与える理想レンズ（f tan θ・f θ・f sin θ・等立体角・立体射影・多項式・表）".into()
    }

    fn params(&self) -> Vec<ParamSpec> {
        let ch = [
            "tan",
            "theta",
            "sin",
            "equisolid",
            "stereographic",
            "poly",
            "table",
        ];
        vec![
            ParamSpec::new("f", ParamType::Num, None, "mm", "焦点距離（0 以外、必須）"),
            ParamSpec::new(
                "mapping",
                ParamType::Choice(ch.iter().map(|s| s.to_string()).collect()),
                Some(Value::Str("tan".into())),
                "",
                "画角 → 像高の関係",
            ),
            ParamSpec::new(
                "coef",
                ParamType::Nums { len: None },
                None,
                "",
                "poly: h = f (θ + c₁ θ³ + c₂ θ⁵ + …) の c₁, c₂, …",
            ),
            ParamSpec::new(
                "theta_deg",
                ParamType::Nums { len: None },
                None,
                "deg",
                "table: 画角（0 から増える順）",
            ),
            ParamSpec::new(
                "height",
                ParamType::Nums { len: None },
                None,
                "mm",
                "table: 像高",
            ),
        ]
    }

    fn prepare(&self, config: &Config) -> Result<Arc<dyn SurfaceInstance>> {
        let mut r = ConfigReader::new(config);
        let f = r.num("f", f64::NAN)?;
        if !f.is_finite() || f == 0.0 {
            invalid!("ideal_mapping の f は 0 以外の有限の値に（{f}）");
        }
        let kind = r.str("mapping", "tan")?;
        let coef = r.nums("coef")?;
        let th = r.nums("theta_deg")?;
        let hh = r.nums("height")?;
        r.finish()?;
        let unused = |what: &str, v: bool| -> Result<()> {
            if v {
                invalid!("ideal_mapping の {what} は mapping = {kind} では使わない");
            }
            Ok(())
        };
        unused("coef", coef.is_some() && kind != "poly")?;
        unused(
            "theta_deg・height",
            (th.is_some() || hh.is_some()) && kind != "table",
        )?;
        let map = match kind.as_str() {
            "tan" => Mapping::Tan,
            "theta" => Mapping::Theta,
            "sin" => Mapping::Sin,
            "equisolid" => Mapping::Equisolid,
            "stereographic" => Mapping::Stereo,
            "poly" => Mapping::Poly(coef.unwrap_or_default()),
            "table" => {
                let (Some(t), Some(h)) = (th, hh) else {
                    invalid!("mapping = table には theta_deg と height が要る");
                };
                if t[0] != 0.0 || h[0] != 0.0 {
                    invalid!("表は θ = 0・h = 0 から始める（{} deg, {} mm）", t[0], h[0]);
                }
                let p = Pchip::new(t, h)?;
                if !(p.d[0] != 0.0) || p.d[0].signum() != f.signum() {
                    invalid!(
                        "表の θ = 0 の傾き（{} mm/deg）は f と同じ符号の 0 以外に",
                        p.d[0]
                    );
                }
                Mapping::Table(p)
            }
            k => invalid!(
                "mapping は tan・theta・sin・equisolid・stereographic・poly・table に（{k}）"
            ),
        };
        Ok(Arc::new(IdealMappingInst { f, map }))
    }
}

impl SurfaceInstance for IdealMappingInst {
    fn deflect(&self, pl: V3, kl: V3, _n_in: f64, _n_out: f64, _wl: f64) -> Option<(V3, f64)> {
        let f = self.f;
        if let Mapping::Tan = self.map {
            // 理想レンズ（物体が無限遠）の式そのもの（ビット同じ）
            return Some(crate::trace::paraxial(f, pl, kl, f64::INFINITY));
        }
        let sz = kl[2].signum();
        let t = [kl[0] / kl[2].abs(), kl[1] / kl[2].abs()];
        let r = [pl[0], pl[1], 0.0];
        // 焦点面の像点 P = (h(θ)·方位, f)。h(θ)/tan θ（θ → 0 は h′(0)）を t にかける
        let tt = t[0].hypot(t[1]);
        let g = if tt > 1e-12 {
            self.height(tt.atan())? / tt
        } else {
            self.slope0()
        };
        let ff = [g * t[0], g * t[1], f];
        if !ff.iter().all(|v| v.is_finite()) {
            return None;
        }
        let d = [(ff[0] - r[0]) / f, (ff[1] - r[1]) / f];
        let nn = (1.0 + d[0] * d[0] + d[1] * d[1]).sqrt();
        let k2 = [d[0] / nn, d[1] / nn, sz / nn];
        let kin = scale(kl, sz);
        Some((k2, f.signum() * (norm(ff) - norm(sub(ff, r)) - dot(kin, r))))
    }

    fn paraxial_power(&self) -> f64 {
        1.0 / self.slope0()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(kv: &[(&str, Value)]) -> Config {
        kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn tan_matches_the_paraxial_lens() {
        let u = UserSurf::new("ideal_mapping", cfg(&[("f", Value::Num(50.0))])).unwrap();
        for (p, k) in [
            ([1.0, -2.0, 0.0], [0.1, 0.05, 1.0]),
            ([0.0, 3.0, 0.0], [0.0, -0.3, 1.0]),
            ([2.0, 0.5, 0.0], [-0.2, 0.1, -1.0]),
        ] {
            let k = crate::math::normalize(k);
            let (k2, dl) = u.instance().deflect(p, k, 1.0, 1.0, 0.5).unwrap();
            let (k3, dl3) = crate::trace::paraxial(50.0, p, k, f64::INFINITY);
            assert_eq!((k2, dl), (k3, dl3));
        }
    }

    #[test]
    fn rays_of_one_direction_meet_at_h_theta_with_equal_path() {
        for (m, h) in [
            ("theta", 50.0 * 0.3),
            ("sin", 50.0 * 0.3f64.sin()),
            ("equisolid", 100.0 * 0.15f64.sin()),
            ("stereographic", 100.0 * 0.15f64.tan()),
        ] {
            let u = UserSurf::new(
                "ideal_mapping",
                cfg(&[("f", Value::Num(50.0)), ("mapping", Value::Str(m.into()))]),
            )
            .unwrap();
            let th: f64 = 0.3;
            let k = [0.0, th.sin(), th.cos()];
            let mut lens = Vec::new();
            for y in [-5.0, -1.0, 0.0, 2.0, 4.0] {
                for x in [-3.0, 0.0, 1.5] {
                    let p = [x, y, 0.0];
                    let (k2, dl) = u.instance().deflect(p, k, 1.0, 1.0, 0.5).unwrap();
                    // 焦点面 z = 50 での位置
                    let s = 50.0 / k2[2];
                    let q = [x + s * k2[0], y + s * k2[1]];
                    assert!(
                        q[0].abs() < 1e-9 && (q[1] - h).abs() < 1e-9,
                        "{m}: {q:?} {h}"
                    );
                    // 平行光の波面（原点を通る）から P までの光路長 + 位相が一定
                    lens.push(dot(k, p) + s + dl);
                }
            }
            let (lo, hi) = lens
                .iter()
                .fold((f64::MAX, f64::MIN), |a, &v| (a.0.min(v), a.1.max(v)));
            assert!(hi - lo < 1e-9, "{m}: {lo} {hi}");
        }
    }

    #[test]
    fn poly_table_and_errors() {
        let poly = UserSurf::new(
            "ideal_mapping",
            cfg(&[
                ("f", Value::Num(20.0)),
                ("mapping", Value::Str("poly".into())),
                ("coef", Value::Nums(vec![-0.1, 0.01])),
            ]),
        )
        .unwrap();
        let th = 0.4f64;
        let (k2, _) = poly
            .instance()
            .deflect([0.0; 3], [0.0, th.sin(), th.cos()], 1.0, 1.0, 0.5)
            .unwrap();
        let want = 20.0 * (th - 0.1 * th.powi(3) + 0.01 * th.powi(5));
        assert!((20.0 * k2[1] / k2[2] - want).abs() < 1e-12);
        assert!((poly.instance().paraxial_power() - 1.0 / 20.0).abs() < 1e-15);
        // 表: f θ を表で与えると theta と同じ（PCHIP は直線を正確に再現）
        let deg: Vec<f64> = (0..=8).map(|i| 10.0 * i as f64).collect();
        let hs: Vec<f64> = deg.iter().map(|d| 20.0 * d.to_radians()).collect();
        let tab = UserSurf::new(
            "ideal_mapping",
            cfg(&[
                ("f", Value::Num(20.0)),
                ("mapping", Value::Str("table".into())),
                ("theta_deg", Value::Nums(deg)),
                ("height", Value::Nums(hs)),
            ]),
        )
        .unwrap();
        let th_ = UserSurf::new(
            "ideal_mapping",
            cfg(&[
                ("f", Value::Num(20.0)),
                ("mapping", Value::Str("theta".into())),
            ]),
        )
        .unwrap();
        let k = [0.0, 0.5f64.sin(), 0.5f64.cos()];
        let a = tab
            .instance()
            .deflect([0.3, 1.0, 0.0], k, 1.0, 1.0, 0.5)
            .unwrap();
        let b = th_
            .instance()
            .deflect([0.3, 1.0, 0.0], k, 1.0, 1.0, 0.5)
            .unwrap();
        assert!((a.0[1] - b.0[1]).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12);
        assert!((tab.instance().paraxial_power() - 1.0 / 20.0).abs() < 1e-12);
        // 表の外（85°）は光線を落とす
        let far = [0.0, 85f64.to_radians().sin(), 85f64.to_radians().cos()];
        assert!(tab
            .instance()
            .deflect([0.0; 3], far, 1.0, 1.0, 0.5)
            .is_none());
        for c in [
            cfg(&[]),
            cfg(&[("f", Value::Num(0.0))]),
            cfg(&[("f", Value::Num(1.0)), ("mapping", Value::Str("x".into()))]),
            cfg(&[("f", Value::Num(1.0)), ("coef", Value::Nums(vec![1.0]))]),
            cfg(&[
                ("f", Value::Num(1.0)),
                ("mapping", Value::Str("table".into())),
                ("theta_deg", Value::Nums(vec![1.0, 2.0])),
                ("height", Value::Nums(vec![0.0, 1.0])),
            ]),
            cfg(&[("f", Value::Num(1.0)), ("g", Value::Num(1.0))]),
        ] {
            assert!(UserSurf::new("ideal_mapping", c.clone()).is_err(), "{c:?}");
        }
        assert!(UserSurf::new("none", cfg(&[])).is_err());
    }
}
