//! `openstance`: the CLI binary.

fn main() -> std::process::ExitCode {
    openstance::run(std::env::args_os())
}
