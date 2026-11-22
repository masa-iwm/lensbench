//! 操作の記録と流し直し: 文書（組合せ・開いた系）に起きた変更を順に覚えてファイルに書き、別の時・別の窓でその順に
//! 当て直す。ファイルは版つき（[`VERSION`]）で、読むときは今の版以下を読み、新しい版は理由を出して読まない。
//! 記録を始めたときの組合せも持ち、流し直す前に今の組合せをそこへ戻す（その戻しも 1 段の書き換え）。
//!
//! 組合せは、頼んだ操作でなく組合せに起きた変更を書き換え（[`Rec::Edit`]）として残す。欄の書き換え・元に戻す・
//! 差し替え探索の挿入・最適化の反映・像面を焦点へのように、返事を待ってから当たる変更も、次の操作の前（と記録を
//! やめるとき）に変わった所として残るので、流し直しでも順が入れ替わらない。組合せを変えなかった操作のうち、開いた系
//! （組合せでない）の編集・元に戻す・構成の切り替えと、見るパラメータ・系を作るは操作として残す。パネルの出し入れ・
//! 見た目・ファイルの窓・計算の実行は記録しない

use super::*;

/// 記録のファイルの種類の名前
pub(super) const FORMAT: &str = "lensbench-gui-ops";
/// 記録のファイルの今の版。操作の形を変えたら上げ、古い版を読む口を [`Record::read`] に足す
pub(super) const VERSION: u32 = 1;

/// 記録した 1 つの操作（ファイルの `ops` の 1 行。`op` で種類を分ける）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(super) enum Rec {
    /// 組合せに起きた変更（[`super::edit::Edit`]）
    Edit {
        ops: Vec<super::edit::Op>,
    },
    /// 元に戻す（back）・やり直す（組合せを変えなかったときだけ）
    Undo {
        back: bool,
        steps: usize,
    },
    /// 見るパラメータの値を変える・足す・外す
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
    /// 多重構成の次・前（back）の構成（組合せを変えなかったときだけ）
    CycleConfig {
        back: bool,
    },
    /// 今の組合せで系を作る
    Build,
    /// 面の表の編集（組合せを変えなかったとき、つまり開いた系だけ）
    LensData {
        act: lens_data::Act,
    },
}

impl Rec {
    /// 組合せを変えなかったときに操作として残すなら、その記録（組合せの変更は [`Recording::note`] が残す）
    pub(super) fn from_msg(m: &Msg) -> Option<Rec> {
        use lens_data::Act as L;
        Some(match m {
            Msg::Undo(back, n) => Rec::Undo {
                back: *back,
                steps: *n,
            },
            Msg::SetParam(p, x) => Rec::SetParam {
                param: p.clone(),
                value: *x,
            },
            Msg::AddParam(p) => Rec::AddParam { param: p.clone() },
            Msg::RemoveParam(i) => Rec::RemoveParam { index: *i },
            Msg::CycleConfig(back) => Rec::CycleConfig { back: *back },
            Msg::Combo(combo::Act::Build) => Rec::Build,
            Msg::LensData(a) => match a {
                L::Edit { .. }
                | L::Lock(..)
                | L::Fields { .. }
                | L::Data { .. }
                | L::Ops(_)
                | L::DropOverride { .. }
                | L::ConfigRow { .. }
                | L::DropPick { .. }
                | L::MakeGroup { .. }
                | L::Ungroup(_) => Rec::LensData { act: a.clone() },
                // 選ぶ・読む・書き出す・断った理由・最適化の変数は文書を変えない
                L::Select(_)
                | L::OptVar(_)
                | L::ExportRange(_)
                | L::Refused(_)
                | L::NeedData(_)
                | L::NeedGlasses => return None,
            },
            // 組合せの変更（欄の書き換え・パネルの操作）は note が残す。ファイル・画面・計算の操作は文書を変えない。
            // Msg を足したらここでどちらかを決める（黙って記録から漏らさない）
            Msg::Edit(_)
            | Msg::Combo(_)
            | Msg::Opt(_)
            | Msg::Tol(_)
            | Msg::Swap(_)
            | Msg::Zoom(_)
            | Msg::Catalog(_)
            | Msg::New
            | Msg::Open
            | Msg::OpenFile(_)
            | Msg::Reload
            | Msg::RunAll
            | Msg::WriteZmx
            | Msg::MakeThermal
            | Msg::Save(_)
            | Msg::Add(_)
            | Msg::Focus(_)
            | Msg::FocusKind(_)
            | Msg::CyclePanes(_)
            | Msg::ToWindow(_)
            | Msg::Preset(_)
            | Msg::Layout(_)
            | Msg::SaveLayout
            | Msg::Theme(_)
            | Msg::Accent(_)
            | Msg::Lang(_)
            | Msg::Folders
            | Msg::Help
            | Msg::ToggleHelp
            | Msg::TogglePalette
            | Msg::RunOpt
            | Msg::RunTol
            | Msg::FocusImage
            | Msg::Analyze(_)
            | Msg::Record
            | Msg::Replay => return None,
            Msg::Replayed(_, m) => return Rec::from_msg(m),
        })
    }

    /// 当て直す操作
    pub(super) fn into_msg(self) -> Msg {
        match self {
            Rec::Edit { ops } => Msg::Edit(super::edit::Edit { ops }),
            Rec::Undo { back, steps } => Msg::Undo(back, steps),
            Rec::SetParam { param, value } => Msg::SetParam(param, value),
            Rec::AddParam { param } => Msg::AddParam(param),
            Rec::RemoveParam { index } => Msg::RemoveParam(index),
            Rec::CycleConfig { back } => Msg::CycleConfig(back),
            Rec::Build => Msg::Combo(combo::Act::Build),
            Rec::LensData { act } => Msg::LensData(act),
        }
    }
}

/// 記録のファイル
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Record {
    pub format: String,
    pub version: u32,
    /// 記録を始めたときの組合せ（組合せでない系を開いていたときは None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Value>,
    pub ops: Vec<Rec>,
}

impl Record {
    /// 今の組合せから始める
    pub(super) fn new(start: &Combo) -> Record {
        Record {
            format: FORMAT.into(),
            version: VERSION,
            start: (!start.picks.is_empty())
                .then(|| serde_json::to_value(start).ok())
                .flatten(),
            ops: Vec::new(),
        }
    }

    /// ファイルの中身を読む（今の版以下）
    pub(super) fn read(text: &str) -> Result<Record, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v["format"] != json!(FORMAT) {
            return Err(tr("This is not a file of recorded operations").into());
        }
        let ver = v["version"].as_u64().unwrap_or(0);
        if ver == 0 || ver > VERSION as u64 {
            return Err(trf(
                "Recorded operations of version {} can't be read (this build reads up to version {})",
                &[&ver.to_string(), &VERSION.to_string()],
            ));
        }
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    /// ファイルに書く中身
    pub(super) fn text(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// 流し直している記録
pub(super) struct Replaying {
    total: usize,
    /// 最初の操作の前の元に戻す段の数
    base: Option<usize>,
    /// 当たらなかった操作があって、残りを当てない
    stopped: bool,
}

/// 記録中の様子: 書くファイルと、最後に残した時の組合せ
pub(super) struct Recording {
    pub file: Record,
    last: Combo,
}

impl Recording {
    fn new(c: &Combo) -> Recording {
        Recording {
            file: Record::new(c),
            last: c.clone(),
        }
    }

    /// 最後に残した時から組合せが変わっていれば、変わった所を残す（変わったら true）
    pub(super) fn note(&mut self, c: &Combo) -> bool {
        let Some(e) = super::edit::Edit::between(&self.last, c) else {
            return false;
        };
        self.file.ops.push(Rec::Edit { ops: e.ops });
        self.last = c.clone();
        true
    }
}

impl Core {
    /// 記録を始める（記録中なら、やめて保存先を選ぶ）
    pub(super) fn toggle_record(&mut self) {
        match self.rec.take() {
            None => {
                self.rec = Some(Recording::new(&self.doc.combo));
                self.info(tr(
                    "Recording operations (run the command again to stop and save)",
                ));
            }
            Some(mut r) => {
                r.note(&self.doc.combo);
                let n = r.file.ops.len();
                let text = r.file.text();
                let d = crate::files::FileDialog::new()
                    .add_filter(tr("Recorded operations"), &["json"])
                    .set_file_name("operations.json");
                self.save_as(d, move |c, p| match lensbench_core::vfs::write(&p, text) {
                    Ok(()) => c.info(trf(
                        "Saved {} recorded operations to {}",
                        &[&n.to_string(), &p.display().to_string()],
                    )),
                    Err(e) => c.fail(format!("{}: {e}", p.display())),
                });
            }
        }
    }

    /// 流し直すファイルを選ぶ窓を出す
    pub(super) fn pick_replay(&mut self) {
        self.dialog(
            "replay-ops",
            crate::files::FileDialog::new().add_filter(tr("Recorded operations"), &["json"]),
        );
    }

    /// 流し直すファイルを選び終わっていれば流し直す（毎フレーム）
    pub(super) fn poll_replay(&mut self) {
        if let Some(p) = crate::files::open("replay-ops", false, crate::files::FileDialog::new) {
            self.replay_file(&p);
        }
    }

    /// ファイルの操作を流し直す
    pub(super) fn replay_file(&mut self, p: &Path) {
        let r = lensbench_core::vfs::read(p)
            .map_err(|e| e.to_string())
            .and_then(|b| Record::read(&String::from_utf8_lossy(&b)));
        match r {
            Ok(r) => self.replay(r),
            Err(e) => self.fail(format!("{}: {e}", p.display())),
        }
    }

    /// 記録の操作を順に当てる（操作は順番待ちに入れ、パネルを描いた後に当てる）
    pub(super) fn replay(&mut self, r: Record) {
        if let Some(start) = r.start {
            match serde_json::from_value::<Combo>(start) {
                Ok(c) => {
                    if let Some(e) = super::edit::Edit::between(&self.doc.combo, &c) {
                        self.msgs.push(Msg::Edit(e));
                    }
                }
                Err(e) => {
                    self.fail(format!("{}: {e}", tr("Recorded operations")));
                    return;
                }
            }
        }
        let n = r.ops.len();
        self.replay = Some(Replaying {
            total: n,
            base: None,
            stopped: false,
        });
        self.msgs.extend(
            r.ops
                .into_iter()
                .enumerate()
                .map(|(i, o)| Msg::Replayed(i + 1, Box::new(o.into_msg()))),
        );
        self.info(trf("Replaying {} recorded operations", &[&n.to_string()]));
    }

    /// 流し直す操作 i を当ててよいか（前の操作が当たらなかったら当てない。元に戻すは記録を始めた所より前へは戻さない）
    pub(super) fn replay_ok(&mut self, i: usize, m: &Msg) -> bool {
        let depth = self.combo_hist.list(true).len();
        let Some(r) = self.replay.as_mut() else {
            return false;
        };
        if r.stopped {
            return false;
        }
        // 記録を始めた所（最初の操作の前）の元に戻す段の数
        let base = *r.base.get_or_insert(depth);
        if let Msg::Undo(true, n) = m {
            if depth < base + (*n).max(1) {
                let total = r.total;
                r.stopped = true;
                self.fail(trf(
                    "Replay stopped at operation {} of {}: it undoes changes made before the recording started",
                    &[&i.to_string(), &total.to_string()],
                ));
                return false;
            }
        }
        true
    }

    /// 流し直す操作 i を当てた後: 誤りが出ていたら（fails が増えた）残りを当てない
    pub(super) fn replayed(&mut self, i: usize, fails: u64) {
        let failed = self.fails > fails;
        let Some(r) = self.replay.as_mut() else {
            return;
        };
        let total = r.total;
        if failed {
            r.stopped = true;
            let why = self.status.text.clone();
            self.fail(trf(
                "Replay stopped at operation {} of {}: {}",
                &[&i.to_string(), &total.to_string(), &why],
            ));
        } else if i == total {
            self.replay = None;
            self.info(trf(
                "Replayed {} recorded operations",
                &[&total.to_string()],
            ));
        }
    }
}

impl App {
    /// 記録中なら、操作を当てる前の変更（返事を待って当たった変更など）を残す
    pub(super) fn record_before(&mut self) {
        let combo = &self.c.doc.combo;
        if let Some(r) = self.c.rec.as_mut() {
            r.note(combo);
        }
    }

    /// 当てた操作を記録する（記録中だけ）: 組合せが変わったら変わった所を、変わらなければ操作を残す
    pub(super) fn record(&mut self, rec: Option<Rec>) {
        let combo = &self.c.doc.combo;
        let Some(r) = self.c.rec.as_mut() else {
            return;
        };
        if !r.note(combo) {
            r.file.ops.extend(rec);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 記録のファイルは書いて読むと同じ。面の表の編集も文字で残る
    #[test]
    fn record_round_trips_through_text() {
        let mut r = Record::new(&Combo::default());
        r.ops = vec![
            Rec::SetParam {
                param: "1.t".into(),
                value: 2.5,
            },
            Rec::LensData {
                act: lens_data::Act::Edit {
                    j: 2,
                    col: lens_data::Col::Par(3),
                    value: json!(0.1),
                },
            },
            Rec::Undo {
                back: true,
                steps: 1,
            },
            Rec::Build,
        ];
        let text = r.text();
        assert!(text.contains("\"op\": \"set_param\""), "{text}");
        assert!(text.contains("\"par\": 3"), "{text}");
        assert_eq!(Record::read(&text).unwrap(), r);
    }

    /// 新しい版・ほかの種類のファイルは読まない
    #[test]
    fn newer_versions_and_other_files_are_refused() {
        let v = json!({"format": FORMAT, "version": VERSION + 1, "ops": []});
        let e = Record::read(&v.to_string()).unwrap_err();
        assert!(e.contains(&(VERSION + 1).to_string()), "{e}");
        assert!(Record::read(&json!({"ops": []}).to_string()).is_err());
        let v = json!({"format": FORMAT, "version": VERSION, "ops": [{"op": "build"}]});
        assert_eq!(Record::read(&v.to_string()).unwrap().ops, vec![Rec::Build]);
    }

    /// 版 1 のファイルはこの先も読む: 書いた文字のまま固定する（形を変えたら版を上げ、ここは残す）
    #[test]
    fn version_1_files_keep_reading() {
        let text = r#"{
  "format": "lensbench-gui-ops",
  "version": 1,
  "start": {"picks": []},
  "ops": [
    {"op": "edit", "ops": [{"path": ["picks", 0, "air"], "value": 12.5}, {"path": ["note"]}]},
    {"op": "undo", "back": true, "steps": 2},
    {"op": "set_param", "param": "1.t", "value": 2.5},
    {"op": "add_param", "param": "EFFL"},
    {"op": "remove_param", "index": 0},
    {"op": "cycle_config", "back": false},
    {"op": "build"},
    {"op": "lens_data", "act": {"edit": {"j": 2, "col": "r", "value": 50.0}}},
    {"op": "lens_data", "act": {"edit": {"j": 2, "col": {"par": 3}, "value": 0.1}}},
    {"op": "lens_data", "act": {"lock": ["L1", true]}},
    {"op": "lens_data", "act": {"fields": {"j": 1, "fields": {"comment": "a"}}}},
    {"op": "lens_data", "act": {"data": {"j": 1, "data": {"decenter_x": 0.1}}}},
    {"op": "lens_data", "act": {"ops": [{"op": "set_stop", "surface": 3}]}},
    {"op": "lens_data", "act": {"drop_override": {"j": 4, "col": "conic"}}}
  ]
}"#;
        let r = Record::read(text).expect("version 1");
        use super::edit::Seg;
        let ld = |act| Rec::LensData { act };
        use lens_data::{Act, Col};
        let mut fields = Map::new();
        fields.insert("comment".into(), json!("a"));
        let mut data = Map::new();
        data.insert("decenter_x".into(), json!(0.1));
        assert_eq!(r.start, Some(json!({"picks": []})));
        assert_eq!(
            r.ops,
            vec![
                Rec::Edit {
                    ops: vec![
                        super::edit::Op {
                            path: vec![
                                Seg::Key("picks".into()),
                                Seg::Index(0),
                                Seg::Key("air".into())
                            ],
                            value: Some(json!(12.5)),
                        },
                        super::edit::Op {
                            path: vec![Seg::Key("note".into())],
                            value: None,
                        },
                    ],
                },
                Rec::Undo {
                    back: true,
                    steps: 2
                },
                Rec::SetParam {
                    param: "1.t".into(),
                    value: 2.5
                },
                Rec::AddParam {
                    param: "EFFL".into()
                },
                Rec::RemoveParam { index: 0 },
                Rec::CycleConfig { back: false },
                Rec::Build,
                ld(Act::Edit {
                    j: 2,
                    col: Col::R,
                    value: json!(50.0)
                }),
                ld(Act::Edit {
                    j: 2,
                    col: Col::Par(3),
                    value: json!(0.1)
                }),
                ld(Act::Lock("L1".into(), true)),
                ld(Act::Fields { j: 1, fields }),
                ld(Act::Data { j: 1, data }),
                ld(Act::Ops(vec![json!({"op": "set_stop", "surface": 3})])),
                ld(Act::DropOverride {
                    j: 4,
                    col: Col::Conic
                }),
            ]
        );
    }

    /// 文書を変えない操作は記録しない
    #[test]
    fn only_document_changes_are_recorded() {
        assert!(Rec::from_msg(&Msg::RunAll).is_none());
        assert!(Rec::from_msg(&Msg::LensData(lens_data::Act::NeedGlasses)).is_none());
        let m = Msg::SetParam("2.r".into(), 10.0);
        let r = Rec::from_msg(&m).unwrap();
        assert!(matches!(r.into_msg(), Msg::SetParam(p, x) if p == "2.r" && x == 10.0));
    }
}
