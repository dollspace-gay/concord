use std::fs;
use uuid::Uuid;

mod common;
use common::VerifiedBinary;

#[test]
fn init_and_validate_are_explicit_and_never_replace_the_secret() {
    let root = std::env::temp_dir().join(format!("concord-cli-{}", Uuid::new_v4()));
    let config = root.join("concord.toml");
    let binary = VerifiedBinary::copy_from(
        std::path::Path::new(env!("CARGO_BIN_EXE_concord-server")),
        root.join("test-bin/concord-server"),
    );

    let initialized = binary
        .command()
        .args(["init", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(initialized.status.success());
    let secret_path = root.join("data/secrets/jwt.key");
    let secret = fs::read_to_string(&secret_path).unwrap();

    let validated = binary
        .command()
        .args(["validate-config", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(validated.status.success());

    let repeated = binary
        .command()
        .args(["init", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert_eq!(secret, fs::read_to_string(secret_path).unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn validate_config_does_not_echo_malformed_secret_source() {
    let root = std::env::temp_dir().join(format!("concord-cli-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let config = root.join("concord.toml");
    let binary = VerifiedBinary::copy_from(
        std::path::Path::new(env!("CARGO_BIN_EXE_concord-server")),
        root.join("test-bin/concord-server"),
    );
    let sentinel = "DO-NOT-LOG-THIS-SECRET";
    fs::write(&config, format!("[auth]\njwt_secret = \"{sentinel}\n")).unwrap();

    let output = binary
        .command()
        .args(["validate-config", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    assert!(!diagnostics.contains(sentinel));
    assert!(diagnostics.contains("line"));
    fs::remove_dir_all(root).unwrap();
}
