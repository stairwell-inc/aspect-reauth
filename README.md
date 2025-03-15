# Aspect credential sync tool

This tool syncs your [Aspect][0] credentials with a remote Linux VM. It first checks whether the credentials are expired (unless `--force` is passed), and if so, runs `aspect-credential-helper login` with your configured remote. Then, it reads the credential out of your OS's keychain and stores it in your Linux VM's [keyutils][1] keychain via `ssh devbox keyctl`.

Because we directly call the macOS keychain APIs ourselves, assuming you trust this program, you should be able to push "Always Allow" to prevent from having to type your password twice every time you run this. (For some reason even if you push "Always Allow", you still need to type your password once if this needs to sync your credential.)

## Installation

```sh
cargo build --release &&
  sudo install target/release/aspect-reauth /usr/local/bin
```

## FAQ

### Why do it this way?

The industry has standardized on a lot of things. `curl | bash`. Docker Compose. YAML. These things are often expedient. They’re not always good.

The [Aspect credential helper][2] breaks to some extent with the industry standard for provisioning application secrets. The standard is that you run a command in a PTY, the command prints out a link, you click on the link in your laptop's browser, you click some buttons on a couple-few websites, and then you put a secret on your clipboard and paste it back into your PTY. Maybe you remember to clear your clipboard afterwards. This workflow has some number of gaping holes in it, but mostly it works okay, e.g. since many developers are polite and don't try to read or write your OS's clipboard when you don't expect them to.

Aspect tries to do a little better than this. It wants you to run its credential login step directly from your laptop, so that it can capture the secret it needs without it having to go through your clipboard. It also stores this secret in your OS's secret store (macOS keychain, keyutils, etc) instead of a dotfile. This is great from a security perspective, but slightly clunky from a UX perspective. This CLI tool tries to bridge that gap, hopefully making Aspect's approach superior both in terms of security and in terms of UX.

[0]: https://www.aspect.build/
[1]: https://man7.org/linux/man-pages/man7/keyutils.7.html
[2]: https://docs.aspect.build/workflows/features/external-remote/#oidc
