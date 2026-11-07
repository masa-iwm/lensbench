# lensbench

カタログレンズを組み合わせた光学系の評価・最適化のための、順次光線追跡ツール（開発中）。
A sequential ray-tracing tool for evaluating and optimizing systems built from stock lenses (work in progress).

- Windows の GUI（インストール不要の単一 exe）と、Linux・Windows の CLI
- .zmx の読込（面のまま、または玉ごと）、ガラスカタログ（AGF 形式）
- スポット、波面・Zernike、MTF、干渉（低コヒーレンス含む）、SH 波面計測の模擬、公差モンテカルロ、最適化
- 計算結果は OpticStudio の出力と数値で照合している（照合に使った基準データは `tests/` に置く）

Zemax および OpticStudio は Ansys, Inc. の商標です。lensbench は Ansys とは関係がなく、その製品の互換品ではありません。

## 構成

| ディレクトリ | 中身 |
|---|---|
| `crates/lensbench-core` | 光線追跡・解析のライブラリ |
| `crates/lensbench-cli` | コマンドライン（`lensbench`） |
| `scripts/check_public.py` | 公開できないファイル（メーカーのカタログなど）が入っていないかの検査。CI で全履歴に対して実行 |

メーカーのレンズ・ガラス・膜のカタログはこのリポジトリに含めない。利用者が手元のファイルを指定して読む。

## ビルドとテスト

```
cargo build --release
cargo test --workspace --release
python3 scripts/check_public.py --history
```

## 使い方（CLI）

視野・波長・コンフィグの番号は 1 始まり。`--json` で JSON 出力。

```
lensbench info     system.zmx --glass-dir <AGF のフォルダ>
lensbench spot     system.zmx --field 2 --wave 1
lensbench wavefront system.zmx --n 64
lensbench zernike  system.zmx --kind fringe --terms 37
lensbench coatings system.zmx --table coat_tables.ctab
```

`coatings` は全コンフィグの境界（`膜|前の媒質|後の媒質`）と膜の扱い（膜なし、理想膜、膜ファイルの薄膜・表、実測表、モデル）を一覧にし、
偏光追跡に実測表かモデルが要る境界（暗号化された膜など）を `needs_table` に挙げる。

膜ファイル（COATING.DAT 形式）の多層膜は、透過・鏡とも OpticStudio と同じ約束で計算する（層は膜の外側から順、
後ろの媒質が空気なら基板は前の媒質、鏡の基板は最後の層の材料、係数に掛かる位相。`lensbench_core::coating` の
`Coatings::prepare` に説明）。膜厚が波長単位の層は系の主波長で厚さを決める。

### 膜の実測表（LBCT 形式）

暗号化された膜（.ZEC）は復号せず、OpticStudio の偏光追跡で測った境界の複素 s・p 係数の表を使う。
テキストで、`LBCT 1` の行のあとに `KEY 膜|前の媒質|後の媒質`、`WAVE 波長[µm]`、`入射角[deg] Re s Im s Re p Im p` の行が続く
（詳しくは `lensbench_core::coating::parse_measured`）。ライブラリでは `Coatings::load_measured` で読み、
`System::try_trace_pol` に渡す。

### 膜のモデル（LBCM 形式）

実測表は境界（膜と前後の媒質）ごとに要る。膜のモデルは膜ごとの特性行列（波長・β = n sinθ・偏光ごと）を持ち、
任意の硝材・向き（空気から・硝材から・硝材どうし）・波長（範囲内）・鏡の入射側の媒質で係数を出す。
`LBCM 2` の行のあとに `COAT 名前`、`D`・`BMAX`、`WAVE` と β ごとの行、鏡なら `MIRROR` の節が続く
（詳しくは `lensbench_core::coating::parse_models`）。`--table` と `Coatings::load_file` は 1 行目で LBCT と LBCM を見分ける。
同じ境界に実測表があれば実測表を使う。

透過の位相は既定で OpticStudio と同じ約束にする。その位相には層ごとの項（光線の層の中での横ずれの位相）が含まれ、
β が層の屈折率に近づくと発散するので、モデルは `BMAX` を超える β では止める。`Coatings::phase` を
`PhaseConvention::Physical` にするとこの項を外す（どの β でも有限。OpticStudio とは s・p 共通の位相だけ違う）。
膜ファイルの表（TABLE）の垂直入射は、OpticStudio と同じく s にも p の値を使う。

ガラスカタログは `--glass-dir`、環境変数 `LENSBENCH_GLASSCAT`（パス区切りで複数）、.zmx と同じフォルダの順に探す。

## テスト用データ

- `tests/glass/`: refractiveindex.info のデータベース（CC0）から `scripts/make_test_glass.py` で作った AGF。メーカー配布のカタログではない
- `tests/refs/`: lensbench 独自の処方のテスト光学系（トリプレット、傾き偏心、理想レンズ、非球面など）について OpticStudio で求めた基準値

## ライセンス

MIT または Apache-2.0 のどちらか（`LICENSE-MIT`、`LICENSE-APACHE`）。
