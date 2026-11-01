//! 사용자 함수를 표준 CLI에 연결하는 정적 번들 예제.

mod lab_signal;
mod lab_state;

fn main() -> std::process::ExitCode {
    match bci_extension::Extensions::new(vec![lab_signal::rectify_registration()])
        .and_then(|r| r.with_stateful(vec![lab_state::RunningMean::bci_registration()]))
    {
        Ok(extensions) => bci_cli::main(&extensions),
        Err(error) => {
            eprintln!("E0510: {error}");
            std::process::ExitCode::from(2)
        }
    }
}
