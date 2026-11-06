//! Bounded CLI execution with process-tree cleanup on completion or cancellation.
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;

async fn read_output(stream: impl AsyncRead + Unpin) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    stream
        .take((OUTPUT_LIMIT + 1) as u64)
        .read_to_end(&mut output)
        .await
        .map_err(|_| "The CLI output could not be read".to_string())?;
    if output.len() > OUTPUT_LIMIT {
        return Err("CLI output exceeded the 2 MiB limit".into());
    }
    Ok(output)
}

pub async fn run(mut command: Command, input: Option<String>) -> Result<String, String> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    tree::prepare(&mut command);
    let mut child = command.spawn().map_err(|error| {
        format!("Could not start the official CLI: {error}. Install it and sign in first.")
    })?;
    let pid = child.id().ok_or("The CLI process did not start")?;
    // Declared after child, so cleanup runs before child is dropped. A suspended
    // Windows child cannot create descendants before it joins the owned job.
    let _tree = tree::ProcessTree::attach(pid)?;
    let stdout = child.stdout.take().ok_or("Missing CLI output pipe")?;
    let stderr = child.stderr.take().ok_or("Missing CLI error pipe")?;
    let stdin = child.stdin.take();
    let operation = async {
        let write_input = async move {
            if let (Some(mut pipe), Some(text)) = (stdin, input) {
                pipe.write_all(text.as_bytes())
                    .await
                    .map_err(|_| "The CLI did not accept its task input".to_string())?;
                pipe.shutdown()
                    .await
                    .map_err(|_| "The CLI input pipe did not close".to_string())?;
            }
            Ok::<(), String>(())
        };
        // Drain both pipes while writing stdin; no pipe can deadlock the others.
        let (output, _errors, status, ()) = tokio::try_join!(
            read_output(stdout),
            read_output(stderr),
            async {
                child
                    .wait()
                    .await
                    .map_err(|_| "Could not wait for the CLI process".to_string())
            },
            write_input,
        )?;
        if !status.success() {
            // CLI error bodies may contain credentials or raw account responses.
            return Err(format!(
                "CLI exited with {}. Check the official CLI's sign-in, model access, and permissions. Raw CLI error output is not retained.",
                status.code().map(|code| code.to_string()).unwrap_or_else(|| "a signal".into())
            ));
        }
        Ok(String::from_utf8_lossy(&output).into_owned())
    };
    tokio::time::timeout(Duration::from_secs(900), operation)
        .await
        .map_err(|_| "CLI timed out after 15 minutes; its process tree was stopped".to_string())?
}

#[cfg(unix)]
mod tree {
    use std::os::unix::process::CommandExt;
    use tokio::process::Command;

    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    pub fn prepare(command: &mut Command) {
        command.as_std_mut().process_group(0);
    }
    pub struct ProcessTree(i32);
    impl ProcessTree {
        pub fn attach(pid: u32) -> Result<Self, String> {
            Ok(Self(pid as i32))
        }
    }
    impl Drop for ProcessTree {
        fn drop(&mut self) {
            // SIGKILL cannot be ignored. Kill the group even after the parent
            // exited, so inherited-output pipes and editing grandchildren stop.
            unsafe {
                kill(-self.0, 9);
            }
        }
    }
}

#[cfg(windows)]
mod tree {
    use std::{
        mem::{size_of, zeroed},
        os::windows::process::CommandExt,
        ptr::null,
    };
    use tokio::process::Command;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                THREADENTRY32,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
                JobObjectExtendedLimitInformation, QueryInformationJobObject,
                SetInformationJobObject, TerminateJobObject,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{
                OpenProcess, OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED,
                PROCESS_SET_QUOTA, PROCESS_TERMINATE, THREAD_SUSPEND_RESUME,
            },
        },
    };

    struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub fn prepare(command: &mut Command) {
        command
            .as_std_mut()
            .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    }
    pub struct ProcessTree(OwnedHandle);
    // Windows kernel handles are process-wide and may be used from any thread.
    unsafe impl Send for ProcessTree {}
    impl ProcessTree {
        pub fn attach(pid: u32) -> Result<Self, String> {
            unsafe {
                let job = CreateJobObjectW(null(), null());
                if job.is_null() {
                    return Err("Could not create the CLI process sandbox job".into());
                }
                let guard = Self(OwnedHandle(job));
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) == 0
                {
                    return Err("Could not configure CLI process-tree cleanup".into());
                }
                let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
                if process.is_null() {
                    return Err("Could not open the CLI process for cleanup".into());
                }
                let process = OwnedHandle(process);
                if AssignProcessToJobObject(job, process.0) == 0 {
                    return Err(
                        "Could not attach the CLI to its cleanup job; execution was blocked".into(),
                    );
                }
                let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
                if snapshot == INVALID_HANDLE_VALUE {
                    return Err("Could not inspect the suspended CLI thread".into());
                }
                let snapshot = OwnedHandle(snapshot);
                let mut entry: THREADENTRY32 = zeroed();
                entry.dwSize = size_of::<THREADENTRY32>() as u32;
                let mut found = Thread32First(snapshot.0, &mut entry);
                while found != 0 {
                    if entry.th32OwnerProcessID == pid {
                        let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                        if thread.is_null() {
                            return Err("Could not resume the protected CLI process".into());
                        }
                        let thread = OwnedHandle(thread);
                        if ResumeThread(thread.0) == u32::MAX {
                            return Err("Could not resume the protected CLI process".into());
                        }
                        return Ok(guard);
                    }
                    found = Thread32Next(snapshot.0, &mut entry);
                }
                Err("The CLI process had no resumable thread".into())
            }
        }
    }
    impl Drop for ProcessTree {
        fn drop(&mut self) {
            unsafe {
                TerminateJobObject(self.0 .0, 1);
                // Wait for kernel-reported process-tree termination before the
                // owning future releases its workspace lock on cancellation.
                loop {
                    let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = zeroed();
                    if QueryInformationJobObject(
                        self.0 .0,
                        JobObjectBasicAccountingInformation,
                        &mut info as *mut _ as _,
                        size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                        std::ptr::null_mut(),
                    ) == 0
                        || info.ActiveProcesses == 0
                    {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_output_and_failure_do_not_leak_stderr() {
        let too_large = tokio::io::repeat(b'x').take((OUTPUT_LIMIT + 1) as u64);
        assert!(read_output(too_large).await.unwrap_err().contains("2 MiB"));
        #[cfg(unix)]
        {
            let mut command = Command::new("sh");
            command.args(["-c", "echo secret-test-value >&2; exit 9"]);
            let error = run(command, None).await.unwrap_err();
            assert!(error.contains('9'));
            assert!(!error.contains("secret-test-value"));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_stops_editing_descendants() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("must-not-exist");
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("(sleep 0.4; echo escaped > \"$1\") & wait")
            .arg("test")
            .arg(&target);
        let handle = tokio::spawn(run(command, None));
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.abort();
        let _ = handle.await;
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(
            !target.exists(),
            "A canceled descendant continued modifying the workspace"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_job_stops_a_started_editing_descendant() {
        let directory = tempfile::tempdir().unwrap();
        let child_script = directory.path().join("child.ps1");
        let parent_script = directory.path().join("parent.ps1");
        let target = directory.path().join("must-not-exist");
        let started = directory.path().join("started");
        std::fs::write(&child_script,
            "Set-Content -LiteralPath $env:AGENT_OS_CHILD_STARTED -Value ready\nStart-Sleep -Milliseconds 900\nSet-Content -LiteralPath $env:AGENT_OS_CANCEL_TARGET -Value escaped\n").unwrap();
        std::fs::write(&parent_script,
            "$child = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList @('-NoProfile','-NonInteractive','-File', ('\"' + $env:AGENT_OS_CHILD_SCRIPT + '\"')) -PassThru\n$child.WaitForExit()\n").unwrap();
        let make_command = || {
            let mut command = Command::new("powershell.exe");
            command
                .args(["-NoProfile", "-NonInteractive", "-File"])
                .arg(&parent_script)
                .env("AGENT_OS_CHILD_SCRIPT", &child_script)
                .env("AGENT_OS_CANCEL_TARGET", &target)
                .env("AGENT_OS_CHILD_STARTED", &started);
            command
        };
        // Prove the fixture really launches a descendant that writes the file.
        run(make_command(), None).await.unwrap();
        assert!(target.exists());
        std::fs::remove_file(&target).unwrap();
        std::fs::remove_file(&started).unwrap();
        let handle = tokio::spawn(run(make_command(), None));
        tokio::time::timeout(Duration::from_secs(10), async {
            while !started.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("The descendant did not start");
        handle.abort();
        let _ = handle.await;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(
            !target.exists(),
            "A canceled Windows job descendant continued editing"
        );
    }
}
