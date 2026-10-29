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

mod legacy;
mod ssh_mux;
mod v6;

use std::{str::FromStr, sync::Arc};

use anyhow::{Context, Result};
use clap::Parser;
use ssh_mux::{CreateSocket, SshMux};

const DEFAULT_REMOTE: &str = env!("ASPECT_REMOTE");
const DEFAULT_HELPER: &str = env!("ASPECT_CREDENTIAL_HELPER");
const DEFAULT_ASPECT_BIN: &str = env!("ASPECT_BIN");
const DEFAULT_DEPLOYMENT: &str = env!("ASPECT_DEPLOYMENT");

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// SSH hostname to which to sync credential
    #[arg(default_value = "devbox")]
    host: String,

    /// Aspect remote DNS name
    #[arg(env = "ASPECT_REMOTE", default_value = DEFAULT_REMOTE, long)]
    remote: String,

    /// Aspect credential helper executable name (pre-v6 deployments)
    #[arg(env = "ASPECT_CREDENTIAL_HELPER", default_value = DEFAULT_HELPER, long)]
    credential_helper: String,

    /// Aspect CLI executable name on the SSH host (v6 deployments)
    #[arg(env = "ASPECT_BIN", default_value = DEFAULT_ASPECT_BIN, long)]
    aspect_bin: String,

    /// Name under which the Aspect deployment is configured on the SSH host (v6 deployments)
    #[arg(env = "ASPECT_DEPLOYMENT", default_value = DEFAULT_DEPLOYMENT, long)]
    deployment: String,

    /// Organization to log in to, for accounts that belong to more than one (v6 deployments)
    #[arg(long)]
    org: Option<String>,

    /// Force re-login and sync even if the credentials are still valid
    #[arg(short, long)]
    force: bool,

    /// Force re-login even if the credentials are still valid (pre-v6 deployments)
    #[arg(short = 'l', long, conflicts_with = "force")]
    force_local: bool,

    /// Force remote sync (or, for v6 deployments, remote re-login) even if the credentials are
    /// still valid
    #[arg(short = 'r', long, conflicts_with = "force")]
    force_remote: bool,

    /// Use the session (rather than user) keyring on the VM (pre-v6 deployments)
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

    if v6::run(&args, &ssh).await? {
        return Ok(());
    }
    legacy::run(&args, &ssh).await
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
