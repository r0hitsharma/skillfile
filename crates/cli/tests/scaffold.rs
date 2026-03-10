use assert_cmd::cargo_bin_cmd;
use predicates::prelude::*;

#[test]
fn help_flag_exits_zero() {
    cargo_bin_cmd!("skillfile")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Tool-agnostic AI skill & agent manager",
        ));
}

#[test]
fn version_flag_exits_zero() {
    cargo_bin_cmd!("skillfile")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("skillfile"));
}

#[test]
fn no_args_exits_nonzero() {
    cargo_bin_cmd!("skillfile")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}
