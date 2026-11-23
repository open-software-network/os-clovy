import { beforeEach, describe, expect, it } from "vitest";
import {
  ATTACHMENT_FOLLOW_UP_NOTE,
  clearQueuedAgentFollowUpSteering,
  loadQueuedAgentFollowUps,
  mergeQueuedAgentFollowUp,
  reconcileConsumedAgentFollowUp,
  saveQueuedAgentFollowUps,
} from "../lib/agent-follow-up-queue";

const queued = {
  "session-1": {
    messageId: "message-1",
    prompt: "Continue with the attachment",
    attachments: ["/tmp/brief.pdf"],
    model: "open-software/auto",
    delivery: "follow_up" as const,
    steering: "accepted" as const,
  },
};

describe("agent follow-up queue", () => {
  beforeEach(() => window.localStorage.clear());

  it("survives a workspace remount", () => {
    saveQueuedAgentFollowUps(queued);
    expect(loadQueuedAgentFollowUps()).toEqual({
      "session-1": {
        ...queued["session-1"],
        steering: undefined,
      },
    });
  });

  it.each([
    "pending",
    "accepted",
  ] as const)("restores persisted %s steering as a plain queued follow-up", (steering) => {
    saveQueuedAgentFollowUps({
      "session-1": { ...queued["session-1"], steering },
    });
    expect(loadQueuedAgentFollowUps()["session-1"]).not.toHaveProperty("steering");
  });

  it("merges attachments into the latest queued submission by path", () => {
    expect(
      mergeQueuedAgentFollowUp(
        {
          ...queued["session-1"],
          prompt: "Earlier prompt",
          attachments: ["/tmp/brief.pdf"],
          delivery: "attachments",
          steering: undefined,
        },
        {
          ...queued["session-1"],
          messageId: "message-2",
          prompt: "Latest prompt",
          attachments: ["/tmp/brief.pdf", "/tmp/notes.md"],
          steering: "pending",
        },
      ),
    ).toEqual({
      ...queued["session-1"],
      messageId: "message-2",
      prompt: "Latest prompt",
      attachments: ["/tmp/brief.pdf", "/tmp/notes.md"],
      steering: "pending",
    });
  });

  it("downgrades an unconsumed steering attempt to a plain queued follow-up", () => {
    expect(clearQueuedAgentFollowUpSteering(queued, "session-1")).toEqual({
      "session-1": {
        ...queued["session-1"],
        steering: undefined,
      },
    });
  });

  it("drops a fallback once persisted history proves steering consumed it", () => {
    const textOnly = {
      "session-1": {
        ...queued["session-1"],
        attachments: [],
      },
    };
    expect(reconcileConsumedAgentFollowUp(textOnly, "session-1", ["steering:message-1"])).toEqual(
      {},
    );
    expect(reconcileConsumedAgentFollowUp(textOnly, "session-1", ["steering:other"])).toBe(
      textOnly,
    );
  });

  it("keeps pending attachments after steering consumes the text", () => {
    expect(reconcileConsumedAgentFollowUp(queued, "session-1", ["steering:message-1"])).toEqual({
      "session-1": {
        messageId: "message-1",
        prompt: `Continue with the attachment\n\n${ATTACHMENT_FOLLOW_UP_NOTE}`,
        attachments: ["/tmp/brief.pdf"],
        model: "open-software/auto",
        delivery: "attachments",
      },
    });
    expect(reconcileConsumedAgentFollowUp(queued, "session-1", ["steering:other"])).toBe(queued);
  });

  it("ignores malformed stored entries", () => {
    window.localStorage.setItem(
      "clovy.agent.queuedFollowUps",
      JSON.stringify({ valid: queued["session-1"], invalid: { prompt: 42 } }),
    );
    expect(loadQueuedAgentFollowUps()).toEqual({
      valid: { ...queued["session-1"], steering: undefined },
    });
  });
});
