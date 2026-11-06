//! Google Drive desktop OAuth and bounded-memory snapshot upload.
//! The application supplies its Google Desktop OAuth client ID; tokens stay in
//! the operating system credential store, never in the project or SQLite.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{timeout, Instant},
};
use uuid::Uuid;

const SERVICE: &str = "dev.agentos.desktop";
const ACCOUNT: &str = "google-drive";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SNAPSHOT_FILES: usize = 10_000;
static AUTH_GENERATION: AtomicU64 = AtomicU64::new(0);
static AUTH_STORE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize)]
struct Credentials {
    client_id: String,
    client_secret: Option<String>,
    refresh_token: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
}

pub struct AuthSession {
    pub authorization_url: String,
    listener: TcpListener,
    redirect_uri: String,
    verifier: String,
    state: String,
    client_id: String,
    client_secret: Option<String>,
    generation: u64,
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|_| "Windows credential storage could not be initialized.".into())
}

fn credentials() -> Result<Credentials, String> {
    let secret = entry()?.get_password().map_err(|_| {
        "Google Drive is not connected, or the operating system credential store is unavailable."
            .to_string()
    })?;
    serde_json::from_str(&secret)
        .map_err(|_| "Stored Drive authorization is invalid. Reconnect Google Drive.".into())
}

fn client() -> Result<Client, String> {
    // Keep normal proxy discovery and TLS certificate verification enabled.
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|_| "Could not initialize the secure Drive connection.".into())
}

pub fn status() -> Result<Value, String> {
    match entry()?.get_password() {
        Ok(secret) => {
            let stored: Credentials = serde_json::from_str(&secret)
                .map_err(|_| "Stored Drive authorization is invalid. Reconnect Google Drive.")?;
            Ok(json!({"connected":true,"clientId":stored.client_id}))
        }
        Err(keyring::Error::NoEntry) => Ok(json!({"connected":false})),
        Err(_) => Err("The operating system credential store is unavailable.".into()),
    }
}

pub fn disconnect() -> Result<Value, String> {
    let _guard = AUTH_STORE_LOCK
        .lock()
        .map_err(|_| "Drive credential storage is busy.")?;
    AUTH_GENERATION.fetch_add(1, Ordering::SeqCst);
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(json!({"connected":false})),
        Err(_) => Err("Could not remove Drive authorization from the credential store.".into()),
    }
}

pub async fn connect(
    client_id: String,
    client_secret: Option<String>,
) -> Result<AuthSession, String> {
    let client_id = client_id.trim().to_string();
    if client_id.len() > 512
        || !client_id.ends_with(".apps.googleusercontent.com")
        || !client_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err("Enter a valid Google OAuth client ID configured as a Desktop app.".into());
    }
    // Two independently generated UUIDs give more than the RFC 7636 minimum
    // verifier entropy; URL-safe encoding produces an 86-character verifier.
    let random = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let verifier = URL_SAFE_NO_PAD.encode(random.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| "Could not open the local OAuth callback listener.".to_string())?;
    let port = listener
        .local_addr()
        .map_err(|_| "OAuth listener is unavailable.")?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/oauth2/callback");
    let mut url = Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
        .map_err(|_| "Invalid Google authorization endpoint.")?;
    url.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", "https://www.googleapis.com/auth/drive.file")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    let generation = {
        let _guard = AUTH_STORE_LOCK
            .lock()
            .map_err(|_| "Drive credential storage is busy.")?;
        AUTH_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
    };
    Ok(AuthSession {
        authorization_url: url.to_string(),
        listener,
        redirect_uri,
        verifier,
        state,
        client_id,
        client_secret: client_secret.filter(|s| !s.is_empty()),
        generation,
    })
}

fn callback_code(request: &str, expected_state: &str) -> Result<String, String> {
    let mut line = request.lines().next().unwrap_or("").split_whitespace();
    if line.next() != Some("GET") {
        return Err("Expected an OAuth redirect request.".into());
    }
    let target = line.next().ok_or("Missing OAuth callback path.")?;
    if !target.starts_with('/') || target.starts_with("//") {
        return Err("Invalid OAuth callback path.".into());
    }
    let url = Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| "Invalid OAuth callback URL.")?;
    if url.path() != "/oauth2/callback" {
        return Err("Unexpected OAuth callback path.".into());
    }
    let pairs: Vec<_> = url.query_pairs().collect();
    let states: Vec<_> = pairs.iter().filter(|(k, _)| k == "state").collect();
    if states.len() != 1 || states[0].1 != expected_state {
        return Err("OAuth state did not match this connection attempt.".into());
    }
    if pairs.iter().any(|(k, _)| k == "error") {
        return Err("Google authorization was declined. Start a new connection attempt.".into());
    }
    let codes: Vec<_> = pairs.iter().filter(|(k, _)| k == "code").collect();
    if codes.len() != 1 || codes[0].1.is_empty() || codes[0].1.len() > 4096 {
        return Err("Google did not return a valid authorization code.".into());
    }
    Ok(codes[0].1.to_string())
}

async fn read_callback(stream: &mut TcpStream) -> Result<String, String> {
    let mut request = Vec::new();
    let mut buf = [0u8; 1024];
    while request.len() < 16 * 1024 {
        let count = timeout(Duration::from_secs(5), stream.read(&mut buf))
            .await
            .map_err(|_| "OAuth callback request timed out.")?
            .map_err(|_| "Could not read OAuth callback.")?;
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buf[..count]);
        if request.windows(4).any(|w| w == b"\r\n\r\n") {
            return String::from_utf8(request)
                .map_err(|_| "Invalid OAuth callback encoding.".into());
        }
    }
    Err("OAuth callback request was incomplete or too large.".into())
}

async fn reply(stream: &mut TcpStream, success: bool) {
    let (status, body) = if success {
        (
            "200 OK",
            "Authorization received. Return to Agent OS to check connection status.",
        )
    } else {
        (
            "400 Bad Request",
            "This authorization request was not accepted. Return to Agent OS.",
        )
    };
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn token_request(form: &[(String, String)]) -> Result<TokenResponse, String> {
    let response = client()?
        .post(TOKEN_URL)
        .form(form)
        .send()
        .await
        .map_err(|_| "Google token request failed. Check your Internet connection.".to_string())?;
    if !response.status().is_success() {
        // Do not forward arbitrary response bodies, authorization codes, or tokens.
        return Err(format!("Google authorization failed (HTTP {}). Check the Desktop OAuth client configuration or reconnect the account.", response.status().as_u16()));
    }
    let tokens: TokenResponse = response
        .json()
        .await
        .map_err(|_| "Google returned an invalid token response.".to_string())?;
    if tokens.access_token.is_empty() {
        return Err("Google returned an empty access token. Reconnect the account.".into());
    }
    Ok(tokens)
}

impl AuthSession {
    pub async fn finish(self) -> Result<Value, String> {
        let deadline = Instant::now() + Duration::from_secs(300);
        let code = loop {
            let (mut stream, _) = tokio::time::timeout_at(deadline, self.listener.accept())
                .await
                .map_err(|_| {
                    "Google sign-in expired after five minutes. Start a new connection attempt."
                })?
                .map_err(|_| "Could not receive Google OAuth callback.")?;
            let parsed = match read_callback(&mut stream).await {
                Ok(request) => callback_code(&request, &self.state),
                Err(error) => Err(error),
            };
            reply(&mut stream, parsed.is_ok()).await;
            match parsed {
                Ok(code) => break code,
                Err(error) if error.starts_with("Google authorization was declined") => {
                    return Err(error)
                }
                Err(_) => continue,
            }
        };
        let mut form = vec![
            ("client_id".into(), self.client_id.clone()),
            ("code".into(), code),
            ("code_verifier".into(), self.verifier),
            ("redirect_uri".into(), self.redirect_uri),
            ("grant_type".into(), "authorization_code".into()),
        ];
        if let Some(secret) = &self.client_secret {
            form.push(("client_secret".into(), secret.clone()));
        }
        let tokens = token_request(&form).await?;
        let _guard = AUTH_STORE_LOCK
            .lock()
            .map_err(|_| "Drive credential storage is busy.")?;
        if self.generation != AUTH_GENERATION.load(Ordering::SeqCst) {
            return Err(
                "This Drive sign-in attempt was replaced or disconnected. Start a new attempt."
                    .into(),
            );
        }
        let refresh_token = tokens
            .refresh_token
            .filter(|token| !token.is_empty())
            .ok_or("Google did not return offline access. Reconnect and grant consent.")?;
        let stored = Credentials {
            client_id: self.client_id,
            client_secret: self.client_secret,
            refresh_token,
        };
        let secret =
            serde_json::to_string(&stored).map_err(|_| "Could not encode Drive authorization.")?;
        entry()?.set_password(&secret).map_err(|_| {
            "Could not save Drive authorization in the operating system credential store."
        })?;
        Ok(json!({"connected":true,"clientId":stored.client_id}))
    }
}

async fn access_token(client_id: &str) -> Result<String, String> {
    let stored = credentials()?;
    if !client_id.is_empty() && stored.client_id != client_id {
        return Err(
            "The selected OAuth client differs from the connected account. Reconnect Drive.".into(),
        );
    }
    let mut form = vec![
        ("client_id".into(), stored.client_id),
        ("refresh_token".into(), stored.refresh_token),
        ("grant_type".into(), "refresh_token".into()),
    ];
    if let Some(secret) = stored.client_secret {
        form.push(("client_secret".into(), secret));
    }
    Ok(token_request(&form).await?.access_token)
}

fn valid_upload_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "Google returned an invalid upload address.")?;
    if url.scheme() != "https"
        || url.host_str() != Some("www.googleapis.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
    {
        return Err("Google returned an unexpected upload destination.".into());
    }
    Ok(url)
}

pub async fn upload_snapshot(
    client_id: &str,
    path: &Path,
    folder_id: Option<&str>,
) -> Result<Value, String> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|_| "The local snapshot is unavailable. Export it again before uploading.")?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("Select a nonempty exported snapshot file.".into());
    }
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid snapshot filename.")?;
    if !name.to_ascii_lowercase().ends_with(".zip") {
        return Err("Drive snapshots must be exported ZIP files.".into());
    }
    let mut document = json!({"name":name,"mimeType":"application/zip","appProperties":{"agentOsSnapshot":"true"}});
    if let Some(folder) = folder_id.filter(|s| !s.is_empty()) {
        if folder.len() > 256
            || !folder
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err("The Drive folder ID is invalid.".into());
        }
        document["parents"] = json!([folder]);
    }
    let token = access_token(client_id).await?;
    let http = client()?;
    let begin = http
        .post("https://www.googleapis.com/upload/drive/v3/files")
        .query(&[
            ("uploadType", "resumable"),
            ("fields", "id,name,webViewLink"),
        ])
        .bearer_auth(&token)
        .header("X-Upload-Content-Type", "application/zip")
        .header("X-Upload-Content-Length", metadata.len())
        .json(&document)
        .send()
        .await
        .map_err(|_| {
            "Drive upload could not start. The snapshot remains on disk; retry when online."
        })?;
    if !begin.status().is_success() {
        return Err(format!("Drive rejected snapshot creation (HTTP {}). Check authorization and folder access. The local snapshot is preserved.", begin.status().as_u16()));
    }
    let upload_url = valid_upload_url(
        begin
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or("Google did not supply a resumable upload address.")?,
    )?;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| "Could not read the local snapshot.")?;
    let mut offset = 0u64;
    let size = metadata.len();
    while offset < size {
        let count = (size - offset).min(CHUNK_BYTES as u64) as usize;
        let mut bytes = vec![0; count];
        file.read_exact(&mut bytes)
            .await
            .map_err(|_| "The local snapshot changed during upload. Export a fresh snapshot.")?;
        let response = http
            .put(upload_url.clone())
            .bearer_auth(&token)
            .header("Content-Type", "application/zip")
            .header(
                "Content-Range",
                format!("bytes {}-{}/{}", offset, offset + count as u64 - 1, size),
            )
            .body(bytes)
            .send()
            .await
            .map_err(|_| {
                "Drive upload was interrupted. The snapshot remains on disk; retry when online."
            })?;
        offset += count as u64;
        if offset == size {
            if !response.status().is_success() {
                return Err(format!(
                    "Drive could not finish the upload (HTTP {}). The local snapshot is preserved.",
                    response.status().as_u16()
                ));
            }
            let result: Value = response
                .json()
                .await
                .map_err(|_| "Drive returned an invalid upload receipt.")?;
            let id = result
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or("Drive did not confirm a file ID; upload success is unconfirmed.")?;
            return Ok(
                json!({"uploaded":true,"fileId":id,"name":result.get("name"),"webViewLink":result.get("webViewLink"),"localPath":path.to_string_lossy()}),
            );
        }
        if response.status() != StatusCode::PERMANENT_REDIRECT {
            return Err(format!(
                "Drive upload stopped unexpectedly (HTTP {}). The local snapshot is preserved.",
                response.status().as_u16()
            ));
        }
        let expected_range = format!("bytes=0-{}", offset - 1);
        if response
            .headers()
            .get("range")
            .and_then(|v| v.to_str().ok())
            != Some(expected_range.as_str())
        {
            return Err("Drive acknowledged a different upload offset. The snapshot is preserved; retry the upload.".into());
        }
    }
    Err("Drive upload did not produce a receipt.".into())
}

fn drive_file_id(value: &str) -> Result<&str, String> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err("Invalid Drive file ID.".into());
    }
    Ok(value)
}

async fn bounded_document(mut response: reqwest::Response) -> Result<Value, String> {
    if response
        .content_length()
        .is_some_and(|size| size > 1024 * 1024)
    {
        return Err("Drive response exceeded its 1 MiB limit.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read Drive's response")?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("Drive response exceeded its 1 MiB limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Drive returned invalid JSON.".into())
}

/// Return a bounded page of this application's snapshots, never the user's whole Drive.
pub async fn list_snapshots(client_id: &str) -> Result<Value, String> {
    let token = access_token(client_id).await?;
    let response = client()?
        .get("https://www.googleapis.com/drive/v3/files")
        .bearer_auth(token)
        .query(&[
            (
                "q",
                "trashed = false and appProperties has { key='agentOsSnapshot' and value='true' }",
            ),
            ("fields", "files(id,name,size,modifiedTime),nextPageToken"),
            ("pageSize", "100"),
            ("orderBy", "modifiedTime desc"),
            ("spaces", "drive"),
        ])
        .send()
        .await
        .map_err(|_| "Could not list Drive snapshots. Check your connection.".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Drive could not list snapshots (HTTP {}). Reconnect if authorization expired.",
            response.status().as_u16()
        ));
    }
    let document = bounded_document(response).await?;
    let files = document
        .get("files")
        .and_then(Value::as_array)
        .ok_or("Drive did not supply a snapshot list.")?;
    if files.len() > 100 {
        return Err("Drive returned more snapshots than requested.".into());
    }
    Ok(
        json!({"files":files,"limit":100,"hasMore":document.get("nextPageToken").and_then(Value::as_str).is_some()}),
    )
}

struct TemporaryDownload(std::path::PathBuf);
impl Drop for TemporaryDownload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn restore_destination(destination: &Path) -> Result<std::path::PathBuf, String> {
    let leaf = destination
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Choose a new project folder name.")?;
    let relative = super::tools::relative_path(leaf, false)?;
    if relative.components().count() != 1 {
        return Err("Choose a single new project folder name.".into());
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = super::tools::root_path(parent)?;
    let target = parent.join(relative);
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err("The restore destination already exists. Choose a new folder; existing files are never overwritten.".into());
    }
    Ok(target)
}

/// Download to disk, then reserve a new destination and extract only safe files.
pub async fn restore_snapshot(
    client_id: &str,
    file_id: &str,
    destination: &Path,
) -> Result<Value, String> {
    let file_id = drive_file_id(file_id)?;
    let destination = restore_destination(destination)?;
    let token = access_token(client_id).await?;
    let http = client()?;
    let metadata = http
        .get(format!(
            "https://www.googleapis.com/drive/v3/files/{file_id}"
        ))
        .bearer_auth(&token)
        .query(&[("fields", "id,mimeType,size,appProperties")])
        .send()
        .await
        .map_err(|_| "Could not inspect the Drive snapshot. Check your connection.".to_string())?;
    if !metadata.status().is_success() {
        return Err(format!(
            "Drive could not inspect the snapshot (HTTP {}).",
            metadata.status().as_u16()
        ));
    }
    let metadata = bounded_document(metadata).await?;
    if metadata
        .pointer("/appProperties/agentOsSnapshot")
        .and_then(Value::as_str)
        != Some("true")
        || metadata.get("mimeType").and_then(Value::as_str) != Some("application/zip")
    {
        return Err("Select a ZIP snapshot created by Agent OS.".into());
    }
    if metadata
        .get("size")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u64>().ok())
        .is_some_and(|n| n > MAX_SNAPSHOT_BYTES)
    {
        return Err("This snapshot exceeds the 256 MiB download limit.".into());
    }
    let mut response = http
        .get(format!(
            "https://www.googleapis.com/drive/v3/files/{file_id}"
        ))
        .bearer_auth(token)
        .query(&[("alt", "media")])
        .send()
        .await
        .map_err(|_| "Could not download the Drive snapshot. Check your connection.".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Drive download failed (HTTP {}).",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_SNAPSHOT_BYTES)
    {
        return Err("This snapshot exceeds the 256 MiB download limit.".into());
    }
    // Initialize cleanup only after create_new succeeds; never remove an existing file.
    let temporary;
    let download_path = destination
        .parent()
        .ok_or("Invalid restore parent")?
        .join(format!(".agent-os-download-{}.zip.part", Uuid::new_v4()));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&download_path)
        .await
        .map_err(|_| "Could not create the temporary snapshot download.".to_string())?;
    temporary = TemporaryDownload(download_path);
    let mut total = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        "Drive download was interrupted. Try restoring again when online.".to_string()
    })? {
        total = total
            .checked_add(chunk.len() as u64)
            .ok_or("Snapshot size overflow")?;
        if total > MAX_SNAPSHOT_BYTES {
            return Err("This snapshot exceeds the 256 MiB download limit.".into());
        }
        file.write_all(&chunk)
            .await
            .map_err(|_| "Could not save the downloaded snapshot. Check disk space.".to_string())?;
    }
    file.flush()
        .await
        .map_err(|_| "Could not finish saving the snapshot.".to_string())?;
    drop(file);
    let archive = temporary.0.clone();
    tokio::task::spawn_blocking(move || {
        extract_snapshot(
            &archive,
            &destination,
            MAX_SNAPSHOT_BYTES,
            MAX_SNAPSHOT_BYTES,
            MAX_SNAPSHOT_FILES,
        )
    })
    .await
    .map_err(|_| "Snapshot restoration was interrupted.".to_string())?
}

fn extract_snapshot(
    archive_path: &Path,
    destination: &Path,
    compressed_limit: u64,
    expanded_limit: u64,
    file_limit: usize,
) -> Result<Value, String> {
    use std::{
        fs::{self, File, OpenOptions},
        io::{Read, Write},
    };
    if fs::metadata(archive_path)
        .map_err(|_| "Snapshot download is unavailable")?
        .len()
        > compressed_limit
    {
        return Err("Compressed snapshot exceeds the restore limit.".into());
    }
    let mut archive =
        zip::ZipArchive::new(File::open(archive_path).map_err(|_| "Cannot open snapshot ZIP")?)
            .map_err(|_| "Snapshot is not a supported ZIP archive")?;
    if archive.len() > file_limit {
        return Err("Snapshot contains too many entries.".into());
    }
    let mut expanded = 0u64;
    let mut paths = std::collections::HashSet::new();
    // Validate every entry before reserving a destination or writing any file.
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| "Snapshot contains an unreadable or encrypted entry")?;
        let name = std::str::from_utf8(entry.name_raw())
            .map_err(|_| "Snapshot contains a non-UTF-8 filename")?;
        let relative = super::tools::relative_path(name, false)?;
        if !paths.insert(relative) {
            return Err("Snapshot contains duplicate file paths.".into());
        }
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170000;
            if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                return Err("Snapshot links and special file entries are not supported.".into());
            }
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or("Snapshot expansion overflow")?;
        if expanded > expanded_limit {
            return Err("Expanded snapshot exceeds the restore limit.".into());
        }
    }
    let destination = restore_destination(destination)?;
    // create_dir is the atomic no-overwrite reservation. Never rename over a directory.
    fs::create_dir(&destination)
        .map_err(|_| "Could not reserve the new project folder; it may already exist")?;
    let result = (|| {
        let root = super::tools::root_path(&destination)?;
        let mut files = 0usize;
        let mut written = 0u64;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|_| "Cannot read snapshot entry")?;
            let relative = super::tools::relative_path(
                std::str::from_utf8(entry.name_raw()).map_err(|_| "Invalid snapshot filename")?,
                false,
            )?;
            let target = super::tools::resolve(&root, &relative, true)?;
            if entry.is_dir() {
                match fs::create_dir(&target) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let meta = fs::symlink_metadata(&target)
                            .map_err(|_| "Cannot inspect restored directory")?;
                        if !meta.is_dir() || meta.file_type().is_symlink() {
                            return Err("A restored directory path is unsafe".into());
                        }
                    }
                    Err(_) => return Err("Could not create a restored directory".into()),
                }
                continue;
            }
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&target)
                .map_err(|_| "Could not create a restored file without overwriting")?;
            let mut buffer = [0u8; 65536];
            let mut file_size = 0u64;
            loop {
                let count = entry
                    .read(&mut buffer)
                    .map_err(|_| "Snapshot contents failed integrity verification")?;
                if count == 0 {
                    break;
                }
                written = written
                    .checked_add(count as u64)
                    .ok_or("Snapshot expansion overflow")?;
                file_size += count as u64;
                if written > expanded_limit {
                    return Err("Expanded snapshot exceeds the restore limit.".into());
                }
                output
                    .write_all(&buffer[..count])
                    .map_err(|_| "Could not write restored files; check disk space")?;
            }
            if file_size != entry.size() {
                return Err("Snapshot file size did not match its ZIP metadata.".into());
            }
            files += 1;
        }
        Ok(
            json!({"restored":true,"path":root.to_string_lossy(),"files":files,"bytes":written,"detail":"Restored into a new local project folder. Register it as a workspace to use agents offline."}),
        )
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&destination);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_fixture(path: &Path, entries: &[(&str, &str)]) {
        use std::io::Write;
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, content) in entries {
            zip.start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn restores_snapshot_into_new_folder_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("snapshot.zip");
        zip_fixture(
            &archive,
            &[("README.md", "restored"), ("src/main.txt", "code")],
        );
        let target = temp.path().join("restored");
        let receipt = extract_snapshot(&archive, &target, 1024 * 1024, 1024 * 1024, 10).unwrap();
        assert_eq!(receipt["files"], 2);
        assert_eq!(
            std::fs::read_to_string(target.join("src/main.txt")).unwrap(),
            "code"
        );
        std::fs::write(target.join("keep.txt"), "user data").unwrap();
        assert!(extract_snapshot(&archive, &target, 1024 * 1024, 1024 * 1024, 10).is_err());
        assert_eq!(
            std::fs::read_to_string(target.join("keep.txt")).unwrap(),
            "user data"
        );
    }

    #[test]
    fn rejects_zip_slip_private_paths_windows_aliases_and_symlinks() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("unsafe.zip");
        for bad in [
            "../escape.txt",
            "/absolute.txt",
            "C:/outside.txt",
            "folder\\escape.txt",
            "CON.txt",
            "src/trailing. ",
            ".env",
            ".ssh/id_rsa",
            ".git/config",
            "secrets/private.key",
        ] {
            zip_fixture(&archive, &[(bad, "must not restore")]);
            let target = temp.path().join("output");
            assert!(
                extract_snapshot(&archive, &target, 1024 * 1024, 1024 * 1024, 10).is_err(),
                "accepted {bad}"
            );
            assert!(!target.exists());
        }
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.add_symlink(
            "link",
            "../outside",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.finish().unwrap();
        assert!(extract_snapshot(
            &archive,
            &temp.path().join("output"),
            1024 * 1024,
            1024 * 1024,
            10
        )
        .is_err());
        // A CRC failure during extraction cleans only the newly reserved folder.
        zip_fixture(&archive, &[("ok.txt", "hello")]);
        let mut bytes = std::fs::read(&archive).unwrap();
        let index = bytes
            .windows(5)
            .position(|window| window == b"hello")
            .expect("stored fixture payload");
        bytes[index] = b'X';
        std::fs::File::create(&archive)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        assert!(extract_snapshot(
            &archive,
            &temp.path().join("damaged"),
            1024 * 1024,
            1024 * 1024,
            10
        )
        .is_err());
        assert!(!temp.path().join("damaged").exists());
    }

    #[test]
    fn bounds_restore_compressed_expanded_and_file_count() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("bounded.zip");
        zip_fixture(&archive, &[("one.txt", "12345"), ("two.txt", "12345")]);
        let target = temp.path().join("output");
        assert!(extract_snapshot(&archive, &target, 4, 1024 * 1024, 10).is_err());
        assert!(extract_snapshot(&archive, &target, 1024 * 1024, 9, 10).is_err());
        assert!(extract_snapshot(&archive, &target, 1024 * 1024, 1024 * 1024, 1).is_err());
        assert!(!target.exists());
        assert!(drive_file_id("../../bad").is_err());
        assert!(drive_file_id("valid_drive-id_1").is_ok());
    }

    #[tokio::test]
    async fn desktop_authorization_uses_pkce_and_loopback() {
        let session = connect("test.apps.googleusercontent.com".into(), None)
            .await
            .unwrap();
        let url = Url::parse(&session.authorization_url).unwrap();
        let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("accounts.google.com"));
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(
            params["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(session.verifier.as_bytes()))
        );
        assert!((43..=128).contains(&session.verifier.len()));
        assert_eq!(params["state"], session.state);
        assert_eq!(
            params["scope"],
            "https://www.googleapis.com/auth/drive.file"
        );
        let redirect = Url::parse(&params["redirect_uri"]).unwrap();
        assert_eq!(redirect.host_str(), Some("127.0.0.1"));
        assert!(redirect.port().unwrap() > 0);
        assert_eq!(redirect.path(), "/oauth2/callback");
    }

    #[test]
    fn oauth_callback_validates_path_state_and_code() {
        assert_eq!(
            callback_code(
                "GET /oauth2/callback?state=correct&code=hello%2Bworld HTTP/1.1\r\n\r\n",
                "correct"
            )
            .unwrap(),
            "hello+world"
        );
        assert!(callback_code(
            "GET /oauth2/callback?state=wrong&code=hello HTTP/1.1",
            "correct"
        )
        .is_err());
        assert!(callback_code("GET /other?state=correct&code=hello HTTP/1.1", "correct").is_err());
        assert!(callback_code(
            "GET /oauth2/callback?state=correct&state=wrong&code=hello HTTP/1.1",
            "correct"
        )
        .is_err());
        assert!(callback_code(
            "GET /oauth2/callback?state=correct&error=access_denied HTTP/1.1",
            "correct"
        )
        .unwrap_err()
        .contains("declined"));
        assert!(callback_code(
            "POST /oauth2/callback?state=correct&code=hello HTTP/1.1",
            "correct"
        )
        .is_err());
    }

    #[test]
    fn upload_destination_stays_on_verified_google_https() {
        assert!(
            valid_upload_url("https://www.googleapis.com/upload/drive/v3/files?upload_id=abc")
                .is_ok()
        );
        for invalid in [
            "http://www.googleapis.com/upload",
            "https://evil.example/upload",
            "https://www.googleapis.com.evil.example/upload",
            "https://secret@www.googleapis.com/upload",
            "https://www.googleapis.com:8080/upload",
        ] {
            assert!(valid_upload_url(invalid).is_err());
        }
    }
}
