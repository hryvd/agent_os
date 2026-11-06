use agent_core::Core;
use serde_json::Value;
use std::sync::Arc;
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

fn google_authorization_url(payload: &Value) -> Result<String, String> {
    let value = payload
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "An authorization URL is required".to_string())?;
    let parsed = url::Url::parse(value).map_err(|_| "Invalid authorization URL".to_string())?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("accounts.google.com")
        || parsed.port_or_known_default() != Some(443)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("Only Google account authorization URLs can be opened".to_string());
    }
    Ok(parsed.into())
}

/// Only the Rust core has access to project files, provider credentials and processes.
/// The webview receives sanitized records through this single command boundary.
#[tauri::command]
async fn execute(
    app: tauri::AppHandle,
    core: State<'_, Arc<Core>>,
    command: String,
    payload: Option<Value>,
) -> Result<Value, String> {
    let payload = payload.unwrap_or_else(|| serde_json::json!({}));
    if command == "open_external" {
        let url = google_authorization_url(&payload)?;
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|error| format!("Could not open the authorization browser: {error}"))?;
        return Ok(serde_json::json!({ "ok": true }));
    }
    if command == "pick_folder" {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.dialog().file().pick_folder(move |selection| {
            let path = selection
                .map(|file| file.into_path().map_err(|error| error.to_string()))
                .transpose();
            let _ = sender.send(path);
        });
        return receiver
            .await
            .map_err(|_| "Folder selection was interrupted".to_string())?
            .map(|path| serde_json::json!(path.map(|path| path.to_string_lossy().into_owned())));
    }

    core.execute(&command, payload).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let core = Core::new(data_dir).map_err(std::io::Error::other)?;
            app.manage(Arc::new(core));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![execute])
        .run(tauri::generate_context!())
        .expect("Agent OS could not start");
}

#[cfg(test)]
mod tests {
    use super::google_authorization_url;
    use serde_json::json;

    #[test]
    fn browser_access_is_limited_to_google_https_authorization() {
        assert!(google_authorization_url(
            &json!({"url": "https://accounts.google.com/o/oauth2/v2/auth?client_id=test"})
        )
        .is_ok());
        for url in [
            "http://accounts.google.com/o/oauth2/v2/auth",
            "https://accounts.google.com.evil.example/",
            "https://evil.example/?next=accounts.google.com",
            "https://accounts.google.com:8443/",
            "https://user:secret@accounts.google.com/",
            "file:///C:/Windows/System32/cmd.exe",
        ] {
            assert!(google_authorization_url(&json!({"url": url})).is_err());
        }
        assert!(google_authorization_url(&json!({})).is_err());
    }
}
