- **値の無い硝材を除く設定とガラス置換のテンプレート**: ガラスの置き換えの絞り込みに `exclude_incomplete`（CLI `--glass-exclude-incomplete`、
  GUI のチェック）。上限を入れた性質に値の無い硝材を除く（OpticStudio の Exclude Glasses With Incomplete Data。相対価格・CR・SR・AR・PR の
  0 も値なし）。.zmx の GSTD（ガラス置換のテンプレート）を読み書きし、システム・プロジェクトの `glass_template` に持つ。絞り込みの欄を
  何も書かなければ、使う設定のテンプレートで絞る。どれも OpticStudio 2026 R1.01 の実測に合わせた
