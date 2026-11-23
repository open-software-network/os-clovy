import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { runWindowsDevCargo } from "../../scripts/cargo-windows-dev-runner.mjs";

const sandboxes = [];
afterEach(() => {
  for (const dir of sandboxes.splice(0)) rmSync(dir, { recursive: true, force: true });
});
const digest = (file) => createHash("sha256").update(readFileSync(file)).digest("hex");
function fixture() {
  const cwd = mkdtempSync(path.join(tmpdir(), "clovy-dev-signing-"));
  sandboxes.push(cwd);
  const out = path.join(cwd, "custom target ' ; literal");
  mkdirSync(out);
  const report = path.join(cwd, "report.json");
  const ready = path.join(cwd, "ready");
  const compiler = path.join(cwd, "compiler.cjs");
  // Executable fixtures reject bytes without a matching FS seal before doing
  // app/relay work. This proves ordering/rebuild/failure behavior, not Authenticode.
  const executable = (generation, host) => `
const fs=require('node:fs'),crypto=require('node:crypto'),cp=require('node:child_process');
const actual=crypto.createHash('sha256').update(fs.readFileSync(__filename)).digest('hex');
if(!fs.existsSync(__filename+'.seal')||fs.readFileSync(__filename+'.seal','utf8')!==actual)process.exit(71);
${
  host
    ? `const relay=cp.execFileSync(process.execPath,[${JSON.stringify(path.join(out, "clovy-mcp.cjs"))}],{input:'{"method":"initialize"}',encoding:'utf8'});
fs.writeFileSync(${JSON.stringify(report)},JSON.stringify({generation:${generation},args:process.argv.slice(2),identity:process.env.CLOVY_DEV_APP_IDENTIFIER,relay:JSON.parse(relay)}));
if(process.argv.includes('--wait')){fs.writeFileSync(${JSON.stringify(ready)},'ready');process.on('SIGTERM',()=>process.exit(23));setInterval(()=>{},1000);}else process.exit(17);`
    : `let input='';process.stdin.on('data',d=>input+=d);process.stdin.on('end',()=>{if(JSON.parse(input).method!=='initialize')process.exit(72);process.stdout.write(JSON.stringify({ready:true}));});`
}
`;
  const compile = (generation) =>
    writeFileSync(
      compiler,
      `
const fs=require('node:fs');
if(process.env.FAIL_COMPILE)process.exit(31);
${["clovy", "clovy-mcp"]
  .map((name) => {
    const file = path.join(out, `${name}.cjs`);
    return `fs.writeFileSync(${JSON.stringify(file)},${JSON.stringify(executable(generation, name === "clovy"))});console.log(JSON.stringify({reason:'compiler-artifact',target:{name:${JSON.stringify(name)},kind:['bin']},executable:${JSON.stringify(file)}}));`;
  })
  .join("\n")}
`,
    );
  const env = {
    ...process.env,
    WINDOWS_CERTIFICATE_THUMBPRINT: "fixture-publisher",
    CLOVY_DEV_APP_IDENTIFIER: "dev.guilhermexp.clovy.runnerproof",
  };
  const options = {
    cwd,
    env,
    spawnProcess: (command, args, options) =>
      spawn(
        process.execPath,
        command === "cargo" ? [compiler, ...args] : [command, ...args],
        options,
      ),
    sign: async (file) => writeFileSync(`${file}.seal`, digest(file)),
  };
  return { compile, options, report, ready };
}
async function waitFor(file) {
  const end = Date.now() + 5000;
  while (!existsSync(file)) {
    if (Date.now() > end) throw new Error(`Fixture did not reach ${file}`);
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

describe("Windows development compile/sign/launch lifecycle", () => {
  it("passes metadata through to real Cargo without requiring signing or compiling the app", async () => {
    const root = mkdtempSync(path.join(tmpdir(), "clovy-cargo-metadata-"));
    sandboxes.push(root);
    mkdirSync(path.join(root, "src"));
    writeFileSync(
      path.join(root, "Cargo.toml"),
      '[package]\nname="runner-metadata-probe"\nversion="0.1.0"\nedition="2021"\n',
    );
    writeFileSync(
      path.join(root, "Cargo.lock"),
      'version=4\n[[package]]\nname="runner-metadata-probe"\nversion="0.1.0"\n',
    );
    writeFileSync(path.join(root, "src/main.rs"), "fn main() {}\n");
    let output = "";
    const env = { ...process.env };
    delete env.WINDOWS_CERTIFICATE_THUMBPRINT;
    const code = await runWindowsDevCargo(
      ["metadata", "--offline", "--locked", "--no-deps", "--format-version=1"],
      {
        cwd: root,
        env,
        spawnProcess: (command, args, options) => {
          const child = spawn(command, args, { ...options, stdio: ["ignore", "pipe", "pipe"] });
          child.stdout.on("data", (data) => {
            output += data;
          });
          return child;
        },
      },
    );
    expect(code).toBe(0);
    const metadata = JSON.parse(output);
    expect(metadata.packages[0].name).toBe("runner-metadata-probe");
    expect(metadata.packages[0].targets[0].src_path).toBe(path.join(root, "src/main.rs"));
  });
  it("signs each fresh Cargo-reported generation before app/relay work and retains identity, literal arguments and exit status", async () => {
    const f = fixture();
    for (const generation of [1, 2]) {
      f.compile(generation);
      const args = ["literal spaces ' ; $(not-code)", "--replay-onboarding"];
      expect(
        await runWindowsDevCargo(
          ["run", "--target", "custom-target", "--profile", "custom", "--", ...args],
          f.options,
        ),
      ).toBe(17);
      expect(JSON.parse(readFileSync(f.report, "utf8"))).toEqual({
        generation,
        args,
        identity: f.options.env.CLOVY_DEV_APP_IDENTIFIER,
        relay: { ready: true },
      });
    }
  });
  it("does not sign or launch after compilation fails", async () => {
    const f = fixture();
    f.compile(1);
    f.options.env.FAIL_COMPILE = "1";
    f.options.sign = async () => {
      throw new Error("Signing must not happen");
    };
    expect(await runWindowsDevCargo(["run"], f.options)).toBe(31);
    expect(existsSync(f.report)).toBe(false);
  });
  it("fails closed before launch if signing fails", async () => {
    const f = fixture();
    f.compile(1);
    f.options.sign = async () => {
      throw new Error("Publisher signing failed");
    };
    await expect(runWindowsDevCargo(["run"], f.options)).rejects.toThrow(
      "Publisher signing failed",
    );
    expect(existsSync(f.report)).toBe(false);
  });
  it("requires a certificate before invoking the compiler", async () => {
    const f = fixture();
    delete f.options.env.WINDOWS_CERTIFICATE_THUMBPRINT;
    await expect(runWindowsDevCargo(["run"], f.options)).rejects.toThrow(
      "WINDOWS_CERTIFICATE_THUMBPRINT",
    );
    expect(existsSync(f.report)).toBe(false);
  });
  it("forwards termination to the active app and preserves its handled exit code", async () => {
    const f = fixture();
    f.compile(1);
    const result = runWindowsDevCargo(["run", "--", "--wait"], f.options);
    await waitFor(f.ready);
    process.emit("SIGTERM");
    expect(await result).toBe(23);
  });
});
