//! 拡張を足した CLI（公開の lensbench と同じ使い方に、example-chief などが増える）

fn main() -> std::process::ExitCode {
    lensbench_sdk::cli::main(lensbench_extension_example::extensions())
}
