import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SourceEntry } from "@/lib/api/settings";
import { SourcesEditor } from "./SourcesEditor";

afterEach(cleanup);

const PROVIDERS = ["claude", "codex", "gemini"];

const renderEditor = (
  value: SourceEntry[] = [],
  onChange: (next: SourceEntry[]) => void = () => {},
) =>
  render(
    <SourcesEditor
      label="Extra session sources"
      value={value}
      providers={PROVIDERS}
      onChange={onChange}
    />,
  );

describe("SourcesEditor", () => {
  it("shows the empty state when there are no sources", () => {
    renderEditor();
    expect(screen.getByText(/No extra sources/)).toBeInTheDocument();
  });

  it("adds a draft row with the first provider preselected", () => {
    const onChange = vi.fn();
    renderEditor([], onChange);
    fireEvent.click(screen.getByRole("button", { name: /Add source/ }));
    expect(onChange).toHaveBeenCalledWith([
      { path: "", provider: "claude", enabled: true },
    ]);
  });

  it("commits an edited path on blur and flags an empty path", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\old", provider: "codex", enabled: true }], onChange);
    const input = screen.getByLabelText("Source 1 path");
    fireEvent.change(input, { target: { value: "D:\\new" } });
    expect(onChange).not.toHaveBeenCalled(); // typing does not spam writes
    fireEvent.blur(input);
    expect(onChange).toHaveBeenCalledWith([
      { path: "D:\\new", provider: "codex", enabled: true },
    ]);

    fireEvent.change(input, { target: { value: "" } });
    fireEvent.blur(input);
    expect(screen.getByRole("alert")).toHaveTextContent("Path is required");
  });

  it("commits an edited path on Enter without blurring", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\old", provider: "codex", enabled: true }], onChange);
    const input = screen.getByLabelText("Source 1 path");
    fireEvent.change(input, { target: { value: "D:\\newer" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onChange).toHaveBeenCalledWith([
      { path: "D:\\newer", provider: "codex", enabled: true },
    ]);
  });

  it("flags duplicate paths per row (display-only)", () => {
    renderEditor([
      { path: "D:\\dup", provider: "codex", enabled: true },
      { path: "D:\\dup", provider: "claude", enabled: true },
    ]);
    const alerts = screen.queryAllByRole("alert");
    expect(alerts).toHaveLength(2);
    expect(alerts[0]).toHaveTextContent("Duplicate path");
  });

  it("changes the provider through the menu picker", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\x", provider: "codex", enabled: true }], onChange);
    fireEvent.click(screen.getByRole("button", { name: /^Source 1 provider: codex/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: "gemini" }));
    expect(onChange).toHaveBeenCalledWith([
      { path: "D:\\x", provider: "gemini", enabled: true },
    ]);
    expect(screen.queryByRole("menu")).toBeNull(); // picker closed after selection
  });

  it("toggles a source's enabled state", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\x", provider: "codex", enabled: true }], onChange);
    fireEvent.click(screen.getByRole("switch", { name: "Source 1 enabled" }));
    expect(onChange).toHaveBeenCalledWith([
      { path: "D:\\x", provider: "codex", enabled: false },
    ]);
  });

  it("removes a row only after confirming in the dialog", () => {
    const onChange = vi.fn();
    renderEditor([
      { path: "D:\\keep", provider: "claude", enabled: true },
      { path: "D:\\drop", provider: "codex", enabled: true },
    ], onChange);
    fireEvent.click(screen.getByRole("button", { name: "Remove source 2" }));

    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent("Remove source?");
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(onChange).toHaveBeenCalledWith([
      { path: "D:\\keep", provider: "claude", enabled: true },
    ]);
  });

  it("keeps the row when the removal is cancelled", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\keep", provider: "claude", enabled: true }], onChange);
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

  it("renders ssh entries as a read-only summary row", () => {
    renderEditor([SSH_ENTRY]);
    expect(screen.getByText("SSH")).toBeInTheDocument();
    expect(screen.getByText("Aliyun dev (ali)")).toBeInTheDocument();
    expect(screen.getByText("admin@192.0.2.10:2222")).toBeInTheDocument();
    expect(screen.getByText(/Edit in settings\.json — remote editor coming/)).toBeInTheDocument();
    // No editable controls for the ssh row.
    expect(screen.queryByLabelText("Source 1 path")).toBeNull();
    expect(screen.queryByRole("switch", { name: "Source 1 enabled" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Remove source 1" })).toBeNull();
  });

  it("includes ssh entries verbatim when a local edit commits", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\old", provider: "codex", enabled: true }, SSH_ENTRY], onChange);
    const input = screen.getByLabelText("Source 1 path");
    fireEvent.change(input, { target: { value: "D:\\new" } });
    fireEvent.blur(input);
    expect(onChange).toHaveBeenCalledWith([
      { path: "D:\\new", provider: "codex", enabled: true },
      SSH_ENTRY,
    ]);
  });

  it("includes ssh entries verbatim when a local row is removed", () => {
    const onChange = vi.fn();
    renderEditor([{ path: "D:\\drop", provider: "codex", enabled: true }, SSH_ENTRY], onChange);
    fireEvent.click(screen.getByRole("button", { name: "Remove source 1" }));
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(onChange).toHaveBeenCalledWith([SSH_ENTRY]);
  });
});
