import { describe, expect, it } from "vitest";
import { computerUseBundleIdentifier } from "../../scripts/computer-use-dev.mjs";

describe("Computer use development identity", () => {
  const baseIdentifier = "dev.guilhermexp.clovy.computer-use-driver";

  it("is stable within a worktree and isolated between worktrees", () => {
    const first = computerUseBundleIdentifier({
      baseIdentifier,
      profile: "debug",
      worktreeRoot: "/tmp/clovy-worktree-a",
    });
    const same = computerUseBundleIdentifier({
      baseIdentifier,
      profile: "debug",
      worktreeRoot: "/tmp/clovy-worktree-a",
    });
    const second = computerUseBundleIdentifier({
      baseIdentifier,
      profile: "debug",
      worktreeRoot: "/tmp/clovy-worktree-b",
    });

    expect(first).toBe(same);
    expect(first).not.toBe(second);
    expect(first).toMatch(/^dev\.guilhermexp\.clovy\.computer-use-driver\.dev\.w[0-9a-f]{12}$/);
  });

  it("keeps the production bundle identifier fixed", () => {
    expect(
      computerUseBundleIdentifier({
        baseIdentifier,
        profile: "release",
        worktreeRoot: "/tmp/any-worktree",
      }),
    ).toBe(baseIdentifier);
  });
});
