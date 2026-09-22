#if DecodeVer(Ver) != RequiredCompilerVersion
    #error Use the Inno Setup version pinned in upstream-lock.json.
#endif

[Setup]
AppId={{64A54F85-79F8-43D3-9B5B-2336052C370E}
AppName=qbutt
AppVersion={#AppVersion}
AppPublisher=The qbutt Project
AppPublisherURL=https://github.com/qbutt-org/qbutt
AppSupportURL=https://github.com/qbutt-org/qbutt/issues
DefaultDirName={localappdata}\Programs\qbutt
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputBaseFilename=qbutt-{#AppVersion}-windows-x64-setup
SetupIconFile={#ProjectDir}\src\icons\qbittorrent.ico
UninstallDisplayIcon={app}\qbutt.exe
VersionInfoVersion={#AppFileVersion}
VersionInfoDescription=qbutt Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#BundleDir}\*"; DestDir: "{app}"; Excludes: "\profile,\profile\*,\build-manifest.json,\upstream-lock.json"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\qbutt"; Filename: "{app}\qbutt.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\qbutt"; Filename: "{app}\qbutt.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\qbutt.exe"; Description: "{cm:LaunchProgram,qbutt}"; Flags: nowait postinstall skipifsilent

[CustomMessages]
english.PortableDirectory=This folder contains a portable qbutt profile. Choose a different installation folder.
russian.PortableDirectory=Эта папка содержит переносной профиль qbutt. Выберите другую папку для установки.

[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
    if DirExists(ExpandConstant('{app}\profile')) then
        Result := CustomMessage('PortableDirectory');
end;
