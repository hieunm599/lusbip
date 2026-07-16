use lusbip::client_agent::{ClientEndpoint, ControlRequest, percent_decode, percent_encode};

#[test]
fn endpoint_runtime_paths_are_stable_and_remote_scoped() {
    let first = ClientEndpoint::new("10.10.61.72", 3240);
    let second = ClientEndpoint::new("10.10.61.72", 3241);

    assert_ne!(first.runtime_dir(), second.runtime_dir());
    assert!(first.runtime_dir().starts_with(std::env::temp_dir()));
}

#[test]
fn control_request_parses_complete_and_rejects_incomplete_messages() {
    assert_eq!(
        ControlRequest::parse("STATUS\n").unwrap(),
        ControlRequest::Status
    );
    assert!(ControlRequest::parse("TOGGLE\n").is_err());
}

#[test]
fn control_protocol_round_trips_reserved_characters() {
    let value = "5-1\tlabel\n100%";
    assert_eq!(percent_decode(&percent_encode(value)).unwrap(), value);
    assert!(percent_decode("%ZZ").is_err());
}
