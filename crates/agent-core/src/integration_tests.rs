//! Real HTTP requests to a loopback provider exercise the persistent execution path.
use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

struct MockProvider {
    address: String,
    requests: Arc<Mutex<Vec<Value>>>,
    server: JoinHandle<()>,
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl MockProvider {
    async fn start(respond: impl Fn(&Value) -> Value + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback mock provider bind");
        let address = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let collected = requests.clone();
        let respond = Arc::new(respond);
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let requests = collected.clone();
                let respond = respond.clone();
                tokio::spawn(async move {
                    let mut received = Vec::new();
                    let mut chunk = [0u8; 8192];
                    let headers_end = loop {
                        let count = stream.read(&mut chunk).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        received.extend_from_slice(&chunk[..count]);
                        assert!(received.len() < 2 * 1024 * 1024, "mock request limit");
                        if let Some(end) = received.windows(4).position(|b| b == b"\r\n\r\n") {
                            break end + 4;
                        }
                    };
                    let headers = String::from_utf8_lossy(&received[..headers_end]);
                    let path = headers
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .to_owned();
                    let length: usize = headers
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim().parse().unwrap())
                        .unwrap_or(0);
                    while received.len() < headers_end + length {
                        let count = stream.read(&mut chunk).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        received.extend_from_slice(&chunk[..count]);
                        assert!(received.len() < 2 * 1024 * 1024, "mock request limit");
                    }
                    let body = if length == 0 {
                        Value::Null
                    } else {
                        serde_json::from_slice(&received[headers_end..headers_end + length])
                            .unwrap()
                    };
                    requests
                        .lock()
                        .unwrap()
                        .push(json!({"path":path,"body":body}));
                    let response = if path == "/v1/models" {
                        json!({"data":[{"id":"mock-model"}]})
                    } else {
                        respond(&body)
                    };
                    let serialized = response.to_string();
                    let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",serialized.len());
                    stream.write_all(header.as_bytes()).await.unwrap();
                    stream.write_all(serialized.as_bytes()).await.unwrap();
                    stream.shutdown().await.unwrap();
                });
            }
        });
        Self {
            address,
            requests,
            server,
        }
    }
}

fn completion(content: impl Into<String>) -> Value {
    json!({"choices":[{"message":{"role":"assistant","content":content.into()}}]})
}

fn last_user(body: &Value) -> &str {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|m| field(m, "role") == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap()
}

fn create_core(provider: &MockProvider) -> (Core, tempfile::TempDir) {
    let data = tempfile::tempdir().unwrap();
    let core = Core::new(data.path().to_path_buf()).unwrap();
    core.save_connection(json!({"id":"mock","name":"Loopback test provider","kind":"openai","baseUrl":provider.address,"model":"mock-model"})).unwrap();
    (core, data)
}

fn agent(core: &Core, id: &str, supervisor: &str, workspace: &str) {
    core.save_agent(json!({"id":id,"name":id,"role":"Specialist","provider":"mock","model":"mock-model","workspaceId":workspace,"supervisorId":supervisor,"instructions":"Return the requested deliverable."})).unwrap();
}

async fn finished(core: &Core, task_id: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let task = core.get("tasks", task_id).unwrap();
            if ["completed", "failed", "canceled", "interrupted"].contains(&field(&task, "status"))
            {
                return task;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("task should finish within five seconds")
}

#[tokio::test]
async fn queued_prompts_keep_their_own_context_and_persist_real_responses() {
    let provider =
        MockProvider::start(|body| completion(format!("Reply to {}", last_user(body)))).await;
    let (core, data) = create_core(&provider);
    agent(&core, "assistant", "", "");
    let connection = core
        .execute("test_connection", json!({"id":"mock"}))
        .await
        .unwrap();
    assert_eq!(connection["models"], json!(["mock-model"]));
    // Hold both execution slots until the two prompt snapshots have been persisted.
    let slots = core.0.cloud.clone().acquire_many_owned(2).await.unwrap();
    let first = core
        .execute(
            "send_message",
            json!({"agentId":"assistant","content":"first request"}),
        )
        .await
        .unwrap();
    let second = core
        .execute(
            "send_message",
            json!({"agentId":"assistant","content":"second request"}),
        )
        .await
        .unwrap();
    assert_eq!(first["task"]["status"], "queued");
    assert_eq!(second["task"]["status"], "queued");
    drop(slots);
    let first = finished(&core, field(&first["task"], "id")).await;
    let second = finished(&core, field(&second["task"], "id")).await;
    assert_eq!(first["status"], "completed", "{first}");
    assert_eq!(second["status"], "completed", "{second}");
    assert_eq!(first["output"], "Reply to first request");
    assert_eq!(second["output"], "Reply to second request");
    let requests = provider.requests.lock().unwrap().clone();
    let first_body = &requests
        .iter()
        .find(|r| r["path"] == "/v1/chat/completions" && last_user(&r["body"]) == "first request")
        .unwrap()["body"];
    assert!(!first_body.to_string().contains("second request"));
    assert_eq!(
        first_body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["content"] == "first request")
            .count(),
        1
    );
    let second_body = &requests
        .iter()
        .find(|r| r["path"] == "/v1/chat/completions" && last_user(&r["body"]) == "second request")
        .unwrap()["body"];
    assert_eq!(
        second_body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["content"] == "second request")
            .count(),
        1
    );
    drop(core);
    let restored = Core::new(data.path().to_path_buf()).unwrap();
    assert_eq!(restored.list("messages").unwrap().len(), 4);
    assert!(restored
        .list("tasks")
        .unwrap()
        .iter()
        .all(|t| t["status"] == "completed"));
}

#[tokio::test]
async fn provider_tool_call_writes_a_real_scoped_file_then_returns_its_result() {
    let provider = MockProvider::start(|body| {
        let tool_result = body["messages"].as_array().unwrap().iter().find(|message|message["role"]=="tool");
        if let Some(result) = tool_result {
            let result:Value = serde_json::from_str(result["content"].as_str().unwrap()).unwrap();
            assert_eq!(result["ok"],true);
            completion("Created src/result.txt")
        } else {
            assert_eq!(body["tools"].as_array().unwrap().len(),3);
            json!({"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"write-1","type":"function","function":{"name":"write_file","arguments":json!({"path":"src/result.txt","content":"Actual file from a provider tool call"}).to_string()}}]}}]})
        }
    }).await;
    let (core, _data) = create_core(&provider);
    let directory = tempfile::tempdir().unwrap();
    core.save_workspace(json!({"id":"project","name":"Test project","path":directory.path()}))
        .unwrap();
    agent(&core, "builder", "", "project");
    let queued = core
        .execute(
            "start_task",
            json!({"agentId":"builder","prompt":"Create a result file"}),
        )
        .await
        .unwrap();
    let task = finished(&core, field(&queued, "id")).await;
    assert_eq!(task["status"], "completed", "{task}");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("src/result.txt")).unwrap(),
        "Actual file from a provider tool call"
    );
    assert_eq!(task["toolCalls"][0]["name"], "write_file");
    assert_eq!(task["toolCalls"][0]["ok"], true);
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn coordinator_plan_assigns_distinct_worker_tasks_and_aggregates_results() {
    let provider = MockProvider::start(|body| {
        let prompt = last_user(body);
        if prompt.starts_with("Plan this task for the assigned workers") {
            assert!(body.get("tools").is_none(),"planner must not receive write tools");
            completion(json!({"tasks":[{"agentId":"builder","prompt":"Implement the scoped feature"},{"agentId":"reviewer","prompt":"Review the feature contract"}]}).to_string())
        } else {
            completion(format!("Completed: {prompt}"))
        }
    }).await;
    let (core, _data) = create_core(&provider);
    agent(&core, "manager", "", "");
    agent(&core, "builder", "manager", "");
    agent(&core, "reviewer", "manager", "");
    let queued = core
        .execute(
            "delegate_task",
            json!({"agentId":"manager","prompt":"Build a complete feature with review"}),
        )
        .await
        .unwrap();
    let team = finished(&core, field(&queued, "id")).await;
    assert_eq!(team["status"], "completed", "{team}");
    let tasks = core.list("tasks").unwrap();
    let workers: Vec<_> = tasks
        .iter()
        .filter(|task| task["parentId"] == team["id"])
        .collect();
    assert_eq!(workers.len(), 2);
    assert!(workers.iter().all(|worker| worker["status"] == "completed"));
    assert!(workers
        .iter()
        .any(|worker| worker["prompt"] == "Implement the scoped feature"));
    assert!(workers
        .iter()
        .any(|worker| worker["prompt"] == "Review the feature contract"));
    assert!(team["output"]
        .as_str()
        .unwrap()
        .contains("Completed: Implement the scoped feature"));
    assert!(team["output"]
        .as_str()
        .unwrap()
        .contains("Completed: Review the feature contract"));
    assert_eq!(provider.requests.lock().unwrap().len(), 3);
}
