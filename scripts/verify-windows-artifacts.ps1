[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$InstallerPath,
  [string]$SevenZipPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($PSVersionTable.PSVersion.Major -lt 7) { throw "PowerShell 7 or newer is required." }
if (
  -not $IsWindows -or
  [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne [System.Runtime.InteropServices.Architecture]::X64 -or
  [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne [System.Runtime.InteropServices.Architecture]::X64
) {
  throw "Native x64 Windows is required."
}
$nodeVersionOutput = & node --version
$nodeVersionExit = $LASTEXITCODE
$nodeVersion = "$nodeVersionOutput".Trim()
if ($nodeVersionExit -ne 0 -or $nodeVersion -notmatch '^v24\.') { throw "Node 24 is required; got '$nodeVersion'." }
$nodeHostOutput = & node -p "process.platform + ':' + process.arch"
$nodeHostExit = $LASTEXITCODE
$nodeHost = "$nodeHostOutput".Trim()
if ($nodeHostExit -ne 0 -or $nodeHost -ne "win32:x64") { throw "Native x64 Node 24 is required; got '$nodeHost'." }

$installer = Get-Item -LiteralPath $InstallerPath
if ($installer.Length -le 0) { throw "Installer is empty: $($installer.FullName)" }

# A release build (updater key present) must ship the installer's minisign
# signature next to it; latest.json carries its content.
$hasUpdaterKey = -not [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY)
if ($hasUpdaterKey) {
  $signaturePath = "$($installer.FullName).sig"
  if (-not (Test-Path -LiteralPath $signaturePath -PathType Leaf) -or (Get-Item -LiteralPath $signaturePath).Length -le 0) {
    throw "The updater signature is missing next to the installer: $signaturePath"
  }
  $signatureText = [System.Text.Encoding]::UTF8.GetString(
    [Convert]::FromBase64String((Get-Content -LiteralPath $signaturePath -Raw).Trim())
  )
  if (-not $signatureText.StartsWith("untrusted comment:")) {
    throw "The updater signature is not a minisign signature: $signaturePath"
  }
}

if (-not $SevenZipPath) {
  $command = Get-Command 7z.exe -ErrorAction SilentlyContinue
  if ($command) { $SevenZipPath = $command.Source }
  elseif ($env:ProgramFiles) { $SevenZipPath = Join-Path $env:ProgramFiles "7-Zip\7z.exe" }
}
if (-not $SevenZipPath -or -not (Test-Path -LiteralPath $SevenZipPath -PathType Leaf)) {
  throw "A usable 7-Zip executable is required."
}

$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("Clovy Windows payload " + [guid]::NewGuid())
try {
  New-Item -ItemType Directory -Path $tempRoot | Out-Null
  $installerRoot = Join-Path $tempRoot "installer contents"
  New-Item -ItemType Directory -Path $installerRoot | Out-Null
  & $SevenZipPath x "-o$installerRoot" -y -- $installer.FullName | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "7-Zip could not extract the NSIS installer." }

  $archives = @(Get-ChildItem -LiteralPath $installerRoot -Recurse -File -Filter "app-*.7z")
  $payloadRoots = @($installerRoot)
  if ($archives.Count -gt 0) {
    for ($index = 0; $index -lt $archives.Count; $index++) {
      $archiveRoot = Join-Path $tempRoot ("nested payload {0}" -f $index)
      New-Item -ItemType Directory -Path $archiveRoot | Out-Null
      & $SevenZipPath x "-o$archiveRoot" -y -- $archives[$index].FullName | Out-Null
      if ($LASTEXITCODE -ne 0) { throw "7-Zip could not extract $($archives[$index].FullName)." }
      $payloadRoots += $archiveRoot
    }
  }

  $items = @($payloadRoots | ForEach-Object { Get-ChildItem -LiteralPath $_ -Recurse })
  $files = @($items | Where-Object { -not $_.PSIsContainer })
  function Require-One([string]$Name) {
    $matches = @($files | Where-Object { $_.Name -ieq $Name })
    if ($matches.Count -ne 1) { throw "Expected exactly one $Name in the final payload; found $($matches.Count)." }
    if ($matches[0].Length -le 0) { throw "Payload file is empty: $($matches[0].FullName)" }
    return $matches[0]
  }

  $app = Require-One "clovy.exe"
  $relay = Require-One "clovy-mcp.exe"
  $relayManifest = Require-One "clovy-mcp-manifest.json"
  if (-not $relay.DirectoryName.Replace('\', '/').ToLowerInvariant().EndsWith('/native/bin') -or
      $relayManifest.DirectoryName -ne $relay.DirectoryName) { throw "MCP relay and provenance must be under native/bin." }
  $stagedRelay = Join-Path $PSScriptRoot "../.tauri-helper/mcp-windows/clovy-mcp.exe"
  if ((Get-FileHash -LiteralPath $stagedRelay -Algorithm SHA256).Hash -ne
      (Get-FileHash -LiteralPath $relay.FullName -Algorithm SHA256).Hash) { throw "Packaged MCP relay differs from the staged signed Cargo artifact." }
  & node (Join-Path $PSScriptRoot "prepare-windows-mcp.mjs") --check --target x86_64-pc-windows-msvc --profile release --executable $relay.FullName --manifest $relayManifest.FullName
  if ($LASTEXITCODE -ne 0) { throw "Final MCP relay provenance verification failed." }
  # The MSVC target statically links WebView2Loader; only GNU builds ship its DLL.
  $helper = Require-One "clovy-dictation-helper.exe"
  $nativeBin = $helper.DirectoryName.Replace('\', '/').ToLowerInvariant()
  if (-not $nativeBin.EndsWith('/native/bin')) { throw "Native helpers are not under native/bin." }

  $browserEngine = Require-One "clovy-agent-browser.exe"
  $chrome = Require-One "chrome.exe"
  $browserPlatformRoot = $browserEngine.DirectoryName.Replace('\', '/').ToLowerInvariant()
  if (-not $browserPlatformRoot.EndsWith('/native/agent-browser/win64')) {
    throw "Private browser engine is not under native/agent-browser/win64."
  }
  $browserResourceRoot = Split-Path -Parent $browserEngine.DirectoryName
  & node (Join-Path $PSScriptRoot "prepare-agent-browser.mjs") --release --check --platform win64 --resource-root $browserResourceRoot
  if ($LASTEXITCODE -ne 0) { throw "Final pinned browser payload verification failed." }

  foreach ($item in $items) {
    if ($item.Name -like 'clovy-agent-runtime*') {
      throw "Retired agent runtime payload found: $($item.FullName)"
    }
    $relative = $item.FullName.Replace('\', '/').ToLowerInvariant()
    if ($relative.Contains('/native/hermes/') -or $relative.EndsWith('/native/hermes') -or
        $item.Name -ieq 'python.exe' -or $item.Extension -ieq '.pyd' -or
        $relative.Contains('hermes-agent')) {
      throw "Legacy Hermes or Python payload found: $($item.FullName)"
    }
  }

  $desktopEngines = @($files | Where-Object { $_.Name -ieq "agent-desktop.exe" -and $_.DirectoryName.Replace('\', '/').ToLowerInvariant().EndsWith('/native/bin/computer-use') })
  if ($desktopEngines.Count -ne 1) { throw "Expected one nested Computer use engine." }
  $desktopEngine = $desktopEngines[0]
  $desktopHelper = Get-Item -LiteralPath (Join-Path $desktopEngine.DirectoryName "clovy-computer-use-driver.exe")
  $stagedHelper = Join-Path $PSScriptRoot "../.tauri-helper/computer-use/clovy-computer-use-driver.exe"
  $stagedHelperHash = (Get-FileHash -LiteralPath $stagedHelper -Algorithm SHA256).Hash
  $packagedHelperHash = (Get-FileHash -LiteralPath $desktopHelper.FullName -Algorithm SHA256).Hash
  if ($stagedHelperHash -ne $packagedHelperHash) { throw "Bundling changed the staged signed helper bytes after the host digest was compiled." }
  $manifest = Get-Content -LiteralPath (Join-Path $desktopEngine.DirectoryName "agent-desktop-manifest.json") -Raw | ConvertFrom-Json
  $pin = Get-Content -LiteralPath (Join-Path $PSScriptRoot "../src-tauri/agent-desktop-pin.json") -Raw | ConvertFrom-Json
  foreach ($field in @("sourceCommit", "protocolVersion", "cargoLockSha256", "licenseSha256", "patchSha256", "sourceReviewSha256")) {
    if ($manifest.$field -ne $pin.$field) { throw "Nested engine manifest $field mismatch." }
  }
  if ($manifest.target -ne "x86_64-pc-windows-msvc" -or $manifest.executable -ne "agent-desktop.exe") { throw "Nested engine target mismatch." }
  $engineHash = (Get-FileHash -LiteralPath $desktopEngine.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($engineHash -ne $manifest.executableSha256) { throw "Nested engine executable digest mismatch." }
  $licenseHash = (Get-FileHash -LiteralPath (Join-Path $desktopEngine.DirectoryName "LICENSE") -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($licenseHash -ne $pin.licenseSha256) { throw "Nested engine license digest mismatch." }
  $reviewHash = (Get-FileHash -LiteralPath (Join-Path $desktopEngine.DirectoryName "source-review.md") -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($reviewHash -ne $pin.sourceReviewSha256) { throw "Nested engine source qualification digest mismatch." }
  $publisher = $null
  foreach ($signedFile in @($installer, $app, $relay, $desktopHelper, $desktopEngine)) {
    $signature = Get-AuthenticodeSignature -LiteralPath $signedFile.FullName
    if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
      throw "Expected a valid Authenticode signature: $($signedFile.FullName)"
    }
    $thumbprint = $signature.SignerCertificate.Thumbprint
    if ($publisher -and $thumbprint -ne $publisher) { throw "Computer use and Clovy must have the same signing publisher." }
    $publisher = $thumbprint
  }
  $updaterNote = if ($hasUpdaterKey) { " with its updater signature" } else { "" }
  Write-Host "Verified signed Windows installer and Computer use payload$($updaterNote): $($installer.FullName)"
}
finally {
  if (Test-Path -LiteralPath $tempRoot) { Remove-Item -LiteralPath $tempRoot -Recurse -Force }
}
