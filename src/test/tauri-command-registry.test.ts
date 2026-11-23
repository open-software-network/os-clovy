import { readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

// Every command production code invokes must be registered in the Rust
// `generate_handler!` list; otherwise the call fails at runtime with
// "Command <name> not found", which component tests that mock `lib/tauri`
// never see.

const root = process.cwd();

function productionSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === "test" ? [] : productionSources(path);
    return /\.tsx?$/.test(entry.name) ? [path] : [];
  });
}

function invokedCommands(): Map<string, string[]> {
  const commands = new Map<string, string[]>();
  for (const file of productionSources(resolve(root, "src"))) {
    const source = readFileSync(file, "utf8");
    for (const match of source.matchAll(/\binvoke\s*(?:<[^()]*?>)?\(\s*["']([^"']+)["']/g)) {
      const files = commands.get(match[1]) ?? [];
      files.push(relative(root, file));
      commands.set(match[1], files);
    }
  }
  return commands;
}

function registeredCommands(): Set<string> {
  const lib = readFileSync(resolve(root, "src-tauri/src/lib.rs"), "utf8");
  const start = lib.indexOf("generate_handler![");
  const end = lib.indexOf("])", start);
  expect(start).toBeGreaterThan(-1);
  return new Set(
    lib
      .slice(start + "generate_handler![".length, end)
      .split(",")
      .map((entry) => entry.trim())
      .filter((entry) => entry && !entry.startsWith("//"))
      .map((entry) => entry.split("::").pop() as string),
  );
}

describe("Tauri command registry", () => {
  it("registers every command production code invokes", () => {
    const registered = registeredCommands();
    const invoked = invokedCommands();
    expect(invoked.size).toBeGreaterThan(100);
    const missing = [...invoked]
      .filter(([command]) => !registered.has(command))
      .map(([command, files]) => `${command} (${[...new Set(files)].join(", ")})`);
    expect(missing).toEqual([]);
  });
});
