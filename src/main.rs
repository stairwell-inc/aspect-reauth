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
    sync::Arc,
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

    /// Deprecated, do not use.
    #[arg(short, long)]
    _persist: bool,

    /// Use the session (rather than user) keyring on the VM
    #[arg(short, long)]
    session_keyring: bool,

    /// Reuse existing socket (host has ControlMaster=auto and ControlPersist)
    #[arg(short, long)]
    reuse_socket: bool,
}

fn main() -> Result<()> {
    let args = Arc::new(Args::parse());
    if args._persist {
        eprintln!("The -p / --persist flag is deprecated and now a no-op, please do not use it.");
    }

    // Refresh the local credential if we need to. We thread this so it can run in parallel with
    // the SSH connection setup and remote credential refresh check.
    let thread_args = Arc::clone(&args);
    let local_handle = thread::spawn(move || -> Result<()> {
        if !thread_args.force
            && !needs_refresh(Command::new(&thread_args.credential_helper), &thread_args)?
        {
            return Ok(());
        }
        let status = Command::new(&thread_args.credential_helper)
            .arg("login")
            .arg(&thread_args.remote)
            .stdin(Stdio::null())
            .status()
            .with_context(|| format!("failed to spawn {}", &thread_args.credential_helper))?;
        if !status.success() {
            anyhow::bail!("{} login: {}", thread_args.credential_helper, status);
        }
        Ok(())
    });

    let ssh_remote_result = ssh_needs_refresh(&args).with_context(|| {
        format!(
            "failed to run {} on {}",
            &args.credential_helper, &args.host
        )
    });
    // Show the local error first; it's most likely that the credential helper just isn't set up
    // properly, which is upstream of many of the other things that could go wrong.
    local_handle
        .join()
        .map_err(|e| anyhow::anyhow!("thread panic: {:?}", e))?
        .with_context(|| format!("failed to run {}", &args.credential_helper))?;
    let (ssh, remote_refresh) = ssh_remote_result?;
    if !remote_refresh {
        println!("Credential refresh not needed. Have a nice day.");
        return Ok(());
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

fn ssh_needs_refresh(args: &Args) -> Result<(SshMux, bool)> {
    let ssh =
        SshMux::new(&args.host, args.reuse_socket).context("failed setting up ssh session")?;
    if !args.force && !needs_refresh(ssh.command(&args.credential_helper), &args)? {
        return Ok((ssh, false));
    }
    return Ok((ssh, true));
}

fn needs_refresh(mut credential_cmd: Command, args: &Args) -> Result<bool> {
    let mut child = credential_cmd
        .arg("get")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn: {:?}", credential_cmd))?;
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
        return Ok(true);
    }
    Ok(false)
}
