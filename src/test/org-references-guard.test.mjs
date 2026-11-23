import { describe, expect, it } from "vitest";
import { findOrgReferences, ORG_REFERENCE_SCOPES } from "../../scripts/check-org-references.mjs";

describe("organization reference guard", () => {
  it("flags each organization reference the guard exists for", () => {
    const [{ pattern }] = ORG_REFERENCE_SCOPES;
    for (const sample of [
      "https://app.opensoftware.co/api",
      "https://june.link/s/shr_1",
      "ghcr.io/open-software-network/clovy-api",
      "runs-on: blacksmith-4vcpu-ubuntu-2404",
    ]) {
      expect(sample).toMatch(pattern);
    }
    expect("co.opensoftware.june").not.toMatch(pattern);
  });

  it("flags the June-era app identity in app code", () => {
    const scope = ORG_REFERENCE_SCOPES.find(({ name }) => name === "app identity");
    for (const sample of [
      '"identifier": "co.opensoftware.june"',
      "Contents/MacOS/os-june",
      "/Applications/June.app",
    ]) {
      expect(sample).toMatch(scope.pattern);
    }
    expect('"identifier": "dev.guilhermexp.clovy"').not.toMatch(scope.pattern);
    expect(scope.pathspecs.filter((spec) => spec.startsWith(":!src-tauri"))).toEqual([
      ":!src-tauri/src/identity_migration",
      ":!src-tauri/src/agent_sessions/retirement.rs",
    ]);
  });

  it("still scans legacy secret retirement for service domains", () => {
    const scope = ORG_REFERENCE_SCOPES.find(({ name }) => name === "app source");
    expect(scope.pathspecs).not.toContain(":!src-tauri/src/agent_sessions/retirement.rs");
    expect("https://app.opensoftware.co/api").toMatch(scope.pattern);
  });

  it("finds no organization references in the repository, CI and app source", () => {
    expect(findOrgReferences()).toEqual([]);
  });
});
