//! 利用者が定義する形の面の口（面の種類 [`crate::surface::SurfType::UserShape`]）。
//!
//! 面の形は、土台の二次曲面（面の曲率 c・円錐係数 k）に、登録したモデルの矢高を足したもの。働きはふつうの屈折・
//! 反射で、材料・膜・偏光の追跡はほかの面と同じように効く（形は法線を通してだけ効く）。曲げ方と位相だけを
//! 決める利用者定義の面（[`crate::usersurf`]、種類 USER）とは別の口。
//!
//! [`ShapeModel`] を実装して拡張の表（[`crate::ext::Ext::add_shape_model`]）に足し、種類 USERSHAPE の面に
//! `shape = {"model": 名前, "config": {…}}`（[`UserShape`]）を与える。設定は検出器・解析のプラグインと同じ
//! 数・文字列・数の並びの小さな辞書（[`crate::plugin::Config`]）。
//!
//! 約束（[`check_shape`] で確かめられる）:
//! - [`ShapeInstance::sag`] は面の局所座標 (x, y) で、土台に足す矢高と勾配 (z, ∂z/∂x, ∂z/∂y) を返す。状態を
//!   持たず、どのスレッドから呼んでもよい。クリア半径の少し外（交点のニュートン法が通る所）まで有限で、
//!   勾配は連続（着地点の微分は勾配の差分で 2 階を求める）。面が無い所は NaN（その光線は落ちる）
//! - [`ShapeInstance::dsag`] は設定の数の項目（名前）による (z, ∂z/∂x, ∂z/∂y) の微分。返す項目は変数・
//!   微分（dOPL/dp・着地点の微分）にでき、パラメータの種類は [`crate::param::Kind::User`]（`u_<名前>`）。
//!   値を変えるときは設定のその項目を書き換えて [`ShapeModel::prepare`] で作り直すので、prepare は軽くする
//!   （重い前処理はモデルの中で使い回す）
//! - 鏡の後に置く・玉を反転するときの裏返しは core が包む（z → −z は矢高の符号、x → −x は x の向き）。
//!   モデルは何もしなくてよい
//! - 近軸の曲率（EFL・瞳など）は、土台の c に、頂点での矢高の 2 階（YZ 面、勾配の中心差分。幅はクリア半径の
//!   1e-6）を足したもの。近軸は YZ 面だけで、XZ 面の曲率は見ない（ほかの回転対称でない面と同じ）
//! - [`ShapeInstance::phase`] を返すモデルは、面の上の位相 Φ [rad] も持つ（回折・メタ面）。扱いは Binary 2 と同じ: 屈折・
//!   反射の後に回折の式で向きを変え（Φ の勾配 × λ/2π を接線の成分に足す）、光路長に λ/2π · Φ を足す（λ はレンズの単位）。
//!   電場は方向の変化と同じ最小の回転で回す。近軸は頂点での ∂²Φ/∂y²（勾配の中心差分）を屈折力に足す。回折次数・波長に
//!   よらない位相（rad）を返し、次数を変えたいときは設定に持つ。裏返し（z）では変えず、x の映しは x の向きを替える。
//!   dOPL/dp・着地点の微分はこの面を通る光線では未対応（Binary 2 と同じく誤り）
//! - .zmx には書けない（系ファイル・プロジェクトには書ける）

use std::collections::HashSet;
use std::sync::{Arc, OnceLock, RwLock};

use crate::plugin::{Config, ParamSpec, Value};
use crate::{invalid, Result, RwLockExt};

/// 利用者定義の形のモデル（設定から [`ShapeInstance`] を作る）
pub trait ShapeModel: Send + Sync {
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
    /// 設定を確かめて形を作る（軽くする。最適化・公差では値を変えるたびに呼ぶ）
    fn prepare(&self, config: &Config) -> Result<Arc<dyn ShapeInstance>>;
    /// 試しの設定（拡張の検査で [`check_shape`] に渡す）。既定は [`ShapeModel::params`] の既定値を並べたもの
    /// （既定値の無い項目があれば None。そのときは自分で返す）
    fn example_config(&self) -> Option<Config> {
        self.params()
            .into_iter()
            .map(|p| Some((p.name, p.default?)))
            .collect()
    }
    /// 試しの設定で確かめる半径 [mm]（形が有効な半直径。既定 10）
    fn example_radius(&self) -> f64 {
        10.0
    }
}

/// 準備した形
pub trait ShapeInstance: Send + Sync {
    /// 土台の二次曲面に足す矢高と勾配 (z, ∂z/∂x, ∂z/∂y)（面の局所座標）
    fn sag(&self, x: f64, y: f64) -> (f64, f64, f64);
    /// 設定の数の項目 `key` による (z, ∂z/∂x, ∂z/∂y) の微分。None はその項目では微分できない（変数にできない）
    fn dsag(&self, key: &str, x: f64, y: f64) -> Option<[f64; 3]> {
        let _ = (key, x, y);
        None
    }
    /// 面の上の位相と勾配 (Φ [rad], ∂Φ/∂x, ∂Φ/∂y)（面の局所座標）。None は位相なし（既定）。頂点 (0, 0) で
    /// None を返すモデルは位相を持たないとみなし、ほかの点でも呼ばない
    fn phase(&self, x: f64, y: f64) -> Option<(f64, f64, f64)> {
        let _ = (x, y);
        None
    }
}

/// 面に付ける利用者定義の形（モデルの名前・設定・裏返しの向きと、準備した形）。系ファイルには名前・設定・向きを
/// 書き、読むときに登録から作り直す
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "UserShapeSpec", into = "UserShapeSpec")]
pub struct UserShape {
    pub model: String,
    pub config: Config,
    /// z について裏返した（矢高の符号が逆）
    pub flip_z: bool,
    /// x について映した（x の向きが逆）
    pub mirror_x: bool,
    inst: Arc<dyn ShapeInstance>,
    /// 位相を持つ（頂点で [`ShapeInstance::phase`] が Some）
    has_phase: bool,
}

/// [`UserShape`] の書き出しの形
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct UserShapeSpec {
    /// モデルの名前
    pub model: String,
    /// 設定（数・文字列・数の並び）
    #[serde(default)]
    pub config: Config,
    /// z について裏返した（鏡の後・玉の反転。矢高の符号が逆）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip_z: bool,
    /// x について映した（玉の反転。x の向きが逆）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror_x: bool,
}

impl TryFrom<UserShapeSpec> for UserShape {
    type Error = crate::Error;
    fn try_from(s: UserShapeSpec) -> Result<Self> {
        let mut u = UserShape::new(&s.model, s.config)?;
        u.flip_z = s.flip_z;
        u.mirror_x = s.mirror_x;
        Ok(u)
    }
}

impl From<UserShape> for UserShapeSpec {
    fn from(u: UserShape) -> Self {
        UserShapeSpec {
            model: u.model,
            config: u.config,
            flip_z: u.flip_z,
            mirror_x: u.mirror_x,
        }
    }
}

impl schemars::JsonSchema for UserShape {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        UserShapeSpec::schema_name()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        UserShapeSpec::json_schema(g)
    }
}

impl std::fmt::Debug for UserShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserShape")
            .field("model", &self.model)
            .field("config", &self.config)
            .field("flip_z", &self.flip_z)
            .field("mirror_x", &self.mirror_x)
            .finish()
    }
}

impl PartialEq for UserShape {
    fn eq(&self, o: &Self) -> bool {
        self.model == o.model
            && self.config == o.config
            && self.flip_z == o.flip_z
            && self.mirror_x == o.mirror_x
    }
}

impl UserShape {
    /// 今の拡張の表（[`crate::ext::current`]）のモデル `model` を設定 `config` で準備する（裏返しなし）
    pub fn new(model: &str, config: Config) -> Result<UserShape> {
        let m = crate::ext::current().shape_model(model)?;
        let inst = m.prepare(&config)?;
        for (k, v) in &config {
            if matches!(v, Value::Num(_)) {
                UserKey::intern(k);
            }
        }
        let has_phase = inst.phase(0.0, 0.0).is_some();
        Ok(UserShape {
            model: model.to_string(),
            config,
            flip_z: false,
            mirror_x: false,
            inst,
            has_phase,
        })
    }

    /// 準備した形（裏返しを含まない、モデルが返したまま）
    pub fn instance(&self) -> &dyn ShapeInstance {
        self.inst.as_ref()
    }

    fn signs(&self) -> (f64, f64) {
        (
            if self.flip_z { -1.0 } else { 1.0 },
            if self.mirror_x { -1.0 } else { 1.0 },
        )
    }

    /// 裏返しを含めた、土台に足す矢高と勾配
    #[inline]
    pub fn sag(&self, x: f64, y: f64) -> (f64, f64, f64) {
        let (s, m) = self.signs();
        let (z, zx, zy) = self.inst.sag(m * x, y);
        (s * z, s * m * zx, s * zy)
    }

    /// 裏返しを含めた、項目 `key` による微分（[`ShapeInstance::dsag`]）
    pub fn dsag(&self, key: &str, x: f64, y: f64) -> Option<[f64; 3]> {
        let (s, m) = self.signs();
        let d = self.inst.dsag(key, m * x, y)?;
        Some([s * d[0], s * m * d[1], s * d[2]])
    }

    /// 位相を持つか（[`ShapeInstance::phase`]）
    pub fn has_phase(&self) -> bool {
        self.has_phase
    }

    /// 裏返しを含めた位相と勾配（位相を持たなければ None）。z の裏返しでは変えず、x の映しは x の向きを替える
    #[inline]
    pub fn phase(&self, x: f64, y: f64) -> Option<(f64, f64, f64)> {
        if !self.has_phase {
            return None;
        }
        let (_, m) = self.signs();
        let (p, px, py) = self.inst.phase(m * x, y)?;
        Some((p, m * px, py))
    }

    /// 頂点での位相の 2 階 ∂²Φ/∂y² [rad/mm²]（近軸の屈折力に足す分。幅 h の勾配の中心差分）。位相なしは 0
    pub fn paraxial_dphi(&self, h: f64) -> f64 {
        match (self.phase(0.0, h), self.phase(0.0, -h)) {
            (Some(a), Some(b)) => (a.2 - b.2) / (2.0 * h),
            _ => 0.0,
        }
    }

    /// 設定の数の項目 `key` の値（無い・数（`Num`）でなければ None。数の並びは変数にできない）
    pub fn value(&self, key: &str) -> Option<f64> {
        match self.config.get(key)? {
            Value::Num(v) => Some(*v),
            _ => None,
        }
    }

    /// 項目 `key` を値 `v` にして作り直したもの（向きはそのまま）
    pub fn with_value(&self, key: &str, v: f64) -> Result<UserShape> {
        let mut config = self.config.clone();
        config.insert(key.to_string(), Value::Num(v));
        let mut u = UserShape::new(&self.model, config)?;
        u.flip_z = self.flip_z;
        u.mirror_x = self.mirror_x;
        Ok(u)
    }

    /// 頂点での矢高の 2 階 ∂²z/∂y²（近軸の曲率に足す分。幅 h [mm] の勾配の中心差分、[`paraxial_step`]）
    pub fn paraxial_curv(&self, h: f64) -> f64 {
        let (_, _, a) = self.sag(0.0, h);
        let (_, _, b) = self.sag(0.0, -h);
        (a - b) / (2.0 * h)
    }
}

/// 登録したモデルが [`ShapeModel::params`] で宣言した数の項目の名前を、パラメータの名前の表に足す（拡張の表に
/// 足すときに呼ぶ。パラメータ `u_<項目>` を、その項目を持つ形を準備する前に読めるように）
pub(crate) fn declare(m: &dyn ShapeModel) {
    for p in m.params() {
        if p.ty == crate::plugin::ParamType::Num && !p.name.is_empty() && !p.name.contains('.') {
            UserKey::intern(&p.name);
        }
    }
}

/// 近軸の曲率を求める差分の幅 [mm]: クリア半径の 1e-6（半径が決まっていなければ 1e-5 mm）
pub fn paraxial_step(semi: f64) -> f64 {
    if semi > 0.0 && semi.is_finite() {
        1e-6 * semi
    } else {
        1e-5
    }
}

/// 利用者定義の形の項目の名前（[`crate::param::Kind::User`]）。`Kind` を Copy のまま名前で持つため、名前をプロセスの
/// 表に 1 つだけ置いた文字列への参照にしたもの（光線ごとの微分で表を引かない）。表に足すのは準備できた形の設定の
/// 数の項目だけ（外から来た誤った名前で増えない）。serde・表示は名前の文字列。
///
/// 登録したモデルが [`ShapeModel::params`] で宣言した数の項目は、形を準備する前（系より先に変数の行を読むときなど）
/// から読める:
///
/// ```
/// use lensbench_core::param::Kind;
/// use lensbench_core::plugin::{Config, ParamSpec, ParamType};
/// use lensbench_core::usershape::{ShapeInstance, ShapeModel};
/// use std::sync::Arc;
///
/// struct Bump;
/// impl ShapeModel for Bump {
///     fn name(&self) -> &str { "doc_bump" }
///     fn params(&self) -> Vec<ParamSpec> {
///         vec![ParamSpec::new("height", ParamType::Num, None, "mm", "")]
///     }
///     fn prepare(&self, _: &Config) -> lensbench_core::Result<Arc<dyn ShapeInstance>> {
///         unimplemented!()
///     }
/// }
///
/// // 新しいプロセスで、形を何も準備していないうちは読めない
/// assert!(Kind::parse("u_height").is_err());
/// let mut ext = lensbench_core::ext::Ext::default();
/// ext.add_shape_model(Arc::new(Bump));
/// assert!(Kind::parse("u_height").is_ok());
/// // 宣言していない名前は表に足さない
/// assert!(Kind::parse("u_nothing").is_err());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct UserKey(&'static str);

impl PartialEq for UserKey {
    fn eq(&self, o: &Self) -> bool {
        // 表に 1 つだけ置くので、同じ名前は同じ場所
        std::ptr::eq(self.0, o.0)
    }
}
impl Eq for UserKey {}
impl std::hash::Hash for UserKey {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        (self.0.as_ptr() as usize).hash(h)
    }
}
impl PartialOrd for UserKey {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for UserKey {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.cmp(o.0)
    }
}

fn names() -> &'static RwLock<HashSet<&'static str>> {
    static N: OnceLock<RwLock<HashSet<&'static str>>> = OnceLock::new();
    N.get_or_init(Default::default)
}

impl UserKey {
    /// 表にある名前（登録したモデルが宣言した項目と、準備した形の設定の数の項目）。無ければ None。
    /// 読むとき（serde）も同じ表を引く:
    ///
    /// ```
    /// use lensbench_core::plugin::{Config, ParamSpec, ParamType};
    /// use lensbench_core::usershape::{ShapeInstance, ShapeModel, UserKey};
    /// use std::sync::Arc;
    ///
    /// struct Tilt;
    /// impl ShapeModel for Tilt {
    ///     fn name(&self) -> &str { "doc_tilt" }
    ///     fn params(&self) -> Vec<ParamSpec> {
    ///         vec![ParamSpec::new("slope", ParamType::Num, None, "", "")]
    ///     }
    ///     fn prepare(&self, _: &Config) -> lensbench_core::Result<Arc<dyn ShapeInstance>> {
    ///         unimplemented!()
    ///     }
    /// }
    ///
    /// // 新しいプロセスで、登録の前は読めない
    /// assert!(serde_json::from_str::<UserKey>("\"slope\"").is_err());
    /// lensbench_core::ext::Ext::default().add_shape_model(Arc::new(Tilt));
    /// let k: UserKey = serde_json::from_str("\"slope\"").unwrap();
    /// assert_eq!((k.name(), UserKey::get("slope")), ("slope", Some(k)));
    /// assert!(serde_json::from_str::<UserKey>("\"nothing\"").is_err());
    /// ```
    pub fn get(name: &str) -> Option<UserKey> {
        names().read_ok().get(name).map(|&n| UserKey(n))
    }

    /// 名前を表に足す（初めての名前だけ。登録したモデルが宣言した項目と、準備できた形の設定の項目から呼ぶ）
    fn intern(name: &str) -> UserKey {
        if let Some(k) = UserKey::get(name) {
            return k;
        }
        let mut n = names().write_ok();
        if let Some(&s) = n.get(name) {
            return UserKey(s);
        }
        let s: &'static str = Box::leak(name.to_string().into_boxed_str());
        n.insert(s);
        UserKey(s)
    }

    /// 名前
    pub fn name(&self) -> &'static str {
        self.0
    }
}

impl serde::Serialize for UserKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

impl<'de> serde::Deserialize<'de> for UserKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        UserKey::get(&s).ok_or_else(|| {
            serde::de::Error::custom(format!("no user-defined shape has the number setting {s}"))
        })
    }
}

impl schemars::JsonSchema for UserKey {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "UserKey".into()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        String::json_schema(g)
    }
}

/// モデルが約束を守るかを確かめる（拡張の試験から呼ぶ）。半径 `radius` の 1.2 倍の円の内の格子の点で:
/// - 矢高・勾配が有限
/// - 勾配が矢高の中心差分と合う
/// - [`ShapeInstance::dsag`] を返す項目は、設定のその項目を少し変えて作り直した矢高・勾配の中心差分と合う
/// - 位相を持つ（頂点で [`ShapeInstance::phase`] が Some）なら、同じ点で位相が有限で、勾配が位相の中心差分と合う
pub fn check_shape(model: &dyn ShapeModel, config: &Config, radius: f64) -> Result<()> {
    let name = model.name();
    if !(radius > 0.0 && radius.is_finite()) {
        invalid!("check_shape: radius must be positive ({radius})");
    }
    let base = model.prepare(config)?;
    let r = 1.2 * radius;
    let n = 7;
    let pts: Vec<(f64, f64)> = (0..n)
        .flat_map(|i| (0..n).map(move |j| (i, j)))
        .map(|(i, j)| {
            let t = |k: usize| -r + 2.0 * r * k as f64 / (n - 1) as f64;
            (t(i), t(j))
        })
        // 円（半径の 1.2 倍）の内だけ
        .filter(|&(x, y)| x * x + y * y <= r * r * (1.0 + 1e-12))
        .collect();
    let h = 1e-6 * radius;
    for &(x, y) in &pts {
        let (z, zx, zy) = base.sag(x, y);
        if !(z.is_finite() && zx.is_finite() && zy.is_finite()) {
            invalid!("user-defined shape model {name}: the sag is not finite at ({x}, {y}) (inside 1.2 × radius)");
        }
        let nx = (base.sag(x + h, y).0 - base.sag(x - h, y).0) / (2.0 * h);
        let ny = (base.sag(x, y + h).0 - base.sag(x, y - h).0) / (2.0 * h);
        let tol = |a: f64| 1e-5 * (1.0 + a.abs());
        if (nx - zx).abs() > tol(zx) || (ny - zy).abs() > tol(zy) {
            invalid!("user-defined shape model {name}: the gradient ({zx}, {zy}) at ({x}, {y}) does not match the difference of the sag ({nx}, {ny})");
        }
        if base.phase(0.0, 0.0).is_some() {
            let ph = |x: f64, y: f64| base.phase(x, y).map(|p| p.0).unwrap_or(f64::NAN);
            let fin = |p: &(f64, f64, f64)| p.0.is_finite() && p.1.is_finite() && p.2.is_finite();
            let Some((_, px, py)) = base.phase(x, y).filter(fin) else {
                invalid!("user-defined shape model {name}: the phase is not finite at ({x}, {y}) (inside 1.2 × radius)");
            };
            let nx = (ph(x + h, y) - ph(x - h, y)) / (2.0 * h);
            let ny = (ph(x, y + h) - ph(x, y - h)) / (2.0 * h);
            let tol = |a: f64| 1e-5 * (1.0 + a.abs());
            if !((nx - px).abs() <= tol(px) && (ny - py).abs() <= tol(py)) {
                invalid!("user-defined shape model {name}: the phase gradient ({px}, {py}) at ({x}, {y}) does not match the difference of the phase ({nx}, {ny})");
            }
        }
    }
    for (key, v) in config {
        let v0 = match v {
            Value::Num(v) => *v,
            _ => continue,
        };
        if base.dsag(key, 0.0, 0.0).is_none() {
            continue;
        }
        let dv = 1e-6 * (1.0 + v0.abs());
        let at = |val: f64| -> Result<Arc<dyn ShapeInstance>> {
            let mut c = config.clone();
            c.insert(key.clone(), Value::Num(val));
            model.prepare(&c)
        };
        let (p, m) = (at(v0 + dv)?, at(v0 - dv)?);
        for &(x, y) in &pts {
            let Some(d) = base.dsag(key, x, y) else {
                invalid!("user-defined shape model {name}: dsag of {key} is given at the vertex but not at ({x}, {y})");
            };
            let (a, b) = (p.sag(x, y), m.sag(x, y));
            let num = [
                (a.0 - b.0) / (2.0 * dv),
                (a.1 - b.1) / (2.0 * dv),
                (a.2 - b.2) / (2.0 * dv),
            ];
            for i in 0..3 {
                if (num[i] - d[i]).abs() > 1e-4 * (1.0 + d[i].abs()) {
                    invalid!("user-defined shape model {name}: dsag of {key} at ({x}, {y}) is {d:?} but the difference of the sag gives {num:?}");
                }
            }
        }
    }
    Ok(())
}
