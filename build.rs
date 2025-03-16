use std::env;

fn main() {
    let default_remote = env::var("ASPECT_REMOTE")
        .unwrap_or_else(|_| "aw-remote-ext.buildremote.stairwell.io".into());
    let default_helper =
        env::var("ASPECT_CREDENTIAL_HELPER").unwrap_or_else(|_| "aspect-credential-helper".into());

    println!("cargo::rerun-if-env-changed=ASPECT_REMOTE");
    println!("cargo::rerun-if-env-changed=ASPECT_CREDENTIAL_HELPER");
    println!("cargo::rustc-env=ASPECT_REMOTE={}", default_remote);
    println!(
        "cargo::rustc-env=ASPECT_CREDENTIAL_HELPER={}",
        default_helper
    );
}
