//! 膜ファイル（COATING.DAT の形式）

use super::super::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub mat: String,
    pub d: f64,
    /// true: d は絶対値 [µm]、false: 基準波長での光学的な波長単位
    pub absolute: bool,
}

/// 膜の表: 角度 [deg] の昇順に、波長ごとの (波長, Rs, Rp, Ts, Tp, 位相 rs rp ts tp [deg])
pub type Table = Vec<(f64, Vec<[f64; 9]>)>;

/// 膜ファイル（COATING.DAT 形式）の内容
#[derive(Debug, Clone, Default)]
pub struct CoatingFile {
    /// 材料: 波長 [µm]、n、k
    pub mate: HashMap<String, Vec<[f64; 3]>>,
    pub coat: HashMap<String, Vec<Layer>>,
    pub table: HashMap<String, Table>,
}

pub(in crate::coating) fn num(s: &str, name: &str) -> Result<f64> {
    s.parse::<f64>()
        .map_err(|_| Error::Invalid(format!("coating file: {name} number {s}")))
}

impl CoatingFile {
    pub fn load(path: &Path) -> Result<CoatingFile> {
        let txt =
            glass::read_text(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        Self::parse(&txt)
    }

    pub fn parse(txt: &str) -> Result<CoatingFile> {
        enum Cur {
            None,
            Mate(String),
            Coat(String),
            Table(String, Option<f64>),
        }
        let mut f = CoatingFile::default();
        let mut tables: HashMap<String, Table> = HashMap::new();
        let mut cur = Cur::None;
        for line in txt.lines() {
            let t: Vec<&str> = line.split_whitespace().collect();
            if t.is_empty() || t[0].starts_with('!') {
                continue;
            }
            match t[0] {
                // 理想膜・テーパ・暗号化の宣言は使わない（I.xx は組込み、暗号化は実測表）
                "IDEAL" | "IDEAL2" | "ENCRYPTED" | "TAPR" => cur = Cur::None,
                "MATE" | "COAT" | "TABLE" if t.len() >= 2 => {
                    let name = t[1].to_string();
                    cur = match t[0] {
                        "MATE" => {
                            f.mate.insert(name.clone(), Vec::new());
                            Cur::Mate(name)
                        }
                        "COAT" => {
                            f.coat.insert(name.clone(), Vec::new());
                            Cur::Coat(name)
                        }
                        _ => {
                            tables.insert(name.clone(), Vec::new());
                            Cur::Table(name, None)
                        }
                    };
                }
                _ => {
                    match &mut cur {
                        Cur::None => {}
                        Cur::Mate(name) => {
                            if t.len() < 2 {
                                invalid!("coating file: material {name} line {line}");
                            }
                            let k = if t.len() > 2 { num(t[2], name)? } else { 0.0 };
                            let row = [num(t[0], name)?, num(t[1], name)?, k];
                            f.mate.get_mut(name.as_str()).unwrap().push(row);
                        }
                        Cur::Coat(name) => {
                            if t.len() < 2 {
                                invalid!("coating file: coating {name} line {line}");
                            }
                            let flag = if t.len() > 2 {
                                num(t[2], name)? as i64
                            } else {
                                0
                            };
                            let l = Layer {
                                mat: t[0].to_string(),
                                d: num(t[1], name)?,
                                absolute: flag == 1,
                            };
                            f.coat.get_mut(name.as_str()).unwrap().push(l);
                        }
                        Cur::Table(name, ang) => {
                            match t[0] {
                                "ANGL" if t.len() >= 2 => {
                                    let a = num(t[1], name)?;
                                    let v = tables.get_mut(name.as_str()).unwrap();
                                    v.retain(|(b, _)| *b != a);
                                    v.push((a, Vec::new()));
                                    *ang = Some(a);
                                }
                                "WAVE" => {
                                    let Some(a) = *ang else {
                                        invalid!("coating file: WAVE before the first ANGL of table {name}");
                                    };
                                    // 波長 Rs Rp Ts Tp に位相 4 つが続く（位相は省略すると 0）
                                    if t.len() < 6 {
                                        invalid!("coating file: table {name} line {line}");
                                    }
                                    let mut row = [0.0; 9];
                                    for i in 0..(t.len() - 1).min(9) {
                                        row[i] = num(t[i + 1], name)?;
                                    }
                                    let v = tables.get_mut(name.as_str()).unwrap();
                                    v.iter_mut().find(|(b, _)| *b == a).unwrap().1.push(row);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        for (k, mut v) in tables {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            f.table.insert(k, v);
        }
        Ok(f)
    }
}
