- **係数を計算で返す膜**（拡張）: `lensbench_core::usercoating::CoatingModel`（SDK の `lensbench_sdk::coating`）を
  `CoatingSet::add_user` で膜の一覧に入れると、面の膜の名前で引ける。波長・前後の屈折率・入射角・透過/鏡/屈折面での反射を受けて
  rs・rp・ts・tp を返し、偏光追跡・ghost・干渉など膜を使う所は同梱の膜と同じ経路を通る。`check_coating_model` で約束を確かめる。
  CLI の `coatings` の種類は `user`。雛形に例 `EXAMPLE_ND50`
