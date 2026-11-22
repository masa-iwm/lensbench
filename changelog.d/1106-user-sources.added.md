- **利用者定義の光源**（拡張の口）: 拡張のクレートが設定から光線の始点・方向・重みを作る光源を名前で足せる（`Extensions::source_model`、
  SDK の `lensbench_sdk::source`）。Python の `source_models()`・`source_rays(system, name, config)` で光線を受け取り、そのまま `trace_rays` に渡せる。
  拡張の雛形に点光源 `example_point` を足した
