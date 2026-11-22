# Builds omenless, installs it to Program Files (admin-only, because it runs elevated at logon)
# and restarts it. Run from the repo root; it asks for elevation itself.
param([switch]$Elevated)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$exe = Join-Path $repo 'target\release\omenless.exe'

if (-not $Elevated) {
    Push-Location $repo
    try { cargo build --release } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw 'build failed' }
    Start-Process pwsh -Verb RunAs -Wait -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath, '-Elevated'
    return
}

$dest = Join-Path $env:ProgramFiles 'omenless'
Stop-Process -Name omenless -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item $exe (Join-Path $dest 'omenless.exe') -Force
Start-Process (Join-Path $dest 'omenless.exe')
