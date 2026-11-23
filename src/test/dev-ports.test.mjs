import { describe, expect, it, vi } from "vitest";
import { chooseDevPort } from "../../scripts/dev-ports.mjs";

describe("frontend development ports", () => {
  it("moves Vite off a port owned by another worktree", async () => {
    const portIsFree = vi.fn(async (port) => port === 1423);

    await expect(
      chooseDevPort({ name: "Vite", explicitValue: undefined, base: 1421, portIsFree }),
    ).resolves.toBe(1423);
    expect(portIsFree.mock.calls).toEqual([[1421], [1422], [1423]]);
  });

  it("fails instead of reusing an occupied explicit port", async () => {
    await expect(
      chooseDevPort({
        name: "Vite",
        explicitValue: "1421",
        base: 1421,
        portIsFree: async () => false,
      }),
    ).rejects.toThrow("Vite port 1421 is already in use");
  });
});
