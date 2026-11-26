#requires -Version 5.1
# PR341 Stage 1 guest entry point. Fail-closed three-stage launcher:
#   1. strict native security diagnostic
#   2. bounded preflight
#   3. seven-test privileged acceptance
# Any stage failure stops the sequence. The final semantic exit is nonzero
# unless every genuine acceptance requirement passes, even when the raw
# acceptance process exit is zero with missing/skipped/ignored results.
param(
  [string]$PackageRoot = $PSScriptRoot,
  [Parameter(Mandatory=$true)][string]$ExpectedSourceSha,
  [Parameter(Mandatory=$true)][string]$ExpectedManifestSha256,
  [Parameter(Mandatory=$true)][string]$ExpectedPackageVersion,
  [switch]$SelfTest
)

$ErrorActionPreference = 'Stop'
$expectedTests = @(
  'elevated_first_install_exact_bytes_and_trust',
  'monotonic_update_and_permanent_revocation',
  'negative_controls_fail_closed',
  'owned_nested_directories_are_removed_deepest_first',
  'test_mutex_local_regression_serializes_and_cleanup_is_verified',
  'sequential_generation_progression_control',
  'concurrent_provisioning_serializes_production_updates'
)
$mandatoryArtifacts = @(
  'pr341_privileged_acceptance.exe',
  'pr397_provision_worker.exe',
  'pr402_security_diagnostic.exe',
  'run-pr341-acceptance-guest.ps1',
  'preflight-v5.ps1',
  'host-checkpoint-attestation.json',
  'source\owner_registry.rs',
  'source\pr341_privileged_acceptance.rs',
  'source\pr394_deadlock_witness.rs',
  'source\pr397_provision_worker.rs',
  'source\pr402_security_diagnostic.rs'
)
$log = $null

function Write-Result([string]$Status, [string]$Check, [string]$Detail) {
  $line = "$(Get-Date -Format o) [$Status] $Check :: $Detail"
  Write-Output $line
  if ($null -ne $log) { Add-Content -LiteralPath $log -Value $line -ErrorAction Stop }
}
function Refuse([string]$Reason) {
  Write-Result 'BLOCKED' 'preflight' $Reason
  Write-Output 'TERMINAL: BLOCKED - PR341 privileged acceptance did not run'
  exit 2
}

function Evaluate-Acceptance([string[]]$Lines, [int]$RawExit, [string[]]$Expected) {
  $results = @()
  $malformed = 0
  foreach ($line in $Lines) {
    if ($line -match '^test\s+(.+?)\s+\.\.\.\s+(ok|FAILED|ignored)\s*$') {
      $results += [PSCustomObject]@{ test = $Matches[1]; status = $Matches[2] }
    } elseif ($line -match '^test\s+' -and $line -notmatch '^test\s+result:') {
      $malformed++
    }
  }
  $counts = @{}
  foreach ($result in $results) {
    if (-not $counts.ContainsKey($result.test)) { $counts[$result.test] = 0 }
    $counts[$result.test]++
  }
  $duplicates = @($counts.Keys | Where-Object { $counts[$_] -gt 1 })
  $missing = @($Expected | Where-Object { -not $counts.ContainsKey($_) })
  $unexpected = @($counts.Keys | Where-Object { $_ -notin $Expected })
  $failed = @($results | Where-Object { $_.status -eq 'FAILED' }).Count
  $ignored = @($results | Where-Object { $_.status -eq 'ignored' }).Count
  $skipped = @($Lines | Select-String -SimpleMatch 'SKIP:').Count
  $incomplete = -not (@($Lines | Select-String -SimpleMatch 'test result:').Count -gt 0)
  $pass = ($RawExit -eq 0 -and $results.Count -eq $Expected.Count -and $missing.Count -eq 0 -and
           $duplicates.Count -eq 0 -and $unexpected.Count -eq 0 -and $failed -eq 0 -and
           $ignored -eq 0 -and $skipped -eq 0 -and $malformed -eq 0 -and -not $incomplete)
  [PSCustomObject]@{
    Semantic = $(if ($pass) { 'PASS' } else { 'FAIL' })
    RawExit = $RawExit
    Results = $results
    Missing = $missing
    Duplicates = $duplicates
    Unexpected = $unexpected
    Failed = $failed
    Ignored = $ignored
    Skipped = $skipped
    Malformed = $malformed
    Incomplete = $incomplete
  }
}

if ($SelfTest) {
  $good = @()
  foreach ($name in $expectedTests) { $good += "test $name ... ok" }
  $good += 'test result: ok. 7 passed; 0 failed'
  $cases = @(
    @{ name='valid'; lines=$good; raw=0; want='PASS' },
    @{ name='missing'; lines=@($good[0..5]) + 'test result: ok. 6 passed'; raw=0; want='FAIL' },
    @{ name='duplicate'; lines=$good + "test $($expectedTests[0]) ... ok"; raw=0; want='FAIL' },
    @{ name='ignored'; lines=@($good[0..5]) + "test $($expectedTests[6]) ... ignored" + 'test result: ok. 6 passed'; raw=0; want='FAIL' },
    @{ name='skipped'; lines=$good + 'SKIP: unsafe control skipped'; raw=0; want='FAIL' },
    @{ name='malformed'; lines=@($good[0..5]) + 'test bogus ... ???' + 'test result: ok. 6 passed'; raw=0; want='FAIL' },
    @{ name='incomplete'; lines=$good[0..6]; raw=0; want='FAIL' },
    @{ name='raw-nonzero'; lines=$good; raw=1; want='FAIL' }
  )
  foreach ($case in $cases) {
    $verdict = Evaluate-Acceptance $case.lines $case.raw $expectedTests
    if ($verdict.Semantic -ne $case.want) {
      Write-Output "SELFTEST_FAIL case=$($case.name) got=$($verdict.Semantic) want=$($case.want)"
      exit 99
    }
    Write-Output "SELFTEST_CASE $($case.name) $($verdict.Semantic)"
  }
  Write-Output 'SELFTEST_PASS'
  exit 0
}

if ($env:OS -notmatch 'Windows_NT') { Refuse 'non-Windows environment' }
if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') { Refuse "unsupported architecture $($env:PROCESSOR_ARCHITECTURE)" }

# Externally pinned identity, never self-reported package identity.
$stage = (Resolve-Path -LiteralPath $PackageRoot).Path
$manifestPath = Join-Path $stage 'manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath)) { Refuse 'manifest.json missing' }
$manifestSha = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($manifestSha -ne $ExpectedManifestSha256.ToLowerInvariant()) { Refuse "manifest SHA256 mismatch: $manifestSha" }
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ($manifest.source_sha.ToLowerInvariant() -ne $ExpectedSourceSha.ToLowerInvariant()) { Refuse 'source SHA mismatch' }
if ([string]$manifest.package_version -ne $ExpectedPackageVersion) { Refuse 'package version mismatch' }
$names = @($manifest.files | ForEach-Object { [string]$_.name })
if (@($names | Where-Object { $_ -match '\.\.' -or [IO.Path]::IsPathRooted($_) -or $_ -match '[/]' }).Count -gt 0) { Refuse 'escaping or malformed manifest entry' }
if (@($names | Group-Object | Where-Object Count -gt 1).Count -gt 0) { Refuse 'duplicate manifest entry' }
$missingArtifacts = @($mandatoryArtifacts | Where-Object { $_ -notin $names })
$extraArtifacts = @($names | Where-Object { $_ -notin $mandatoryArtifacts })
if ($missingArtifacts.Count -gt 0 -or $extraArtifacts.Count -gt 0) {
  Refuse "mandatory artifact inventory mismatch missing=$missingArtifacts extra=$extraArtifacts"
}
foreach ($entry in $manifest.files) {
  $path = Join-Path $stage ([string]$entry.name)
  if (-not (Test-Path -LiteralPath $path)) { Refuse "package artifact missing: $($entry.name)" }
  $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne ([string]$entry.sha256).ToLowerInvariant()) { Refuse "hash mismatch for $($entry.name)" }
}
Write-Result 'PASS' 'package-integrity' "source_sha=$($manifest.source_sha) manifest_sha=$manifestSha files=$($names.Count)"

$att = Get-Content -LiteralPath (Join-Path $stage 'host-checkpoint-attestation.json') -Raw | ConvertFrom-Json
$prefix = [string]$att.sid_prefix
if ($prefix -notmatch '^S-1-5-21-\d+-\d+-\d+$') { Refuse "invalid pinned SID prefix syntax: $prefix" }
$expectedName = [string]$att.machine_name
if ([string]::IsNullOrWhiteSpace($expectedName)) { Refuse 'pinned machine name missing' }
if ($att.vm_id -ne '2c3f151d-5974-4c7b-ae89-7d233dc2c614') { Refuse 'host attestation VM identity mismatch' }
if ($att.checkpoint -ne 'PR341-Windows11-Clean-Install') { Refuse 'host attestation checkpoint mismatch' }
$cs = Get-CimInstance Win32_ComputerSystem
if (-not $cs.Name.Equals($expectedName, [StringComparison]::OrdinalIgnoreCase)) { Refuse "machine name $($cs.Name) != pinned $expectedName" }
$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$actualSid = [string]$id.User.Value
if (-not $actualSid.StartsWith($prefix + '-', [StringComparison]::OrdinalIgnoreCase)) { Refuse "account SID $actualSid is not under pinned prefix $prefix" }
$rid = $actualSid.Substring($prefix.Length + 1)
if ($rid -notmatch '^\d+$') { Refuse "account SID $actualSid has an invalid RID" }
Write-Result 'PASS' 'guest-identity' "machine=$($cs.Name) sid=$actualSid prefix=$prefix"

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Pr341Elevation {
  [StructLayout(LayoutKind.Sequential)] public struct TOKEN_ELEVATION { public uint TokenIsElevated; }
  [DllImport("advapi32.dll", SetLastError=true)] public static extern bool OpenProcessToken(IntPtr p, uint a, out IntPtr h);
  [DllImport("advapi32.dll", SetLastError=true)] public static extern bool GetTokenInformation(IntPtr h, int c, out TOKEN_ELEVATION i, int s, out int r);
  [DllImport("kernel32.dll")] public static extern IntPtr GetCurrentProcess();
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
}
'@ -ErrorAction Stop
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

# Unique, nonreparse, guest-bound evidence destination. Earlier bytes are never touched.
$evidenceRoot = Join-Path $common 'PrometheOS-Lite-PR341-Evidence'
if (-not $evidenceRoot.StartsWith($common, [StringComparison]::OrdinalIgnoreCase)) { Refuse 'evidence root escapes guest ProgramData' }
if (Test-Path -LiteralPath $evidenceRoot) {
  $rootItem = Get-Item -LiteralPath $evidenceRoot -Force
  if (($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { Refuse 'evidence root is a reparse path' }
} else {
  New-Item -ItemType Directory -Path $evidenceRoot -ErrorAction Stop | Out-Null
}
$attemptId = 'attempt-{0}-{1}-{2}' -f $manifest.source_sha.Substring(0,12), (Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssfffZ'), $PID
$evidenceDir = Join-Path $evidenceRoot $attemptId
if (Test-Path -LiteralPath $evidenceDir) { Refuse "attempt evidence directory already exists: $attemptId" }
New-Item -ItemType Directory -Path $evidenceDir -ErrorAction Stop | Out-Null
$log = Join-Path $evidenceDir 'launcher.log'
Write-Result 'PASS' 'evidence-dir' "attempt=$attemptId path=$evidenceDir"
$startUtc = (Get-Date).ToUniversalTime().ToString('o')

$diagOut = Join-Path $evidenceDir 'diagnostic.log'
$preflightOut = Join-Path $evidenceDir 'preflight.log'
$testOut = Join-Path $evidenceDir 'privileged-stdout.log'
$testErr = Join-Path $evidenceDir 'privileged-stderr.log'
$testLog = Join-Path $evidenceDir 'privileged-tests.log'

$diagnostic = Join-Path $stage 'pr402_security_diagnostic.exe'
$env:PR402_STRICT_DIAGNOSTIC = '1'
& $diagnostic --nocapture --test-threads 1 2>&1 | Tee-Object -LiteralPath $diagOut -ErrorAction Stop
$diagnosticExit = $LASTEXITCODE
Remove-Item Env:PR402_STRICT_DIAGNOSTIC -ErrorAction SilentlyContinue
Write-Result $(if ($diagnosticExit -eq 0) { 'PASS' } else { 'FAIL' }) 'strict-diagnostic' "exit=$diagnosticExit evidence=$diagOut"
if ($diagnosticExit -ne 0) {
  $summary = [PSCustomObject]@{ attempt=$attemptId; source_sha=$manifest.source_sha; package_version=$ExpectedPackageVersion; manifest_sha256=$manifestSha; start_utc=$startUtc; end_utc=(Get-Date).ToUniversalTime().ToString('o'); diagnostic_exit=$diagnosticExit; preflight_exit='NOT_RUN'; raw_acceptance_exit='NOT_RUN'; semantic_exit=$diagnosticExit; status='FAIL'; evidence_dir=$evidenceDir }
  $summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $evidenceDir 'summary.json') -Encoding utf8 -ErrorAction Stop
  Write-Output "TERMINAL: FAIL diagnostic_exit=$diagnosticExit preflight=NOT_RUN acceptance=NOT_RUN"
  exit $diagnosticExit
}

$preflight = Join-Path $stage 'preflight-v5.ps1'
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $preflight 2>&1 | Tee-Object -LiteralPath $preflightOut -ErrorAction Stop
$preflightExit = $LASTEXITCODE
Write-Result $(if ($preflightExit -eq 0) { 'PASS' } else { 'FAIL' }) 'preflight' "exit=$preflightExit evidence=$preflightOut"
if ($preflightExit -ne 0) {
  $summary = [PSCustomObject]@{ attempt=$attemptId; source_sha=$manifest.source_sha; package_version=$ExpectedPackageVersion; manifest_sha256=$manifestSha; start_utc=$startUtc; end_utc=(Get-Date).ToUniversalTime().ToString('o'); diagnostic_exit=$diagnosticExit; preflight_exit=$preflightExit; raw_acceptance_exit='NOT_RUN'; semantic_exit=$preflightExit; status='FAIL'; evidence_dir=$evidenceDir }
  $summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $evidenceDir 'summary.json') -Encoding utf8 -ErrorAction Stop
  Write-Output "TERMINAL: FAIL diagnostic_exit=$diagnosticExit preflight_exit=$preflightExit acceptance=NOT_RUN"
  exit $preflightExit
}

$exe = Join-Path $stage 'pr341_privileged_acceptance.exe'
$process = Start-Process -FilePath $exe -ArgumentList @('--nocapture','--test-threads','1') -RedirectStandardOutput $testOut -RedirectStandardError $testErr -Wait -PassThru -NoNewWindow -ErrorAction Stop
$rawExit = $process.ExitCode
$combined = @()
if (Test-Path -LiteralPath $testOut) { $combined += Get-Content -LiteralPath $testOut }
if (Test-Path -LiteralPath $testErr) { $combined += Get-Content -LiteralPath $testErr }
$combined | Set-Content -LiteralPath $testLog -Encoding utf8 -ErrorAction Stop
$verdict = Evaluate-Acceptance $combined $rawExit $expectedTests
foreach ($result in $verdict.Results) {
  $mapped = switch ($result.status) { 'ok' { 'PASS' } 'FAILED' { 'FAIL' } default { 'NOT_RUN' } }
  Write-Result $mapped 'acceptance-test' "$($result.test) => $($result.status)"
}
foreach ($name in $verdict.Missing) { Write-Result 'NOT_RUN' 'acceptance-test' "missing: $name" }
foreach ($name in $verdict.Duplicates) { Write-Result 'FAIL' 'acceptance-test' "duplicate: $name" }
Write-Result $(if ($verdict.Skipped -eq 0) { 'PASS' } else { 'FAIL' }) 'skip-control' "SKIP_count=$($verdict.Skipped)"
Write-Result $(if ($verdict.Incomplete) { 'FAIL' } else { 'PASS' }) 'output-completeness' "test_result_line=$(-not $verdict.Incomplete)"
$semanticExit = if ($verdict.Semantic -eq 'PASS') { $rawExit } elseif ($rawExit -ne 0) { $rawExit } else { 90 }
$summary = [PSCustomObject]@{
  attempt=$attemptId; source_sha=$manifest.source_sha; package_version=$ExpectedPackageVersion
  manifest_sha256=$manifestSha; start_utc=$startUtc; end_utc=(Get-Date).ToUniversalTime().ToString('o')
  diagnostic_exit=$diagnosticExit; preflight_exit=$preflightExit
  raw_acceptance_exit=$rawExit; semantic_acceptance=$verdict.Semantic; semantic_exit=$semanticExit
  expected_test_count=$expectedTests.Count; executed_test_count=$verdict.Results.Count
  missing=$verdict.Missing; duplicates=$verdict.Duplicates; unexpected=$verdict.Unexpected
  failed=$verdict.Failed; ignored=$verdict.Ignored; skipped=$verdict.Skipped
  malformed=$verdict.Malformed; incomplete=$verdict.Incomplete
  status=$verdict.Semantic; evidence_dir=$evidenceDir
}
$summaryPath = Join-Path $evidenceDir 'summary.json'
$summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $summaryPath -Encoding utf8 -ErrorAction Stop
if (-not (Test-Path -LiteralPath $summaryPath)) { Refuse 'evidence persistence failed: summary.json missing' }
Write-Result $verdict.Semantic 'terminal' "raw_exit=$rawExit semantic_exit=$semanticExit evidence=$evidenceDir"
Write-Output "TERMINAL: $($verdict.Semantic) raw_exit=$rawExit semantic_exit=$semanticExit"
exit $semanticExit
