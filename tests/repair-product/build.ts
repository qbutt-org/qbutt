import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const [buildArgument, outputArgument, environmentArgument] = process.argv.slice(2);
assert(buildArgument && outputArgument && environmentArgument,
    "Pass native build directory, output directory and MSVC environment .cmd");
const build = resolve(buildArgument);
const output = resolve(outputArgument);
await mkdir(output, { recursive: true });
const commands = JSON.parse(await readFile(join(build, "compile_commands.json"), "utf8")) as { file: string; command: string }[];
const entry = commands.find(item => item.file.replaceAll("\\", "/").endsWith("/repairpreviewdialog.cpp"));
assert(entry, "Configure the current native GUI implementation first");
const source = resolve(import.meta.dir, "gui-smoke.cpp");
const object = join(output, "gui-smoke.obj");
const command = entry.command.replace(/ -c .+$/, ` -c "${source}"`)
    .replace(/\/Fo(?:"[^"]+"|\S+)/, `/Fo"${object}"`)
    .replace(/\/Fd(?:"[^"]+"|\S+)/, `/Fd"${join(output, "gui-smoke.pdb")}"`);
assert(command.includes(` -c "${source}"`), "Cannot replace the compiler input safely");
const ninja = await readFile(join(build, "build.ninja"), "utf8");
const appLink = ninja.slice(ninja.indexOf("build qbutt.exe:"));
const libraries = /^  LINK_LIBRARIES = (.+)$/m.exec(appLink)?.[1];
assert(libraries, "Cannot find application linker dependencies");
const executable = join(output, "gui-smoke.exe");
const batch = ["@echo off", `call "${resolve(environmentArgument)}" >nul`, "if errorlevel 1 exit /b %errorlevel%", command,
    "if errorlevel 1 exit /b %errorlevel%", `link /nologo /OUT:"${executable}" /SUBSYSTEM:CONSOLE /OPT:REF "${object}" ${libraries.replaceAll("$:", ":").replaceAll("$ ", " ")}`,
    "exit /b %errorlevel%", ""].join("\r\n");
const batchPath = join(output, "build-gui-smoke.cmd");
await writeFile(batchPath, batch);
const child = Bun.spawn(["cmd.exe", "/d", "/c", batchPath], { cwd: build, stdout: "pipe", stderr: "pipe" });
const [code, stdout, stderr] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
await writeFile(join(output, "build-gui-smoke.log"), stdout + stderr);
assert.equal(code, 0, `Repair preview driver failed to build; see ${join(output, "build-gui-smoke.log")}`);
console.log(JSON.stringify({ executable }));
