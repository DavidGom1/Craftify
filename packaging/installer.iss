; Craftify installer (Inno Setup 6). package.ps1 builds it from dist\Craftify:
;   ISCC.exe /DAppVersion=0.2.0 packaging\installer.iss
; It installs for the current user (no administrator rights), with Start menu and optional
; desktop shortcuts and an uninstaller, and warns when SolidCraft is not installed.

#ifndef AppVersion
  #error Pass the version: ISCC.exe /DAppVersion=x.y.z packaging\installer.iss
#endif

[Setup]
; Never change AppId: it is how Windows recognises an update of the same program.
AppId={{92407A08-1D5B-4096-AD47-A023F4D9E3E9}
AppName=Craftify
AppVersion={#AppVersion}
AppVerName=Craftify {#AppVersion}
AppPublisher=Hidroconta
AppPublisherURL=https://github.com/DavidGom1/Craftify
AppSupportURL=https://github.com/DavidGom1/Craftify/issues
DefaultDirName={autopf}\Craftify
DefaultGroupName=Craftify
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=Craftify-{#AppVersion}-setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\Craftify.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
es.NoSolidCraft=No se ha encontrado SolidCraft en este equipo. Craftify lo necesita para crear los diseños.%n%nInstálalo antes de convertir:%n    winget install bherbruck.SolidCraft
en.NoSolidCraft=SolidCraft was not found on this computer. Craftify needs it to build the designs.%n%nInstall it before converting:%n    winget install bherbruck.SolidCraft

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\dist\Craftify\Craftify.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\Craftify\craftify-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\Craftify\cadmpeg.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\Craftify\README.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\Craftify\NOTICE.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\Craftify\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\Craftify\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\Craftify\OFL-IBMPlex.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Craftify"; Filename: "{app}\Craftify.exe"
Name: "{group}\{cm:UninstallProgram,Craftify}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Craftify"; Filename: "{app}\Craftify.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Craftify.exe"; Description: "{cm:LaunchProgram,Craftify}"; Flags: nowait postinstall skipifsilent

[Code]
function SolidCraftInstalled(): Boolean;
begin
  Result := FileExists(ExpandConstant('{commonpf64}\SolidCraft\solidcraft-cli.exe'))
    or FileExists(ExpandConstant('{commonpf}\SolidCraft\solidcraft-cli.exe'))
    or FileExists(ExpandConstant('{localappdata}\Programs\SolidCraft\solidcraft-cli.exe'))
    or FileExists(ExpandConstant('{localappdata}\SolidCraft\solidcraft-cli.exe'));
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and not WizardSilent() and not SolidCraftInstalled() then
    MsgBox(CustomMessage('NoSolidCraft'), mbInformation, MB_OK);
end;
