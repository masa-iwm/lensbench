# .github/scripts

## project_status.py

Projects ボード（https://github.com/users/masa-iwm/projects/2）の Status を Issue・PR に合わせる（`.github/workflows/project-status.yml` から呼ぶ）。

`in-progress` ラベルは、開いている PR の本文に `Closes/Fixes/Resolves #N` があればその Issue に自動で付く（PR のイベントと毎日の一括）。
PR を出す前の作業（調査・PC での照合など）は自動では分からないので、着手したスレッドがラベルを付ける。
`Refs #N` は Status だけ In Progress にし、ラベルは付けない（まとめの Issue に残り続けないように）。
マージせずに閉じた PR が付けたラベル（PR を開いた後に付いたもの）は、ほかに開いている PR が閉じる参照をしていなければ外して Todo に戻す。PR より前に手で付けたラベルは残す。

### token（Secret `ADD_TO_PROJECT_PAT`）の作り直し

公開・非公開の両リポジトリで同じ token を使う。期限が切れると Status が更新されなくなる（ワークフローは失敗する）。

1. GitHub の Settings → Developer settings → Personal access tokens → **Tokens (classic)** → Generate new token (classic)。
   - Note: `lensbench project-status`
   - Expiration: 90 days など期限を付ける（期限の 1 週間前に GitHub からメールが来る）
   - Scopes: `repo`、`project`
2. 両リポジトリの Settings → Secrets and variables → Actions で `ADD_TO_PROJECT_PAT` を新しい値に更新する。
3. Actions → project-status → Run workflow（`dry_run` にチェック）で、エラーなくボードを読めることを確かめる。
4. 古い token を削除する。

ユーザー所有の Projects は fine-grained token に対応していないので classic を使う。

## apt_install.sh

CI で apt の包みを入れる（`ci.yml` の gui-tour、`pages.yml`）。

- 入っていない包みだけ入れる。
- 前に入れた deb を `~/.cache/apt-debs`（actions/cache、像の版ごとの鍵）に保存し、あればネットを使わずにそこから入れる。11/11 は Ubuntu のミラーの名前が azure.archive・archive とも 2 時間以上引けなかったため。
- 取りに行くときは、apt の再試行を少なめにし、update は 4 分、deb を落とす段は 5 分で打ち切る。2 回まで試し、2 回目はランナー既定のミラー（azure.archive.ubuntu.com）をやめて archive.ubuntu.com を直に使う。ステップ全体も `timeout-minutes: 20` で打ち切る。

web ジョブの Playwright は `--with-deps` を使わない。Chromium の共有ライブラリはランナーの像に入っており、apt の障害で落ちないようにするため。
