# Run with pwsh -NoProfile -File windows/tests/measure_memory_failure.test.ps1.
# Exercise the actual sampler with a process that disappears after enumeration.
$ErrorActionPreference = 'Stop'
$testDirectory = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path (Join-Path $testDirectory 'firefox') | Out-Null
Set-Content -LiteralPath (Join-Path $testDirectory 'firefox/firefox.exe') -Value 'fixture'
Set-Content -LiteralPath (Join-Path $testDirectory 'firefox/xul.dll') -Value 'fixture'
function Get-CimInstance($ClassName) {
    if ($ClassName -eq 'Win32_OperatingSystem') {
        return [pscustomobject]@{Version='fixture'; OSArchitecture='ARM64'; TotalVisibleMemorySize=1000}
    }
    return [pscustomobject]@{
        ProcessId=1234; ParentProcessId=0; Name='firefox.exe'; CreationDate=[DateTime]::UtcNow
        ExecutablePath=$testDirectory.TrimEnd('\') + '\firefox\firefox.exe'
    }
}
function Get-Process { throw 'Injected process exit before read' }
try {
    $outputFile = Join-Path $testDirectory 'sample.csv'
    & (Join-Path $PSScriptRoot 'measure_memory.ps1') -InstallDir $testDirectory -Build fixture -Run 1 -Phase loaded -DurationSeconds 1 -IntervalMs 100 -Output $outputFile | Out-Null
    $summary = Get-Content -Raw -LiteralPath "$outputFile.summary.json" | ConvertFrom-Json
    if ($summary.Comparable -or $summary.CompleteSamples -ne 0 -or $summary.IncompleteSamples -lt 1) {
        throw 'Failed process was counted as a complete sample'
    }
    if ($null -ne $summary.PrivateBytes.Median -or $null -ne $summary.PrivateBytes.Peak) {
        throw 'Missing measurements were converted to memory savings'
    }
    foreach ($row in (Import-Csv -LiteralPath $outputFile)) {
        if ($row.PrivateBytes -ne '' -or $row.Error -notmatch 'Injected process exit') {
            throw 'Incomplete aggregate or error was not preserved'
        }
    }
    foreach ($row in (Import-Csv -LiteralPath "$outputFile.processes.csv")) {
        if ($row.Complete -ne 'False' -or $row.Error -notmatch 'Injected process exit') {
            throw 'Per-process failure was not preserved'
        }
    }
    Write-Output 'PASS: failed processes remain incomplete and cannot count as savings'
} finally {
    Remove-Item -Recurse -Force -LiteralPath $testDirectory
}
