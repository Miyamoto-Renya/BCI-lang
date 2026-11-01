//! 표준 BCi 실행 번들의 CLI 진입점.

fn main() -> std::process::ExitCode {
    bci_cli::main(&bci_extension::Extensions::empty())
}
