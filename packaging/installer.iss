; Craftify installer (Inno Setup 6). package.ps1 builds it from dist\Craftify:
;   ISCC.exe /DAppVersion=0.2.0 packaging\installer.iss
; It installs for the current user (no administrator rights), with Start menu and optional
; desktop shortcuts and an uninstaller. SolidCraft's solidcraft-cli comes with it (folder solidcraft).

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

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\dist\Craftify\Craftify.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\Craftify\craftify-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\Craftify\cadmpeg.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\Craftify\solidcraft\*"; DestDir: "{app}\solidcraft"; Flags: ignoreversion
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
