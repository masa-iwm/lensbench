// 大きな配列を追跡ごとに確保し直しても OS からページを取り直さないように
#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> std::process::ExitCode {
    lensbench_cli::main()
}
