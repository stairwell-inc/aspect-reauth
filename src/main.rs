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

use std::{str::FromStr, sync::Arc};

use anyhow::{Context, Result};
use clap::Parser;
use keyring::Entry;
use regex::bytes::Regex;
use smol::{
    io::AsyncWriteExt,
    process::{Command, Stdio},
};
use ssh_mux::{CreateSocket, SshMux};

const DEFAULT_REMOTE: &str = env!("ASPECT_REMOTE");
const DEFAULT_HELPER: &str = env!("ASPECT_CREDENTIAL_HELPER");

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

    /// Force re-login and sync even if the credentials are still valid
    #[arg(short, long)]
    force: bool,

    /// Force re-login even if the credentials are still valid
    #[arg(short = 'l', long, conflicts_with = "force")]
    force_local: bool,

    /// Force remote sync even if the credentials are still valid
    #[arg(short = 'r', long, conflicts_with = "force")]
    force_remote: bool,

    /// Use the session (rather than user) keyring on the VM
    #[arg(short, long)]
    session_keyring: bool,

    /// Create a temporary SSH control socket [values: true, false, infer]
    #[arg(
        short,
        long,
        conflicts_with = "no_create_socket",
        default_value = "infer",
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = true,
    )]
    create_socket: CreateSocket,

    /// Do not create a temporary SSH control socket
    #[arg(short = 'C', long, conflicts_with = "create_socket")]
    no_create_socket: bool,

    /// Call SSH with an additional argument (takes multiple: --ssh-arg='-p 23' --ssh-arg='-A')
    #[arg(short = 'A', long = "ssh-arg", alias = "ssh_arg", action = clap::ArgAction::Append)]
    ssh_args: Vec<String>,
}

fn main() -> Result<()> {
    smol::block_on(async_main())
}

async fn async_main() -> Result<()> {
    let mut args = Args::parse();
    if args.no_create_socket {
        args.create_socket = CreateSocket::Specify(false);
    }
    if args.force {
        args.force_remote = true;
        args.force_local = true;
    }
    let args = Arc::new(args);

    let ssh = SshMux::new(&args.host, &args.ssh_args, args.create_socket)
        .await
        .context("failed setting up ssh session")?;

    let remote_needs_refresh = async {
        Ok::<bool, anyhow::Error>(
            args.force_remote || needs_refresh(&args, |s| ssh.command(s)).await?,
        )
    };
    if args.force_local || needs_refresh(&args, Command::new).await? {
        let status = Command::new(&args.credential_helper)
            .arg("login")
            .arg(&args.remote)
            .stdin(Stdio::null())
            .status()
            .await
            .with_context(|| format!("failed to spawn {}", &args.credential_helper))?;
        if !status.success() {
            anyhow::bail!("{} login: {}", args.credential_helper, status);
        }
        let password = get_credential("AspectWorkflows", &args)
            .await
            .context("failed to fetch fresh password from by aspect-credential-helper")?;
        set_credential("aspect-reauth", &args, password)
            .await
            .context("failed to store password for aspect-reauth")?;
    }
    if !remote_needs_refresh.await? {
        println!("Credential refresh not needed. Have a nice day.");
        return Ok(());
    }

    let password = match get_credential("aspect-reauth", &args).await {
        Ok(p) => p,
        _ => {
            let password = get_credential("AspectWorkflows", &args)
                .await
                .context("failed to fetch password from aspect-credential-helper")?;
            if let Err(e) = set_credential("aspect-reauth", &args, password.clone()).await {
                eprintln!("failed to sync aspect-reauth password:\n{e}");
            }
            password
        }
    };

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
    stdin.write_all(password.as_bytes()).await?;
    drop(stdin);
    let output = child.output().await?;
    if !output.status.success() {
        anyhow::bail!(
            "ssh {} keyctl padd: {}\n\n{}",
            args.host,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
        );
    }

    if needs_refresh(&args, |s| ssh.command(s)).await? {
        anyhow::bail!(
            concat!(
                "We tried syncing your credentials to {} but they are still invalid.\n",
                "Try rerunning with --force."
            ),
            args.host
        );
    }

    println!(
        "Aspect credentials synced to {}. Have a nice day.",
        args.host
    );
    Ok(())
}

async fn needs_refresh<'a>(args: &'a Args, f: impl FnOnce(&'a str) -> Command) -> Result<bool> {
    let mut child = f(&args.credential_helper)
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
    stdin.write_all(test_string.as_bytes()).await?;
    drop(stdin);
    let output = child
        .output()
        .await
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

async fn get_credential(name: &'static str, args: &Arc<Args>) -> Result<String> {
    let args = args.clone();
    smol::unblock(move || -> Result<String> {
        Entry::new(name, &args.remote)
            .and_then(|e| e.get_password())
            .context("failed to get aspect credential from keychain")
    })
    .await
}

async fn set_credential(name: &'static str, args: &Arc<Args>, password: String) -> Result<()> {
    let args = args.clone();
    smol::unblock(move || -> Result<()> {
        Entry::new(name, &args.remote)
            .and_then(|e| e.set_password(&password))
            .context("failed to set aspect credential in keychain")
    })
    .await
}

impl FromStr for CreateSocket {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "infer" => Ok(CreateSocket::Infer),
            // Regrettably there is not any easy way to get at clap's BoolishValueParser from here,
            // so we inline its current implementation instead.
            _ => Ok(CreateSocket::Specify(match s {
                "y" | "yes" | "t" | "true" | "on" | "1" => true,
                "n" | "no" | "f" | "false" | "off" | "0" => false,
                _ => anyhow::bail!("unknown value {s}"),
            })),
        }
    }
}
