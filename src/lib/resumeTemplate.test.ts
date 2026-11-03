import { describe, expect, it } from "vitest";
import { renderResumeTemplate } from "./resumeTemplate";

const values = {
  resumeCommand: "opencode -s abc123",
  workDir: "D:\\Document\\Projects\\foo",
  sessionId: "abc123",
};

describe("renderResumeTemplate", () => {
  it("falls back to the raw resume command on an empty or missing template", () => {
    expect(renderResumeTemplate(undefined, values)).toBe("opencode -s abc123");
    expect(renderResumeTemplate("", values)).toBe("opencode -s abc123");
    // Whitespace-only counts as empty (hand-edited file residue).
    expect(renderResumeTemplate("   ", values)).toBe("opencode -s abc123");
  });

  it("renders every documented placeholder", () => {
    expect(renderResumeTemplate("{SessionId}", values)).toBe("abc123");
    expect(renderResumeTemplate("{WorkDir}", values)).toBe("D:\\Document\\Projects\\foo");
    expect(
      renderResumeTemplate('cd "{WorkDir}" && {ResumeCommand}', values),
    ).toBe('cd "D:\\Document\\Projects\\foo" && opencode -s abc123');
  });

  it("renders missing values as empty strings, never as the placeholder", () => {
    // Sessions without a project dir or resume command must not leak
    // `{WorkDir}` into the copied line.
    expect(renderResumeTemplate("cd {WorkDir}", { ...values, workDir: null })).toBe("cd ");
    expect(renderResumeTemplate("{ResumeCommand}", { ...values, resumeCommand: null })).toBe("");
  });

  it("keeps unknown placeholder names literal so typos stay visible", () => {
    expect(renderResumeTemplate("{WorkDi} {SessionId}", values)).toBe("{WorkDi} abc123");
    // Braces without a placeholder shape are untouched.
    expect(renderResumeTemplate("a { b", values)).toBe("a { b");
  });

  it("does not touch shell syntax around the placeholders", () => {
    expect(renderResumeTemplate("$env:{SessionId}$(echo)", values)).toBe("$env:abc123$(echo)");
  });
});
