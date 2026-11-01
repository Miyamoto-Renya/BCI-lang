use bci_extension::operator;

/// 전압의 음수 성분을 0으로 만드는 사용자 함수.
#[operator(
    name = "lab.signal.rectify",
    version = "0.1.0",
    kind = "pointwise",
    contract = "rectify.operator.toml"
)]
pub fn rectify(value: f64) -> f64 {
    if value < 0.0 { 0.0 } else { value }
}
