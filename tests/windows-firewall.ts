import assert from "node:assert/strict";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { dirname, join } from "node:path";

// Register the canonical executable before it opens a listener. Windows keys
// application rules by path, so another temporary bundle needs its own rule.
export async function allowLabNetwork(programs: string[]): Promise<void> {
    if (process.platform !== "win32")
        return;
    const paths = new Set(programs.map(program => realpathSync(program)));
    for (const program of paths) {
        if (!program.toLowerCase().endsWith("\\python.exe"))
            continue;
        const config = join(dirname(dirname(program)), "pyvenv.cfg");
        if (existsSync(config)) {
            const executable = /^executable\s*=\s*(.+)$/m.exec(readFileSync(config, "utf8"))?.[1]?.trim();
            if (executable)
                paths.add(realpathSync(executable));
        }
    }
    assert(paths.size > 0 && [...paths].every(path => path.toLowerCase().endsWith(".exe")), "Expected executable paths");
    const script = `
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
[string[]]$programs = ConvertFrom-Json -InputObject $env:QBUTT_TEST_FIREWALL_PROGRAMS
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not ([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run the Windows integration lab from an elevated terminal to register its firewall rules before launch.'
}
foreach ($program in $programs) {
    $digest = [Security.Cryptography.SHA256]::Create()
    try { $hash = [BitConverter]::ToString($digest.ComputeHash([Text.Encoding]::UTF8.GetBytes($program.ToLowerInvariant()))).Replace('-', '') }
    finally { $digest.Dispose() }
    $name = 'qbutt-lab-' + $hash
    $rule = Get-NetFirewallRule -Name $name -PolicyStore PersistentStore -ErrorAction SilentlyContinue
    if (-not $rule) {
        try {
            New-NetFirewallRule -Name $name -DisplayName ('qbutt lab: ' + [IO.Path]::GetFileName($program)) -Group 'qbutt integration lab' -Program $program -Direction Inbound -Action Allow -Profile Any -Protocol Any -Enabled True | Out-Null
        }
        catch {
            # Parallel labs may both observe an absent rule. Accept only the
            # exact rule created by the other preflight; propagate every other
            # firewall failure before a fixture process can open a listener.
            $rule = Get-NetFirewallRule -Name $name -PolicyStore PersistentStore -ErrorAction SilentlyContinue
            if (-not $rule) { throw }
        }
    }
    elseif (($rule.Enabled -ne 'True') -or ($rule.Action -ne 'Allow') -or ($rule.Direction -ne 'Inbound')) {
        Set-NetFirewallRule -Name $name -Program $program -Direction Inbound -Action Allow -Profile Any -Protocol Any -Enabled True | Out-Null
    }
    # Dismissed prompts can leave automatic block rules, which override allow.
    # Remove only these cached prompt decisions for the exact registered file.
    Get-NetFirewallApplicationFilter -Program $program -PolicyStore PersistentStore | Get-NetFirewallRule |
        Where-Object { $_.Name -like '*Query User*' } | Remove-NetFirewallRule
    $active = Get-NetFirewallRule -Name $name -PolicyStore ActiveStore
    $filter = $active | Get-NetFirewallApplicationFilter
    if (($active.Enabled -ne 'True') -or ($active.Action -ne 'Allow') -or ($filter.Program -ine $program)) {
        throw ('Firewall registration did not take effect: ' + $program)
    }
}
`;
    const powershell = Bun.which("pwsh.exe");
    assert(powershell, "PowerShell 7 is required for Windows lab firewall registration");
    const child = Bun.spawn([powershell,
        "-NoProfile", "-NonInteractive", "-Command", script], {
        env: { ...process.env, QBUTT_TEST_FIREWALL_PROGRAMS: JSON.stringify([...paths]) },
        stdout: "pipe", stderr: "pipe", windowsHide: true,
    });
    const timeout = setTimeout(() => child.kill(), 60000);
    try {
        const [exit, output, error] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
        assert(exit === 0, `Firewall preflight failed (exit ${exit}); no fixture was launched. ${error || output}`);
    }
    finally { clearTimeout(timeout); }
}

if (import.meta.main)
    await allowLabNetwork(process.argv.slice(2));
