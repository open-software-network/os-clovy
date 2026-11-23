import { beforeEach, describe, expect, it } from "vitest";
import {
  forgetClovyHomeStoredSessionId,
  isClovyHomeStartTaskTool,
  clovyHomeDailyCheckIn,
  clovyHomeDayKey,
  clovyHomeDayLabel,
  clovyHomeGreetingParts,
  clovyHomeNudgePrompts,
  clovyHomeProfileRemovalPlan,
  readClovyHomeStoredSessionId,
  reconcileClovyHomeProfileRemoval,
  stripClovyHomeContext,
  stripClovyHomeContextFromPreview,
  writeClovyHomeStoredSessionId,
} from "../lib/clovy-home";
import {
  compareHomeTurnOrder,
  homeTaskCardTurns,
  readHomeTaskHandoffs,
  readLegacyHomeDirectTurns,
} from "../components/agent/home-thread";

describe("Clovy Home", () => {
  beforeEach(() => window.localStorage.clear());

  it("stores one Home session per profile", () => {
    writeClovyHomeStoredSessionId("default", "session-default");
    writeClovyHomeStoredSessionId("work", "session-work");

    expect(readClovyHomeStoredSessionId("default")).toBe("session-default");
    expect(readClovyHomeStoredSessionId("work")).toBe("session-work");

    forgetClovyHomeStoredSessionId("work", "another-session");
    expect(readClovyHomeStoredSessionId("work")).toBe("session-work");
    forgetClovyHomeStoredSessionId("work", "session-work");
    expect(readClovyHomeStoredSessionId("work")).toBeUndefined();
  });

  it("moves a removed profile's Home thread into the existing default thread", () => {
    writeClovyHomeStoredSessionId("default", "home-default");
    writeClovyHomeStoredSessionId("research", "home-research");
    window.localStorage.setItem(
      "clovy:home:direct-turns:v1",
      JSON.stringify({
        "home-default": [{ id: "default-turn", createdAt: "2026-07-24T09:00:00Z" }],
        "home-research": [{ id: "research-turn", createdAt: "2026-07-24T10:00:00Z" }],
      }),
    );
    window.localStorage.setItem(
      "clovy:home:task-handoffs:v1",
      JSON.stringify({
        "home-default": [{ id: "default-task" }],
        "home-research": [{ id: "research-task" }],
      }),
    );
    window.localStorage.setItem(
      "clovy:home:check-ins:v1",
      JSON.stringify({
        default: { date: "2026-07-24", createdAt: "2026-07-24T09:00:00Z" },
        research: { date: "2026-07-24", createdAt: "2026-07-24T10:00:00Z" },
      }),
    );

    expect(clovyHomeProfileRemovalPlan("research", "move")).toEqual({
      sourceSessionId: "home-research",
      targetSessionId: "home-default",
      redundantSessionId: "home-research",
    });
    reconcileClovyHomeProfileRemoval("research", "move");

    expect(readClovyHomeStoredSessionId("research")).toBeUndefined();
    expect(readClovyHomeStoredSessionId("default")).toBe("home-default");
    const turns = JSON.parse(
      window.localStorage.getItem("clovy:home:direct-turns:v1") ?? "{}",
    ) as Record<string, Array<{ id: string }>>;
    expect(turns["home-default"].map((turn) => turn.id)).toEqual(["default-turn", "research-turn"]);
    expect(turns).not.toHaveProperty("home-research");
    const handoffs = JSON.parse(
      window.localStorage.getItem("clovy:home:task-handoffs:v1") ?? "{}",
    ) as Record<string, Array<{ id: string }>>;
    expect(handoffs["home-default"].map((handoff) => handoff.id)).toEqual([
      "default-task",
      "research-task",
    ]);
    expect(handoffs).not.toHaveProperty("home-research");
    const checkIns = JSON.parse(
      window.localStorage.getItem("clovy:home:check-ins:v1") ?? "{}",
    ) as Record<string, unknown>;
    expect(checkIns).toHaveProperty("default");
    expect(checkIns).not.toHaveProperty("research");
  });

  it("purges current and legacy Home history when a profile is deleted permanently", () => {
    writeClovyHomeStoredSessionId("research", "home-research");
    for (const key of [
      "clovy:home:direct-turns:v1",
      "clovy:home:task-handoffs:v1",
      "clovy.home.directTurns.v1",
      "clovy.home.taskHandoffs.v1",
    ]) {
      window.localStorage.setItem(
        key,
        JSON.stringify({ "home-research": [{ id: `${key}:private` }] }),
      );
    }

    reconcileClovyHomeProfileRemoval("research", "delete");

    expect(readClovyHomeStoredSessionId("research")).toBeUndefined();
    for (const key of [
      "clovy:home:direct-turns:v1",
      "clovy:home:task-handoffs:v1",
      "clovy.home.directTurns.v1",
      "clovy.home.taskHandoffs.v1",
    ]) {
      expect(JSON.parse(window.localStorage.getItem(key) ?? "{}")).not.toHaveProperty(
        "home-research",
      );
    }
  });

  it("reads the old Home turns as stored and keeps their task cards in place", () => {
    const directTurns = [
      {
        id: "user-1",
        role: "user",
        createdAt: "2026-07-24T09:00:00Z",
        parts: [{ type: "text", text: "Research wines" }],
      },
      {
        id: "card-turn",
        role: "assistant",
        createdAt: "2026-07-24T09:00:01Z",
        parts: [{ type: "tool", id: "direct:1", name: "mcp_clovy_home_start_task", text: "" }],
      },
      "unreadable",
    ];
    window.localStorage.setItem(
      "clovy:home:direct-turns:v1",
      JSON.stringify({ "home-cards": directTurns }),
    );
    window.localStorage.setItem(
      "clovy:home:task-handoffs:v1",
      JSON.stringify({
        "home-cards": [
          {
            id: "home-task-direct:1",
            title: "Wine research",
            prompt: "Research wines.",
            status: "starting",
          },
        ],
      }),
    );
    const before = { ...window.localStorage };

    expect(readLegacyHomeDirectTurns("home-cards")).toEqual(directTurns);
    const handoffs = readHomeTaskHandoffs("home-cards");
    // Nothing starts a card anymore: one left starting was interrupted.
    expect(handoffs).toEqual([
      expect.objectContaining({ id: "home-task-direct:1", status: "failed" }),
    ]);
    const cards = homeTaskCardTurns(directTurns, handoffs);
    expect(cards.map((card) => [card.turn.id, card.handoff.title])).toEqual([
      ["card-turn", "Wine research"],
    ]);
    expect({ ...window.localStorage }).toEqual(before);
  });

  it("orders Home turns by time and keeps equal times in stored order", () => {
    const turn = (id: string, createdAt: string) => ({
      id,
      role: "user" as const,
      createdAt,
      status: "complete" as const,
      parts: [],
    });
    expect(
      [
        turn("b", "2026-07-24T09:00:01Z"),
        turn("a1", "2026-07-24T09:00:00Z"),
        turn("a2", "2026-07-24T09:00:00Z"),
      ]
        .sort(compareHomeTurnOrder)
        .map((value) => value.id),
    ).toEqual(["a1", "a2", "b"]);
  });

  it("hides the old Home context block in the transcript and previews", () => {
    const runtimePrompt = [
      "[Clovy home context]",
      "This is Clovy's persistent Home conversation with the user.",
      "[/Clovy home context]",
      "",
      "Help me plan tomorrow",
    ].join("\n");

    expect(stripClovyHomeContext(runtimePrompt)).toBe("Help me plan tomorrow");
    expect(stripClovyHomeContextFromPreview(runtimePrompt)).toBe("Help me plan tomorrow");
    const legacyPrompt = runtimePrompt
      .replace("[Clovy home context]", "[June home context]")
      .replace("[/Clovy home context]", "[/June home context]");
    expect(stripClovyHomeContext(legacyPrompt)).toBe("Help me plan tomorrow");
    expect(stripClovyHomeContextFromPreview("[June home context]\nThis is Ju")).toBe(
      "Home message",
    );
  });

  it("recognizes the old task card tool names", () => {
    expect(isClovyHomeStartTaskTool("start_task")).toBe(true);
    expect(isClovyHomeStartTaskTool("mcp_clovy_home_start_task")).toBe(true);
    expect(isClovyHomeStartTaskTool("clovy_home.start_task")).toBe(true);
    expect(isClovyHomeStartTaskTool("Mcp june home start task")).toBe(true);
    expect(isClovyHomeStartTaskTool("start_session")).toBe(false);
  });

  it("labels day boundaries relative to now and keys them by local day", () => {
    const now = new Date(2026, 6, 22, 14, 45);

    expect(clovyHomeDayLabel(new Date(2026, 6, 22, 9, 4).toISOString(), now)).toMatch(
      /^Today at 9:04/,
    );
    expect(clovyHomeDayLabel(new Date(2026, 6, 21, 16, 12).toISOString(), now)).toMatch(
      /^Yesterday at /,
    );
    // Two to six days back reads as the weekday; older dates spell the date.
    expect(clovyHomeDayLabel(new Date(2026, 6, 20, 8, 0).toISOString(), now)).toMatch(
      /^Monday at /,
    );
    expect(clovyHomeDayLabel(new Date(2026, 5, 12, 8, 0).toISOString(), now)).toMatch(
      /^June 12 at /,
    );
    expect(clovyHomeDayLabel(new Date(2025, 11, 31, 8, 0).toISOString(), now)).toMatch(/2025/);
    expect(clovyHomeDayLabel("not-a-date", now)).toBe("");

    expect(clovyHomeDayKey(new Date(2026, 6, 22, 0, 5).toISOString())).toBe(
      clovyHomeDayKey(new Date(2026, 6, 22, 23, 55).toISOString()),
    );
    expect(clovyHomeDayKey(new Date(2026, 6, 22, 23, 55).toISOString())).not.toBe(
      clovyHomeDayKey(new Date(2026, 6, 23, 0, 5).toISOString()),
    );
    expect(clovyHomeDayKey("not-a-date")).toBe("");
  });

  it("keeps one proactive check-in timestamp for the local day", () => {
    const morning = new Date(2026, 6, 21, 9, 30);
    const later = new Date(2026, 6, 21, 16, 0);
    const nextDay = new Date(2026, 6, 22, 9, 0);

    const first = clovyHomeDailyCheckIn("default", morning);
    const sameDay = clovyHomeDailyCheckIn("default", later);
    const following = clovyHomeDailyCheckIn("default", nextDay);

    expect(first.text).toContain("Good morning");
    expect(sameDay.createdAt).toBe(first.createdAt);
    expect(following.createdAt).not.toBe(first.createdAt);
  });

  it("adapts greetings and grounded conversation starters to the local day", () => {
    const morning = new Date(2026, 6, 21, 9);
    const afternoon = new Date(2026, 6, 21, 15);
    const evening = new Date(2026, 6, 21, 21);

    expect(clovyHomeGreetingParts(morning, { displayName: "  Alex Rivera  " }).salutation).toBe(
      "Good morning, Alex",
    );
    expect(clovyHomeGreetingParts(afternoon).salutation).toBe("Good afternoon");
    expect(clovyHomeGreetingParts(evening).salutation).toBe("Good evening");
    expect(clovyHomeGreetingParts(morning).question).toBe("What would you like help with today?");
    expect(clovyHomeGreetingParts(morning, { returning: true }).question).toBe(
      "What should we pick up today?",
    );
    expect(clovyHomeNudgePrompts(morning)).toEqual([
      "Plan my day",
      "Think through a decision",
      "Help me get something done",
    ]);
    expect(clovyHomeNudgePrompts(afternoon)).toEqual([
      "Plan the rest of my day",
      "Work through a blocker",
      "Help me prioritize",
    ]);
    expect(clovyHomeNudgePrompts(evening)).toEqual([
      "Review my day",
      "Plan tomorrow",
      "Think through a decision",
    ]);
  });
});
