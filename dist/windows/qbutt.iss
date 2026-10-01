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
UninstallDisplayName=qbutt
VersionInfoVersion={#AppFileVersion}
VersionInfoDescription=qbutt Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
ChangesAssociations=yes

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

[Registry]
Root: HKA; Subkey: "Software\Classes\qbutt.Magnet"; ValueType: string; ValueName: ""; ValueData: "qbutt magnet link"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\qbutt.Magnet"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\qbutt.Magnet\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\qbutt.exe"",0"
Root: HKA; Subkey: "Software\Classes\qbutt.Magnet\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\qbutt.exe"" ""%1"""
Root: HKA; Subkey: "Software\qbutt\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "qbutt"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\qbutt\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "qbutt BitTorrent client"
Root: HKA; Subkey: "Software\qbutt\Capabilities\URLAssociations"; ValueType: string; ValueName: "magnet"; ValueData: "qbutt.Magnet"
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "qbutt"; ValueData: "Software\qbutt\Capabilities"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\qbutt.exe"; ValueType: string; ValueName: ""; ValueData: "{app}\qbutt.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\magnet"; ValueType: string; ValueName: ""; ValueData: "URL:Magnet URI"; Check: CanRegisterMagnetFallback
Root: HKA; Subkey: "Software\Classes\magnet"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""; Check: CanRegisterMagnetFallback
Root: HKA; Subkey: "Software\Classes\magnet\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\qbutt.exe"",0"; Check: CanRegisterMagnetFallback
Root: HKA; Subkey: "Software\Classes\magnet\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\qbutt.exe"" ""%1"""; Check: CanRegisterMagnetFallback

[Run]
Filename: "{app}\qbutt.exe"; Description: "{cm:LaunchProgram,qbutt}"; Flags: nowait postinstall skipifsilent; Check: not IsUpdate

Filename: "{app}\qbutt.exe"; WorkingDir: "{code:UpdateWorkingDirectory}"; Flags: nowait runascurrentuser; Check: IsUpdate

[CustomMessages]
english.PortableDirectory=This folder contains a portable qbutt profile. Choose a different installation folder.
russian.PortableDirectory=Эта папка содержит переносной профиль qbutt. Выберите другую папку для установки.

english.UpdateBusy=qbutt is still closing. Try the update again after it exits.
russian.UpdateBusy=qbutt ещё закрывается. Повторите обновление после выхода из программы.
english.UpdateInvalid=Could not update this installation. Run the installer again.
russian.UpdateInvalid=Не удалось обновить эту установку. Запустите установщик ещё раз.

[Code]
function CanRegisterMagnetFallback: Boolean;
var
    Command: String;
begin
    Result := not RegQueryStringValue(HKCR64, 'magnet\shell\open\command', '', Command)
        or (Command = '')
        or (CompareText(Command, ExpandConstant('"{app}\qbutt.exe" "%1"')) = 0);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
    Command: String;
begin
    if CurUninstallStep <> usPostUninstall then
        Exit;
    if RegQueryStringValue(HKCU64, 'Software\Classes\magnet\shell\open\command', '', Command)
        and (CompareText(Command, ExpandConstant('"{app}\qbutt.exe" "%1"')) = 0) then
    begin
        RegDeleteKeyIncludingSubkeys(HKCU64, 'Software\Classes\magnet\shell\open\command');
        RegDeleteKeyIfEmpty(HKCU64, 'Software\Classes\magnet\shell\open');
        RegDeleteKeyIfEmpty(HKCU64, 'Software\Classes\magnet\shell');
    end;
    if RegQueryStringValue(HKCU64, 'Software\Classes\magnet\DefaultIcon', '', Command)
        and (CompareText(Command, ExpandConstant('"{app}\qbutt.exe",0')) = 0) then
        RegDeleteKeyIncludingSubkeys(HKCU64, 'Software\Classes\magnet\DefaultIcon');
end;

function OpenProcess(Access: LongWord; InheritHandle: LongWord; ProcessID: LongWord): THandle;
    external 'OpenProcess@kernel32.dll stdcall';
function QueryFullProcessImageName(Process: THandle; Flags: LongWord; Name: String; var Size: LongWord): Boolean;
    external 'QueryFullProcessImageNameW@kernel32.dll stdcall';
function OpenEvent(Access: LongWord; InheritHandle: LongWord; Name: String): THandle;
    external 'OpenEventW@kernel32.dll stdcall';
function SetEvent(Handle: THandle): Boolean;
    external 'SetEvent@kernel32.dll stdcall';
function WaitForSingleObject(Handle: THandle; Milliseconds: LongWord): LongWord;
    external 'WaitForSingleObject@kernel32.dll stdcall';
function CloseHandle(Handle: THandle): Boolean;
    external 'CloseHandle@kernel32.dll stdcall';

function IsUpdate: Boolean;
begin
    Result := ExpandConstant('{param:QBUTTUPDATE|}') <> '';
end;

function UpdateWorkingDirectory(Param: String): String;
begin
    Result := ExpandConstant('{param:QBUTTWORKDIR|}');
end;

function WaitForQbutt: String;
var
    ProcessID: Integer;
    Process, Ready: THandle;
    ProcessPath, InstallPath: String;
    Size: LongWord;
begin
    Result := CustomMessage('UpdateInvalid');
    if not RegQueryStringValue(HKCU64,
        'Software\Microsoft\Windows\CurrentVersion\Uninstall\{64A54F85-79F8-43D3-9B5B-2336052C370E}_is1',
        'Inno Setup: App Path', InstallPath) then
        Exit;
    if CompareText(RemoveBackslashUnlessRoot(InstallPath), ExpandConstant('{app}')) <> 0 then
        Exit;
    if not DirExists(UpdateWorkingDirectory('')) then
        Exit;
    ProcessID := StrToIntDef(ExpandConstant('{param:QBUTTUPDATE|}'), 0);
    if ProcessID <= 0 then
        Exit;
    Process := OpenProcess($100000 or $1000, 0, ProcessID);
    if Process = 0 then
        Exit;
    try
        Size := 32768;
        SetLength(ProcessPath, Size);
        if not QueryFullProcessImageName(Process, 0, ProcessPath, Size) then
            Exit;
        SetLength(ProcessPath, Size);
        if CompareText(ProcessPath, ExpandConstant('{app}\qbutt.exe')) <> 0 then
            Exit;
        Ready := OpenEvent(2, 0, ExpandConstant('{param:QBUTTREADY|}'));
        if Ready = 0 then
            Exit;
        try
            if not SetEvent(Ready) then
                Exit;
        finally
            CloseHandle(Ready);
        end;
        { Never terminate qbutt: its normal shutdown drains and saves torrent I/O. }
        if WaitForSingleObject(Process, 120000) = 0 then
            Result := ''
        else
            Result := CustomMessage('UpdateBusy');
    finally
        CloseHandle(Process);
    end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
    if DirExists(ExpandConstant('{app}\profile')) then
        Result := CustomMessage('PortableDirectory')
    else if IsUpdate then
        Result := WaitForQbutt;
end;
