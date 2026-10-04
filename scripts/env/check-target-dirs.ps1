# Fails (exit 1) when two lanes share a CARGO_TARGET_DIR or any dir is not on D:.
[CmdletBinding()]
param([Parameter(Mandatory)] [string[]] $Assignment)  # each item: lane=dir
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$seen = @{}
$errors = @()
foreach ($item in ($Assignment | ForEach-Object { $_ -split ',' } | Where-Object { $_ })) {
    $i = $item.IndexOf('=')
    if ($i -lt 1) { $errors += "Malformed assignment '$item' (expected lane=dir)."; continue }
    $lane = $item.Substring(0, $i)
    $dir = $item.Substring($i + 1)
    $norm = ($dir -replace '/', [string][char]92).TrimEnd([char]92).ToLowerInvariant()
    if ($norm -notmatch '^d:.+') { $errors += "Lane '$lane': target dir '$dir' is not on D:."; continue }
    if ($seen.ContainsKey($norm)) { $errors += "Lanes '$($seen[$norm])' and '$lane' share target dir '$dir'." }
    else { $seen[$norm] = $lane }
}
if ($errors.Count) { $errors | ForEach-Object { [Console]::Error.WriteLine($_) }; exit 1 }
Write-Output "OK: $($seen.Count) distinct target dirs on D:."
exit 0
