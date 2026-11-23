- **細かい直し（レビューの残り）**: `beams` 解析の同じ注意（ガラスなど）を光束の数だけ出さず 1 つに。`coatings` にプロジェクト・系ファイルを
  渡したとき `catalogs` にガラスのカタログ（`glass_catalogs`・系ファイルの `catalogs`）を出す。`--glass-catalog SCHOTT,OHARA` をカンマで
  分ける。熱の解析（Make Thermal）の注意の「surface 7 t」を「thickness solve of surface 7」に。大域探索の中断で止まった始点を失敗に
  数えない（`skipped` に入れる）。GUI のガラスの置き換えの反映が途中で当てられなくなったとき、書きかけの状態の上書きを戻す。
  多重構成の .zmx から光束を読む `zmx-beams` で、戻りの面の値が往路と違うときの誤りの文を分かりやすく。像面の位置が
  取れないときは照合を飛ばさず誤りに
