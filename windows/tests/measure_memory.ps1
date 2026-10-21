param(
    [Parameter(Mandatory=$true)][string]$InstallDir,
    [Parameter(Mandatory=$true)][string]$Build,
    [Parameter(Mandatory=$true)][ValidateRange(1,3)][int]$Run,
    [Parameter(Mandatory=$true)][ValidateSet('startup','loaded10','background6m','closed')][string]$Phase,
    [string]$Output = 'memory.csv'
)
$ErrorActionPreference = 'Stop'
$installation = (Resolve-Path -LiteralPath $InstallDir).Path.TrimEnd('\') + '\'
$processes = @(Get-CimInstance Win32_Process)
$selected = @{}
# Include the launcher, all engine processes and their descendants.
foreach ($entry in $processes) {
    if ($entry.ExecutablePath -and $entry.ExecutablePath.StartsWith($installation, [StringComparison]::OrdinalIgnoreCase)) {
        $selected[[int]$entry.ProcessId] = $true
    }
}
do {
    $added = $false
    foreach ($entry in $processes) {
        if ($selected.ContainsKey([int]$entry.ParentProcessId) -and !$selected.ContainsKey([int]$entry.ProcessId)) {
            $selected[[int]$entry.ProcessId] = $true
            $added = $true
        }
    }
} while ($added)
$samples = @(foreach ($processId in $selected.Keys) {
    try { Get-Process -Id $processId -ErrorAction Stop } catch { } # May exit between snapshots.
})
if (!$samples.Count) { throw "No Still processes found under $installation" }
$engine = Join-Path $installation 'firefox\firefox.exe'
$os = Get-CimInstance Win32_OperatingSystem
[pscustomobject]@{
    TimeUtc = (Get-Date).ToUniversalTime().ToString('o')
    Build = $Build
    Run = $Run
    Phase = $Phase
    Processes = $samples.Count
    PrivateBytes = [long](($samples | Measure-Object -Property PrivateMemorySize64 -Sum).Sum)
    WorkingSetBytes = [long](($samples | Measure-Object -Property WorkingSet64 -Sum).Sum)
    EngineSha256 = (Get-FileHash -LiteralPath $engine -Algorithm SHA256).Hash
    OSVersion = $os.Version
    Architecture = $os.OSArchitecture
    TotalPhysicalKiB = $os.TotalVisibleMemorySize
} | Export-Csv -LiteralPath $Output -NoTypeInformation -Append -Encoding UTF8
