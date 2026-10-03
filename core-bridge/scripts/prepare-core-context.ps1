<#
.SYNOPSIS
  Exports the pinned agent-core checkout (git archive of HEAD == pin) into a clean build context WITHOUT upstream's
  .dockerignore (agent-core 894fa65 ignores `contracts`, `tests`, `docs`; our Dockerfile reads contracts/VERSION).
  Prints the context directory. Fails closed when HEAD != -PinSha.
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Checkout, [Parameter(Mandatory)][string]$PinSha, [string]$Out)
$ErrorActionPreference = 'Stop'
$head = (git -C $Checkout rev-parse HEAD).Trim()
if ($head -ne $PinSha) { Write-Error "pulso:image_build_failed checkout HEAD $head != pin $PinSha"; exit 2 }
$run = [guid]::NewGuid().ToString('N')
if (-not $Out) { $Out = Join-Path ([IO.Path]::GetTempPath()) "pulso-core-context-$($PinSha.Substring(0,7))-$run" }
if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
# Unique zip per invocation so concurrent prepares for the same sha never collide.
$zip = Join-Path ([IO.Path]::GetTempPath()) "pulso-core-$($PinSha.Substring(0,7))-$run.zip"
try {
    git -C $Checkout archive --format=zip -o $zip $PinSha
    if ($LASTEXITCODE -ne 0) { Write-Error 'pulso:image_build_failed git archive'; exit 2 }
    Expand-Archive -Path $zip -DestinationPath $Out -Force
    Remove-Item -Force (Join-Path $Out '.dockerignore') -ErrorAction SilentlyContinue
    if (-not (Test-Path (Join-Path $Out 'contracts\VERSION'))) { Write-Error 'pulso:image_build_failed contracts/VERSION missing from context'; exit 2 }
} finally {
    Remove-Item -Force $zip -ErrorAction SilentlyContinue
}
Write-Output $Out
