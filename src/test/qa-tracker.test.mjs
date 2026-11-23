import { existsSync, readFileSync, statSync } from "node:fs";
import { beforeAll, describe, expect, it } from "vitest";

const TRACKER_PATH = "docs/qa/feature-user-stories.tsv";
const REQUIRED_COLUMNS = [
  "Story ID",
  "Area",
  "Feature",
  "User story",
  "Expected behavior",
  "Code refs",
  "Automated coverage",
  "Test status",
  "Defect status",
  "Priority",
  "Notes",
];

function splitRefs(value) {
  return value
    .split(";")
    .map((part) => part.trim())
    .filter(Boolean);
}

function isCommandRef(ref) {
  return /^pnpm (?:exec |run )?[A-Za-z0-9:_./-]+$/.test(ref);
}

describe("feature user-story tracker", () => {
  let lines;
  let header;
  let rows;
  let columnIndex;

  beforeAll(() => {
    lines = readFileSync(TRACKER_PATH, "utf8").trimEnd().split("\n");
    header = lines[0].split("\t");
    rows = lines.slice(1).map((line) => line.split("\t"));
    columnIndex = Object.fromEntries(header.map((column, index) => [column, index]));
  });

  it("keeps the canonical TSV schema intact", () => {
    expect(header).toEqual(REQUIRED_COLUMNS);
    expect(rows.length).toBeGreaterThan(0);

    for (const row of rows) {
      expect(row).toHaveLength(header.length);
    }
  });

  it("uses unique, well-formed story ids", () => {
    const ids = rows.map((row) => row[columnIndex["Story ID"]]);

    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) {
      expect(id).toMatch(/^JUN-\d{3}$/);
    }
  });

  it("keeps code and coverage references resolvable", () => {
    for (const row of rows) {
      const id = row[columnIndex["Story ID"]];
      const refs = [
        ...splitRefs(row[columnIndex["Code refs"]]),
        ...splitRefs(row[columnIndex["Automated coverage"]]),
      ];

      for (const ref of refs) {
        if (isCommandRef(ref)) continue;
        expect(existsSync(ref), `${id} missing ref: ${ref}`).toBe(true);
        const stat = statSync(ref);
        expect(stat.isFile() || stat.isDirectory(), `${id} non-file-or-directory ref: ${ref}`).toBe(
          true,
        );
      }
    }
  });

  it("does not carry stale single-instance blocker language", () => {
    const content = lines.join("\n");

    expect(content).not.toContain("local dev app could not own the single-instance identity");
    expect(content).not.toContain("already running with the same single-instance identifier");
  });
});
