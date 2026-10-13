//! Expand each value once, then invoke the executable without implicit shell parsing.

use codex_app_server_protocol::Thread;
use codex_config::types::AgentCommand;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const OUTPUT_LIMIT: usize = 64 * 1024;

/// Keep cancellation from leaving a shell's children running with captured pipes open.
#[cfg(unix)]
struct KillGroupOnDrop(Option<u32>);

#[cfg(unix)]
impl Drop for KillGroupOnDrop {
    fn drop(&mut self) {
        if let Some(id) = self.0 {
            let _ = codex_utils_pty::process_group::kill_process_group(id);
        }
    }
}

pub(super) fn prepare(config: &AgentCommand, thread: &Thread) -> anyhow::Result<Command> {
    let context = BTreeMap::from([
        ("thread_id", thread.id.clone()),
        ("session_id", thread.session_id.clone()),
        ("cwd", thread.cwd.display().to_string()),
        (
            "title",
            thread
                .name
                .as_deref()
                .unwrap_or(&thread.preview)
                .to_string(),
        ),
        ("model", thread.model.clone().unwrap_or_default()),
        (
            "git_branch",
            thread
                .git_info
                .as_ref()
                .and_then(|git| git.branch.clone())
                .unwrap_or_default(),
        ),
    ]);
    let interpreter = expand(&config.interpreter, &context)?;
    anyhow::ensure!(
        !interpreter.trim().is_empty(),
        "interpreter must not be empty"
    );
    let args = config
        .args
        .iter()
        .map(|arg| expand(arg, &context))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let environment = config
        .environment
        .iter()
        .map(|(key, value)| {
            anyhow::ensure!(
                !key.is_empty() && !key.contains(['=', '\0']),
                "invalid environment variable name {key:?}"
            );
            Ok((key.clone(), expand(value, &context)?))
        })
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    let cwd = match &config.cwd {
        Some(cwd) => {
            let cwd = expand(cwd, &context)?;
            anyhow::ensure!(!cwd.is_empty(), "cwd must not be empty");
            thread.cwd.as_path().join(PathBuf::from(cwd))
        }
        None => thread.cwd.to_path_buf(),
    };
    let mut command = Command::from(codex_utils_process::background_command(interpreter));
    command
        .args(args)
        .envs(environment)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    Ok(command)
}

fn expand(template: &str, context: &BTreeMap<&str, String>) -> anyhow::Result<String> {
    let mut rest = template;
    let mut output = String::new();
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        let end = rest
            .find("}}")
            .ok_or_else(|| anyhow::anyhow!("unclosed template variable"))?;
        let key = rest[..end].trim();
        let value = context
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("unknown template variable {key:?}"))?;
        output.push_str(value);
        rest = &rest[end + 2..];
    }
    output.push_str(rest);
    anyhow::ensure!(
        !output.contains('\0'),
        "command values cannot contain NUL bytes"
    );
    Ok(output)
}

async fn capture(mut stream: impl AsyncRead + Unpin) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = [0; 8192];
    loop {
        let count = stream.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let keep = count.min(OUTPUT_LIMIT.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..keep]);
        truncated |= keep < count;
    }
    // Keep process control sequences out of the terminal and pager.
    let mut output = String::from_utf8_lossy(&bytes)
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect::<String>();
    if truncated {
        output.push_str("\n[output truncated at 64 KiB]\n");
    }
    Ok(output)
}

pub(super) async fn run(mut command: Command) -> anyhow::Result<String> {
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    #[cfg(unix)]
    let mut group = KillGroupOnDrop(child.id());
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("stdout was not captured"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("stderr was not captured"))?;
    let (status, stdout, stderr) =
        tokio::try_join!(child.wait(), capture(stdout), capture(stderr))?;
    #[cfg(unix)]
    {
        group.0 = None;
    }
    let mut output = format!("{status}\n");
    if !stdout.is_empty() {
        output.push_str(&format!("\nstdout:\n{stdout}"));
    }
    if !stderr.is_empty() {
        output.push_str(&format!("\nstderr:\n{stderr}"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn templates_expand_once_and_reject_misspelled_variables() {
        let context = BTreeMap::from([("title", "spaces; $HOME {{cwd}}".to_string())]);
        assert_eq!(
            expand("prefix {{ title }} suffix", &context).unwrap(),
            "prefix spaces; $HOME {{cwd}} suffix"
        );
        assert!(expand("{{missing}}", &context).is_err());
        assert!(expand("{{title", &context).is_err());
        assert!(expand("bad\0value", &context).is_err());
        assert_eq!(
            expand("${SHELL} {single braces}", &context).unwrap(),
            "${SHELL} {single braces}"
        );
    }

    #[tokio::test]
    async fn capture_drains_large_output_and_removes_terminal_controls() {
        let mut bytes = vec![b'x'; OUTPUT_LIMIT + 100];
        bytes[0] = 0x1b;
        let output = capture(bytes.as_slice()).await.unwrap();
        assert!(!output.contains('\x1b'));
        assert_eq!(output.matches('x').count(), OUTPUT_LIMIT - 1);
        assert!(output.ends_with("[output truncated at 64 KiB]\n"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_kills_the_interpreter_and_its_child() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("child.pid");
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "sleep 30 & echo $! > \"$1\"; wait", "test"])
            .arg(&pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let task = tokio::spawn(run(command));
        let child_pid = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(pid) = tokio::fs::read_to_string(&pid_file).await
                    && let Ok(pid) = pid.trim().parse::<libc::pid_t>()
                {
                    break pid;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            // Signal zero only checks whether this known test child still exists.
            while unsafe { libc::kill(child_pid, 0) } == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("canceled command left its child running");
    }
}
