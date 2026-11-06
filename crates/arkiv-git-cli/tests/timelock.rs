//! `arkiv-git timelock` through the real binary; needs no network and no backend.

use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_arkiv-git"))
        .args(args)
        .output()
        .expect("spawn")
}

#[test]
fn open_before_the_time_fails_with_e_too_early() {
    let dir = tempfile::tempdir().expect("tmp");
    let (plain, sealed, out) = (
        dir.path().join("p"),
        dir.path().join("s"),
        dir.path().join("o"),
    );
    std::fs::write(&plain, b"embargoed").expect("write");
    let s = |p: &std::path::Path| p.to_str().expect("utf8").to_owned();

    let seal = run(&[
        "timelock",
        "seal",
        "--until",
        "+1d",
        "--in",
        &s(&plain),
        "--out",
        &s(&sealed),
    ]);
    assert!(
        seal.status.success(),
        "{}",
        String::from_utf8_lossy(&seal.stderr)
    );
    assert!(String::from_utf8_lossy(&seal.stdout).contains("sealed until "));

    let open = run(&["timelock", "open", "--in", &s(&sealed), "--out", &s(&out)]);
    assert_eq!(open.status.code(), Some(1));
    let err = String::from_utf8_lossy(&open.stderr);
    assert!(err.contains("E_TOO_EARLY") && err.contains("20"), "{err}");
    assert!(!out.exists(), "no plaintext may be written");
}

#[test]
fn bad_until_is_a_usage_error() {
    let o = run(&[
        "timelock",
        "seal",
        "--until",
        "soon",
        "--in",
        "/dev/null",
        "--out",
        "/dev/null",
    ]);
    assert_eq!(o.status.code(), Some(2));
}
