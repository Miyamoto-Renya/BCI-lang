//! 하나의 실제 예제로 타입·효과·위치 진단을 함께 검사한다.

use bci_compiler::compile;
use bci_schema::{
    number::Rational,
    types::{Mode, Occurrence, ValueType},
};
use std::path::PathBuf;

const EXAMPLE: &str = include_str!("../../examples/typed.bci");
fn base() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

#[test]
fn typed_pipeline_and_boundary_diagnostics() {
    let compiled = compile(EXAMPLE, &base()).expect("typed example");
    let graph = &compiled.graphs[0];
    assert_eq!(graph.nodes.len(), 7);
    assert_eq!(graph.effect.mode, Mode::Live);
    let covariance = graph
        .nodes
        .iter()
        .find(|n| n.operator == "std.signal.regularize")
        .unwrap()
        .output
        .as_ref()
        .unwrap();
    assert!(matches!(
        covariance.value,
        ValueType::Covariance(_, 2, true)
    ));
    assert!(
        matches!(covariance.occurrence,Occurrence::PendingUniform{rate,..} if rate==Rational::integer(10))
    );
    assert_eq!(compiled.pending_count(), 3);
    assert!(graph.configuration.contains_key("resources"));
    let filter = &graph.nodes[2].parameters["coeffs"];
    assert!(
        matches!(filter, bci_ir::value::ArgumentValue::Coefficients(values) if values.len() == 3 && values[1] == Rational::new(1,2).unwrap())
    );
    let fixed = compile(&EXAMPLE.replace("WindowStart.FirstValid", "0"), &base()).unwrap();
    let window = fixed.graphs[0].nodes[3].output.as_ref().unwrap();
    assert!(
        matches!(window.value, ValueType::Epoch(_, 2, 250, rate) if rate == Rational::integer(250))
    );
    assert!(
        matches!(window.occurrence, Occurrence::Uniform { rate, phase } if rate == Rational::integer(10) && phase == Rational::new(249, 250).unwrap())
    );
    assert_eq!(fixed.pending_count(), 2);
    for (before, after, code) in [
        ("Smooth(volts)", "Smooth(raw)", "E0203"),
        (
            "clock: Session, rate: 250[Hz]",
            "clock: Session, rate: 200[Hz]",
            "E0205",
        ),
        ("const Hop = BaseHop * 5;", "const Hop = Hop * 5;", "E0107"),
        ("Smooth(volts)", "Smooth(missing)", "E0103"),
        ("size: 250, hop: Hop", "size: 250, size: Hop", "E0104"),
        ("size: 250, hop: Hop", "size: 1, hop: Hop", "E0501"),
        ("floor: 1e-12[V^2]", "floor: 1e-12[V]", "E0203"),
        (
            "let smooth = Smooth(volts);",
            "let Leak = runtime.budget(memory: raw); let smooth = Smooth(volts);",
            "E0211",
        ),
        (
            "let smooth = Smooth(volts);",
            "let ordered = volts |> signal.reorder(channels: [\"C4\", \"C3\"]); let smooth = Smooth(ordered);",
            "E0204",
        ),
        (
            "clock Session = time.session_clock();",
            "clock Session = time.session_clock(); clock Session = time.session_clock();",
            "E0102",
        ),
        (
            "runtime.configure(mode: Mode.Live, resources: Budget);",
            "runtime.configure(mode: Mode.Live, resources: Budget); let orphan = source.synthetic(schema: EEG, clock: Session, rate: 250[Hz]);",
            "E0110",
        ),
    ] {
        assert!(EXAMPLE.contains(before));
        let source = EXAMPLE.replace(before, after);
        let errors = compile(&source, &base()).expect_err(code);
        let error = errors
            .iter()
            .find(|e| e.code == code)
            .unwrap_or_else(|| panic!("{code}: {errors:?}"));
        assert!(error.span.start < source.len());
        assert!(
            error
                .render("example.bci", &source)
                .starts_with("example.bci:")
        );
    }
}

#[test]
fn clock_and_record_effect_flow_through_graph_calls() {
    let other = EXAMPLE
        .replace(
            "clock Session = time.session_clock();",
            "clock Session = time.session_clock(); clock Other = time.session_clock();",
        )
        .replace("clock: Session, rate:", "clock: Other, rate:");
    assert!(
        compile(&other, &base())
            .unwrap_err()
            .iter()
            .any(|e| e.code == "E0401")
    );
    let offline = EXAMPLE
        .replace(
            "signal.fir(coeffs: Filter, warmup: Warmup.Full)",
            "signal.filtfilt(coeffs: Filter)",
        )
        .replace("! live", "! record");
    assert!(
        compile(&offline, &base())
            .unwrap_err()
            .iter()
            .any(|e| e.code == "E0301")
    );
    let compiled = compile(&offline.replace("Mode.Live", "Mode.Record"), &base()).unwrap();
    assert_eq!(compiled.graphs[0].effect.mode, Mode::Record);
    assert!(
        compiled.graphs[0]
            .obligations
            .iter()
            .any(|o| o.kind == "RecordEnd")
    );
    let identity = EXAMPLE
        .replace("Mode.Live", "Mode.Record")
        .replace(
            "return x |> signal.fir(coeffs: Filter, warmup: Warmup.Full);",
            "return x;",
        )
        .replace(
            "Smooth(volts)",
            "Smooth(volts |> signal.filtfilt(coeffs: Filter))",
        );
    assert!(
        compile(&identity, &base())
            .unwrap_err()
            .iter()
            .any(|e| e.code == "E0301")
    );
    let invalid = EXAMPLE.replace("return x |>", "return (x |>");
    assert_eq!(compile(&invalid, &base()).unwrap_err()[0].code, "E0101");
    let deep = EXAMPLE.replace("BaseHop * 5", &vec!["1"; 256].join(" + "));
    assert_eq!(compile(&deep, &base()).unwrap_err()[0].code, "E0108");
}
