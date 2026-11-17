//! STEP の測りで当てる規格の表（光学部品のねじ・ケージのロッドの間隔）。
//!
//! 同梱の表は `standards.json`。環境変数 `LENSBENCH_STEP_STANDARDS` に同じ形の JSON のファイルを与えると、
//! そのねじ（同じ名前は置き換え）とロッドの間隔を足す。読めなければ測りは誤りになる。
use serde::Deserialize;
use std::sync::OnceLock;

/// 光学部品のねじ（名前、ねじの外径（メスの谷の径）[mm]、ピッチ [mm]、入る素子の径 [mm]（0 は素子を入れない
/// ねじ）、メスを外径の円柱でも判定するか（C マウントの外径 25.4 mm は 1 インチの素子の穴と区別できないので、
/// 端のねじで、奥にねじの逃げの溝（外径より太い短い穴）のあるときだけ））
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Thread {
    pub(super) name: String,
    #[serde(rename = "major_mm")]
    pub(super) major: f64,
    #[serde(rename = "pitch_mm")]
    pub(super) pitch: f64,
    #[serde(rename = "holds_mm", default)]
    pub(super) holds: f64,
    #[serde(default = "yes")]
    pub(super) female_major: bool,
}

fn yes() -> bool {
    true
}

/// 規格の表
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Standards {
    /// ケージのロッドの間隔 [mm]（16・30・60 は Thorlabs、26.87 は Edmund の 25・30 mm のケージ（ロッドの円の
    /// 直径 38 mm））
    #[serde(default)]
    pub(super) rod_spacings_mm: Vec<f64>,
    #[serde(default)]
    pub(super) threads: Vec<Thread>,
}

impl Standards {
    fn parse(txt: &str, what: &str) -> Result<Standards, String> {
        let s: Standards = serde_json::from_str(txt).map_err(|e| format!("{what}: {e}"))?;
        for t in &s.threads {
            if !(t.major > 0.0 && t.pitch > 0.0 && t.holds >= 0.0) {
                return Err(format!(
                    "{what}: thread {} needs major_mm and pitch_mm over 0 and holds_mm of 0 or more",
                    t.name
                ));
            }
        }
        if s.rod_spacings_mm.iter().any(|&x| x.is_nan() || x <= 0.0) {
            return Err(format!("{what}: rod_spacings_mm must be over 0"));
        }
        Ok(s)
    }

    /// 足す表を重ねる（同じ名前のねじは置き換え）
    fn extend(&mut self, o: Standards) {
        for t in o.threads {
            match self.threads.iter_mut().find(|x| x.name == t.name) {
                Some(x) => *x = t,
                None => self.threads.push(t),
            }
        }
        for r in o.rod_spacings_mm {
            if !self.rod_spacings_mm.iter().any(|&x| (x - r).abs() < 1e-9) {
                self.rod_spacings_mm.push(r);
            }
        }
    }
}

/// 同梱の表に、`LENSBENCH_STEP_STANDARDS` のファイルを重ねた表（プロセスで 1 回だけ読む）
pub(super) fn standards() -> lensbench_core::Result<&'static Standards> {
    static S: OnceLock<Result<Standards, String>> = OnceLock::new();
    S.get_or_init(|| {
        let mut s = Standards::parse(include_str!("standards.json"), "standards.json")
            .map_err(|e| format!("the built-in STEP standards: {e}"))?;
        if let Some(p) = std::env::var_os("LENSBENCH_STEP_STANDARDS").filter(|p| !p.is_empty()) {
            let p = std::path::PathBuf::from(p);
            let txt = lensbench_core::vfs::read_to_string(&p)
                .map_err(|e| format!("LENSBENCH_STEP_STANDARDS {}: {e}", p.display()))?;
            s.extend(Standards::parse(
                &txt,
                &format!("LENSBENCH_STEP_STANDARDS {}", p.display()),
            )?);
        }
        Ok(s)
    })
    .as_ref()
    .map_err(|e| lensbench_core::Error::Invalid(e.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同梱の表は前の定数と同じ値。足す表は同じ名前を置き換え、新しいねじ・間隔を足す
    #[test]
    fn built_in_table_and_extension() {
        let mut s = Standards::parse(include_str!("standards.json"), "t").unwrap();
        let sm1 = s.threads.iter().find(|t| t.name == "SM1").unwrap();
        assert!(
            (sm1.major - 1.035 * 25.4).abs() < 1e-12 && (sm1.pitch - 25.4 / 40.0).abs() < 1e-12
        );
        let c = s.threads.iter().find(|t| t.name == "C-Mount").unwrap();
        assert!((c.pitch - 25.4 / 32.0).abs() < 1e-12 && !c.female_major && c.holds == 0.0);
        assert_eq!(s.rod_spacings_mm, [16.0, 26.87, 30.0, 60.0]);
        let add = Standards::parse(
            r#"{"threads":[{"name":"SM1","major_mm":26.3,"pitch_mm":0.635,"holds_mm":25.4},
                {"name":"M27","major_mm":27,"pitch_mm":0.75}],"rod_spacings_mm":[30,40]}"#,
            "t",
        )
        .unwrap();
        s.extend(add);
        assert_eq!(s.threads.len(), 6);
        assert_eq!(
            s.threads.iter().find(|t| t.name == "SM1").unwrap().major,
            26.3
        );
        assert_eq!(s.rod_spacings_mm, [16.0, 26.87, 30.0, 60.0, 40.0]);
        assert!(Standards::parse(
            r#"{"threads":[{"name":"x","major_mm":0,"pitch_mm":1}]}"#,
            "t"
        )
        .is_err());
        assert!(Standards::parse(r#"{"thread":[]}"#, "t").is_err());
    }
}
