//! OpticStudio の順次モードの .zmx を読む。
//!
//! 順次の光線追跡に効くものだけを読み、対応していないもの（未知の面タイプや面の行、扱わない多重構成の
//! オペランドが構成で変わる、非順次、mm 以外の単位など）は `Error::NotImplemented` で止める。

use lensbench_core::error::{Error, Result};
use lensbench_core::glass::{self, Glass};
use lensbench_core::import::{MacroRequest, MacroTarget};
use lensbench_core::param::Kind;
use lensbench_core::pickup::{surf_parm, Pickup};
use lensbench_core::solves::{Solve, SolveKind};
use lensbench_core::surface::{After, Aperture, Form, SurfType, Surface, Tilt};
use lensbench_core::system::{
    Aiming, ApertureKind, Field, FieldNorm, FieldType, Prescription, System,
};
use lensbench_core::{invalid, notimpl};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

const HARMLESS: &[&str] = &[
    "HIDE", "MIRR", "SLAB", "POPS", "SSID", "FIMP", "MEMA", "VPAR", "VDSZ", "VCRV", "VCON", "OEMA",
    "BLNK", "NAME", // 縁の描き方（光線追跡に効かない）
    "EDGE", // 面の色（表示用）・面ごとの熱膨張係数（熱の解析だけに使う）
    "SCOL", "TCED",
];
const MCE_HANDLED: &[&str] = &[
    "THIC", "IGNR", "APMX", "SDIA", "STPS", "SATP", "APER", "TELE", "CRVT", "CONN", "WAVE", "GLSS",
    "XFIE", "YFIE", "MOFF", "LTTL", "PRAM",
    // 光線追跡に効かない: 構成の名前（MCOM）・最適化での構成の重み（CWGT）
    "MCOM", "CWGT",
];

#[derive(Debug, Clone, Default)]
pub struct RawSurf {
    pub n: i64,
    pub typ: Option<String>,
    pub coat: Option<String>,
    pub glas: Option<String>,
    /// モデルガラス（nd, vd, ΔPgF）
    pub model_glass: Option<(f64, f64, f64)>,
    /// 材料のピックアップの元の面（GLAS 名前 2 元の面 …。名前・モデルの値は使わない）
    pub glas_pickup: Option<usize>,
    pub curv: Option<f64>,
    /// 曲率のピックアップ（元の面, 倍率, 元の列。None は同じ列）
    pub curv_pickup: Option<(usize, f64, Option<Kind>)>,
    pub coni: Option<f64>,
    pub disz: Option<f64>,
    pub diam: Option<f64>,
    /// DIAM の 2 番目の欄（0 自動、1 固定、2 ピックアップ、4 マクロ）
    pub diam_mode: Option<u8>,
    /// 半直径を固定（DIAM の 2 番目が 1）
    pub diam_fixed: bool,
    pub comment: String,
    pub parm: BTreeMap<i64, f64>,
    /// ピックアップ（元の面, 倍率, オフセット, 元の列。None は同じ列）
    pub ppar: BTreeMap<i64, (usize, f64, f64, Option<Kind>)>,
    pub pzup: Option<(usize, f64, f64, Option<Kind>)>,
    /// 厚さのソルブ（種類, 値）。DISZ の次の行（MAZH・CHZH・EDGE・TOLE・TCOM・TCOC・TPUP）
    pub tsolve: Option<(SolveKind, f64)>,
    /// 曲率のソルブ（種類, 値）。CURV の 3 番目の欄（2・3・5〜11）
    pub csolve: Option<(SolveKind, f64)>,
    /// 曲率の ZPL マクロのソルブのマクロの名前（CURV c 12 … "名前"）
    pub cmacro: Option<String>,
    /// 厚さの ZPL マクロのソルブのマクロの名前（TZPL "名前"）
    pub tmacro: Option<String>,
    /// 円錐係数の ZPL マクロのソルブ（KZPL "名前"、OpticStudio 26.1.1 で実測）
    pub kmacro: Option<String>,
    /// 半直径の ZPL マクロのソルブ（DIAM 値 4 0 0 1 "名前"、OpticStudio 26.1.1 で実測）
    pub smacro: Option<String>,
    /// パラメータの ZPL マクロのソルブ（番号, 名前）。PZPL 番号 "名前"
    pub pmacros: Vec<(u8, String)>,
    /// 円錐係数のピックアップ（元の面, 倍率, オフセット（常に 0）, 元の列）
    pub pkup: Option<(usize, f64, f64, Option<Kind>)>,
    /// 半直径のピックアップ（元の面, 倍率, 元の列）
    pub diam_pickup: Option<(usize, f64, Option<Kind>)>,
    /// 拡張データ（番号 → 値）
    pub xdat: BTreeMap<i64, f64>,
    /// Grid Sag の格子の頭（GDAT nx ny dx dy xdec ydec unit …）と点（GARR 番号 z zx zy zxy nodata）、.DAT の名前（FIMP）
    pub gdat: Option<Vec<f64>>,
    pub garr: Vec<(usize, [f64; 5])>,
    pub fimp: Option<String>,
    pub clap: Option<Vec<f64>>,
    pub sqap: Option<Vec<f64>>,
    pub elap: Option<Vec<f64>>,
    pub flap: Option<Vec<f64>>,
    pub obsc: Option<Vec<f64>>,
    /// スパイダー（SPID 腕の幅 腕の本数 開口のピックアップの元の面）
    pub spid: Option<Vec<f64>>,
    pub obdc: Option<Vec<f64>>,
    pub scbd: Vec<Vec<String>>,
    pub stop: bool,
    pub ignore: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Raw {
    pub surfs: Vec<RawSurf>,
    /// (オペランド, 番号, 構成, 値, 残り)
    pub mce: Vec<(String, i64, i64, String, Vec<String>)>,
    pub sysl: HashMap<String, Vec<Vec<String>>>,
    /// 評価関数の行（面の後、TOL・MNUM の前の字下げしない行。[`merit::MeritRow`]）。系を作るときは使わない
    pub merit: Vec<merit::MeritRow>,
    /// 読まずに飛ばした行など、読込の注意（[`System::import_notes`] に入る）
    pub notes: Vec<String>,
}

mod configs;
mod macros;
pub mod merit;
mod parse;
mod pickups;
mod surfaces;
mod system_lines;
mod units;
mod values;

use configs::config_table;
pub use parse::parse_text;
use units::to_mm;

fn fv(v: &str) -> Result<f64> {
    if v.eq_ignore_ascii_case("INFINITY") {
        return Ok(f64::INFINITY);
    }
    v.parse::<f64>()
        .map_err(|_| Error::Invalid(format!("not a number: {v}")))
}

/// 構成・視野・波長の数の上限（壊れた .zmx の巨大な数でメモリを確保しないため）
const MAX_CONFIGS: usize = 1000;
const MAX_FIELDS: usize = 10_000;
const MAX_WAVES: usize = 1000;
/// Grid Sag の格子の点の数の上限
const MAX_GRID: usize = 16_000_000;

/// 数の欄 v を lo〜hi の整数として読む（負・小数・範囲外・NaN は誤り。`as usize` で巨大な数や負の数が
/// メモリ確保の暴走・panic になっていた）
fn count_in(v: f64, what: &str, lo: usize, hi: usize) -> Result<usize> {
    if !(v.fract() == 0.0 && v >= lo as f64 && v <= hi as f64) {
        invalid!("{what} {v} (an integer {lo} to {hi})");
    }
    Ok(v as usize)
}

fn sys_str<'a>(raw: &'a Raw, key: &str, i: usize) -> Option<&'a str> {
    raw.sysl.get(key)?.first()?.get(i).map(|s| s.as_str())
}
/// 系の行 key の i 番目の数（行・欄がなければ default、数として読めなければ誤り。以前は読み損ねを黙って既定値に
/// していた: `ENPD abc` が入射瞳径 0 になる）
fn sys_f(raw: &Raw, key: &str, i: usize, default: f64) -> Result<f64> {
    match sys_str(raw, key, i) {
        None => Ok(default),
        Some(v) => fv(v).map_err(|e| e.at(format!("zmx {key} value {}", i + 1))),
    }
}

impl Raw {
    /// マルチコンフィグの数（MNUM、なければ 1）
    pub fn configurations(&self) -> usize {
        // 範囲は parse_text で確かめている
        (sys_f(self, "MNUM", 0, 1.0).unwrap_or(1.0) as usize).clamp(1, MAX_CONFIGS)
    }

    /// 数の欄の範囲の確かめ（構成・視野・波長の数）
    fn check_counts(&self) -> Result<()> {
        count_in(
            sys_f(self, "MNUM", 0, 1.0)?,
            "number of zmx configurations MNUM",
            1,
            MAX_CONFIGS,
        )?;
        if let Some(v) = self.sysl.get("FTYP").and_then(|v| v.first()) {
            if v.is_empty() {
                invalid!("the zmx FTYP has no value");
            }
            for x in v {
                fv(x).map_err(|e| e.at("zmx FTYP"))?;
            }
            if let Some(x) = v.get(2) {
                count_in(fv(x)?, "zmx field count (FTYP 3)", 1, MAX_FIELDS)?;
            }
            if let Some(x) = v.get(3) {
                count_in(fv(x)?, "zmx wavelength count (FTYP 4)", 1, MAX_WAVES)?;
            }
        }
        Ok(())
    }

    /// ガラスカタログの名前（GCAT）
    pub fn catalogs(&self) -> Vec<String> {
        self.sysl
            .get("GCAT")
            .and_then(|v| v.first())
            .cloned()
            .unwrap_or_default()
    }

    /// Gradium の分布のファイルの名前（COFN の .GRD の項目、なければ PROFILE.GRD）
    pub fn grd_file_name(&self) -> String {
        self.sysl
            .get("COFN")
            .into_iter()
            .flatten()
            .flatten()
            .map(|v| v.trim_matches('"'))
            .find(|v| v.to_uppercase().ends_with(".GRD"))
            .unwrap_or("PROFILE.GRD")
            .to_string()
    }

    /// 膜ファイルの名前（COFN の最初の項目。OpticStudio の Coatings フォルダから探す名前）
    pub fn coating_file_name(&self) -> Option<String> {
        let rows = self.sysl.get("COFN")?;
        let pick = |a: &Vec<String>| {
            let a: Vec<&String> = a.iter().filter(|v| v.as_str() != "QF").collect();
            a.first().map(|v| v.trim_matches('"').to_string())
        };
        rows.iter()
            .find(|a| a.first().map(|v| v.as_str()) == Some("QF"))
            .and_then(pick)
            .or_else(|| rows.first().and_then(pick))
    }
}

/// 読込の追加指定。
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    pub cfg: usize,
    pub catalog_dirs: Vec<PathBuf>,
    /// 波長を置き換える [µm]
    pub wavelengths: Option<Vec<f64>>,
    /// ファイルの GCAT の代わりに使うガラス
    pub glasses: Option<HashMap<String, Glass>>,
    /// ZPL Macro Solve のマクロ（.ZPL）を探すフォルダ（.zmx のフォルダ・その隣の Macros・環境変数
    /// LENSBENCH_MACROS の前に探す。[`lensbench_core::zpl_solve::macro_dirs`]）
    pub macro_dirs: Vec<PathBuf>,
    /// カタログの玉として読む: 半直径をファイルの値で固定する（径は決まっている。自動の面も光線から
    /// 求めない）。系だけの設定（瞳のアポダイゼーション GFAC）は誤りにせず注意だけにする
    pub fixed_semi: bool,
}

pub fn load(path: &Path) -> Result<System> {
    load_with(
        path,
        &LoadOptions {
            cfg: 1,
            ..Default::default()
        },
    )
}

pub fn load_with(path: &Path, opt: &LoadOptions) -> Result<System> {
    load_text_with(&read_zmx(path)?, path, opt)
}

/// [`load_with`] で、ファイル path の中身 txt をもう読んであるとき（読み直さない。相対パスは path のフォルダから）
pub fn load_text_with(txt: &str, path: &Path, opt: &LoadOptions) -> Result<System> {
    build(&parse_text(txt)?, opt, &file_dirs(opt, path), path.parent())
}

fn read_zmx(path: &Path) -> Result<String> {
    glass::read_text(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))
}

/// ガラスのカタログを探すフォルダ（opt の後にファイルのフォルダ）
fn file_dirs(opt: &LoadOptions, path: &Path) -> Vec<PathBuf> {
    let mut dirs = opt.catalog_dirs.clone();
    dirs.extend(path.parent().map(Path::to_path_buf));
    glass::catalog_dirs(&dirs)
}

/// 多重構成の .zmx から構成 cfgs（1 始まり、空なら全部）の系をまとめて作る（ファイルは 1 回だけ読む）。
/// opt.cfg は使わない。結果は構成ごとに `load_with` で読んだものと同じ
pub fn load_configs(path: &Path, opt: &LoadOptions, cfgs: &[usize]) -> Result<Vec<System>> {
    load_configs_text(&read_zmx(path)?, path, opt, cfgs)
}

/// [`load_configs`] で、ファイル path の中身 txt をもう読んであるとき
pub fn load_configs_text(
    txt: &str,
    path: &Path,
    opt: &LoadOptions,
    cfgs: &[usize],
) -> Result<Vec<System>> {
    let raw = parse_text(txt)?;
    let dirs = file_dirs(opt, path);
    let all: Vec<usize>;
    let cfgs = if cfgs.is_empty() {
        all = (1..=raw.configurations()).collect();
        &all[..]
    } else {
        cfgs
    };
    cfgs.iter()
        .map(|&c| {
            if c == 0 {
                invalid!("configurations start at 1 (0)");
            }
            let o = LoadOptions {
                cfg: c,
                ..opt.clone()
            };
            build(&raw, &o, &dirs, path.parent())
        })
        .collect()
}

/// 構成の多重構成の値（オペランドと 1 番目の番号 → 値）
type Ops = HashMap<(String, i64), String>;

/// 多重構成の値 op p1 の数（無い・数でなければ default）
fn fop(ops: &Ops, op: &str, p1: i64, default: f64) -> f64 {
    ops.get(&(op.to_string(), p1))
        .and_then(|v| fv(v).ok())
        .unwrap_or(default)
}

/// [`Raw`] から系を作る（構成は opt.cfg）。dirs はガラス・GRIN の材料・膜のファイルを探すフォルダ、base は .zmx のフォルダ
pub fn build(
    raw: &Raw,
    opt: &LoadOptions,
    dirs: &[PathBuf],
    base: Option<&Path>,
) -> Result<System> {
    let (rx, macros) = document(raw, opt, dirs, base)?;
    System::from_import(rx, macros)
}

/// [`Raw`] から系の文書と、マクロのソルブの依頼を作る（ピックアップ・ソルブは書いたまま。解くのは
/// [`System::from_import`]）。core の公開の口だけを使う
pub fn document(
    raw: &Raw,
    opt: &LoadOptions,
    dirs: &[PathBuf],
    base: Option<&Path>,
) -> Result<(Prescription, Vec<MacroRequest>)> {
    let cfg = opt.cfg.max(1) as i64;
    let mode = sys_str(raw, "MODE", 0).unwrap_or("SEQ");
    if mode != "SEQ" {
        notimpl!("mode {mode} (sequential only)");
    }
    let unit = sys_str(raw, "UNIT", 0).unwrap_or("MM");
    let scaled;
    let raw = if unit.eq_ignore_ascii_case("MM") {
        raw
    } else {
        scaled = to_mm(raw, unit)?;
        &scaled
    };
    if raw.sysl.contains_key("AFOC") && sys_f(raw, "AFOC", 0, 0.0)? != 0.0 {
        notimpl!("afocal image space");
    }
    let ncfg = count_in(
        sys_f(raw, "MNUM", 0, 1.0)?,
        "number of zmx configurations MNUM",
        1,
        MAX_CONFIGS,
    )? as i64;
    if !(1..=ncfg).contains(&cfg) {
        invalid!("configuration {cfg} ({ncfg} in all)");
    }
    let ops = values::config_ops(raw, cfg)?;
    let temp = sys_f(raw, "ENVD", 0, 20.0)?;
    let pres = sys_f(raw, "ENVD", 1, 1.0)?;
    let adjust = sys_f(raw, "ENVD", 2, 0.0)? != 0.0;
    let waves = values::wavelengths(raw, &ops, opt)?;
    let glasses = values::glasses(raw, opt, dirs);
    let th = pickups::thicknesses(raw, &ops)?;
    let parm = pickups::parms(raw, cfg)?;
    let mats = surfaces::materials(raw, &ops)?;
    let stop_default = raw.surfs.iter().position(|s| s.stop).unwrap_or(1) as f64;
    let stop = fop(&ops, "STPS", 0, stop_default) as usize;
    // GRIN の材料（PROFILE.GRD・SGRIN.DAT・GRADIENT_9.DAT）。GRIN の面があるときだけ読む
    let grin_data = if raw
        .surfs
        .iter()
        .any(|r| matches!(r.typ.as_deref(), Some("GRINSUR8") | Some("GRINSUR9")))
    {
        Some(lensbench_core::grin::GrinData::load(
            dirs,
            &raw.grd_file_name(),
        )?)
    } else {
        None
    };
    let (mut sf, after) = surfaces::Ctx {
        raw,
        opt,
        ops: &ops,
        th: &th,
        parm: &parm,
        mats: &mats,
        stop,
        grin: grin_data.as_ref(),
        wl: &waves.wl,
        glasses: &glasses,
        env: (temp, pres, adjust),
    }
    .surfaces()?;
    pickups::after_tilts(&mut sf, &after);
    pickups::conics(raw, &mut sf)?;
    let pickups = pickups::links(raw, &sf, &after);
    let aperture = values::aperture_kind(raw, &ops, &mut sf)?;
    let fields = values::fields(raw, &ops, &waves.ftyp)?;
    let set = values::settings(raw, dirs, base)?;
    let s = System::new(
        sf,
        waves.wl,
        waves.wwt,
        waves.primary,
        fields.list,
        fields.typ,
        fields.norm,
        aperture,
        fields.tele,
        set.aiming,
        glasses.map,
    )?;
    // 多重構成の表（表せなければ構成 cfg だけを読んだと知らせる）
    let configs = (ncfg > 1).then(|| config_table(raw, &s, ncfg as usize, cfg as usize));
    let mut rx = s.into_prescription();
    rx.temperature = temp;
    rx.pressure = pres;
    rx.adjust_index = adjust;
    rx.glrs = set.glrs;
    rx.ropd = set.ropd;
    rx.apodization = system_lines::apodization(raw, opt.fixed_semi)?;
    // Afocal Image Space は FTYP の 7 番目（OpticStudio 26.1 で確認。保存すると HYPR も 1 になるが、読むときは FTYP だけが効く）
    rx.afocal = waves.ftyp.get(6) == Some(&1);
    // UNIT の値の 6 番目（MM X W X CM の次）が Afocal Mode Units（UR・MR・RR・AS・AM・DG、OpticStudio 26.1 で確認）。無ければ mrad
    // （知らない記号はアフォーカルのときだけ誤り）
    if let Some(u) = sys_str(raw, "UNIT", 5) {
        match lensbench_core::system::AfocalUnit::from_code(u) {
            Some(a) => rx.afocal_unit = a,
            None if rx.afocal => invalid!("UNIT Afocal Mode Units {u}"),
            None => {}
        }
    }
    rx.coating_file = set.coating_file;
    rx.name = set.name;
    rx.import_notes = raw.notes.clone();
    rx.import_notes.extend(raw.system_notes(opt.fixed_semi)?);
    if opt.glasses.is_none() {
        let n = glass::builtin::note_for(&rx, &glasses.cats, dirs); // 同梱のガラスの知らせ
        rx.import_notes.extend(n);
    }
    match configs {
        Some(Ok(c)) => rx.configs = Some(c),
        Some(Err(why)) => rx.import_notes.push(format!(
            "only configuration {cfg} (of {ncfg}) of the multi-configuration was read ({why} can't be expressed in the multi-configuration table)"
        )),
        None => {}
    }
    rx.pickups = pickups;
    rx.solves = macros::solves(raw);
    Ok((rx, macros::requests(raw, opt, base)))
}
