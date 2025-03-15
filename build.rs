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

use std::env;

fn main() {
    let default_remote = env::var("ASPECT_REMOTE")
        .unwrap_or_else(|_| "aw-remote-ext.buildremote.stairwell.io".into());
    let default_helper = env::var("ASPECT_CREDENTIAL_HELPER")
        .unwrap_or_else(|_| "aspect-credential-helper".into());

    println!("cargo::rerun-if-env-changed=ASPECT_REMOTE");
    println!("cargo::rerun-if-env-changed=ASPECT_CREDENTIAL_HELPER");
    println!("cargo::rustc-env=ASPECT_REMOTE={}", default_remote);
    println!("cargo::rustc-env=ASPECT_CREDENTIAL_HELPER={}", default_helper);
}
