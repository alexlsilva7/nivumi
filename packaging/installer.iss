#ifndef AppVersion
  #define AppVersion "0.2.0"
#endif

[Setup]
AppId={{B3E7D092-51F9-4D95-B91B-806126794091}
AppName=Nivumi
AppVersion={#AppVersion}
AppPublisher=Nivumi
DefaultDirName={localappdata}\Programs\Nivumi
DefaultGroupName=Nivumi
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
MinVersion=10.0.19045
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=NivumiSetup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\brand\app.ico
UninstallDisplayIcon={app}\nivumi.exe
AppMutex=Local\NusaRebuild.SingleInstance
CloseApplications=yes
RestartApplications=no
VersionInfoVersion={#AppVersion}

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\target\release\nivumi.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Nivumi"; Filename: "{app}\nivumi.exe"; AppUserModelID: "Nivumi.Desktop"
Name: "{autodesktop}\Nivumi"; Filename: "{app}\nivumi.exe"; Tasks: desktopicon; AppUserModelID: "Nivumi.Desktop"

[Run]
Filename: "{app}\nivumi.exe"; Description: "{cm:LaunchProgram,Nivumi}"; Flags: nowait postinstall skipifsilent

Filename: "{win}\explorer.exe"; Parameters: "/select,""{autoprograms}\Nivumi.lnk"""; Description: "{cm:PinTaskbar}"; Flags: nowait postinstall skipifsilent unchecked

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "NusaRebuild"; Flags: uninsdeletevalue

[CustomMessages]
brazilianportuguese.PinTaskbar=Abrir atalho para fixar na barra de tarefas (botão direito → Mostrar mais opções → Fixar na barra de tarefas)
english.PinTaskbar=Open shortcut to pin to taskbar (right-click → Show more options → Pin to taskbar)
brazilianportuguese.RemoveData=Também remover configurações, tarefas, hidratação e Shelf do Nivumi? Os arquivos originais da Shelf e os dados do Nusa original serão preservados.
english.RemoveData=Also remove Nivumi settings, tasks, water logs and Shelf? Original Shelf files and data from the original Nusa will be preserved.

[Code]
procedure CurStepChanged(CurStep: TSetupStep);
var
  StartupCommand: String;
begin
  if (CurStep = ssPostInstall) and FileExists(ExpandConstant('{app}\nivumi.exe')) then
  begin
    { Keep the existing Run value identity. Only update it if startup was enabled. }
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run',
      'NusaRebuild', StartupCommand) then
      RegWriteStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run',
        'NusaRebuild', '"' + ExpandConstant('{app}\nivumi.exe') + '"');
    { The new payload is present before removing obsolete files and shortcuts. }
    DeleteFile(ExpandConstant('{app}\nusa-rebuild.exe'));
    DeleteFile(ExpandConstant('{autoprograms}\Nusa Rebuild.lnk'));
    DeleteFile(ExpandConstant('{autodesktop}\Nusa Rebuild.lnk'));
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    if not UninstallSilent then
    begin
      if MsgBox(CustomMessage('RemoveData'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(ExpandConstant('{localappdata}\NusaRebuild'), True, True, True);
    end;
  end;
end;
