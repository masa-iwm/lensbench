- **像の模擬の中心の視野・回転・反転**: `image-sim` の `--field`（元の画像の中心を視野 N に置き、検出器をその主光線の像点に合わせる。
  OpticStudio の Field）、`--flip-source`・`--rotate`（元の画像の反転と反時計回りの回転）・`--flip-image`（検出器の像の反転）。
  Python の `image_sim(field=, flip_source=, rotate=, flip_image=)` も。中心の視野は OpticStudio 2026 R1.01 の像（D0 の視野 2・4）と照合した
