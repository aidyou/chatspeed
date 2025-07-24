@echo off
setlocal EnableDelayedExpansion

  rem =================================================================
  rem Setup Environment Variables for Building
  rem =================================================================
  rem This script automatically configures the build environment by setting
  rem up the necessary environment variables for building the project.
  rem
  rem It dynamically detects and sets up:
  rem - Visual Studio installation path and version
  rem - Windows SDK location and version
  rem - VCPKG root directory
  rem - Include paths for headers
  rem - Library paths for linking
  rem - Binary paths for tools
  rem
  rem Features:
  rem - Automatic detection of latest Visual Studio installation
  rem - Dynamic Windows SDK version detection
  rem - Flexible VCPKG location discovery
  rem - Comprehensive error checking
  rem - Detailed progress reporting
  rem =================================================================

  rem Find Visual Studio installation using vswhere
  set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
  if not exist "%VSWHERE%" (
    echo Visual Studio installer not found. Please install Visual Studio first.
    exit /b 1
  )

  rem Get latest Visual Studio installation path
  for /f "usebackq tokens=*" %%i in (`"%VSWHERE%" -latest -property installationPath`) do (
    set "VSINSTALLDIR=%%i"
  )

  if not defined VSINSTALLDIR (
    echo Visual Studio installation not found.
    exit /b 1
  )

  rem Get VC Tools version from Visual Studio installation
  set "VCTOOLSVERSION_FILE=%VSINSTALLDIR%\VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt"
  if exist "%VCTOOLSVERSION_FILE%" (
    for /f "usebackq tokens=*" %%i in ("%VCTOOLSVERSION_FILE%") do (
      set "VCTOOLSVERSION=%%i"
    )
  ) else (
    echo VC Tools version file not found.
    exit /b 1
  )

  rem Get Windows SDK information from registry
  for /f "tokens=3* delims= " %%a in ('reg query "HKLM\SOFTWARE\WOW6432Node\Microsoft\Microsoft SDKs\Windows\v10.0" /v "InstallationFolder"') do (
    set "WindowsSdkDir=%%a %%b"
  )

  for /f "tokens=3* delims= " %%a in ('reg query "HKLM\SOFTWARE\WOW6432Node\Microsoft\Microsoft SDKs\Windows\v10.0" /v "ProductVersion"') do (
    set "WindowsSdkVersion=%%a.0"
  )

  if not defined WindowsSdkDir (
    echo Windows SDK not found in registry.
    exit /b 1
  )

  rem Set VCPKG_ROOT based on user directory installation first
  set "USER_VCPKG=%USERPROFILE%\vcpkg"
  if exist "%USER_VCPKG%" (
    setx VCPKG_ROOT "%USER_VCPKG%"
    set "VCPKG_ROOT=%USER_VCPKG%"
    rem Set default triplet for static linking
    setx VCPKG_DEFAULT_TRIPLET "x64-windows-static"
    set "VCPKG_DEFAULT_TRIPLET=x64-windows-static"
  ) else (
    rem Try Visual Studio installation
    set "VS_VCPKG=%VSINSTALLDIR%\VC\vcpkg"
    if exist "%VS_VCPKG%" (
      setx VCPKG_ROOT "%VS_VCPKG%"
      set "VCPKG_ROOT=%VS_VCPKG%"
      rem Set default triplet for static linking
      setx VCPKG_DEFAULT_TRIPLET "x64-windows-static"
      set "VCPKG_DEFAULT_TRIPLET=x64-windows-static"
    ) else (
      rem Try other common locations
      set "COMMON_VCPKG=C:\vcpkg"
      if exist "%COMMON_VCPKG%" (
        setx VCPKG_ROOT "%COMMON_VCPKG%"
        set "VCPKG_ROOT=%COMMON_VCPKG%"
        rem Set default triplet for static linking
        setx VCPKG_DEFAULT_TRIPLET "x64-windows-static"
        set "VCPKG_DEFAULT_TRIPLET=x64-windows-static"
      ) else (
        set "LOCAL_VCPKG=%LOCALAPPDATA%\vcpkg"
        if exist "%LOCAL_VCPKG%" (
          setx VCPKG_ROOT "%LOCAL_VCPKG%"
          set "VCPKG_ROOT=%LOCAL_VCPKG%"
          rem Set default triplet for static linking
          setx VCPKG_DEFAULT_TRIPLET "x64-windows-static"
          set "VCPKG_DEFAULT_TRIPLET=x64-windows-static"
        ) else (
          echo Warning: vcpkg not found. Please install vcpkg and set VCPKG_ROOT manually if needed.
        )
      )
    )
  )

  echo VCPKG Root: %VCPKG_ROOT%

  rem Build paths
  set "VsPath=%VSINSTALLDIR%\VC\Tools\MSVC\%VCTOOLSVERSION%"
  set "SdkPath=%WindowsSdkDir%"

  rem Set VCINSTALLDIR
  set "VCINSTALLDIR=%VSINSTALLDIR%\VC\"

  rem Set include paths
  set "INCLUDE=%VsPath%\include;%VsPath%\atlmfc\include;%SdkPath%Include\%WindowsSdkVersion%\ucrt;%SdkPath%Include\%WindowsSdkVersion%\um;%SdkPath%Include\%WindowsSdkVersion%\shared"

  rem Set library paths
  set "LIB=%VsPath%\lib\x64;%SdkPath%Lib\%WindowsSdkVersion%\um\x64;%SdkPath%Lib\%WindowsSdkVersion%\ucrt\x64"

  rem Set LIBPATH for .NET Framework
  set "LIBPATH=%VsPath%\lib\x64;%VsPath%\atlmfc\lib\x64"

  rem Set tool paths
  set "Path=%VsPath%\bin\HostX64\x64;%SdkPath%bin\%WindowsSdkVersion%\x64;%Path%"

  rem Set additional VS environment variables
  set "Platform=x64"
  set "VSCMD_ARG_HOST_ARCH=x64"
  set "VSCMD_ARG_TGT_ARCH=x64"
  set "PreferredToolArchitecture=x64"

  echo Environment variables have been set for Visual Studio and Windows SDK.
  echo VS Installation: %VSINSTALLDIR%
  echo VC Tools Version: %VCTOOLSVERSION%
  echo Windows SDK: %WindowsSdkDir%
  echo Windows SDK Version: %WindowsSdkVersion%
  if defined VCPKG_ROOT (
    echo VCPKG Root: %VCPKG_ROOT%
  )

  echo.
  echo LIB paths:
  for %%i in (%LIB:;=^
%) do (
    if exist "%%i" (
      echo ✓ %%i
    ) else (
      echo ✗ %%i
    )
  )

  echo.
  echo Checking for kernel32.lib:
  set "KERNEL32_PATHS=%SdkPath%Lib\%WindowsSdkVersion%\um\x64\kernel32.lib;%VsPath%\lib\x64\kernel32.lib"
  for %%i in (%KERNEL32_PATHS:;=^
%) do (
    if exist "%%i" (
      echo ✓ %%i
    ) else (
      echo ✗ %%i
    )
  )

endlocal & (
  set "VSINSTALLDIR=%VSINSTALLDIR%"
  set "WindowsSdkDir=%WindowsSdkDir%"
  set "WindowsSdkVersion=%WindowsSdkVersion%"
  set "VCTOOLSVERSION=%VCTOOLSVERSION%"
  set "INCLUDE=%INCLUDE%"
  set "LIB=%LIB%"
  set "Path=%Path%"
  if defined VCPKG_ROOT set "VCPKG_ROOT=%VCPKG_ROOT%"
  if defined VCPKG_DEFAULT_TRIPLET set "VCPKG_DEFAULT_TRIPLET=%VCPKG_DEFAULT_TRIPLET%"
)
