[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Provision only the disposable hosted release runner. Local builders use an
# existing current-user certificate selected by its thumbprint.
if ($env:GITHUB_ACTIONS -ne "true" -or $env:RUNNER_ENVIRONMENT -ne "github-hosted" -or -not $IsWindows) {
  throw "Release certificate provisioning requires an ephemeral GitHub-hosted Windows runner."
}
if ([string]::IsNullOrWhiteSpace($env:WINDOWS_CERTIFICATE) -or [string]::IsNullOrWhiteSpace($env:WINDOWS_CERTIFICATE_PASSWORD)) {
  throw "WINDOWS_CERTIFICATE (base64 PFX) and WINDOWS_CERTIFICATE_PASSWORD are required for the signed Windows release."
}
if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP) -or [string]::IsNullOrWhiteSpace($env:GITHUB_ENV)) {
  throw "GitHub runner temporary and environment paths are required."
}
$certificatePath = Join-Path $env:RUNNER_TEMP ("clovy-release-" + [guid]::NewGuid() + ".pfx")
try {
  [System.IO.File]::WriteAllBytes($certificatePath, [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE))
  $password = ConvertTo-SecureString -String $env:WINDOWS_CERTIFICATE_PASSWORD -AsPlainText -Force
  $certificates = @(Import-PfxCertificate -FilePath $certificatePath -CertStoreLocation Cert:\CurrentUser\My -Password $password)
  $signers = @($certificates | Where-Object { $_.HasPrivateKey })
  if ($signers.Count -ne 1) { throw "The release PFX must contain exactly one signing identity with its private key." }
  $signer = $signers[0]
  if ($signer.NotAfter -le [DateTime]::UtcNow -or $signer.NotBefore -gt [DateTime]::UtcNow) { throw "The signing certificate is outside its validity period." }
  $thumbprint = $signer.Thumbprint
  if ($thumbprint -notmatch '^[0-9A-Fa-f]{40}$') { throw "The signing certificate has no usable thumbprint." }
  Add-Content -LiteralPath $env:GITHUB_ENV -Value "WINDOWS_CERTIFICATE_THUMBPRINT=$thumbprint" -Encoding utf8
  $sdkRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
  $signTools = @(Get-ChildItem -LiteralPath $sdkRoot -Filter signtool.exe -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Directory.Name -eq "x64" } | Sort-Object FullName -Descending)
  if ($signTools.Count -eq 0) { throw "The Windows SDK x64 SignTool is required." }
  Add-Content -LiteralPath $env:GITHUB_ENV -Value "WINDOWS_SIGNTOOL_PATH=$($signTools[0].FullName)" -Encoding utf8
  Write-Host "The Windows release signing identity is available in the ephemeral runner certificate store."
}
finally {
  if (Test-Path -LiteralPath $certificatePath) { Remove-Item -LiteralPath $certificatePath -Force }
  Remove-Item Env:WINDOWS_CERTIFICATE -ErrorAction SilentlyContinue
  Remove-Item Env:WINDOWS_CERTIFICATE_PASSWORD -ErrorAction SilentlyContinue
}
