// リリースでは Windows でコンソールの窓を出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// 大きな配列を追跡ごとに確保し直しても OS からページを取り直さないように
#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    lensbench_gui::main()
}

// Web 版の入口は lensbench-web
#[cfg(target_arch = "wasm32")]
fn main() {}
