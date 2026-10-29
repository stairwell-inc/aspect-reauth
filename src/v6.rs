// Copyright 2026 Stairwell, Inc.
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

//! Login for Aspect Workflows v6 deployments.
//!
//! v6 deployments authenticate through the Aspect CLI (`aspect auth login`), which stores its
//! credential, including a refresh token, on the machine where it runs. Rather than log in locally
//! and copy the credential across (which would leave two machines rotating one refresh token), we
//! run the login on the remote host itself and tunnel its loopback OAuth callback back to this
//! machine's browser over SSH. The remote host then keeps its own credential fresh.

use std::time::Duration;

use anyhow::{Context, Result};
use regex::Regex;
use smol::{
    Timer,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Command, Stdio},
    stream::StreamExt,
};

use crate::Args;
use crate::ssh_mux::SshMux;

/// Error text `aspect auth configure` prints for a host that is not a v6 deployment.
const NOT_A_DEPLOYMENT: &str = "is not an Aspect Workflows deployment authentication endpoint";

/// Exit status a shell reports for a command that is not found.
const COMMAND_NOT_FOUND: i32 = 127;

/// How long the remote login waits for the browser callback before giving up. Bounds the lifetime
/// of the remote process if this one goes away without the callback ever arriving.
const LOGIN_TIMEOUT: &str = "10m";

/// The remote host's credential state for the configured Aspect remote.
enum Status {
    /// `aspect get` returned an `Authorization` header.
    Authorized,
    /// `aspect get` succeeded but returned no header: no configured deployment owns the remote.
    Unconfigured,
    /// `aspect get` failed, typically because the credential is missing or expired and cannot be
    /// refreshed. Carries its stderr.
    NeedsLogin(String),
    /// The Aspect CLI is not installed on the remote host.
    Unavailable,
}

/// Ensure the remote host holds a valid credential for the Aspect remote, logging in if needed.
///
/// Returns `false` without changing anything if the remote is not a v6 deployment (or the remote
/// host has no Aspect CLI), in which case the caller should fall back to the legacy flow.
pub async fn run(args: &Args, ssh: &SshMux<'_, String>) -> Result<bool> {
    match status(args, ssh).await? {
        Status::Unavailable => return Ok(false),
        Status::Authorized if !args.force_remote => {
            println!("Credential refresh not needed. Have a nice day.");
            return Ok(true);
        }
        Status::Authorized | Status::NeedsLogin(_) => {}
        Status::Unconfigured => {
            // Configuring may repoint an existing deployment name at a new host, leaving a
            // credential minted for the old one filed under it, so always log in afterwards.
            if !configure(args, ssh).await? {
                return Ok(false);
            }
        }
    }

    login(args, ssh).await?;

    match status(args, ssh).await? {
        Status::Authorized => {}
        Status::NeedsLogin(stderr) => anyhow::bail!(
            "Logged in on {} but the credential is still invalid:\n\n{}",
            args.host,
            stderr
        ),
        _ => anyhow::bail!(
            "Logged in on {} but no credential is issued for {}.",
            args.host,
            args.remote
        ),
    }
    println!(
        "Aspect credentials refreshed on {}. Have a nice day.",
        args.host
    );
    Ok(true)
}

/// Ask the remote Aspect CLI, acting as a Bazel credential helper, for the remote's credential.
async fn status(args: &Args, ssh: &SshMux<'_, String>) -> Result<Status> {
    let mut child = ssh
        .command(&args.aspect_bin)
        .arg("get")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run {} on {}", &args.aspect_bin, &args.host))?;
    let mut stdin = child.stdin.take().context("failed to open stdin")?;
    let request = format!(concat!(r#"{{"uri":"https://{}"}}"#, "\n"), &args.remote);
    stdin.write_all(request.as_bytes()).await?;
    drop(stdin);
    let output = child
        .output()
        .await
        .with_context(|| format!("failed waiting for {} get", &args.aspect_bin))?;
    if output.status.code() == Some(COMMAND_NOT_FOUND) {
        return Ok(Status::Unavailable);
    }
    if !output.status.success() {
        return Ok(Status::NeedsLogin(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    if has_authorization(&String::from_utf8_lossy(&output.stdout)) {
        Ok(Status::Authorized)
    } else {
        Ok(Status::Unconfigured)
    }
}

/// Record the Aspect remote as a deployment on the remote host. Returns `false` if the remote is
/// not a v6 deployment.
async fn configure(args: &Args, ssh: &SshMux<'_, String>) -> Result<bool> {
    let output = ssh
        .command(&args.aspect_bin)
        .args(["auth", "configure", &args.remote])
        .args(["--name", &args.deployment, "--login=false"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("failed to run {} on {}", &args.aspect_bin, &args.host))?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(NOT_A_DEPLOYMENT) {
        return Ok(false);
    }
    anyhow::bail!(
        "ssh {} {} auth configure: {}\n\n{}",
        args.host,
        args.aspect_bin,
        output.status,
        stderr.trim(),
    );
}

/// Run `aspect auth login` on the remote host, forwarding its OAuth callback port from this
/// machine and opening the login URL in the local browser.
///
/// Over SSH the Aspect CLI does not try to open a browser itself, and with stdin closed it prints
/// the login URL and waits for the callback on a loopback port rather than prompting for a pasted
/// callback URL. That port is carried in the URL's `state` parameter.
async fn login(args: &Args, ssh: &SshMux<'_, String>) -> Result<()> {
    let mut cmd = ssh.command("timeout");
    cmd.args([LOGIN_TIMEOUT, &args.aspect_bin, "auth", "login"])
        .args(["--deployment", &args.deployment]);
    if let Some(org) = &args.org {
        cmd.args(["--org", org]);
    }
    // The remote command runs under a shell, so fold the CLI's stderr into stdout: we need to see
    // the login URL whichever stream it is printed on.
    let mut child = cmd
        .arg("2>&1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run {} on {}", &args.aspect_bin, &args.host))?;
    let stdout = child.stdout.take().context("failed to open stdout")?;
    let mut lines = BufReader::new(stdout).lines();

    let mut forward = None;
    while let Some(line) = lines.next().await {
        let line = line.context("failed reading login output")?;
        println!("{line}");
        if forward.is_some() {
            continue;
        }
        let Some((url, port)) = login_url(&line) else {
            continue;
        };
        let mut tunnel = ssh
            .forward_local(port)
            .await
            .with_context(|| format!("failed to forward login callback port {port}"))?;
        if let Some(tunnel) = &mut tunnel {
            // The dedicated forward exits promptly if it cannot bind or connect; catch that before
            // sending the user off to a browser whose callback would go nowhere.
            Timer::after(Duration::from_secs(1)).await;
            if let Some(status) = tunnel.try_status()? {
                anyhow::bail!("ssh port forward for login callback exited: {status}");
            }
        }
        forward = Some(tunnel);
        open_browser(url).await;
    }

    let status = child
        .status()
        .await
        .with_context(|| format!("failed waiting for {} auth login", &args.aspect_bin))?;
    drop(forward);
    if !status.success() {
        anyhow::bail!(
            "ssh {} {} auth login: {}",
            args.host,
            args.aspect_bin,
            status
        );
    }
    Ok(())
}

async fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let opened = Command::new(opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success());
    if opened {
        println!("Opened the login URL in your browser.");
    } else {
        eprintln!("Could not open a browser; open the URL above to log in.");
    }
}

/// Whether a credential helper response carries a non-empty `Authorization` header.
fn has_authorization(response: &str) -> bool {
    Regex::new(r#""Authorization"\s*:\s*\[\s*"[^"]"#)
        .expect("valid regex")
        .is_match(response)
}

/// If `line` is the login URL, return it along with the callback port encoded in its `state`
/// parameter (`<nonce>.<port>`).
fn login_url(line: &str) -> Option<(&str, u16)> {
    let url = line.trim();
    if !url.starts_with("https://") {
        return None;
    }
    let state = Regex::new(r"[?&]state=([^&\s]+)")
        .expect("valid regex")
        .captures(url)?
        .get(1)?
        .as_str()
        .replace("%2E", ".")
        .replace("%2e", ".");
    let (_, port) = state.rsplit_once('.')?;
    Some((url, port.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_url_extracts_port_from_state() {
        let line = "  https://auth.example/oauth/authorize?client_id=abc&redirect_uri=https%3A%2F%2Fapp.example%2Fauth%2Fcli%2Fcallback&response_type=code&scope=openid&code_challenge=xyz&code_challenge_method=S256&state=Zm9v-bar_baz.54321";
        assert_eq!(login_url(line), Some((line.trim(), 54321)));
    }

    #[test]
    fn login_url_handles_encoded_dot() {
        let line = "https://auth.example/authorize?client_id=abc&state=nonce%2E8080";
        assert_eq!(login_url(line).map(|(_, p)| p), Some(8080));
    }

    #[test]
    fn login_url_ignores_other_lines() {
        assert_eq!(login_url("Open this URL in a browser to log in:"), None);
        assert_eq!(
            login_url("https://auth.example/authorize?client_id=abc"),
            None
        );
        assert_eq!(
            login_url("https://auth.example/authorize?state=noport"),
            None
        );
        assert_eq!(
            login_url("https://auth.example/authorize?state=n.99999"),
            None
        );
    }

    #[test]
    fn has_authorization_matches_bearer_header_only() {
        assert!(has_authorization(
            r#"{"headers":{"Authorization":["Bearer abc.def"]}}"#
        ));
        assert!(!has_authorization(r#"{"headers":{}}"#));
        assert!(!has_authorization(r#"{"headers":{"Authorization":[]}}"#));
    }
}
