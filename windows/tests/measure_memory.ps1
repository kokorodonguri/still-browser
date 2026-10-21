param(
    [Parameter(Mandatory=$true)][string]$InstallDir,
    [Parameter(Mandatory=$true)][string]$Build,
    [Parameter(Mandatory=$true)][ValidateRange(1,100)][int]$Run,
    [Parameter(Mandatory=$true)][string]$Phase,
    [ValidateRange(1,86400)][int]$DurationSeconds = 120,
    [ValidateRange(100,60000)][int]$IntervalMs = 1000,
    [string]$Output = 'memory.csv'
)
$ErrorActionPreference = 'Stop'
# Do not combine independent runs by appending to an existing result.
$processOutput = "$Output.processes.csv"
$summaryOutput = "$Output.summary.json"
foreach ($file in @($Output, $processOutput, $summaryOutput)) {
    if (Test-Path -LiteralPath $file) { throw "Output already exists: $file" }
}
$installation = (Resolve-Path -LiteralPath $InstallDir).Path.TrimEnd('\') + '\'
$engine = Join-Path $installation 'firefox\firefox.exe'
$engineHash = (Get-FileHash -LiteralPath $engine -Algorithm SHA256).Hash
$xulHash = (Get-FileHash -LiteralPath (Join-Path $installation 'firefox\xul.dll') -Algorithm SHA256).Hash
$os = Get-CimInstance Win32_OperatingSystem
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class StillProcessMemory {
    [StructLayout(LayoutKind.Sequential)] public struct Counters {
        public uint Size, PageFaultCount;
        public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage,
            QuotaPagedPoolUsage, QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage,
            PagefileUsage, PeakPagefileUsage, PrivateUsage;
    }
    [DllImport("psapi.dll", SetLastError=true)]
    static extern bool GetProcessMemoryInfo(IntPtr process, ref Counters counters, uint size);
    public static Counters Read(IntPtr process) {
        var counters = new Counters();
        counters.Size = (uint)Marshal.SizeOf(counters);
        if (!GetProcessMemoryInfo(process, ref counters, counters.Size))
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        return counters;
    }
}
'@
$clock = [Diagnostics.Stopwatch]::StartNew()
$rows = [Collections.Generic.List[object]]::new()
$index = 0
while ($clock.Elapsed.TotalSeconds -lt $DurationSeconds) {
    $startMs = $clock.Elapsed.TotalMilliseconds
    $errors = [Collections.Generic.List[string]]::new()
    $selected = @{}
    try {
        $processes = @(Get-CimInstance Win32_Process)
        foreach ($entry in $processes) {
            if ($entry.ExecutablePath -and $entry.ExecutablePath.StartsWith($installation, [StringComparison]::OrdinalIgnoreCase)) {
                $selected[[int]$entry.ProcessId] = $entry
            }
        }
        do {
            $added = $false
            foreach ($entry in $processes) {
                if ($selected.ContainsKey([int]$entry.ParentProcessId) -and !$selected.ContainsKey([int]$entry.ProcessId)) {
                    $selected[[int]$entry.ProcessId] = $entry
                    $added = $true
                }
            }
        } while ($added)
        # An inaccessible browser process cannot silently disappear from the total.
        foreach ($entry in $processes) {
            if (!$entry.ExecutablePath -and $entry.Name -match '^(firefox|still)\.exe$') {
                $errors.Add("Unresolved executable path for PID $($entry.ProcessId)")
            }
        }
    } catch { $errors.Add("Enumeration: $($_.Exception.Message)") }
    if (!$selected.Count) { $errors.Add('No Still processes found') }
    $private = 0L; $working = 0L; $success = 0
    $timeUtc = [DateTime]::UtcNow.ToString('o')
    foreach ($processId in $selected.Keys) {
        $entry = $selected[$processId]
        $process = $null
        $row = [ordered]@{
            Sample = $index; TimeUtc = $timeUtc; ElapsedMs = $startMs
            Build = $Build; Run = $Run; Phase = $Phase; PID = $processId
            Name = $entry.Name; CreationUtc = $entry.CreationDate.ToUniversalTime().ToString('o')
            Complete = $false; PrivateBytes = $null; WorkingSetBytes = $null
            PageFaultCount = $null; CpuMs = $null; Error = ''
        }
        try {
            $process = Get-Process -Id $processId -ErrorAction Stop
            if ([Math]::Abs(($process.StartTime.ToUniversalTime() - $entry.CreationDate.ToUniversalTime()).TotalSeconds) -gt 0.01) {
                throw 'PID reused between enumeration and sampling'
            }
            $counters = [StillProcessMemory]::Read($process.Handle)
            $row.PrivateBytes = $counters.PrivateUsage.ToUInt64()
            $row.WorkingSetBytes = $counters.WorkingSetSize.ToUInt64()
            $row.PageFaultCount = $counters.PageFaultCount
            $row.CpuMs = $process.TotalProcessorTime.TotalMilliseconds
            if ($process.HasExited) { throw 'Process exited during sampling' }
            $row.Complete = $true
            $private += $row.PrivateBytes; $working += $row.WorkingSetBytes; $success++
        } catch {
            $row.Error = $_.Exception.Message
            $row.PrivateBytes = $null; $row.WorkingSetBytes = $null
            $row.PageFaultCount = $null; $row.CpuMs = $null
            $errors.Add("PID ${processId}: $($row.Error)")
        } finally { if ($process) { $process.Dispose() } }
        [pscustomobject]$row | Export-Csv -LiteralPath $processOutput -NoTypeInformation -Append -Encoding UTF8
    }
    $complete = $errors.Count -eq 0
    $sample = [pscustomobject]@{
        Sample = $index; TimeUtc = $timeUtc; ElapsedMs = $startMs; Build = $Build
        Run = $Run; Phase = $Phase; Complete = $complete
        ExpectedProcesses = $selected.Count; MeasuredProcesses = $success
        PrivateBytes = $(if ($complete) { $private } else { $null })
        WorkingSetBytes = $(if ($complete) { $working } else { $null })
        SamplingMs = $clock.Elapsed.TotalMilliseconds - $startMs
        Error = $errors -join '; '; EngineSha256 = $engineHash; XulSha256 = $xulHash
    }
    $rows.Add($sample)
    $sample | Export-Csv -LiteralPath $Output -NoTypeInformation -Append -Encoding UTF8
    $index++
    $remainingMs = [Math]::Min($IntervalMs - ($clock.Elapsed.TotalMilliseconds - $startMs), $DurationSeconds * 1000 - $clock.Elapsed.TotalMilliseconds)
    if ($remainingMs -gt 0) { Start-Sleep -Milliseconds ([int]$remainingMs) }
}
function Stats($values) {
    $sorted = @($values | Sort-Object)
    if (!$sorted.Count) { return @{ Median = $null; Peak = $null } }
    $middle = [int][Math]::Floor($sorted.Count / 2)
    $median = $sorted[$middle]
    if ($sorted.Count % 2 -eq 0) { $median = ($sorted[$middle - 1] + $median) / 2 }
    return @{ Median = $median; Peak = $sorted[-1] }
}
$valid = @($rows | Where-Object Complete)
[ordered]@{
    Build = $Build; Run = $Run; Phase = $Phase; EngineSha256 = $engineHash; XulSha256 = $xulHash
    OSVersion = $os.Version; Architecture = $os.OSArchitecture; TotalPhysicalKiB = $os.TotalVisibleMemorySize
    RequestedDurationSeconds = $DurationSeconds; RequestedIntervalMs = $IntervalMs; DurationSeconds = $clock.Elapsed.TotalSeconds
    Samples = $rows.Count; CompleteSamples = $valid.Count; IncompleteSamples = $rows.Count - $valid.Count
    Comparable = ($valid.Count -eq $rows.Count -and $valid.Count -ge 2)
    PrivateBytes = Stats @($valid | ForEach-Object PrivateBytes)
    WorkingSetBytes = Stats @($valid | ForEach-Object WorkingSetBytes)
    Notes = 'Statistics use complete samples only. Incomplete runs must not support reduction claims. PageFaultCount includes soft faults and wraps at 2^32; use deltas per PID+CreationUtc. Working sets double-count shared pages.'
} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $summaryOutput -Encoding UTF8
Write-Output $summaryOutput
