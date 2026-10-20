//! `ost`: the short alias for `openstance`. Same commands, same behaviour.

fn main() -> std::process::ExitCode {
    openstance::run(std::env::args_os())
}
