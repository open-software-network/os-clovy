[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$RepoPath,

  [Parameter(Mandatory = $true)]
  [string]$Branch,

  [Parameter(Mandatory = $true)]
  [ValidatePattern('^[0-9a-fA-F]{40}$')]
  [string]$ExpectedSha,

  [string]$EvidenceDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Get-Variable PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
  $PSNativeCommandUseErrorActionPreference = $false
}

$repo = (Resolve-Path -LiteralPath $RepoPath).Path
$ExpectedSha = $ExpectedSha.ToLowerInvariant()
$shortSha = $ExpectedSha.Substring(0, 12)
$safeBranch = $Branch -replace '[^A-Za-z0-9._-]', '-'
$repoParent = Split-Path -Parent $repo
$caller = Join-Path $repoParent "Clovy Windows Qualification"
if ($caller -eq $repo) { $caller = Join-Path $repoParent "Clovy Windows Qualification Runner" }
$outputDirectory = Join-Path $repo "artifacts\windows-signed"
if (-not $EvidenceDirectory) {
  $EvidenceDirectory = Join-Path $repo ".tmp\windows-qualification\$safeBranch\$shortSha"
} elseif (-not [System.IO.Path]::IsPathRooted($EvidenceDirectory)) {
  $EvidenceDirectory = Join-Path $repo $EvidenceDirectory
}
$EvidenceDirectory = [System.IO.Path]::GetFullPath($EvidenceDirectory)
$transcriptPath = Join-Path $EvidenceDirectory "build-transcript.txt"
$reportPath = Join-Path $EvidenceDirectory "evidence.md"
$builder = Join-Path $repo "scripts\build-windows.ps1"
$verifier = Join-Path $repo "scripts\verify-windows-artifacts.ps1"
$originalLocation = Get-Location
$mutexPath = [System.IO.Path]::GetFullPath($repo).TrimEnd([char[]]"\/").ToUpperInvariant()
$mutexBytes = [System.Text.Encoding]::UTF8.GetBytes($mutexPath)
$mutexHasher = [System.Security.Cryptography.SHA256]::Create()
try { $mutexHash = [Convert]::ToHexString($mutexHasher.ComputeHash($mutexBytes)) }
finally { $mutexHasher.Dispose() }
$buildMutex = [System.Threading.Mutex]::new($false, "Local\ClovyWindowsBuild-$mutexHash")
$hasBuildMutex = $false

function Find-Application([string]$Name, [string[]]$Candidates) {
  $command = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue
  if ($command) { return $command.Source }
  return $Candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}

$sevenZipCandidates = @()
if ($env:ProgramFiles) { $sevenZipCandidates += Join-Path $env:ProgramFiles "7-Zip\7z.exe" }
if (${env:ProgramFiles(x86)}) { $sevenZipCandidates += Join-Path ${env:ProgramFiles(x86)} "7-Zip\7z.exe" }
$sevenZip = Find-Application "7z.exe" $sevenZipCandidates
$makensisCandidates = @()
if (${env:ProgramFiles(x86)}) { $makensisCandidates += Join-Path ${env:ProgramFiles(x86)} "NSIS\makensis.exe" }
if ($env:ProgramFiles) { $makensisCandidates += Join-Path $env:ProgramFiles "NSIS\makensis.exe" }
$makensis = Find-Application "makensis.exe" $makensisCandidates

if (-not (Test-Path -LiteralPath (Join-Path $repo ".git"))) {
  throw "RepoPath is not a Git repository: $repo"
}
if (-not (Test-Path -LiteralPath $builder -PathType Leaf) -or
    -not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
  throw "RepoPath does not contain the expected Windows build scripts: $repo"
}
foreach ($tool in @("git", "but", "node", "pnpm", "rustc", "cargo")) {
  if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
    throw "Required tool is unavailable: $tool"
  }
}

New-Item -ItemType Directory -Force -Path $caller, $EvidenceDirectory | Out-Null

$originalPath = $env:PATH
$hadCertificatePassword = Test-Path Env:WINDOWS_CERTIFICATE_PASSWORD
$originalCertificatePassword = if ($hadCertificatePassword) {
  (Get-Item Env:WINDOWS_CERTIFICATE_PASSWORD).Value
} else {
  $null
}

$result = [ordered]@{
  status = "FAIL"
  branch = $Branch
  expectedSha = $ExpectedSha
  exactSha = $null
  firstBuild = $false
  firstInBuilderVerifier = $false
  firstExplicitVerifier = $false
  firstPathRestored = $false
  firstArtifact = $null
  secondBuild = $false
  secondInBuilderVerifier = $false
  secondExplicitVerifier = $false
  secondPathRestored = $false
  secondArtifact = $null
  finalPathRestored = $false
  finalCertificateEnvironmentRestored = $false
  finalContentClean = $false
  finalStatusClean = $false
  failure = $null
}

function Invoke-Checked([scriptblock]$Command, [string]$Description) {
  & $Command
  if (-not $?) { throw "$Description failed." }
}

function Get-ArtifactInfo([System.IO.FileInfo]$Installer) {
  $signature = Get-AuthenticodeSignature -LiteralPath $Installer.FullName
  [ordered]@{
    name = $Installer.Name
    sizeBytes = $Installer.Length
    sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $Installer.FullName).Hash
    authenticodeStatus = $signature.Status.ToString()
  }
}

function Get-OneStagedInstaller {
  $installers = @(Get-ChildItem -LiteralPath $outputDirectory -File -Filter "*-setup.exe")
  if ($installers.Count -ne 1) {
    throw "Expected exactly one staged installer; found $($installers.Count)."
  }
  $installers[0]
}

function Assert-GitDiffIsEmpty([string[]]$Arguments, [string]$Description) {
  & git @Arguments
  $exitCode = $LASTEXITCODE
  if ($exitCode -eq 1) { throw "$Description has a content change." }
  if ($exitCode -ne 0) { throw "Could not inspect $Description (git exit $exitCode)." }
}

function Assert-CargoTomlIsStatOnly {
  Assert-GitDiffIsEmpty @("diff", "--quiet", "--", "src-tauri/Cargo.toml") "Cargo.toml worktree"
  Assert-GitDiffIsEmpty @("diff", "--cached", "--quiet", "--", "src-tauri/Cargo.toml") "Cargo.toml index"

  $indexBlob = & git rev-parse ":src-tauri/Cargo.toml"
  if ($LASTEXITCODE -ne 0) { throw "Could not resolve the Cargo.toml index blob." }
  $worktreeBlob = & git hash-object --path=src-tauri/Cargo.toml src-tauri/Cargo.toml
  if ($LASTEXITCODE -ne 0) { throw "Could not hash the normalized Cargo.toml worktree content." }
  $indexBlob = "$indexBlob".Trim()
  $worktreeBlob = "$worktreeBlob".Trim()
  if ($indexBlob -ne $worktreeBlob) {
    throw "Cargo.toml normalized index/worktree blob IDs differ."
  }
  Write-Host "Cargo.toml normalized content is identical: $indexBlob"
}

function Assert-AllTrackedContentIsClean {
  Assert-GitDiffIsEmpty @("diff", "--quiet") "worktree"
  Assert-GitDiffIsEmpty @("diff", "--cached", "--quiet") "index"
}

function Assert-WorkspaceMatchesExpectedCommit {
  $headTree = & git rev-parse "HEAD^{tree}"
  if ($LASTEXITCODE -ne 0) { throw "Could not resolve the GitButler workspace tree." }
  $expectedTree = & git rev-parse "$ExpectedSha^{tree}"
  if ($LASTEXITCODE -ne 0) { throw "Could not resolve the expected commit tree." }
  if ("$headTree".Trim() -ne "$expectedTree".Trim()) {
    throw "The GitButler workspace tree does not match the expected commit."
  }

  $untracked = @(& git ls-files --others --exclude-standard)
  if ($LASTEXITCODE -ne 0) { throw "Could not inspect untracked repository files." }
  if ($untracked.Count -gt 0) {
    throw "The repository has non-ignored untracked files, so its build input is not exact."
  }
}

function Restore-Environment {
  $env:PATH = $originalPath
  if ($hadCertificatePassword) { $env:WINDOWS_CERTIFICATE_PASSWORD = $originalCertificatePassword }
  else { Remove-Item Env:WINDOWS_CERTIFICATE_PASSWORD -ErrorAction SilentlyContinue }
}

Start-Transcript -LiteralPath $transcriptPath -Force | Out-Null
try {
  Set-Location -LiteralPath $repo

  try { $hasBuildMutex = $buildMutex.WaitOne(0) }
  catch [System.Threading.AbandonedMutexException] { $hasBuildMutex = $true }
  if (-not $hasBuildMutex) {
    throw "Another tracked Windows build or qualification is active against this repository."
  }

  # The mutex coordinates tracked scripts. This scan also catches common
  # unmanaged build commands whose command line includes the repository path.
  $repoPattern = [regex]::Escape($repo)
  $scriptPattern = "build-windows\.ps1|verify-windows-artifacts\.ps1|qualify-windows-build\.ps1"
  $activeBuilds = @(Get-CimInstance Win32_Process | Where-Object {
      $_.ProcessId -ne $PID -and
      $_.Name -match "^(cargo|rustc|pnpm|npm|powershell|pwsh|node)\.exe$" -and
      $_.CommandLine -match $repoPattern -and
      ($_.CommandLine -match $scriptPattern -or $_.CommandLine -match "tauri|cargo|pnpm")
    })
  if ($activeBuilds.Count -gt 0) {
    $activeBuilds | Select-Object ProcessId, ParentProcessId, Name, CommandLine | Format-List
    throw "Another build or qualification process is active against this repository."
  }

  Assert-CargoTomlIsStatOnly
  & git update-index --refresh
  if ($LASTEXITCODE -ne 0) {
    Write-Warning "Git retained the known stat-only Cargo.toml marker after refresh."
  }
  Assert-AllTrackedContentIsClean

  Invoke-Checked { & git check-ref-format "refs/heads/$Branch" } "branch name validation"
  Invoke-Checked { & git fetch origin --prune } "origin fetch"
  $dryRunOutput = & but branch update $Branch --strategy pick-remote --dry-run --verbose --format agent 2>&1
  $dryRunExit = $LASTEXITCODE
  $dryRunOutput | Write-Host
  if ($dryRunExit -eq 0) {
    Invoke-Checked {
      & but branch update $Branch --strategy pick-remote --format agent
    } "GitButler branch update"
  } else {
    if ($dryRunExit -ne 1) {
      throw "GitButler dry run failed with unexpected exit code $dryRunExit."
    }
    $preUpdateBranchSha = & git rev-parse $Branch
    if ($LASTEXITCODE -ne 0) { throw "GitButler dry run failed and the local branch could not be resolved." }
    $preUpdateRemoteSha = & git rev-parse "refs/remotes/origin/$Branch"
    if ($LASTEXITCODE -ne 0) { throw "GitButler dry run failed and the remote branch could not be resolved." }
    if ("$preUpdateBranchSha".Trim() -ne $ExpectedSha -or "$preUpdateRemoteSha".Trim() -ne $ExpectedSha) {
      throw "GitButler dry run failed before the branch reached the expected remote SHA."
    }
    if (($dryRunOutput -join "`n") -notmatch "Integration steps cannot be empty") {
      throw "GitButler dry run failed for an unexpected reason."
    }
    Write-Host "GitButler dry run had no integration steps; branch is already exact."
  }

  $branchSha = & git rev-parse $Branch
  if ($LASTEXITCODE -ne 0) { throw "Could not resolve branch: $Branch" }
  $remoteSha = & git rev-parse "refs/remotes/origin/$Branch"
  if ($LASTEXITCODE -ne 0) { throw "Could not resolve origin branch: $Branch" }
  $branchSha = "$branchSha".Trim().ToLowerInvariant()
  $remoteSha = "$remoteSha".Trim().ToLowerInvariant()
  if ($branchSha -ne $ExpectedSha -or $remoteSha -ne $ExpectedSha) {
    throw "Branch SHA mismatch: branch=$branchSha remote=$remoteSha expected=$ExpectedSha"
  }
  $result.exactSha = $branchSha
  Assert-AllTrackedContentIsClean
  Assert-WorkspaceMatchesExpectedCommit

  Write-Host "Exact SHA: $branchSha"
  Write-Host "PowerShell: $($PSVersionTable.PSVersion) $($PSVersionTable.PSEdition)"
  Write-Host "Node: $(& node --version) $(& node -p `"process.platform + ':' + process.arch`")"
  Write-Host "pnpm: $(& pnpm --version)"
  & rustc -vV
  & cargo --version
  & rustup target list --installed
  Write-Host "makensis initially on PATH: $([bool](Get-Command makensis.exe -CommandType Application -ErrorAction SilentlyContinue))"
  if (-not $makensis) { throw "A usable NSIS makensis.exe installation was not found." }
  if (-not $sevenZip) { throw "A usable 7-Zip installation was not found." }
  Write-Host "makensis path: $makensis"
  & $makensis /VERSION
  Write-Host "7-Zip path: $sevenZip"
  & $sevenZip | Select-String "^7-Zip " | Select-Object -First 1

  Remove-Item Env:WINDOWS_CERTIFICATE_PASSWORD -ErrorAction SilentlyContinue
  Set-Location -LiteralPath $caller

  Write-Host "=== First full builder run ==="
  Invoke-Checked { & $builder -OutputDirectory $outputDirectory } "first builder"
  $result.firstBuild = $true
  $result.firstInBuilderVerifier = $true
  $result.firstPathRestored = $env:PATH -ceq $originalPath
  if (-not $result.firstPathRestored) { throw "First builder did not restore PATH exactly." }

  $firstInstaller = Get-OneStagedInstaller
  $result.firstArtifact = Get-ArtifactInfo $firstInstaller
  Write-Host "=== Explicit verifier after first build ==="
  Invoke-Checked {
    & $verifier -InstallerPath $firstInstaller.FullName -SevenZipPath $sevenZip
  } "first explicit verifier"
  $result.firstExplicitVerifier = $true

  Write-Host "=== Second full builder run ==="
  Invoke-Checked { & $builder -OutputDirectory $outputDirectory } "second builder"
  $result.secondBuild = $true
  $result.secondInBuilderVerifier = $true
  $result.secondPathRestored = $env:PATH -ceq $originalPath
  if (-not $result.secondPathRestored) { throw "Second builder did not restore PATH exactly." }

  $secondInstaller = Get-OneStagedInstaller
  $result.secondArtifact = Get-ArtifactInfo $secondInstaller
  Write-Host "=== Explicit verifier after second build ==="
  Invoke-Checked {
    & $verifier -InstallerPath $secondInstaller.FullName -SevenZipPath $sevenZip
  } "second explicit verifier"
  $result.secondExplicitVerifier = $true
}
catch {
  $result.failure = $_.Exception.Message
  Write-Warning $_
}
finally {
  try {
    Restore-Environment
    $result.finalPathRestored = $env:PATH -ceq $originalPath
    $result.finalCertificateEnvironmentRestored =
      ((Test-Path Env:WINDOWS_CERTIFICATE_PASSWORD) -eq $hadCertificatePassword) -and
      ((-not $hadCertificatePassword) -or ($env:WINDOWS_CERTIFICATE_PASSWORD -ceq $originalCertificatePassword))

    Set-Location -LiteralPath $repo
    try {
      Assert-CargoTomlIsStatOnly
      & git update-index --refresh
      if ($LASTEXITCODE -ne 0) {
        Write-Warning "Git retained the known stat-only Cargo.toml marker after final refresh."
      }
      Assert-AllTrackedContentIsClean
      Assert-WorkspaceMatchesExpectedCommit
      $result.finalContentClean = $true
      $porcelain = @(git status --porcelain=v2 --untracked-files=all)
      if ($LASTEXITCODE -ne 0) { throw "Could not read final Git status." }
      $result.finalStatusClean = $porcelain.Count -eq 0
      git status --short --branch
      but status
    }
    catch {
      if (-not $result.failure) { $result.failure = $_.Exception.Message }
    }

    $allPassed =
      (-not $result.failure) -and $result.exactSha -eq $ExpectedSha -and
      $result.firstBuild -and $result.firstInBuilderVerifier -and $result.firstExplicitVerifier -and
      $result.secondBuild -and $result.secondInBuilderVerifier -and $result.secondExplicitVerifier -and
      $result.firstPathRestored -and $result.secondPathRestored -and $result.finalPathRestored -and
      $result.finalCertificateEnvironmentRestored -and
      $result.finalContentClean
    $result.status = if ($allPassed) { "PASS" } else { "FAIL" }

    $firstJson = if ($result.firstArtifact) { $result.firstArtifact | ConvertTo-Json -Compress } else { "Not produced" }
    $secondJson = if ($result.secondArtifact) { $result.secondArtifact | ConvertTo-Json -Compress } else { "Not produced" }
    @"
# Native Windows build qualification

- Result: **$($result.status)**
- Branch: $Branch
- Expected SHA: $ExpectedSha
- Exact SHA: $($result.exactSha)
- First build/in-builder verifier/explicit verifier: $($result.firstBuild) / $($result.firstInBuilderVerifier) / $($result.firstExplicitVerifier)
- Second build/in-builder verifier/explicit verifier: $($result.secondBuild) / $($result.secondInBuilderVerifier) / $($result.secondExplicitVerifier)
- First PATH restoration: $($result.firstPathRestored)
- Second PATH restoration: $($result.secondPathRestored)
- Final PATH/certificate restoration: $($result.finalPathRestored) / $($result.finalCertificateEnvironmentRestored)
- Final content clean: $($result.finalContentClean)
- Final porcelain status clean: $($result.finalStatusClean)
- First artifact: $firstJson
- Second artifact: $secondJson
- Failure: $($result.failure)
- Transcript: $transcriptPath
"@ | Set-Content -LiteralPath $reportPath -Encoding utf8

    $result | ConvertTo-Json -Depth 5
  }
  finally {
    Set-Location -LiteralPath $originalLocation -ErrorAction SilentlyContinue
    Stop-Transcript -ErrorAction SilentlyContinue | Out-Null
    if ($hasBuildMutex) { $buildMutex.ReleaseMutex() }
    $buildMutex.Dispose()
  }
}

if ($result.status -ne "PASS") { exit 1 }
