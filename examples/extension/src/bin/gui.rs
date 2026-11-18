//! 拡張を足した GUI

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    lensbench_sdk::gui::main(lensbench_extension_example::extensions())
}
