use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{io::AsyncReadExt, process::Command};

const OUTPUT_LIMIT: usize = 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15 * 60);

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn workspace_root(workspace: &Value) -> Result<PathBuf, String> {
    let path = text(workspace, "path");
    if path.trim().is_empty() {
        return Err("Choose a workspace directory first.".into());
    }
    let root = fs::canonicalize(path).map_err(|e| format!("Cannot open workspace: {e}"))?;
    if !root.is_dir() {
        return Err("The workspace must be a directory.".into());
    }
    Ok(root)
}

async fn bounded_read(mut reader: impl tokio::io::AsyncRead + Unpin) -> Vec<u8> {
    let mut retained = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut omitted = false;
    loop {
        let count = match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let available = OUTPUT_LIMIT.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..count.min(available)]);
        omitted |= count > available;
    }
    if omitted {
        retained.extend_from_slice(b"\n[Further output omitted.]\n");
    }
    retained
}

struct CommandResult {
    success: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl CommandResult {
    fn output(&self) -> String {
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&self.stdout),
            String::from_utf8_lossy(&self.stderr)
        );
        // Git can echo authenticated URLs on failure. Preserve the reason without
        // exposing URL userinfo, query tokens, or signed URL paths in app logs.
        output
            .split_inclusive(char::is_whitespace)
            .map(|segment| {
                let word = segment.trim_end_matches(char::is_whitespace);
                if word.contains("://") {
                    format!("[remote URL]{}", &segment[word.len()..])
                } else {
                    segment.to_owned()
                }
            })
            .collect()
    }
}

async fn run(root: &Path, program: &str, args: &[&str]) -> Result<CommandResult, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.as_std_mut().creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Could not start {program}: {e}"))?;
    let stdout = child.stdout.take().ok_or("Missing command output pipe")?;
    let stderr = child.stderr.take().ok_or("Missing command error pipe")?;
    let stdout_task = tokio::spawn(bounded_read(stdout));
    let stderr_task = tokio::spawn(bounded_read(stderr));
    let result = tokio::time::timeout(COMMAND_TIMEOUT, child.wait()).await;
    match result {
        Ok(Ok(status)) => Ok(CommandResult {
            success: status.success(),
            code: status.code(),
            stdout: stdout_task.await.map_err(|e| e.to_string())?,
            stderr: stderr_task.await.map_err(|e| e.to_string())?,
        }),
        other => {
            if let Some(id) = child.id() {
                #[cfg(windows)]
                let _ = Command::new("taskkill")
                    .args(["/PID", &id.to_string(), "/T", "/F"])
                    .output()
                    .await;
                #[cfg(unix)]
                let _ = Command::new("kill")
                    .args(["-KILL", "--", &format!("-{id}")])
                    .output()
                    .await;
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            match other {
                Ok(Err(e)) => Err(format!("Command failed: {e}")),
                _ => Err(
                    "Command exceeded the 15-minute timeout; its process tree was stopped.".into(),
                ),
            }
        }
    }
}

async fn git(root: &Path, args: &[&str]) -> Result<CommandResult, String> {
    run(root, "git", args).await
}

async fn git_ok(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let result = git(root, args).await?;
    if result.stdout.ends_with(b"\n[Further output omitted.]\n") {
        return Err(
            "Git output exceeded the capture limit; refusing an incomplete repository inventory."
                .into(),
        );
    }
    if result.success {
        Ok(result.stdout)
    } else {
        Err(format!("Git operation failed: {}", result.output().trim()))
    }
}

async fn require_repository(root: &Path) -> Result<(), String> {
    let output = git_ok(root, &["rev-parse", "--show-toplevel"]).await?;
    let actual =
        fs::canonicalize(String::from_utf8_lossy(&output).trim()).map_err(|e| e.to_string())?;
    if actual != root {
        return Err(
            "Select the Git repository root as the workspace before validation or publication."
                .into(),
        );
    }
    Ok(())
}

async fn head(root: &Path) -> Result<String, String> {
    let result = git(root, &["rev-parse", "--verify", "HEAD^{commit}"]).await?;
    if result.success {
        return Ok(String::from_utf8_lossy(&result.stdout).trim().into());
    }
    let branch = git_ok(root, &["symbolic-ref", "--quiet", "HEAD"]).await?;
    let branch = String::from_utf8_lossy(&branch).trim().to_owned();
    let exists = git(root, &["show-ref", "--verify", "--quiet", &branch]).await?;
    if exists.code == Some(1) {
        Ok(String::new())
    } else {
        Err("Cannot resolve the repository HEAD.".into())
    }
}

fn safe_relative(value: &str) -> Result<PathBuf, String> {
    let relative = PathBuf::from(value);
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("The repository contains an unsafe file path.".into());
    }
    Ok(relative)
}

async fn repository_files(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
    // Git's NUL-delimited form preserves spaces and newlines in filenames.
    let output = git_ok(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )
    .await?;
    output
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let path = std::str::from_utf8(part)
                .map_err(|_| "A repository filename is not valid UTF-8.".to_string())?;
            safe_relative(path)
        })
        .collect()
}

async fn fingerprint(root: &Path) -> Result<String, String> {
    let mut hash = Sha256::new();
    for relative in repository_files(root).await? {
        let path = root.join(&relative);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            // A deletion is absent from the candidate tree both before and after staging.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("Cannot inspect {}: {e}", relative.display())),
        };
        let name = relative.to_string_lossy();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        if metadata.file_type().is_symlink() {
            hash.update(b"link");
            hash.update(
                fs::read_link(&path)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .as_bytes(),
            );
        } else if metadata.is_file() {
            if !fs::canonicalize(&path)
                .map_err(|e| e.to_string())?
                .starts_with(root)
            {
                return Err(
                    "A repository file escapes the workspace through a directory symlink.".into(),
                );
            }
            hash.update(b"file");
            hash.update(metadata.len().to_le_bytes());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                hash.update((metadata.permissions().mode() & 0o111).to_le_bytes());
            }
            let mut file = File::open(&path)
                .map_err(|e| format!("Cannot read {}: {e}", relative.display()))?;
            let mut buffer = [0u8; 65536];
            loop {
                let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
        } else {
            return Err("Submodules or special repository entries require separate validation and are not supported for automatic publication.".into());
        }
    }
    Ok(hex::encode(hash.finalize()))
}

struct PublicationTarget {
    remote: String,
    branch: String,
    url: String,
}

impl PublicationTarget {
    fn fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(self.remote.as_bytes());
        hash.update([0]);
        hash.update(self.branch.as_bytes());
        hash.update([0]);
        hash.update(self.url.as_bytes());
        hex::encode(hash.finalize())
    }
}

async fn publication_target(root: &Path, workspace: &Value) -> Result<PublicationTarget, String> {
    let remote = match text(workspace, "remote") {
        "" => "origin",
        other => other,
    };
    if remote.starts_with('-')
        || !remote
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        return Err("Choose a named Git remote, such as origin.".into());
    }
    let urls = git_ok(root, &["remote", "get-url", "--push", "--all", remote]).await?;
    let urls = String::from_utf8(urls).map_err(|_| "The remote URL is not valid UTF-8.")?;
    let urls: Vec<_> = urls.lines().filter(|url| !url.trim().is_empty()).collect();
    if urls.len() != 1 || urls[0].starts_with('-') {
        return Err("Automatic publication requires exactly one push destination.".into());
    }
    if let Ok(url) = reqwest::Url::parse(urls[0]) {
        if (url.scheme() == "https" || url.scheme() == "http")
            && (!url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some())
        {
            return Err("Use Git credential management instead of credentials or query tokens embedded in a remote URL.".into());
        }
    }
    let branch = if text(workspace, "branch").trim().is_empty() {
        String::from_utf8_lossy(
            &git_ok(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).await?,
        )
        .trim()
        .to_owned()
    } else {
        text(workspace, "branch").trim().to_owned()
    };
    if branch.starts_with('-') {
        return Err("Invalid publication branch.".into());
    }
    git_ok(root, &["check-ref-format", "--branch", &branch]).await?;
    Ok(PublicationTarget {
        remote: remote.to_owned(),
        branch,
        url: urls[0].to_owned(),
    })
}

pub async fn validate(workspace: &Value) -> Result<Value, String> {
    let root = workspace_root(workspace)?;
    require_repository(&root).await?;
    let command = text(workspace, "validationCommand").trim();
    if command.is_empty() {
        return Err(
            "Set a validation command before validating or publishing this workspace.".into(),
        );
    }
    let before = fingerprint(&root).await?;
    let before_head = head(&root).await?;
    let target = publication_target(&root, workspace).await.ok();
    #[cfg(windows)]
    let result = run(&root, "cmd.exe", &["/D", "/S", "/C", command]).await?;
    #[cfg(not(windows))]
    let result = run(&root, "sh", &["-lc", command]).await?;
    let after = fingerprint(&root).await?;
    let after_head = head(&root).await?;
    let unchanged = before == after && before_head == after_head;
    let output = if unchanged {
        result.output()
    } else {
        format!("{}\nValidation changed project contents or HEAD. Run validation again against the final candidate.", result.output())
    };
    Ok(json!({
        "ok":result.success && unchanged, "fingerprint":after, "head":after_head,
        "workspacePath":root.to_string_lossy(), "command":command, "validatedAt":now(),
        "publicationTarget":target.as_ref().map(PublicationTarget::fingerprint),
        "exitCode":result.code, "output":output,
        "detail":if !unchanged { "Project changed during validation" } else if result.success { "Configured validation command passed" } else { "Validation command failed" }
    }))
}

pub async fn publish(workspace: &Value, receipt: &Value) -> Result<Value, String> {
    let root = workspace_root(workspace)?;
    require_repository(&root).await?;
    if receipt.get("ok").and_then(Value::as_bool) != Some(true)
        || text(receipt, "command") != text(workspace, "validationCommand").trim()
        || text(receipt, "workspacePath") != root.to_string_lossy()
        || text(receipt, "fingerprint") != fingerprint(&root).await?
        || text(receipt, "head") != head(&root).await?
    {
        return Err("Publication requires a passing validation of this exact project version. Validate again.".into());
    }
    let target = publication_target(&root, workspace).await?;
    if text(receipt, "publicationTarget") != target.fingerprint() {
        return Err("The publication destination changed or was not configured during validation. Validate again.".into());
    }
    git_ok(&root, &["add", "-A", "--", "."]).await?;
    if text(receipt, "fingerprint") != fingerprint(&root).await? {
        return Err(
            "Project changed while preparing publication. Validate again; no push was made.".into(),
        );
    }
    let difference = git(&root, &["diff", "--cached", "--quiet", "--exit-code"]).await?;
    if difference.code == Some(1) {
        git_ok(&root, &["commit", "-m", "Agent OS: verified checkpoint"]).await?;
    } else if !difference.success {
        return Err(format!(
            "Cannot inspect the candidate commit: {}",
            difference.output()
        ));
    }
    let commit = head(&root).await?;
    if commit.is_empty() {
        return Err("There are no project files or commits to publish.".into());
    }
    // Hooks may modify the candidate. Never push their changes without fresh validation.
    if text(receipt, "fingerprint") != fingerprint(&root).await?
        || !git(&root, &["diff", "--quiet", "--exit-code", &commit])
            .await?
            .success
        || !git(&root, &["diff", "--cached", "--quiet", "--exit-code"])
            .await?
            .success
    {
        return Err(
            "The candidate changed during checkpoint creation. Validate again; no push was made."
                .into(),
        );
    }
    // Resolve both the immutable source and destination before the push. A concurrent
    // HEAD/config update must not redirect the already verified operation.
    let destination = format!("{commit}:refs/heads/{}", target.branch);
    let result = git(&root, &["push", "--", &target.url, &destination]).await?;
    if !result.success {
        return Err(format!(
            "Verified locally; push pending. {}",
            result
                .output()
                .replace(&target.url, "[configured remote]")
                .trim()
        ));
    }
    let reference = format!("refs/heads/{}", target.branch);
    let remote_head = git_ok(
        &root,
        &["ls-remote", "--exit-code", &target.url, &reference],
    )
    .await
    .map_err(|error| error.replace(&target.url, "[configured remote]"))?;
    let remote_commit = String::from_utf8_lossy(&remote_head)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_owned();
    if remote_commit != commit {
        return Err("Push completed, but the remote commit could not be confirmed. Refresh before retrying.".into());
    }
    Ok(
        json!({"ok":true,"commit":commit,"branch":target.branch,"remote":target.remote,"publishedAt":now(),"detail":"Remote publication confirmed"}),
    )
}

fn snapshot_excluded(path: &Path) -> bool {
    path.components().any(|part| {
        let Component::Normal(name) = part else {
            return true;
        };
        let name = name.to_string_lossy().to_ascii_lowercase();
        [
            ".git",
            ".ssh",
            ".aws",
            ".azure",
            ".secrets",
            ".netrc",
            ".npmrc",
            ".pypirc",
            ".git-credentials",
            "id_rsa",
            "id_ed25519",
            "node_modules",
            "target",
            ".venv",
            "venv",
            "__pycache__",
            ".next",
            "dist",
            "build",
        ]
        .contains(&name.as_str())
            || name == ".env"
            || (name.starts_with(".env.")
                && ![".env.example", ".env.sample", ".env.template"].contains(&name.as_str()))
            || name.ends_with(".pem")
            || name.ends_with(".key")
            || name.ends_with(".p12")
            || name.ends_with(".pfx")
            || name.ends_with(".sqlite")
            || name.ends_with(".sqlite3")
            || ["credentials.json", "token.json", "agent-os.db"].contains(&name.as_str())
    })
}

pub async fn snapshot(workspace: &Value, destination: &str) -> Result<Value, String> {
    let root = workspace_root(workspace)?;
    let destination = PathBuf::from(destination);
    if destination
        .extension()
        .and_then(|v| v.to_str())
        .map(|v| v.eq_ignore_ascii_case("zip"))
        != Some(true)
    {
        return Err("Choose a .zip filename for the snapshot.".into());
    }
    let parent = fs::canonicalize(
        destination
            .parent()
            .ok_or("Choose a snapshot destination")?,
    )
    .map_err(|e| e.to_string())?;
    if parent.starts_with(&root) {
        return Err(
            "Save the snapshot outside the project to avoid including it in its own archive."
                .into(),
        );
    }
    let destination = parent.join(
        destination
            .file_name()
            .ok_or("Choose a snapshot filename")?,
    );
    let mut files = Vec::new();
    if require_repository(&root).await.is_ok() {
        files.extend(repository_files(&root).await?);
    } else {
        for entry in walkdir::WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                entry
                    .path()
                    .strip_prefix(&root)
                    .map(|path| !snapshot_excluded(path))
                    .unwrap_or(false)
            })
        {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().is_file() {
                files.push(
                    entry
                        .path()
                        .strip_prefix(&root)
                        .map_err(|e| e.to_string())?
                        .to_path_buf(),
                );
            }
        }
    }
    tokio::task::spawn_blocking(move || {
        let file = OpenOptions::new().write(true).create_new(true).open(&destination).map_err(|e| format!("Cannot create snapshot (existing files are not overwritten): {e}"))?;
        let outcome = (|| -> Result<usize, String> {
            let mut archive = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            let mut count = 0;
            for relative in files {
                if snapshot_excluded(&relative) { continue; }
                let path = root.join(&relative);
                let Ok(metadata) = fs::symlink_metadata(&path) else { continue; };
                if !metadata.is_file() || metadata.file_type().is_symlink() { continue; }
                let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
                if !canonical.starts_with(&root) { return Err("A snapshot path escapes the project.".into()); }
                archive.start_file(relative.to_string_lossy().replace('\\', "/"), options).map_err(|e| e.to_string())?;
                let mut input = File::open(path).map_err(|e| e.to_string())?;
                std::io::copy(&mut input, &mut archive).map_err(|e| e.to_string())?;
                count += 1;
            }
            archive.finish().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())?;
            Ok(count)
        })();
        match outcome {
            Ok(count) => Ok(json!({"ok":true,"path":destination.to_string_lossy(),"files":count,"detail":"Local snapshot created. Cloud synchronization requires a connected Drive-synced destination."})),
            Err(error) => { let _ = fs::remove_file(&destination); Err(error) }
        }
    }).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn repository() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        git_ok(directory.path(), &["init", "-q"]).await.unwrap();
        git_ok(directory.path(), &["config", "user.name", "Agent OS test"])
            .await
            .unwrap();
        git_ok(
            directory.path(),
            &["config", "user.email", "tests@example.invalid"],
        )
        .await
        .unwrap();
        directory
    }

    #[tokio::test]
    async fn validation_is_bound_to_contents_and_command() {
        let directory = repository().await;
        fs::write(directory.path().join("file.txt"), "original").unwrap();
        let mut workspace = json!({"path":directory.path(),"validationCommand":"echo validated"});
        let receipt = validate(&workspace).await.unwrap();
        assert_eq!(receipt["ok"], true);
        fs::write(directory.path().join("file.txt"), "changed").unwrap();
        assert!(publish(&workspace, &receipt)
            .await
            .unwrap_err()
            .contains("exact project version"));
        fs::write(directory.path().join("file.txt"), "original").unwrap();
        workspace["validationCommand"] = json!("echo something-else");
        assert!(publish(&workspace, &receipt)
            .await
            .unwrap_err()
            .contains("exact project version"));
    }

    #[tokio::test]
    async fn failed_or_mutating_validation_cannot_publish() {
        let directory = repository().await;
        let workspace = json!({"path":directory.path(),"validationCommand":"exit 7"});
        let receipt = validate(&workspace).await.unwrap();
        assert_eq!(receipt["ok"], false);
        assert_eq!(receipt["exitCode"], 7);
        assert!(publish(&workspace, &receipt).await.is_err());
        let workspace =
            json!({"path":directory.path(),"validationCommand":"echo changed > candidate.txt"});
        assert_eq!(validate(&workspace).await.unwrap()["ok"], false);
    }

    #[tokio::test]
    async fn publishes_verified_contents_including_deletions_to_local_remote() {
        let directory = repository().await;
        let remote = tempfile::tempdir().unwrap();
        git_ok(remote.path(), &["init", "--bare", "-q"])
            .await
            .unwrap();
        git_ok(
            directory.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        )
        .await
        .unwrap();
        fs::write(directory.path().join("first.txt"), "first candidate").unwrap();
        let workspace =
            json!({"path":directory.path(),"validationCommand":"echo validated","branch":"main"});
        let receipt = validate(&workspace).await.unwrap();
        let published = publish(&workspace, &receipt).await.unwrap();
        assert_eq!(published["ok"], true);
        let expected = git_ok(remote.path(), &["rev-parse", "refs/heads/main"])
            .await
            .unwrap();
        assert_eq!(
            published["commit"].as_str().unwrap(),
            String::from_utf8_lossy(&expected).trim()
        );
        fs::remove_file(directory.path().join("first.txt")).unwrap();
        fs::write(directory.path().join("replacement.txt"), "replacement").unwrap();
        let receipt = validate(&workspace).await.unwrap();
        let published = publish(&workspace, &receipt).await.unwrap();
        assert_eq!(published["ok"], true);
        let files = git_ok(
            remote.path(),
            &["ls-tree", "--name-only", "refs/heads/main"],
        )
        .await
        .unwrap();
        assert_eq!(String::from_utf8_lossy(&files).trim(), "replacement.txt");
    }

    #[tokio::test]
    async fn changing_push_destination_invalidates_validation() {
        let directory = repository().await;
        let original = tempfile::tempdir().unwrap();
        let replacement = tempfile::tempdir().unwrap();
        git_ok(original.path(), &["init", "--bare", "-q"])
            .await
            .unwrap();
        git_ok(replacement.path(), &["init", "--bare", "-q"])
            .await
            .unwrap();
        git_ok(
            directory.path(),
            &["remote", "add", "origin", original.path().to_str().unwrap()],
        )
        .await
        .unwrap();
        fs::write(directory.path().join("candidate.txt"), "verified source").unwrap();
        let workspace =
            json!({"path":directory.path(),"validationCommand":"echo validated","branch":"main"});
        let receipt = validate(&workspace).await.unwrap();
        git_ok(
            directory.path(),
            &[
                "remote",
                "set-url",
                "origin",
                replacement.path().to_str().unwrap(),
            ],
        )
        .await
        .unwrap();
        assert!(publish(&workspace, &receipt)
            .await
            .unwrap_err()
            .contains("destination changed"));
        assert!(git(original.path(), &["show-ref"])
            .await
            .unwrap()
            .stdout
            .is_empty());
        assert!(git(replacement.path(), &["show-ref"])
            .await
            .unwrap()
            .stdout
            .is_empty());
    }

    #[tokio::test]
    async fn embedded_remote_credentials_are_not_accepted_for_publication() {
        let directory = repository().await;
        git_ok(
            directory.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://user:secret@example.invalid/project.git?token=private",
            ],
        )
        .await
        .unwrap();
        let error = publication_target(directory.path(), &json!({"branch":"main"}))
            .await
            .err()
            .unwrap();
        assert!(error.contains("credential management"));
        assert!(!error.contains("secret"));
        assert!(!error.contains("private"));
    }

    #[test]
    fn command_errors_do_not_expose_authenticated_urls() {
        let result = CommandResult {success:false,code:Some(1),stdout:vec![],stderr:b"fatal: unable to access 'https://user:secret@example.test/path?token=secret': refused\n".to_vec()};
        let output = result.output();
        assert!(!output.contains("secret"));
        assert!(output.contains("refused"));
    }

    #[tokio::test]
    async fn snapshots_exclude_credentials_and_do_not_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("source.txt"), "project").unwrap();
        fs::write(directory.path().join(".env"), "SECRET=never-export").unwrap();
        fs::write(directory.path().join(".env.example"), "SECRET=").unwrap();
        fs::create_dir(directory.path().join(".ssh")).unwrap();
        fs::write(directory.path().join(".ssh/config"), "IdentityFile private").unwrap();
        fs::write(
            directory.path().join(".npmrc"),
            "//registry.example/:_authToken=secret",
        )
        .unwrap();
        fs::write(
            directory.path().join("certificate.pfx"),
            "private key material",
        )
        .unwrap();
        let path = output.path().join("snapshot.zip");
        let workspace = json!({"path":directory.path()});
        let result = snapshot(&workspace, path.to_str().unwrap()).await.unwrap();
        assert_eq!(result["files"], 2);
        let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        assert!(archive.by_name("source.txt").is_ok());
        assert!(archive.by_name(".env").is_err());
        assert!(archive.by_name(".ssh/config").is_err());
        assert!(archive.by_name(".npmrc").is_err());
        assert!(archive.by_name("certificate.pfx").is_err());
        assert!(snapshot(&workspace, path.to_str().unwrap()).await.is_err());
    }
}
