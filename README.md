# Aspect credential sync tool

This tool keeps your [Aspect][0] credentials fresh on a remote Linux VM. It supports two modes, chosen automatically based on the Aspect Workflows version of your configured remote.

### Aspect Workflows v6

v6 deployments authenticate through the [Aspect CLI][3], which must be installed on the VM. The tool asks the VM's Aspect CLI (acting as a Bazel credential helper) whether it holds a valid credential for the remote. If it does not, the tool:

1. Runs `aspect auth configure <remote> --name <deployment>` on the VM if no deployment there owns the remote yet.
2. Runs `aspect auth login --deployment <deployment>` on the VM.
3. Forwards the login's loopback callback port from your machine to the VM over SSH, and opens the login URL in your local browser.

The VM then holds its own credential, including a refresh token, and renews it on its own; nothing is copied off your machine and no local Aspect tooling is required. Re-running the tool is a no-op until the VM's credential can no longer be refreshed.

The deployment name defaults to `stairwell` and can be changed with `--deployment` or the `ASPECT_DEPLOYMENT` environment variable. The Aspect CLI executable on the VM defaults to `aspect` and can be changed with `--aspect-bin` or `ASPECT_BIN`. Pass `--org` if your account belongs to more than one organization.

### Earlier versions

If the remote is not a v6 deployment (`aspect auth configure` rejects it) or the VM has no Aspect CLI, the tool falls back to the original flow. It first checks whether the credentials are expired (unless `--force` is passed), and if so, runs `aspect-credential-helper login` with your configured remote. Then, it reads the credential out of your OS's keychain and stores it in your Linux VM's [keyutils][1] keychain via `ssh devbox keyctl`.

Because we directly call the macOS keychain APIs ourselves, assuming you trust this program, you should be able to push "Always Allow" to prevent from having to type your password twice every time you run this. (For some reason even if you push "Always Allow", you still need to type your password once if this needs to sync your credential.)

## Installation

```sh
cargo install aspect-reauth
```

You may want to customize the default remote name, the default deployment name, or the default names of the Aspect CLI and credential helper binaries. These can be customized by setting the `ASPECT_REMOTE`, `ASPECT_DEPLOYMENT`, `ASPECT_BIN`, and `ASPECT_CREDENTIAL_HELPER` environment variables. If they are set at runtime, they override the defaults; if they are set at build time, they become the defaults for that binary.

E.g.:

```sh
env ASPECT_REMOTE=aw-remote-ext.mydomain.example \
    ASPECT_CREDENTIAL_HELPER=credential-helper \
    cargo install aspect-reauth
```

## FAQ

### Why do it this way?

The industry has standardized on a lot of things. `curl | bash`. Docker Compose. YAML. These things are often expedient. They’re not always good.

The [Aspect credential helper][2] breaks to some extent with the industry standard for provisioning application secrets. The standard is that you run a command in a PTY, the command prints out a link, you click on the link in your laptop's browser, you click some buttons on a couple-few websites, and then you put a secret on your clipboard and paste it back into your PTY. Maybe you remember to clear your clipboard afterwards. This workflow has some number of gaping holes in it, but mostly it works okay, e.g. since many developers are polite and don't try to read or write your OS's clipboard when you don't expect them to.

Aspect tries to do a little better than this. It wants you to run its credential login step directly from your laptop, so that it can capture the secret it needs without it having to go through your clipboard. It also stores this secret in your OS's secret store (macOS keychain, keyutils, etc) instead of a dotfile. This is great from a security perspective, but slightly clunky from a UX perspective. This CLI tool tries to bridge that gap, hopefully making Aspect's approach superior both in terms of security and in terms of UX.

[0]: https://www.aspect.build/
[1]: https://man7.org/linux/man-pages/man7/keyutils.7.html
[2]: https://docs.aspect.build/workflows/features/external-remote/#oidc
[3]: https://aspect.build/docs/cli/authentication
