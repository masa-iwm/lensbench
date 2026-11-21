//! ガラスカタログの既定のフォルダ: LENSBENCH_GLASSCAT と、exe の隣の `catalogs/glass`

/// 起動時の環境（[`lensbench_core::init_from_env`]）を読み、exe の隣の `catalogs/glass` を既定のフォルダに足す
pub(crate) fn init_from_env() {
    lensbench_core::init_from_env();
    #[cfg(not(target_arch = "wasm32"))]
    add_bundled_glass_dir(std::env::current_exe().ok().as_deref());
}

/// exe の隣の `catalogs/glass`（GUI の既定のカタログと同じ配置）があれば、`--glass-dir`・LENSBENCH_GLASSCAT の
/// 後に探す既定のガラスのフォルダに足す（カタログを exe の隣に置いた配置なら、CLI も同梱の屈折率だけのカタログでなくそこのカタログを使う）
#[cfg(not(target_arch = "wasm32"))]
fn add_bundled_glass_dir(exe: Option<&std::path::Path>) {
    let Some(d) = exe
        .and_then(std::path::Path::parent)
        .map(|d| d.join("catalogs").join("glass"))
        .filter(|d| d.is_dir())
    else {
        return;
    };
    let mut v = lensbench_core::glass::default_dirs();
    if !v.contains(&d) {
        v.push(d);
        lensbench_core::glass::set_default_dirs(v);
    }
}
