//! 画面の通し確認（`--tour DIR`）: GUI を自分で操作し、各画面を PNG に撮って閉じる。
//!
//! CI（Linux の仮想ディスプレイ）と手元で、起動 → 系を開く → 解析 → カタログ → 組合せ → 系を作る →
//! 面の表 → スポット → 最適化・差し替え探索・公差 → 値を動かす → ガラス → 別の窓 → パレット → 英語・明るい配色 → 系の設定 → 干渉計（BS の分岐）→ 簡易 POP → 状態 → 膜の定義 → ズーム軌跡 → 玉・群・機構部品、を通して画面を残す。途中でエラーが出れば `report.json` の errors に書き、終了コード 1 で終える。
//!
//! ```text
//! lensbench-gui --tour shots --glass-dir tests/glass --lens-catalog examples/lenses/LB examples/projects/P8_catalog_two.json
//! ```

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use web_time::Instant;

/// 通し確認が失敗したか（main が終了コードにする）
pub static FAILED: AtomicBool = AtomicBool::new(false);

/// 全体の時間の上限の既定（`--tour-timeout` で変えられる。ソフト描画では足りないことがある、公開 #236）
const TIMEOUT: Duration = Duration::from_secs(240);
/// 計算が終わってから撮るまでに待つフレーム数（配置が落ち着くまで）
const SETTLE: u32 = 6;

/// コマンドライン
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Args {
    pub open: Option<PathBuf>,
    pub tour: Option<PathBuf>,
    pub glass_dirs: Vec<String>,
    pub lens_dirs: Vec<String>,
    /// ファイルメニューの「見本」に出す系のファイル（Web 版の同梱の系）
    pub samples: Vec<String>,
    /// 設定のファイル（省略で exe の隣のポータブルか OS の場所）
    pub settings: Option<PathBuf>,
    /// 通し確認の時間の上限（省略で 240 秒）
    pub tour_timeout: Option<Duration>,
    /// 起動時に出す知らせ（設定の取り込みなど）
    pub notice: Option<crate::portable::Notice>,
}

impl Args {
    /// `[--tour DIR] [--tour-timeout 秒] [--glass-dir D]... [--lens-catalog D]... [--settings FILE] [系のファイル]`
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
        let mut a = Args::default();
        let mut it = args.into_iter();
        while let Some(x) = it.next() {
            let mut val = |k: &str| it.next().ok_or(format!("{k} の後に値がない"));
            match x.as_str() {
                "--tour" => a.tour = Some(PathBuf::from(val("--tour")?)),
                "--glass-dir" => a.glass_dirs.push(val("--glass-dir")?),
                "--lens-catalog" => a.lens_dirs.push(val("--lens-catalog")?),
                "--settings" => a.settings = Some(PathBuf::from(val("--settings")?)),
                "--tour-timeout" => {
                    let v = val("--tour-timeout")?;
                    let sec: u64 = v
                        .parse()
                        .map_err(|_| format!("--tour-timeout の値 {v} は秒の整数でない"))?;
                    a.tour_timeout = Some(Duration::from_secs(sec));
                }
                s if s.starts_with("--") => return Err(format!("{s} は知らない引数")),
                _ => a.open = Some(PathBuf::from(x)),
            }
        }
        Ok(a)
    }
}

/// 通し確認の設定: 日本語、確かめる窓で止まらない（答えは「いいえ」）、引数でフォルダを渡さなければ通常の起動と
/// 同じく exe の隣の catalogs/ を使う（配る版の確認、公開 #236）
pub fn prepare(args: &Args, s: &mut crate::app::Settings) {
    s.lang = crate::i18n::Lang::Ja;
    crate::files::set_unattended(true);
    if args.glass_dirs.is_empty() && args.lens_dirs.is_empty() {
        if let Some(d) = crate::portable::exe_dir() {
            (s.glass_dirs, s.lens_dirs) = crate::portable::default_catalogs(&d);
        }
    }
}

/// ズームの段の例の系（P17）。例のフォルダの無い配る版でも回るように、exe に埋めた系を出力先に書く（公開 #236）
pub fn zoom_sample(dir: &Path) -> PathBuf {
    let p = dir.join("P17_zoom_three_groups.json");
    let _ = lensbench_core::vfs::write(
        &p,
        include_str!("../../../examples/projects/P17_zoom_three_groups.json"),
    );
    p
}

/// 1 段: 操作して、計算が終わったら撮る
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Open,
    Analysis,
    Pop,
    States,
    Coatings,
    Zoom,
    Units,
    Catalog,
    Combo,
    Build,
    Layout,
    Layout3d,
    LensData,
    LensDataTree,
    ComboSpot,
    Optimize,
    OptApply,
    Swap,
    SwapInsert,
    Tolerance,
    Param,
    Glasses,
    Window,
    Palette,
    English,
    Settings,
    Interferometer,
    Done,
}

impl Step {
    const ORDER: [Step; 28] = [
        Step::Open,
        Step::Analysis,
        Step::Catalog,
        Step::Combo,
        Step::Build,
        Step::Layout,
        Step::Layout3d,
        Step::LensData,
        Step::LensDataTree,
        Step::ComboSpot,
        Step::Optimize,
        Step::OptApply,
        Step::Swap,
        Step::SwapInsert,
        Step::Tolerance,
        Step::Param,
        Step::Glasses,
        Step::Window,
        Step::Palette,
        Step::English,
        Step::Settings,
        Step::Interferometer,
        Step::Pop,
        Step::States,
        Step::Coatings,
        Step::Zoom,
        Step::Units,
        Step::Done,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Step::Open => "01_open",
            Step::Analysis => "02_analysis",
            Step::Catalog => "03_catalog",
            Step::Combo => "04_combo",
            Step::Build => "05_build",
            Step::Layout => "06_layout",
            Step::Layout3d => "06a_layout_3d",
            Step::LensData => "06b_lens_data",
            Step::LensDataTree => "06c_lens_data_tree",
            Step::ComboSpot => "07_combo_spot",
            Step::Optimize => "07b_optimize",
            Step::OptApply => "07c_optimize_apply",
            Step::Swap => "07d_swap",
            Step::SwapInsert => "07e_swap_insert",
            Step::Tolerance => "07f_tolerance",
            Step::Param => "08_param_live",
            Step::Glasses => "09_glasses",
            Step::Window => "10_window",
            Step::Palette => "11_palette",
            Step::English => "12_english_light",
            Step::Settings => "13_settings",
            Step::Interferometer => "14_interferometer",
            Step::Pop => "15_pop",
            Step::States => "16_states",
            Step::Coatings => "17_coatings",
            Step::Zoom => "18_zoom",
            Step::Units => "19_units",
            Step::Done => "done",
        }
    }
    fn next(self) -> Step {
        let i = Self::ORDER.iter().position(|&s| s == self).unwrap_or(0);
        Self::ORDER.get(i + 1).copied().unwrap_or(Step::Done)
    }
}

pub struct Tour {
    pub dir: PathBuf,
    pub step: Step,
    /// この段の操作をしたか
    pub acted: bool,
    /// 撮影を頼んだか
    shooting: bool,
    idle: u32,
    start: Instant,
    timeout: Duration,
    pub errors: Vec<String>,
    pub shots: Vec<String>,
}

impl Tour {
    pub fn new(dir: PathBuf, timeout: Option<Duration>) -> Tour {
        Tour {
            dir,
            timeout: timeout.unwrap_or(TIMEOUT),
            step: Step::Open,
            acted: false,
            shooting: false,
            idle: 0,
            start: Instant::now(),
            errors: Vec::new(),
            shots: Vec::new(),
        }
    }

    /// 計算中かどうかを渡し、撮る時か（操作の後、計算が終わって落ち着いた）を返す
    pub fn ready(&mut self, busy: bool) -> bool {
        if busy {
            self.idle = 0;
            return false;
        }
        self.idle += 1;
        self.idle >= SETTLE
    }

    pub fn timed_out(&self) -> bool {
        self.start.elapsed() > self.timeout
    }

    /// 撮影を頼む（一度だけ）
    pub fn request_shot(&mut self, ctx: &egui::Context) {
        if !self.shooting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.shooting = true;
        }
    }

    /// 届いた画像を保存して次の段へ。まだ届いていなければ false
    pub fn take_shot(&mut self, ctx: &egui::Context) -> bool {
        let img = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(img) = img else {
            return false;
        };
        let name = format!("{}.png", self.step.name());
        if let Err(e) = save_png(&self.dir.join(&name), &img) {
            self.errors.push(format!("{name}: {e}"));
        } else {
            self.shots.push(name);
        }
        self.shooting = false;
        self.acted = false;
        self.idle = 0;
        self.step = self.step.next();
        true
    }

    /// report.json を書き、失敗なら FAILED を立てる
    pub fn finish(&self) {
        let ok = self.errors.is_empty() && self.step == Step::Done;
        let v = serde_json::json!({
            "ok": ok,
            "reached": self.step.name(),
            "shots": self.shots,
            "errors": self.errors,
            "elapsed_s": self.start.elapsed().as_secs_f64(),
        });
        let _ = lensbench_core::vfs::write(
            self.dir.join("report.json"),
            serde_json::to_string_pretty(&v).unwrap_or_default(),
        );
        println!("{v}");
        if !ok {
            FAILED.store(true, Ordering::SeqCst);
        }
    }
}

fn save_png(path: &Path, img: &egui::ColorImage) -> Result<(), String> {
    let [w, h] = img.size;
    let mut buf = Vec::with_capacity(w * h * 4);
    for p in &img.pixels {
        buf.extend_from_slice(&p.to_srgba_unmultiplied());
    }
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(&buf).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        let v = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        let a = Args::parse(v(
            "--tour out --glass-dir g --lens-catalog l1 --lens-catalog l2 sys.zmx",
        ))
        .unwrap();
        assert_eq!(a.tour, Some(PathBuf::from("out")));
        assert_eq!(a.glass_dirs, vec!["g"]);
        assert_eq!(a.lens_dirs, vec!["l1", "l2"]);
        assert_eq!(a.open, Some(PathBuf::from("sys.zmx")));
        assert_eq!(
            Args::parse(v("a.zmx")).unwrap().open,
            Some(PathBuf::from("a.zmx"))
        );
        assert!(Args::parse(v("--tour")).is_err());
        assert!(Args::parse(v("--what")).is_err());
    }

    #[test]
    fn steps_end() {
        let mut s = Step::Open;
        let mut n = 0;
        while s != Step::Done {
            s = s.next();
            n += 1;
        }
        assert_eq!(n, 27);
    }
}
