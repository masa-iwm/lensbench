//! serve の結果のうち GUI が型で受けるもの（GAR-04 の一歩目）。serve はこの型を JSON にし、GUI は同じ型で読む
//! （鍵の綴りをコンパイラが見る。JSON の鍵の並びはフィールドの並び）
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// JSON の値を型に読む（足りない鍵・型の違いは誤り）
pub fn read<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, String> {
    T::deserialize(v).map_err(|e| e.to_string())
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
    /// 剛体の動き dx dy tx ty tz [mm, deg]
    pub motion: [f64; 5],
    /// 群の移動 [mm]
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
    pub picks: Vec<ConfigPick>,
}

/// 構成のピックアップ（core の型をそのまま）
pub use lensbench_core::edit::ConfigPick;

/// serde_json は NaN を null に書くので、読むときは null を NaN に戻す
fn nan_for_null<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f64>, D::Error> {
    let v = Vec::<Option<f64>>::deserialize(d)?;
    Ok(v.into_iter().map(|x| x.unwrap_or(f64::NAN)).collect())
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
    fn history_needs_its_keys() {
        assert!(read::<HistoryView>(&json!({ "surfaces": [] })).is_err());
        let h: HistoryView = read(&json!({
            "undo": 1, "redo": 0, "undo_list": [{ "set": ["3.t"] }], "redo_list": [], "loaded": {}
        }))
        .unwrap();
        assert_eq!((h.undo, h.undo_list.len()), (1, 1));
    }
}
