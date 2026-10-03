<#
.SYNOPSIS
  Builds pulso-core-runtime from the pinned agent-core checkout and prints the image digest (plan 17.3.2).
  Fails closed when the checkout HEAD is not the pin. Tag: pulso-core-runtime:<core7>-<pulso7>.
#>
[CmdletBinding()]
param(
    [string]$Checkout = 'D:\.codex\factored\references\agent-core',
    [string]$Connection = 'pulso-dev',
    [string]$Podman = 'C:\Users\alexg\AppData\Local\Programs\Podman\podman.exe'
)
$ErrorActionPreference = 'Stop'
$PinSha = '86a767474042a566a0dbd6ed23588959f27ebdb3'
$head = (git -C $Checkout rev-parse HEAD).Trim()
if ($head -ne $PinSha) { Write-Error "pulso:image_build_failed checkout HEAD $head != pin $PinSha"; exit 2 }
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = Resolve-Path (Join-Path $here '..')
$pulso = (git -C $root rev-parse --short=7 HEAD).Trim()
$tag = "pulso-core-runtime:$($PinSha.Substring(0,7))-$pulso"
& $Podman --connection $Connection build --build-context "core=$Checkout" --build-arg "CORE_SHA=$PinSha" `
    --build-arg "PULSO_SHA=$pulso" -f (Join-Path $root 'Dockerfile') -t $tag $root
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$id = (& $Podman --connection $Connection image inspect $tag --format '{{.Id}}').Trim()
Write-Output "tag=$tag"
Write-Output "digest=sha256:$id"
