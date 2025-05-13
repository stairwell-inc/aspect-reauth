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

use std::{ffi::OsStr, fs::remove_dir_all, path::Path};

use anyhow::Result;
use tempfile::TempDir;

/// Exposes and controls a path suitable for use as a temporary socket. The path is made available
/// by `AsRef<OsStr>` on `&TempSocket`, so that a reference to this may be passed directly to
/// `Command::arg`:
/// ```
/// let socket = TempSocket::new()?;
/// let _ = Command::new("ssh").arg("-MS").arg(&socket);
/// ```
/// The temporary directory and its contents are removed by `drop`.
pub struct TempSocket {
    path: Box<Path>,
}

impl TempSocket {
    pub fn new(prefix: &str) -> Result<Self> {
        let mut builder = tempfile::Builder::new();
        #[cfg(unix)]
        {
            use std::{fs::Permissions, os::unix::fs::PermissionsExt};
            builder.permissions(Permissions::from_mode(0o700));
        }
        Ok(Self::from_tempdir(builder.prefix(prefix).tempdir()?))
    }

    fn from_tempdir(dir: TempDir) -> Self {
        let mut path = dir.into_path();
        path.push("sock");
        TempSocket {
            path: path.into_boxed_path(),
        }
    }
}

impl AsRef<OsStr> for &TempSocket {
    fn as_ref(&self) -> &OsStr {
        self.path.as_os_str()
    }
}

impl Drop for TempSocket {
    fn drop(&mut self) {
        if let Some(dir) = self.path.parent() {
            let _ = remove_dir_all(dir);
        }
    }
}
