import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: vi.fn((path: string) => path),
  invoke,
}));

import { agentRuntimeBindings } from "../lib/tauri";

describe("agent runtime Tauri bindings", () => {
  beforeEach(() => invoke.mockReset());

  it("uses the Clovy-owned session and run commands", async () => {
    invoke.mockResolvedValue(undefined);

    await agentRuntimeBindings.listSessions();
    await agentRuntimeBindings.getSession("session-1");
    await agentRuntimeBindings.startRun({
      sessionId: "session-1",
      prompt: "Hello",
      model: "__clovy_cli_engine__:claude",
      safetyMode: "sandboxed",
      workspacePath: "/tmp/session-1",
      enabledSkillIds: ["notes"],
      attachments: ["/tmp/brief.pdf"],
    });
    await agentRuntimeBindings.cancelRun("run-1");

    expect(invoke.mock.calls).toEqual([
      ["list_agent_sessions"],
      ["get_agent_session", { sessionId: "session-1" }],
      [
        "start_agent_run",
        {
          request: {
            sessionId: "session-1",
            prompt: "Hello",
            model: "__clovy_cli_engine__:claude",
            safetyMode: "sandboxed",
            workspacePath: "/tmp/session-1",
            enabledSkillIds: ["notes"],
            attachments: ["/tmp/brief.pdf"],
          },
        },
      ],
      ["cancel_agent_run", { runId: "run-1" }],
    ]);
  });

  it("exposes skills and interruption approvals without native sidecar controls", () => {
    expect(agentRuntimeBindings.listSkills).toBeTypeOf("function");
    expect(agentRuntimeBindings.resolveInterruption).toBeTypeOf("function");
    expect(agentRuntimeBindings).not.toHaveProperty("compactSession");
    expect(agentRuntimeBindings).not.toHaveProperty("steerRun");
  });

  it("sends typed interruption resolutions", async () => {
    invoke.mockResolvedValue(undefined);

    await agentRuntimeBindings.resolveInterruption({
      runId: "run-1",
      interruptionId: "approval-1",
      resolution: { kind: "approval", choice: "once" },
    });

    expect(invoke).toHaveBeenCalledWith("resolve_agent_interruption", {
      request: {
        runId: "run-1",
        interruptionId: "approval-1",
        resolution: { kind: "approval", choice: "once" },
      },
    });
  });
});
