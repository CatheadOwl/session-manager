import { describe, expect, it } from "vitest";
import {
  RESUME_TEMPLATE_EXAMPLE,
  effectiveResumeTemplate,
  renderResumeTemplate,
} from "./resumeTemplate";

const values = {
  resumeCommand: "opencode -s abc123",
  workDir: "D:\\Document\\Projects\\foo",
  sessionId: "abc123",
};

const DEFAULT = 'powershell -NoExit -Command "{ResumeCommand}"';

describe("renderResumeTemplate", () => {
  it("renders every documented placeholder", () => {
    expect(renderResumeTemplate("{SessionId}", values)).toBe("abc123");
    expect(renderResumeTemplate("{WorkDir}", values)).toBe("D:\\Document\\Projects\\foo");
    expect(renderResumeTemplate(DEFAULT, values)).toBe(
      'powershell -NoExit -Command "opencode -s abc123"',
    );
  });

  it("renders missing values as empty strings, never as the placeholder", () => {
    // Sessions without a project dir or resume command must not leak
    // `{WorkDir}` into the launched line.
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

  it("resolves the codex example end-to-end (settings preview parity)", () => {
    expect(
      renderResumeTemplate(
        effectiveResumeTemplate(true, "", DEFAULT),
        RESUME_TEMPLATE_EXAMPLE,
      ),
    ).toBe('powershell -NoExit -Command "codex resume a1b2c3"');
  });
});

describe("effectiveResumeTemplate", () => {
  it("gate off → null (raw resume command) regardless of the template", () => {
    expect(effectiveResumeTemplate(false, undefined, DEFAULT)).toBeNull();
    expect(effectiveResumeTemplate(false, "wt {ResumeCommand}", DEFAULT)).toBeNull();
  });

  it("gate on + blank template → the caller-supplied per-OS default", () => {
    expect(effectiveResumeTemplate(true, undefined, DEFAULT)).toBe(DEFAULT);
    expect(effectiveResumeTemplate(true, "", DEFAULT)).toBe(DEFAULT);
    // Whitespace-only counts as blank (hand-edited file residue).
    expect(effectiveResumeTemplate(true, "  \n ", DEFAULT)).toBe(DEFAULT);
  });

  it("gate on + real template → the template itself", () => {
    expect(effectiveResumeTemplate(true, "{SessionId}", DEFAULT)).toBe("{SessionId}");
  });
});
