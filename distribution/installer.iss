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
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=.
OutputBaseFilename=BDJ_Studio_Search_Pro_Setup_{#MyAppVersion}
Compression=lzma2/ultra64
SolidCompression=yes
PrivilegesRequired=admin
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=..\frontend\windows\runner\resources\app_icon.ico
WizardImageFile=setup_assets\wizard_logo.bmp
WizardSmallImageFile=setup_assets\wizard_small.bmp
UninstallDisplayIcon={app}\{#MyAppExeName}
WizardStyle=modern

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: checkedonce
Name: "autostart"; Description: "Iniciar BDJ Studio Search Pro al encender el equipo (en segundo plano)"; GroupDescription: "{cm:AdditionalIcons}"; Flags: checkedonce

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "BDJ Studio Search Pro"; ValueData: """{app}\{#MyAppExeName}"" --startup"; Flags: uninsdeletevalue; Tasks: autostart

[Files]
Source: "..\frontend\build\windows\x64\runner\Release\frontend.exe"; DestDir: "{app}"; DestName: "{#MyAppExeName}"; Flags: ignoreversion
Source: "..\frontend\build\windows\x64\runner\Release\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs; Excludes: "frontend.exe"
Source: "..\engine\target\release\bdj_search_indexer.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\engine\target\release\bdj_search_ffi.dll"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\Desinstalar {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
; Registro e inicio del servicio elevado para el indexador en segundo plano
Filename: "{sys}\sc.exe"; Parameters: "create BDJSearchProIndexer binPath= ""{app}\bdj_search_indexer.exe"" start= delayed-auto DisplayName= ""BDJ Studio Search Pro Indexer"""; Flags: runhidden
Filename: "{sys}\sc.exe"; Parameters: "description BDJSearchProIndexer ""Mantiene el índice de archivos de BDJ Studio Search Pro."""; Flags: runhidden
Filename: "{sys}\sc.exe"; Parameters: "failure BDJSearchProIndexer reset= 86400 actions= restart/5000/restart/10000/restart/60000"; Flags: runhidden
Filename: "{sys}\sc.exe"; Parameters: "start BDJSearchProIndexer"; Flags: runhidden
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Cerrar procesos activos antes de borrar archivos para que ninguna DLL quede bloqueada
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM {#MyAppExeName} /T"; Flags: runhidden
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM search_pro.exe /T"; Flags: runhidden
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM frontend.exe /T"; Flags: runhidden
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM bdj_search_indexer.exe /T"; Flags: runhidden
; Pausa breve para que el kernel de Windows libere los manejadores de archivo de las DLLs
Filename: "{sys}\cmd.exe"; Parameters: "/c ping 127.0.0.1 -n 2 > nul"; Flags: runhidden
; Detención y eliminación del servicio antes de remover archivos
Filename: "{sys}\sc.exe"; Parameters: "stop BDJSearchProIndexer"; Flags: runhidden
Filename: "{sys}\sc.exe"; Parameters: "delete BDJSearchProIndexer"; Flags: runhidden
; Eliminar entrada de auto-inicio del registro de usuario
Filename: "{sys}\reg.exe"; Parameters: "delete ""HKCU\Software\Microsoft\Windows\CurrentVersion\Run"" /v ""BDJ Studio Search Pro"" /f"; Flags: runhidden

[UninstallDelete]
; Política del producto: los datos NO sobreviven a la desinstalación.
; Eliminar por completo el directorio de instalación
Type: filesandordirs; Name: "{app}"
; Índice, configuración y logs del indexador (servicio que corre como sistema).
Type: filesandordirs; Name: "{commonappdata}\BDJ Studio\Search Pro"
; Datos del frontend en Roaming (BDJ Studio\BDJ Studio Search Pro)
Type: filesandordirs; Name: "{userappdata}\BDJ Studio\BDJ Studio Search Pro"
Type: filesandordirs; Name: "{userappdata}\BDJ Studio\bdj_studio_search_pro"
Type: filesandordirs; Name: "{userappdata}\BDJ Studio Search Pro"
; Limpieza de compatibilidad por si existía la ruta anterior con com.bdjstudio
Type: filesandordirs; Name: "{userappdata}\com.bdjstudio\BDJ Studio Search Pro"
Type: filesandordirs; Name: "{userappdata}\com.bdjstudio"
; Licencia local y datos de usuario en LOCALAPPDATA
Type: filesandordirs; Name: "{localappdata}\BDJ Studio\Search Pro"
