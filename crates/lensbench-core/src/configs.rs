//! 多重構成の表（OpticStudio の Multi-Configuration Editor の数の行）と、その編集。

use crate::error::Result;
use crate::invalid;
use crate::param::{Kind, Param};
use crate::system::System;

/// 多重構成の表（OpticStudio の Multi-Configuration Editor の数の行に当たる）。
/// 行ごとにパラメータ 1 つと、構成ごとの値を持つ。[`System::config`] で構成 k の値を入れた系を作る
#[derive(
    Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Configs {
    /// 構成の数（1 以上）
    pub n: usize,
    /// 今の系に入っている構成（1 始まり）
    pub active: usize,
    pub rows: Vec<ConfigRow>,
}

/// 多重構成の表の 1 行
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ConfigRow {
    pub param: Param,
    /// 構成ごとの値（長さは構成の数）。ピックアップの構成の値は元の構成から求めた値。
    /// 材料・膜の行（[`Kind::Material`]・[`Kind::Coating`]）は `names` の番号（0 始まり）
    pub values: Vec<f64>,
    /// 材料の行の材料の名前（面の表の材料と同じ書き方）、膜の行の膜の名前（空は膜なし）。ほかの行は空で、省く
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    /// 構成のピックアップ（OpticStudio の多重構成の pickup）。無ければ省く
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub picks: Vec<ConfigPick>,
}

/// 多重構成の行の、構成 k の値 = 構成 from の値 × scale + offset（どちらも 1 始まり）
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct ConfigPick {
    pub k: usize,
    pub from: usize,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub offset: f64,
}

fn one() -> f64 {
    1.0
}

pub(crate) fn yes() -> bool {
    true
}

pub(crate) fn is_yes(b: &bool) -> bool {
    *b
}

impl ConfigRow {
    /// ピックアップの値を元の構成から求め直す（ピックアップの元がピックアップでも、並びの順によらず）
    pub fn resolve_picks(&mut self) {
        for _ in 0..=self.picks.len() {
            let mut changed = false;
            for p in &self.picks {
                if let (Some(&v), true) = (self.values.get(p.from - 1), p.k <= self.values.len()) {
                    let x = v * p.scale + p.offset;
                    if self.values[p.k - 1] != x {
                        self.values[p.k - 1] = x;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// 構成 k のピックアップ
    pub fn pick(&self, k: usize) -> Option<&ConfigPick> {
        self.picks.iter().find(|p| p.k == k)
    }

    /// 材料の行か
    pub fn is_material(&self) -> bool {
        self.param.kind == Kind::Material
    }

    /// 膜の行か
    pub fn is_coating(&self) -> bool {
        self.param.kind == Kind::Coating
    }

    /// 値が名前の番号の行（材料・膜の行）か
    pub fn has_names(&self) -> bool {
        self.is_material() || self.is_coating()
    }

    /// 材料・膜の行の構成 k（1 始まり）の名前
    pub fn name_at(&self, k: usize) -> Result<&str> {
        let i = self.values.get(k - 1).copied().unwrap_or(f64::NAN);
        match (i >= 0.0 && i.fract() == 0.0)
            .then(|| self.names.get(i as usize))
            .flatten()
        {
            Some(n) => Ok(n),
            None => invalid!(
                "multi-configuration row {} configuration {k}: no name {i}",
                self.param.name()
            ),
        }
    }

    /// 材料・膜の行の名前 name の番号（無ければ足す）
    fn name_index(&mut self, name: &str) -> f64 {
        match self.names.iter().position(|n| n == name) {
            Some(i) => i as f64,
            None => {
                self.names.push(name.to_string());
                (self.names.len() - 1) as f64
            }
        }
    }
}

impl System {
    /// 多重構成の表（無ければ None）
    pub fn configs(&self) -> Option<&Configs> {
        self.configs.as_ref()
    }

    /// 構成の数を n にする（増やした構成の値は今の値。1 で表を消す）。今の構成が n を超えたら 1 にする
    pub fn set_config_count(&mut self, n: usize) -> Result<()> {
        if n == 0 {
            invalid!("0 configurations");
        }
        if n == 1 {
            self.rx_mut().configs = None;
            return Ok(());
        }
        let mut c = self.configs.clone().unwrap_or(Configs {
            n: 1,
            active: 1,
            rows: Vec::new(),
        });
        for r in c.rows.iter_mut() {
            // 開口の無い面の半径・偏心の行は 0（OpticStudio も使わない）
            let now = match self.row_value(r) {
                Err(_) if is_aperture_value(r.param.kind) && self.no_aperture(r.param) => 0.0,
                v => v?,
            };
            r.values.resize(n, now);
            r.picks.retain(|p| p.k <= n && p.from <= n);
        }
        c.n = n;
        let back = c.active > n;
        if back {
            c.active = 1;
        }
        if back {
            // 今の構成が無くなるので構成 1 の値を系に入れる
            let mut t = self.clone();
            t.rx_mut().configs = Some(c);
            *self = t.config(1)?;
        } else {
            self.rx_mut().configs = Some(c);
        }
        Ok(())
    }

    /// 表に行を足す（同じパラメータの行は置き換える）。values の長さは構成の数。今の構成の値を系に入れる。
    /// 材料・膜の行は [`System::set_config_materials`]・[`System::set_config_coatings`] で
    pub fn set_config_row(&mut self, p: Param, values: Vec<f64>) -> Result<()> {
        if p.kind == Kind::Material || p.kind == Kind::Coating {
            invalid!("{}: set a material or coating row by names", p.name());
        }
        self.put_config_row(p, values, Vec::new())
    }

    /// 面 j の材料の行を足す（同じ面の材料の行は置き換える）。names は構成ごとの材料の名前（面の表の材料と同じ書き方）
    pub fn set_config_materials(&mut self, j: usize, names: &[String]) -> Result<()> {
        let mut row = ConfigRow {
            param: Param::new(j, Kind::Material),
            values: Vec::new(),
            names: Vec::new(),
            picks: Vec::new(),
        };
        row.values = names.iter().map(|n| row.name_index(n.trim())).collect();
        for n in &row.names {
            // 名前の書き方を先に確かめる（座標ブレークの面のガラスなどは構成の系を作るときに断る）
            self.clone().set_material(j, n)?;
        }
        self.put_config_row(row.param, row.values, row.names)
    }

    /// 面 j の膜の行を足す（同じ面の膜の行は置き換える）。names は構成ごとの膜の名前（空は膜なし）
    pub fn set_config_coatings(&mut self, j: usize, names: &[String]) -> Result<()> {
        if j > self.image() {
            invalid!("surface {j}");
        }
        let mut row = ConfigRow {
            param: Param::new(j, Kind::Coating),
            values: Vec::new(),
            names: Vec::new(),
            picks: Vec::new(),
        };
        row.values = names.iter().map(|n| row.name_index(n.trim())).collect();
        self.put_config_row(row.param, row.values, row.names)
    }

    fn put_config_row(&mut self, p: Param, values: Vec<f64>, names: Vec<String>) -> Result<()> {
        let Some(c) = &self.configs else {
            invalid!("only one configuration (set_config_count first)");
        };
        if values.len() != c.n {
            invalid!("{} values ({} configurations)", values.len(), c.n);
        }
        self.check_param(p)?;
        if p.kind.is_object() {
            invalid!("{} can't go in the table (it is a trace input)", p.name());
        }
        let mut c = c.clone();
        // ピックアップは残す（ピックアップの構成の値を変えたら、その構成のピックアップは外す）
        let mut picks = Vec::new();
        if let Some(old) = c.rows.iter().find(|r| r.param == p) {
            picks = old.picks.clone();
            picks.retain(|q| old.values.get(q.k - 1) == values.get(q.k - 1));
        }
        c.rows.retain(|r| r.param != p);
        if !names.is_empty() {
            // 材料・膜の番号は行ごとに付け直すので、ピックアップは番号ではなく名前で比べられない。外す
            picks.clear();
        }
        let mut row = ConfigRow {
            param: p,
            values,
            names,
            picks,
        };
        row.resolve_picks();
        c.rows.push(row);
        let mut s = self.clone();
        s.rx_mut().configs = Some(c);
        let (k, n) = s
            .configs
            .as_ref()
            .map(|c| (c.active, c.n))
            .unwrap_or((1, 1));
        // ほかの構成の値も系にできるか確かめる（範囲の外の主波長・0 の開口など）
        for j in (1..=n).filter(|&j| j != k) {
            s.config(j)?;
        }
        *self = s.config(k)?;
        Ok(())
    }

    /// 表の今の構成の列を、系の今の値に合わせる（面の表・set で表の行の値を変えたとき。
    /// OpticStudio で Lens Data の値を変えると Multi-Configuration Editor の今の構成の値も変わるのと同じ）
    pub fn sync_active_config(&mut self) {
        let Some(mut c) = self.rx_mut().configs.take() else {
            return;
        };
        let k = c.active.saturating_sub(1);
        for r in &mut c.rows {
            if let (Ok(v), true) = (self.row_value(r), k < r.values.len()) {
                let x = &mut r.values[k];
                if *x != v {
                    // 今の構成がピックアップなら、値を変えたのでピックアップを外す
                    *x = v;
                    r.picks.retain(|p| p.k != k + 1);
                }
            }
            // 今の構成を元にするピックアップの値も変わる（今の構成でない列なので系はそのまま）
            r.resolve_picks();
        }
        self.rx_mut().configs = Some(c);
    }

    /// 構成を位置 at（1 始まり、1〜n+1）に挿入する。値は構成 from（挿入前の番号）の値。pickup なら各行の新しい構成を
    /// 構成 from のピックアップにする（OpticStudio の Insert Configuration with Pickups）。ピックアップの番号は付け替える。
    /// 今の構成は同じ構成を指したまま（番号はずれる）。表が無ければ構成 2 つの表を作る
    pub fn insert_config(&mut self, at: usize, from: usize, pickup: bool) -> Result<()> {
        let mut c = self.configs.clone().unwrap_or(Configs {
            n: 1,
            active: 1,
            rows: Vec::new(),
        });
        if at == 0 || at > c.n + 1 {
            invalid!("position {at} to insert a configuration (1 to {})", c.n + 1);
        }
        if from == 0 || from > c.n {
            invalid!("configuration {from} (1 to {})", c.n);
        }
        let shift = |k: usize| if k >= at { k + 1 } else { k };
        let src = shift(from);
        for r in c.rows.iter_mut() {
            let v = r.values[from - 1];
            r.values.insert(at - 1, v);
            for p in r.picks.iter_mut() {
                p.k = shift(p.k);
                p.from = shift(p.from);
            }
            if pickup {
                r.picks.push(ConfigPick {
                    k: at,
                    from: src,
                    scale: 1.0,
                    offset: 0.0,
                });
            } else if let Some(q) = r.picks.iter().find(|p| p.k == src).copied() {
                // 元がピックアップなら、新しい構成も同じ元のピックアップ
                r.picks.push(ConfigPick { k: at, ..q });
            }
        }
        c.n += 1;
        c.active = shift(c.active);
        self.rx_mut().configs = Some(c);
        Ok(())
    }

    /// 構成 k（1 始まり）を消す。k を元にするピックアップは値を残して外し、番号を付け替える。今の構成を消したら
    /// 次の構成（最後なら前）を系に入れる。構成が 1 つになったら表を消す（残った構成の値は系に入る）
    pub fn remove_config(&mut self, k: usize) -> Result<()> {
        let Some(mut c) = self.configs.clone() else {
            invalid!("only one configuration");
        };
        if k == 0 || k > c.n {
            invalid!("configuration {k} (1 to {})", c.n);
        }
        let shift = |j: usize| if j > k { j - 1 } else { j };
        for r in c.rows.iter_mut() {
            r.values.remove(k - 1);
            r.picks.retain(|p| p.k != k && p.from != k);
            for p in r.picks.iter_mut() {
                p.k = shift(p.k);
                p.from = shift(p.from);
            }
        }
        let was = c.active;
        c.n -= 1;
        c.active = if was == k { k.min(c.n) } else { shift(was) };
        let mut t = self.clone();
        if c.n == 1 {
            // 表の値の数を 1 つにはできないので、残った構成の値を系に入れて表を消す
            t.rx_mut().configs = None;
            *self = t.with_config_values(&c.rows, 1)?;
            return Ok(());
        }
        t.rx_mut().configs = Some(c.clone());
        *self = if was == k { t.config(c.active)? } else { t };
        Ok(())
    }

    /// 表の行 p の構成 k を、構成 from の値 × scale + offset のピックアップにする（同じ構成のピックアップは置き換える）
    pub fn set_config_pickup(
        &mut self,
        p: Param,
        k: usize,
        from: usize,
        scale: f64,
        offset: f64,
    ) -> Result<()> {
        let Some(c) = &self.configs else {
            invalid!("only one configuration");
        };
        let n = c.n;
        if k == 0 || k > n || from == 0 || from > n || k == from {
            invalid!("configuration pickup {from} → {k} (1 to {n}, another configuration)");
        }
        if !scale.is_finite() || !offset.is_finite() {
            invalid!("configuration pickup scale or offset");
        }
        let mut c = c.clone();
        let Some(r) = c.rows.iter_mut().find(|r| r.param == p) else {
            invalid!("no row {} in the table", p.name());
        };
        let mut picks = r.picks.clone();
        picks.retain(|q| q.k != k);
        picks.push(ConfigPick {
            k,
            from,
            scale,
            offset,
        });
        // 循環（k が元をたどって k に戻る）は不可
        let mut at = from;
        for _ in 0..=picks.len() {
            match picks.iter().find(|q| q.k == at) {
                Some(q) if q.from == k => {
                    invalid!("configuration pickups go round in a loop ({k})")
                }
                Some(q) => at = q.from,
                None => break,
            }
        }
        r.picks = picks;
        r.resolve_picks();
        let active = c.active;
        let mut s = self.clone();
        s.rx_mut().configs = Some(c);
        *self = if active == k { s.config(k)? } else { s };
        Ok(())
    }

    /// 表の行 p の構成 k のピックアップを外す（値は今のまま）
    pub fn remove_config_pickup(&mut self, p: Param, k: usize) -> Result<()> {
        let Some(c) = &mut self.rx_mut().configs else {
            invalid!("only one configuration");
        };
        let Some(r) = c.rows.iter_mut().find(|r| r.param == p) else {
            invalid!("no row {} in the table", p.name());
        };
        let n0 = r.picks.len();
        r.picks.retain(|q| q.k != k);
        if r.picks.len() == n0 {
            invalid!("no pickup on {} configuration {k}", p.name());
        }
        Ok(())
    }

    /// 表の行（パラメータ p）を消す。系の値は今のまま
    pub fn remove_config_row(&mut self, p: Param) -> Result<()> {
        let Some(c) = &mut self.rx_mut().configs else {
            invalid!("only one configuration");
        };
        let n0 = c.rows.len();
        c.rows.retain(|r| r.param != p);
        if c.rows.len() == n0 {
            invalid!("no row {} in the table", p.name());
        }
        Ok(())
    }

    /// 構成 k（1 始まり）の値を入れた系（表は残し、今の構成を k にする）
    pub fn config(&self, k: usize) -> Result<System> {
        let Some(c) = &self.configs else {
            if k == 1 {
                return Ok(self.clone());
            }
            invalid!("configuration {k} (there is one configuration)");
        };
        if k == 0 || k > c.n {
            invalid!("configuration {k} (1 to {})", c.n);
        }
        let mut s = self.with_config_values(&c.rows, k)?;
        if let Some(c) = &mut s.rx_mut().configs {
            c.active = k;
        }
        Ok(s)
    }

    /// 表の行 rows の構成 k（1 始まり）の値を入れた系。材料・膜の行は名前で面の材料・膜にする
    fn with_config_values(&self, rows: &[ConfigRow], k: usize) -> Result<System> {
        let (base, off) = self.with_config_aperture_types(rows, k)?;
        let ch: Vec<(Param, f64)> = rows
            .iter()
            .filter(|r| !r.has_names() && r.param.kind != Kind::ApType)
            // 開口の無い構成の面の半径・偏心の行は使わない（OpticStudio と同じ）
            .filter(|r| !(is_aperture_value(r.param.kind) && off.contains(&r.param.index())))
            .map(|r| (r.param, r.values[k - 1]))
            .collect();
        let mut s = base.with_params(&ch)?;
        for r in rows.iter().filter(|r| r.is_material()) {
            let name = r.name_at(k)?;
            if s.surfaces[r.param.index()].mat.name() != name {
                s.set_material(r.param.index(), name)?;
            }
        }
        for r in rows.iter().filter(|r| r.is_coating()) {
            let name = r.name_at(k)?;
            if s.surfaces[r.param.index()].coat != name {
                s.set_coating(r.param.index(), name)?;
            }
        }
        Ok(s)
    }

    /// 開口の種類の行（[`Kind::ApType`]）の構成 k の値を先に入れた系と、開口の無い面。今の構成で開口の無い面を円形の開口・
    /// 遮蔽にするときは、同じ構成の最大の半径の行が要る（半径はその行から入る。OpticStudio は開口の無い構成でも半径の行の値を持つ）
    fn with_config_aperture_types(
        &self,
        rows: &[ConfigRow],
        k: usize,
    ) -> Result<(System, Vec<usize>)> {
        let mut s = self.clone();
        let mut off = Vec::new();
        for r in rows.iter().filter(|r| r.param.kind == Kind::ApType) {
            let (j, v) = (r.param.index(), r.values[k - 1]);
            let none = matches!(
                s.surfaces.get(j).map(|d| &d.ap),
                Some(crate::surface::Aperture::None)
            );
            let has_max = rows.iter().any(|q| q.param == Param::new(j, Kind::ApMax));
            if v == 0.0 {
                off.push(j);
            }
            if none && (v == 1.0 || v == 2.0) && !has_max {
                invalid!(
                    "multi-configuration row {}: surface {j} has no aperture in the current configuration; add the row {j}.apmax to give its radius",
                    r.param.name()
                );
            }
            if none && (v == 1.0 || v == 2.0) {
                s.edit_raw(|rx| {
                    rx.surfaces[j].ap = crate::surface::Aperture::Circular {
                        min: 0.0,
                        max: 0.0,
                        dx: 0.0,
                        dy: 0.0,
                    };
                    Ok(())
                })?;
            }
            s = s.with_params(&[(r.param, v)])?;
        }
        Ok((s, off))
    }

    /// 行 r の今の系の値。材料・膜の行は今の材料・膜の名前の番号（無ければ名前を足す）
    fn row_value(&self, r: &mut ConfigRow) -> Result<f64> {
        if r.has_names() {
            let Some(d) = self.surfaces.get(r.param.index()) else {
                invalid!("multi-configuration row {}", r.param.name());
            };
            let name = if r.is_material() {
                d.mat.name()
            } else {
                d.coat.clone()
            };
            return Ok(r.name_index(&name));
        }
        self.get_param(r.param)
    }

    pub(crate) fn check_configs(&self) -> Result<()> {
        let Some(c) = &self.configs else {
            return Ok(());
        };
        if c.n < 2 || c.active == 0 || c.active > c.n {
            invalid!(
                "multi-configuration table ({} configurations, now {})",
                c.n,
                c.active
            );
        }
        for r in &c.rows {
            if r.values.len() != c.n {
                invalid!(
                    "multi-configuration row {} has {} values",
                    r.param.name(),
                    r.values.len()
                );
            }
            self.check_param(r.param)?;
            if r.has_names() {
                for k in 1..=c.n {
                    r.name_at(k)?;
                }
            } else if !r.names.is_empty() {
                invalid!(
                    "multi-configuration row {} has names (only material and coating rows do)",
                    r.param.name()
                );
            }
            for p in &r.picks {
                if p.k == 0 || p.k > c.n || p.from == 0 || p.from > c.n || p.k == p.from {
                    invalid!(
                        "multi-configuration row {} pickup {} → {}",
                        r.param.name(),
                        p.from,
                        p.k
                    );
                }
            }
        }
        Ok(())
    }
}

/// 円形の開口・遮蔽の半径・偏心の行か
fn is_aperture_value(k: Kind) -> bool {
    matches!(k, Kind::ApMin | Kind::ApMax | Kind::ApDx | Kind::ApDy)
}

impl System {
    /// p の面に開口が無いか
    fn no_aperture(&self, p: Param) -> bool {
        matches!(
            self.surfaces.get(p.index()).map(|d| &d.ap),
            Some(crate::surface::Aperture::None)
        )
    }
}
