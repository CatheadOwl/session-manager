import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { SessionMessage } from "@/types";
import { SessionQaPair } from "./SessionQaPair";

const messages: SessionMessage[] = [
  { role: "user", content: "Where is the search target?", ts: 1 },
  { role: "assistant", content: "The answer contains Needle in the body.", ts: 2 },
];

const interleavedMessages: SessionMessage[] = [
  { role: "user", content: "Question without the hidden term", ts: 1 },
  { role: "system", content: "Needle in a non-rendered QA-mode message", ts: 2 },
  { role: "assistant", content: "Answer without the hidden term", ts: 3 },
];

describe("SessionQaPair", () => {
  it("marks Q&A message blocks with source message indexes for search navigation", () => {
    const { container } = render(
      <SessionQaPair
        pair={{ questionIdx: 0, answerIdx: 1 }}
        messages={messages}
        index={0}
        questionJumpIndex={0}
        showRendered={false}
      />,
    );

    expect(container.querySelector(".qa-question")?.getAttribute("data-msg-idx")).toBe("0");
    expect(container.querySelector(".qa-answer")?.getAttribute("data-msg-idx")).toBe("1");
  });

  it("highlights answer matches in Q&A mode", () => {
    const { container } = render(
      <SessionQaPair
        pair={{ questionIdx: 0, answerIdx: 1 }}
        messages={messages}
        index={0}
        questionJumpIndex={0}
        showRendered={false}
        answerSearchActive
        answerSearchCurrent
        searchQuery="needle"
      />,
    );

    const answer = container.querySelector(".qa-answer");
    expect(answer).toHaveClass("search-match-current");
    expect(screen.getByText("Needle")).toHaveClass("highlight");
  });

  it("does not render search anchors for interleaved non-QA messages", () => {
    const { container } = render(
      <SessionQaPair
        pair={{ questionIdx: 0, answerIdx: 2 }}
        messages={interleavedMessages}
        index={0}
        questionJumpIndex={0}
        showRendered={false}
      />,
    );

    expect(container.querySelector('[data-msg-idx="1"]')).not.toBeInTheDocument();
  });
});
