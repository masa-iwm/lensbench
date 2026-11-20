//! serve の結果のうち GUI が型で受けるもの（型で受ける一歩目）。serve はこの型を JSON にし、GUI は同じ型で読む
//! （鍵の綴りをコンパイラが見る。JSON の鍵の並びはフィールドの並び）
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// JSON の値を型に読む（足りない鍵・型の違いは誤り）
pub fn read<T: serde::de::DeserializeOwned>(v: &Value) -> lensbench_core::Result<T> {
    Ok(T::deserialize(v).map_err(|e| e.to_string())?)
}

/// 元に戻せる段・やり直せる段の数と、その変更（`undo_list` は新しい順、`redo_list` は次にやり直す順）。
/// undo・redo・history の結果と、系を変える要求の結果に入る
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryView {
    pub undo: usize,
    pub redo: usize,
    /// 段の変更の短い記述（`{"set": [...]}`・`{"edit": [...]}`・`{"open": ...}` など）
    pub undo_list: Vec<Value>,
    pub redo_list: Vec<Value>,
}

/// 玉・群（surface の結果の units の 1 つ）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UnitView {
    pub name: String,
    /// 面の範囲 (最初, 最後) の並び
    pub spans: Vec<[usize; 2]>,
    /// 読込で作った玉（lens1 …）
    pub auto: bool,
    /// 剛体の動き dx dy tx ty tz [mm, deg]（NaN は JSON の null）
    #[serde(deserialize_with = "nan_for_null_5")]
    pub motion: [f64; 5],
    /// 群の移動 [mm]
    #[serde(deserialize_with = "nan_for_null_1")]
    pub shift: f64,
    /// 範囲の中の媒質の dn・disp（媒質が無いか面ごとに違えば None）
    pub dn: Option<f64>,
    pub disp: Option<f64>,
}

/// 多重構成の表（config_table の結果）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigTable {
    /// 構成の数（表が無ければ 1）
    pub n: usize,
    /// 今の構成（1 始まり）
    pub active: usize,
    pub rows: Vec<ConfigRow>,
    /// .zmx の多重構成の数（.zmx でなければ 1）と、読んだ構成
    pub zmx_cfgs: usize,
    pub zmx_cfg: usize,
}

impl Default for ConfigTable {
    fn default() -> Self {
        ConfigTable {
            n: 1,
            active: 1,
            rows: Vec::new(),
            zmx_cfgs: 1,
            zmx_cfg: 1,
        }
    }
}

/// 多重構成の表の行
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigRow {
    /// パラメータの名前（set と同じ: `3.t`・`G1.shift` など）
    pub param: String,
    /// 構成ごとの値（NaN は JSON の null）
    #[serde(deserialize_with = "nan_for_null")]
    pub values: Vec<f64>,
    /// 材料の行（`N.material`）の材料の名前。values はこの番号（0 始まり）。ほかの行は空で、省く
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    pub picks: Vec<ConfigPick>,
}

/// 構成のピックアップ（core の型をそのまま）
pub use lensbench_core::configs::ConfigPick;

/// serde_json は NaN を null に書くので、読むときは null を NaN に戻す
fn nan_for_null<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f64>, D::Error> {
    let v = Vec::<Option<f64>>::deserialize(d)?;
    Ok(v.into_iter().map(|x| x.unwrap_or(f64::NAN)).collect())
}

fn nan_for_null_5<'de, D: Deserializer<'de>>(d: D) -> Result<[f64; 5], D::Error> {
    let v = <[Option<f64>; 5]>::deserialize(d)?;
    Ok(v.map(|x| x.unwrap_or(f64::NAN)))
}

fn nan_for_null_1<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(d)?.unwrap_or(f64::NAN))
}

/// 最適化（optimize）の結果
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OptimizeView {
    /// 変数の名前
    pub names: Vec<String>,
    /// 変数の初めの値・最適化した値
    #[serde(deserialize_with = "nan_for_null")]
    pub x0: Vec<f64>,
    #[serde(deserialize_with = "nan_for_null")]
    pub x: Vec<f64>,
    /// 読み込んだ系の構成に効く変数か（apply で当てられるもの）
    pub applicable: Vec<bool>,
    /// 読み込んだ系に当てたか
    pub applied: bool,
    #[serde(deserialize_with = "nan_for_null_1")]
    pub merit0: f64,
    #[serde(deserialize_with = "nan_for_null_1")]
    pub merit: f64,
    /// 反復ごとの評価関数の値（初期値を含む）
    #[serde(deserialize_with = "nan_for_null")]
    pub history: Vec<f64>,
    pub iterations: usize,
    pub evals: usize,
    /// 止まった理由（converged・small_step・no_progress・max_iter・jacobian_failed・cancelled）
    pub stop: String,
    /// 項ごとの値（既定の評価関数の項が先）
    pub terms: Vec<TermValue>,
    pub notes: Vec<String>,
}

/// 評価関数の項の値（最適化の前と後）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TermValue {
    /// 既定の評価関数の項か
    pub default: bool,
    #[serde(deserialize_with = "nan_for_null_1")]
    pub value0: f64,
    #[serde(deserialize_with = "nan_for_null_1")]
    pub value: f64,
    /// 値の意味（目標のある項は量そのもの quantity、ない項は √Σr² の rms）
    pub value_kind: String,
    /// 評価関数への寄与 [%]（最適化の前・後。重みを掛けた残差の 2 乗の和の割合、OpticStudio の % Contrib）
    #[serde(default)]
    pub contrib0: f64,
    #[serde(default)]
    pub contrib: f64,
    /// 瞳の光線を並べる項の、最適化の後の光線の数
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rays: Option<RayCount>,
}

/// 項の光線の数（全部・蹴られた・止まった）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RayCount {
    pub total: usize,
    pub vignetted: usize,
    pub stopped: usize,
}

/// ズーム軌跡（zoom）の結果。点ごとの変数の値は点の `vars` の名前の鍵（[`ZoomPoint::extra`]）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ZoomView {
    /// 近軸量（efl・pmag）
    pub quantity: String,
    /// 像の評価関数（spot・wavefront）
    pub merit_kind: String,
    /// 解いたパラメータの名前
    pub vars: Vec<String>,
    pub path: Vec<ZoomPoint>,
    pub warnings: Vec<String>,
    /// 読み込んだ後の変更を当てなかった説明（serve）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// ズーム軌跡の点
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ZoomPoint {
    /// 状態の間の位置（状態 k は k）
    pub pos: f64,
    /// 状態の名前（間の点は None）
    pub state: Option<String>,
    pub target: Option<f64>,
    /// 像の評価関数の値（NaN は null）
    #[serde(deserialize_with = "nan_for_null_1")]
    pub merit: f64,
    /// 止まった理由（解けなかった点は failed）
    pub stop: String,
    /// 解けなかった理由
    pub error: Option<String>,
    pub iterations: usize,
    /// 近軸量（`quantity` の名前の鍵）と変数の値（変数の名前の鍵）
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// 面の値（surface の結果の surfaces の 1 つ）。鍵と並びは `edit_ops::surf_fields_json` と同じで、面のソルブ・ピックアップを
/// 続けて持つ。inf・NaN は `{"$f": "inf"}` の形（系ファイルと同じ）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceView {
    /// 面タイプの名前（STANDARD・COORDBRK など）
    #[serde(rename = "type")]
    pub typ: String,
    pub comment: String,
    #[serde(with = "fnum")]
    pub t: f64,
    #[serde(with = "fnum")]
    pub c: f64,
    /// 曲率半径（平面は inf）
    #[serde(with = "fnum")]
    pub r: f64,
    #[serde(with = "fnum")]
    pub k: f64,
    /// 非球面の係数（面タイプのパラメータ）
    #[serde(with = "fnums")]
    pub asph: Vec<f64>,
    /// x の曲率（Toroidal）
    #[serde(with = "fnum")]
    pub cx: f64,
    pub material: String,
    pub mirror: bool,
    pub coating: String,
    #[serde(with = "fnum")]
    pub semi: f64,
    pub semi_auto: bool,
    /// 玉の半径（自動なら求めた値）と、自動か・チップゾーン
    #[serde(with = "fnum")]
    pub mech: f64,
    pub mech_auto: bool,
    #[serde(with = "fnum")]
    pub chip: f64,
    pub stop: bool,
    pub ignore: bool,
    /// 開口（無ければ null、ある時は {"type": "circular", ...}）
    pub aperture: Value,
    /// 座標ブレークの偏心 x y・傾き x y z
    #[serde(with = "fnums")]
    pub cb: Vec<f64>,
    #[serde(with = "fnum")]
    pub cb_order: f64,
    /// 焦点距離（Paraxial）
    #[serde(with = "fnum")]
    pub focal: f64,
    /// Jones 行列（実・虚 ×4）
    #[serde(with = "fnums")]
    pub jones: Vec<f64>,
    #[serde(with = "fnum")]
    pub dn: f64,
    #[serde(with = "fnum")]
    pub disp: f64,
    /// 波長ごとの屈折率
    #[serde(with = "fnums")]
    pub n: Vec<f64>,
    pub solves: Vec<SolveView>,
    pub pickups: Vec<PickupView>,
}

/// 面のソルブ（surface_data・surface の solves の 1 つ。有限でない値は null）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolveView {
    /// 解く量（c か t）
    pub what: String,
    /// ソルブの種類の名前（marginal_height など）
    pub kind: String,
    #[serde(deserialize_with = "nan_for_null_1")]
    pub value: f64,
    #[serde(deserialize_with = "nan_for_null_1")]
    pub param: f64,
    /// 参照する面
    pub other: Option<usize>,
}

/// 面のピックアップ（surface_data・surface の pickups の 1 つ。有限でない値は null）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickupView {
    /// 写す先の量（c r t k pN semi …）
    pub what: String,
    /// 写す元の面
    pub src: usize,
    #[serde(deserialize_with = "nan_for_null_1")]
    pub scale: f64,
    #[serde(deserialize_with = "nan_for_null_1")]
    pub offset: f64,
    /// 写す元の量（None は what と同じ）
    pub from: Option<String>,
}

/// surface の結果（面の番号を省いたとき）: 全部の面・玉・群・機構部品
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SurfacesView {
    pub surfaces: Vec<SurfaceView>,
    pub units: Vec<UnitView>,
    /// 機構部品（系ファイルの mechanics と同じ形）
    pub mechanics: Value,
}

/// inf・NaN を `{"$f": "inf"}` で読み書きする実数
mod fnum {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    pub(super) enum F {
        Num(f64),
        Tag {
            #[serde(rename = "$f")]
            f: String,
        },
    }

    pub(super) fn to(x: f64) -> F {
        if x.is_finite() {
            F::Num(x)
        } else {
            let f = if x.is_nan() {
                "nan"
            } else if x > 0.0 {
                "inf"
            } else {
                "-inf"
            };
            F::Tag { f: f.into() }
        }
    }

    pub(super) fn from<E: serde::de::Error>(v: F) -> Result<f64, E> {
        match v {
            F::Num(x) => Ok(x),
            F::Tag { f } => match f.as_str() {
                "inf" => Ok(f64::INFINITY),
                "-inf" => Ok(f64::NEG_INFINITY),
                "nan" => Ok(f64::NAN),
                _ => Err(E::custom(format!("unknown number {f}"))),
            },
        }
    }

    pub(super) fn serialize<S: Serializer>(x: &f64, s: S) -> Result<S::Ok, S::Error> {
        to(*x).serialize(s)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        from(F::deserialize(d)?)
    }
}

/// [`fnum`] の並び
mod fnums {
    use super::fnum::{from, to, F};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(x: &[f64], s: S) -> Result<S::Ok, S::Error> {
        x.iter().map(|&v| to(v)).collect::<Vec<_>>().serialize(s)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f64>, D::Error> {
        Vec::<F>::deserialize(d)?.into_iter().map(from).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn config_table_round_trips_nan() {
        let t = ConfigTable {
            n: 2,
            active: 1,
            rows: vec![ConfigRow {
                param: "3.t".into(),
                values: vec![1.0, f64::NAN],
                names: Vec::new(),
                picks: vec![ConfigPick {
                    k: 2,
                    from: 1,
                    scale: 1.0,
                    offset: 0.5,
                }],
            }],
            zmx_cfgs: 1,
            zmx_cfg: 1,
        };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["rows"][0]["values"], json!([1.0, null]));
        let back: ConfigTable = read(&v).unwrap();
        assert!(back.rows[0].values[1].is_nan());
        assert_eq!(back.rows[0].picks, t.rows[0].picks);
    }

    #[test]
    fn unit_round_trips_nan() {
        let u = UnitView {
            name: "G".into(),
            spans: vec![[1, 2]],
            motion: [0.0, f64::NAN, 0.0, 0.0, 0.0],
            shift: f64::INFINITY,
            ..Default::default()
        };
        let v = serde_json::to_value(&u).unwrap();
        assert_eq!(v["shift"], json!(null));
        let back: UnitView = read(&v).unwrap();
        assert!(back.motion[1].is_nan() && back.shift.is_nan());
    }

    /// 公開の系の全部の面で、面の値の JSON（edit_ops）を型に読んで書き戻すと同じ JSON になる（鍵の抜け・増え・inf の形）
    #[test]
    fn surface_view_round_trips_every_reference_surface() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let opt = lensbench_formats::zmx::LoadOptions {
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let mut n = 0;
        for e in std::fs::read_dir(root.join("tests/refs")).unwrap() {
            let f = e.unwrap().path().join("system.zmx");
            let Ok(s) = lensbench_formats::zmx::load_with(&f, &opt) else {
                continue;
            };
            for j in 0..s.surfaces.len() {
                let mut v = lensbench_project::edit_ops::surf_fields_json(&s, j).unwrap();
                let sp = lensbench_project::edit_ops::surface_solves(&s, j);
                v.as_object_mut()
                    .unwrap()
                    .extend(sp.as_object().unwrap().clone());
                let t: SurfaceView = read(&v).unwrap_or_else(|e| panic!("{f:?} {j}: {e}"));
                assert_eq!(serde_json::to_value(&t).unwrap(), v, "{f:?} {j}");
                n += 1;
            }
        }
        assert!(n > 100, "{n}");
        assert!(read::<SurfaceView>(&json!({ "type": "STANDARD" })).is_err());
    }

    /// 有限でない値（JSON の null）があっても読める（ソルブ・ピックアップの値、公差の公称の評価関数）
    #[test]
    fn null_numbers_read_as_nan() {
        let q: SolveView = read(&json!({
            "what": "t", "kind": "marginal_height", "value": null, "param": 0.0, "other": null
        }))
        .unwrap();
        assert!(q.value.is_nan());
        let p: PickupView =
            read(&json!({ "what": "c", "src": 1, "scale": null, "offset": 0.0, "from": null }))
                .unwrap();
        assert!(p.scale.is_nan());
        let e: lensbench_design::montecarlo::Eval =
            read(&json!({ "merit": null, "terms": [null], "comp": [] })).unwrap();
        assert!(e.merit.is_nan() && e.terms[0].is_nan());
    }

    #[test]
    fn history_needs_its_keys() {
        assert!(read::<HistoryView>(&json!({ "surfaces": [] })).is_err());
        let h: HistoryView = read(&json!({
            "undo": 1, "redo": 0, "undo_list": [{ "set": ["3.t"] }], "redo_list": [], "loaded": {}
        }))
        .unwrap();
        assert_eq!((h.undo, h.undo_list.len()), (1, 1));
    }
}
