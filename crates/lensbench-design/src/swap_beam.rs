//! 所ごとのビーム探索（[`SwapSpec::beam`]）: 全組の総当たりの代わりに、所を 1 つずつ進める。
//!
//! 1. 選び方の並び（所ごとの候補の番号。まだ選ばない所はプロジェクトの玉のまま）を、最初はすべて選ばない 1 通りにする。
//! 2. 所 k について、残した各選び方の所 k を候補の全部に替えた選び方を最適化し、評価関数の小さい `width` 通りを残す。
//! 3. 所を最後まで進めたら 1 巡。残した選び方が巡回の前後で同じになるか、`passes` 巡で止める。
//!
//! 一度評価した選び方は覚えて評価し直さない。結果はすべての所を選んだ組（総当たりの組の番号 `i` と同じ組）だけで、
//! 各組の結果は総当たりのその組の結果と同じ。順位の同点は選び方の番号の順で、スレッド数によらない。

use super::*;
use lensbench_runs::Progress;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 所ごとのビーム探索の設定（[`SwapSpec::beam`]）
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Beam {
    /// 所ごとに残す選び方の数
    #[serde(default = "twenty")]
    pub width: usize,
    /// 所を一巡する回数の上限（残す選び方が巡回の前後で変わらなければその前に止める）
    #[serde(default = "two")]
    pub passes: usize,
}

fn twenty() -> usize {
    20
}

type Assign = Vec<Option<usize>>;

impl SwapSearch<'_> {
    /// ビーム探索で評価する組の数の上限の見込み（`passes` × `width` × 候補の数の和、あふれれば None）
    pub(super) fn beam_estimate(&self) -> Option<usize> {
        let b = self.spec.beam.as_ref()?;
        let sum = self
            .picks
            .iter()
            .try_fold(0usize, |a, v| a.checked_add(v.len()))?;
        sum.checked_mul(b.width)?.checked_mul(b.passes)
    }

    /// ビーム探索か（[`SwapSpec::beam`]）
    pub fn uses_beam(&self) -> bool {
        self.spec.beam.is_some()
    }

    /// 選び方（すべて Some）の組の番号（[`Self::combo`] の逆）
    fn index_of(&self, a: &[Option<usize>]) -> Option<usize> {
        let mut i = 0usize;
        for (k, c) in a.iter().enumerate() {
            i = i * self.picks[k].len() + (*c)?;
        }
        Some(i)
    }

    /// ビーム探索（[`SwapSpec::beam`] がなければ全組の総当たり [`Self::run`]）:
    /// (すべての所を選んだ組の結果（評価関数の小さい順）, 最適化した選び方の数, 止めたか, 結果が空のときの理由)。
    /// 進み具合は文脈の受け手（[`par::with_progress`]）へ段 `"main"` の [`par::Event::Count`] で知らせ、`total` は評価する数の
    /// 見込みの上限（[`Self::beam_estimate`]）。文脈の中断の印（[`par::with_cancel`]）が倒れたら新しい評価を始めない
    pub fn beam_run(
        &self,
        threads: Option<usize>,
    ) -> (Vec<SwapResult>, usize, bool, Option<String>) {
        let Some(b) = &self.spec.beam else {
            let v = self.run(threads);
            let n = v.len();
            return (v, n, false, None);
        };
        let total = self.beam_estimate().unwrap_or(usize::MAX);
        let t0 = web_time::Instant::now();
        let done = AtomicUsize::new(0);
        let stop = par::cancelled;
        let mut merit: HashMap<Assign, f64> = HashMap::new();
        let mut full: HashMap<usize, SwapResult> = HashMap::new();
        let mut beams: Vec<Assign> = vec![vec![None; self.picks.len()]];
        let mut stopped = false;
        let mut why = None;
        'passes: for _ in 0..b.passes {
            let before = beams.clone();
            for k in 0..self.picks.len() {
                let mut seen = HashSet::new();
                let mut stage: Vec<Assign> = Vec::new();
                for bm in &beams {
                    for c in 0..self.picks[k].len() {
                        let mut a = bm.clone();
                        a[k] = Some(c);
                        if seen.insert(a.clone()) {
                            stage.push(a);
                        }
                    }
                }
                let mut last_err: Option<String> = None;
                let todo: Vec<&Assign> = stage.iter().filter(|a| !merit.contains_key(*a)).collect();
                let out = par::with_threads(threads, || {
                    par::map(&todo, |a| {
                        if stop() {
                            return None;
                        }
                        let r = self.optimize(a);
                        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if par::reporting() {
                            let el = t0.elapsed().as_secs_f64();
                            par::report(&par::Event::Count(Progress {
                                stage: SwapStage::Main.name(),
                                done: d,
                                total,
                                ran: d,
                                elapsed: el,
                                eta: (d > 0)
                                    .then(|| el / d as f64 * total.saturating_sub(d) as f64),
                            }));
                        }
                        Some(r)
                    })
                });
                for (a, r) in todo.into_iter().zip(out) {
                    let Some(r) = r else {
                        stopped = true;
                        continue;
                    };
                    let m = match &r {
                        Ok(f) => f.merit.unwrap_or(f64::INFINITY),
                        Err(e) => {
                            last_err.get_or_insert_with(|| String::from(e.clone()));
                            f64::INFINITY
                        }
                    };
                    merit.insert(a.clone(), m);
                    if let Some(i) = self.index_of(a) {
                        full.insert(i, self.result(i, r));
                    }
                }
                if stopped {
                    break 'passes;
                }
                let mut ranked: Vec<(f64, Assign)> = stage
                    .into_iter()
                    .map(|a| (merit[&a], a))
                    .filter(|x| x.0.is_finite())
                    .collect();
                ranked.sort_by(|x, y| x.0.total_cmp(&y.0).then_with(|| x.1.cmp(&y.1)));
                if ranked.is_empty() {
                    // この所のどの候補も評価できない: 残す選び方がない
                    why = Some(format!(
                        "beam search: no candidate of place {} could be evaluated (first: {})",
                        self.spec.slots[k].block,
                        last_err.unwrap_or_else(|| "the merit function is not finite".into())
                    ));
                    break 'passes;
                }
                beams = ranked.into_iter().take(b.width).map(|x| x.1).collect();
            }
            if beams == before {
                break;
            }
        }
        let mut v: Vec<SwapResult> = full.into_values().collect();
        sort_results(&mut v);
        let why = why.filter(|_| v.is_empty());
        (v, merit.len(), stopped, why)
    }

    /// [`Self::run_to`] のビーム探索: 結果は out に一括実行の形で書く（ビーム探索は続きから回せないので、いつも書き直す）
    pub(super) fn beam_to(
        &self,
        out: Option<&std::path::Path>,
        threads: Option<usize>,
    ) -> lensbench_core::Result<Vec<SwapResult>> {
        let (v, _, _, why) = self.beam_run(threads);
        if let Some(e) = why {
            return Err(e.into());
        }
        if let Some(path) = out {
            let o = lensbench_runs::RunOpts {
                threads: Some(1),
                out: Some(path),
                header: self.header(),
                resume: false,
                ..Default::default()
            };
            // 書き出しは進み具合に数えない
            par::with_progress(None, || {
                lensbench_runs::run(v.len(), 0, &o, |j, _| {
                    serde_json::to_string(&v[j])
                        .map_err(|e| lensbench_core::Error::Invalid(e.to_string()))
                })
            })?;
        }
        Ok(v)
    }
}
