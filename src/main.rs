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
    io::Write,
    process::{Command, Stdio},
    thread,
};

use anyhow::{Context, Result};
use clap::Parser;
use keyring::Entry;
use regex::Regex;

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
}

fn main() -> Result<()> {
    let args = Args::parse();

    let helper_command = Command::new(&args.credential_helper);
    let helper_exe = helper_command.get_program();

    let mut need_login = args.force;
    if !args.force {
        // Check the error output from the credential helper. If it says we need to rerun
        // "credential-helper login", we do it.
        let mut child = Command::new(helper_exe)
            .arg("get")
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn {}", &args.credential_helper))?;
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
            need_login = true;
        } else {
            println!("Reusing existing credentials.");
        }
    }

    if need_login {
        let status = Command::new(helper_exe)
            .arg("login")
            .arg(&args.remote)
            .stdin(Stdio::null())
            .status()
            .with_context(|| format!("failed to spawn {}", &args.credential_helper))?;
        if !status.success() {
            anyhow::bail!("{} login: {}", args.credential_helper, status);
        }
    }

    let key_name = format!("keyring-rs:{}@AspectWorkflows", args.remote);
    let keychain = if args.persist { "@u" } else { "@s" };

    let mut child = Command::new("ssh")
        // cf. scp.c in openssh-portable.
        .args([
            "-x",
            "-oPermitLocalCommand=no",
            "-oClearAllForwardings=yes",
            "-oRemoteCommand=none",
            "-oRequestTTY=no",
            "-oControlMaster=no",
            "-oForwardAgent=no",
            "-oBatchMode=yes",
            "--",
            &args.host,
            "keyctl",
            "padd",
            "user",
            &key_name,
            keychain,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn ssh")?;
    {
        let entry = Entry::new("AspectWorkflows", &args.remote)
            .context("failed to find aspect credential")?;
        let credential = entry
            .get_password()
            .context("failed to get aspect credential from keychain")?;
        let mut stdin = child.stdin.take().context("failed to open stdin")?;
        thread::spawn(move || -> Result<()> {
            stdin.write_all(credential.as_bytes())?;
            Ok(())
        });
    }
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
