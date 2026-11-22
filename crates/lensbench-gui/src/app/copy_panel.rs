//! Web 版: 解析以外のパネルも、別の窓が元のタブの写しを持って自分で描く（[`copies`]）。
//!
//! - 系の写しは解析の窓と同じ（[`super::calc_panel`]）。組合せを見るパネルには、組合せ（保存の形）と、その後の
//!   組合せの書き換え（[`super::edit::Edit`]）も送る（[`CopySync`]）。履歴のパネルには元に戻す段の一覧を送る
//!   （[`HistCopy`]）
//! - 別の窓でした操作は自分では当てず、[`Fwd`] にして元のタブへ送る。文書を変えるのは元のタブだけで、当てた結果
//!   （組合せの書き換え・系の読み込みと変更）がまた写しとして全部の別の窓に届く。元に戻す段・保存・ファイルの窓も
//!   元のタブ
//! - 別の窓の中で当てるのは、文書を変えない操作（解析の計算・表の候補の読み込み・選び方）だけ（[`local`]）

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use super::*;

/// 別の窓が元のタブの写しを持って自分で描くパネルか。ほかのパネル（最適化・公差・差し替え探索・ズームなど
/// 長い計算を元のタブで回すもの）は、元のタブが描いて画面を送る
pub(super) fn copies(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Analysis(_)
            | Kind::System
            | Kind::SysSettings
            | Kind::LensData
            | Kind::MultiConfig
            | Kind::History
    )
}

/// 組合せの写しも要るパネルか（解析のパネルは系の写しだけで描ける）
pub(super) fn needs_combo(kind: &Kind) -> bool {
    copies(kind) && !matches!(kind, Kind::Analysis(_))
}

/// 別の窓から元のタブへ送る操作（[`Msg`] のうち文書・元のタブの状態を変えるもの）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(super) enum Fwd {
    /// 組合せの欄の書き換え
    Edit {
        ops: Vec<super::edit::Op>,
    },
    /// 面の表の操作
    LensData {
        act: lens_data::Act,
    },
    /// 元に戻す（back）・やり直す
    Undo {
        back: bool,
        steps: usize,
    },
    SetParam {
        param: String,
        value: f64,
    },
    AddParam {
        param: String,
    },
    RemoveParam {
        index: usize,
    },
    /// 多重構成の次・前（back）の構成
    CycleConfig {
        back: bool,
    },
    New,
    Open,
    /// 最近のファイル（元のタブと同じ置き場のパス）
    OpenFile {
        path: PathBuf,
    },
    /// 保存（as_new は名前を付けて保存）
    Save {
        as_new: bool,
    },
    /// 今の組合せで系を作る
    Build,
    /// 像面を近軸の焦点へ
    FocusImage,
}

impl Fwd {
    /// 元のタブへ送る操作なら、その形
    pub(super) fn from_msg(m: &Msg) -> Option<Fwd> {
        Some(match m {
            Msg::Edit(e) => Fwd::Edit { ops: e.ops.clone() },
            Msg::LensData(a) if !local(m) => Fwd::LensData { act: a.clone() },
            Msg::Undo(back, steps) => Fwd::Undo {
                back: *back,
                steps: *steps,
            },
            Msg::SetParam(p, x) => Fwd::SetParam {
                param: p.clone(),
                value: *x,
            },
            Msg::AddParam(p) => Fwd::AddParam { param: p.clone() },
            Msg::RemoveParam(i) => Fwd::RemoveParam { index: *i },
            Msg::CycleConfig(back) => Fwd::CycleConfig { back: *back },
            Msg::New => Fwd::New,
            Msg::Open => Fwd::Open,
            Msg::OpenFile(p) => Fwd::OpenFile { path: p.clone() },
            Msg::Save(as_new) => Fwd::Save { as_new: *as_new },
            Msg::Combo(combo::Act::Build) => Fwd::Build,
            Msg::Combo(combo::Act::Focus) | Msg::FocusImage => Fwd::FocusImage,
            _ => return None,
        })
    }

    /// 元のタブで当てる操作
    pub(super) fn into_msg(self) -> Msg {
        match self {
            Fwd::Edit { ops } => Msg::Edit(super::edit::Edit { ops }),
            Fwd::LensData { act } => Msg::LensData(act),
            Fwd::Undo { back, steps } => Msg::Undo(back, steps),
            Fwd::SetParam { param, value } => Msg::SetParam(param, value),
            Fwd::AddParam { param } => Msg::AddParam(param),
            Fwd::RemoveParam { index } => Msg::RemoveParam(index),
            Fwd::CycleConfig { back } => Msg::CycleConfig(back),
            Fwd::New => Msg::New,
            Fwd::Open => Msg::Open,
            Fwd::OpenFile { path } => Msg::OpenFile(path),
            Fwd::Save { as_new } => Msg::Save(as_new),
            Fwd::Build => Msg::Combo(combo::Act::Build),
            Fwd::FocusImage => Msg::FocusImage,
        }
    }
}

/// 別の窓の中で当てる操作か（文書を変えない: 解析の計算、表の選び方・候補と面のデータの読み込み・断る理由）
pub(super) fn local(m: &Msg) -> bool {
    use lens_data::Act as L;
    match m {
        Msg::Analyze(_) => true,
        Msg::LensData(a) => matches!(
            a,
            L::Select(_) | L::NeedData(_) | L::NeedGlasses | L::NeedCoatings | L::Refused(_)
        ),
        _ => false,
    }
}

/// 履歴の 1 段の写し
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct StepCopy {
    pub title: String,
    pub details: Vec<String>,
    /// 送ったときに何秒前の段か
    pub ago: f64,
    pub doc: bool,
}

/// 元に戻す・やり直す段の一覧の写し（履歴のパネル）
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct HistCopy {
    pub undo: Vec<StepCopy>,
    pub redo: Vec<StepCopy>,
    /// 元に戻せるか・やり直せるか
    pub can: [bool; 2],
}

impl HistCopy {
    pub(super) fn of(c: &Core) -> HistCopy {
        let now = web_time::Instant::now();
        let steps = |back: bool| -> Vec<StepCopy> {
            c.undo_list(back)
                .into_iter()
                .map(|s| StepCopy {
                    ago: now.saturating_duration_since(s.at).as_secs_f64(),
                    title: s.title,
                    details: s.details,
                    doc: s.doc,
                })
                .collect()
        };
        HistCopy {
            undo: steps(true),
            redo: steps(false),
            can: [c.can_undo(true), c.can_undo(false)],
        }
    }

    /// 前に送ったものと同じか（何秒前かは比べない）
    fn same(&self, o: &HistCopy) -> bool {
        let key = |v: &[StepCopy]| -> Vec<(String, Vec<String>, bool)> {
            v.iter()
                .map(|s| (s.title.clone(), s.details.clone(), s.doc))
                .collect()
        };
        self.can == o.can && key(&self.undo) == key(&o.undo) && key(&self.redo) == key(&o.redo)
    }

    /// 写しの段（受けた時刻から何秒前かを戻す）
    pub(super) fn steps(&self, back: bool, got: web_time::Instant) -> Vec<super::history::Step> {
        let v = if back { &self.undo } else { &self.redo };
        v.iter()
            .map(|s| super::history::Step {
                title: s.title.clone(),
                details: s.details.clone(),
                at: got
                    .checked_sub(std::time::Duration::from_secs_f64(s.ago.max(0.0)))
                    .unwrap_or(got),
                doc: s.doc,
            })
            .collect()
    }
}

/// 写しを持つ別の窓の受け渡しの状態（[`super::calc_panel::PopupDoc`] が持つ）
#[derive(Default)]
pub(super) struct Copies {
    /// 元のタブ: 窓ごとに送った組合せ・履歴
    pub sync: CopySync,
    /// 別の窓: 受けた組合せ・履歴の写し
    pub recv: CopyRecv,
    /// 元のタブ: 窓ごとの、ポインタを押しているか欄が入力を受けているか・その欄
    pub busy: HashMap<u64, (bool, Option<u64>)>,
    /// 別の窓（元のタブの写しを持つ）: 文書を自分では変えない（組合せの手直し・作り直しをしない）
    pub follower: bool,
}

/// 元のタブ: 別の窓ごとに送った組合せと履歴
#[derive(Default)]
pub(super) struct CopySync {
    /// 窓 id → 送った組合せの版（[`crate::combo_doc::ComboDoc::rev`]）と中身
    combo: HashMap<u64, (u64, Combo)>,
    /// 窓 id → 送った履歴
    hist: HashMap<u64, HistCopy>,
}

impl CopySync {
    /// keep に無い窓の覚えを捨てる
    pub(super) fn retain(&mut self, keep: &[u64]) {
        self.combo.retain(|id, _| keep.contains(id));
        self.hist.retain(|id, _| keep.contains(id));
    }

    /// 窓 id が始まり直した・写しが続かなかった: 次は全部を送る
    pub(super) fn restart(&mut self, id: u64) {
        self.combo.remove(&id);
        self.hist.remove(&id);
    }

    /// 窓 id に送る組合せ（版 rev の combo）。前に送った版と同じなら None。前に送っていれば変わった所だけ
    /// （`from` は前に送った版）、でなければ全部
    pub(super) fn combo_for(&mut self, id: u64, rev: u64, combo: &Combo) -> Option<Value> {
        let v = match self.combo.get(&id) {
            Some((r, _)) if *r == rev => return None,
            Some((r, c)) => match super::edit::Edit::between(c, combo) {
                Some(e) => json!({ "seq": rev, "from": r, "edit": e }),
                // 版だけ進んで中身は同じ: 窓の版を進める（空の書き換え）
                None => json!({ "seq": rev, "from": r, "edit": { "ops": [] } }),
            },
            None => json!({ "seq": rev, "full": combo }),
        };
        self.combo.insert(id, (rev, combo.clone()));
        Some(v)
    }

    /// 窓 id に送る履歴（前に送ったものと同じなら None）
    pub(super) fn hist_for(&mut self, id: u64, h: HistCopy) -> Option<Value> {
        if self.hist.get(&id).is_some_and(|o| o.same(&h)) {
            return None;
        }
        let v = serde_json::to_value(&h).ok();
        self.hist.insert(id, h);
        v
    }
}

/// 別の窓: 受けた組合せの写し
#[derive(Default)]
pub(super) struct CopyRecv {
    /// 当てた組合せの版（元のタブの版）
    rev: Option<u64>,
    /// 受けた履歴と受けた時刻
    pub hist: Option<(HistCopy, web_time::Instant)>,
}

impl CopyRecv {
    /// 組合せの写しを受けたか
    pub(super) fn ready(&self) -> bool {
        self.rev.is_some()
    }

    /// 組合せの写し v を combo に当てる。前の写しと続かない・当てられないなら false（全部を送り直してもらう）
    pub(super) fn combo(&mut self, combo: &mut Combo, v: &Value) -> bool {
        let seq = v["seq"].as_u64();
        if !v["full"].is_null() {
            return match serde_json::from_value::<Combo>(v["full"].clone()) {
                Ok(c) => {
                    *combo = c;
                    self.rev = seq;
                    true
                }
                Err(_) => {
                    self.rev = None;
                    false
                }
            };
        }
        let e = serde_json::from_value::<super::edit::Edit>(v["edit"].clone());
        match (self.rev, v["from"].as_u64(), e) {
            (Some(r), Some(from), Ok(e)) if r == from && e.apply(combo).is_ok() => {
                self.rev = seq;
                true
            }
            _ => {
                self.rev = None;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Combo {
        let mut c = Combo::default();
        c.insert_plane(0);
        c.insert_plane(1);
        c
    }

    /// 送る操作は JSON を通しても同じ操作に戻る。文書を変えない操作は送らない
    #[test]
    fn fwd_round_trips_and_keeps_local_ops() {
        let a = sample();
        let mut b = a.clone();
        b.stop_gap = 3.5;
        let e = super::super::edit::Edit::between(&a, &b).unwrap();
        let msgs = [
            Msg::Edit(e),
            Msg::Undo(true, 2),
            Msg::SetParam("3.t".into(), 1.5),
            Msg::AddParam("2.r".into()),
            Msg::RemoveParam(1),
            Msg::CycleConfig(true),
            Msg::Save(true),
            Msg::OpenFile("/uploads/a.zmx".into()),
            Msg::LensData(lens_data::Act::Lock("L1".into(), true)),
            Msg::Combo(combo::Act::Build),
            Msg::FocusImage,
        ];
        for m in msgs {
            let f = Fwd::from_msg(&m).unwrap_or_else(|| panic!("{m:?}"));
            let j = serde_json::to_value(&f).unwrap();
            let back: Fwd = serde_json::from_value(j).unwrap();
            assert_eq!(back, f);
            assert_eq!(Fwd::from_msg(&back.into_msg()), Some(f));
        }
        for m in [
            Msg::Analyze(3),
            Msg::LensData(lens_data::Act::NeedData(2)),
            Msg::LensData(lens_data::Act::Select(None)),
            Msg::TogglePalette,
        ] {
            assert_eq!(Fwd::from_msg(&m), None, "{m:?}");
        }
        assert!(local(&Msg::Analyze(1)));
        assert!(!local(&Msg::Undo(true, 1)));
    }

    /// 組合せの写し: 初めは全部、後は変わった所だけ。窓は続かない写しを断る
    #[test]
    fn combo_copy_sends_edits_after_the_first() {
        let mut sync = CopySync::default();
        let mut recv = CopyRecv::default();
        let mut c = sample();
        let mut win = Combo::default();
        let v = sync.combo_for(7, 1, &c).unwrap();
        assert!(v["full"].is_object());
        assert!(recv.combo(&mut win, &v));
        assert_eq!(win, c);
        assert_eq!(sync.combo_for(7, 1, &c), None);
        c.stop_gap = 2.0;
        let v = sync.combo_for(7, 2, &c).unwrap();
        assert!(v["full"].is_null() && v["edit"].is_object());
        assert!(recv.combo(&mut win, &v));
        assert_eq!(win, c);
        // 版だけ進んだ: 空の書き換えで窓の版を合わせる
        let v = sync.combo_for(7, 3, &c).unwrap();
        assert!(recv.combo(&mut win, &v));
        // 間の写しが抜けた: 断る（全部を送り直してもらう）
        c.stop_gap = 4.0;
        let _lost = sync.combo_for(7, 4, &c).unwrap();
        c.stop_gap = 5.0;
        let v = sync.combo_for(7, 5, &c).unwrap();
        assert!(!recv.combo(&mut win, &v));
        sync.restart(7);
        let v = sync.combo_for(7, 5, &c).unwrap();
        assert!(recv.combo(&mut win, &v));
        assert_eq!(win, c);
    }

    /// 履歴の写しは中身が変わったときだけ送る（何秒前かは比べない）
    #[test]
    fn hist_copy_is_sent_on_change() {
        let mut sync = CopySync::default();
        let step = |t: &str, ago: f64| StepCopy {
            title: t.into(),
            details: vec![],
            ago,
            doc: false,
        };
        let h = HistCopy {
            undo: vec![step("a", 1.0)],
            redo: vec![],
            can: [true, false],
        };
        assert!(sync.hist_for(1, h.clone()).is_some());
        let mut later = h.clone();
        later.undo[0].ago = 9.0;
        assert!(sync.hist_for(1, later).is_none());
        let mut more = h.clone();
        more.undo.insert(0, step("b", 0.0));
        assert!(sync.hist_for(1, more.clone()).is_some());
        let got = web_time::Instant::now();
        let s = more.steps(true, got);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].title, "a");
        assert!(s[1].at <= got);
    }
}
