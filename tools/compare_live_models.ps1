# Run an isolated model comparison and record total GPU pressure (including the game).
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Baseline,
    [Parameter(Mandatory = $true)][string]$Candidate,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Server,
    [ValidateRange(1, 100)][int]$Runs = 3,
    [string]$GameState = 'unspecified',
    [ValidateSet('hy-mt', 'maru', 'hy-mt2')][string]$CandidatePrompt = 'hy-mt',
    [ValidateSet('matched', 'tencent')][string]$CandidateSampling = 'matched',
    [switch]$RollingContext,
    [string]$File
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$exe = Join-Path $repo 'target\release\maru.exe'
if (!(Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw 'Build the test CLI first: cargo build --release --locked -p maru-cli'
}
$reportPath = [IO.Path]::GetFullPath($Out)
$Baseline = [IO.Path]::GetFullPath($Baseline)
$Candidate = [IO.Path]::GetFullPath($Candidate)
if ($File) { $File = [IO.Path]::GetFullPath($File) }
if ($Server -and ($Server.Contains('\') -or $Server.Contains('/'))) { $Server = [IO.Path]::GetFullPath($Server) }
foreach ($path in @($reportPath, "$reportPath.gpu.csv", "$reportPath.console.log", "$reportPath.server.log")) {
    if (Test-Path -LiteralPath $path) { throw "Use a new output path; already exists: $path" }
}
$parent = Split-Path $reportPath -Parent
[IO.Directory]::CreateDirectory($parent) | Out-Null
$cliArgs = @('--verbose', 'bench-live-models', '--baseline', $Baseline, '--candidate', $Candidate, '--out', $reportPath, '-n', "$Runs", '--game-state', $GameState, '--candidate-prompt', $CandidatePrompt, '--candidate-sampling', $CandidateSampling)
if ($Server) { $cliArgs += @('--server', $Server) }
if ($RollingContext) { $cliArgs += '--rolling-context' }
if ($File) { $cliArgs += @('--file', $File) }
# Start-Process joins its argument array. Quote each argument so paths with spaces survive.
foreach ($arg in $cliArgs) {
    if ($arg.Contains('"') -or $arg.EndsWith('\')) { throw 'Arguments must not contain quotes or end in a backslash.' }
}
$quotedArgs = ($cliArgs | ForEach-Object { '"' + $_ + '"' }) -join ' '
$gpu = $null
$benchmark = $null
$previousRustLog = $env:RUST_LOG
try {
    $smi = Get-Command nvidia-smi -ErrorAction SilentlyContinue
    if ($smi) {
        $gpu = Start-Process -FilePath $smi.Source -ArgumentList '--query-gpu=timestamp,name,memory.used,memory.total,utilization.gpu,utilization.memory --format=csv -l 1' -WindowStyle Hidden -PassThru -RedirectStandardOutput "$reportPath.gpu.csv"
    } else {
        Write-Host 'nvidia-smi is unavailable; GPU telemetry will be absent.'
    }
    $env:RUST_LOG = 'info,llama=trace'
    $benchmark = Start-Process -FilePath $exe -ArgumentList $quotedArgs -WorkingDirectory $repo -WindowStyle Hidden -PassThru -RedirectStandardOutput "$reportPath.console.log" -RedirectStandardError "$reportPath.server.log"
    # Windows PowerShell needs the handle opened before exit to retain the exit code.
    $null = $benchmark.Handle
    Write-Host "Comparison running. Progress: $reportPath.console.log"
    $benchmark.WaitForExit()
    $exitCode = $benchmark.ExitCode
} finally {
    $env:RUST_LOG = $previousRustLog
    # Only processes launched by this invocation are stopped; never the game or another server.
    if ($benchmark -and !$benchmark.HasExited) { Stop-Process -Id $benchmark.Id }
    if ($gpu -and !$gpu.HasExited) { Stop-Process -Id $gpu.Id }
}
Get-Content -LiteralPath "$reportPath.console.log" -Tail 12
if ($exitCode -ne 0) {
    Get-Content -LiteralPath "$reportPath.server.log" -Tail 12
    throw "Comparison exited with code $exitCode; inspect the report and logs."
}
Write-Host "Review: $reportPath\review.md"
