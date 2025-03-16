// Copyright 2025 Stairwell, Inc.
// Author: andy@stairwell.com
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

use std::process::Command;

/// Returns true if it looks like we should create our own socket to manage SSH connection
/// multiplexing, false otherwise. Specifically this returns true if the host configuration does
/// not include `ControlMaster auto`; if it does, then we assume we can just use (or create) the
/// pre-existing socket.
///
/// This is not a perfect heuristic; e.g. if the host has too short a `ControlPersist` value, then
/// we might wind up starting up multiple connections regardless.
///
/// If this function encounters an error, it just returns false. The assumption is that whatever
/// went wrong reading the config will also go wrong connecting to the host, and that there's no
/// reason to stand up our own managed socket for a connection we expect to fail anyway.
pub fn infer_create_socket(host: &str) -> bool {
    let Ok(output) = Command::new("ssh").args(["-G", "--", host]).output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8(output.stdout)
        .map(|stdout| !stdout.lines().any(|line| line == "controlmaster auto"))
        .unwrap_or(false)
}
