# 照合試験（verify）

プラグイン・解析部品を、別の実装（従来の実装・OpticStudio・Python の試作）が作った基準データと比べる。照合 1 件を
`*.verify.json` に書き、CI では `--ci` で回す（`ci: false` の照合は手元でだけ）。通らなければ終了コード 1。

```sh
lensbench verify tests/verify            # フォルダの下の *.verify.json を全部
lensbench verify --ci --json refs/        # CI 用（ci: false を飛ばす）、結果を JSON で
```

```json
{
  "format": "lensbench-verify", "version": 1, "name": "…",
  "source": {"by": "OpticStudio 26.1", "commit": "…", "how": "作ったスクリプトと引数", "date": "2026-11-09"},
  "ci": true,
  "system": {"path": "system.zmx", "cfg": 1, "glass_dirs": ["glass"]},
  "plugin": "zernike", "config": {"terms": 37},
  "threads": [1, 4],
  "checks": {"coef": {"abs": 2e-8, "note": "基準の表示桁"}, "jones": {"rel": 1e-9}, "ok": {"exact": true}},
  "runs": [{"config": {"wave": 1}, "inputs": {"field": 1}, "expected": {"coef": [0.5, 0.0]}},
           {"inputs": "in.npz", "expected": "out.npz"}]
}
```

- 許容差は量ごとに `abs`（差の絶対値）・`rel`（基準の配列の最大の絶対値に対する差）・`exact`（ビット同じ）。両方書けばどちらかで通る。
  緩めた理由は `note` に。基準（`expected`）の名前には全部 `checks` が要る（書き忘れを通さない）。プラグインの出力で基準にない名前は比べない
- 値でない要素（NaN = けられた光線など）は、両方が値でないときだけ一致。片方だけなら許容差によらず不一致。bool は全要素の一致
- `threads` を 2 つ以上書くと、各スレッド数の出力（全部の名前）がビット同じかも確かめる
- 基準を作った側の版と作り方を `source` に残す（古い基準と今のコードの差を、置き換えの差と取り違えないため）
- 入力・基準は npz（float64・complex128・bool）か JSON に直接（数・入れ子の並び・真偽、`{"re", "im"}` は complex、null は NaN）。
  パスは記述のファイルからの相対。`runs` の `config` は上の `config` に重ねる。系は .zmx か系ファイル（.json）
- 登録のプラグイン（同梱・非公開版の exe が足した分）で回す。Python なら `lb.verify(path)`（`register_plugin` で足した分も）、
  比べ方だけなら `lb.verify_compare(actual, expected, checks)`。Rust の試験からは `lensbench_verify::assert_dir`
- 例題は `tests/verify`（OpticStudio の Zernike 係数。`tests/verify/make_samples.py` で作る）
