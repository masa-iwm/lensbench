# JSON スキーマ

lensbench が読み書きする JSON ファイルのスキーマです。ファイルの先頭に `"$schema"` を書くと、エディタの補完・検査が効きます
（[プロジェクトファイル](../project-file.md)）。

| スキーマ | ファイル |
|---|---|
| [project](../schema/project.schema.json) | プロジェクトファイル |
| [system](../schema/system.schema.json) | 系ファイル（`lensbench convert` の .json） |
| [user-catalog](../schema/user-catalog.schema.json) | ユーザーカタログ |
| [swap](../schema/swap.schema.json) | 差し替え探索の `--spec` |

serde の型から作っているので、型を変えたら `LENSBENCH_UPDATE_SCHEMA=1 cargo test -p lensbench-project --test schema` で作り直す。
