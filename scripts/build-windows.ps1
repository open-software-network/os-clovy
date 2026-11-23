[CmdletBinding()]
param([string]$OutputDirectory)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repoRoot "artifacts\windows-signed" }
elseif (-not [System.IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory = Join-Path $repoRoot $OutputDirectory }
$callerLocation = Get-Location
$previousPath = $env:PATH
$hadSignTool = Test-Path Env:WINDOWS_SIGNTOOL_PATH
$previousSignTool = $env:WINDOWS_SIGNTOOL_PATH
$mutexPath = [System.IO.Path]::GetFullPath($repoRoot).TrimEnd([char[]]"\/").ToUpperInvariant()
$mutexBytes = [System.Text.Encoding]::UTF8.GetBytes($mutexPath)
$mutexHasher = [System.Security.Cryptography.SHA256]::Create()
try { $mutexHash = [Convert]::ToHexString($mutexHasher.ComputeHash($mutexBytes)) }
finally { $mutexHasher.Dispose() }
$buildMutex = [System.Threading.Mutex]::new($false, "Local\ClovyWindowsBuild-$mutexHash")
$hasBuildMutex = $false

function Require-Command([string]$Name) {
  if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) { throw "Required tool is unavailable: $Name" }
}

try {
  try { $hasBuildMutex = $buildMutex.WaitOne(0) }
  catch [System.Threading.AbandonedMutexException] { $hasBuildMutex = $true }
  if (-not $hasBuildMutex) { throw "Another Windows build is active against this repository." }
  if ($PSVersionTable.PSVersion.Major -lt 7) { throw "PowerShell 7 or newer is required." }
  if (
    -not $IsWindows -or
    [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne [System.Runtime.InteropServices.Architecture]::X64 -or
    [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne [System.Runtime.InteropServices.Architecture]::X64
  ) { throw "Native x64 Windows is required." }
  foreach ($tool in @("node", "pnpm", "rustc", "cargo")) { Require-Command $tool }
  foreach ($tool in @("perl", "nasm")) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
      throw "SQLCipher requires Strawberry Perl and NASM on PATH; missing tool: $tool."
    }
  }
  $nodeVersionOutput = & node --version
  $nodeVersionExit = $LASTEXITCODE
  $nodeVersion = "$nodeVersionOutput".Trim()
  if ($nodeVersionExit -ne 0 -or $nodeVersion -notmatch '^v24\.') { throw "Node 24 is required; got '$nodeVersion'." }
  $nodeHostOutput = & node -p "process.platform + ':' + process.arch"
  $nodeHostExit = $LASTEXITCODE
  $nodeHost = "$nodeHostOutput".Trim()
  if ($nodeHostExit -ne 0 -or $nodeHost -ne "win32:x64") { throw "Native x64 Node 24 is required; got '$nodeHost'." }
  $rustDetails = & rustc -vV
  $rustExit = $LASTEXITCODE
  if ($rustExit -ne 0 -or ($rustDetails -join "`n") -notmatch '(?m)^host: x86_64-pc-windows-msvc$') {
    throw "The x86_64-pc-windows-msvc Rust host is required."
  }
  $cargoVersion = & cargo --version
  if ($LASTEXITCODE -ne 0 -or "$cargoVersion" -notmatch '^cargo ') { throw "cargo is not usable." }
  $targetTriple = "x86_64-pc-windows-msvc"
  $targetLibDirectory = & rustc --print target-libdir --target $targetTriple
  if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath "$targetLibDirectory" -PathType Container)) {
    throw "The $targetTriple Rust target is required."
  }
  if ([string]::IsNullOrWhiteSpace($env:WINDOWS_CERTIFICATE_THUMBPRINT)) {
    throw "WINDOWS_CERTIFICATE_THUMBPRINT is required: Computer use authenticates the signed helper, engine and Clovy host by publisher."
  }
  if ([string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNTOOL_PATH)) {
    $signCommand = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($signCommand) { $env:WINDOWS_SIGNTOOL_PATH = $signCommand.Source }
    else {
      $sdkRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
      $signTools = @(Get-ChildItem -LiteralPath $sdkRoot -Filter signtool.exe -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Directory.Name -eq "x64" } | Sort-Object FullName -Descending)
      if ($signTools.Count -eq 0) { throw "The Windows SDK SignTool is required for the nested Computer use executables." }
      $env:WINDOWS_SIGNTOOL_PATH = $signTools[0].FullName
    }
  }
  if (-not (Test-Path -LiteralPath $env:WINDOWS_SIGNTOOL_PATH -PathType Leaf)) { throw "WINDOWS_SIGNTOOL_PATH is not a usable executable." }
  $makensisCommand = Get-Command makensis.exe -ErrorAction SilentlyContinue
  $makensisCandidates = @()
  if (${env:ProgramFiles(x86)}) { $makensisCandidates += Join-Path ${env:ProgramFiles(x86)} "NSIS\makensis.exe" }
  if ($env:ProgramFiles) { $makensisCandidates += Join-Path $env:ProgramFiles "NSIS\makensis.exe" }
  $makensisPath = if ($makensisCommand) {
    $makensisCommand.Source
  } else {
    $makensisCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
  }
  if (-not $makensisPath) { throw "A usable NSIS makensis.exe is required." }
  $env:PATH = "$(Split-Path -Parent $makensisPath);$env:PATH"
  $sevenZipCommand = Get-Command 7z.exe -ErrorAction SilentlyContinue
  $sevenZipPath = if ($sevenZipCommand) { $sevenZipCommand.Source } else { Join-Path $env:ProgramFiles "7-Zip\7z.exe" }
  if (-not (Test-Path -LiteralPath $sevenZipPath -PathType Leaf)) { throw "A usable 7-Zip executable is required." }

  Set-Location -LiteralPath $repoRoot
  New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
  Get-ChildItem -LiteralPath $OutputDirectory -File -Filter "*-setup.exe*" | Remove-Item -Force
  $cargoMetadataOutput = & cargo metadata --manifest-path src-tauri/Cargo.toml --no-deps --format-version 1
  if ($LASTEXITCODE -ne 0) { throw "Could not resolve Cargo's target directory." }
  $cargoTargetDirectory = ("$cargoMetadataOutput" | ConvertFrom-Json).target_directory
  if ([string]::IsNullOrWhiteSpace($cargoTargetDirectory)) { throw "Cargo metadata did not report a target directory." }
  $releaseDirectory = Join-Path $cargoTargetDirectory "$targetTriple\release"
  $nsisDirectory = Join-Path $releaseDirectory "bundle\nsis"
  if (Test-Path -LiteralPath $nsisDirectory -PathType Container) {
    Get-ChildItem -LiteralPath $nsisDirectory -File -Filter "*-setup.exe*" | Remove-Item -Force
  }
  $appExecutable = Join-Path $releaseDirectory "clovy.exe"
  Remove-Item -LiteralPath $appExecutable -Force -ErrorAction SilentlyContinue
  # Cargo validates bundled browser resources while preparing the helper.
  & node scripts/prepare-agent-browser.mjs --release --target $targetTriple
  if ($LASTEXITCODE -ne 0) { throw "Browser preparation failed." }
  # Sign every nested executable before Tauri signs the host and installer.
  # A current-user certificate keeps the helper and host on one publisher.
  & node scripts/prepare-computer-use.mjs --release --target $targetTriple
  if ($LASTEXITCODE -ne 0) { throw "Computer use helper/engine preparation failed." }
  $hasUpdaterKey = -not [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY)
  $tauriArgs = @("--target", $targetTriple)
  if ($hasUpdaterKey) {
    $tauriArgs += @("--config", "src-tauri/tauri.windows.conf.json", "--config", "src-tauri/tauri.release.conf.json")
  }
  & node scripts/tauri-build.mjs @tauriArgs
  if ($LASTEXITCODE -ne 0) { throw "Signed Tauri release build failed." }

  $installers = @(Get-ChildItem -LiteralPath $nsisDirectory -File -Filter "*-setup.exe")
  if ($installers.Count -ne 1) { throw "Expected exactly one release NSIS installer; found $($installers.Count)." }
  if ($installers[0].Length -le 0) { throw "Release NSIS installer is empty." }

  if ($hasUpdaterKey) {
    & pnpm tauri signer sign $installers[0].FullName | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Signing the installer with the updater key failed." }
  }

  $stagedPath = Join-Path $OutputDirectory $installers[0].Name
  Copy-Item -LiteralPath $installers[0].FullName -Destination $stagedPath -Force
  if ($hasUpdaterKey) {
    Copy-Item -LiteralPath "$($installers[0].FullName).sig" -Destination "$stagedPath.sig" -Force
  }
  try {
    & (Join-Path $PSScriptRoot "verify-windows-artifacts.ps1") -InstallerPath $stagedPath -SevenZipPath $sevenZipPath
    if ($LASTEXITCODE -ne 0) { throw "Staged installer verification failed." }
  }
  catch {
    Remove-Item -LiteralPath $stagedPath, "$stagedPath.sig" -Force -ErrorAction SilentlyContinue
    throw
  }
  Write-Host "Windows installer: $stagedPath"
}
finally {
  try {
    $env:PATH = $previousPath
    if ($hadSignTool) { $env:WINDOWS_SIGNTOOL_PATH = $previousSignTool }
    else { Remove-Item Env:WINDOWS_SIGNTOOL_PATH -ErrorAction SilentlyContinue }
    Set-Location -LiteralPath $callerLocation
  }
  finally {
    if ($hasBuildMutex) { $buildMutex.ReleaseMutex() }
    $buildMutex.Dispose()
  }
}
