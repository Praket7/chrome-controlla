@echo off
if /i "%~1"=="mcp" goto compact
"%~dp0controlla-core.exe" %*
exit /b %errorlevel%
:compact
"%~dp0controlla-v2-core.exe" %*
exit /b %errorlevel%
