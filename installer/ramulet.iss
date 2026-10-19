; Ramulet 安装包（Inno Setup 6）。用 scripts\installer.ps1 构建：它先编译，再传入版本号。
; 按当前用户安装到 %LOCALAPPDATA%\Programs\Ramulet，不要管理员权限。

#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
AppId=Ramulet
AppName=Ramulet
AppVersion={#Version}
AppVerName=Ramulet {#Version}
AppPublisher=ArthurLau
AppPublisherURL=https://github.com/ArthurLauCS/Ramulet
DefaultDirName={autopf}\Ramulet
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
LicenseFile=..\LICENSE
OutputDir=..\target\installer
OutputBaseFilename=Ramulet-Setup-{#Version}
SetupIconFile=..\assets\app.ico
UninstallDisplayIcon={app}\ramulet.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; 每次都让用户选语言；第一次安装时选中的语言也是程序的界面语言
UsePreviousLanguage=no
; 升级时关掉正在运行的实例（设置在每次改动时就已存盘，直接关不丢东西）
CloseApplications=force
RestartApplications=no

[Languages]
Name: "zh"; MessagesFile: "ChineseSimplified.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
zh.AutoStart=开机自动启动
en.AutoStart=Start with Windows
zh.DeleteData=同时删除设置吗？%n%n设置里有你起的仓库和分支别名、已关闭窗口的列表。重装时保留它们可以接着用。
en.DeleteData=Also delete your settings?%n%nThey include your repo and branch aliases and the list of closed windows. Keep them if you plan to reinstall.

[Tasks]
Name: "autostart"; Description: "{cm:AutoStart}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "..\target\release\ramulet.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Ramulet"; Filename: "{app}\ramulet.exe"
Name: "{autodesktop}\Ramulet"; Filename: "{app}\ramulet.exe"; Tasks: desktopicon

[Registry]
; 和菜单里“开机自启”写的是同一个值，两边开关互认
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Ramulet"; ValueData: """{app}\ramulet.exe"""; Tasks: autostart; Flags: uninsdeletevalue

[Run]
Filename: "{app}\ramulet.exe"; Description: "{cm:LaunchProgram,Ramulet}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM ramulet.exe"; Flags: runhidden; RunOnceId: "Kill"

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

function ConfigFile: String;
begin
  Result := ExpandConstant('{userappdata}\Ramulet\config.txt');
end;

// 第一次安装：把安装时选的语言写进设置；已经有设置（升级、重装）就不动
procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and not FileExists(ConfigFile) then
  begin
    ForceDirectories(ExtractFileDir(ConfigFile));
    SaveStringToFile(ConfigFile, 'lang|' + ActiveLanguage + #13#10, False);
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    // 菜单里打开的开机自启（安装时没勾也可能有）一并删掉
    RegDeleteValue(HKCU, RunKey, 'Ramulet');
    // 静默卸载时按默认“否”，保留数据
    if SuppressibleMsgBox(CustomMessage('DeleteData'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
      DelTree(ExpandConstant('{userappdata}\Ramulet'), True, True, True);
  end;
end;
