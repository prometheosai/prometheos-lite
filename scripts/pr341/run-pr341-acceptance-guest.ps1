#requires -Version 5.1
# PR341 Stage 1 guest entry point. Fail-closed three-stage launcher:
#   1. strict native security diagnostic
#   2. bounded preflight (stale process/protected-state/evidence preservation)
#   3. six-test privileged acceptance
# Any nonzero stage exit stops the sequence and becomes the operator-visible exit.
# Identity is pinned by host-produced attestation, not by caller parameters.

param([string]$PackageRoot = $PSScriptRoot)

$ErrorActionPreference = 'Stop'
$stage = (Resolve-Path -LiteralPath $PackageRoot).Path
$log = $null
$evidenceDir = $null

function Write-Result([string]$Status, [string]$Check, [string]$Detail) {
  $line = "$(Get-Date -Format o) [$Status] $Check :: $Detail"
  Write-Output $line
  if ($null -ne $log) { Add-Content -Path $log -Value $line }
}
function Refuse([string]$Reason) {
  Write-Result 'BLOCKED' 'preflight' $Reason
  Write-Output 'TERMINAL: BLOCKED - PR341 privileged acceptance did not run'
  exit 2
}
function Invoke-Stage([string]$Label, [string]$Path, [string[]]$Arguments) {
  Write-Result 'INFO' "$Label-start" $Path
  if ($Label -eq 'diagnostic') {
    $env:PR402_STRICT_DIAGNOSTIC = '1'
    & $Path @Arguments
  } else {
    Remove-Item Env:PR402_STRICT_DIAGNOSTIC -ErrorAction SilentlyContinue
  }
  $code = $LASTEXITCODE
  Write-Result $(if ($code -eq 0) { 'PASS' } else { 'FAIL' }) "$Label-exit" "exit=$code"
  return $code
}

if ($env:OS -notmatch 'Windows_NT') { Refuse 'non-Windows environment' }
if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') { Refuse "unsupported architecture $($env:PROCESSOR_ARCHITECTURE)" }

$manifestPath = Join-Path $stage 'manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath)) { Refuse 'manifest.json missing' }
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
foreach ($entry in $manifest.files) {
  $path = Join-Path $stage $entry.name
  if (-not (Test-Path -LiteralPath $path)) { Refuse "package artifact missing: $($entry.name)" }
  $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne $entry.sha256.ToLowerInvariant()) { Refuse "hash mismatch for $($entry.name)" }
}
Write-Result 'PASS' 'package-integrity' "source_sha=$($manifest.source_sha) files=$($manifest.files.Count)"

$att = Get-Content -LiteralPath (Join-Path $stage 'host-checkpoint-attestation.json') -Raw | ConvertFrom-Json
$prefix = [string]$att.sid_prefix
if ($prefix -notmatch '^S-1-5-21-\d+-\d+-\d+$') { Refuse "invalid pinned SID prefix syntax: $prefix" }
$expectedName = [string]$att.machine_name
if ([string]::IsNullOrWhiteSpace($expectedName)) { Refuse 'pinned machine name missing' }
if ($att.vm_id -ne '2c3f151d-5974-4c7b-ae89-7d233dc2c614') { Refuse 'host attestation VM identity mismatch' }
if ($att.checkpoint -ne 'PR341-Windows11-Clean-Install') { Refuse 'host attestation checkpoint mismatch' }
$cs = Get-CimInstance Win32_ComputerSystem
if (-not $cs.Name.Equals($expectedName, [StringComparison]::OrdinalIgnoreCase)) {
  Refuse "machine name $($cs.Name) != pinned $expectedName"
}
$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$actualSid = [string]$id.User.Value
if (-not $actualSid.StartsWith($prefix + '-', [StringComparison]::OrdinalIgnoreCase)) {
  Refuse "account SID $actualSid is not under pinned prefix $prefix"
}
$rid = $actualSid.Substring($prefix.Length + 1)
if ($rid -notmatch '^\d+$') { Refuse "account SID $actualSid has an invalid RID" }
Write-Result 'PASS' 'guest-identity' "machine=$($cs.Name) sid=$actualSid prefix=$prefix"

$src = @'
using System;
using System.Runtime.InteropServices;
public static class Pr341Elevation {
  [StructLayout(LayoutKind.Sequential)] public struct TOKEN_ELEVATION { public uint TokenIsElevated; }
  [DllImport("advapi32.dll", SetLastError=true)] public static extern bool OpenProcessToken(IntPtr p, uint a, out IntPtr h);
  [DllImport("advapi32.dll", SetLastError=true)] public static extern bool GetTokenInformation(IntPtr h, int c, out TOKEN_ELEVATION i, int s, out int r);
  [DllImport("kernel32.dll")] public static extern IntPtr GetCurrentProcess();
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
}
'@
Add-Type -TypeDefinition $src -ErrorAction Stop
$token = [IntPtr]::Zero
if (-not [Pr341Elevation]::OpenProcessToken([Pr341Elevation]::GetCurrentProcess(), 8, [ref]$token)) { Refuse 'OpenProcessToken failed' }
try {
  $elev = New-Object Pr341Elevation+TOKEN_ELEVATION
  $returned = 0
  if (-not [Pr341Elevation]::GetTokenInformation($token, 20, [ref]$elev, 4, [ref]$returned) -or $returned -ne 4) { Refuse 'TokenElevation query failed' }
  if ($elev.TokenIsElevated -ne 1) { Refuse "TokenElevation=$($elev.TokenIsElevated) is not elevated" }
} finally { [void][Pr341Elevation]::CloseHandle($token) }
$principal = [Security.Principal.WindowsPrincipal]::new($id)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { Refuse 'active Administrators authority missing' }
Write-Result 'PASS' 'token-elevation' 'TokenElevation=1; Administrator=True'

$common = [Environment]::GetFolderPath('CommonApplicationData')
if ($common -ne [string]$att.guest_programdata) { Refuse "guest ProgramData $common != pinned $($att.guest_programdata)" }
$registryPath = Join-Path $common 'PrometheOS\Lite\owner-registry\registry.json'
if (Test-Path -LiteralPath $registryPath) { Refuse 'pre-existing owner registry is not test-owned' }
Write-Result 'PASS' 'guest-programdata' "root=$common target=$registryPath"

$evidenceDir = Join-Path $common 'PrometheOS-Lite-PR341-Evidence'
New-Item -ItemType Directory -Path $evidenceDir -Force | Out-Null
$log = Join-Path $evidenceDir 'acceptance.log'
Write-Result 'PASS' 'evidence-dir' $evidenceDir

# Stage 1: strict diagnostic. A failure stops the sequence immediately.
$diagnostic = Join-Path $stage 'pr402_security_diagnostic.exe'
$diagnosticLog = Join-Path $evidenceDir 'diagnostic.log'
$env:PR402_STRICT_DIAGNOSTIC = '1'
& $diagnostic --nocapture --test-threads 1 2>&1 | Tee-Object -FilePath $diagnosticLog
$diagnosticExit = $LASTEXITCODE
Remove-Item Env:PR402_STRICT_DIAGNOSTIC -ErrorAction SilentlyContinue
Write-Result $(if ($diagnosticExit -eq 0) { 'PASS' } else { 'FAIL' }) 'strict-diagnostic' "exit=$diagnosticExit log=$diagnosticLog"
if ($diagnosticExit -ne 0) {
  Write-Output "TERMINAL: FAIL - strict diagnostic exit=$diagnosticExit; preflight and acceptance NOT RUN"
  exit $diagnosticExit
}

# Stage 2: bounded preflight. Failure stops before privileged acceptance.
$preflight = Join-Path $stage 'preflight-v5.ps1'
$preflightLog = Join-Path $evidenceDir 'preflight.log'
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $preflight 2>&1 | Tee-Object -FilePath $preflightLog
$preflightExit = $LASTEXITCODE
Write-Result $(if ($preflightExit -eq 0) { 'PASS' } else { 'FAIL' }) 'preflight' "exit=$preflightExit log=$preflightLog"
if ($preflightExit -ne 0) {
  Write-Output "TERMINAL: FAIL - preflight exit=$preflightExit; acceptance NOT RUN"
  exit $preflightExit
}

# Stage 3: six-test privileged acceptance.
$exe = Join-Path $stage 'pr341_privileged_acceptance.exe'
$testLog = Join-Path $evidenceDir 'privileged-tests.log'
$testOut = Join-Path $evidenceDir 'privileged-stdout.log'
$testErr = Join-Path $evidenceDir 'privileged-stderr.log'
foreach ($p in @($testLog, $testOut, $testErr)) { if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Force } }
$process = Start-Process -FilePath $exe -ArgumentList @('--nocapture', '--test-threads', '1') -RedirectStandardOutput $testOut -RedirectStandardError $testErr -Wait -PassThru -NoNewWindow -ErrorAction Stop
$exit = $process.ExitCode
$combined = @()
if (Test-Path -LiteralPath $testOut) { $combined += Get-Content -LiteralPath $testOut }
if (Test-Path -LiteralPath $testErr) { $combined += Get-Content -LiteralPath $testErr }
$combined | Set-Content -LiteralPath $testLog -Encoding utf8
$results = @()
foreach ($line in $combined) {
  if ($line -match '^test\s+(.+?)\s+\.\.\.\s+(ok|FAILED|ignored)') {
    $results += [PSCustomObject]@{ test = $Matches[1]; status = $Matches[2] }
  }
}
foreach ($result in $results) {
  $mapped = switch ($result.status) { 'ok' { 'PASS' } 'FAILED' { 'FAIL' } default { 'NOT_RUN' } }
  Write-Result $mapped 'acceptance-test' "$($result.test) => $($result.status)"
}
$expected = @(
  'elevated_first_install_exact_bytes_and_trust',
  'monotonic_update_and_permanent_revocation',
  'negative_controls_fail_closed',
  'owned_nested_directories_are_removed_deepest_first',
  'test_mutex_local_regression_serializes_and_cleanup_is_verified',
  'sequential_generation_progression_control',
  'concurrent_provisioning_serializes_production_updates'
)
$missing = @($expected | Where-Object { $name = $_; -not ($results | Where-Object { $_.test -eq $name }) })
foreach ($name in $missing) { Write-Result 'NOT_RUN' 'acceptance-test' "missing: $name" }
$failed = @($results | Where-Object { $_.status -eq 'FAILED' }).Count
$ignored = @($results | Where-Object { $_.status -eq 'ignored' }).Count
$skips = @($combined | Select-String -SimpleMatch 'SKIP:').Count
$incomplete = -not (@($combined | Select-String -SimpleMatch 'test result:').Count -gt 0)
$status = if ($exit -eq 0 -and $results.Count -eq 7 -and $missing.Count -eq 0 -and $failed -eq 0 -and $ignored -eq 0 -and $skips -eq 0 -and -not $incomplete) { 'PASS' } else { 'FAIL' }
$summary = [PSCustomObject]@{
  source_sha = $manifest.source_sha
  acceptance_binary_sha256 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
  diagnostic_exit = $diagnosticExit
  preflight_exit = $preflightExit
  test_exit = $exit
  tests = $results
  failed_count = $failed
  ignored_count = $ignored
  skip_count = $skips
  missing_tests = $missing
  output_incomplete = $incomplete
  status = $status
  captured_utc = (Get-Date).ToUniversalTime().ToString('o')
}
$summary | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $evidenceDir 'summary.json') -Encoding utf8
Write-Result $status 'terminal' "exit=$exit evidence=$evidenceDir"
Write-Output "TERMINAL: $status - PR341 Stage 1 privileged acceptance exit=$exit"
exit $exit
