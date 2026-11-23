import { describe, expect, it, vi } from "vitest";
import {
  DevSigningIdentityError,
  resolveDevSigningIdentity,
  signatureSatisfies,
} from "../../scripts/dev-signing-identity.mjs";

const DEVELOPMENT = "1111111111111111111111111111111111111111";
const SECOND_DEVELOPMENT = "2222222222222222222222222222222222222222";
const LOCAL = "3333333333333333333333333333333333333333";

function keychain(...identities) {
  const lines = identities.map(([hash, name], index) => `  ${index + 1}) ${hash} "${name}"`);
  const stdout = `${lines.join("\n")}\n     ${identities.length} valid identities found\n`;
  return vi.fn(() => ({ status: 0, stdout, stderr: "" }));
}

describe("development signing identity", () => {
  it("reuses the same signature for an exact name or its hash and rejects another certificate", () => {
    const selectedName = "Apple Development: Fixture Person (TEAM000001)";
    const run = (command, args) => {
      if (command === "/usr/bin/security")
        return {
          status: 0,
          stdout: `  1) ${DEVELOPMENT} "${selectedName}"\n  2) ${SECOND_DEVELOPMENT} "Other"\n`,
        };
      if (command === "/usr/bin/codesign")
        return {
          status: args.includes(`-R=certificate leaf = H"${DEVELOPMENT}"`) ? 0 : 1,
        };
      throw new Error("unexpected command");
    };
    for (const configured of [selectedName, DEVELOPMENT.toLowerCase()]) {
      const identity = resolveDevSigningIdentity({
        env: { CLOVY_DEV_SIGNING_IDENTITY: configured },
        run,
      }).identity;
      expect(signatureSatisfies("fixture", identity, run)).toBe(true);
    }
    const other = resolveDevSigningIdentity({
      env: { CLOVY_DEV_SIGNING_IDENTITY: SECOND_DEVELOPMENT },
      run,
    }).identity;
    expect(signatureSatisfies("fixture", other, run)).toBe(false);
  });
  it("discovers the single valid Apple Development identity", () => {
    const run = keychain(
      [LOCAL, "Local Dev Signing"],
      [DEVELOPMENT, "Apple Development: Fixture Person (TEAM000001)"],
    );

    expect(resolveDevSigningIdentity({ env: {}, run })).toEqual({
      identity: DEVELOPMENT,
      source: "discovered",
    });
    expect(run).toHaveBeenCalledWith(
      "/usr/bin/security",
      ["find-identity", "-v", "-p", "codesigning"],
      { encoding: "utf8" },
    );
  });

  it("refuses to guess between several Apple Development identities", () => {
    const run = keychain(
      [DEVELOPMENT, "Apple Development: Fixture Person (TEAM000001)"],
      [SECOND_DEVELOPMENT, "Apple Development: Fixture Person (TEAM000002)"],
    );

    expect(() => resolveDevSigningIdentity({ env: {}, run })).toThrow(
      /2 valid Apple Development identities were found\. Set CLOVY_DEV_SIGNING_IDENTITY/,
    );
  });

  it("fails with a concrete next step when no Apple Development identity is valid", () => {
    const run = keychain([LOCAL, "Local Dev Signing"]);

    expect(() => resolveDevSigningIdentity({ env: {}, run })).toThrow(DevSigningIdentityError);
    expect(() => resolveDevSigningIdentity({ env: {}, run })).toThrow(
      /No valid Apple Development code signing identity was found/,
    );
  });

  it("uses a configured identity only when it names one valid identity", () => {
    const run = keychain(
      [LOCAL, "Local Dev Signing"],
      [DEVELOPMENT, "Apple Development: Fixture Person (TEAM000001)"],
      [SECOND_DEVELOPMENT, "Apple Development: Fixture Person (TEAM000002)"],
    );

    expect(
      resolveDevSigningIdentity({
        env: { CLOVY_DEV_SIGNING_IDENTITY: SECOND_DEVELOPMENT.toLowerCase() },
        run,
      }),
    ).toEqual({ identity: SECOND_DEVELOPMENT, source: "configured" });
    expect(
      resolveDevSigningIdentity({ env: { CLOVY_DEV_SIGNING_IDENTITY: "Local Dev Signing" }, run }),
    ).toEqual({ identity: LOCAL, source: "configured" });
    expect(() =>
      resolveDevSigningIdentity({
        env: { CLOVY_DEV_SIGNING_IDENTITY: "4444444444444444444444444444444444444444" },
        run,
      }),
    ).toThrow(/must name exactly one valid code signing identity; it matched 0/);
  });

  it("signs ad hoc only when the developer opts in explicitly", () => {
    const run = vi.fn();

    expect(resolveDevSigningIdentity({ env: { CLOVY_DEV_SIGNING_IDENTITY: "-" }, run })).toEqual({
      identity: "-",
      source: "configured",
    });
    expect(run).not.toHaveBeenCalled();
  });

  it("reports a keychain listing failure instead of falling back", () => {
    const run = vi.fn(() => ({ status: 1, stdout: "", stderr: "security: keychain locked" }));

    expect(() => resolveDevSigningIdentity({ env: {}, run })).toThrow(
      "Could not list code signing identities: security: keychain locked",
    );
  });
});
