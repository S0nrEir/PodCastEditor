#define AppVersion "0.1.0"
[Setup]
AppId={{DB44AC77-637D-4082-95ED-8632A65398E4}
AppName=声剪
AppVersion={#AppVersion}
AppPublisher=S0nrEir
AppPublisherURL=https://github.com/S0nrEir/PodCastEditor
DefaultDirName={localappdata}\Programs\PodCastEditor
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\podcast-studio.exe
Compression=lzma2
SolidCompression=yes
OutputDir=..\..\dist
OutputBaseFilename=PodCastEditor-{#AppVersion}-Windows-x64-Setup

[Files]
Source: "..\..\target\release\podcast-studio.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\podcast-editor.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\声剪"; Filename: "{app}\podcast-studio.exe"
