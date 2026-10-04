[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $InputPath,

    [Parameter(Mandatory = $true)]
    [string] $OutputPath,

    [Parameter(Mandatory = $true)]
    [string] $ObservedCutoff,

    [ValidateSet('e0', 'original')]
    [string] $Source = 'e0',

    [ValidateRange(0, 5000)]
    [int] $ArranqueCases = 0,

    [ValidateRange(5, 5000)]
    [int] $MinimumRecurringQueryCases = 20
)

$ErrorActionPreference = 'Stop'

if ($Source -eq 'original' -and (
    $PSBoundParameters.ContainsKey('ArranqueCases') -or
    $PSBoundParameters.ContainsKey('MinimumRecurringQueryCases')
)) {
    throw 'E0-only case-count options cannot be supplied for original source.'
}

function Get-FullDirectoryPath {
    param([Parameter(Mandatory = $true)][string] $Path)

    try {
        return [System.IO.Path]::GetFullPath($Path)
    }
    catch {
        throw 'Input and output must be valid filesystem paths.'
    }
}

function Test-PathsOverlap {
    param(
        [Parameter(Mandatory = $true)][string] $Left,
        [Parameter(Mandatory = $true)][string] $Right
    )

    $leftPath = [System.IO.Path]::GetFullPath($Left).TrimEnd('\', '/')
    $rightPath = [System.IO.Path]::GetFullPath($Right).TrimEnd('\', '/')
    $separator = [System.IO.Path]::DirectorySeparatorChar.ToString()
    $comparison = [System.StringComparison]::OrdinalIgnoreCase
    $same = $leftPath.Equals($rightPath, $comparison)
    $leftContainsRight = $rightPath.StartsWith($leftPath + $separator, $comparison)
    $rightContainsLeft = $leftPath.StartsWith($rightPath + $separator, $comparison)

    return $same -or $leftContainsRight -or $rightContainsLeft
}

function Assert-NoReparsePointComponents {
    param(
        [Parameter(Mandatory = $true)][string] $Path,
        [Parameter(Mandatory = $true)][string] $Label
    )

    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $root = [System.IO.Path]::GetPathRoot($fullPath)
    if ([string]::IsNullOrWhiteSpace($root)) {
        throw "$Label must be a filesystem path."
    }

    $current = $root
    $components = $fullPath.Substring($root.Length).Split(
        [char[]] @([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar),
        [System.StringSplitOptions]::RemoveEmptyEntries
    )
    foreach ($component in $components) {
        $current = Join-Path $current $component
        try {
            $attributes = [System.IO.File]::GetAttributes($current)
        }
        catch [System.IO.FileNotFoundException] {
            continue
        }
        catch [System.IO.DirectoryNotFoundException] {
            continue
        }

        if (($attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "$Label traverses a reparse point; use a direct filesystem path."
        }
    }
}

function Get-NonNegativeInteger {
    param(
        [Parameter(Mandatory = $true)][System.Collections.IDictionary] $Object,
        [Parameter(Mandatory = $true)][string] $Name
    )

    $value = $Object[$Name]
    if ($null -eq $value -or $value -isnot [ValueType] -or $value -is [bool]) {
        throw 'Engine result is missing a safe aggregate; raw result values are suppressed.'
    }

    try {
        $number = [decimal] $value
    }
    catch {
        throw 'Engine result contains an invalid safe aggregate; raw result values are suppressed.'
    }

    if ($number -lt 0 -or [decimal]::Truncate($number) -ne $number) {
        throw 'Engine result contains an invalid safe aggregate; raw result values are suppressed.'
    }

    return [long] $number
}

function Assert-AllowedValue {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string] $Value,
        [Parameter(Mandatory = $true)][string[]] $Allowed
    )

    if ($Value -notin $Allowed) {
        throw 'Engine result contains an unsupported summary code; raw result values are suppressed.'
    }
}

function Invoke-LocalCargoRun {
    param([Parameter(Mandatory = $true)][string[]] $Arguments)
    & cargo @Arguments 2>&1 | ForEach-Object {
        $line = [string] $_
        try {
            $event = $line | ConvertFrom-Json -ErrorAction Stop
            if ($event.schema_version -ne 1 -or $event.event -ne 'run_progress') {
                return
            }
            if ($event.phase -eq 'source_preparation' -and $event.stage -in @('inventory', 'manifest_scan', 'contact_projection')) {
                $stageLabels = @{
                    inventory = 'source inventory'
                    manifest_scan = 'manifest scan'
                    contact_projection = 'contact projection'
                }
                if ($event.status -notin @('started', 'progress', 'completed', 'failed') -or
                    $event.files_completed -isnot [long] -or $event.files_total -isnot [long] -or
                    $event.bytes_completed -isnot [long] -or $event.bytes_total -isnot [long] -or
                    $event.elapsed_ms -isnot [long] -or $event.files_completed -gt $event.files_total -or
                    $event.bytes_completed -gt $event.bytes_total) {
                    return
                }
                $bytesCompleted = Format-SafeCount -Value $event.bytes_completed
                $bytesTotal = Format-SafeCount -Value $event.bytes_total
                $statusLabel = switch ($event.status) {
                    'started' { 'started' }
                    'progress' { 'in progress' }
                    'completed' { 'complete' }
                    'failed' { 'failed' }
                }
                $fileSummary = if ($event.stage -eq 'inventory' -and $event.status -eq 'started') {
                    'inventory pending'
                }
                else {
                    "{0}/{1} files" -f $event.files_completed, $event.files_total
                }
                $byteSummary = if ($event.status -eq 'started' -and
                    $event.stage -in @('inventory', 'contact_projection')) {
                    'size pending'
                }
                else {
                    "{0}/{1} bytes" -f $bytesCompleted, $bytesTotal
                }
                Write-Host ("Progress: {0} {1} — {2}; {3}; {4} ms." -f `
                    $stageLabels[$event.stage], $statusLabel, $fileSummary, $byteSummary, $event.elapsed_ms)
                return
            }
            if ($event.phase -in @('source_preparation', 'detection', 'post_selection_holdout', 'persist_outputs') -and
                $event.status -in @('started', 'completed', 'failed', 'skipped')) {
                Write-Host ("Progress: {0} {1}." -f $event.phase.Replace('_', ' '), $event.status)
            }
        }
        catch {
            # Cargo diagnostics and all unrecognized output are intentionally suppressed.
        }
    }
    return $LASTEXITCODE
}

function Invoke-LocalE0SourceValidation {
    param([Parameter(Mandatory = $true)][string[]] $Arguments)

    try {
        $cargoExitCode = Invoke-LocalCargoRun -Arguments $Arguments
    }
    catch {
        throw 'E0 source validation failed: could not start Cargo. No run was started.'
    }

    if ($cargoExitCode -ne 0) {
        throw "E0 source validation failed (Cargo exit code $cargoExitCode). No run was started."
    }
}

function Format-SafeCount {
    param([Parameter(Mandatory = $true)][long] $Value)
    return $Value.ToString([System.Globalization.CultureInfo]::InvariantCulture)
}

try {
    $resolvedInput = Resolve-Path -LiteralPath $InputPath -ErrorAction Stop
}
catch {
    throw 'InputPath must name an existing filesystem directory.'
}
if ($resolvedInput.Provider.Name -ne 'FileSystem' -or -not (Test-Path -LiteralPath $resolvedInput.Path -PathType Container)) {
    throw 'InputPath must name an existing filesystem directory.'
}
$inputFullPath = Get-FullDirectoryPath -Path $resolvedInput.Path
$outputFullPath = Get-FullDirectoryPath -Path $OutputPath

Assert-NoReparsePointComponents -Path $inputFullPath -Label 'InputPath'
Assert-NoReparsePointComponents -Path $outputFullPath -Label 'OutputPath'
if (Test-PathsOverlap -Left $inputFullPath -Right $outputFullPath) {
    throw 'InputPath and OutputPath must not overlap.'
}
if (Test-Path -LiteralPath $outputFullPath) {
    throw 'OutputPath already exists; choose a new path. Existing output is never overwritten.'
}

if ($ObservedCutoff -cnotmatch '\A\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z\z') {
    throw 'ObservedCutoff must be UTC RFC3339 with whole-second precision, for example 2026-10-02T18:00:00Z.'
}
$parsedCutoff = [System.DateTimeOffset]::MinValue
$cutoffFormat = "yyyy-MM-dd'T'HH:mm:ss'Z'"
$cutoffStyle = [System.Globalization.DateTimeStyles]::AssumeUniversal -bor [System.Globalization.DateTimeStyles]::AdjustToUniversal
$validCutoff = [System.DateTimeOffset]::TryParseExact(
    $ObservedCutoff,
    $cutoffFormat,
    [System.Globalization.CultureInfo]::InvariantCulture,
    $cutoffStyle,
    [ref] $parsedCutoff
)
if (-not $validCutoff) {
    throw 'ObservedCutoff is not a valid UTC calendar timestamp.'
}

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot 'Cargo.toml') -PathType Leaf)) {
    throw 'Could not locate the improvement-engine Rust workspace.'
}

$effectiveArranqueCases = $ArranqueCases
if ($effectiveArranqueCases -eq 0) {
    if ($Source -eq 'e0') {
        $effectiveArranqueCases = 200
    }
    else {
        $effectiveArranqueCases = 1
    }
}
$effectiveArranqueCasesText = [Convert]::ToString(
    $effectiveArranqueCases,
    [System.Globalization.CultureInfo]::InvariantCulture
)

if ($Source -eq 'e0') {
    $validationArguments = @(
        'run',
        '--locked',
        '--offline',
        '--target-dir',
        'target-local-e2e',
        '-p',
        'improvement-engine-runner',
        '--',
        'source',
        'validate',
        '--kind',
        'enriched_history',
        '--input',
        $inputFullPath,
        '--contract-version',
        '0.5.1'
    )
    Push-Location -LiteralPath $repositoryRoot
    try {
        Invoke-LocalE0SourceValidation -Arguments $validationArguments
    }
    finally {
        Pop-Location
    }
}

$cargoArguments = @(
    'run',
    '--locked',
    '--offline',
    '--target-dir',
    'target-local-e2e',
    '-p',
    'improvement-engine-runner',
    '--',
    'local-sim',
    '--mode',
    'local-simulation',
    '--source',
    $Source,
    '--input',
    $inputFullPath,
    '--output',
    $outputFullPath,
    '--tenant-id',
    'pulso_local',
    '--observed-cutoff',
    $ObservedCutoff,
    '--progress-jsonl'
)
if ($Source -eq 'e0') {
    $cargoArguments += @(
        '--arranque-cases',
        $effectiveArranqueCasesText,
        '--min-recurring-query-cases',
        $MinimumRecurringQueryCases.ToString([System.Globalization.CultureInfo]::InvariantCulture)
    )
}

Push-Location -LiteralPath $repositoryRoot
try {
    try {
        $cargoExitCode = Invoke-LocalCargoRun -Arguments $cargoArguments
    }
    catch {
        throw 'Could not start Cargo. Confirm Rust and Cargo are installed and available on PATH.'
    }
}
finally {
    Pop-Location
}

if ($cargoExitCode -ne 0) {
    throw "Local E0 run failed (Cargo exit code $cargoExitCode). Cargo diagnostics were suppressed to avoid printing dataset paths or content."
}
if (-not (Test-Path -LiteralPath $outputFullPath -PathType Container)) {
    throw 'The local runner completed without creating its expected output directory.'
}

$resultFiles = @(Get-ChildItem -LiteralPath $outputFullPath -Directory | ForEach-Object {
    $candidate = Join-Path $_.FullName 'result.json'
    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
        Get-Item -LiteralPath $candidate
    }
})
if ($resultFiles.Count -ne 1) {
    throw 'Expected exactly one completed run result; raw output is suppressed.'
}

try {
    $result = Get-Content -LiteralPath $resultFiles[0].FullName -Raw | ConvertFrom-Json -AsHashtable
}
catch {
    throw 'The local runner produced an unreadable result; raw output is suppressed.'
}

$allowedRunStatuses = if ($Source -eq 'e0') {
    @('complete_simulated', 'complete_no_opportunity', 'unsupported_source')
}
else {
    @('snapshot_descriptive_finding_ready', 'snapshot_projection_complete', 'unsupported_source')
}
Assert-AllowedValue -Value ([string] $result['terminal_status']) -Allowed $allowedRunStatuses
Assert-AllowedValue -Value ([string] $result['formal_route']) -Allowed @('do_nothing')
$expectedSourceKind = if ($Source -eq 'e0') { 'e0' } else { 'original_bank' }
if ([string] $result['source_kind'] -ne $expectedSourceKind) {
    throw 'Engine result source does not match the requested source; raw output is suppressed.'
}
$discoveryCases = Get-NonNegativeInteger -Object $result -Name 'discovery_case_count'
$holdout = $result['e0_recurrence_holdout']
$holdoutStatus = if ($null -ne $holdout) { [string] $holdout['status'] } else { '' }
if ($Source -eq 'e0') {
    if (($result.Keys -notcontains 'excluded_replay_case_count') -or ($null -ne $result['excluded_replay_case_count'])) {
        throw 'Engine result contains an unsafe E0 replay count; raw result values are suppressed.'
    }
    $excludedReplayCases = 'suppressed'
    $portfolio = $result['local_simulation_portfolio']
    if (($portfolio -isnot [System.Collections.IDictionary]) -or
        ([string] $portfolio['source_family'] -ne 'e0') -or
        ([string] $portfolio['authority'] -ne 'simulator_only')) {
        throw 'Engine result is missing the E0 simulator portfolio; raw result values are suppressed.'
    }
    $portfolioStatus = [string] $portfolio['status']
    Assert-AllowedValue -Value $portfolioStatus -Allowed @('candidates_ready', 'no_qualifying_signals', 'insufficient_evidence')
    $allowedMetrics = @('e0_technical_error_rate', 'e0_tool_retry_case_rate', 'e0_recurring_copilot_query_cases')
    $signalByMetric = @{}
    foreach ($signal in @($result['signals'])) {
        if ($signal -isnot [System.Collections.IDictionary]) {
            throw 'Engine result does not match E0 signals and portfolio; raw result values are suppressed.'
        }
        $metricId = [string] $signal['metric_id']
        if ($metricId -notin $allowedMetrics -or $signalByMetric.ContainsKey($metricId) -or
            [string]::IsNullOrWhiteSpace([string] $signal['digest'])) {
            throw 'Engine result does not match E0 signals and portfolio; raw result values are suppressed.'
        }
        $signalByMetric[$metricId] = [string] $signal['digest']
    }
    if ($result.Keys -notcontains 'recurrence_measurement_status') {
        throw 'Engine result does not match E0 signals and portfolio; raw result values are suppressed.'
    }
    $recurrenceMeasurementStatus = [string] $result['recurrence_measurement_status']
    Assert-AllowedValue -Value $recurrenceMeasurementStatus -Allowed @('observed', 'source_table_unavailable')
    if (-not $signalByMetric.ContainsKey('e0_technical_error_rate') -or
        -not $signalByMetric.ContainsKey('e0_tool_retry_case_rate') -or
        ($recurrenceMeasurementStatus -eq 'observed' -and -not $signalByMetric.ContainsKey('e0_recurring_copilot_query_cases')) -or
        ($recurrenceMeasurementStatus -eq 'source_table_unavailable' -and $signalByMetric.ContainsKey('e0_recurring_copilot_query_cases'))) {
        throw 'Engine result does not match E0 signals and portfolio; raw result values are suppressed.'
    }
    $portfolioCounts = @{
        candidate_for_simulated_investigation = 0
        not_qualified = 0
        insufficient_evidence = 0
        unavailable = 0
    }
    $portfolioDispositions = @($portfolio['dispositions'])
    if ($portfolioDispositions.Count -eq 0) {
        throw 'Engine result has no E0 portfolio dispositions; raw result values are suppressed.'
    }
    $seenDispositionMetrics = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $candidateDigests = [System.Collections.Generic.List[string]]::new()
    $unavailableCount = 0
    foreach ($disposition in $portfolioDispositions) {
        if ($disposition -isnot [System.Collections.IDictionary]) {
            throw 'Engine result has an invalid E0 portfolio disposition; raw result values are suppressed.'
        }
        $metricId = [string] $disposition['metric_id']
        Assert-AllowedValue -Value $metricId -Allowed $allowedMetrics
        if (-not $seenDispositionMetrics.Add($metricId)) {
            throw 'Engine result does not match E0 signals and portfolio; raw result values are suppressed.'
        }
        $state = [string] $disposition['state']
        Assert-AllowedValue -Value $state -Allowed @('candidate_for_simulated_investigation', 'not_qualified', 'insufficient_evidence', 'unavailable')
        $digest = $disposition['signal_digest']
        if ($state -eq 'unavailable') {
            if ($metricId -ne 'e0_recurring_copilot_query_cases' -or $null -ne $digest -or $signalByMetric.ContainsKey($metricId)) {
                throw 'Engine result does not match E0 signals and portfolio; raw result values are suppressed.'
            }
            $unavailableCount++
        }
        elseif (-not $signalByMetric.ContainsKey($metricId) -or [string] $digest -cne $signalByMetric[$metricId]) {
            throw 'Engine result does not match E0 signals and portfolio; raw result values are suppressed.'
        }
        if ($state -eq 'candidate_for_simulated_investigation') {
            $candidateDigests.Add([string] $digest)
        }
        $portfolioCounts[$state]++
    }
    if ($seenDispositionMetrics.Count -ne ($signalByMetric.Count + $unavailableCount)) {
        throw 'Engine result does not match E0 signals and portfolio; raw result values are suppressed.'
    }
    if (($recurrenceMeasurementStatus -eq 'source_table_unavailable' -and $unavailableCount -ne 1) -or
        ($recurrenceMeasurementStatus -eq 'observed' -and $unavailableCount -ne 0)) {
        throw 'Engine result does not match E0 signals and portfolio; raw result values are suppressed.'
    }
    $declaredCandidates = @($portfolio['candidate_signal_digests'])
    if ($declaredCandidates.Count -ne $candidateDigests.Count) {
        throw 'Engine result does not match E0 signals and portfolio; raw result values are suppressed.'
    }
    for ($index = 0; $index -lt $candidateDigests.Count; $index++) {
        if ([string] $declaredCandidates[$index] -cne $candidateDigests[$index]) {
            throw 'Engine result does not match E0 signals and portfolio; raw result values are suppressed.'
        }
    }
    if (($result['signal'] -isnot [System.Collections.IDictionary]) -or
        [string]::IsNullOrWhiteSpace([string] $result['signal']['digest']) -or
        [string] $portfolio['primary_signal_digest'] -cne [string] $result['signal']['digest'] -or
        [string] $portfolio['primary_signal_digest'] -cnotin @($signalByMetric.Values)) {
        throw 'Engine result does not match E0 signals and portfolio; raw result values are suppressed.'
    }
    $candidateCount = $portfolioCounts['candidate_for_simulated_investigation']
    $incompleteCount = $portfolioCounts['insufficient_evidence'] + $portfolioCounts['unavailable']
    if (($portfolioStatus -eq 'candidates_ready' -and $candidateCount -eq 0) -or
        ($portfolioStatus -eq 'no_qualifying_signals' -and ($candidateCount -gt 0 -or $incompleteCount -gt 0)) -or
        ($portfolioStatus -eq 'insufficient_evidence' -and ($candidateCount -gt 0 -or $incompleteCount -eq 0))) {
        throw 'Engine result has an inconsistent E0 portfolio status; raw result values are suppressed.'
    }
    $portfolioSummary = "status=$portfolioStatus; candidates=$($portfolioCounts['candidate_for_simulated_investigation']); not_qualified=$($portfolioCounts['not_qualified']); insufficient=$($portfolioCounts['insufficient_evidence']); unavailable=$($portfolioCounts['unavailable'])"
}
else {
    if ($null -ne $result['local_simulation_portfolio']) {
        throw 'Original-bank snapshot runs must not report an E0 portfolio; raw result values are suppressed.'
    }
    if ($null -ne $holdout) {
        throw 'Original-bank runs must not report E0 recurrence holdout results.'
    }
    if (@($result['signals']).Count -gt 0 -or $null -ne $result['proposal']) {
        throw 'Original-bank snapshot runs must not report E0 signals or executable proposals.'
    }
    $excludedReplayCases = Get-NonNegativeInteger -Object $result -Name 'excluded_replay_case_count'
}

$metricLines = [System.Collections.Generic.List[string]]::new()
if ($Source -eq 'e0' -and $null -ne $result['signals']) {
    foreach ($metric in $result['signals']) {
        $metricId = [string] $metric['metric_id']
        Assert-AllowedValue -Value $metricId -Allowed @('e0_technical_error_rate', 'e0_tool_retry_case_rate', 'e0_recurring_copilot_query_cases')
        $numerator = Get-NonNegativeInteger -Object $metric -Name 'numerator'
        $denominator = Get-NonNegativeInteger -Object $metric -Name 'denominator'
        $missing = Get-NonNegativeInteger -Object $metric -Name 'missing'
        $metricLines.Add("  $($metricId): $numerator/$denominator; missing=$missing")
    }
}

$proposalSummary = 'none'
if ($Source -eq 'e0' -and $null -ne $result['proposal']) {
    $proposalStatus = [string] $result['proposal']['status']
    $executionStatus = [string] $result['proposal']['execution_status']
    Assert-AllowedValue -Value $proposalStatus -Allowed @('simulated_unverified')
    Assert-AllowedValue -Value $executionStatus -Allowed @('not_executed')
    $proposalSummary = "status=$proposalStatus; execution=$executionStatus"
}
elseif ($Source -eq 'original') {
    $snapshotEnvelope = $result['snapshot_descriptive_envelope']
    if ($null -ne $snapshotEnvelope) {
        Assert-AllowedValue -Value ([string] $snapshotEnvelope['agent_core_candidate']) -Allowed @('dependency_blocked_snapshot_semantics')
        $finding = $snapshotEnvelope['finding']
        Assert-AllowedValue -Value ([string] $finding['coverage']) -Allowed @('partial')
        Assert-AllowedValue -Value ([string] $finding['temporal_basis']) -Allowed @('literal_source_wall_clock_month')
        Assert-AllowedValue -Value ([string] $finding['value_semantics']) -Allowed @('final_extract_facts_only')
        $snapshotProposal = $snapshotEnvelope['proposal']
        Assert-AllowedValue -Value ([string] $snapshotProposal['status']) -Allowed @('simulated_unverified')
        Assert-AllowedValue -Value ([string] $snapshotProposal['execution_status']) -Allowed @('not_executed')
        Assert-AllowedValue -Value ([string] $snapshotProposal['formal_route']) -Allowed @('do_nothing')
        if ($snapshotProposal['publication_eligible'] -ne $false) {
            throw 'Original-bank snapshot draft must remain non-publishable.'
        }
        $proposalSummary = 'descriptive_status=simulated_unverified; execution=not_executed; publication_eligible=false; agent_core=dependency_blocked_snapshot_semantics'
    }
}

$holdoutSummary = 'none'
if ($null -ne $holdout) {
    Assert-AllowedValue -Value $holdoutStatus -Allowed @('replicated', 'not_observed', 'insufficient_support', 'unavailable')
    if ($holdoutStatus -eq 'unavailable') {
        $holdoutSummary = 'status=unavailable; descriptive_only'
    }
    elseif ($holdoutStatus -eq 'insufficient_support') {
        foreach ($field in @('reproduction_case_count', 'queried_case_count', 'matching_case_count', 'recurrence_rate_basis_points')) {
            if (($holdout.Keys -notcontains $field) -or ($null -ne $holdout[$field])) {
                throw 'Engine result contains an unsafe low-support holdout aggregate; raw result values are suppressed.'
            }
        }
        $holdoutSummary = 'status=insufficient_support; counts=suppressed; descriptive_only'
    }
    else {
        $queried = Get-NonNegativeInteger -Object $holdout -Name 'queried_case_count'
        $matching = Get-NonNegativeInteger -Object $holdout -Name 'matching_case_count'
        if ($matching -gt $queried) {
            throw 'Engine result contains an invalid holdout aggregate; raw result values are suppressed.'
        }
        $holdoutSummary = "status=$holdoutStatus; matches=$matching/$queried; descriptive_only"
    }
}

Write-Output "Source: $expectedSourceKind"
Write-Output "Status: $($result['terminal_status'])"
if ($Source -eq 'e0') {
    Write-Output "Cases: discovery=$discoveryCases; replay_excluded=$excludedReplayCases"
    Write-Output "Portfolio: $portfolioSummary"
    Write-Output 'Metrics:'
    if ($metricLines.Count -eq 0) {
        Write-Output '  none'
    }
    else {
        foreach ($line in $metricLines) {
            Write-Output $line
        }
    }
    Write-Output "Proposal: $proposalSummary"
    Write-Output "Holdout: $holdoutSummary"
}
else {
    Write-Output 'Scope: final-extract descriptive snapshot; partial coverage; no causal or outcome claim'
    Write-Output "Descriptive draft: $proposalSummary"
}
Write-Output "Formal route: $($result['formal_route'])"
