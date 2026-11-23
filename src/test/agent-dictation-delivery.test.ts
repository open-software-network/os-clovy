import { beforeEach, describe, expect, it } from "vitest";
import {
  resetAgentSessionDraftsForTests,
  AGENT_DICTATION_DRAFT_WILL_CHANGE,
  deliverAgentDictation,
  invalidateAgentSessionDraft,
  readAgentSessionDraft,
  writeAgentSessionDraft,
} from "../lib/agent-session-drafts";

describe("bound dictation composer delivery", () => {
  beforeEach(() => resetAgentSessionDraftsForTests());
  it("appends once to the fixed conversation even when another composer owns current text", () => {
    writeAgentSessionDraft("first-conversation", "Existing draft");
    writeAgentSessionDraft("other-conversation", "Do not change");
    const delivery = {
      takeId: "take-one",
      composerRequestId: "composer-one",
      conversationId: "first-conversation",
      profile: "first-partition",
      text: "Dictated text",
    };
    expect(deliverAgentDictation(delivery)).toBe(true);
    expect(deliverAgentDictation(delivery)).toBe(true);
    expect(readAgentSessionDraft("first-conversation")).toBe("Existing draft\nDictated text");
    expect(readAgentSessionDraft("other-conversation")).toBe("Do not change");
  });
  it("rejects late text for a deleted conversation and never creates a replacement draft", () => {
    invalidateAgentSessionDraft("deleted-conversation");
    expect(
      deliverAgentDictation({
        takeId: "late-take",
        composerRequestId: "late-composer",
        conversationId: "deleted-conversation",
        profile: "default",
        text: "Late text",
      }),
    ).toBe(false);
    expect(readAgentSessionDraft("deleted-conversation")).toBeUndefined();
  });
  it("flushes concurrent editor text before appending and respects an unfinished composition", () => {
    const delivery = {
      takeId: "concurrent-take",
      composerRequestId: "concurrent-composer",
      conversationId: "current",
      profile: "default",
      text: "Dictation",
    };
    const flush = () => writeAgentSessionDraft("current", "Typed while transcribing");
    window.addEventListener(AGENT_DICTATION_DRAFT_WILL_CHANGE, flush);
    expect(deliverAgentDictation(delivery)).toBe(true);
    window.removeEventListener(AGENT_DICTATION_DRAFT_WILL_CHANGE, flush);
    expect(readAgentSessionDraft("current")).toBe("Typed while transcribing\nDictation");
    const composing = (event: Event) => event.preventDefault();
    window.addEventListener(AGENT_DICTATION_DRAFT_WILL_CHANGE, composing);
    expect(deliverAgentDictation({ ...delivery, takeId: "blocked-take", text: "Must wait" })).toBe(
      false,
    );
    window.removeEventListener(AGENT_DICTATION_DRAFT_WILL_CHANGE, composing);
    expect(readAgentSessionDraft("current")).toBe("Typed while transcribing\nDictation");
  });
});
