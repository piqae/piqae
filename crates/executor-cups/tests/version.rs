use std::process::Command;

#[test]
fn reports_the_embedded_release_version_without_reading_protocol_input() {
    let output = Command::new(env!("CARGO_BIN_EXE_piqae-executor-cups"))
        .arg("--version")
        .output()
        .expect("executor version command should start");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version output should be UTF-8"),
        format!("piqae-executor-cups {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}
