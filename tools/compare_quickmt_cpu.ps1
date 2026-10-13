# Direct CPU translation only; keep other model services unloaded for comparable results.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Python,
    [Parameter(Mandatory = $true)][string]$Model,
    [Parameter(Mandatory = $true)][string]$File,
    [Parameter(Mandatory = $true)][string]$Out,
    [ValidateSet(1, 2, 5)][int]$BeamSize = 1,
    [ValidateSet('int8', 'float32')][string]$ComputeType = 'int8',
    [ValidateRange(1, 32)][int]$Threads = 4,
    [ValidateRange(1, 100)][int]$Runs = 3,
    [string]$GameState = 'unspecified'
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$reportPath = [IO.Path]::GetFullPath($Out)
foreach ($path in @($reportPath, "$reportPath.gpu.csv", "$reportPath.console.log", "$reportPath.error.log")) {
    if (Test-Path -LiteralPath $path) { throw "Use a new output path: $path" }
}
[IO.Directory]::CreateDirectory((Split-Path $reportPath -Parent)) | Out-Null
$arguments = @((Join-Path $PSScriptRoot 'bench_quickmt.py'), '--model', [IO.Path]::GetFullPath($Model), '--file', [IO.Path]::GetFullPath($File), '--out', $reportPath, '--beam-size', "$BeamSize", '--compute-type', $ComputeType, '--threads', "$Threads", '--runs', "$Runs", '--game-state', $GameState)
foreach ($argument in $arguments) {
    if ($argument.Contains('"') -or $argument.EndsWith('\')) { throw 'Arguments must not contain quotes or end in a backslash.' }
}
$quoted = ($arguments | ForEach-Object { '"' + $_ + '"' }) -join ' '
$previousUtf8 = $env:PYTHONUTF8
$benchmark = $null
$gpu = $null
try {
    $env:PYTHONUTF8 = '1'
    $smi = Get-Command nvidia-smi -ErrorAction SilentlyContinue
    if ($smi) {
        $gpu = Start-Process -FilePath $smi.Source -ArgumentList '--query-gpu=timestamp,name,memory.used,memory.total,utilization.gpu,utilization.memory --format=csv -l 1' -WindowStyle Hidden -PassThru -RedirectStandardOutput "$reportPath.gpu.csv"
    }
    $benchmark = Start-Process -FilePath ([IO.Path]::GetFullPath($Python)) -ArgumentList $quoted -WorkingDirectory $repo -WindowStyle Hidden -PassThru -RedirectStandardOutput "$reportPath.console.log" -RedirectStandardError "$reportPath.error.log"
    $null = $benchmark.Handle
    $benchmark.WaitForExit()
    $exitCode = $benchmark.ExitCode
} finally {
    $env:PYTHONUTF8 = $previousUtf8
    if ($benchmark -and !$benchmark.HasExited) { Stop-Process -Id $benchmark.Id }
    if ($gpu -and !$gpu.HasExited) { Stop-Process -Id $gpu.Id }
}
Get-Content -LiteralPath "$reportPath.console.log" -Tail 5
if ($exitCode -ne 0) {
    Get-Content -LiteralPath "$reportPath.error.log" -Tail 12
    throw "QuickMT exited with code $exitCode; inspect preserved records."
}
