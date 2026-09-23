# Gives the installed Orbom package identity (a package with external location) so it shows up in
# app pickers that only list packaged apps, like the pen shortcut button's "Open an app".
# Run after installing Orbom. Asks for administrator rights once, to trust the self-signed cert.
# ponytail: self-signed and local to this PC; a purchased code-signing cert would drop the admin step.
param([switch]$Unregister)
$ErrorActionPreference = 'Stop'

Get-AppxPackage -Name Orbom | Remove-AppxPackage
if ($Unregister) { return }

$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Orbom'
if (-not (Test-Path (Join-Path $installDir 'Orbom.exe'))) { throw "Install Orbom first ($installDir)." }

$subject = 'CN=Orbom Local'
$cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object Subject -eq $subject | Select-Object -First 1
if (-not $cert) {
    $cert = New-SelfSignedCertificate -Type Custom -Subject $subject -KeyUsage DigitalSignature `
        -CertStoreLocation Cert:\CurrentUser\My -NotAfter (Get-Date).AddYears(10) `
        -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
}
$trusted = Get-ChildItem Cert:\LocalMachine\TrustedPeople | Where-Object Thumbprint -eq $cert.Thumbprint
if (-not $trusted) {
    $cer = Join-Path $env:TEMP 'orbom-local.cer'
    Export-Certificate -Cert $cert -FilePath $cer | Out-Null
    $p = Start-Process powershell -Verb RunAs -Wait -PassThru -ArgumentList `
        "-NoProfile -Command Import-Certificate -FilePath '$cer' -CertStoreLocation Cert:\LocalMachine\TrustedPeople"
    if ($p.ExitCode -ne 0) { throw 'Trusting the certificate failed.' }
}

$sdk = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" |
    Sort-Object FullName -Descending | Select-Object -First 1
if (-not $sdk) { throw 'Windows SDK (makeappx.exe) is required.' }
$bin = $sdk.DirectoryName

$work = Join-Path $env:TEMP 'orbom-package'
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory $work | Out-Null
Copy-Item (Join-Path $PSScriptRoot 'AppxManifest.xml') $work
# Package logos are the exe's own orb icon at each size the shell asks for (targetsize-N, plus the
# unplated variants lists and the taskbar use); resources.pri lets the shell find them by size.
Add-Type -AssemblyName System.Drawing
Add-Type -Namespace Win32 -Name Icons -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern uint PrivateExtractIcons(string file, int index, int cx, int cy, IntPtr[] icons, uint[] ids, uint count, uint flags);
'@
foreach ($size in 16, 24, 32, 48, 256) {
    $h = [IntPtr[]]::new(1)
    [Win32.Icons]::PrivateExtractIcons((Join-Path $installDir 'Orbom.exe'), 0, $size, $size, $h, $null, 1, 0) | Out-Null
    $png = [Drawing.Icon]::FromHandle($h[0]).ToBitmap()
    foreach ($name in "logo.targetsize-$size.png", "logo.targetsize-${size}_altform-unplated.png") {
        $png.Save((Join-Path $work $name), [Drawing.Imaging.ImageFormat]::Png)
    }
    if ($size -eq 256) { $png.Save((Join-Path $work 'logo.png'), [Drawing.Imaging.ImageFormat]::Png) }
}
Push-Location $work
& "$bin\makepri.exe" createconfig /cf priconfig.xml /dq en-US /o | Out-Null
& "$bin\makepri.exe" new /pr . /cf priconfig.xml /of resources.pri /o | Out-Null
$priOk = $LASTEXITCODE -eq 0
Remove-Item priconfig.xml
Pop-Location
if (-not $priOk) { throw 'makepri failed.' }

$msix = Join-Path $env:TEMP 'Orbom.msix'
& "$bin\makeappx.exe" pack /o /nv /d $work /p $msix | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'makeappx failed.' }
& "$bin\signtool.exe" sign /q /fd SHA256 /sha1 $cert.Thumbprint $msix
if ($LASTEXITCODE -ne 0) { throw 'signtool failed.' }

Add-AppxPackage -Path $msix -ExternalLocation $installDir
Get-AppxPackage -Name Orbom | Select-Object Name, Version, InstallLocation
