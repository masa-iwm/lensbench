# .github/scripts

## project_status.py

Projects ボード（https://github.com/users/masa-iwm/projects/2）の Status を Issue・PR に合わせる（`.github/workflows/project-status.yml` から呼ぶ）。

`in-progress` ラベルは、開いている PR の本文に `Closes/Fixes/Resolves #N` があればその Issue に自動で付く（PR のイベントと毎日の一括）。
PR を出す前の作業（調査・PC での照合など）は自動では分からないので、着手したスレッドがラベルを付ける。
`Refs #N` は Status だけ In Progress にし、ラベルは付けない（まとめの Issue に残り続けないように）。

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
