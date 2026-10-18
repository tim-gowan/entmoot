//! Exercise the actual executable stack, including debug builds on Windows.
#[test]
fn help_and_argument_errors_work_on_the_process_stack() {
    for args in [
        vec!["--help"],
        vec!["query", "--help"],
        vec!["find", "--help"],
        vec!["path", "--help"],
        vec!["neighborhood", "--help"],
        vec!["impact", "--help"],
        vec!["assess", "clones", "--help"],
        vec!["assess", "unused", "--help"],
        vec!["assess", "impact", "--help"],
        vec!["collaborate", "--help"],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_entmoot"))
            .args(&args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {:?}", output);
        assert!(!output.stdout.is_empty());
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_entmoot"))
        .arg("--not-an-option")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}
