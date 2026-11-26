@echo off
setlocal
rem Abre Maru (la app del overlay).
rem   run-maru.cmd         usa el ejecutable ya compilado si existe; si no, compila.
rem   run-maru.cmd build   vuelve a compilar con cargo y luego abre la app.
set "ROOT=%~dp0"
set "APPDIR=%ROOT%app\src-tauri"
set "EXE=%APPDIR%\target\release\maru-app.exe"

if /i "%~1"=="build" goto build
if exist "%EXE%" (
  echo Abriendo Maru: "%EXE%"
  start "" /d "%APPDIR%" "%EXE%"
  goto :eof
)

:build
where cargo >nul 2>nul
if errorlevel 1 (
  echo No se encontro cargo en el PATH. Abre una terminal donde cargo funcione o instala Rust.
  pause
  exit /b 1
)
echo Compilando y abriendo Maru. La primera vez tarda varios minutos...
pushd "%APPDIR%"
cargo run --release
set "CODE=%ERRORLEVEL%"
popd
if not "%CODE%"=="0" (
  echo.
  echo Maru termino con error %CODE%. Revisa el mensaje de arriba.
  pause
)
exit /b %CODE%
