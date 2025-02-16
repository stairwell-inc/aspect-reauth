use anyhow::Result;
use std::process::Command;

pub fn has_user_socket(host: &str) -> Result<bool> {
    // Get the output of `ssh -G <host>` this will have a standard
    // lowercase represesntation of:
    //
    // <key> <value>
    //
    // so a basic match should be enough.
    let output = Command::new("ssh").arg("-G").arg(host).output()?;

    if !output.status.success() {
        anyhow::bail!(
            "failed to check for existing control socket: {}\n\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
        );
    }
    let stdout = String::from_utf8(output.stdout)?;

    let mut has_controlmaster_auto = false;
    let mut has_controlpersist = false;

    for line in stdout.lines() {
        let line = line.trim();
        if line == "controlmaster auto" {
            has_controlmaster_auto = true;
        }
        if line.starts_with("controlpersist") {
            has_controlpersist = true;
        }
    }

    Ok(has_controlmaster_auto && has_controlpersist)
}
