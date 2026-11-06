import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import {
  Activity,
  ArrowDownToLine,
  ArrowRight,
  ArrowUp,
  Blocks,
  Box,
  Cable,
  Check,
  CheckCheck,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  CircleHelp,
  Cpu,
  ExternalLink,
  Folder,
  FolderOpen,
  GitBranch,
  GitPullRequest,
  LayoutGrid,
  List,
  Loader2,
  MessageSquare,
  MoreHorizontal,
  Network,
  PanelRightClose,
  Plus,
  RefreshCw,
  Search,
  Send,
  Settings2,
  ShieldCheck,
  Sparkles,
  Square,
  Terminal,
  Trash2,
  Users,
  Wifi,
  X,
  Zap,
} from "lucide-react";
import { api, isDesktop } from "./bridge";
const Office = lazy(() => import("./Office"));

type Agent = {
  id: string;
  name: string;
  role: string;
  provider: string;
  model: string;
  supervisorId?: string;
  workspaceId?: string;
  color: string;
  instructions?: string;
  ponytail?: boolean;
};
type Connection = {
  id: string;
  name: string;
  kind: string;
  baseUrl?: string;
  model?: string;
  hasKey?: boolean;
};
type Workspace = {
  id: string;
  name: string;
  path: string;
  remote?: string;
  branch?: string;
  validationCommand?: string;
  autoPublish?: boolean;
  validation?: any;
};
type Task = {
  id: string;
  agentId: string;
  parentId?: string;
  prompt: string;
  status: string;
  result?: string;
  output?: string;
  error?: string;
  createdAt?: string | number;
  updatedAt?: string | number;
};
type Message = {
  id: string;
  agentId: string;
  role: string;
  content: string;
  createdAt?: string | number;
};
type State = {
  agents: Agent[];
  tasks: Task[];
  messages: Message[];
  connections: Connection[];
  workspaces: Workspace[];
  settings: Record<string, any>;
  dataDir?: string;
};
const EMPTY: State = {
  agents: [],
  tasks: [],
  messages: [],
  connections: [],
  workspaces: [],
  settings: {},
};
const ACTIVE = ["queued", "planning", "running", "validating"];
const COLORS = [
  "#a998ed",
  "#7cbdc9",
  "#e9b977",
  "#a5bf88",
  "#d892ad",
  "#a1adcb",
];
const NAV = [
  { id: "office", icon: Box, label: "Office" },
  { id: "tasks", icon: List, label: "Tasks" },
  { id: "workspaces", icon: FolderOpen, label: "Projects" },
  { id: "connections", icon: Cable, label: "Connections" },
  { id: "activity", icon: Activity, label: "Activity" },
];
const PROVIDERS: Record<string, string> = {
  ollama: "Ollama · local models",
  openai: "OpenAI-compatible API",
  omniroute: "OmniRoute gateway",
  codex: "Codex CLI",
  claude: "Claude Code CLI",
};
function when(date?: string | number) {
  if (!date) return "Now";
  const d = new Date(
    typeof date === "number" && date < 1e12 ? date * 1000 : date,
  );
  return Number.isNaN(+d)
    ? String(date)
    : d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}
function statusLabel(status: string) {
  return status.replace(/_/g, " ");
}
function initial(name: string) {
  return (
    name
      ?.split(" ")
      .map((x) => x[0])
      .slice(0, 2)
      .join("")
      .toUpperCase() || "A"
  );
}
function Avatar({ agent, small = false }: { agent?: Agent; small?: boolean }) {
  return (
    <span
      className={`avatar ${small ? "small" : ""}`}
      style={{ "--agent-color": agent?.color || COLORS[0] } as any}
    >
      <span className="avatar-head" />
      <span className="avatar-body" />
    </span>
  );
}
function Status({ status }: { status: string }) {
  return (
    <span className={`status status-${status}`}>
      <i />
      {statusLabel(status)}
    </span>
  );
}
function Empty({
  icon: Icon = Blocks,
  title,
  children,
  action,
}: {
  icon?: any;
  title: string;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <div className="empty-icon">
        <Icon size={25} strokeWidth={1.4} />
      </div>
      <h3>{title}</h3>
      <p>{children}</p>
      {action}
    </div>
  );
}
function Modal({
  title,
  description,
  close,
  children,
}: {
  title: string;
  description?: string;
  close: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const old = document.activeElement as HTMLElement;
    const timer = setTimeout(
      () =>
        ref.current
          ?.querySelector<HTMLElement>("input,select,button,textarea")
          ?.focus(),
      20,
    );
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
      if (e.key === "Tab") {
        const els = [
          ...(ref.current?.querySelectorAll<HTMLElement>(
            "button:not([disabled]),input,select,textarea,a[href]",
          ) || []),
        ];
        const first = els[0],
          last = els[els.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last?.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first?.focus();
        }
      }
    };
    document.addEventListener("keydown", key);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("keydown", key);
      old?.focus();
    };
  }, [close]);
  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div
        ref={ref}
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="modal-title"
      >
        <div className="modal-heading">
          <div>
            <span className="eyebrow">AGENT OS</span>
            <h2 id="modal-title">{title}</h2>
            {description && <p>{description}</p>}
          </div>
          <button
            className="icon-button"
            onClick={close}
            aria-label="Close dialog"
          >
            <X size={19} />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}

export default function App() {
  const [state, setState] = useState<State>(EMPTY);
  const [loading, setLoading] = useState(true);
  const [view, setView] = useState("office");
  const [selected, setSelected] = useState("");
  const [officeMode, setOfficeMode] = useState<"3d" | "list">("3d");
  const [inspector, setInspector] = useState(true);
  const [panelTab, setPanelTab] = useState("chat");
  const [modal, setModal] = useState<string | null>(null);
  const [editAgent, setEditAgent] = useState<Agent | undefined>();
  const [editConnection, setEditConnection] = useState<
    Connection | undefined
  >();
  const [editWorkspace, setEditWorkspace] = useState<Workspace | undefined>();
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const [notice, setNotice] = useState<{
    message: string;
    error?: boolean;
  } | null>(null);
  const [draft, setDraft] = useState("");
  const [delegate, setDelegate] = useState(false);
  const [taskFilter, setTaskFilter] = useState("all");
  const [search, setSearch] = useState("");
  const [taskDetail, setTaskDetail] = useState<Task | null>(null);
  const [checks, setChecks] = useState<Record<string, any>>({});
  const end = useRef<HTMLDivElement>(null);
  const refresh = useCallback(async () => {
    const data = await api("bootstrap");
    setState({ ...EMPTY, ...data });
  }, []);
  const notify = (message: string, error = false) =>
    setNotice({ message, error });
  const run = useCallback(
    async (
      key: string,
      command: string,
      payload: any = {},
      success?: string,
    ) => {
      setBusy((x) => ({ ...x, [key]: true }));
      try {
        const result = await api(command, payload);
        await refresh();
        if (success) setNotice({ message: success });
        return result;
      } catch (error) {
        setNotice({
          message: String(error instanceof Error ? error.message : error),
          error: true,
        });
        return undefined;
      } finally {
        setBusy((x) => ({ ...x, [key]: false }));
      }
    },
    [refresh],
  );
  useEffect(() => {
    refresh()
      .catch((e) => notify(String(e), true))
      .finally(() => setLoading(false));
  }, [refresh]);
  useEffect(() => {
    if (!state.agents.some((x) => x.id === selected))
      setSelected(state.agents[0]?.id || "");
  }, [state.agents, selected]);
  useEffect(() => {
    if (!isDesktop) return;
    const timer = setInterval(() => refresh().catch(() => {}), 2000);
    return () => clearInterval(timer);
  }, [refresh]);
  useEffect(() => {
    const timer = setTimeout(
      () => setNotice(null),
      notice?.error ? 14000 : 6000,
    );
    return () => clearTimeout(timer);
  }, [notice]);
  useEffect(() => {
    end.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [state.messages.length, selected]);
  const closeModal = useCallback(() => setModal(null), []);
  const agent = state.agents.find((x) => x.id === selected);
  const messages = state.messages.filter((x) => x.agentId === selected);
  const running = state.tasks.filter((x) => ACTIVE.includes(x.status));
  const connection = state.connections.find((x) => x.id === agent?.provider);
  const workspace = state.workspaces.find((x) => x.id === agent?.workspaceId);
  const children = state.agents.filter((x) => x.supervisorId === agent?.id);
  const canWrite = isDesktop;
  const openAgent = (a?: Agent) => {
    setEditAgent(a);
    setModal("agent");
  };
  const openConnection = (c?: Connection) => {
    setEditConnection(c);
    setModal("connection");
  };
  const openWorkspace = (w?: Workspace) => {
    setEditWorkspace(w);
    setModal("workspace");
  };
  const selectAgent = (id: string) => {
    setSelected(id);
    setInspector(true);
    setPanelTab("chat");
  };
  async function send(e: FormEvent) {
    e.preventDefault();
    if (!agent || !draft.trim() || busy.send || !canWrite) return;
    const content = draft.trim();
    setDraft("");
    const result = await run(
      "send",
      delegate && children.length ? "delegate_task" : "send_message",
      delegate && children.length
        ? {
            agentId: agent.id,
            prompt: content,
            workerIds: children.map((x) => x.id),
          }
        : { agentId: agent.id, content },
    );
    if (!result) setDraft(content);
  }
  async function testConnection(id: string) {
    const result = await run(`test-${id}`, "test_connection", { id });
    if (result) {
      setChecks((x) => ({ ...x, [id]: result }));
      notify(
        result.detail ||
          (result.ok ? "Connection verified." : "Connection unavailable."),
        !result.ok,
      );
    }
  }
  const taskList = state.tasks.filter(
    (t) =>
      (taskFilter === "all" ||
        (taskFilter === "active"
          ? ACTIVE.includes(t.status)
          : t.status === taskFilter)) &&
      `${t.prompt} ${state.agents.find((a) => a.id === t.agentId)?.name || ""}`
        .toLowerCase()
        .includes(search.toLowerCase()),
  );

  return (
    <div className="application">
      <aside className="rail">
        <button
          className="brand-mark"
          aria-label="Agent OS home"
          onClick={() => setView("office")}
        >
          <Box size={25} strokeWidth={1.5} />
        </button>
        <div className="rail-separator" />
        <nav aria-label="Main navigation">
          {NAV.map(({ id, icon: Icon, label }) => (
            <button
              key={id}
              className={`nav-item ${view === id ? "active" : ""}`}
              onClick={() => setView(id)}
              title={label}
              aria-label={id === "workspaces" ? "Workspaces" : label}
              aria-current={view === id ? "page" : undefined}
            >
              <Icon size={21} strokeWidth={1.65} />
              <span>{label}</span>
              {id === "tasks" && running.length > 0 && (
                <i className="nav-count">{running.length}</i>
              )}
            </button>
          ))}
        </nav>
        <div className="rail-bottom">
          <button
            className={`nav-item ${view === "settings" ? "active" : ""}`}
            onClick={() => setView("settings")}
            title="Settings"
            aria-label="Settings"
          >
            <Settings2 size={21} />
            <span>Settings</span>
          </button>
          <span className="profile" title="Local workspace">
            L
          </span>
        </div>
      </aside>
      <div className="app-body">
        <header className="topbar">
          <div className="brand-word">
            AGENT<span>OS</span>
            <span className="version">WORKSPACE / 01</span>
          </div>
          <div className="topbar-right">
            <span className="local-pill">
              <span />
              {isDesktop ? "Local workspace" : "Browser preview"}
            </span>
            <div className="top-divider" />
            <button
              className="icon-button"
              title="Refresh workspace"
              aria-label="Refresh workspace"
              onClick={() => run("refresh", "bootstrap")}
            >
              <RefreshCw size={17} className={busy.refresh ? "spin" : ""} />
            </button>
            <button
              className="icon-button"
              aria-label="About this workspace"
              onClick={() => setView("settings")}
            >
              <CircleHelp size={18} />
            </button>
          </div>
        </header>
        {!isDesktop && (
          <div className="preview-banner">
            <Box size={15} />
            <span>
              Design preview. Open the desktop app to connect providers, run
              agents, and access files.
            </span>
            <span className="preview-tag">NO SIMULATED AI</span>
          </div>
        )}
        <main
          className={`main-layout ${view === "office" && inspector ? "with-inspector" : ""}`}
        >
          <section className="main-content">
            <div className="page-heading">
              <div>
                <div className="breadcrumb">
                  My workspace <ChevronRight size={12} />{" "}
                  {view === "workspaces"
                    ? "Projects"
                    : view.charAt(0).toUpperCase() + view.slice(1)}
                </div>
                <h1>
                  {view === "office"
                    ? "Your team, in sync."
                    : view === "tasks"
                      ? "From intent to done."
                      : view === "workspaces"
                        ? "A place for every project."
                        : view === "connections"
                          ? "The intelligence behind it."
                          : view === "activity"
                            ? "Every step, accounted for."
                            : "Make yourself at home."}
                </h1>
                <p>
                  {view === "office"
                    ? "One space to think, build, and move things forward."
                    : view === "tasks"
                      ? "Delegate with clarity. Follow the work as it happens."
                      : view === "workspaces"
                        ? "Your files stay local. Publish only what has been verified."
                        : view === "connections"
                          ? "Bring your own accounts, gateways, and local models."
                          : view === "activity"
                            ? "A persistent history of your team’s actual work."
                            : "A quieter workspace. A lighter footprint."}
                </p>
              </div>
              <div className="heading-actions">
                {view === "office" && (
                  <>
                    <button
                      className="button secondary compact"
                      onClick={() => setInspector(!inspector)}
                      title="Toggle conversation panel"
                    >
                      <MessageSquare size={16} />
                      <span>Conversations</span>
                    </button>
                    <button
                      className="button primary"
                      onClick={() => openAgent()}
                      disabled={!canWrite}
                    >
                      <Plus size={16} />
                      Create agent
                    </button>
                  </>
                )}
                {view === "tasks" && (
                  <button
                    className="button primary"
                    onClick={() => setModal("task")}
                    disabled={!canWrite || !state.agents.length}
                  >
                    <Plus size={16} />
                    New task
                  </button>
                )}
                {view === "workspaces" && (
                  <button
                    className="button primary"
                    onClick={() => openWorkspace()}
                    disabled={!canWrite}
                  >
                    <Plus size={16} />
                    Add project
                  </button>
                )}
                {view === "connections" && (
                  <button
                    className="button primary"
                    onClick={() => openConnection()}
                    disabled={!canWrite}
                  >
                    <Plus size={16} />
                    Add connection
                  </button>
                )}
              </div>
            </div>
            {loading ? (
              <div className="page-loading">
                <Loader2 className="spin" />
                Opening your workspace…
              </div>
            ) : (
              <>
                {view === "office" && (
                  <>
                    <div className="workspace-summary">
                      <span>
                        <Users size={15} />
                        <strong>{state.agents.length}</strong> agents
                      </span>
                      <span>
                        <span
                          className={`tiny-dot ${running.length ? "live" : ""}`}
                        />
                        <strong>{running.length}</strong> working
                      </span>
                      <span>
                        <CheckCheck size={15} />
                        <strong>
                          {
                            state.tasks.filter((t) => t.status === "completed")
                              .length
                          }
                        </strong>{" "}
                        completed
                      </span>
                      <span className="summary-mode">
                        <ShieldCheck size={14} />
                        Your workspace. Your control.
                      </span>
                    </div>
                    <div className="studio-panel">
                      <div className="studio-toolbar">
                        <div className="studio-name">
                          <span className="tiny-dot live" />
                          The studio <span className="muted">/</span>
                          <span className="muted">Floor 01</span>
                        </div>
                        <div className="segmented" aria-label="Office display">
                          <button
                            className={officeMode === "3d" ? "selected" : ""}
                            onClick={() => setOfficeMode("3d")}
                            aria-pressed={officeMode === "3d"}
                          >
                            <Box size={14} />
                            3D office
                          </button>
                          <button
                            className={officeMode === "list" ? "selected" : ""}
                            onClick={() => setOfficeMode("list")}
                            aria-pressed={officeMode === "list"}
                          >
                            <LayoutGrid size={14} />
                            Team view
                          </button>
                        </div>
                      </div>
                      {officeMode === "3d" ? (
                        <div className="studio-scene">
                          <Suspense
                            fallback={
                              <div className="page-loading">
                                <Loader2 className="spin" />
                                Opening the studio…
                              </div>
                            }
                          >
                            <Office
                              agents={state.agents}
                              selectedId={selected}
                              onSelect={selectAgent}
                              runningIds={running.map((t) => t.agentId)}
                              quality={state.settings.quality || "efficient"}
                            />
                          </Suspense>
                          <div className="scene-hint">
                            <span className="hint-mouse" />
                            Select an agent to start a conversation
                          </div>
                          <div className="scene-coordinate">
                            STUDIO — 01 <span>LOCAL FIRST</span>
                          </div>
                        </div>
                      ) : (
                        <div className="team-grid">
                          {state.agents.map((a) => (
                            <button
                              className={`team-grid-card ${selected === a.id ? "chosen" : ""}`}
                              key={a.id}
                              onClick={() => selectAgent(a.id)}
                            >
                              <Avatar agent={a} />
                              <strong>{a.name}</strong>
                              <span>{a.role}</span>
                              <Status
                                status={
                                  running.some((t) => t.agentId === a.id)
                                    ? "running"
                                    : "idle"
                                }
                              />
                            </button>
                          ))}
                          {!state.agents.length && (
                            <Empty icon={Users} title="Your studio is ready">
                              Create your first agent and give it a place on the
                              team.
                            </Empty>
                          )}
                        </div>
                      )}
                      <div className="studio-foot">
                        <span>
                          <span className="keycap">↖</span> Click or tap to
                          select <span className="foot-divider">/</span> Scroll
                          to zoom
                        </span>
                        <span>
                          <Cpu size={13} />
                          Efficient rendering
                        </span>
                      </div>
                    </div>
                    <div className="section-title">
                      <div>
                        <h2>
                          Your team{" "}
                          <span className="count-badge">
                            {state.agents.length}
                          </span>
                        </h2>
                        <p>Different strengths. Shared direction.</p>
                      </div>
                      <button
                        className="text-button"
                        onClick={() => openAgent()}
                        disabled={!canWrite}
                      >
                        Manage agents <ArrowRight size={14} />
                      </button>
                    </div>
                    <div className="agent-roster">
                      {state.agents.map((a) => {
                        const c = state.connections.find(
                          (x) => x.id === a.provider,
                        );
                        return (
                          <button
                            className={`roster-card ${selected === a.id ? "selected" : ""}`}
                            key={a.id}
                            onClick={() => selectAgent(a.id)}
                            style={{ "--agent-color": a.color } as any}
                          >
                            <div className="roster-top">
                              <Avatar agent={a} />
                              <span
                                className={`roster-dot ${running.some((t) => t.agentId === a.id) ? "working" : ""}`}
                              />
                            </div>
                            <strong>{a.name}</strong>
                            <span>{a.role}</span>
                            <div className="roster-provider">
                              <span />
                              {c?.name ||
                                (isDesktop
                                  ? "Not connected"
                                  : a.model || "Preview agent")}
                              {a.ponytail && (
                                <span
                                  className="skill-badge"
                                  title="Ponytail profile"
                                >
                                  P
                                </span>
                              )}
                            </div>
                          </button>
                        );
                      })}
                      <button
                        className="roster-card add-agent"
                        onClick={() => openAgent()}
                        disabled={!canWrite}
                      >
                        <span className="add-agent-icon">
                          <Plus size={22} strokeWidth={1.4} />
                        </span>
                        <strong>A new perspective</strong>
                        <span>Add an agent to your team</span>
                      </button>
                    </div>
                    <div className="workspace-bottom">
                      <div>
                        <ShieldCheck size={16} />
                        <span>
                          Built around your workspace
                          <span className="muted">
                            {" "}
                            · Files and history stay on this device.
                          </span>
                        </span>
                      </div>
                      <span className="muted">
                        Agent OS <span className="mono">0.1</span>
                      </span>
                    </div>
                  </>
                )}

                {view === "tasks" && (
                  <>
                    <div className="list-toolbar">
                      <div className="filter-tabs">
                        {["all", "active", "completed", "failed"].map((f) => (
                          <button
                            className={taskFilter === f ? "selected" : ""}
                            key={f}
                            onClick={() => setTaskFilter(f)}
                          >
                            {f === "all"
                              ? "All tasks"
                              : f.charAt(0).toUpperCase() + f.slice(1)}
                            <span>
                              {f === "all"
                                ? state.tasks.length
                                : f === "active"
                                  ? running.length
                                  : state.tasks.filter((x) => x.status === f)
                                      .length}
                            </span>
                          </button>
                        ))}
                      </div>
                      <label className="search-field">
                        <Search size={15} />
                        <input
                          placeholder="Find a task…"
                          value={search}
                          onChange={(e) => setSearch(e.target.value)}
                          aria-label="Search tasks"
                        />
                      </label>
                    </div>
                    {taskList.length ? (
                      <div className="task-list">
                        {taskList.map((t) => {
                          const a = state.agents.find(
                            (x) => x.id === t.agentId,
                          );
                          const parent = state.tasks.find(
                            (x) => x.id === t.parentId,
                          );
                          return (
                            <button
                              key={t.id}
                              className={`task-row ${t.parentId ? "child-task" : ""}`}
                              onClick={() => setTaskDetail(t)}
                            >
                              <div className="task-icon">
                                {t.status === "completed" ? (
                                  <CheckCheck size={19} />
                                ) : ACTIVE.includes(t.status) ? (
                                  <Loader2 size={19} className="spin" />
                                ) : (
                                  <Terminal size={19} />
                                )}
                              </div>
                              <div className="task-copy">
                                <strong>{t.prompt}</strong>
                                <span>
                                  {parent && (
                                    <>
                                      <GitBranch size={11} />
                                      Subtask ·{" "}
                                    </>
                                  )}
                                  {a?.name || "Deleted agent"} <span>·</span>{" "}
                                  {when(t.createdAt)}
                                </span>
                              </div>
                              <Status status={t.status} />
                              <ChevronRight size={16} />
                            </button>
                          );
                        })}
                      </div>
                    ) : (
                      <Empty
                        icon={CheckCheck}
                        title={
                          search
                            ? "No matching tasks"
                            : "Clear space. Fresh possibilities."
                        }
                      >
                        {search
                          ? "Try another search or filter."
                          : "Assign a task to an agent. Its progress, outputs, and validation will appear here."}
                        {}
                      </Empty>
                    )}
                  </>
                )}

                {view === "workspaces" && (
                  <>
                    <div className="callout">
                      <Folder size={19} />
                      <div>
                        <strong>
                          Local by default. Connected when you choose.
                        </strong>
                        <p>
                          Agents work inside the project folder you select. Git
                          publication requires a successful validation of the
                          current version.
                        </p>
                      </div>
                    </div>
                    <div className="content-grid">
                      {state.workspaces.map((w) => (
                        <article
                          className="surface-card project-card"
                          key={w.id}
                        >
                          <div className="card-topline">
                            <span className="card-symbol">
                              <FolderOpen size={23} />
                            </span>
                            <button
                              className="icon-button"
                              aria-label={`Edit ${w.name}`}
                              onClick={() => openWorkspace(w)}
                            >
                              <MoreHorizontal size={18} />
                            </button>
                          </div>
                          <h3>{w.name}</h3>
                          <code className="path-text">{w.path}</code>
                          <div className="project-meta">
                            <span>
                              <GitBranch size={14} />
                              {w.branch || "Current branch"}
                            </span>
                            <span>
                              {
                                state.agents.filter(
                                  (a) => a.workspaceId === w.id,
                                ).length
                              }{" "}
                              agents
                            </span>
                          </div>
                          <div className="project-validation">
                            <ShieldCheck size={15} />
                            <span>
                              {w.validationCommand
                                ? `Validation: ${w.validationCommand}`
                                : "Set a validation command before publishing."}
                            </span>
                          </div>
                          <div className="card-actions">
                            <button
                              className="button secondary"
                              disabled={!canWrite || busy[`validate-${w.id}`]}
                              onClick={async () => {
                                const r = await run(
                                  `validate-${w.id}`,
                                  "validate_workspace",
                                  { id: w.id },
                                );
                                if (r) {
                                  setChecks((x) => ({
                                    ...x,
                                    [`workspace-${w.id}`]: r,
                                  }));
                                  notify(
                                    r.detail ||
                                      r.output ||
                                      "Validation finished.",
                                  );
                                }
                              }}
                            >
                              {busy[`validate-${w.id}`] ? (
                                <Loader2 className="spin" size={15} />
                              ) : (
                                <CheckCheck size={15} />
                              )}
                              Validate
                            </button>
                            <button
                              className="button secondary"
                              disabled={!canWrite || busy[`publish-${w.id}`]}
                              onClick={() => {
                                setEditWorkspace(w);
                                setModal("publish");
                              }}
                            >
                              <GitPullRequest size={15} />
                              Publish
                            </button>
                            <button
                              className="icon-button"
                              title="Export local snapshot"
                              aria-label={`Export ${w.name} snapshot`}
                              onClick={() => {
                                setEditWorkspace(w);
                                setModal("export");
                              }}
                              disabled={!canWrite}
                            >
                              <ArrowDownToLine size={17} />
                            </button>
                          </div>
                          {checks[`workspace-${w.id}`] && (
                            <pre className="inline-output">
                              {JSON.stringify(
                                checks[`workspace-${w.id}`],
                                null,
                                2,
                              )}
                            </pre>
                          )}
                        </article>
                      ))}
                    </div>
                    {!state.workspaces.length && (
                      <Empty
                        icon={FolderOpen}
                        title="Bring a project into the studio"
                      >
                        Choose an existing local folder. Your agents will use it
                        as their working directory.
                      </Empty>
                    )}
                    <DrivePanel
                      state={state}
                      run={run}
                      busy={busy}
                      notify={notify}
                    />
                  </>
                )}

                {view === "connections" && (
                  <>
                    <div className="callout">
                      <ShieldCheck size={19} />
                      <div>
                        <strong>
                          Your accounts. Your choice of intelligence.
                        </strong>
                        <p>
                          Use an official coding CLI session, an API key, a
                          gateway, or a locally running model. Chat
                          subscriptions and API billing are separate.
                        </p>
                      </div>
                    </div>
                    <div className="content-grid">
                      {state.connections.map((c) => (
                        <article
                          className="surface-card connection-card"
                          key={c.id}
                        >
                          <div className="card-topline">
                            <span
                              className={`connection-symbol kind-${c.kind}`}
                            >
                              {c.kind === "ollama" ? (
                                <Cpu size={24} />
                              ) : c.kind === "omniroute" ? (
                                <Network size={24} />
                              ) : (
                                <Terminal size={24} />
                              )}
                            </span>
                            <span className="neutral-badge">
                              {c.kind === "ollama"
                                ? "LOCAL"
                                : ["codex", "claude"].includes(c.kind)
                                  ? "CLI"
                                  : "API"}
                            </span>
                          </div>
                          <h3>{c.name}</h3>
                          <p>{PROVIDERS[c.kind] || c.kind}</p>
                          <code className="connection-endpoint">
                            {c.baseUrl || "Official CLI authentication"}
                          </code>
                          {checks[c.id] && (
                            <div
                              className={`connection-check ${checks[c.id].ok ? "ok" : "error"}`}
                            >
                              {checks[c.id].ok ? (
                                <CheckCircle2 size={15} />
                              ) : (
                                <CircleHelp size={15} />
                              )}
                              <span>{checks[c.id].detail}</span>
                            </div>
                          )}
                          <div className="card-actions">
                            <button
                              className="button secondary"
                              disabled={!canWrite || busy[`test-${c.id}`]}
                              onClick={() => testConnection(c.id)}
                            >
                              {busy[`test-${c.id}`] ? (
                                <Loader2 size={15} className="spin" />
                              ) : (
                                <Zap size={15} />
                              )}
                              Test connection
                            </button>
                            <button
                              className="icon-button"
                              aria-label={`Edit ${c.name}`}
                              onClick={() => openConnection(c)}
                            >
                              <Settings2 size={17} />
                            </button>
                          </div>
                        </article>
                      ))}
                    </div>
                    {!state.connections.length && (
                      <Empty icon={Cable} title="Choose how your team thinks">
                        Connect an installed Codex or Claude CLI, a running
                        Ollama instance, or an API endpoint.
                      </Empty>
                    )}
                    <div className="section-title">
                      <div>
                        <h2>Tools that fit your workflow</h2>
                        <p>
                          Clear boundaries, rather than promised compatibility.
                        </p>
                      </div>
                    </div>
                    <div className="integration-note">
                      <div className="integration-icon ponytail">P</div>
                      <div>
                        <strong>
                          Ponytail{" "}
                          <span className="neutral-badge">Agent skill</span>
                        </strong>
                        <p>
                          Enable the Ponytail instruction profile per agent.
                          Native plugin commands require installation in the
                          corresponding CLI.
                        </p>
                      </div>
                      <a
                        className="icon-button"
                        href="https://github.com/dietrichgebert/ponytail"
                        target="_blank"
                        rel="noreferrer"
                        aria-label="Ponytail repository"
                      >
                        <ExternalLink size={16} />
                      </a>
                    </div>
                    <div className="integration-note">
                      <div className="integration-icon">A</div>
                      <div>
                        <strong>Google Antigravity</strong>
                        <p>
                          Direct adapter not implemented. Official CLI integration
                          remains unverified; this app does not reuse private
                          sign-in data.
                        </p>
                      </div>
                      <span className="neutral-badge">Not connected</span>
                    </div>
                    <div className="fine-print">
                      <Cpu size={15} />
                      For your 8 GB device, use a remote OmniRoute gateway for
                      long coding sessions. Local gateway memory requirements
                      vary by workload.
                    </div>
                  </>
                )}

                {view === "activity" && (
                  <>
                    {state.tasks.length ? (
                      <div className="activity-list">
                        {[...state.tasks].reverse().map((t) => (
                          <div className="activity-item" key={t.id}>
                            <div
                              className={`activity-point ${t.status === "completed" ? "done" : ""}`}
                            >
                              {t.status === "completed" ? (
                                <Check size={14} />
                              ) : (
                                <Terminal size={14} />
                              )}
                            </div>
                            <div>
                              <div className="activity-heading">
                                <strong>
                                  {state.agents.find((x) => x.id === t.agentId)
                                    ?.name || "Agent"}
                                </strong>
                                <Status status={t.status} />
                                <time>{when(t.updatedAt || t.createdAt)}</time>
                              </div>
                              <p>{t.prompt}</p>
                              {(t.error || t.result || t.output) && (
                                <button
                                  className="text-button"
                                  onClick={() => setTaskDetail(t)}
                                >
                                  View result <ArrowRight size={13} />
                                </button>
                              )}
                            </div>
                          </div>
                        ))}
                      </div>
                    ) : (
                      <Empty
                        icon={Activity}
                        title="Your story starts with the first task"
                      >
                        Actual task events and results appear here. Nothing is
                        simulated.
                      </Empty>
                    )}
                  </>
                )}

                {view === "settings" && (
                  <div className="settings-stack">
                    <section className="surface-card settings-card">
                      <div className="settings-section-icon">
                        <Cpu size={22} />
                      </div>
                      <div className="settings-section">
                        <h2>A lighter footprint</h2>
                        <p>
                          Designed for your Ryzen 4500, GTX 1650, and 8 GB RAM.
                          Performance on your device still needs measurement.
                        </p>
                        <label className="setting-row">
                          <span>
                            <strong>Office graphics</strong>
                            <small>
                              Efficient mode limits resolution and effects.
                            </small>
                          </span>
                          <select
                            value={state.settings.quality || "efficient"}
                            disabled={!canWrite}
                            onChange={(e) =>
                              run("settings", "update_settings", {
                                settings: {
                                  ...state.settings,
                                  quality: e.target.value,
                                },
                              })
                            }
                          >
                            <option value="efficient">
                              Efficient · 30 FPS
                            </option>
                            <option value="balanced">Balanced</option>
                          </select>
                        </label>
                        <div className="setting-row">
                          <span>
                            <strong>Parallel cloud tasks</strong>
                            <small>
                              Bounded execution keeps the workspace responsive.
                            </small>
                          </span>
                          <span className="setting-value">2 tasks</span>
                        </div>
                        <div className="setting-row">
                          <span>
                            <strong>Local inference</strong>
                            <small>
                              One small model at a time. Actual fit depends on
                              memory.
                            </small>
                          </span>
                          <span className="setting-value">1 model</span>
                        </div>
                        <div className="setting-row">
                          <span>
                            <strong>System paging</strong>
                            <small>
                              Managed by Windows. Agent OS does not change it.
                            </small>
                          </span>
                          <span className="neutral-badge">System managed</span>
                        </div>
                      </div>
                    </section>
                    <section className="surface-card settings-card">
                      <div className="settings-section-icon">
                        <ShieldCheck size={22} />
                      </div>
                      <div className="settings-section">
                        <h2>Your data stays yours</h2>
                        <p>
                          Conversations and task history are stored locally.
                          Selected providers receive the prompts and context
                          required for their tasks.
                        </p>
                        <div className="setting-row">
                          <span>
                            <strong>Credentials</strong>
                            <small>
                              Provider keys use supported operating-system
                              credential storage.
                            </small>
                          </span>
                          <span className="neutral-badge">OS protected</span>
                        </div>
                        <div className="setting-row">
                          <span>
                            <strong>Publishing</strong>
                            <small>
                              Push requires successful validation of the exact
                              current candidate.
                            </small>
                          </span>
                          <span className="setting-value">
                            <CheckCheck size={15} />
                            Verification required
                          </span>
                        </div>
                        {state.dataDir && (
                          <code className="path-text">{state.dataDir}</code>
                        )}
                      </div>
                    </section>
                    <section className="about-card">
                      <Box size={29} strokeWidth={1.2} />
                      <div>
                        <strong>
                          Agent OS <span>0.1.0</span>
                        </strong>
                        <p>One calm workspace for a capable team.</p>
                      </div>
                      <a
                        className="text-button"
                        href="https://github.com/hryvd/agent_os"
                        target="_blank"
                        rel="noreferrer"
                      >
                        Repository <ExternalLink size={14} />
                      </a>
                    </section>
                  </div>
                )}
              </>
            )}
          </section>
          {view === "office" && inspector && (
            <aside className="inspector">
              <div className="inspector-heading">
                <span>CONVERSATION</span>
                <button
                  className="icon-button"
                  onClick={() => setInspector(false)}
                  aria-label="Close conversation panel"
                >
                  <PanelRightClose size={17} />
                </button>
              </div>
              {agent ? (
                <>
                  <div className="agent-heading">
                    <Avatar agent={agent} />
                    <div>
                      <h2>{agent.name}</h2>
                      <p>{agent.role}</p>
                    </div>
                    <button
                      className="icon-button"
                      onClick={() => openAgent(agent)}
                      title="Agent settings"
                      aria-label="Edit selected agent"
                    >
                      <MoreHorizontal size={19} />
                    </button>
                  </div>
                  <div className="agent-meta">
                    <span
                      className={`tiny-dot ${running.some((t) => t.agentId === selected) ? "live" : ""}`}
                    />
                    {running.some((t) => t.agentId === selected)
                      ? "Working on your task"
                      : connection
                        ? connection.name
                        : isDesktop
                          ? "Connection required"
                          : "Preview agent"}
                    <span className="meta-spacer" />
                    {agent.ponytail && (
                      <span className="neutral-badge tiny">PONYTAIL</span>
                    )}
                  </div>
                  <div className="inspector-tabs">
                    <button
                      className={panelTab === "chat" ? "active" : ""}
                      onClick={() => setPanelTab("chat")}
                    >
                      <MessageSquare size={14} />
                      Chat
                    </button>
                    <button
                      className={panelTab === "work" ? "active" : ""}
                      onClick={() => setPanelTab("work")}
                    >
                      <List size={14} />
                      Work
                      <span>
                        {
                          state.tasks.filter((t) => t.agentId === selected)
                            .length
                        }
                      </span>
                    </button>
                    <button
                      className={panelTab === "team" ? "active" : ""}
                      onClick={() => setPanelTab("team")}
                    >
                      <Users size={14} />
                      Team<span>{children.length}</span>
                    </button>
                  </div>
                  {panelTab === "chat" ? (
                    <>
                      <div className="chat-history" aria-live="polite">
                        {!messages.length ? (
                          <div className="chat-welcome">
                            <div className="welcome-symbol">
                              <Sparkles size={23} strokeWidth={1.3} />
                            </div>
                            <h3>
                              A little direction.
                              <br />A lot of possibility.
                            </h3>
                            <p>
                              {isDesktop
                                ? `Give ${agent.name} a goal, ask a question, or talk through an idea.`
                                : `This is ${agent.name}’s conversation space. Connect a provider in the desktop app to begin.`}
                            </p>
                            <div className="suggestion-list">
                              {[
                                "Help me plan a new project",
                                "Review the current workspace",
                                "Break a goal into smaller tasks",
                              ].map((s) => (
                                <button
                                  key={s}
                                  onClick={() => setDraft(s)}
                                  disabled={!canWrite}
                                >
                                  {s}
                                  <ArrowUp size={13} />
                                </button>
                              ))}
                            </div>
                          </div>
                        ) : (
                          messages.map((m) => (
                            <div
                              key={m.id}
                              className={`message message-${m.role}`}
                            >
                              <div className="message-label">
                                {m.role === "user" ? (
                                  <span className="user-avatar">Y</span>
                                ) : (
                                  <Avatar agent={agent} small />
                                )}
                                <strong>
                                  {m.role === "user" ? "You" : agent.name}
                                </strong>
                                <time>{when(m.createdAt)}</time>
                              </div>
                              <div className="message-content">{m.content}</div>
                            </div>
                          ))
                        )}
                        {busy.send && (
                          <div className="thinking">
                            <Loader2 size={14} className="spin" />
                            {agent.name} is working…
                          </div>
                        )}
                        <div ref={end} />
                      </div>
                      <form className="composer-area" onSubmit={send}>
                        {children.length > 0 && (
                          <label className="delegate-toggle">
                            <input
                              type="checkbox"
                              checked={delegate}
                              onChange={(e) => setDelegate(e.target.checked)}
                              disabled={!canWrite}
                            />
                            <Network size={12} />
                            Delegate to team{" "}
                            <span>{children.length} workers</span>
                          </label>
                        )}
                        <div className="composer">
                          <textarea
                            value={draft}
                            onChange={(e) => setDraft(e.target.value)}
                            placeholder={`Message ${agent.name}…`}
                            aria-label={`Message ${agent.name}`}
                            rows={3}
                            disabled={!canWrite || busy.send}
                            onKeyDown={(e) => {
                              if (e.key === "Enter" && !e.shiftKey) {
                                e.preventDefault();
                                send(e);
                              }
                            }}
                          />
                          <div className="composer-footer">
                            <span>
                              <span className="tiny-dot" />
                              {agent.model || "Select a model"}
                            </span>
                            <button
                              type="submit"
                              className="send-button"
                              disabled={!canWrite || busy.send || !draft.trim()}
                              aria-label="Send message"
                            >
                              {busy.send ? (
                                <Loader2 className="spin" size={17} />
                              ) : (
                                <ArrowUp size={18} />
                              )}
                            </button>
                          </div>
                        </div>
                        <p className="composer-hint">
                          {workspace ? (
                            <>
                              <Folder size={11} />
                              {workspace.name}
                            </>
                          ) : (
                            <>
                              <ShieldCheck size={11} />
                              {isDesktop
                                ? "No project selected"
                                : "Desktop connection required"}
                            </>
                          )}
                          <span>Enter to send</span>
                        </p>
                      </form>
                    </>
                  ) : panelTab === "work" ? (
                    <div className="inspector-scroll">
                      <button
                        className="button secondary full"
                        onClick={() => setModal("task")}
                        disabled={!canWrite}
                      >
                        <Plus size={15} />
                        Assign a task
                      </button>
                      {state.tasks
                        .filter((t) => t.agentId === selected)
                        .map((t) => (
                          <button
                            className="inspector-task"
                            key={t.id}
                            onClick={() => setTaskDetail(t)}
                          >
                            <Status status={t.status} />
                            <strong>{t.prompt}</strong>
                            <span>{when(t.createdAt)}</span>
                          </button>
                        ))}
                      {!state.tasks.some((t) => t.agentId === selected) && (
                        <p className="panel-empty">No tasks assigned yet.</p>
                      )}
                    </div>
                  ) : (
                    <div className="inspector-scroll">
                      <p className="panel-description">
                        Agents reporting to {agent.name}. Give each worker a
                        clear role and assign independent tasks.
                      </p>
                      {children.map((a) => (
                        <button
                          className="child-agent"
                          key={a.id}
                          onClick={() => selectAgent(a.id)}
                        >
                          <Avatar agent={a} small />
                          <span>
                            <strong>{a.name}</strong>
                            <small>{a.role}</small>
                          </span>
                          <ChevronRight size={15} />
                        </button>
                      ))}
                      <button
                        className="button secondary full"
                        onClick={() => {
                          setEditAgent({ supervisorId: agent.id } as Agent);
                          setModal("agent");
                        }}
                        disabled={!canWrite}
                      >
                        <Plus size={15} />
                        Add a worker
                      </button>
                    </div>
                  )}
                </>
              ) : (
                <div className="inspector-scroll">
                  <Empty icon={MessageSquare} title="Meet your first agent">
                    Create an agent, choose its connection, and start a
                    conversation.
                  </Empty>
                  <button
                    className="button primary full"
                    disabled={!canWrite}
                    onClick={() => openAgent()}
                  >
                    <Plus size={15} />
                    Create agent
                  </button>
                </div>
              )}
            </aside>
          )}
        </main>
        <footer className="app-status">
          <span>
            <span className="tiny-dot live" />
            {isDesktop ? "Local engine" : "Visual preview"}
          </span>
          <span>{state.connections.length} connections</span>
          <span className="status-right">
            <Cpu size={12} />
            LOW MEMORY MODE <span className="foot-divider">/</span>{" "}
            <span>
              {running.length ? `${running.length} active` : "All quiet"}
            </span>
          </span>
        </footer>
      </div>
      {notice && (
        <div
          className={`toast ${notice.error ? "toast-error" : ""}`}
          role={notice.error ? "alert" : "status"}
        >
          {notice.error ? <CircleHelp size={18} /> : <CheckCircle2 size={18} />}
          <span>{notice.message}</span>
          <button
            className="icon-button"
            aria-label="Dismiss notification"
            onClick={() => setNotice(null)}
          >
            <X size={16} />
          </button>
        </div>
      )}
      {modal === "agent" && (
        <Modal
          title={
            editAgent?.id ? "Shape your agent." : "A new member of the team."
          }
          description="Give it a role, a connection, and a clear place to work."
          close={closeModal}
        >
          <AgentForm
            initial={editAgent}
            state={state}
            busy={busy.saveAgent}
            close={closeModal}
            submit={async (a) => {
              const result = await run(
                "saveAgent",
                "save_agent",
                { agent: a },
                "Agent saved.",
              );
              if (result) {
                setSelected(result.id);
                closeModal();
              }
            }}
            remove={
              editAgent?.id
                ? async () => {
                    const result = await run(
                      "saveAgent",
                      "delete_agent",
                      { id: editAgent.id },
                      "Agent removed.",
                    );
                    if (result !== undefined) closeModal();
                  }
                : undefined
            }
          />
        </Modal>
      )}
      {modal === "connection" && (
        <Modal
          title={
            editConnection
              ? "Connection settings."
              : "Connect your intelligence."
          }
          description="Credentials stay in operating-system storage, not in your project."
          close={closeModal}
        >
          <ConnectionForm
            initial={editConnection}
            busy={busy.saveConnection}
            close={closeModal}
            submit={async (c) => {
              const result = await run(
                "saveConnection",
                "save_connection",
                { connection: c },
                "Connection saved. Test it before assigning work.",
              );
              if (result) closeModal();
            }}
          />
        </Modal>
      )}
      {modal === "workspace" && (
        <Modal
          title={editWorkspace ? "Project settings." : "Bring a project along."}
          description="Choose the folder agents can access and how its work is validated."
          close={closeModal}
        >
          <WorkspaceForm
            initial={editWorkspace}
            busy={busy.saveWorkspace}
            close={closeModal}
            submit={async (w) => {
              const result = await run(
                "saveWorkspace",
                "save_workspace",
                { workspace: w },
                "Project saved.",
              );
              if (result) closeModal();
            }}
          />
        </Modal>
      )}
      {modal === "task" && (
        <Modal
          title="Give the work a direction."
          description="Assign a specific outcome to an agent, with an optional parent task."
          close={closeModal}
        >
          <TaskForm
            state={state}
            selected={selected}
            close={closeModal}
            submit={async (payload) => {
              closeModal();
              setView("tasks");
              await run(`task-${payload.agentId}`, "start_task", payload);
            }}
          />
        </Modal>
      )}
      {modal === "publish" && editWorkspace && (
        <Modal title="Publish a verified version." close={closeModal}>
          <div className="modal-body">
            <p className="dialog-explanation">
              Agent OS will check that the current files match a successful
              validation, create a checkpoint, and push to{" "}
              <strong>{editWorkspace.remote || "origin"}</strong> on{" "}
              <strong>{editWorkspace.branch || "the configured branch"}</strong>
              . A changed or unverified version is blocked.
            </p>
            <code className="path-text">{editWorkspace.path}</code>
          </div>
          <div className="modal-actions">
            <button className="button secondary" onClick={closeModal}>
              Cancel
            </button>
            <button
              className="button primary"
              disabled={busy[`publish-${editWorkspace.id}`]}
              onClick={async () => {
                const r = await run(
                  `publish-${editWorkspace.id}`,
                  "publish_workspace",
                  { id: editWorkspace.id },
                );
                if (r) {
                  notify(r.detail || "Verified version published.");
                  closeModal();
                }
              }}
            >
              {busy[`publish-${editWorkspace.id}`] ? (
                <Loader2 size={15} className="spin" />
              ) : (
                <GitPullRequest size={15} />
              )}
              Publish verified version
            </button>
          </div>
        </Modal>
      )}
      {modal === "export" && editWorkspace && (
        <Modal
          title="Export a project snapshot."
          description="Save outside the project, including to a Google Drive-synced folder."
          close={closeModal}
        >
          <form
            onSubmit={async (e) => {
              e.preventDefault();
              const data = new FormData(e.currentTarget);
              const r = await run("export", "export_snapshot", {
                workspaceId: editWorkspace.id,
                destination: data.get("destination"),
              });
              if (r) {
                notify(r.detail || "Snapshot exported.");
                closeModal();
              }
            }}
          >
            <div className="modal-body">
              <label>
                Destination path
                <input
                  name="destination"
                  placeholder="C:\Users\you\Backups\project-snapshot.zip"
                  required
                />
              </label>
              <p className="field-note">
                Do not put the destination inside the source project.
                Credentials, Git metadata, and dependency caches should not be
                included.
              </p>
            </div>
            <div className="modal-actions">
              <button
                type="button"
                className="button secondary"
                onClick={closeModal}
              >
                Cancel
              </button>
              <button className="button primary" disabled={busy.export}>
                {busy.export && <Loader2 className="spin" size={15} />}Export
                snapshot
              </button>
            </div>
          </form>
        </Modal>
      )}
      {taskDetail && (
        <Modal
          title="Task details"
          description={`Assigned to ${state.agents.find((a) => a.id === taskDetail.agentId)?.name || "an agent"}`}
          close={() => setTaskDetail(null)}
        >
          <div className="modal-body task-detail">
            <Status
              status={
                state.tasks.find((t) => t.id === taskDetail.id)?.status ||
                taskDetail.status
              }
            />
            <h3>{taskDetail.prompt}</h3>
            <pre>
              {state.tasks.find((t) => t.id === taskDetail.id)?.error ||
                state.tasks.find((t) => t.id === taskDetail.id)?.result ||
                state.tasks.find((t) => t.id === taskDetail.id)?.output ||
                taskDetail.error ||
                taskDetail.result ||
                taskDetail.output ||
                "No result yet. The task may still be queued or running."}
            </pre>
          </div>
          <div className="modal-actions">
            <button
              className="button secondary"
              onClick={() => setTaskDetail(null)}
            >
              Close
            </button>
            {ACTIVE.includes(
              state.tasks.find((t) => t.id === taskDetail.id)?.status ||
                taskDetail.status,
            ) ? (
              <button
                className="button secondary"
                disabled={!canWrite || busy.cancel}
                onClick={() =>
                  run(
                    "cancel",
                    "cancel_task",
                    { id: taskDetail.id },
                    "Cancellation requested.",
                  )
                }
              >
                <Square size={13} />
                Cancel task
              </button>
            ) : (
              <button
                className="button primary"
                disabled={!canWrite || busy.retry}
                onClick={async () => {
                  const id = taskDetail.id;
                  setTaskDetail(null);
                  await run("retry", "retry_task", { id });
                }}
              >
                <RefreshCw size={14} />
                Retry task
              </button>
            )}
          </div>
        </Modal>
      )}
    </div>
  );
}

function AgentForm({
  initial: a,
  state,
  busy,
  close,
  submit,
  remove,
}: {
  initial?: Agent;
  state: State;
  busy?: boolean;
  close: () => void;
  submit: (a: any) => void;
  remove?: () => void;
}) {
  const [color, setColor] = useState(
    a?.color || COLORS[state.agents.length % COLORS.length],
  );
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        const f = new FormData(e.currentTarget);
        submit({
          id: a?.id,
          name: f.get("name"),
          role: f.get("role"),
          provider: f.get("provider"),
          model: f.get("model"),
          supervisorId: f.get("supervisorId") || null,
          workspaceId: f.get("workspaceId") || null,
          color,
          instructions: f.get("instructions"),
          ponytail: f.get("ponytail") === "on",
        });
      }}
    >
      <div className="modal-body">
        <div className="form-row">
          <label>
            Name
            <input
              name="name"
              defaultValue={a?.name}
              placeholder="e.g. Atlas"
              required
              maxLength={60}
            />
          </label>
          <label>
            Role
            <input
              name="role"
              defaultValue={a?.role}
              placeholder="e.g. Lead developer"
              required
              maxLength={100}
            />
          </label>
        </div>
        <div className="form-row">
          <label>
            Connection
            <select
              name="provider"
              defaultValue={a?.provider || state.connections[0]?.id || ""}
              required
            >
              <option value="" disabled>
                Select a connection
              </option>
              {state.connections.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            Model
            <input
              name="model"
              defaultValue={a?.model}
              placeholder="e.g. gemma3:1b or CLI default"
            />
          </label>
        </div>
        {!state.connections.length && (
          <p className="field-note">
            Add a connection first so this agent has a provider to use.
          </p>
        )}
        <div className="form-row">
          <label>
            Project
            <select name="workspaceId" defaultValue={a?.workspaceId || ""}>
              <option value="">No project · chat only</option>
              {state.workspaces.map((w) => (
                <option value={w.id} key={w.id}>
                  {w.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            Reports to
            <select name="supervisorId" defaultValue={a?.supervisorId || ""}>
              <option value="">Independent agent</option>
              {state.agents
                .filter((x) => x.id !== a?.id)
                .map((x) => (
                  <option key={x.id} value={x.id}>
                    {x.name}
                  </option>
                ))}
            </select>
          </label>
        </div>
        <label>
          Working instructions
          <textarea
            name="instructions"
            defaultValue={a?.instructions}
            placeholder="What should this agent focus on? Define its responsibilities and boundaries."
            rows={3}
          />
        </label>
        <div className="color-field">
          <span>Agent accent</span>
          <div className="color-options">
            {COLORS.map((c) => (
              <button
                key={c}
                type="button"
                className={color === c ? "chosen" : ""}
                onClick={() => setColor(c)}
                style={{ "--swatch": c } as any}
                aria-label={`Use ${c} accent`}
                aria-pressed={color === c}
              >
                {color === c && <Check size={16} />}
              </button>
            ))}
          </div>
        </div>
        <label className="checkbox-field">
          <input type="checkbox" name="ponytail" defaultChecked={a?.ponytail} />
          <span>
            <strong>Ponytail profile</strong>
            <small>
              Favor existing tools and simple code. Native CLI plugin features
              require separate installation.
            </small>
          </span>
        </label>
      </div>
      <div className="modal-actions">
        {remove && (
          <button
            type="button"
            className="button danger remove-agent"
            onClick={remove}
            disabled={busy}
          >
            <Trash2 size={14} />
            Remove
          </button>
        )}
        <button type="button" className="button secondary" onClick={close}>
          Cancel
        </button>
        <button
          className="button primary"
          disabled={busy || !state.connections.length}
        >
          {busy && <Loader2 size={15} className="spin" />}
          {a?.id ? "Save changes" : "Create agent"}
        </button>
      </div>
    </form>
  );
}

function ConnectionForm({
  initial: c,
  busy,
  close,
  submit,
}: {
  initial?: Connection;
  busy?: boolean;
  close: () => void;
  submit: (c: any) => void;
}) {
  const [kind, setKind] = useState(c?.kind || "ollama");
  const cli = ["codex", "claude"].includes(kind);
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        const f = new FormData(e.currentTarget);
        submit({
          id: c?.id || undefined,
          name: f.get("name"),
          kind,
          baseUrl: f.get("baseUrl") || "",
          model: f.get("model") || "",
          apiKey: f.get("apiKey") || undefined,
        });
      }}
    >
      <div className="modal-body">
        <label>
          Connection type
          <select value={kind} onChange={(e) => setKind(e.target.value)}>
            {Object.entries(PROVIDERS).map(([id, label]) => (
              <option key={id} value={id}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label>
          Connection name
          <input
            aria-label="Connection name"
            name="name"
            placeholder="e.g. My local models"
            defaultValue={c?.name}
            required
            maxLength={80}
          />
        </label>
        {cli ? (
          <div className="form-callout">
            <Terminal size={20} />
            <p>
              Install the official {kind === "codex" ? "Codex" : "Claude Code"}{" "}
              CLI and complete its sign-in on this computer first. Agent OS uses
              its documented noninteractive interface.
            </p>
          </div>
        ) : (
          <>
            <label>
              Base URL
              <input
                key={kind}
                name="baseUrl"
                type="url"
                defaultValue={
                  c?.baseUrl ||
                  (kind === "ollama"
                    ? "http://localhost:11434"
                    : kind === "omniroute"
                      ? "http://localhost:20128/v1"
                      : "https://api.openai.com/v1")
                }
                required
              />
            </label>
            <label>
              API key
              <input
                name="apiKey"
                type="password"
                autoComplete="off"
                placeholder={
                  c
                    ? "Leave blank to keep the saved key"
                    : kind === "ollama"
                      ? "Optional for local Ollama"
                      : "Stored securely by the desktop app"
                }
              />
            </label>
          </>
        )}
        <label>
          Default model
          <input
            name="model"
            defaultValue={c?.model}
            placeholder={
              kind === "ollama"
                ? "e.g. gemma3:1b"
                : "Provider model identifier, or CLI default"
            }
          />
        </label>
        {kind === "omniroute" && (
          <p className="field-note">
            A remote gateway is recommended for long coding sessions on an 8 GB
            machine.
          </p>
        )}
      </div>
      <div className="modal-actions">
        <button type="button" className="button secondary" onClick={close}>
          Cancel
        </button>
        <button className="button primary" disabled={busy}>
          {busy && <Loader2 className="spin" size={15} />}Save connection
        </button>
      </div>
    </form>
  );
}

function WorkspaceForm({
  initial: w,
  busy,
  close,
  submit,
}: {
  initial?: Workspace;
  busy?: boolean;
  close: () => void;
  submit: (w: any) => void;
}) {
  const [path, setPath] = useState(w?.path || "");
  const [picking, setPicking] = useState(false);
  const [pickError, setPickError] = useState("");
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        const f = new FormData(e.currentTarget);
        submit({
          id: w?.id,
          name: f.get("name"),
          path,
          remote: f.get("remote") || "origin",
          branch: f.get("branch") || "",
          validationCommand: f.get("validationCommand") || "",
          autoPublish: f.get("autoPublish") === "on",
        });
      }}
    >
      <div className="modal-body">
        <label>
          Project name
          <input
            name="name"
            defaultValue={w?.name}
            placeholder="e.g. My next idea"
            required
          />
        </label>
        <label>
          Local directory
          <div className="input-button-row">
            <input
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="Choose an existing project folder"
              required
            />
            <button
              className="button secondary"
              type="button"
              disabled={picking}
              onClick={async () => {
                setPicking(true);
                setPickError("");
                try {
                  const r = await api("pick_folder");
                  if (r) setPath(typeof r === "string" ? r : r.path);
                } catch (e) {
                  setPickError(String(e));
                } finally {
                  setPicking(false);
                }
              }}
              aria-label="Browse project folder"
            >
              <FolderOpen size={17} />
            </button>
          </div>
        </label>
        {pickError && <p className="field-error">{pickError}</p>}
        <div className="form-row">
          <label>
            Git remote
            <input
              name="remote"
              defaultValue={w?.remote || "origin"}
              placeholder="origin"
            />
          </label>
          <label>
            Publication branch
            <input
              name="branch"
              defaultValue={w?.branch}
              placeholder="e.g. main"
            />
          </label>
        </div>
        <label>
          Validation command
          <input
            name="validationCommand"
            defaultValue={w?.validationCommand}
            placeholder="e.g. npm test"
          />
        </label>
        <label className="checkbox-field">
          <input
            type="checkbox"
            name="autoPublish"
            defaultChecked={w?.autoPublish}
          />
          <span>
            <strong>Automatically push after successful validation</strong>
            <small>
              Running validation marks a version boundary. Push only the exact
              verified candidate to the configured remote and branch.
            </small>
          </span>
        </label>
        <p className="field-note">
          Only configure commands you trust. They execute inside this project
          with your user permissions. Publishing is blocked until this command
          succeeds on the current candidate.
        </p>
      </div>
      <div className="modal-actions">
        <button type="button" className="button secondary" onClick={close}>
          Cancel
        </button>
        <button className="button primary" disabled={busy}>
          {busy && <Loader2 className="spin" size={15} />}Save project
        </button>
      </div>
    </form>
  );
}

function TaskForm({
  state,
  selected,
  close,
  submit,
}: {
  state: State;
  selected: string;
  close: () => void;
  submit: (p: any) => void;
}) {
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        const f = new FormData(e.currentTarget);
        submit({
          agentId: f.get("agentId"),
          prompt: f.get("prompt"),
          parentId: f.get("parentId") || null,
        });
      }}
    >
      <div className="modal-body">
        <label>
          Assign to
          <select
            name="agentId"
            defaultValue={selected || state.agents[0]?.id}
            required
          >
            {state.agents.map((a) => (
              <option value={a.id} key={a.id}>
                {a.name} · {a.role}
              </option>
            ))}
          </select>
        </label>
        <label>
          Desired outcome
          <textarea
            name="prompt"
            rows={5}
            placeholder="Describe the result you need, the relevant context, and how success should be checked."
            required
          />
        </label>
        <label>
          Parent task
          <select name="parentId">
            <option value="">Independent task</option>
            {state.tasks.map((t) => (
              <option value={t.id} key={t.id}>
                {t.prompt.slice(0, 80)}
              </option>
            ))}
          </select>
        </label>
        <p className="field-note">
          Independent tasks can run concurrently within the engine’s resource
          limits. A parent links related work; it does not invent a delegation
          result.
        </p>
      </div>
      <div className="modal-actions">
        <button type="button" className="button secondary" onClick={close}>
          Cancel
        </button>
        <button className="button primary">
          <Send size={14} />
          Assign task
        </button>
      </div>
    </form>
  );
}

function DrivePanel({
  state,
  run,
  busy,
  notify,
}: {
  state: State;
  run: (
    key: string,
    command: string,
    payload?: any,
    success?: string,
  ) => Promise<any>;
  busy: Record<string, boolean>;
  notify: (m: string, e?: boolean) => void;
}) {
  const [status, setStatus] = useState<any>(null);
  const [error, setError] = useState("");
  const [authorizing, setAuthorizing] = useState(false);
  const [project, setProject] = useState(state.workspaces[0]?.id || "");
  const [folderId, setFolderId] = useState("");
  const connected = !!(status?.connected || status?.status === "connected");
  const check = useCallback(() => {
    if (!isDesktop) return;
    api("drive_status")
      .then((s) => {
        setStatus(s);
        if (s?.error) {
          setError(String(s.error));
          setAuthorizing(false);
        }
        if (s?.connected || s?.status === "connected") {
          setAuthorizing(false);
          setError("");
        }
      })
      .catch(() => {});
  }, []);
  useEffect(() => {
    check();
  }, [check]);
  useEffect(() => {
    if (!authorizing) return;
    const timer = setInterval(check, 3000);
    return () => clearInterval(timer);
  }, [authorizing, check]);
  useEffect(() => {
    if (!project && state.workspaces.length) setProject(state.workspaces[0].id);
  }, [project, state.workspaces]);
  return (
    <section className="surface-card drive-panel">
      <div className="drive-header">
        <div className="integration-icon">G</div>
        <div>
          <strong>Google Drive snapshots</strong>
          <p>
            Back up a project to your Drive account. Local work stays available
            offline.
          </p>
        </div>
        {connected && (
          <span className="drive-connected">
            <CheckCircle2 size={14} />
            Connected
          </span>
        )}
      </div>
      {!connected ? (
        <form
          className="drive-form"
          onSubmit={async (e) => {
            e.preventDefault();
            const form = e.currentTarget;
            const f = new FormData(form);
            setError("");
            const result = await run("drive-connect", "connect_drive", {
              clientId: f.get("clientId"),
              clientSecret: f.get("clientSecret") || undefined,
            });
            if (result?.authorizationUrl) {
              try {
                await api("open_external", { url: result.authorizationUrl });
                setAuthorizing(true);
                (
                  form.elements.namedItem("clientSecret") as HTMLInputElement
                ).value = "";
                notify(
                  "Complete Google sign-in in your browser, then return here.",
                );
              } catch (e) {
                setError(String(e));
              }
            }
          }}
        >
          <div className="form-row">
            <label>
              Desktop OAuth client ID
              <input
                name="clientId"
                required
                autoComplete="off"
                placeholder="Your Google desktop OAuth client ID"
                disabled={!isDesktop}
              />
            </label>
            <label>
              OAuth client secret · if issued
              <input
                name="clientSecret"
                type="password"
                autoComplete="off"
                placeholder="Stored securely by the desktop app"
                disabled={!isDesktop}
              />
            </label>
          </div>
          <div className="drive-actions">
            <button
              className="button secondary"
              disabled={!isDesktop || busy["drive-connect"] || authorizing}
            >
              {busy["drive-connect"] || authorizing ? (
                <Loader2 size={14} className="spin" />
              ) : (
                <ExternalLink size={14} />
              )}
              {authorizing
                ? "Waiting for Google sign-in…"
                : "Connect Google Drive"}
            </button>
            {authorizing && (
              <button
                type="button"
                className="text-button"
                onClick={() => {
                  setAuthorizing(false);
                  check();
                }}
              >
                Refresh status
              </button>
            )}
          </div>
          <p className="drive-note">
            Create a Desktop app OAuth client in your Google Cloud project and
            enable the Drive API. Agent OS requests file access for its
            snapshots. It does not synchronize live Git metadata or your
            conversation database.
          </p>
        </form>
      ) : (
        <div className="drive-form">
          <div className="form-row">
            <label>
              Project to back up
              <select
                value={project}
                onChange={(e) => setProject(e.target.value)}
              >
                <option value="" disabled>
                  Select a project
                </option>
                {state.workspaces.map((w) => (
                  <option key={w.id} value={w.id}>
                    {w.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Drive folder ID · optional
              <input
                value={folderId}
                onChange={(e) => setFolderId(e.target.value)}
                placeholder="Leave empty for My Drive"
              />
            </label>
          </div>
          <div className="drive-actions">
            <button
              className="button primary"
              disabled={!project || busy["drive-upload"]}
              onClick={async () => {
                const r = await run("drive-upload", "upload_drive_snapshot", {
                  workspaceId: project,
                  folderId: folderId || undefined,
                });
                if (r) notify("Project snapshot uploaded to Google Drive.");
              }}
            >
              {busy["drive-upload"] ? (
                <Loader2 size={15} className="spin" />
              ) : (
                <ArrowUp size={15} />
              )}
              Upload snapshot
            </button>
            <button
              className="text-button"
              disabled={busy["drive-disconnect"]}
              onClick={async () => {
                const r = await run("drive-disconnect", "disconnect_drive");
                if (r !== undefined) {
                  setStatus(null);
                  check();
                }
              }}
            >
              Disconnect
            </button>
          </div>
          <p className="drive-note">
            Uploads are explicit versioned snapshots, not continuous two-way
            sync. Offline uploads report a connection failure; your local
            project remains untouched.
          </p>
        </div>
      )}
      {error && (
        <p className="drive-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
