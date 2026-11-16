//! 品の STEP を測る（以前は lensbench-project の `step_geom`・`catalog_fill` の一部、PRJ-05・XCT-11）。
//!
//! - [`read_step`]・[`measure`] は品の厚さ・径・枠・機構部品の寸法を測る
//! - [`step_shell_poses`]・[`shell_poses`] は立体の殻ごとの、STEP の座標から部品の座標への置き方
//!
//! 文字の読みは lensbench-step（[`StepFile`]。書き出しへの写しと同じ表、PRJ-06）。

mod geom;

pub use lensbench_step::StepFile;
use serde_json::Value;

/// STEP から測った大きさ
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct StepSize {
    /// 厚さ [mm]（円の軸の向きの長さ。円が無ければ箱の最も短い辺）
    pub thickness: f64,
    /// 径 [mm]（円の最大の直径。円が無ければ箱の残りの 2 辺 [幅, 高さ]）
    pub diameter: Value,
    /// 長さの単位の倍率（mm へ）
    pub unit_mm: f64,
    /// 測り方の説明
    pub note: String,
    /// 枠付きの品の枠（機構部品の寸法、z は枠の端から）。ガラスは `thickness`・`diameter`
    pub frame: Option<lensbench_project::MechSpec>,
    /// ガラスと枠が 1 つの立体で、ガラスの径は見えている径（本当の径は仕様表で補う。lensbench-catalog の `spec_glass_diameter`）
    pub merged: bool,
    /// ガラスの無い機構部品（チューブ・ケージ板・マウント）。厚さ・径は枠の長さ・外径で、寸法は `frame`
    pub mech: bool,
    /// 機構部品の付属の止め輪（SM1RR など）。素子に合わせて動かすので `frame` に含めない。z は枠の端から
    pub retaining_rings: Vec<RetainingRing>,
}

/// 機構部品の付属の止め輪 [mm]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct RetainingRing {
    pub z0: f64,
    pub z1: f64,
    pub inner_diameter: f64,
    pub outer_diameter: f64,
}

/// 殻の実体の番号と、STEP の座標から部品の座標 [mm] への変換（3 行 4 列 `[R | t]`）
pub type ShellPose = (u64, [[f64; 4]; 3]);

/// STEP の立体の殻（`CLOSED_SHELL` の実体の番号）ごとの、STEP の座標から部品の座標 [mm] への変換（3 行 4 列 `[R | t]`、
/// 組立の置き方ごとに 1 つ）。部品の座標は [`read_step`] で測った枠・機構部品の寸法の座標（z は枠の端から光軸の向き、
/// x は角の部品の辺に直交）。取った STEP の形を部品の簡易外形と同じ所に描くのに使う
pub fn step_shell_poses(text: &str) -> Result<Vec<ShellPose>, String> {
    shell_poses(&StepFile::parse(text)?)
}

/// 読んだ STEP の [`step_shell_poses`]（読み直さない）
pub fn shell_poses(f: &StepFile) -> Result<Vec<ShellPose>, String> {
    geom::shell_poses(f, f.unit_mm())
}

fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// STEP（AP203/AP214/AP242 の B-rep）の文字から、品の厚さ・径を測る。
///
/// 立体（`MANIFOLD_SOLID_BREP`）ごとに組立の置き方を当てて共通の座標に置き、最大の円の軸を光軸とする。
/// 光軸と同じ軸の穴（内壁の円柱の面）を持つ立体を枠、持たない立体をガラスとみなし、両方あれば
/// 厚さ・径はガラスの値で、枠は [`StepSize::frame`]（機構部品の寸法）に入れる。ガラスが無く穴のある立体だけなら機構部品（チューブ・
/// ケージ板・マウント）として全体を枠にし（[`StepSize::mech`]）、ねじ（SM05〜SM3 の内径）・ケージのロッドの間隔も読む。枠の分からない品は、
/// 径は軸に平行な円の最大の直径、厚さは頂点と円の中心の軸の向きの広がり。円が無ければ（四角い品）頂点の箱の
/// 最も短い辺を厚さ、残りを [幅, 高さ] とする
pub fn read_step(text: &str) -> Result<StepSize, String> {
    measure(&StepFile::parse(text)?)
}

/// 読んだ STEP の [`read_step`]（読み直さない）
pub fn measure(f: &StepFile) -> Result<StepSize, String> {
    let k = f.unit_mm();
    let m = geom::measure(f, k, round6)?;
    Ok(StepSize {
        thickness: m.thickness,
        diameter: m.diameter,
        unit_mm: k,
        note: m.note,
        frame: m.frame,
        merged: m.merged,
        mech: m.mech,
        retaining_rings: m.retaining_rings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 厚さ t・半径 r の円板の B-rep（上下の円と側面の円柱）
    fn disc_step(t: f64, r: f64, unit: &str) -> String {
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n\
#1=CARTESIAN_POINT('',(0.,0.,0.));\n#2=DIRECTION('',(0.,0.,1.));\n#3=DIRECTION('',(1.,0.,0.));\n\
#4=AXIS2_PLACEMENT_3D('',#1,#2,#3);\n#5=CIRCLE('',#4,{r});\n#6=CARTESIAN_POINT('',(0.,0.,{t}));\n\
#7=AXIS2_PLACEMENT_3D('',#6,#2,#3);\n#8=CIRCLE('',#7,{r});\n#9=CYLINDRICAL_SURFACE('',#4,{r});\n\
#10=CARTESIAN_POINT('',({r},0.,0.));\n#11=CARTESIAN_POINT('',({r},0.,{t}));\n\
#12=VERTEX_POINT('',#10);\n#13=VERTEX_POINT('',#11);\n#14=PLANE('',#7);\n\
#20=( LENGTH_UNIT() NAMED_UNIT(*) {unit} );\nENDSEC;\nEND-ISO-10303-21;\n"
        )
    }

    #[test]
    fn step_disc_size() {
        let s = read_step(&disc_step(3.0, 12.5, "SI_UNIT(.MILLI.,.METRE.)")).unwrap();
        assert_eq!(s.thickness, 3.0);
        assert_eq!(s.diameter, json!(25.0));
        assert_eq!(s.unit_mm, 1.0);
        // インチの STEP
        let s = read_step(&disc_step(0.25, 0.5, "CONVERSION_BASED_UNIT('INCH',#21)")).unwrap();
        assert_eq!(s.thickness, 6.35);
        assert_eq!(s.diameter, json!(25.4));
        assert!(read_step("not a step").is_err());
    }

    #[test]
    fn step_box_size() {
        let mut d = String::from("ISO-10303-21;\nDATA;\n");
        let mut n = 1;
        for x in [0.0, 25.0] {
            for y in [0.0, 35.0] {
                for z in [0.0, 1.1] {
                    d += &format!(
                        "#{n}=CARTESIAN_POINT('',({x},{y},{z}));\n#{}=VERTEX_POINT('',#{n});\n",
                        n + 1
                    );
                    n += 2;
                }
            }
        }
        d += "ENDSEC;\n";
        let s = read_step(&d).unwrap();
        assert_eq!(s.thickness, 1.1);
        assert_eq!(s.diameter, json!([25.0, 35.0]));
    }
}
