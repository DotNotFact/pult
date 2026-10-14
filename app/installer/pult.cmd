@echo off
setlocal
set "PULT_CLI=%~dp0pult-cli.exe"
if not exist "%PULT_CLI%" set "PULT_CLI=%~dp0..\pult-cli.exe"
"%PULT_CLI%" %*
