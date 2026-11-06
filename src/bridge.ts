import { invoke } from "@tauri-apps/api/core";

export const isDesktop = "__TAURI_INTERNALS__" in window;

const preview = {
  agents: [
    {
      id: "preview-lead",
      name: "Atlas",
      role: "Team lead",
      provider: "",
      model: "",
      color: "#B4A7EA",
      instructions: "Coordinate clearly scoped tasks.",
      ponytail: true,
    },
    {
      id: "preview-builder",
      name: "Nova",
      role: "Builder",
      provider: "",
      model: "",
      supervisorId: "preview-lead",
      color: "#8BC4B0",
      instructions: "",
      ponytail: true,
    },
    {
      id: "preview-reviewer",
      name: "Sage",
      role: "Reviewer",
      provider: "",
      model: "",
      supervisorId: "preview-lead",
      color: "#E3B589",
      instructions: "",
      ponytail: true,
    },
  ],
  tasks: [],
  messages: [],
  connections: [],
  workspaces: [],
  settings: {
    lowPower: true,
    maxCloudTasks: 2,
    maxLocalTasks: 1,
    officeEnabled: true,
  },
};

export async function api<T = any>(
  command: string,
  payload: Record<string, unknown> = {},
): Promise<T> {
  if (isDesktop) return invoke<T>("execute", { command, payload });
  if (command === "bootstrap") return structuredClone(preview) as T;
  throw new Error(
    "This is the browser design preview. Open the Windows desktop app to connect providers, save projects, or run agents.",
  );
}
