# Agent OS — Windows application plan

Status: original planning baseline. Implementation and current operating limits are documented in [README.md](README.md).

## 1. Product and first release

Build a Windows desktop application that gives you one place to talk to your AI team, choose projects, delegate work, inspect results, and publish verified versions. It runs on Windows rather than replacing Windows. Working name: **Agent OS**.

Your main conversation is with a coordinator. You can also message any agent directly. An agent has a name, role, chosen provider/model, instructions, workspace access, and an optional supervisor. Creating an agent creates this saved configuration; it does not load another large model or launch a permanent process.

The first useful release must complete this real workflow:

1. Choose a local project and connect an available provider.
2. Create a coordinator and two workers, such as a builder and a reviewer.
3. Ask the coordinator to make a small change. It proposes bounded subtasks and dispatches independent work concurrently.
4. Watch real activity in chat, task cards, and the office; inspect outputs and changed files.
5. Run the project's configured validation commands against the candidate version.
6. After successful validation, checkpoint and push that version to the configured GitHub branch, according to your publishing policy.
7. Close and reopen the application without losing conversations, tasks, or project history.

The office is an interactive view of actual agents and tasks. Animation never fabricates execution or progress. A completed chat response and a verified project version are separate outcomes.

## 2. Proposed architecture

| Part | Choice | Purpose |
| --- | --- | --- |
| Windows desktop shell | Tauri 2 | Native window and operating-system integration using WebView2 |
| Interface | React + TypeScript + Vite | Chat, task views, connection setup, and settings |
| Office | Three.js, loaded when needed | Lightweight interactive 3D scene |
| Application engine | Rust inside Tauri | Task queue, provider requests, child processes, file operations, and validation |
| Local database | SQLite | Conversations, agents, task state, project metadata, and run history |
| Credential storage | Windows Credential Manager, or supported OS-backed equivalent | Keep credentials out of project files and the conversation database |
| Local inference | Existing Ollama runtime first | Run supported Llama, Gemma, and other models without building a new inference engine |
| Remote project services | Git/GitHub and Google Drive adapters | Repository operations and explicit project snapshot transfers |

One application owns the workflow. No Docker requirement, Redis server, application-owned Node server, Kubernetes, or separate scheduler service. Optional integrations can have their own runtime requirements, which must be shown and measured separately. Do not invent a general plugin marketplace or a new model gateway in the first release.

The UI talks to a small set of validated Tauri commands and receives streamed events. The Rust engine persists task transitions and runs provider calls or supported CLIs. Connections and delegation capability are implemented as needed for working integrations, rather than a speculative abstraction framework.

The installed application should not require a Python or Node runtime just to launch. External coding CLIs and Ollama remain optional dependencies for the corresponding connections and have visible setup/status checks.

Windows development and packaging require Rust, Microsoft C++ Build Tools, and the documented WebView2 prerequisites. Keep those development tools out of the end-user runtime requirements. Use Windows CI for Windows packaging and platform checks.

## 3. Providers, accounts, and gateways

| Connection | Planned route | Boundary |
| --- | --- | --- |
| Codex | Supported Codex CLI workflow, using its own sign-in, or a separately configured supported API | Verify machine-readable execution and cancellation before promising the complete adapter |
| Claude | Supported Claude Code CLI workflow or separately configured Anthropic API | Account eligibility, CLI sign-in, and API billing are distinct |
| Llama / Gemma | Models served through Ollama initially | Discover installed models; actual fit depends on model size, context, RAM, and VRAM |
| Other local AI | Add a documented local runtime endpoint when needed | An available chat endpoint does not automatically provide file editing or coding tools |
| Other compatible cloud providers | A documented compatible HTTP endpoint with streaming | Detect supported capabilities and surface unsupported features |
| [OmniRoute](https://github.com/cheaper-inference/OmniRoute) | Connect to its documented `/v1/*` gateway endpoints; retain its provider configuration and routing | Prefer a remote/existing instance on the target device; local runtime memory depends heavily on workload |
| [Ponytail](https://github.com/dietrichgebert/ponytail) | Optional coding skill/profile for compatible agents, using the repository's documented host-specific installation | An instruction/plugin layer rather than an inference provider; commands and UI features depend on the host |
| Google Antigravity | Investigate Google's product for a supported extension/API/CLI route | Product identity is confirmed; external control and account integration still require a documented interface |

Reuse existing official CLI authentication where supported. Do not collect account passwords, copy session cookies, or assume a chat subscription supplies generic API access. Show sign-in status, supported operations, selected model, and a real connection test in the app.

Start with Codex and Ollama as the target first adapters. If the user has no usable Codex connection, another available supported cloud provider can validate the same core flow. Claude follows early; ambiguous integrations remain explicitly pending until identified.

Keep supported gateways optional: Agent OS can call a provider directly or route through a configured gateway. It must avoid recursively routing a request back into itself.

**Ponytail integration:** provide a visible per-agent skill setting, with supported modes and review/audit actions where the host exposes them. Its README documents plugin installation for Codex and Claude Code, and instruction-file/skill adapters for other hosts. Preserve existing project instructions; do not overwrite `AGENTS.md` or `CLAUDE.md` to enable it. Pin and review the chosen version before installation, and preserve the MIT license notices if redistributing its contents. Its Claude-specific band/board UI is not a universal API and must not be promised inside every Agent OS connection. Ponytail review output can inform a reviewer, but it does not replace the application's revision-bound validation gate.

**OmniRoute integration:** add a connection with gateway URL, secure authentication settings, capability checks, and a real model request. Use OmniRoute's routing rather than implementing a second competing router. Prefer connecting to a remote or already running instance. A later optional local-instance control can manage an installed service, with explicit start/stop status and process-tree resource accounting. Its documentation offers a headless mode (`omniroute serve --headless`) that serves `/v1/*` without the dashboard; configured background memory backends can remain active. Verify the selected version's authentication and lifecycle behavior before implementation. Do not bundle a second Electron desktop UI merely to access the gateway.

Documented integration targets include `/v1/models`, `/v1/chat/completions`, `/v1/responses`, and `/api/monitoring/health`. Send the gateway's bearer key from the Rust backend, not the rendered interface. If managing a local instance, restrict its listening interface and configure inference-key enforcement; headless mode does not disable management endpoints. The inspected manifest at version 3.8.53 requires Node `>=22.22.2 <23 || >=24.0.0 <27`, making Node an additional dependency only for that optional local gateway. Check these requirements against the pinned version at implementation time. Preserve MIT license notices if redistributing OmniRoute. No native Windows gateway run has been validated yet.

## 4. Team execution and control

The coordinator turns your request into a task graph with dependencies and clear deliverables. Subagents inherit only the permissions and project context they need. Initial limits: two concurrent cloud runs, at most one local model, one supervisor level with workers beneath it, and a configurable finite task budget. Higher limits come after measurements.

Parallelize independent research, analysis, review, and independent edits. Give editing workers task-specific working directories and integrate their changes through the coordinator. Serialize writes to a shared checkout. Detect conflicts and preserve both versions rather than silently overwriting files. Do not automatically create Git worktrees.

For basic chat models, add only the tools actually needed: scoped file reads, proposed file edits, and configured validation commands. Use documented coding-agent capabilities when a CLI already provides them. Treat repository text and tool output as data, not authority to broaden permissions.

Provide configurable project permission presets so normal work can proceed without repeated prompts. File access remains scoped to selected project roots. Expanding to another directory, exposing credentials, deleting substantial user data, or changing a remote publishing target requires explicit authorization. Pass process arguments safely and keep secrets out of logs.

Every run records provider/model, parent task, allowed workspace, start/end time, command exit status, outputs, and available usage information. Do not invent exact costs for providers that do not expose usage. Set timeouts, task-count limits, and budget limits to prevent runaway delegation.

Task states: queued, planning, running, waiting for input, validating, completed, failed, canceled, and interrupted. Provider/model connectivity is displayed separately. Canceling a supervisor offers an explicit way to cancel its children; provider limitations are visible. Pause/resume appears only where it actually works. After a crash, uncertain runs reopen as interrupted and are reconciled before any command is retried.

## 5. Files, offline work, Drive, and GitHub

**Local projects are the execution workspace.** Select folders with a Windows folder picker; remember recent projects and restrict agent access to approved roots. Keep the application database in the user's application-data directory, outside Git repositories.

**Offline mode means local execution without an Internet connection.** Local models, files, history, and queued tasks can work offline. Cloud agents, GitHub operations, and Google Drive need connectivity. Drive is cloud storage; a previously cached Drive project can be edited locally while offline.

Google Drive integration uses an appropriate supported desktop OAuth flow. Request the narrowest scopes that support the selected workflow. Download/import a project into a local workspace and upload versioned snapshots or explicit changed-file sets. Surface upload status, remote revisions, retries, and conflicts. Do not synchronize a live SQLite database or an actively changing `.git` directory. Do not promise reliable two-way real-time collaboration in the first release.

GitHub publication follows this sequence:

1. Configure the repository, remote, publication branch, and validation commands once. Automatic pushing after verified versions is part of your requested workflow.
2. Assemble a candidate version and record its exact content/revision.
3. Run the required commands and retain fresh results tied to that candidate. Any later edit invalidates those results.
4. On success, create the version checkpoint and push only to the configured destination. Apply tags only under an explicit versioning policy.
5. Confirm the remote accepted the expected commit before showing **Published**.

Failed or missing validation blocks automatic publication. A network failure leaves the version **Verified locally, push pending**. Divergent remote history enters conflict resolution; never force-push or overwrite remote changes automatically. Agents cannot self-declare a version verified merely by saying its tests passed.

## 6. Interface and office design

Design direction: a professional monochrome operations studio. Graphite architecture, pale desks, white text, gray secondary surfaces, and restrained glass partitions. Agents carry stable color accents plus names and badges so they remain recognizable without relying on color alone.

Suggested palette: background `#0D0E10`, panel `#181A1E`, raised surface `#24272C`, primary text `#F5F5F5`, secondary text `#B7BAC2`, and light office surfaces `#D9DCE1`. Use a Windows system font stack, an 8 px spacing system, and readable 14–16 px interface text. Check actual contrast on the final composited surfaces.

Main layout:

- Left: compact navigation for Office, Tasks, Workspaces, Connections, and Activity.
- Top: current project, provider availability, running-task count, and memory mode.
- Center: isometric office or an equivalent 2D team/task view.
- Right: selected agent's chat, assigned work, files, and subordinate agents.
- Bottom: persistent team message composer with recipient, attachments, and send/cancel controls.

Use lightweight stylized figures with contrasting silhouettes, clothing tones, and a small colored accent. A selected agent gets a clear outline and nameplate. Supervisors occupy team desks with their workers grouped nearby. Large teams collapse into expandable groups rather than filling the scene with hundreds of animated figures.

Click or tap an agent to select it and open its conversation. Tap its desk/task badge to inspect its work. Click a supervisor to reveal its team. The composer clearly says **Message team** or **Message [agent]**. Creating an agent asks for name, role, provider/model, workspace, and supervisor. Dragging is optional; all actions also have ordinary buttons.

Use a bounded isometric camera with gentle orbit, zoom, and reset. No required walking or navigating an avatar through rooms. Keep agent targets stationary while the user interacts. Provide at least 44 px touch targets and keyboard access through the agent list, including visible focus and selection synchronization.

Refraction and smoothness are scalable effects: inexpensive transparent materials, edge highlights, baked reflections and lighting by default; optional localized refraction on capable hardware. Avoid full-scene real-time ray tracing and expensive full-screen blur. Use 120–200 ms interface transitions, reduced-motion support, and opaque high-contrast panels where necessary for reading.

Every screen includes useful empty, connecting, running, offline, failure, and recovery states. A blocked task explains what is missing and exposes the next action. The full workflow remains usable with 3D disabled.

## 7. Hardware budget

Target device, as confirmed by the user: Ryzen 4500, 8 GB system RAM, and NVIDIA GeForce GTX 1650. Dedicated VRAM and available memory still need detection on Windows; do not assume a capacity or promise model sizes from the model name alone.

- Default to cloud inference with optional small local inference.
- Begin evaluation with approximately 1–3B-parameter quantized local models, short context, and one model loaded at a time. Availability and fit must be measured.
- Use a GTX 1650-compatible inference backend. Detect its available VRAM and supported runtime before selecting GPU offload; retain a CPU fallback and measure its responsiveness. Graphics and inference share GPU resources, so reduce office effects during local inference when necessary.
- Prefer remote/existing OmniRoute or direct provider connections for coding tasks on this device. OmniRoute's Docker guide recommends an 8 GiB V8 heap and at least a 10 GiB container limit for one coding-agent session, versus a 1 GiB heap and at least a 2 GiB container for light chat. These are workload-specific recommendations, not universal minimums, but the documented coding configuration exceeds this machine's total 8 GB RAM. Headless mode alone does not establish that long-context coding fits. Evaluate any optional lightweight local routing separately, without relying on virtual RAM to meet the coding workload recommendation.
- Query the runtime for loaded models; unload idle models and avoid launching multiple runtimes for separate agents.
- Gate new local runs on measured available memory. Reduce concurrency and offer cloud fallback before memory pressure becomes severe.
- Target 30 FPS for the default office mode, cap render resolution, pause rendering when hidden, and adapt effects to frame time.
- Provisional benchmark goals: application idle memory at or below roughly 500 MB, office-mode memory at or below roughly 700 MB, excluding local model/CLI processes. These are engineering targets, not measured claims.
- Measure the entire process tree, including WebView2, coding CLIs, and local inference. Test responsiveness during inference and validation, not only on an empty screen.
- Use Windows' system-managed page file if needed for stability. Do not change paging settings automatically. Virtual memory does not replace VRAM or make large models fast; avoid relying on sustained paging for normal operation.

The Linux cloud environment can validate much of the UI and core logic. Installation, Credential Manager, WebView2 behavior, GPU performance, and compatibility with the actual Windows machine require Windows testing. Final device suitability cannot be claimed from this cloud session alone.

## 8. Build sequence and acceptance gates

| Phase | Deliverable | Gate before moving on |
| --- | --- | --- |
| 1. Desktop foundation | Tauri window, chat/office layout, local project picker, SQLite persistence, settings | Windows app launches; history survives restart; actual baseline memory recorded |
| 2. Real connections | Codex target adapter, Ollama discovery/chat, credential handling, streaming/cancel | A real task returns a result; auth, disconnect, missing-model, and cancellation paths behave correctly |
| 3. Agent teamwork | Coordinator, worker creation, bounded delegation, task queue, scoped tools | Two independent tasks execute concurrently; conflicting edits are preserved; child failure/cancel propagates accurately |
| 4. Interactive office | Monochrome 3D office, distinct agents, touch/click selection, team inspector, 2D equivalent | Every key action works without 3D; scene reflects real task state; frame time and memory measured |
| 5. Verified versions | GitHub connection, revision-bound validation, checkpoints, automatic post-validation push | Failed validation prevents push; edited candidates need revalidation; push success is confirmed remotely |
| 6. Drive and more providers | Drive project snapshots/offline cache, Claude, verified gateway integrations | Offline edits recover; upload conflicts preserve data; each added adapter passes real capability tests |
| 7. Device release | Windows package, recovery behavior, graphics tuning, onboarding | End-to-end project task succeeds on the actual 8 GB Windows device; resource budgets are recorded |

Start with a small real coding task rather than a demo with simulated agent messages. Use focused core tests for task transitions, workspace boundaries, conflicts, credential redaction, crash recovery, and publication gating; use UI and provider smoke tests for the corresponding integrated workflows.

Keep dependencies and lockfiles explicit when implementation begins. Capture tested installation and startup steps in the cloud environment configuration after those steps actually work. Each cloud task is already isolated: use its existing checkout and do not create Git worktrees unless explicitly requested.

## 9. Open details and current state

Confirmed product clarifications: Ponytail is https://github.com/dietrichgebert/ponytail, OmniRoute is https://github.com/cheaper-inference/OmniRoute for AI management, and Antigravity means Google Antigravity. The GTX 1650 replaces the earlier uncertain GPU name.

Nonblocking details to confirm: dedicated GPU memory, available provider accounts, Google Antigravity's supported integration route, and whether Google Drive access should use an existing connection or a new desktop OAuth registration. Credentials belong in secure settings, never chat or committed files.

No application code, installer, benchmark, or provider execution is complete yet. The current cloud checkout is empty, GitHub read access has been verified, and base Python/Node/Git tools are available. This document is the planning artifact. No untested setup/startup scripts have been saved as working environment configuration.

Official references checked during planning:

- Codex: https://github.com/openai/codex
- Claude Code: https://github.com/anthropics/claude-code
- Ollama API: https://github.com/ollama/ollama/blob/main/docs/api.md
- Tauri prerequisites: https://v2.tauri.app/start/prerequisites/
- Ponytail: https://github.com/dietrichgebert/ponytail
- OmniRoute: https://github.com/cheaper-inference/OmniRoute
- OmniRoute coding workload memory guidance: https://github.com/cheaper-inference/OmniRoute/blob/main/docs/guides/DOCKER_GUIDE.md#runtime-ram-for-coding-agents
- OmniRoute headless mode: https://github.com/cheaper-inference/OmniRoute/blob/main/docs/guides/HEADLESS.md

Before implementing each adapter, confirm its current documented authentication, structured output, tool permissions, and lifecycle behavior. Account access and exact product capabilities are not inferred from the existence of a website or model name.
