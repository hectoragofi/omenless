# Builds omenless, installs it to Program Files (admin-only, because it runs elevated at logon)
# and restarts it. Run from the repo root; it asks for elevation itself.
# -Test <file>: run the hardware self-test with the new build first and write the report there.
param([switch]$Elevated, [string]$Test)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$exe = Join-Path $repo 'target\release\omenless.exe'

if (-not $Elevated) {
    Push-Location $repo
    try { cargo build --release } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw 'build failed' }
    $argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath, '-Elevated')
    if ($Test) { $argv += @('-Test', [IO.Path]::GetFullPath($Test)) }
    # -Wait would also wait for the tray app it launches, so wait on the elevated shell only.
    $p = Start-Process pwsh -Verb RunAs -PassThru -WindowStyle Hidden -ArgumentList $argv
    $p.WaitForExit()
    return
}

$dest = Join-Path $env:ProgramFiles 'omenless'
$target = Join-Path $dest 'omenless.exe'
Stop-Process -Name omenless -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item $exe $target -Force
if ($Test) { (Start-Process $target -ArgumentList '--test', $Test -PassThru).WaitForExit() }
Start-Process $target
