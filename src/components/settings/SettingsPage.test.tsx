import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SettingsSnapshot } from "@/lib/api/settings";
import { SettingsPage } from "./SettingsPage";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

const fixture = (): SettingsSnapshot => ({
  version: 1,
  descriptors: [
    { key: "update.autoCheck", type: "bool", default: { bool: true }, group: "update" },
    { key: "sources", type: "sourceList", default: { sourceList: [] }, group: "sources" },
  ],
  values: {
    "update.autoCheck": { bool: true },
    sources: {
      sourceList: [
        // Local entries are `{ path, enabled }` — a legacy
        // `provider` key (as the loader preserves it) must still render.
        { path: "D:\\dump", provider: "codex", enabled: true },
        // Mixed-kind lists must render (ssh read-only row) without
        // crashing the single sourceList renderer.
        {
          kind: "ssh",
          id: "ali",
          host: "192.0.2.10",
          user: "admin",
          auth: { mode: "agent" },
          enabled: true,
        },
      ],
    },
  },
});

describe("SettingsPage", () => {
  let queryClient: QueryClient;
  const onClose = vi.fn();

  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  const renderPage = () => render(<SettingsPage onClose={onClose} />, { wrapper });

  beforeEach(() => {
    mocks.invoke.mockReset();
    onClose.mockReset();
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "get_settings") return Promise.resolve(fixture());
      if (command === "list_providers") return Promise.resolve(["claude", "codex", "gemini"]);
      return Promise.resolve(null);
    });
  });

  afterEach(cleanup);

  it("renders categories and descriptors from the snapshot", async () => {
    renderPage();
    expect(await screen.findByRole("switch", { name: "Check for updates automatically" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByRole("navigation", { name: "Settings categories" })).toHaveTextContent("General");
    fireEvent.click(screen.getByRole("button", { name: "Sources" }));
    expect(screen.getByLabelText("Source 1 path")).toHaveValue("D:\\dump");
    // Mixed-kind list: the ssh entry renders as a read-only summary row.
    expect(screen.getByText("SSH")).toBeInTheDocument();
    expect(screen.getByText("ali")).toBeInTheDocument();
  });

  it("switches categories via the sidebar", async () => {
    renderPage();
    await screen.findByRole("switch", { name: "Check for updates automatically" });
    fireEvent.click(screen.getByRole("button", { name: "Sources" }));
    expect(screen.queryByRole("switch", { name: "Check for updates automatically" })).toBeNull();
    expect(screen.getByLabelText("Source 1 path")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Sources" })).toBeInTheDocument();
  });

  it("writes per-key through set_setting_value on toggle", async () => {
    renderPage();
    const sw = await screen.findByRole("switch", { name: "Check for updates automatically" });
    fireEvent.click(sw);
    await waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith("set_setting_value", {
        key: "update.autoCheck",
        value: { bool: false },
      });
    });
    // Optimistic cache update happened before the write settled.
    const cached = queryClient.getQueryData<SettingsSnapshot>(["settings"]);
    expect(cached?.values["update.autoCheck"]).toEqual({ bool: false });
  });

  it("surfaces a failed write inline on the row and reverts the optimistic value", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "set_setting_value") return Promise.reject("write failed");
      if (command === "get_settings") return Promise.resolve(fixture());
      if (command === "list_providers") return Promise.resolve(["claude", "codex"]);
      return Promise.resolve(null);
    });
    renderPage();
    fireEvent.click(await screen.findByRole("switch", { name: "Check for updates automatically" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("write failed");
    const cached = queryClient.getQueryData<SettingsSnapshot>(["settings"]);
    expect(cached?.values["update.autoCheck"]).toEqual({ bool: true });
  });

  it("renders a visible fallback row for an unsupported setting type", async () => {
    const snapshot = fixture();
    snapshot.descriptors = [
      ...snapshot.descriptors,
      // A type no v1 renderer covers — must render a visible fallback, not crash.
      { key: "future.knob", type: "fancy" as never, default: { string: "" }, group: "update" },
    ];
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "get_settings") return Promise.resolve(snapshot);
      if (command === "list_providers") return Promise.resolve(["claude"]);
      return Promise.resolve(null);
    });
    renderPage();
    expect(await screen.findByText("Unsupported setting type: fancy")).toBeInTheDocument();
  });

  it("closes on Esc, backdrop click, and the back button", async () => {
    const { container } = renderPage();
    await screen.findByRole("switch", { name: "Check for updates automatically" });

    fireEvent.click(screen.getByRole("button", { name: "Close settings" }));
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(2);

    fireEvent.click(container.querySelector(".settings-overlay") as HTMLElement);
    expect(onClose).toHaveBeenCalledTimes(3);

    // Clicks inside the page do NOT close it.
    fireEvent.click(screen.getByRole("dialog", { name: "Settings" }));
    expect(onClose).toHaveBeenCalledTimes(3);
  });
});
