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

//! Credential sync for pre-v6 Aspect Workflows deployments.
//!
//! Logs in locally with `aspect-credential-helper`, reads the resulting credential out of the
//! local OS keychain, and stores it in the remote host's keyutils keychain via `keyctl padd`.

use std::sync::Arc;

use anyhow::{Context, Result};
use keyring::Entry;
use regex::bytes::Regex;
use smol::{
    io::AsyncWriteExt,
    process::{Command, Stdio},
};

use crate::Args;
use crate::ssh_mux::SshMux;

pub async fn run(args: &Arc<Args>, ssh: &SshMux<'_, String>) -> Result<()> {
    let remote_needs_refresh = async {
        Ok::<bool, anyhow::Error>(args.force_remote || needs_refresh(args, Some(ssh)).await?)
    };
    if args.force_local || needs_refresh(args, None).await? {
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
        let password = get_credential("AspectWorkflows", args)
            .await
            .context("failed to fetch fresh password from by aspect-credential-helper")?;
        set_credential("aspect-reauth", args, password)
            .await
            .context("failed to store password for aspect-reauth")?;
    }
    if !remote_needs_refresh.await? {
        println!("Credential refresh not needed. Have a nice day.");
        return Ok(());
    }

    let password = match get_credential("aspect-reauth", args).await {
        Ok(p) => p,
        _ => {
            let password = get_credential("AspectWorkflows", args)
                .await
                .context("failed to fetch password from aspect-credential-helper")?;
            if let Err(e) = set_credential("aspect-reauth", args, password.clone()).await {
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

    if needs_refresh(args, Some(ssh)).await? {
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

async fn needs_refresh<'a>(args: &'a Args, ssh: Option<&'a SshMux<'a, String>>) -> Result<bool> {
    let helper = &args.credential_helper;
    let mut cmd = ssh
        .map(|ssh| ssh.command(helper))
        .unwrap_or_else(|| Command::new(helper));
    let mut child = cmd
        .arg("get")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            ssh.is_some()
                .then(|| format!("failed to run {helper} on {}", &args.host))
                .unwrap_or_else(|| format!("failed to run {helper}"))
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
