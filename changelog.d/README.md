# 変更の記録の断片

利用者に見える変更は、Pull Request ごとにこのフォルダへ `<名前>.<節>.md` を 1 つ足す（`CHANGELOG.md` は直接書き換えない）。
ほかの PR と同じファイルに触らないので、並行の PR が衝突しない。版上げの PR で `python3 scripts/changelog.py collect X.Y.Z` が
`CHANGELOG.md` の版の節へまとめ、断片を消す。

- 名前: 英小文字・数字・ハイフン。Issue の番号から始める（例 `1083-coating-notes.fixed.md`）。同じ節の中は名前の順に並ぶ
- 節: `breaking`（互換が切れる所）・`results`（結果が変わる所）・`changed`（変えたもの）・`added`（足したもの）・`perf`（速さ）・
  `fixed`（直した所）・`known`（既知の問題）
- 中身: `CHANGELOG.md` にそのまま入る「- 」で始まる項目（`- **短い題**: 説明`。長い行は折り返してよい）。見出しは書かない
- 前の PR の断片を直すときは、その断片のファイルを書き換える（まとめたあとの版は `CHANGELOG.md` を直す）

まとめた形は `python3 scripts/changelog.py preview`、形式の検査は `python3 scripts/changelog.py check`（CI の試験でも見る）。
