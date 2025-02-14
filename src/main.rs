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
//
use std::{
    fs::Permissions,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
};

use anyhow::{Context, Result};
use clap::Parser;
use keyring::Entry;
use regex::Regex;
use tempfile::{Builder, TempDir};

const DEFAULT_REMOTE: &str = "aw-remote-ext.buildremote.stairwell.io";
const DEFAULT_HELPER: &str = "aspect-credential-helper";

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// SSH hostname to which to sync credential
    #[arg(default_value_t = String::from("devbox"))]
    host: String,

    /// Aspect remote DNS name
    #[arg(env = "ASPECT_REMOTE", default_value_t = DEFAULT_REMOTE.into(), long)]
    remote: String,

    /// Aspect credential helper executable name
    #[arg(env = "ASPECT_CREDENTIAL_HELPER", default_value_t = DEFAULT_HELPER.into(), long)]
    credential_helper: String,

    /// Force re-login even if the credentials are still valid
    #[arg(short, long)]
    force: bool,

    /// Use the user (rather than session) keyring on the VM
    #[arg(short, long)]
    persist: bool,

    /// Reuse existing socket (host has ControlMaster=auto and ControlPersist)
    #[arg(short, long)]
    reuse_socket: bool,
}
fn main() -> Result<()> {
    let args = Args::parse();

    let ssh = SshMux::new(&args.host, args.reuse_socket)
        .with_context(|| format!("failed to ssh to {}", &args.host))?;

    if !args.force {
        // Check the error output from the credential helper. If it says we need to rerun
        // "credential-helper login", we do it.
        let mut child = ssh
            .command(&args.credential_helper)
            .arg("get")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "failed to run {} on {}",
                    &args.credential_helper, &args.host
                )
            })?;
        let mut stdin = child.stdin.take().context("failed to open stdin")?;
        let test_string = format!(concat!(r#"{{"uri":"https://{}"}}"#, "\n"), &args.remote);
        thread::spawn(move || -> Result<()> {
            stdin.write_all(test_string.as_bytes())?;
            Ok(())
        });
        let output = child
            .wait_with_output()
            .with_context(|| format!("failed waiting for {}", &args.credential_helper))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let re = Regex::new(&format!(
                r"(?mis)please\s+run.*{}\s+login",
                regex::escape(&args.credential_helper)
            ))
            .context("failed to compile regex")?;
            if !re.is_match(&stderr) {
                anyhow::bail!(
                    "{} get: {}\n\n{}",
                    args.credential_helper,
                    output.status,
                    stderr.trim(),
                );
            }
        } else {
            println!("Credential refresh not needed. Have a nice day.");
            return Ok(());
        }
    }

    let status = Command::new(&args.credential_helper)
        .arg("login")
        .arg(&args.remote)
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("failed to spawn {}", &args.credential_helper))?;
    if !status.success() {
        anyhow::bail!("{} login: {}", args.credential_helper, status);
    }

    let entry =
        Entry::new("AspectWorkflows", &args.remote).context("failed to find aspect credential")?;
    let credential = entry
        .get_password()
        .context("failed to get aspect credential from keychain")?;

    let key_name = format!("keyring-rs:{}@AspectWorkflows", args.remote);
    let keychain = if args.persist { "@u" } else { "@s" };
    let mut child = ssh
        .command("keyctl")
        .args(["padd", "user", &key_name, keychain])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run keyctl on {}", &args.host))?;
    let mut stdin = child.stdin.take().context("failed to open stdin")?;
    thread::spawn(move || -> Result<()> {
        stdin.write_all(credential.as_bytes())?;
        Ok(())
    });

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "ssh {} keyctl padd: {}\n\n{}",
            args.host,
            output.status,
            stderr.trim(),
        );
    }
    println!(
        "Aspect credentials synced to {}. Have a nice day.",
        args.host
    );
    Ok(())
}

struct SshMux {
    temp_dir: TempDir,
    host: String,
    open_socket: bool,
}

impl SshMux {
    fn new(host: &str, reuse_socket: bool) -> Result<Self> {
        let temp_dir = Builder::new()
            .prefix("aspect-reauth-")
            .permissions(Permissions::from_mode(0o700))
            .tempdir()
            .context("failed to create temporary directory")?;
        let host = host.to_string();
        let ret = SshMux {
            temp_dir,
            host,
            open_socket: !reuse_socket,
        };
        let mut cmd = Command::new("ssh");
        if reuse_socket {
            cmd.args(["--", &ret.host, "exit"]);
        } else {
            cmd.args([
                "-xMTS",
                &ret.control_path().to_string_lossy(),
                "-oControlPersist=yes",
                "-oPermitLocalCommand=no",
                "-oClearAllForwardings=yes",
                "-oRemoteCommand=none",
                "-oForwardAgent=no",
                "-oBatchMode=yes",
                "--",
                &ret.host,
                "exit",
            ]);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .context("failed to start SSH control master")?;
        Ok(ret)
    }

    fn command(&self, command: &str) -> Command {
        let mut ret = Command::new("ssh");
        if self.open_socket {
            ret.args(["-S", &self.control_path().to_string_lossy()]);
        }
        ret.args([
            "-xT",
            "-oPermitLocalCommand=no",
            "-oClearAllForwardings=yes",
            "-oRemoteCommand=none",
            "-oForwardAgent=no",
            "-oBatchMode=yes",
            "--",
            &self.host,
            command,
        ]);
        ret
    }

    fn control_path(&self) -> PathBuf {
        self.temp_dir.path().join("sock")
    }

    fn cleanup(&mut self) -> Result<()> {
        if !self.open_socket {
            return Ok(());
        }
        self.open_socket = false;
        Command::new("ssh")
            .args([
                "-S",
                &self.control_path().to_string_lossy(),
                "-Oexit",
                "--",
                &self.host,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("failed to cleanup SSH control master")?;
        Ok(())
    }
}

impl Drop for SshMux {
    fn drop(&mut self) {
        if let Err(e) = self.cleanup() {
            eprintln!("cleanup ssh: {}", e);
        }
    }
}
