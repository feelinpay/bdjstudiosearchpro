; Script de instalación Inno Setup 6 para BDJ Studio Search Pro
; Registra el servicio de indexación Windows en modo delayed-auto y al
; desinstalar detiene el servicio y borra TODOS los datos (nada sobrevive:
; es la política compartida con las demás apps BDJ Studio).

; La versión puede inyectarse desde fuera para que no se desincronice de
; pubspec.yaml / preflight:  ISCC /DMyAppVersion=1.0.0 installer.iss
; El valor de abajo es solo el respaldo cuando se compila a mano.
#ifndef MyAppVersion
  #define MyAppVersion "1.0.0"
#endif
#define MyAppName "BDJ Studio Search Pro"
#define MyAppPublisher "BDJ Studio"
#define MyAppExeName "bdj_studio_search_pro.exe"

[Setup]
AppId={{C47E8B19-5D9B-4D2A-8C21-9B5A3D4E6F70}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\BDJ Studio\Search Pro
DefaultGroupName=BDJ Studio
OutputDir=..\output
OutputBaseFilename=BDJ_Studio_Search_Pro_Setup_{#MyAppVersion}
Compression=lzma2/ultra64
SolidCompression=yes
PrivilegesRequired=admin
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: checkedonce

[Files]
Source: "..\frontend\build\windows\x64\runner\Release\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\engine\target\release\bdj_search_indexer.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
; Registro e inicio del servicio elevado para el indexador en segundo plano
Filename: "{sys}\sc.exe"; Parameters: "create BDJSearchProIndexer binPath= ""{app}\bdj_search_indexer.exe"" start= delayed-auto DisplayName= ""BDJ Studio Search Pro Indexer"""; Flags: runhidden
Filename: "{sys}\sc.exe"; Parameters: "description BDJSearchProIndexer ""Mantiene el índice de archivos de BDJ Studio Search Pro."""; Flags: runhidden
Filename: "{sys}\sc.exe"; Parameters: "start BDJSearchProIndexer"; Flags: runhidden
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Detención y eliminación del servicio antes de remover archivos
Filename: "{sys}\sc.exe"; Parameters: "stop BDJSearchProIndexer"; Flags: runhidden
Filename: "{sys}\sc.exe"; Parameters: "delete BDJSearchProIndexer"; Flags: runhidden

[UninstallDelete]
; Política del producto: los datos NO sobreviven a la desinstalación.
; Índice, configuración y logs del indexador (servicio que corre como sistema).
Type: filesandordirs; Name: "{commonappdata}\BDJ Studio\Search Pro"
; Datos del frontend (preferencias y almacén seguro heredado):
; getApplicationSupportDirectory() en Windows es %APPDATA%\CompanyName\ProductName
; (Runner.rc: CompanyName "com.bdjstudio", ProductName "BDJ Studio Search Pro").
Type: filesandordirs; Name: "{userappdata}\com.bdjstudio\BDJ Studio Search Pro"
