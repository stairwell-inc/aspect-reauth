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
use core::{error, result, str};
use std::{
    io::Write,
    process::{Command, Stdio},
};

use clap::Parser;
use keyring::Entry;

type Result<T> = result::Result<T, Box<dyn error::Error>>;

const DEFAULT_REMOTE: &str = "aw-remote-ext.buildremote.stairwell.io";
const DEFAULT_HELPER: &str = "aspect-credential-helper";

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// VM SSH hostname to which to sync credential
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
        let mut child = Command::new(helper_exe)
            .arg("get")
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()?;
        {
            let mut stdin = child.stdin.take().expect("failed to open stdin");
            writeln!(stdin, r#"{{"uri":"https://{}"}}"#, args.remote)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            need_login = true;
        }
    }

    if need_login {
        println!("Your browser will be opened.");
        let output = Command::new(helper_exe)
            .arg("login")
            .arg(&args.remote)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;
        if !output.status.success() {
            return Err(format!("{} login: {:?}", &args.credential_helper, &output).into());
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
        .spawn()?;
    {
        let entry = Entry::new("AspectWorkflows", &args.remote)?;
        let credential = entry.get_password()?;
        let mut stdin = child.stdin.take().expect("failed to open stdin");
        stdin.write_all(credential.as_bytes())?;
    }
    let status = child.wait()?;
    if !status.success() {
        return Err(format!("ssh ... keyctl: {:?}", status).into());
    }
    Ok(())
}
