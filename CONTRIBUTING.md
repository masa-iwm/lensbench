# コードの書き方の約束

このリポジトリは Pull Request を受け付けていない（README）。ここに書くのは、コードを読む人・Issue を書く人のための、
今のコードが従っている約束。

## 言語

- 識別子（型・関数・変数・JSON の鍵・CLI のオプション）は英語。略さずに書く（`object_distance`、`paraxial_image_height`）。
- コメント・doc コメント・ドキュメント（`docs/`）は日本語。
- 画面（GUI）・誤り・CLI の説明の文は英語を元にし、日本語の訳を表に付ける。GUI は `tr("…")` の英語が鍵で訳は
  `crates/lensbench-gui/lang/ja.tsv`、誤りの文の訳は `crates/lensbench-core/lang/ja.tsv`。
- 日本語の訳語は OpticStudio の日本語版などで広く使われる言い回し（カタカナ語を含む）に揃え、どの表でも同じ語にする。
  用語集は `crates/lensbench-core/lang/glossary.tsv`（使わない語 → 使う語）。状態・誤りの文の文末は「〜ません」の形。

## 名前

- .zmx の 4 文字の略語（`ropd`・`glrs`・`enpp`・`expp`・`mema` など）は .zmx の読み書き（`lensbench_core::zmx`・`zmx_write`）と、
  OpticStudio の評価関数の項の名前をそのまま使う所（最適化の項）の中だけで使う。そこから外へ出す名前は略さない英語にする。
- 同じ概念は層（core・プロジェクトファイル・GUI・Python）が違っても同じ英語の名前にする。

## コメントに書くこと

- Issue・Pull Request の番号はコメントに書かない。なぜそうしたか（OpticStudio での実測の値、一致の桁、捨てた案）を文で書く。
  番号はリポジトリの作り直しで変わることがあり、読み手が引けなくなる。
- OpticStudio との照合の根拠は、使った版（例: OpticStudio 26.1）と一致の桁を書く。照合の基準が公開できないもの
  （メーカーのデータを含むもの）でも、数値はここに書いてよい。

## 番号・単位

利用者から見える番号（視野・波長は 1 始まり、面は 0 始まり）と単位は [番号・単位・省いたときの約束](docs/conventions.md)。
