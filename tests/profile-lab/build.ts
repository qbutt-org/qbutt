import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

// Reuse the exact configured application's compiler and linker inputs.
// This builds standalone integration drivers, without enabling upstream tests.
const [buildArgument, outputArgument, environmentArgument, driver = "service"] = process.argv.slice(2);
assert(buildArgument && outputArgument && environmentArgument, "Pass native build directory, output directory and MSVC environment .cmd");
assert(driver === "service" || driver === "gui-smoke", "Unknown integration driver");
const build = resolve(buildArgument);
const output = resolve(outputArgument);
await mkdir(output, { recursive: true });
const commands = JSON.parse(await readFile(join(build, "compile_commands.json"), "utf8")) as { file: string; command: string }[];
const sourceName = driver === "service" ? "profileimport.cpp" : "profileimportdialog.cpp";
const entry = commands.find(item => item.file.replaceAll("\\", "/").endsWith(`/${sourceName}`));
assert(entry, "Configure the current native profile implementation first");
const source = resolve(import.meta.dir, `${driver}.cpp`);
const object = join(output, `${driver}.obj`);
const command = entry.command.replace(/ -c .+$/, ` -c "${source}"`)
    .replace(/\/Fo\S+/, `/Fo"${object}"`).replace(/\/Fd\S+/, `/Fd"${join(output, `${driver}.pdb`)}"`);
assert(command.includes(` -c "${source}"`), "Cannot replace the compiler input safely");
const ninja = await readFile(join(build, "build.ninja"), "utf8");
const appLink = ninja.slice(ninja.indexOf("build qbutt.exe:"));
const libraries = /^  LINK_LIBRARIES = (.+)$/m.exec(appLink)?.[1];
assert(libraries, "Cannot find application linker dependencies");
const executable = join(output, `${driver}.exe`);
const batch = ["@echo off", `call "${resolve(environmentArgument)}" >nul`, "if errorlevel 1 exit /b %errorlevel%", command,
    "if errorlevel 1 exit /b %errorlevel%", `link /nologo /OUT:"${executable}" /SUBSYSTEM:CONSOLE /OPT:REF "${object}" ${libraries.replaceAll("$:", ":").replaceAll("$ ", " ")}`,
    "exit /b %errorlevel%", ""].join("\r\n");
const batchPath = join(output, `build-${driver}.cmd`);
await writeFile(batchPath, batch);
const child = Bun.spawn(["cmd.exe", "/d", "/c", batchPath], { cwd: build, stdout: "pipe", stderr: "pipe" });
const [code, stdout, stderr] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
await writeFile(join(output, `build-${driver}.log`), stdout + stderr);
assert.equal(code, 0, `Native integration driver failed; see ${join(output, `build-${driver}.log`)}`);
console.log(JSON.stringify({ executable }));
