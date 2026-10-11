; SPDX-License-Identifier: GPL-3.0-or-later
; Inno-Setup-Skript für den Windows-Installer von FadeWerk DJ (Windows 10 und 11, 64 Bit).
; Gebaut vom Release-Workflow: iscc /DVersion=x.y.z /DSource=<Ordner mit den Dateien> fadewerk-dj.iss
; Installiert nach „Programme“, legt einen Startmenü-Eintrag und (abwählbar) eine Desktop-Verknüpfung an.
; Einstellungen, Playlists und Caches liegen im Benutzerprofil und bleiben beim Deinstallieren erhalten.
; Das Update aus der App startet dieses Setup mit /SILENT; danach startet es die neue Version selbst.

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef Source
  #define Source "..\..\target\release"
#endif

[Setup]
; Nie ändern: daran erkennt Windows frühere Installationen (Update landet im selben Ordner)
AppId={{56B7BE79-7E57-4146-B6B0-A7871E9445DD}
AppName=FadeWerk DJ
AppVersion={#Version}
AppVerName=FadeWerk DJ {#Version}
AppPublisher=Markus Eckert
AppPublisherURL=https://github.com/eckonator/fadewerk-dj
AppSupportURL=https://github.com/eckonator/fadewerk-dj/issues
AppUpdatesURL=https://github.com/eckonator/fadewerk-dj/releases
DefaultDirName={autopf}\FadeWerk DJ
DefaultGroupName=FadeWerk DJ
DisableProgramGroupPage=yes
LicenseFile=..\..\LICENSE
OutputBaseFilename=FadeWerk-DJ-{#Version}-Windows-Setup
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\FadeWerk DJ.exe
UninstallDisplayName=FadeWerk DJ
VersionInfoVersion={#Version}
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "de"; MessagesFile: "compiler:Languages\German.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#Source}\fadewerk-dj.exe"; DestDir: "{app}"; DestName: "FadeWerk DJ.exe"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\fonts\OFL-*.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\THIRD-PARTY-LICENSES.yml"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\FadeWerk DJ"; Filename: "{app}\FadeWerk DJ.exe"
Name: "{autodesktop}\FadeWerk DJ"; Filename: "{app}\FadeWerk DJ.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\FadeWerk DJ.exe"; Description: "{cm:LaunchProgram,FadeWerk DJ}"; Flags: nowait postinstall skipifsilent
; Update aus der App (still): neue Version wieder starten, als angemeldeter Benutzer, nicht als Administrator
Filename: "{app}\FadeWerk DJ.exe"; Parameters: "--after-update"; Flags: nowait runasoriginaluser; Check: WizardSilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM ""FadeWerk DJ.exe"""; Flags: runhidden; RunOnceId: "StopApp"
