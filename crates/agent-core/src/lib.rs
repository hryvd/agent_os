//! Persistent, platform-neutral Agent OS engine. No simulated provider responses.
mod drive;
#[cfg(test)]
mod integration_tests;
mod process;
mod publishing;
mod tools;

use futures_util::StreamExt;
use reqwest::{Client, Url};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    process::Command,
    sync::{Mutex as AsyncMutex, Semaphore},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_HTTP_BYTES: usize = 4 * 1024 * 1024;
const MAX_TASKS: usize = 12;

#[derive(Clone)]
pub struct Core(Arc<Inner>);
struct Inner {
    data_dir: PathBuf,
    db: Mutex<Connection>,
    http: Client,
    cloud: Arc<Semaphore>,
    local: Arc<Semaphore>,
    cancellations: Mutex<HashMap<String, CancellationToken>>,
    workspace_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    drive_auth: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn field<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn required<'a>(v: &'a Value, k: &str) -> Result<&'a str, String> {
    let s = field(v, k).trim();
    if s.is_empty() {
        Err(format!("{k} is required"))
    } else if s.len() > 32768 {
        Err(format!("{k} is too long"))
    } else {
        Ok(s)
    }
}
fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}
fn object(v: &Value, key: &str) -> Result<Value, String> {
    v.get(key)
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| format!("{key} must be an object"))
}

impl Core {
    pub fn new(data_dir: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&data_dir).map_err(err)?;
        let db = Connection::open(data_dir.join("agent-os.sqlite3")).map_err(err)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS entities(kind TEXT NOT NULL,id TEXT NOT NULL,json TEXT NOT NULL,PRIMARY KEY(kind,id));").map_err(err)?;
        let core = Self(Arc::new(Inner {
            data_dir,
            db: Mutex::new(db),
            http: Client::builder()
                .timeout(Duration::from_secs(180))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(err)?,
            cloud: Arc::new(Semaphore::new(2)),
            local: Arc::new(Semaphore::new(1)),
            cancellations: Mutex::new(HashMap::new()),
            workspace_locks: Mutex::new(HashMap::new()),
            drive_auth: Mutex::new(None),
        }));
        // Commands are never silently replayed after a crash.
        for mut task in core.list("tasks")? {
            if ["running", "queued", "planning", "validating"].contains(&field(&task, "status")) {
                task["status"] = json!("interrupted");
                task["error"] = json!("The application stopped before this task completed. Review its workspace before retrying.");
                core.put("tasks", &task)?;
            }
        }
        if core.list("connections")?.is_empty() {
            core.put("connections", &json!({"id":"ollama","name":"Local Ollama","kind":"ollama","baseUrl":"http://127.0.0.1:11434","model":"","hasApiKey":false}))?;
        }
        if core.get("settings", "app").is_err() {
            core.put("settings", &json!({"id":"app","quality":"efficient","reducedMotion":false,"office3d":true,"maxCloudTasks":2,"maxLocalTasks":1}))?;
        }
        Ok(core)
    }

    fn put(&self, kind: &str, value: &Value) -> Result<(), String> {
        let entity_id = required(value, "id")?;
        if entity_id.len() > 128
            || !entity_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(
                "Identifiers must contain at most 128 letters, digits, hyphens, or underscores"
                    .into(),
            );
        }
        self.0.db.lock().map_err(err)?.execute("INSERT INTO entities(kind,id,json) VALUES(?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET json=excluded.json",params![kind,entity_id,value.to_string()]).map_err(err)?;
        Ok(())
    }
    fn get(&self, kind: &str, entity_id: &str) -> Result<Value, String> {
        let db = self.0.db.lock().map_err(err)?;
        let raw: String = db
            .query_row(
                "SELECT json FROM entities WHERE kind=?1 AND id=?2",
                params![kind, entity_id],
                |r| r.get(0),
            )
            .map_err(|_| format!("{kind} item not found"))?;
        serde_json::from_str(&raw).map_err(err)
    }
    fn list(&self, kind: &str) -> Result<Vec<Value>, String> {
        let db = self.0.db.lock().map_err(err)?;
        let mut stmt = db
            .prepare("SELECT json FROM entities WHERE kind=?1 ORDER BY rowid")
            .map_err(err)?;
        let rows = stmt
            .query_map([kind], |r| r.get::<_, String>(0))
            .map_err(err)?;
        rows.map(|r| serde_json::from_str(&r.map_err(err)?).map_err(err))
            .collect()
    }
    fn delete(&self, kind: &str, entity_id: &str) -> Result<(), String> {
        self.0
            .db
            .lock()
            .map_err(err)?
            .execute(
                "DELETE FROM entities WHERE kind=?1 AND id=?2",
                params![kind, entity_id],
            )
            .map_err(err)?;
        Ok(())
    }
    fn workspace_lock(&self, key: &str) -> Result<Arc<AsyncMutex<()>>, String> {
        Ok(self
            .0
            .workspace_locks
            .lock()
            .map_err(err)?
            .entry(key.to_owned())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone())
    }

    pub async fn execute(&self, command: &str, payload: Value) -> Result<Value, String> {
        match command {
            "bootstrap" => Ok(
                json!({"agents":self.list("agents")?,"tasks":self.list("tasks")?,"messages":self.list("messages")?,"connections":self.list("connections")?,"workspaces":self.list("workspaces")?,"settings":self.get("settings","app")?,"capabilities":{"desktop":true,"googleDriveOAuth":true,"antigravity":false,"streaming":false,"delegation":true}}),
            ),
            "save_agent" => self.save_agent(object(&payload, "agent")?),
            "delete_agent" => {
                let entity_id = required(&payload, "id")?;
                if self.list("tasks")?.iter().any(|t| {
                    field(t, "agentId") == entity_id
                        && ["running", "queued", "planning"].contains(&field(t, "status"))
                }) {
                    return Err("Cancel this agent's active tasks first".into());
                }
                if self
                    .list("agents")?
                    .iter()
                    .any(|a| field(a, "supervisorId") == entity_id)
                {
                    return Err("Reassign this agent's workers first".into());
                }
                self.delete("agents", entity_id)?;
                Ok(json!({"ok":true}))
            }
            "save_connection" => self.save_connection(object(&payload, "connection")?),
            "delete_connection" => {
                let entity_id = required(&payload, "id")?;
                if self
                    .list("agents")?
                    .iter()
                    .any(|a| field(a, "provider") == entity_id)
                {
                    return Err("Reassign agents using this connection first".into());
                }
                self.delete("connections", entity_id)?;
                if let Ok(entry) = keyring::Entry::new("agent-os", entity_id) {
                    let _ = entry.delete_credential();
                }
                Ok(json!({"ok":true}))
            }
            "test_connection" => self.test_connection(required(&payload, "id")?).await,
            "save_workspace" => self.save_workspace(object(&payload, "workspace")?),
            "send_message" | "start_task" => {
                let prompt = if command == "send_message" {
                    required(&payload, "content")?
                } else {
                    required(&payload, "prompt")?
                };
                let (message, task) = self.enqueue(
                    required(&payload, "agentId")?,
                    prompt,
                    payload.get("parentId").and_then(Value::as_str),
                )?;
                if command == "send_message" {
                    Ok(json!({"message":message,"task":task}))
                } else {
                    Ok(task)
                }
            }
            "delegate_task" => self.enqueue_team(
                required(&payload, "agentId")?,
                required(&payload, "prompt")?,
                payload.get("workerIds"),
            ),
            "cancel_task" => {
                let task_id = required(&payload, "id")?;
                self.cancel_tree(task_id)?;
                Ok(self.get("tasks", task_id)?)
            }
            "retry_task" => {
                let task = self.get("tasks", required(&payload, "id")?)?;
                if ["running", "queued", "planning"].contains(&field(&task, "status")) {
                    return Err("Task is already active".into());
                }
                if field(&task, "kind") == "team" {
                    return self.enqueue_team(
                        field(&task, "agentId"),
                        field(&task, "prompt"),
                        None,
                    );
                }
                let (_, task) =
                    self.enqueue(field(&task, "agentId"), field(&task, "prompt"), None)?;
                Ok(task)
            }
            "validate_workspace" => {
                let ws = self.get("workspaces", required(&payload, "id")?)?;
                let lock = self.workspace_lock(field(&ws, "path"))?;
                let _guard = lock.lock().await;
                let mut receipt = publishing::validate(&ws).await?;
                receipt["id"] = json!(field(&ws, "id"));
                self.put("validations", &receipt)?;
                if receipt.get("ok").and_then(Value::as_bool) == Some(true)
                    && ws.get("autoPublish").and_then(Value::as_bool) == Some(true)
                {
                    let publication = publishing::publish(&ws, &receipt).await?;
                    self.delete("validations", field(&ws, "id"))?;
                    receipt["publication"] = publication;
                }
                Ok(receipt)
            }
            "publish_workspace" => {
                let ws = self.get("workspaces", required(&payload, "id")?)?;
                let lock = self.workspace_lock(field(&ws, "path"))?;
                let _guard = lock.lock().await;
                let receipt = self
                    .get("validations", field(&ws, "id"))
                    .map_err(|_| "Run validation successfully before publishing".to_string())?;
                let result = publishing::publish(&ws, &receipt).await?;
                // A receipt is consumed: a new candidate must be revalidated.
                self.delete("validations", field(&ws, "id"))?;
                Ok(result)
            }
            "update_settings" => {
                let mut settings = object(&payload, "settings")?;
                settings["id"] = json!("app");
                settings["maxCloudTasks"] = json!(2);
                settings["maxLocalTasks"] = json!(1);
                self.put("settings", &settings)?;
                Ok(settings)
            }
            "export_snapshot" => {
                let ws = self.get("workspaces", required(&payload, "workspaceId")?)?;
                let lock = self.workspace_lock(field(&ws, "path"))?;
                let _guard = lock.lock().await;
                publishing::snapshot(&ws, required(&payload, "destination")?).await
            }
            "connect_drive" => {
                let client_id = required(&payload, "clientId")?.to_owned();
                let secret = payload
                    .get("clientSecret")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                if let Some(previous) = self.0.drive_auth.lock().map_err(err)?.take() {
                    previous.abort();
                }
                let session = drive::connect(client_id.clone(), secret).await?;
                let url = session.authorization_url.clone();
                self.put(
                    "drive",
                    &json!({"id":"status","pending":true,"clientId":client_id}),
                )?;
                let core = self.clone();
                let pending = tokio::spawn(async move {
                    let mut result = match session.finish().await {
                        Ok(status) => status,
                        Err(error) => json!({"connected":false,"error":error}),
                    };
                    result["id"] = json!("status");
                    result["pending"] = json!(false);
                    result["clientId"] = json!(client_id);
                    let _ = core.put("drive", &result);
                });
                *self.0.drive_auth.lock().map_err(err)? = Some(pending);
                Ok(json!({"authorizationUrl":url,"url":url}))
            }
            "drive_status" => {
                let mut status = drive::status()?;
                if let Ok(last) = self.get("drive", "status") {
                    status["pending"] = last["pending"].clone();
                    status["error"] = last["error"].clone();
                    status["clientId"] = last["clientId"].clone();
                }
                Ok(status)
            }
            "disconnect_drive" => {
                if let Some(pending) = self.0.drive_auth.lock().map_err(err)?.take() {
                    pending.abort();
                }
                let result = drive::disconnect()?;
                self.delete("drive", "status")?;
                Ok(result)
            }
            "upload_drive_snapshot" => {
                let ws = self.get("workspaces", required(&payload, "workspaceId")?)?;
                let config = self
                    .get("drive", "status")
                    .map_err(|_| "Connect Google Drive first".to_string())?;
                let dir = self.0.data_dir.join("snapshots");
                std::fs::create_dir_all(&dir).map_err(err)?;
                let destination = dir.join(format!("{}-{}.zip", field(&ws, "id"), now()));
                let lock = self.workspace_lock(field(&ws, "path"))?;
                let guard = lock.lock().await;
                publishing::snapshot(&ws, &destination.to_string_lossy()).await?;
                drop(guard);
                let result = drive::upload_snapshot(
                    field(&config, "clientId"),
                    &destination,
                    payload
                        .get("folderId")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty()),
                )
                .await;
                if result.is_ok() {
                    let _ = std::fs::remove_file(destination);
                }
                result
            }
            _ => Err(format!("Unsupported command: {command}")),
        }
    }

    fn save_agent(&self, mut agent: Value) -> Result<Value, String> {
        required(&agent, "name")?;
        if field(&agent, "id").is_empty() {
            agent["id"] = json!(id());
        }
        let provider = required(&agent, "provider")?;
        self.connection(provider)?;
        if !field(&agent, "workspaceId").is_empty() {
            self.get("workspaces", field(&agent, "workspaceId"))?;
        }
        let supervisor = field(&agent, "supervisorId");
        if !supervisor.is_empty() {
            if supervisor == field(&agent, "id") {
                return Err("An agent cannot supervise itself".into());
            }
            let parent = self.get("agents", supervisor)?;
            if !field(&parent, "supervisorId").is_empty() {
                return Err("This release supports one supervisor level".into());
            }
            if self
                .list("agents")?
                .iter()
                .any(|a| field(a, "supervisorId") == field(&agent, "id"))
            {
                return Err("A supervisor with workers cannot become a worker".into());
            }
        }
        agent["createdAt"] = agent.get("createdAt").cloned().unwrap_or(json!(now()));
        self.put("agents", &agent)?;
        Ok(agent)
    }

    fn connection(&self, name: &str) -> Result<Value, String> {
        self.get("connections", name).or_else(|_| {
            self.list("connections")?
                .into_iter()
                .find(|c| field(c, "kind") == name)
                .ok_or("Select a configured connection".into())
        })
    }

    fn save_connection(&self, mut connection: Value) -> Result<Value, String> {
        required(&connection, "name")?;
        let kind = required(&connection, "kind")?.to_owned();
        if !["ollama", "openai", "omniroute", "codex", "claude"].contains(&kind.as_str()) {
            return Err("Unsupported connection type".into());
        }
        if field(&connection, "id").is_empty() {
            connection["id"] = json!(id());
        }
        let connection_id = field(&connection, "id").to_owned();
        if !["codex", "claude"].contains(&kind.as_str()) {
            connection["baseUrl"] = json!(validate_url(required(&connection, "baseUrl")?)?);
        }
        let prior = self.get("connections", &connection_id).ok();
        let supplied = connection
            .get("apiKey")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut has_key = prior
            .as_ref()
            .and_then(|c| c.get("hasApiKey"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(secret) = supplied.filter(|s| !s.is_empty()) {
            if secret.len() > 16384 {
                return Err("API key is too long".into());
            }
            keyring::Entry::new("agent-os",&connection_id).map_err(err)?.set_password(&secret).map_err(|_|"Secure credential storage is unavailable. No key was saved. On Linux, start a Secret Service keyring; Windows uses Credential Manager.".to_string())?;
            has_key = true;
        }
        connection = json!({"id":connection_id,"name":connection["name"],"kind":kind,"baseUrl":field(&connection,"baseUrl"),"model":field(&connection,"model"),"hasApiKey":has_key});
        // Credentials are referenced by id, never serialized to SQLite.
        self.put("connections", &connection)?;
        Ok(connection)
    }
    fn secret(&self, connection: &Value) -> Result<Option<String>, String> {
        if connection.get("hasApiKey").and_then(Value::as_bool) != Some(true) {
            return Ok(None);
        }
        keyring::Entry::new("agent-os",field(connection,"id")).map_err(err)?.get_password().map(Some).map_err(|_|"The credential could not be read from the OS keyring. Save the connection key again.".to_string())
    }
    fn save_workspace(&self, mut ws: Value) -> Result<Value, String> {
        required(&ws, "name")?;
        let path = PathBuf::from(required(&ws, "path")?)
            .canonicalize()
            .map_err(|_| "Workspace folder does not exist or is inaccessible".to_string())?;
        if !path.is_dir() {
            return Err("Workspace must be a folder".into());
        }
        if path.parent().is_none() {
            return Err("Choose a project folder rather than an entire drive".into());
        }
        ws["path"] = json!(path.to_string_lossy());
        if field(&ws, "id").is_empty() {
            ws["id"] = json!(id());
        }
        for existing in self.list("workspaces")? {
            if field(&existing, "id") != field(&ws, "id")
                && field(&existing, "path") == field(&ws, "path")
            {
                return Err("This folder is already registered".into());
            }
        }
        self.delete("validations", field(&ws, "id"))?;
        self.put("workspaces", &ws)?;
        Ok(ws)
    }

    async fn test_connection(&self, connection_id: &str) -> Result<Value, String> {
        let connection = self.connection(connection_id)?;
        let kind = field(&connection, "kind");
        if ["codex", "claude"].contains(&kind) {
            let mut cmd = Command::new(kind);
            cmd.arg("--version");
            let output = process::run(cmd, None).await?;
            return Ok(
                json!({"ok":true,"detail":format!("{} — installed. Sign in using the official CLI before running tasks.",output.trim()),"models":[]}),
            );
        }
        let suffix = if kind == "ollama" {
            "api/tags"
        } else {
            "models"
        };
        let req = self.authorized(self.0.http.get(endpoint(&connection, suffix)?), &connection)?;
        let data = response_json(req.send().await.map_err(|e| safe_network_error(&e))?).await?;
        let models: Vec<Value> = if kind == "ollama" {
            data.get("models")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|m| m.get("name").cloned()).collect())
                .unwrap_or_default()
        } else {
            data.get("data")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|m| m.get("id").cloned()).collect())
                .unwrap_or_default()
        };
        Ok(
            json!({"ok":true,"detail":format!("Connected. {} models available.",models.len()),"models":models}),
        )
    }
    fn authorized(
        &self,
        req: reqwest::RequestBuilder,
        connection: &Value,
    ) -> Result<reqwest::RequestBuilder, String> {
        Ok(if let Some(key) = self.secret(connection)? {
            req.bearer_auth(key)
        } else {
            req
        })
    }

    fn enqueue(
        &self,
        agent_id: &str,
        prompt: &str,
        parent_id: Option<&str>,
    ) -> Result<(Value, Value), String> {
        if prompt.trim().is_empty() || prompt.len() > 32768 {
            return Err("Message must contain 1–32768 characters".into());
        }
        let agent = self.get("agents", agent_id)?;
        self.connection(field(&agent, "provider"))?;
        let active = self
            .list("tasks")?
            .iter()
            .filter(|t| ["queued", "running", "planning"].contains(&field(t, "status")))
            .count();
        if active >= MAX_TASKS {
            return Err("The task queue is full (12 tasks). Wait or cancel a task.".into());
        }
        if let Some(parent_id) = parent_id {
            let parent = self.get("tasks", parent_id)?;
            if field(&agent, "supervisorId") != field(&parent, "agentId") {
                return Err("Only the assigned supervisor may delegate to this worker".into());
            }
            if !field(&parent, "parentId").is_empty() {
                return Err("Nested delegation is limited to one worker level".into());
            }
            if !["running", "planning"].contains(&field(&parent, "status")) {
                return Err("Cannot delegate under an inactive task".into());
            }
        }
        let message =
            json!({"id":id(),"agentId":agent_id,"role":"user","content":prompt,"createdAt":now()});
        self.put("messages", &message)?;
        let history: Vec<Value> = self
            .list("messages")?
            .into_iter()
            .filter(|m| field(m, "agentId") == agent_id && field(m, "id") != field(&message, "id"))
            .collect();
        let context = history
            .into_iter()
            .rev()
            .take(11)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|m| json!({"role":m["role"],"content":m["content"]}))
            .collect::<Vec<_>>();
        let task = json!({"id":id(),"agentId":agent_id,"workspaceId":field(&agent,"workspaceId"),"parentId":parent_id,"prompt":prompt,"context":context,"status":"queued","createdAt":now(),"output":"","error":""});
        self.put("tasks", &task)?;
        let token = CancellationToken::new();
        self.0
            .cancellations
            .lock()
            .map_err(err)?
            .insert(field(&task, "id").to_owned(), token.clone());
        let core = self.clone();
        let queued_task = task.clone();
        tokio::spawn(async move {
            core.run_task(queued_task, agent, token).await;
        });
        Ok((message, task))
    }

    fn enqueue_team(
        &self,
        agent_id: &str,
        prompt: &str,
        selected: Option<&Value>,
    ) -> Result<Value, String> {
        if prompt.trim().is_empty() || prompt.len() > 32768 {
            return Err("Task must contain 1–32768 characters".into());
        }
        let manager = self.get("agents", agent_id)?;
        if !field(&manager, "supervisorId").is_empty() {
            return Err("Only a supervisor can delegate a team task".into());
        }
        let selected_ids: Option<Vec<&str>> = selected
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty())
            .map(|a| a.iter().filter_map(Value::as_str).collect());
        let workers: Vec<Value> = self
            .list("agents")?
            .into_iter()
            .filter(|a| {
                field(a, "supervisorId") == agent_id
                    && selected_ids
                        .as_ref()
                        .map(|ids| ids.contains(&field(a, "id")))
                        .unwrap_or(true)
            })
            .collect();
        if workers.is_empty() {
            return Err("Create at least one worker assigned to this supervisor first".into());
        }
        if workers.len() > 4 {
            return Err("Select at most four workers for one team task".into());
        }
        let active = self
            .list("tasks")?
            .iter()
            .filter(|t| ["queued", "running", "planning"].contains(&field(t, "status")))
            .count();
        if active + workers.len() + 1 > MAX_TASKS {
            return Err("Not enough room in the bounded task queue".into());
        }
        let task = json!({"id":id(),"kind":"team","agentId":agent_id,"workspaceId":field(&manager,"workspaceId"),"prompt":prompt,"status":"planning","createdAt":now(),"output":"","error":""});
        self.put("tasks", &task)?;
        self.put("messages",&json!({"id":id(),"agentId":agent_id,"role":"user","content":prompt,"createdAt":now(),"taskId":task["id"]}))?;
        let token = CancellationToken::new();
        self.0
            .cancellations
            .lock()
            .map_err(err)?
            .insert(field(&task, "id").to_owned(), token.clone());
        let core = self.clone();
        let team = task.clone();
        tokio::spawn(async move {
            core.run_team(team, manager, workers, token).await;
        });
        Ok(task)
    }

    async fn run_team(
        &self,
        mut task: Value,
        manager: Value,
        workers: Vec<Value>,
        token: CancellationToken,
    ) {
        let result:Result<String,String>=async {
            let connection=self.connection(field(&manager,"provider"))?;
            let slots=if field(&connection,"kind")=="ollama"{self.0.local.clone()}else{self.0.cloud.clone()};
            let permit=tokio::select!{_=token.cancelled()=>return Err("Canceled".into()),p=slots.acquire_owned()=>p.map_err(err)?};
            let roster:Vec<Value>=workers.iter().map(|w|json!({"agentId":w["id"],"name":w["name"],"role":w["role"]})).collect();
            let mut plan_task=task.clone();
            plan_task["planning"]=json!(true);
            plan_task["prompt"]=json!(format!("Plan this task for the assigned workers. Do not execute work or edit files during planning. Return ONLY JSON in this exact shape: {{\"tasks\":[{{\"agentId\":\"worker-id\",\"prompt\":\"specific task and deliverable\"}}]}}. Use each worker at most once. Use only these worker IDs: {}. Maximum four tasks. User request: {}",json!(roster),field(&task,"prompt")));
            let raw=tokio::select!{_=token.cancelled()=>return Err("Canceled".into()),r=self.perform(&manager,&connection,&plan_task)=>r?};
            drop(permit);
            let clean=raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
            let plan:Value=serde_json::from_str(clean).map_err(|_|"Coordinator did not return a valid task plan. No workers were launched.".to_string())?;
            let assignments=plan.get("tasks").and_then(Value::as_array).filter(|a|!a.is_empty() && a.len()<=4).ok_or("Coordinator plan must contain one to four tasks")?;
            let mut seen=std::collections::HashSet::new();
            // Validate the complete plan before launching any worker.
            for assignment in assignments {
                let worker=required(assignment,"agentId")?;required(assignment,"prompt")?;
                if !workers.iter().any(|w|field(w,"id")==worker)||!seen.insert(worker){return Err("Coordinator requested an unassigned or duplicate worker. No workers were launched.".into());}
            }
            if token.is_cancelled(){return Err("Canceled".into());}
            task["status"]=json!("running");task["plan"]=plan.clone();self.put("tasks",&task)?;
            let mut children=Vec::new();
            for assignment in assignments {
                let (_,child)=self.enqueue(field(assignment,"agentId"),field(assignment,"prompt"),Some(field(&task,"id")))?;
                children.push(field(&child,"id").to_owned());
            }
            task["childIds"]=json!(children);self.put("tasks",&task)?;
            loop {
                if token.is_cancelled(){return Err("Canceled".into());}
                let results:Vec<Value>=children.iter().map(|child|self.get("tasks",child)).collect::<Result<_,_>>()?;
                if results.iter().all(|r|!["queued","running"].contains(&field(r,"status"))) {
                    let report=results.iter().map(|r|format!("### {} — {}\n{}{}",field(r,"agentId"),field(r,"status"),field(r,"output"),field(r,"error"))).collect::<Vec<_>>().join("\n\n");
                    if results.iter().any(|r|field(r,"status")!="completed"){task["output"]=json!(report);return Err("One or more worker tasks did not complete. Inspect their results before retrying.".into());}
                    return Ok(report);
                }
                tokio::select!{_=token.cancelled()=>return Err("Canceled".into()),_=tokio::time::sleep(Duration::from_millis(250))=>{}}
            }
        }.await;
        task["finishedAt"] = json!(now());
        match result {
            Ok(output) => {
                task["status"] = json!("completed");
                task["output"] = json!(output);
                let _=self.put("messages",&json!({"id":id(),"agentId":manager["id"],"role":"assistant","content":output,"taskId":task["id"],"createdAt":now()}));
            }
            Err(error) => {
                task["status"] = json!(if token.is_cancelled() {
                    "canceled"
                } else {
                    "failed"
                });
                task["error"] = json!(error);
                let _ = self.cancel_tree(field(&task, "id"));
            }
        }
        let _ = self.put("tasks", &task);
        if let Ok(mut map) = self.0.cancellations.lock() {
            map.remove(field(&task, "id"));
        }
    }

    fn cancel_tree(&self, task_id: &str) -> Result<(), String> {
        let mut task = self.get("tasks", task_id)?;
        if !["completed", "failed", "canceled", "interrupted"].contains(&field(&task, "status")) {
            if let Some(token) = self.0.cancellations.lock().map_err(err)?.get(task_id) {
                token.cancel();
            }
            task["status"] = json!("canceled");
            task["finishedAt"] = json!(now());
            self.put("tasks", &task)?;
        }
        for child in self.list("tasks")? {
            if field(&child, "parentId") == task_id {
                self.cancel_tree(field(&child, "id"))?;
            }
        }
        Ok(())
    }
    async fn run_task(&self, mut task: Value, agent: Value, token: CancellationToken) {
        let task_id = field(&task, "id").to_owned();
        let result:Result<String,String>=async {
            let connection=self.connection(field(&agent,"provider"))?;
            let local=field(&connection,"kind")=="ollama";
            let slots=if local {self.0.local.clone()} else {self.0.cloud.clone()};
            let _permit=tokio::select! {_=token.cancelled()=>return Err("Canceled".into()),p=slots.acquire_owned()=>p.map_err(err)?};
            if token.is_cancelled() {return Err("Canceled".into());}
            task["status"]=json!("running"); task["startedAt"]=json!(now()); self.put("tasks",&task)?;
            tokio::select! {
                _=token.cancelled()=>Err("Canceled".into()),
                result=tokio::time::timeout(Duration::from_secs(900),self.perform(&agent,&connection,&task))=>result.map_err(|_|"Task timed out after 15 minutes".to_string())?,
            }
        }.await;
        task["finishedAt"] = json!(now());
        match result {
            Ok(output) if !token.is_cancelled() => {
                task["status"] = json!("completed");
                task["output"] = json!(output);
                let _=self.put("messages",&json!({"id":id(),"agentId":field(&agent,"id"),"taskId":task_id,"role":"assistant","content":output,"createdAt":now()}));
            }
            Ok(_) => {
                task["status"] = json!("canceled");
            }
            Err(error) => {
                task["status"] = json!(if token.is_cancelled() {
                    "canceled"
                } else {
                    "failed"
                });
                task["error"] = json!(error);
            }
        }
        if let Ok(current) = self.get("tasks", &task_id) {
            if let Some(log) = current.get("toolCalls") {
                task["toolCalls"] = log.clone();
            }
        }
        let _ = self.put("tasks", &task);
        if let Ok(mut map) = self.0.cancellations.lock() {
            map.remove(&task_id);
        }
    }

    async fn perform(
        &self,
        agent: &Value,
        connection: &Value,
        task: &Value,
    ) -> Result<String, String> {
        let kind = field(connection, "kind");
        let mut instructions = format!(
            "You are {}. Role: {}. {}",
            field(agent, "name"),
            field(agent, "role"),
            field(agent, "instructions")
        );
        if agent.get("ponytail").and_then(Value::as_bool) == Some(true) {
            instructions.push_str("\nUse the simplest maintainable solution. Reuse existing code and standard libraries. Preserve validation, security, accessibility, and error handling. Verify consequential changes. This is an application instruction profile; it does not claim that an external Ponytail plugin is installed.");
        }
        if ["codex", "claude"].contains(&kind) {
            let workspace = self
                .get("workspaces", field(agent, "workspaceId"))
                .map_err(|_| "Select a workspace before running a coding CLI".to_string())?;
            let path = PathBuf::from(field(&workspace, "path"))
                .canonicalize()
                .map_err(err)?;
            if !path.is_dir() {
                return Err("Workspace no longer exists".into());
            }
            let lock = self.workspace_lock(&path.to_string_lossy())?;
            let _guard = lock.lock().await;
            let mut cmd = Command::new(kind);
            cmd.current_dir(&path);
            let prompt = format!("{}\n\nTask:\n{}", instructions, field(task, "prompt"));
            if kind == "codex" {
                cmd.args([
                    "exec",
                    "--json",
                    "--sandbox",
                    if task.get("planning").and_then(Value::as_bool) == Some(true) {
                        "read-only"
                    } else {
                        "workspace-write"
                    },
                    "--skip-git-repo-check",
                    "-",
                ]);
            } else {
                cmd.args(["-p", "--output-format", "json"]);
                if task.get("planning").and_then(Value::as_bool) == Some(true) {
                    cmd.args(["--permission-mode", "plan"]);
                }
            }
            let model = field(agent, "model");
            if !model.is_empty() {
                cmd.args(["--model", model]);
            }
            let raw = process::run(cmd, Some(prompt)).await?;
            return parse_cli(kind, &raw);
        }
        let mut messages = vec![json!({"role":"system","content":instructions})];
        // Bound context: retain recent messages instead of loading an unbounded thread into a local model.
        if let Some(context) = task.get("context").and_then(Value::as_array) {
            messages.extend(context.iter().cloned());
        }
        messages.push(json!({"role":"user","content":field(task,"prompt")}));
        let model = if !field(agent, "model").is_empty() {
            field(agent, "model")
        } else {
            field(connection, "model")
        };
        if model.is_empty() {
            return Err("Choose an installed/available model for this agent".into());
        }
        let workspace = if task.get("planning").and_then(Value::as_bool) != Some(true)
            && !field(agent, "workspaceId").is_empty()
        {
            Some(self.get("workspaces", field(agent, "workspaceId"))?)
        } else {
            None
        };
        let root = workspace
            .as_ref()
            .map(|ws| PathBuf::from(field(ws, "path")).canonicalize().map_err(err))
            .transpose()?;
        let lock = if let Some(root) = root.as_ref() {
            Some(self.workspace_lock(&root.to_string_lossy())?)
        } else {
            None
        };
        let _guard = if let Some(lock) = lock.as_ref() {
            Some(lock.lock().await)
        } else {
            None
        };
        for _round in 0..8 {
            let mut body = if kind == "ollama" {
                json!({"model":model,"messages":messages,"stream":false,"keep_alive":"1m","options":{"num_ctx":4096}})
            } else {
                json!({"model":model,"messages":messages,"stream":false,"max_tokens":2048})
            };
            if root.is_some() {
                body["tools"] = json!(tools::definitions());
            }
            let suffix = if kind == "ollama" {
                "api/chat"
            } else {
                "chat/completions"
            };
            let req = self.authorized(
                self.0.http.post(endpoint(connection, suffix)?).json(&body),
                connection,
            )?;
            let data = response_json(req.send().await.map_err(|e| safe_network_error(&e))?).await?;
            let answer = if kind == "ollama" {
                data.get("message")
            } else {
                data.pointer("/choices/0/message")
            }
            .ok_or("Provider response contained no message. Check endpoint/model compatibility.")?;
            let calls = answer.get("tool_calls").and_then(Value::as_array);
            if let Some(calls) = calls.filter(|a| !a.is_empty()) {
                let root = root
                    .as_ref()
                    .ok_or("Model requested a file tool without an assigned workspace")?;
                if calls.len() > 8 {
                    return Err("Model requested too many file operations in one round".into());
                }
                messages.push(answer.clone());
                for call in calls {
                    let name = call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .ok_or("Malformed tool call")?;
                    let raw = call
                        .pointer("/function/arguments")
                        .cloned()
                        .unwrap_or(json!({}));
                    let arguments = if let Some(s) = raw.as_str() {
                        serde_json::from_str::<Value>(s)
                            .map_err(|_| "Model returned invalid tool arguments".to_string())?
                    } else {
                        raw
                    };
                    let result = match tools::execute(root, name, &arguments) {
                        Ok(value) => value,
                        Err(error) => json!({"error":error}),
                    };
                    let mut tool_message = json!({"role":"tool","content":result.to_string()});
                    if kind == "ollama" {
                        tool_message["tool_name"] = json!(name);
                    } else {
                        tool_message["tool_call_id"] = call["id"].clone();
                    }
                    messages.push(tool_message);
                    if let Ok(mut saved) = self.get("tasks", field(task, "id")) {
                        let mut log = saved
                            .get("toolCalls")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default();
                        log.push(json!({"name":name,"path":arguments.get("path"),"ok":result.get("error").is_none(),"at":now()}));
                        saved["toolCalls"] = json!(log);
                        self.put("tasks", &saved)?;
                    }
                }
            } else {
                let content = answer
                    .get("content")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or("Provider returned an empty text response")?;
                return Ok(content.to_owned());
            }
        }
        Err("Agent reached the limit of eight tool rounds. Inspect changed files before continuing.".into())
    }
}

fn validate_url(raw: &str) -> Result<String, String> {
    let parsed =
        Url::parse(raw).map_err(|_| "Connection URL must be a full HTTP(S) URL".to_string())?;
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Put credentials in the API key field, not in the URL".into());
    }
    let local = matches!(
        parsed.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && local) {
        return Err("Use HTTPS for remote connections; HTTP is allowed only on loopback".into());
    }
    Ok(raw.trim_end_matches('/').to_owned())
}
fn endpoint(connection: &Value, suffix: &str) -> Result<String, String> {
    let mut base = validate_url(required(connection, "baseUrl")?)?;
    if field(connection, "kind") != "ollama" && !base.ends_with("/v1") {
        base.push_str("/v1");
    }
    Ok(format!("{base}/{suffix}"))
}
fn safe_network_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "Provider request timed out".into()
    } else if error.is_connect() {
        "Could not connect to provider. Check its address, running state, network access, and trusted certificate.".into()
    } else {
        "Provider request failed. Check its connection settings and network access.".into()
    }
}
async fn response_json(response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Provider returned HTTP {}. Check account access, API key, model, and quota.",
            status.as_u16()
        ));
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Provider response could not be read".to_string())?;
        if body.len() + chunk.len() > MAX_HTTP_BYTES {
            return Err("Provider response exceeded the 4 MiB limit".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|_| "Provider did not return a supported JSON response".into())
}
fn parse_cli(kind: &str, raw: &str) -> Result<String, String> {
    if kind == "claude" {
        let data: Value = serde_json::from_str(raw)
            .map_err(|_| "Claude CLI did not return supported JSON".to_string())?;
        if data.get("is_error").and_then(Value::as_bool) == Some(true) {
            return Err(
                "Claude reported an error; check its sign-in and permission configuration".into(),
            );
        }
        return data
            .get("result")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or("Claude returned no result".into());
    }
    let mut outputs = Vec::new();
    for line in raw.lines() {
        if let Ok(event) = serde_json::from_str::<Value>(line) {
            if event.get("type").and_then(Value::as_str) == Some("item.completed")
                && event.pointer("/item/type").and_then(Value::as_str) == Some("agent_message")
            {
                if let Some(text) = event.pointer("/item/text").and_then(Value::as_str) {
                    outputs.push(text.to_owned());
                }
            }
            if matches!(
                event.get("type").and_then(Value::as_str),
                Some("error" | "turn.failed")
            ) {
                return Err(
                    "Codex reported an error; check its sign-in, model access, and permissions"
                        .into(),
                );
            }
        }
    }
    if outputs.is_empty() {
        Err("Codex returned no completed agent message".into())
    } else {
        Ok(outputs.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_urls_enforce_transport_and_no_embedded_keys() {
        assert!(validate_url("https://example.com/v1").is_ok());
        assert!(validate_url("http://127.0.0.1:11434").is_ok());
        assert!(validate_url("http://remote.example").is_err());
        assert!(validate_url("https://secret@example.com").is_err());
        assert!(validate_url("https://example.com?key=secret").is_err());
    }
    #[test]
    fn state_persists_and_agent_hierarchy_rejects_cycles() {
        let temp = tempfile::tempdir().unwrap();
        let core = Core::new(temp.path().to_owned()).unwrap();
        let manager = core
            .save_agent(json!({"name":"Lead","provider":"ollama"}))
            .unwrap();
        let worker = core
            .save_agent(json!({"name":"Builder","provider":"ollama","supervisorId":manager["id"]}))
            .unwrap();
        let mut invalid = manager.clone();
        invalid["supervisorId"] = worker["id"].clone();
        assert!(core.save_agent(invalid).is_err());
        drop(core);
        let reopened = Core::new(temp.path().to_owned()).unwrap();
        assert_eq!(reopened.list("agents").unwrap().len(), 2);
    }
    #[test]
    fn interrupted_runs_are_not_replayed() {
        let temp = tempfile::tempdir().unwrap();
        let core = Core::new(temp.path().to_owned()).unwrap();
        core.put("tasks", &json!({"id":"run","status":"running"}))
            .unwrap();
        drop(core);
        let reopened = Core::new(temp.path().to_owned()).unwrap();
        assert_eq!(
            reopened.get("tasks", "run").unwrap()["status"],
            "interrupted"
        );
    }
    #[test]
    fn api_key_is_not_written_to_database() {
        let temp = tempfile::tempdir().unwrap();
        let core = Core::new(temp.path().to_owned()).unwrap();
        let result=core.save_connection(json!({"id":"safe","name":"Test","kind":"openai","baseUrl":"https://example.com/v1","apiKey":"test-secret-must-not-persist"}));
        if let Ok(value) = result {
            assert!(value.get("apiKey").is_none());
        }
        let raw = std::fs::read(temp.path().join("agent-os.sqlite3")).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("test-secret-must-not-persist"));
    }
    #[test]
    fn cli_parser_reports_only_real_completed_messages() {
        let result = parse_cli(
            "codex",
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"done"}}"#,
        )
        .unwrap();
        assert_eq!(result, "done");
        assert!(parse_cli("codex", "{}").is_err());
        assert!(parse_cli("claude", r#"{"is_error":true}"#).is_err());
    }
}
