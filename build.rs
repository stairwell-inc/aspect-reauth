use std::env::{self, VarError};

fn main() -> Result<(), VarError> {
    build_env_var("ASPECT_REMOTE", "aw-remote-ext.buildremote.stairwell.io")?;
    build_env_var("ASPECT_CREDENTIAL_HELPER", "aspect-credential-helper")?;
    Ok(())
}

/// Exposes the named environment variable transparently as a build environment variable, using the
/// passed default if the variable is unset.
fn build_env_var(name: &str, default: &str) -> Result<(), VarError> {
    let val = match env::var(name) {
        r@(Ok(_) | Err(VarError::NotUnicode(_))) => r?,
        Err(VarError::NotPresent) => default.into(),
    };
    println!(
        "cargo::rerun-if-env-changed={name}\n\
         cargo::rustc-env={name}={val}"
    );
    Ok(())
}
