import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("Clovy repository branding", () => {
  it("uses the canonical source repository in current public surfaces", () => {
    const currentSurfaces = [
      "CONTEXT.md",
      "README.md",
      "CONTRIBUTING.md",
      ".github/ISSUE_TEMPLATE/bug_report.yml",
      ".github/ISSUE_TEMPLATE/config.yml",
      ".github/ISSUE_TEMPLATE/feature_request.yml",
      "docs/private-connectors-implementation-plan.md",
      "docs/plugins/browser-use-prd.md",
      "docs/telemetry-p3a-implementation-plan.md",
      "scripts/check-cargo-release-age.py",
      "scripts/check-pnpm-release-age.py",
    ].map(read);

    for (const source of currentSurfaces) {
      expect(source).not.toMatch(/open-software-network\/os-june(?!-releases)/);
    }
    expect(read("docs/private-connectors-implementation-plan.md")).toContain(
      "**Repos:** `os-clovy`",
    );
    expect(read("docs/telemetry-p3a-implementation-plan.md")).toContain("desktop (os-clovy)");
    expect(read("README.md")).toContain("git clone https://github.com/guilhermexp/os-clovy");
    expect(read("README.md")).not.toContain("clovy-demo.gif");
    expect(read("README.md")).not.toContain("https://june-api.opensoftware.co/verify");
  });
});
