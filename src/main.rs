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

mod ssh_mux;

use std::{
    io::Write,
    process::{Command, Stdio},
    thread,
};

use anyhow::{Context, Result};
use clap::Parser;
use keyring::Entry;
use regex::bytes::Regex;
use ssh_mux::SshMux;

const DEFAULT_REMOTE: &str = "aw-remote-ext.buildremote.stairwell.io";
const DEFAULT_HELPER: &str = "aspect-credential-helper";

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// SSH hostname to which to sync credential
    #[arg(default_value = "devbox")]
    host: String,

    /// Aspect remote DNS name
    #[arg(env = "ASPECT_REMOTE", default_value = DEFAULT_REMOTE, long)]
    remote: String,

    /// Aspect credential helper executable name
    #[arg(env = "ASPECT_CREDENTIAL_HELPER", default_value = DEFAULT_HELPER, long)]
    credential_helper: String,

    /// Force re-login even if the credentials are still valid
    #[arg(short, long)]
    force: bool,

    /// Use the session (rather than user) keyring on the VM
    #[arg(short, long)]
    session_keyring: bool,

    /// Create a temporary SSH control socket (if unset, this is automatically inferred)
    #[arg(short, long, conflicts_with = "no_create_socket", default_missing_value = "true",
          num_args = 0..=1, require_equals = true)]
    create_socket: Option<bool>,

    /// Do not create a temporary SSH control socket
    #[arg(short = 'C', long, conflicts_with = "create_socket")]
    no_create_socket: bool,

    /// Reuse existing socket (Deprecated: use --create-socket=false instead)
    #[arg(short, long)]
    _reuse_socket: bool,

    /// Use persistent keyring (Deprecated: now default)
    #[arg(short, long)]
    _persist: bool,
}

fn main() -> Result<()> {
    let mut args = Args::parse();
    if args.no_create_socket {
        args.create_socket = Some(false);
    }
    if args._persist {
        eprintln!("The -p / --persist flag is deprecated, please do not use it.");
    }
    if args._reuse_socket {
        eprintln!("The -r / --reuse-socket flag is deprecated, please do not use it.");
        args.create_socket = Some(false);
    }
    let args = args;

    let ssh =
        SshMux::new(&args.host, args.create_socket).context("failed setting up ssh session")?;

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
        thread::spawn(move || {
            let _ = stdin.write_all(test_string.as_bytes());
        });
        let output = child
            .wait_with_output()
            .with_context(|| format!("failed waiting for {}", &args.credential_helper))?;
        if !output.status.success() {
            let re = Regex::new(&format!(
                r"(?mis)please\s+run.*{}\s+login",
                regex::escape(&args.credential_helper)
            ))
            .context("failed to compile regex")?;
            if !re.is_match(&output.stderr) {
                anyhow::bail!(
                    "{} get: {}\n\n{}",
                    args.credential_helper,
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim(),
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
    let keychain = if args.session_keyring { "@s" } else { "@u" };
    let mut child = ssh
        .command("keyctl")
        .args(["padd", "user", &key_name, keychain])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run keyctl on {}", &args.host))?;
    let mut stdin = child.stdin.take().context("failed to open stdin")?;
    thread::spawn(move || {
        let _ = stdin.write_all(credential.as_bytes());
    });
    let output = child.wait_with_output()?;
    if !output.status.success() {
        anyhow::bail!(
            "ssh {} keyctl padd: {}\n\n{}",
            args.host,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
        );
    }

    println!(
        "Aspect credentials synced to {}. Have a nice day.",
        args.host
    );
    Ok(())
}
