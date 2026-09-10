import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SourceEntry } from "@/lib/api/settings";
import { SourcesEditor } from "./SourcesEditor";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

afterEach(cleanup);

const renderEditor = (
  value: SourceEntry[] = [],
  onChange: (next: SourceEntry[]) => void = () => {},
) =>
  render(
    <SourcesEditor
      label="Extra session sources"
      value={value}
      onChange={onChange}
    />,
  );

describe("SourcesEditor", () => {
  it("shows the empty state when there are no sources", () => {
    renderEditor();
    expect(screen.getByText(/No extra sources/)).toBeInTheDocument();
  });

  it("adds a draft row with the minimal local shape (no provider)", () => {
    const onChange = vi.fn();
    renderEditor([], onChange);
    fireEvent.click(screen.getByRole("button", { name: /Add source/ }));
    expect(onChange).toHaveBeenCalledWith([
      { path: "", enabled: true },
    ]);
  });

  it("commits an edited path on blur and flags an empty path", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\old", enabled: true }], onChange);
    const input = screen.getByLabelText("Source 1 path");
    fireEvent.change(input, { target: { value: "D:\\new" } });
    expect(onChange).not.toHaveBeenCalled(); // typing does not spam writes
    fireEvent.blur(input);
    expect(onChange).toHaveBeenCalledWith([
      { path: "D:\\new", enabled: true },
    ]);

    fireEvent.change(input, { target: { value: "" } });
    fireEvent.blur(input);
    expect(screen.getByRole("alert")).toHaveTextContent("Path is required");
  });

  it("commits an edited path on Enter without blurring", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\old", enabled: true }], onChange);
    const input = screen.getByLabelText("Source 1 path");
    fireEvent.change(input, { target: { value: "D:\\newer" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onChange).toHaveBeenCalledWith([
      { path: "D:\\newer", enabled: true },
    ]);
  });

  it("flags duplicate paths per row (display-only)", () => {
    renderEditor([
      { path: "D:\\dup", enabled: true },
      { path: "D:\\dup", enabled: true },
    ]);
    const alerts = screen.queryAllByRole("alert");
    expect(alerts).toHaveLength(2);
    expect(alerts[0]).toHaveTextContent("Duplicate path");
  });

  it("renders a legacy provider key without any provider picker", () => {
    // ADR 0011: a pre-0011 file's `provider` key is preserved by the
    // loader and must round-trip; the row itself has NO provider menu.
    renderEditor([{ path: "D:\\dump", provider: "codex", enabled: true }]);
    expect(screen.getByLabelText("Source 1 path")).toHaveValue("D:\\dump");
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("toggles a source's enabled state", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\x", enabled: true }], onChange);
    fireEvent.click(screen.getByRole("switch", { name: "Source 1 enabled" }));
    expect(onChange).toHaveBeenCalledWith([
      { path: "D:\\x", enabled: false },
    ]);
  });

  it("removes a row only after confirming in the dialog", () => {
    const onChange = vi.fn();
    renderEditor([
      { path: "D:\\keep", enabled: true },
      { path: "D:\\drop", enabled: true },
    ], onChange);
    fireEvent.click(screen.getByRole("button", { name: "Remove source 2" }));

    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent("Remove source?");
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(onChange).toHaveBeenCalledWith([
      { path: "D:\\keep", enabled: true },
    ]);
  });

  it("keeps the row when the removal is cancelled", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\keep", enabled: true }], onChange);
    fireEvent.click(screen.getByRole("button", { name: "Remove source 1" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  // ADR 0008: ssh entries are read-only in the UI until the remote-line
  // editor lands, and every commit must carry them verbatim.
  const SSH_ENTRY: SourceEntry = {
    kind: "ssh",
    id: "ali",
    label: "Aliyun dev",
    host: "192.0.2.10",
    port: 2222,
    user: "admin",
    auth: { mode: "key", keyPath: "~/.ssh/id_ed25519" },
    enabled: true,
  };

  it("renders ssh rows with unified structural controls (toggle + guarded remove)", () => {
    renderEditor([SSH_ENTRY]);
    expect(screen.getByText("SSH")).toBeInTheDocument();
    // One-line identity: title · host on the same summary line.
    expect(screen.getByText("Aliyun dev (ali)")).toBeInTheDocument();
    expect(screen.getByText("admin@192.0.2.10:2222")).toBeInTheDocument();
    const summary = screen.getByText("Aliyun dev (ali)").closest(".setting-source-ssh-summary");
    expect(summary).toContainElement(screen.getByText("admin@192.0.2.10:2222"));
    // Same structural controls as local rows, keyed by the ssh id.
    const toggle = screen.getByRole("switch", { name: "SSH source ali enabled" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("button", { name: "Remove SSH source ali" })).toBeInTheDocument();
    // No path input for the ssh row — identity stays read-only.
    expect(screen.queryByLabelText("Source 1 path")).toBeNull();
  });

  it("commits an enabled flip when the ssh row toggle is clicked", () => {
    const onChange = vi.fn();
    renderEditor([SSH_ENTRY], onChange);
    fireEvent.click(screen.getByRole("switch", { name: "SSH source ali enabled" }));
    expect(onChange).toHaveBeenCalledWith([{ ...SSH_ENTRY, enabled: false }]);
  });

  it("removes an ssh row through the confirm dialog (ssh-scoped copy)", () => {
    const onChange = vi.fn();
    renderEditor([SSH_ENTRY], onChange);
    fireEvent.click(screen.getByRole("button", { name: "Remove SSH source ali" }));
    expect(screen.getByText("Remove SSH source?")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(onChange).toHaveBeenCalledWith([]);
  });

  it("includes ssh entries verbatim when a local edit commits", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\old", enabled: true }, SSH_ENTRY], onChange);
    const input = screen.getByLabelText("Source 1 path");
    fireEvent.change(input, { target: { value: "D:\\new" } });
    fireEvent.blur(input);
    expect(onChange).toHaveBeenCalledWith([
      { path: "D:\\new", enabled: true },
      SSH_ENTRY,
    ]);
  });

  it("includes ssh entries verbatim when a local row is removed", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\drop", enabled: true }, SSH_ENTRY], onChange);
    fireEvent.click(screen.getByRole("button", { name: "Remove source 1" }));
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(onChange).toHaveBeenCalledWith([SSH_ENTRY]);
  });

  it("renders the sshConfig alias reference on the summary row without resolving", () => {
    renderEditor([{
      ...SSH_ENTRY,
      host: "",
      user: "",
      auth: { mode: "sshConfig" as const, alias: "ali" },
    }]);
    expect(screen.getByText("ssh config alias: ali")).toBeInTheDocument();
  });

  // ── Add SSH source flow (ADR 0010) ────────────────────────────────────

  const ALIASES = [
    { alias: "ali", host: "192.0.2.10", user: "admin", supported: true },
    { alias: "jumped", host: "192.0.2.20", user: "op", supported: false },
  ];

  beforeEach(() => {
    mocks.invoke.mockReset();
  });

  const openAddPanel = () => {
    fireEvent.click(screen.getByRole("button", { name: /Add SSH source/ }));
  };

  it("lists ssh config aliases with previews and greys out ProxyJump", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_ssh_aliases") return Promise.resolve(ALIASES);
      if (command === "get_ssh_config_path") return Promise.resolve("C:/Users/u/.ssh/config");
      return Promise.resolve(null);
    });
    renderEditor();
    openAddPanel();
    expect(await screen.findByText("ali")).toBeInTheDocument();
    expect(screen.getByText("admin@192.0.2.10")).toBeInTheDocument();
    // ProxyJump row: greyed (disabled) + explicit note.
    const jumped = screen.getByRole("button", { name: /jumped/ });
    expect(jumped).toBeDisabled();
    expect(screen.getByText(/ProxyJump — not supported yet/)).toBeInTheDocument();
    // With nothing selected, test/add are disabled.
    expect(screen.getByRole("button", { name: "Test connection" })).toBeDisabled();
    expect(screen.getByRole("button", { name: /^Add$/ })).toBeDisabled();
  });

  it("select → test → add commits a sshConfig entry and closes the panel", async () => {
    mocks.invoke.mockImplementation((command: string, args?: { request?: unknown }) => {
      if (command === "list_ssh_aliases") return Promise.resolve(ALIASES);
      if (command === "get_ssh_config_path") return Promise.resolve("C:/Users/u/.ssh/config");
      if (command === "test_ssh_source") {
        expect((args?.request as { auth: { alias: string } }).auth.alias).toBe("ali");
        return Promise.resolve({ ok: true, sessionCount: 3 });
      }
      return Promise.resolve(null);
    });
    const onChange = vi.fn();
    renderEditor([], onChange);
    openAddPanel();
    fireEvent.click(await screen.findByRole("button", { name: /^ali/ }));
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Connected — 3 sessions found");
    fireEvent.click(screen.getByRole("button", { name: /^Add$/ }));
    expect(onChange).toHaveBeenCalledWith([
      {
        kind: "ssh",
        id: "ali",
        host: "",
        port: 22,
        user: "",
        auth: { mode: "sshConfig", alias: "ali" },
        enabled: true,
      },
    ]);
    // Panel closed after adding.
    expect(screen.queryByRole("group", { name: "Add SSH source" })).toBeNull();
  });

  it("shows the actionable error when the test connection fails", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_ssh_aliases") return Promise.resolve(ALIASES);
      if (command === "get_ssh_config_path") return Promise.resolve("C:/Users/u/.ssh/config");
      if (command === "test_ssh_source") {
        return Promise.resolve({
          ok: false,
          error: "ssh config alias `ali` uses ProxyJump, not supported yet",
        });
      }
      return Promise.resolve(null);
    });
    renderEditor();
    openAddPanel();
    fireEvent.click(await screen.findByRole("button", { name: /^ali/ }));
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/not supported yet/);
  });

  it("empty state shows the config path guide and re-lists on refresh", async () => {
    let listCalls = 0;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_ssh_aliases") {
        listCalls += 1;
        return Promise.resolve(listCalls === 1 ? [] : ALIASES);
      }
      if (command === "get_ssh_config_path") {
        return Promise.resolve("C:/Users/u/.ssh/config");
      }
      return Promise.resolve(null);
    });
    renderEditor();
    openAddPanel();
    // Guide: copyable full path + three-line example + refresh.
    expect(await screen.findByText("C:/Users/u/.ssh/config")).toBeInTheDocument();
    // Three-line example block (whitespace-normalized text match).
    expect(screen.getByText(/Host ali\s+HostName 192\.0\.2\.10\s+User admin/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Copy config path" }));
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(await screen.findByRole("button", { name: /^ali/ })).toBeInTheDocument();
  });

  it("empty state manual form: fill → add commits a hand-filled entry", async () => {
    mocks.invoke.mockImplementation((command: string, args?: { request?: unknown }) => {
      if (command === "list_ssh_aliases") return Promise.resolve([]);
      if (command === "get_ssh_config_path") return Promise.resolve("C:/Users/u/.ssh/config");
      if (command === "test_ssh_source") {
        const req = args?.request as { host: string; user: string; port: number };
        expect(req.host).toBe("192.0.2.10");
        expect(req.user).toBe("admin");
        return Promise.resolve({ ok: true, sessionCount: 0 });
      }
      return Promise.resolve(null);
    });
    const onChange = vi.fn();
    renderEditor([], onChange);
    openAddPanel();
    fireEvent.click(await screen.findByRole("button", { name: /Advanced: manual configuration/ }));
    fireEvent.change(screen.getByLabelText("SSH host"), { target: { value: "192.0.2.10" } });
    fireEvent.change(screen.getByLabelText("SSH port"), { target: { value: "2222" } });
    fireEvent.change(screen.getByLabelText("SSH user"), { target: { value: "admin" } });
    // Key mode surfaces the keyPath input with a ~ hint.
    fireEvent.click(screen.getByRole("button", { name: /SSH auth mode/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Key file" }));
    fireEvent.change(screen.getByLabelText("SSH key path"), {
      target: { value: "~/.ssh/id_ed25519" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    expect(await screen.findByRole("status")).toHaveTextContent("0 sessions found");
    fireEvent.click(screen.getByRole("button", { name: /^Add$/ }));
    expect(onChange).toHaveBeenCalledWith([
      {
        kind: "ssh",
        id: "192.0.2.10",
        host: "192.0.2.10",
        port: 2222,
        user: "admin",
        auth: { mode: "key", keyPath: "~/.ssh/id_ed25519" },
        enabled: true,
      },
    ]);
  });

  it("surfaces a listing error with a retry that recovers", async () => {
    let fail = true;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_ssh_aliases") {
        if (fail) return Promise.reject("cannot open ssh config");
        return Promise.resolve(ALIASES);
      }
      if (command === "get_ssh_config_path") return Promise.resolve("C:/Users/u/.ssh/config");
      return Promise.resolve(null);
    });
    renderEditor();
    openAddPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent("cannot open ssh config");
    fail = false;
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /^ali/ })).toBeInTheDocument();
    });
  });
});
