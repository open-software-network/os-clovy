param(
  [Parameter(Mandatory = $true)]
  [string]$TargetPath,
  [string]$ExpectedThumbprint
)

$ErrorActionPreference = "Stop"
if (!(Test-Path -LiteralPath $TargetPath -PathType Leaf)) {
  throw "[windows-authenticode] TargetPath does not point to a readable file: $TargetPath"
}
$signature = Get-AuthenticodeSignature -LiteralPath $TargetPath
if ($signature.Status -ne "Valid" -or $null -eq $signature.SignerCertificate) {
  throw "[windows-authenticode] Invalid Authenticode signature for $TargetPath : $($signature.Status)"
}
$thumbprint = $signature.SignerCertificate.Thumbprint
if ([string]::IsNullOrWhiteSpace($thumbprint)) {
  throw "[windows-authenticode] Signing publisher is missing for $TargetPath"
}
if (
  ![string]::IsNullOrWhiteSpace($ExpectedThumbprint) -and
  ![string]::Equals($thumbprint, $ExpectedThumbprint, [System.StringComparison]::OrdinalIgnoreCase)
) {
  throw "[windows-authenticode] Signing publisher does not match the required publisher for $TargetPath"
}
Write-Output $thumbprint
