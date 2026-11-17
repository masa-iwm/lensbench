//! 系の膜（[`Coatings`]）: 膜ファイル・実測表・モデルの置き場、フォルダの探索、境界の鍵

use super::*;

/// 境界の鍵（`膜|前の媒質|後の媒質`、鏡は後が `MIRROR`、屈折面での反射は `膜|入射側|反対側|R`）を分けたもの
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::coating) struct IfaceKey<'a> {
    pub(in crate::coating) coat: &'a str,
    pub(in crate::coating) after: After<'a>,
}

/// 境界の後ろ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::coating) enum After<'a> {
    /// 透過: 後ろの媒質の名前
    Medium(&'a str),
    /// 鏡
    Mirror,
    /// 屈折面での反射: 反対側の媒質の名前
    Reflect(&'a str),
}

impl<'a> IfaceKey<'a> {
    pub(in crate::coating) fn parse(key: &'a str) -> IfaceKey<'a> {
        let mut it = key.splitn(4, '|');
        let coat = it.next().unwrap_or("");
        let _before = it.next();
        let m2 = it.next().unwrap_or("");
        let after = if it.next() == Some("R") {
            After::Reflect(m2)
        } else if m2 == "MIRROR" {
            After::Mirror
        } else {
            After::Medium(m2)
        };
        IfaceKey { coat, after }
    }

    pub(in crate::coating) fn is_reflect(&self) -> bool {
        matches!(self.after, After::Reflect(_))
    }

    /// 後ろ（反射では反対側）が空気
    pub(in crate::coating) fn after_is_air(&self) -> bool {
        matches!(self.after, After::Medium("AIR") | After::Reflect("AIR"))
    }
}

/// 境界の膜の扱い
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoatKind {
    /// 膜なし（フレネル、鏡は金属基板）
    Bare,
    /// 理想膜 I.xx
    Ideal,
    /// 膜ファイルの薄膜（透過・鏡）
    Stack,
    /// 膜ファイルの表
    Table,
    /// 実測表
    Measured,
    /// 特性行列モデル（[`CoatModel`]）
    Model,
    /// 膜ファイルになく、実測表もない（暗号化された膜など）
    Missing,
}

impl CoatKind {
    pub fn name(self) -> &'static str {
        match self {
            CoatKind::Bare => "bare",
            CoatKind::Ideal => "ideal",
            CoatKind::Stack => "stack",
            CoatKind::Table => "table",
            CoatKind::Measured => "measured",
            CoatKind::Model => "model",
            CoatKind::Missing => "missing",
        }
    }

    /// 実測表が要る（なければ偏光追跡が止まる）
    pub fn needs_table(self) -> bool {
        matches!(self, CoatKind::Missing)
    }
}

/// 膜の透過の位相の約束
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PhaseConvention {
    /// OpticStudio と同じ: 層ごとの項 −Σ d_j Re(β²/q_j)（光線の層の中での横ずれの位相）を含む。
    /// β が層の屈折率に近づくと発散するので、膜のモデルは BMAX を超える β で止める
    #[default]
    OpticStudio,
    /// 層ごとの項を外す（波として解いた係数のまま。どの β でも有限で連続）。OpticStudio とは
    /// s・p 共通の位相だけ違う。層のわかる膜（膜ファイルの COAT）とモデルにだけ効き、実測表・TABLE・
    /// 理想膜・膜なしには効かない（層の項がない、または測った値そのもの）
    Physical,
}

/// TABLE 膜の垂直入射の扱い（表の 0° で s と p が違うとき）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableNormal {
    /// OpticStudio と同じ: 垂直入射（1 − cosθ < [`TABLE_NORMAL`]）だけ s に p の値を使い、
    /// その外は表の s を補間する（0° のすぐ外で s に段差が出る）
    #[default]
    OpticStudio,
    /// 段差なし: 0° の行の s を p で置き換えて補間する（連続。小さい角度で OpticStudio と違う）
    Continuous,
}

/// 系の膜: 膜ファイルと実測表
#[derive(Debug, Clone, Default)]
pub struct Coatings {
    pub file: CoatingFile,
    /// 境界の鍵 → 実測表
    pub measured: HashMap<String, Measured>,
    /// 膜の名前 → 特性行列モデル
    pub models: HashMap<String, CoatModel>,
    /// 透過の位相の約束
    pub phase: PhaseConvention,
    /// TABLE 膜の垂直入射の扱い
    pub table_normal: TableNormal,
    /// 膜ファイルのフォルダから膜の名前で自動で読んだモデル・実測表（系の膜の名前 → ファイル）。
    /// [`Coatings::for_system`] が埋める
    pub auto: Vec<(String, PathBuf)>,
    /// 自動で読んだ実測表（`.lbct`）の境界の鍵（明示したモデルのある膜から外すため）
    pub auto_measured: Vec<String>,
}

impl Coatings {
    /// 系の膜ファイル（zmx の COFN）。なければ空（膜なし・理想膜・実測表だけ）
    pub fn for_system(sys: &System) -> Result<Coatings> {
        let mut file = match &sys.coating_file {
            Some(p) => CoatingFile::load(p)?,
            None => CoatingFile::default(),
        };
        // 系に書いた表（フィルタ）は膜ファイルより優先
        for (k, t) in &sys.coat_tables {
            file.table.insert(k.clone(), t.clone());
        }
        let mut c = Coatings {
            file,
            measured: HashMap::new(),
            models: HashMap::new(),
            phase: PhaseConvention::default(),
            table_normal: TableNormal::default(),
            auto: Vec::new(),
            auto_measured: Vec::new(),
        };
        if let Some(dir) = sys.coating_file.as_ref().and_then(|p| p.parent()) {
            c.load_dir_for(sys, dir)?;
        }
        Ok(c)
    }

    /// フォルダ dir から、系 sys が使う膜のうち膜ファイルで決まらないもの（暗号化された膜など）の
    /// モデル・実測表を膜の名前で探して読む。ファイル名は `<膜の名前>.lbcb`・`.lbcm`・`.lbct`、
    /// 層のわかる近似の膜なら `.lbcs`（膜ファイルの形式で COAT とその材料の MATE）
    /// （大文字小文字は区別しない。同じ膜で複数あれば lbcb・lbcm・lbct・lbcs の順に 1 つ）。
    /// 使う膜だけ読む。読んだものは [`Coatings::auto`] に記録し、読んだ数を返す。
    /// 後から [`Coatings::load_file`] で読んだもの（明示の指定）が勝つ
    pub fn load_dir_for(&mut self, sys: &System, dir: &Path) -> Result<usize> {
        let (keys, _) = media_keys(sys);
        let mut want: Vec<&str> = Vec::new();
        for k in keys.iter().flatten() {
            let coat = IfaceKey::parse(k).coat;
            if coat.is_empty()
                || coat == "-"
                || coat.starts_with("I.")
                || self.file.table.contains_key(coat)
                || self.file.coat.contains_key(coat)
                || want.contains(&coat)
            {
                continue;
            }
            want.push(coat);
        }
        if want.is_empty() || !crate::vfs::is_dir(dir) {
            return Ok(0);
        }
        let files =
            crate::vfs::read_dir(dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
        // 膜の名前（大文字）→ (拡張子の順位, パス)
        let mut found: HashMap<String, (usize, PathBuf)> = HashMap::new();
        for p in files {
            let (Some(stem), Some(ext)) = (p.file_stem(), p.extension()) else {
                continue;
            };
            let ext = ext.to_string_lossy().to_ascii_lowercase();
            let Some(rank) = ["lbcb", "lbcm", "lbct", "lbcs"]
                .iter()
                .position(|e| *e == ext)
            else {
                continue;
            };
            let name = stem.to_string_lossy().to_uppercase();
            if found.get(&name).is_none_or(|(r, _)| rank < *r) {
                found.insert(name, (rank, p));
            }
        }
        let mut n = 0;
        for coat in want {
            // フォルダに無ければ、共有の置き場から取る（[`set_remote_loader`]、配る版で膜のモデルを外したとき）
            let remote;
            let p = match found.get(&coat.to_uppercase()) {
                Some((_, p)) => p,
                None => match super::remote::remote_loader().and_then(|f| f(dir, coat)) {
                    Some(r) => {
                        remote = r;
                        &remote
                    }
                    None => continue,
                },
            };
            let lbcs = p
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lbcs"));
            if lbcs {
                // 層のわかる近似の膜（膜ファイルの形式）: 膜を系の名前で入れ、足りない材料も足す
                let f = CoatingFile::load(p)?;
                let Some(ls) = f
                    .coat
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(coat))
                    .map(|(_, l)| l.clone())
                else {
                    let path = p.display();
                    invalid!("{path}: no COAT for coating {coat} (the file name and the coating name inside differ)");
                };
                for (m, rows) in f.mate {
                    self.file.mate.entry(m).or_insert(rows);
                }
                self.file.coat.insert(coat.to_string(), ls);
                self.auto.push((coat.to_string(), p.clone()));
                n += 1;
                continue;
            }
            let mut one = Coatings::default();
            one.load_file(p)?;
            // ファイルの中の膜の名前の大文字小文字が系と違っても、系の名前で引けるようにする
            let mut hit = false;
            for (name, m) in one.models {
                if name.eq_ignore_ascii_case(coat) {
                    self.models.insert(coat.to_string(), m);
                    hit = true;
                }
            }
            for (key, m) in one.measured {
                let mut it = key.splitn(2, '|');
                let (c0, rest) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
                if c0.eq_ignore_ascii_case(coat) {
                    let k = format!("{coat}|{rest}");
                    self.measured.insert(k.clone(), m);
                    self.auto_measured.push(k);
                    hit = true;
                }
            }
            if !hit {
                let path = p.display();
                invalid!("{path}: no model or measured table for coating {coat} (the file name and the coating name inside differ)");
            }
            self.auto.push((coat.to_string(), p.clone()));
            n += 1;
        }
        Ok(n)
    }

    pub fn add_measured(&mut self, key: &str, m: Measured) -> Result<()> {
        if m.data.len() != m.wavelengths.len()
            || m.data.iter().any(|r| r.len() != m.angles_deg.len())
        {
            invalid!("size of measured table {key}");
        }
        if m.angles_deg.windows(2).any(|w| !(w[0] < w[1])) {
            invalid!("the angles of measured table {key} are not ascending");
        }
        self.measured.insert(key.to_string(), m);
        Ok(())
    }

    /// 境界 key の膜の扱い（[`Coatings::prepare`] もこの判定で係数の作り方を選ぶ）
    pub fn kind(&self, key: &str) -> CoatKind {
        self.resolve(&IfaceKey::parse(key), key)
    }

    /// 分けた境界の鍵 k（元の文字列 key）の膜の扱い。優先の順: 膜ファイルの表、膜なし、理想膜、膜ファイルの薄膜、
    /// 実測表（透過・鏡だけ）、モデル（その向きのデータがあるとき）
    pub(in crate::coating) fn resolve(&self, k: &IfaceKey, key: &str) -> CoatKind {
        let coat = k.coat;
        if self.file.table.contains_key(coat) {
            CoatKind::Table
        } else if coat == "-" {
            CoatKind::Bare
        } else if coat.starts_with("I.") {
            CoatKind::Ideal
        } else if self.file.coat.contains_key(coat) {
            CoatKind::Stack
        } else if !k.is_reflect() && self.measured.contains_key(key) {
            CoatKind::Measured
        } else if self.models.get(coat).is_some_and(|m| {
            if k.after == After::Mirror {
                m.mirror.is_some()
            } else {
                m.has_transmit()
            }
        }) {
            CoatKind::Model
        } else {
            CoatKind::Missing
        }
    }

    /// 膜のモデルのファイル（[`parse_models`] の形式）を読み、モデルの数を返す
    pub fn load_models(&mut self, path: &Path) -> Result<usize> {
        let b =
            crate::vfs::read(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        let v = if b.starts_with(MODEL_BIN_HEADER) {
            parse_models_bin(&b)
        } else {
            std::str::from_utf8(&b)
                .map_err(|_| Error::Invalid("not UTF-8".into()))
                .and_then(parse_models)
        }
        .map_err(|e| match e {
            Error::Invalid(m) => Error::Invalid(format!("{}: {m}", path.display())),
            e => e,
        })?;
        let n = v.len();
        // 自動で読んだ実測表は、明示したモデルのある膜から外す（実測表はモデルより先に引かれる）
        let names: Vec<String> = v.iter().map(|(c, _)| c.clone()).collect();
        self.models.extend(v);
        let auto = std::mem::take(&mut self.auto_measured);
        for k in auto {
            let coat = IfaceKey::parse(&k).coat;
            if names.iter().any(|c| c == coat) {
                self.measured.remove(&k);
            } else {
                self.auto_measured.push(k);
            }
        }
        self.drop_auto_stacks();
        Ok(n)
    }

    /// 自動で読んだ近似の膜（`.lbcs`）のうち、後から明示して読んだモデル・実測表のある膜を膜ファイルから外す
    /// （膜ファイルの COAT はモデル・実測表より先に引かれるので、外さないと明示の指定が負ける）
    fn drop_auto_stacks(&mut self) {
        for (coat, p) in &self.auto {
            if !p
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lbcs"))
            {
                continue;
            }
            let pre = format!("{coat}|");
            if self.models.contains_key(coat) || self.measured.keys().any(|k| k.starts_with(&pre)) {
                self.file.coat.remove(coat);
            }
        }
    }

    /// 実測表（LBCT）か膜のモデル（LBCM）のファイルを、1 行目で見分けて読む
    pub fn load_file(&mut self, path: &Path) -> Result<usize> {
        let b =
            crate::vfs::read(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        if b.starts_with(MODEL_BIN_HEADER) {
            return self.load_models(path);
        }
        let txt = String::from_utf8_lossy(&b);
        let first = txt
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .unwrap_or("");
        if first == MODEL_HEADER {
            self.load_models(path)
        } else {
            self.load_measured(path)
        }
    }

    /// 実測表のファイル（[`parse_measured`] の形式）を読み、表の数を返す
    pub fn load_measured(&mut self, path: &Path) -> Result<usize> {
        let txt = crate::vfs::read_to_string(path)
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        let v = parse_measured(&txt).map_err(|e| match e {
            Error::Invalid(m) => Error::Invalid(format!("{}: {m}", path.display())),
            e => e,
        })?;
        let n = v.len();
        for (k, m) in v {
            self.auto_measured.retain(|a| *a != k);
            self.add_measured(&k, m)?;
        }
        self.drop_auto_stacks();
        Ok(n)
    }

    /// 材料の複素屈折率（MATE の表、なければ系のガラス）
    pub(in crate::coating) fn mat_n(&self, sys: &System, name: &str, wl: f64) -> Result<C64> {
        if let Some(m) = self.file.mate.get(name) {
            if m.is_empty() {
                invalid!("coating material {name} has no data");
            }
            let xs: Vec<f64> = m.iter().map(|r| r[0]).collect();
            // 表の中は隣の 2 行の線形、外は近い方の端の行の値（外挿しない）。OpticStudio 2026 R1.01 と同じ
            // （2026-11-13 実測: 7 膜 0.30〜1.10 µm で反射率・透過率 5e-8、位相 5e-5° 一致。OpticStudio は注意も出さない。
            // 膜厚を波の単位から直す基準波長の n も同じ）。1 行だけの材料は分散なし
            let n: Vec<f64> = m.iter().map(|r| r[1]).collect();
            let k: Vec<f64> = m.iter().map(|r| r[2]).collect();
            // N = n + ik の約束で吸収は k < 0（文献の n − ik の k は符号を変えて書く）。k > 0 は増幅する媒質になる。
            // 使う波長で補間した k を見る。OpticStudio 標準の COATING.DAT にも数値の誤差で +1e-11〜+5e-7 の行が
            // あるので（赤外の端の行など）、それより大きい正の k だけを誤りにする（値はそのまま使う）
            let kk = interp(wl, &xs, &k);
            if kk > MATE_K_POS_TOL {
                invalid!("coating material {name} has a positive k ({kk} at {wl} µm); write absorption as k < 0");
            }
            return Ok(C64::new(interp(wl, &xs, &n), interp(wl, &xs, &k)));
        }
        match sys.glasses.get(name) {
            Some(g) => Ok(C64::new(
                g.index(wl, sys.temperature, sys.pressure, sys.adjust_index),
                0.0,
            )),
            None => invalid!(
                "coating material {name} is neither in the coating file nor in the glass catalogs"
            ),
        }
    }
}

/// 面ごとの境界の鍵（`膜|前の媒質|後の媒質（鏡は MIRROR）`、境界でなければ None）と、各面の前の媒質の名前
pub fn media_keys(sys: &System) -> (Vec<Option<String>>, Vec<String>) {
    let sf = &sys.surfaces;
    let mut keys = vec![None; sf.len()];
    let mut before = vec!["AIR".to_string(); sf.len()];
    let mut prev = "AIR".to_string();
    let mut nprev = sys.n(0)[0];
    for (k, d) in sf.iter().enumerate() {
        before[k] = prev.clone();
        if d.ignore || d.is_coord_break() {
            continue;
        }
        let after = if d.is_mirror() {
            prev.clone()
        } else {
            d.mat.label()
        };
        // 名前の違う媒質は、ある波長で屈折率が同じでも境界にする（他の波長でフレネルが抜けないように）。
        // 複屈折材の板の入口・出口も境界にする（同じ材料の常光どうしでも、異常光の成分に Fresnel がある）
        if k > 0
            && (!d.coat.is_empty()
                || d.is_mirror()
                || after != prev
                || (sys.n(k)[0] - nprev).abs() > 1e-12
                || d.bire.is_some())
        {
            let c = if d.coat.is_empty() { "-" } else { &d.coat };
            keys[k] = Some(match &d.reflect {
                // 屈折面での反射（ゴースト）: `膜|入射側|反対側|R`
                Some(r) if d.is_mirror() => format!("{c}|{prev}|{}|R", r.mat),
                _ => {
                    let a = if d.is_mirror() { "MIRROR" } else { &after };
                    format!("{c}|{prev}|{a}")
                }
            });
        }
        if !d.is_mirror() {
            prev = after;
        }
        nprev = sys.n(k)[0];
    }
    (keys, before)
}

/// 媒質の内部透過率（厚さ 1 mm あたりの強度。IT データがない・波長が範囲外なら None）
pub(crate) fn internal_t(g: &glass::Glass, wl: f64) -> Option<f64> {
    if g.it.is_empty() {
        return None;
    }
    // 多くは波長の順に並んでいるので、並んでいなければだけ並べ替えた写しを作る
    let sorted;
    let it: &[[f64; 3]] = if g.it.is_sorted_by(|a, b| a[0] <= b[0]) {
        &g.it
    } else {
        let mut v = g.it.clone();
        v.sort_by(|a, b| a[0].total_cmp(&b[0]));
        sorted = v;
        &sorted
    };
    let n = it.len();
    if !(it[0][0] <= wl && wl <= it[n - 1][0]) {
        return None;
    }
    // interp と同じ線形補間（両端は端の値）を、内部透過率と厚さの 2 列で同じ区間について
    let col = |k: usize| {
        if n == 1 || wl <= it[0][0] {
            return it[0][k];
        }
        if wl >= it[n - 1][0] {
            return it[n - 1][k];
        }
        let i = it.partition_point(|r| r[0] <= wl) - 1;
        let t = (wl - it[i][0]) / (it[i + 1][0] - it[i][0]);
        it[i][k] + t * (it[i + 1][k] - it[i][k])
    };
    Some(col(1).powf(1.0 / col(2)))
}
