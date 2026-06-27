@echo off
REM Copy the freshly-built DevWhisp installers into .\installers\
REM so they're easy to find and test.
if not exist "%~dp0" mkdir "%~dp0"
copy /Y "D:\projects\DevWhisp\src-tauri\target\release\bundle\nsis\DevWhisp_0.1.0_x64-setup.exe" "%~dp0DevWhisp_0.1.0_x64-setup.exe" >nul
copy /Y "D:\projects\DevWhisp\src-tauri\target\release\bundle\msi\DevWhisp_0.1.0_x64_en-US.msi" "%~dp0DevWhisp_0.1.0_x64_en-US.msi" >nul
echo Installers copied to %~dp0
dir /b "%~dp0"
