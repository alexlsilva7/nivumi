#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

[Setup]
AppId={{B3E7D092-51F9-4D95-B91B-806126794091}
AppName=Nusa Rebuild
AppVersion={#AppVersion}
AppPublisher=Nusa Rebuild
DefaultDirName={localappdata}\Programs\NusaRebuild
DefaultGroupName=Nusa Rebuild
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
MinVersion=10.0.19045
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=NusaRebuildSetup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\nusa-rebuild.exe
AppMutex=Local\NusaRebuild.SingleInstance
CloseApplications=yes
RestartApplications=no
VersionInfoVersion={#AppVersion}

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\nusa-rebuild.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Nusa Rebuild"; Filename: "{app}\nusa-rebuild.exe"
Name: "{autodesktop}\Nusa Rebuild"; Filename: "{app}\nusa-rebuild.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\nusa-rebuild.exe"; Description: "{cm:LaunchProgram,Nusa Rebuild}"; Flags: nowait postinstall skipifsilent

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "NusaRebuild"; Flags: uninsdeletevalue

[CustomMessages]
brazilianportuguese.RemoveData=Também remover configurações, tarefas, hidratação e Shelf do Nusa Rebuild? Os arquivos originais da Shelf e os dados do Nusa original serão preservados.
english.RemoveData=Also remove Nusa Rebuild settings, tasks, water logs and Shelf? Original Shelf files and data from the original Nusa will be preserved.

[Code]
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
