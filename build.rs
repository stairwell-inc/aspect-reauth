// Copyright 2025 Stairwell, Inc.
// Author: mrdomino@stairwell.com
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

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
        r @ (Ok(_) | Err(VarError::NotUnicode(_))) => r?,
        Err(VarError::NotPresent) => default.into(),
    };
    println!(
        "cargo::rerun-if-env-changed={name}\n\
         cargo::rustc-env={name}={val}"
    );
    Ok(())
}
