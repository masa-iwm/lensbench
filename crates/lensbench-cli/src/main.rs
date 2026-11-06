fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("--version") | Some("-V") => println!("lensbench {}", lensbench_core::VERSION),
        _ => {
            eprintln!("lensbench {}: 開発中です", lensbench_core::VERSION);
            std::process::exit(2);
        }
    }
}
