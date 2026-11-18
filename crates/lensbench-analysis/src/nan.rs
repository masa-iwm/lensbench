//! 結果の型を JSON から読み直すときの数: JSON に書けない値（NaN・無限大）は null になるので、null は NaN として読む

use serde::{Deserialize, Deserializer};

fn num(x: Option<f64>) -> f64 {
    x.unwrap_or(f64::NAN)
}

fn nums(v: Vec<Option<f64>>) -> Vec<f64> {
    v.into_iter().map(num).collect()
}

/// 数（null は NaN）
pub(crate) fn f<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(num(Option::deserialize(d)?))
}

/// 数の並び
pub(crate) fn v<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f64>, D::Error> {
    Ok(nums(Vec::deserialize(d)?))
}

/// 数 2 つ
pub(crate) fn a2<'de, D: Deserializer<'de>>(d: D) -> Result<[f64; 2], D::Error> {
    let [a, b] = <[Option<f64>; 2]>::deserialize(d)?;
    Ok([num(a), num(b)])
}

/// 数 3 つ
pub(crate) fn a3<'de, D: Deserializer<'de>>(d: D) -> Result<[f64; 3], D::Error> {
    let [a, b, c] = <[Option<f64>; 3]>::deserialize(d)?;
    Ok([num(a), num(b), num(c)])
}

/// 省ける数の並び
pub(crate) fn ov<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<f64>>, D::Error> {
    Ok(Option::<Vec<Option<f64>>>::deserialize(d)?.map(nums))
}

/// 省ける数の並びの並び
pub(crate) fn ovv<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<Vec<f64>>>, D::Error> {
    Ok(Option::<Vec<Vec<Option<f64>>>>::deserialize(d)?.map(|v| v.into_iter().map(nums).collect()))
}

#[test]
fn nulls_read_as_nan() {
    #[derive(Deserialize)]
    struct T {
        #[serde(deserialize_with = "f")]
        x: f64,
        #[serde(deserialize_with = "v")]
        y: Vec<f64>,
    }
    let t: T = serde_json::from_str(r#"{"x": null, "y": [1, null]}"#).unwrap();
    assert!(t.x.is_nan() && t.y[0] == 1.0 && t.y[1].is_nan());
}
