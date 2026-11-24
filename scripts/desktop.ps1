param(
    [ValidateSet('dev', 'build', 'test', 'check')][string]$Action = 'dev',
    [string]$CmakeBin = ''
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if ($CmakeBin) {
    if (-not (Test-Path (Join-Path $CmakeBin 'cmake.exe'))) { throw 'CmakeBin must contain cmake.exe' }
    $env:PATH = "$CmakeBin;$env:PATH"
} elseif (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    $cmakeCandidate = Get-ChildItem 'C:\Espressif\tools\cmake' -Filter cmake.exe -Recurse -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | Select-Object -First 1
    if ($cmakeCandidate) { $env:PATH = "$($cmakeCandidate.DirectoryName);$env:PATH" }
}
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) { throw 'Install CMake or supply -CmakeBin.' }
$env:CMAKE_POLICY_VERSION_MINIMUM = '3.5'
function Invoke-Checked([scriptblock]$Command) {
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "Command failed with exit code $LASTEXITCODE" }
}
switch ($Action) {
    'dev' { Invoke-Checked { npm run tauri -- dev } }
    'build' { Invoke-Checked { npm run tauri -- build } }
    'test' {
        Invoke-Checked { npm run build }
        Invoke-Checked { npm test }
        Invoke-Checked { cargo test --manifest-path src-tauri/Cargo.toml }
    }
    'check' {
        Invoke-Checked { npm run build }
        Invoke-Checked { cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings }
    }
}
