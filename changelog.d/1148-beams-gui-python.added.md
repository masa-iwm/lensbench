- **GUI の光束の組**: 組合せの道具の列の「光束」で、プロジェクトの光束（`beams`）の一覧を見て、足す・消す・名前・BS の経路・状態・役割・
  折り返す面（`turn`）・off を変えられる（保存・読み込みはプロジェクトの `beams` のまま往復）。レイアウト図の「重ねる構成」で光束を選んで重ねて描ける
- **Python で光束を扱う**: `System.beams(path)`（光束の一覧）・`System.load_beams(path, beams=None, all=False)`（光束の名前 → System の dict、
  `load(path, state=名前)` と同じ）・`System.beam_table(path, ...)`（光束の表、CLI の `beams` と同じ）。Python API 2.10
