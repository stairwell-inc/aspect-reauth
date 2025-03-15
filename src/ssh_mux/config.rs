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

pub fn infer_create_socket(host: &str) -> bool {
    // Get the output of `ssh -G <host>` this will have a standard
    // lowercase represesntation of:
    //
    // <key> <value>
    //
    // so a basic match should be enough.
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
