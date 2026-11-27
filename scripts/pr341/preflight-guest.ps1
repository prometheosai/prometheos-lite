#requires -Version 5.1
# Bounded, fail-closed, READ-ONLY guest preflight for PR341 Stage 1.
# No process termination, no checkpoint operation, no protected-state mutation.
# No dependency on historical V5 evidence directories or PID 4248 presence.
param(
  [string]$ExpectedLauncherPath,
  [switch]$ProcessFilterOnly
)
$ErrorActionPreference = 'Stop'
$root = 'C:\ProgramData'
$registry = Join-Path $root 'PrometheOS\Lite\owner-registry\registry.json'
$ownerDir = Split-Path -Parent $registry
$evidenceRoot = Join-Path $root 'PrometheOS-Lite-PR341-Evidence'

function Write-Line([string]$Status, [string]$Check, [string]$Detail) {
  Write-Output "$(Get-Date -Format o) [$Status] $Check :: $Detail"
}
function Refuse([string]$Reason) {
  Write-Line 'BLOCKED' 'preflight' $Reason
  exit 3
}
function Assert-PlainDirectory([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path)) { return }
  $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
  if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or -not $item.PSIsContainer) {
    Refuse "reparse or non-directory at $Path"
  }
}

function Test-AcceptanceProcesses {
  if ([string]::IsNullOrWhiteSpace($ExpectedLauncherPath)) {
    Refuse 'expected launcher path is required for process validation'
  }
  $expected = (Resolve-Path -LiteralPath $ExpectedLauncherPath -ErrorAction Stop).Path
  $self = Get-CimInstance Win32_Process -Filter "ProcessId=$PID" -ErrorAction Stop
  if ($null -eq $self) { Refuse 'cannot identify current preflight process' }
  $parentPid = [int]$self.ParentProcessId
  $parent = Get-CimInstance Win32_Process -Filter "ProcessId=$parentPid" -ErrorAction Stop
  if ($null -eq $parent) { Refuse 'cannot identify direct parent process' }
  $parentCmd = [string]$parent.CommandLine
  if ($parentCmd -notmatch 'run-pr341-acceptance-guest\.ps1' -or
      $parentCmd -notmatch [regex]::Escape($expected)) {
    Refuse "direct parent $parentPid is not the expected reviewed launcher at $expected"
  }

  $pattern = 'pr341_privileged_acceptance|pr402_security_diagnostic|pr397_provision_worker|pr394_deadlock_witness|run-pr341-acceptance-guest\.ps1|preflight-guest\.ps1'
  $nativeNames = '^(pr341_privileged_acceptance|pr402_security_diagnostic|pr397_provision_worker|pr394_deadlock_witness)(\.exe)?$'
  $procs = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
    $_.ProcessId -ne $PID -and $_.ProcessId -ne $parentPid -and
    (([string]$_.CommandLine) -match $pattern -or ([string]$_.Name) -match $nativeNames)
  })
  if (@($procs | Where-Object { $_.ProcessId -eq 4248 }).Count -gt 0) {
    Refuse 'historical PID 4248 still exists and matches acceptance tooling'
  }
  if ($procs.Count -gt 0) {
    Refuse "competing acceptance tooling processes: $(($procs | ForEach-Object { $_.ProcessId }) -join ',')"
  }
  Write-Line 'PASS' 'process-state' "validated direct parent=$parentPid; no competing acceptance tooling"
}

if ($ProcessFilterOnly) {
  Test-AcceptanceProcesses
  Write-Line 'PASS' 'terminal' 'process-filter-only check complete; no process stopped'
  exit 0
}

if ($env:OS -notmatch 'Windows_NT') { Refuse 'non-Windows environment' }
$id = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not ([Security.Principal.WindowsPrincipal]::new($id)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Refuse 'elevated Administrator token required'
}
if ([Environment]::GetFolderPath('CommonApplicationData') -ne $root) { Refuse 'ProgramData root mismatch' }

foreach ($path in @($root, (Join-Path $root 'PrometheOS'), (Join-Path $root 'PrometheOS\Lite'), $ownerDir)) {
  Assert-PlainDirectory $path
}

if (Test-Path -LiteralPath $registry) {
  Refuse "protected registry leaf present: $registry"
}
if (Test-Path -LiteralPath $ownerDir) {
  $unexpected = @(Get-ChildItem -LiteralPath $ownerDir -Force -ErrorAction Stop)
  if ($unexpected.Count -gt 0) {
    Refuse "unexplained protected registry contents: $($unexpected.Name -join ',')"
  }
}

if (Test-Path -LiteralPath $evidenceRoot) {
  Assert-PlainDirectory $evidenceRoot
  foreach ($item in Get-ChildItem -LiteralPath $evidenceRoot -Force -ErrorAction Stop) {
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
      Refuse "reparse evidence object: $($item.FullName)"
    }
  }
}
Write-Line 'PASS' 'historical-evidence' 'existing evidence inspected read-only; nothing copied or deleted'

Test-AcceptanceProcesses

Write-Line 'PASS' 'terminal' 'read-only preflight complete; no process stopped, no protected state changed, no checkpoint restored'
exit 0
