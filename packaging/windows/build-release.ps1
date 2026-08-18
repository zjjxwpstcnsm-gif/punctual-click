param(
    [string]$Version = "0.1.0-alpha.5",
    [string]$DistDir = ""
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
if ([string]::IsNullOrWhiteSpace($DistDir)) {
    $Dist = Join-Path $Root "dist"
} else {
    $Dist = [System.IO.Path]::GetFullPath($DistDir)
}
$Runtime = Join-Path $Dist "runtime-windows"
$PortableRoot = Join-Path $Dist "Punctual-windows-x64"

Set-Location $Root
cargo build --release --locked -p punctual-app
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
$AppExe = Join-Path $Root "target/release/punctual-app.exe"
if (-not (Test-Path $AppExe)) { throw "Punctual executable was not produced" }

Remove-Item -Recurse -Force $Runtime, $PortableRoot -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Runtime, $PortableRoot | Out-Null

$ChromeManifest = Invoke-RestMethod -Uri "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json"
$Stable = $ChromeManifest.channels.Stable
$ChromeAsset = $Stable.downloads.chrome | Where-Object { $_.platform -eq "win64" } | Select-Object -First 1
if (-not $ChromeAsset) { throw "Chrome for Testing win64 asset was not found" }
$ChromeZip = Join-Path $Runtime "chrome-win64.zip"
Invoke-WebRequest -Uri $ChromeAsset.url -OutFile $ChromeZip
Expand-Archive -Path $ChromeZip -DestinationPath (Join-Path $Runtime "chrome") -Force

$GeckoVersion = if ($env:PUNCTUAL_GECKODRIVER_VERSION) { $env:PUNCTUAL_GECKODRIVER_VERSION } else { "0.37.1" }
$GeckoRelease = Invoke-RestMethod -Uri "https://api.github.com/repos/mozilla/geckodriver/releases/tags/v$GeckoVersion"
$GeckoName = "geckodriver-v$GeckoVersion-win64.zip"
$GeckoAsset = $GeckoRelease.assets | Where-Object { $_.name -eq $GeckoName } | Select-Object -First 1
if (-not $GeckoAsset) { throw "geckodriver win64 asset was not found" }
$GeckoZip = Join-Path $Runtime $GeckoName
Invoke-WebRequest -Uri $GeckoAsset.browser_download_url -OutFile $GeckoZip
if ($GeckoAsset.digest -and $GeckoAsset.digest.StartsWith("sha256:")) {
    $Expected = $GeckoAsset.digest.Substring(7).ToLowerInvariant()
    $Actual = (Get-FileHash -Algorithm SHA256 $GeckoZip).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) { throw "geckodriver SHA-256 mismatch" }
}
Expand-Archive -Path $GeckoZip -DestinationPath (Join-Path $Runtime "gecko") -Force

$Resources = Join-Path $PortableRoot "resources"
New-Item -ItemType Directory -Force -Path (Join-Path $Resources "managed-browser"), (Join-Path $Resources "bin") | Out-Null
Copy-Item $AppExe (Join-Path $PortableRoot "Punctual.exe")
Copy-Item -Recurse -Force (Join-Path $Runtime "chrome/chrome-win64") (Join-Path $Resources "managed-browser/chrome-win64")
Copy-Item (Join-Path $Runtime "gecko/geckodriver.exe") (Join-Path $Resources "bin/geckodriver.exe")
Set-Content -NoNewline -Encoding ascii (Join-Path $Resources "managed-browser/version.txt") $Stable.version

@"
Punctual $Version

Double-click Punctual.exe to run. The package contains a managed fallback browser.
Installed Google Chrome remains the first automatic choice.
This Alpha build is unsigned; Windows SmartScreen may show an unknown publisher warning.
"@ | Set-Content -Encoding utf8 (Join-Path $PortableRoot "README.txt")

$PortableZip = Join-Path $Dist "Punctual-$Version-windows-x64-portable.zip"
Remove-Item -Force $PortableZip -ErrorAction SilentlyContinue
Compress-Archive -Path "$PortableRoot/*" -DestinationPath $PortableZip -CompressionLevel Optimal

$Iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (-not (Test-Path $Iscc)) {
    if (-not (Get-Command choco -ErrorAction SilentlyContinue)) {
        throw "Inno Setup 6 is required to build the installer"
    }
    choco install innosetup -y --no-progress
}
if (-not (Test-Path $Iscc)) { throw "Inno Setup compiler was not found" }

$OutputDir = $Dist
$IssPath = Join-Path $Dist "punctual.iss"
@"
#define MyAppName "Punctual"
#define MyAppVersion "$Version"
#define MyAppPublisher "Punctual"
#define MyAppExeName "Punctual.exe"
[Setup]
AppId={{A545F0B0-5389-47D5-A4B9-6A90B2C93E51}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={localappdata}\Programs\Punctual
DefaultGroupName=Punctual
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=$OutputDir
OutputBaseFilename=Punctual-$Version-windows-x64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#MyAppExeName}
[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked
[Files]
Source: "$PortableRoot\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
[Icons]
Name: "{autoprograms}\Punctual"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\Punctual"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon
[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch Punctual"; Flags: nowait postinstall skipifsilent
"@ | Set-Content -Encoding utf8 $IssPath

& $Iscc $IssPath
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }
$SetupExe = Join-Path $Dist "Punctual-$Version-windows-x64-setup.exe"
if (-not (Test-Path $SetupExe)) { throw "Installer was not produced" }

$Checksums = Join-Path $Dist "Punctual-$Version-windows-x64-SHA256SUMS.txt"
$SetupHash = (Get-FileHash -Algorithm SHA256 $SetupExe).Hash.ToLowerInvariant()
$PortableHash = (Get-FileHash -Algorithm SHA256 $PortableZip).Hash.ToLowerInvariant()
@(
    "$SetupHash  $(Split-Path $SetupExe -Leaf)",
    "$PortableHash  $(Split-Path $PortableZip -Leaf)"
) | Set-Content -Encoding ascii $Checksums

Write-Host "Created $SetupExe"
Write-Host "Created $PortableZip"
