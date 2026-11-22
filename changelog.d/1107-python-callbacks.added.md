- **Python の関数を光源・膜・形に**（試作用。Python のプロセスの中だけで、CLI・GUI からは使えない）: `register_source(name, func)`（波長ごとに 1 回、
  配列で返す）、`register_coating_catalog(name, models={膜の名前: func})`（係数を計算で返す膜。波長・屈折率・経路・向きの組ごとに cos1 の
  2001 点で 1 回呼び、線形に補間）、`register_shape(name, func, params)`（USERSHAPE の形。形の準備ごとに格子で 1 回呼び、双 3 次 Hermite で引く）
