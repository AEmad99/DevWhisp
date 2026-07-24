@echo off
REM Copy the freshly-built DevWhisp NSIS installer into .\installers\
REM so it's easy to find and test.
if not exist "%~dp0" mkdir "%~dp0"
copy /Y "D:\projects\DevWhisp\src-tauri\target\release\bundle\nsis\DevWhisp_0.1.6_x64-setup.exe" "%~dp0DevWhisp_0.1.6_x64-setup.exe" >nul
echo Installer copied to %~dp0
dir /b "%~dp0"
