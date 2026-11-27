- **光線の重み（非順次の追跡の下地）**: 利用者定義の光源の重みを追跡の結果まで通す（Rust の `System::trace_source`、`Trace::w`。
  重みの無い追跡の結果は変わらない）。光線を画素に数える検出器（`irradiance::detect`、重みの和・照度 W/mm²）と重み付きのスポットの
  重心・RMS 半径（`spot_moments`）、光源の全パワーを重みで分ける `SourceRays::powers`、光線の番号から決まるカウンタ式の乱数
  （`rng::CounterRng`）、出来事と経路の型（`raypath`、光束の turn・ゴーストの面 a・b を経路に直す）を足した
