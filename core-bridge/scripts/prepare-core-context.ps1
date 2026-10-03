<#
.SYNOPSIS
  Exports the pinned agent-core checkout (git archive of HEAD == pin) into a clean build context WITHOUT upstream's
  .dockerignore (agent-core 789d6c8 ignores `contracts`, `tests`, `docs`; our Dockerfile reads contracts/VERSION).
  Prints the context directory. Fails closed when HEAD != -PinSha.
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Checkout, [Parameter(Mandatory)][string]$PinSha, [string]$Out)
$ErrorActionPreference = 'Stop'
$head = (git -C $Checkout rev-parse HEAD).Trim()
if ($head -ne $PinSha) { Write-Error "pulso:image_build_failed checkout HEAD $head != pin $PinSha"; exit 2 }
if (-not $Out) { $Out = Join-Path ([IO.Path]::GetTempPath()) "pulso-core-context-$($PinSha.Substring(0,7))" }
if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
$zip = Join-Path ([IO.Path]::GetTempPath()) "pulso-core-$($PinSha.Substring(0,7)).zip"
git -C $Checkout archive --format=zip -o $zip $PinSha
if ($LASTEXITCODE -ne 0) { Write-Error 'pulso:image_build_failed git archive'; exit 2 }
Expand-Archive -Path $zip -DestinationPath $Out -Force
Remove-Item -Force $zip
Remove-Item -Force (Join-Path $Out '.dockerignore') -ErrorAction SilentlyContinue
if (-not (Test-Path (Join-Path $Out 'contracts\VERSION'))) { Write-Error 'pulso:image_build_failed contracts/VERSION missing from context'; exit 2 }
Write-Output $Out
