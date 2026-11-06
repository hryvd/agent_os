use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_ENTRIES: usize = 200;

pub fn definitions() -> Vec<Value> {
    vec![
        json!({"type":"function","function":{
            "name":"list_files","description":"List up to 200 files and directories inside the approved workspace. Secret files and symlinks are excluded.",
            "parameters":{"type":"object","properties":{"path":{"type":"string","description":"Relative directory path; omit or use an empty string for the workspace root."}},"additionalProperties":false}
        }}),
        json!({"type":"function","function":{
            "name":"read_file","description":"Read a UTF-8 text file inside the approved workspace, up to 256 KiB. Secrets and symlinks are inaccessible.",
            "parameters":{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative file path."}},"required":["path"],"additionalProperties":false}
        }}),
        json!({"type":"function","function":{
            "name":"write_file","description":"Atomically create or replace a UTF-8 text file in the approved workspace, up to 256 KiB. Parent directories are created when necessary. Cannot access secrets or symlinks.",
            "parameters":{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative file path."},"content":{"type":"string","description":"Complete replacement text."}},"required":["path","content"],"additionalProperties":false}
        }}),
    ]
}

fn restricted(path: &Path) -> bool {
    path.components().any(|part| {
        let Component::Normal(part) = part else {
            return true;
        };
        let name = part.to_string_lossy().to_ascii_lowercase();
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
            "credentials.json",
            "token.json",
        ]
        .contains(&name.as_str())
            || name == ".env"
            || (name.starts_with(".env.")
                && ![".env.example", ".env.sample", ".env.template"].contains(&name.as_str()))
            || [".pem", ".key", ".p12", ".pfx"]
                .iter()
                .any(|extension| name.ends_with(extension))
    })
}

fn relative_path(value: &str, allow_root: bool) -> Result<PathBuf, String> {
    if value.is_empty() || value == "." {
        return if allow_root {
            Ok(PathBuf::new())
        } else {
            Err("A workspace-relative file path is required.".into())
        };
    }
    // Reject Windows aliases even on Unix so trust rules are platform-independent.
    if value.contains('\\') || value.contains(':') || value.contains('\0') {
        return Err("Use a normal relative path with forward slashes.".into());
    }
    let path = PathBuf::from(value);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("Absolute paths and parent-directory traversal are not allowed.".into());
    }
    for part in path.components() {
        let Component::Normal(part) = part else {
            continue;
        };
        let name = part.to_string_lossy();
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        if name.ends_with(['.', ' '])
            || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(
                "Windows device names and trailing-dot/space path aliases are not supported."
                    .into(),
            );
        }
    }
    if restricted(&path) {
        return Err(
            "This path is protected because it may contain credentials or repository control data."
                .into(),
        );
    }
    Ok(path)
}

fn root_path(root: &Path) -> Result<PathBuf, String> {
    let root =
        fs::canonicalize(root).map_err(|e| format!("Cannot open the approved workspace: {e}"))?;
    if !root.is_dir() {
        return Err("The approved workspace is not a directory.".into());
    }
    let normal_components: PathBuf = root
        .components()
        .filter_map(|part| match part {
            Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect();
    if restricted(&normal_components) {
        return Err(
            "A credential or repository-control directory cannot be used as an agent workspace."
                .into(),
        );
    }
    Ok(root)
}

fn resolve(root: &Path, relative: &Path, create_parents: bool) -> Result<PathBuf, String> {
    let mut current = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(part) = component else {
            return Err("Invalid relative path.".into());
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err("Symlink paths are not available to agent file tools.".into());
                }
                let canonical = fs::canonicalize(&current).map_err(|e| e.to_string())?;
                if !canonical.starts_with(root) {
                    return Err("The path escapes the approved workspace.".into());
                }
                if canonical.strip_prefix(root).map(restricted).unwrap_or(true) {
                    return Err("The resolved path refers to protected project data.".into());
                }
                if index + 1 != components.len() && !metadata.is_dir() {
                    return Err("A parent path is not a directory.".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if create_parents && index + 1 != components.len() {
                    fs::create_dir(&current)
                        .map_err(|e| format!("Cannot create a project directory: {e}"))?;
                    let metadata = fs::symlink_metadata(&current).map_err(|e| e.to_string())?;
                    if metadata.file_type().is_symlink()
                        || !fs::canonicalize(&current)
                            .map_err(|e| e.to_string())?
                            .starts_with(root)
                    {
                        return Err("A directory changed while preparing the file write.".into());
                    }
                } else if index + 1 != components.len() {
                    return Err("The requested parent directory does not exist.".into());
                }
            }
            Err(error) => return Err(format!("Cannot inspect a project path: {error}")),
        }
    }
    Ok(current)
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn execute(root: &Path, name: &str, args: &Value) -> Result<Value, String> {
    let root = root_path(root)?;
    let path = args.get("path").and_then(Value::as_str).unwrap_or("");
    let relative = relative_path(path, name == "list_files")?;
    match name {
        "list_files" => {
            let directory = resolve(&root, &relative, false)?;
            let mut entries = Vec::new();
            let mut truncated = false;
            for entry in fs::read_dir(&directory)
                .map_err(|e| format!("Cannot list the project directory: {e}"))?
            {
                let entry = entry.map_err(|e| e.to_string())?;
                let file_type = entry.file_type().map_err(|e| e.to_string())?;
                let relative = relative.join(entry.file_name());
                if restricted(&relative) || file_type.is_symlink() {
                    continue;
                }
                if entries.len() >= MAX_ENTRIES {
                    truncated = true;
                    break;
                }
                entries.push(json!({"path":display_path(&relative),"type":if file_type.is_dir() {"directory"} else {"file"}}));
            }
            entries.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
            Ok(json!({"entries":entries,"truncated":truncated,"limit":MAX_ENTRIES}))
        }
        "read_file" => {
            let path = resolve(&root, &relative, false)?;
            let metadata =
                fs::metadata(&path).map_err(|e| format!("Cannot inspect the file: {e}"))?;
            if !metadata.is_file() {
                return Err("The requested path is not a regular file.".into());
            }
            if metadata.len() > MAX_FILE_BYTES as u64 {
                return Err("The file exceeds the 256 KiB agent read limit.".into());
            }
            let mut data = Vec::new();
            File::open(&path)
                .map_err(|e| e.to_string())?
                .take((MAX_FILE_BYTES + 1) as u64)
                .read_to_end(&mut data)
                .map_err(|e| e.to_string())?;
            if data.len() > MAX_FILE_BYTES {
                return Err("The file grew beyond the 256 KiB read limit.".into());
            }
            if data.contains(&0) {
                return Err("Binary files are not supported by the text file tool.".into());
            }
            let content =
                String::from_utf8(data).map_err(|_| "The file is not UTF-8 text.".to_string())?;
            Ok(json!({"path":display_path(&relative),"content":content}))
        }
        "write_file" => {
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .ok_or("Text content is required.")?;
            if content.len() > MAX_FILE_BYTES {
                return Err("The replacement exceeds the 256 KiB agent write limit.".into());
            }
            if content.contains('\0') {
                return Err("Binary content is not supported by the text file tool.".into());
            }
            let path = resolve(&root, &relative, true)?;
            let old_permissions = match fs::metadata(&path) {
                Ok(metadata) => {
                    if !metadata.is_file() {
                        return Err("Only regular files can be replaced.".into());
                    }
                    if metadata.len() > MAX_FILE_BYTES as u64 {
                        return Err(
                            "The existing file exceeds the 256 KiB replacement limit.".into()
                        );
                    }
                    Some(metadata.permissions())
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.to_string()),
            };
            let parent = path.parent().ok_or("The file has no parent directory.")?;
            let temporary = parent.join(format!(".agent-os-{}.tmp", uuid::Uuid::new_v4()));
            let outcome = (|| -> Result<(), String> {
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)
                    .map_err(|e| e.to_string())?;
                output
                    .write_all(content.as_bytes())
                    .map_err(|e| e.to_string())?;
                if let Some(permissions) = old_permissions {
                    output
                        .set_permissions(permissions)
                        .map_err(|e| e.to_string())?;
                }
                output.sync_all().map_err(|e| e.to_string())?;
                drop(output);
                // Recheck existing ancestors/target immediately before atomic replacement.
                resolve(&root, &relative, false)?;
                fs::rename(&temporary, &path)
                    .map_err(|e| format!("Cannot atomically replace the project file: {e}"))?;
                Ok(())
            })();
            if outcome.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            outcome?;
            Ok(json!({"ok":true,"path":display_path(&relative),"bytesWritten":content.len()}))
        }
        _ => Err("Unsupported agent file tool.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_tools_are_scoped_and_exclude_secrets() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(".env"), "SECRET=private").unwrap();
        fs::write(directory.path().join(".env.example"), "SECRET=").unwrap();
        assert!(execute(directory.path(), "read_file", &json!({"path":".env"})).is_err());
        for path in [
            "../outside",
            "/outside",
            "C:\\outside",
            ".git/config",
            ".git./config",
            "NUL.txt",
            "folder /file",
            "file:stream",
        ] {
            assert!(execute(
                directory.path(),
                "write_file",
                &json!({"path":path,"content":"bad"})
            )
            .is_err());
        }
        let listed = execute(directory.path(), "list_files", &json!({})).unwrap();
        assert_eq!(listed["entries"].as_array().unwrap().len(), 1);
        assert_eq!(listed["entries"][0]["path"], ".env.example");
        execute(
            directory.path(),
            "write_file",
            &json!({"path":"src/main.txt","content":"hello"}),
        )
        .unwrap();
        assert_eq!(
            execute(
                directory.path(),
                "read_file",
                &json!({"path":"src/main.txt"})
            )
            .unwrap()["content"],
            "hello"
        );
        assert!(execute(
            directory.path(),
            "write_file",
            &json!({"path":"src/main.txt","content":"x".repeat(MAX_FILE_BYTES+1)})
        )
        .is_err());
        assert_eq!(
            fs::read_to_string(directory.path().join("src/main.txt")).unwrap(),
            "hello"
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlinks_cannot_escape_read_or_new_write() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        fs::write(external.path().join("private.txt"), "private").unwrap();
        symlink(external.path(), root.path().join("escape")).unwrap();
        assert!(execute(
            root.path(),
            "read_file",
            &json!({"path":"escape/private.txt"})
        )
        .is_err());
        assert!(execute(
            root.path(),
            "write_file",
            &json!({"path":"escape/new/nested.txt","content":"bad"})
        )
        .is_err());
        assert!(!external.path().join("new").exists());
    }
}
