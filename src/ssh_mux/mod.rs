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

mod config;
mod temp_socket;

use std::ffi::OsStr;

use anyhow::{Context, Result};
use config::infer_create_socket;
use smol::process::{Command, Stdio};
use temp_socket::TempSocket;

#[derive(Clone, Copy)]
pub enum CreateSocket {
    Infer,
    Specify(bool),
}

/// A batched SSH command multiplexer.
///
/// This class does two things:
/// 1. It passes a set of restrictive options to `ssh` suitable for use in a batch context.
/// 2. Optionally, it stands up a temporary SSH master and control socket, allowing the same socket
///    to be reused across SSH commands so that subsequent commands do not incur connection setup
///    overhead.
pub struct SshMux<'a, T: AsRef<OsStr>> {
    host: &'a str,
    ssh_args: &'a [T],
    socket: Option<TempSocket>,
}

impl<'a, T: AsRef<OsStr>> SshMux<'a, T> {
    pub async fn new(
        host: &'a str,
        ssh_args: &'a [T],
        create_socket: CreateSocket,
    ) -> Result<Self> {
        let socket = match create_socket.into_option_bool() {
            Some(val) => val,
            None => infer_create_socket(host).await,
        }
        .then(|| TempSocket::new("aspect-reauth-"))
        .transpose()?;
        let mut cmd = Command::new("ssh");
        cmd.args(ssh_args);
        if let Some(socket) = &socket {
            // cf. scp.c in openssh-portable.
            cmd.arg("-xMTS").arg(socket).args([
                "-oControlPersist=yes",
                "-oPermitLocalCommand=no",
                "-oClearAllForwardings=yes",
                "-oRemoteCommand=none",
                "-oForwardAgent=no",
                "-oBatchMode=yes",
            ]);
        }
        // If we're reusing an existing socket but the host has ControlMaster=auto and no currently
        // running master, we do not want the created master to have the restrictive set of options
        // we pass to individual commands, so we still run an initial ssh to open a normal session.
        let output = cmd
            .args(["--", host, "true"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("failed to start SSH control master")?;
        if !output.status.success() {
            anyhow::bail!(
                "ssh {}: {}\n\n{}",
                host,
                output.status,
                String::from_utf8_lossy(&output.stderr).trim(),
            );
        }
        Ok(SshMux {
            host,
            ssh_args,
            socket,
        })
    }

    pub fn command(&self, command: &str) -> Command {
        let mut ret = Command::new("ssh");
        ret.args(self.ssh_args);
        if let Some(socket) = &self.socket {
            ret.arg("-S").arg(socket);
        }
        ret.args([
            "-xT",
            "-oPermitLocalCommand=no",
            "-oClearAllForwardings=yes",
            "-oRemoteCommand=none",
            "-oForwardAgent=no",
            "-oBatchMode=yes",
            "--",
            self.host,
            command,
        ]);
        ret
    }

    pub async fn cleanup(&mut self) -> Result<()> {
        let Some(socket) = self.socket.take() else {
            return Ok(());
        };
        Command::new("ssh")
            .args(self.ssh_args)
            .arg("-S")
            .arg(&socket)
            .args(["-Oexit", "--", self.host])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .context("failed to cleanup SSH control master")?;
        Ok(())
    }
}

impl<T: AsRef<OsStr>> Drop for SshMux<'_, T> {
    fn drop(&mut self) {
        smol::block_on(async {
            if let Err(e) = self.cleanup().await {
                eprintln!("cleanup ssh: {}", e);
            }
        });
    }
}

impl CreateSocket {
    fn into_option_bool(self) -> Option<bool> {
        match self {
            CreateSocket::Infer => None,
            CreateSocket::Specify(b) => Some(b),
        }
    }
}
