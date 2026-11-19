//! 多重構成の表（OpticStudio の Multi-Configuration Editor の数の行）と、その編集。

use crate::error::Result;
use crate::invalid;
use crate::param::Param;
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
    /// 構成ごとの値（長さは構成の数）。ピックアップの構成の値は元の構成から求めた値
    pub values: Vec<f64>,
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
            let now = self.get_param(r.param)?;
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

    /// 表に行を足す（同じパラメータの行は置き換える）。values の長さは構成の数。今の構成の値を系に入れる
    pub fn set_config_row(&mut self, p: Param, values: Vec<f64>) -> Result<()> {
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
        let mut row = ConfigRow {
            param: p,
            values,
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
            if let (Ok(v), Some(x)) = (self.get_param(r.param), r.values.get_mut(k)) {
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
            let ch: Vec<(Param, f64)> = c.rows.iter().map(|r| (r.param, r.values[0])).collect();
            t.rx_mut().configs = None;
            *self = t.with_params(&ch)?;
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
        let ch: Vec<(Param, f64)> = c.rows.iter().map(|r| (r.param, r.values[k - 1])).collect();
        let mut s = self.with_params(&ch)?;
        if let Some(c) = &mut s.rx_mut().configs {
            c.active = k;
        }
        Ok(s)
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
