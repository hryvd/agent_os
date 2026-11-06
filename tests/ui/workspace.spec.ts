import { test, expect } from "@playwright/test";

test("preview is honest, navigable, and keeps agent selection synchronized", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/");
  await expect(
    page.getByText("Browser preview", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Your team, in sync." }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Create agent", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Team view", exact: true }).click();
  await expect(page.getByText("Atlas", { exact: true }).first()).toBeVisible();
  await page.locator(".team-grid-card").filter({ hasText: "Nova" }).click();
  await expect(
    page.getByRole("textbox", { name: "Message Nova", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Connections", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "The intelligence behind it." }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Add connection", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Workspaces", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "A place for every project." }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Tasks", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "From intent to done." }),
  ).toBeVisible();
  await expect(page.locator("body")).not.toContainText(
    "Task completed successfully",
  );
  expect(errors).toEqual([]);
});

test("desktop connection form sends the configured provider through the native bridge", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const state: any = {
      agents: [],
      tasks: [],
      messages: [],
      connections: [],
      workspaces: [],
      settings: {},
    };
    (window as any).__commands = [];
    (window as any).__TAURI_INTERNALS__ = {
      invoke: async (_: string, args: any) => {
        (window as any).__commands.push(args);
        if (args.command === "bootstrap") return structuredClone(state);
        if (args.command === "save_connection") {
          const connection = { ...args.payload.connection, id: "test-local" };
          delete connection.apiKey;
          state.connections.push(connection);
          return connection;
        }
        throw new Error(`Unexpected test command: ${args.command}`);
      },
    };
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Connections", exact: true }).click();
  await page
    .getByRole("button", { name: "Add connection", exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await dialog
    .getByLabel("Connection name", { exact: true })
    .fill("My local Ollama");
  await dialog
    .getByRole("button", { name: "Save connection", exact: true })
    .click();
  await expect(dialog).not.toBeVisible();
  await expect(
    page.getByText("My local Ollama", { exact: true }),
  ).toBeVisible();
  const saved = await page.evaluate(() =>
    (window as any).__commands.find(
      (c: any) => c.command === "save_connection",
    ),
  );
  expect(saved.payload.connection.name).toBe("My local Ollama");
  expect(saved.payload.connection.kind).toBe("ollama");
  const endpoint = new URL(saved.payload.connection.baseUrl);
  expect(["localhost", "127.0.0.1"]).toContain(endpoint.hostname);
  expect(endpoint.port).toBe("11434");
});
