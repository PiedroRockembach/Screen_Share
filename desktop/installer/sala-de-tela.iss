; Instalador da Sala de Tela para Windows (Inno Setup 6).
#define AppVersion GetEnv("APP_VERSION")
#if AppVersion == ""
  #define AppVersion "0.1.0"
#endif

[Setup]
AppId={{6C6F1E3A-5A1B-4C55-9D7B-2F1C8B8E4A10}
AppName=Sala de Tela
AppVersion={#AppVersion}
AppPublisher=Piedro Rockembach
DefaultDirName={autopf}\Sala de Tela
DefaultGroupName=Sala de Tela
DisableProgramGroupPage=yes
OutputDir=..\..\dist
OutputBaseFilename=SalaDeTela-Setup
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
WizardStyle=modern
UninstallDisplayIcon={app}\sala-de-tela.exe

[Languages]
Name: "ptbr"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Files]
Source: "bundle\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{group}\Sala de Tela"; Filename: "{app}\sala-de-tela.exe"
Name: "{autodesktop}\Sala de Tela"; Filename: "{app}\sala-de-tela.exe"

[Run]
; Libera a porta da sala uma vez, aqui, para o app nunca pedir permissão depois.
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""Sala de Tela"""; Flags: runhidden
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""Sala de Tela"" dir=in action=allow protocol=UDP localport=48900"; Flags: runhidden
Filename: "{app}\sala-de-tela.exe"; Description: "Abrir a Sala de Tela"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""Sala de Tela"""; Flags: runhidden; RunOnceId: "firewall"
