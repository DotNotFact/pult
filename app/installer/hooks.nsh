; Хуки установщика Пульта. Tauri вставляет этот файл в свой шаблон NSIS
; (bundle.windows.nsis.installerHooks в app/tauri.conf.json), макросы NSIS_HOOK_*
; он зовёт в начале и в конце установки и удаления.
;
; Что здесь:
;   - папка по умолчанию %LOCALAPPDATA%\Programs\Pult (у Tauri - %LOCALAPPDATA%\Pult);
;   - флажок «Добавить команду pult в PATH» на первой странице; в тихой установке - ключ /ADDTOPATH;
;   - перед установкой и удалением закрывается pult-cli.exe из папки программы
;     (Pult.exe закрывает сам шаблон, Пульты из других папок не трогаются);
;   - при удалении вопрос «Удалить также настройки, историю и кэш Пульта?»;
;     тихое удаление данные оставляет, ключ /WIPEDATA удаляет и их;
;   - уборка при удалении: запись в PATH, автозапуск, записи значка в трее.
;
; Данные Пульта живут не в папке программы, поэтому обновление их не трогает:
;   %LOCALAPPDATA%\dev.dotnotfact.pult\data       настройки, история, кэш (core/src/paths.rs)
;   %LOCALAPPDATA%\dev.dotnotfact.pult\EBWebView  данные WebView2 окна
;   %APPDATA%\dev.dotnotfact.pult                 размер и место окна (window-state)
; Обе папки удаляет код шаблона, если выбрано «удалить данные».
;
; ЗАВИСИТ ОТ ШАБЛОНА. Шаблон меняется вместе с @tauri-apps/cli, версия закреплена в
; tools/build-installer.ps1. При её смене сверить с новым installer.nsi и utils.nsh три места,
; помеченные ниже «ШАБЛОН»: макрос SetContext, первую страницу (Welcome) и имена переменных
; $PassiveMode, $UpdateMode, $DeleteAppDataCheckboxState. Расхождение в первых двух makensis
; покажет ошибкой сборки, третье проверяет tools/build-installer.ps1.
;
; Ключи командной строки, которые разбирает ${GetOptions}, ищутся по началу слова: шаблон уже
; занял /P, /NS, /R, /ARGS, /UPDATE, поэтому свои ключи не начинаются с этих букв.

!include LogicLib.nsh
!include nsDialogs.nsh

; папка этого файла: makensis работает в target\release\nsis\x64, а pult-setup.ps1 лежит здесь
!define PULT_HOOKS_DIR "${__FILEDIR__}"
!define PULT_CLI "pult-cli.exe"
!define PULT_RUNKEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define PULT_STARTUPKEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
!define PULT_TRAYKEY "Control Panel\NotifyIconSettings"
; известные папки, которыми Windows записывает путь к exe в настройках значков трея
!define PULT_FOLDERID_LOCALAPPDATA "{F1B32785-6FBA-4FCF-9D55-7B8E7F157091}"
!define PULT_FOLDERID_USERPROGRAMS "{5CD7AEE2-2219-4A67-B85D-6C9CE15660CB}"

Var PultKey        ; HKCU\Software\<издатель>\<программа> - место установки и выбор про PATH
Var PultExe        ; полный путь к Pult.exe в папке установки
Var PultPathBox    ; флажок PATH на первой странице
Var PultAddPath    ; 1 - папку программы добавить в PATH
Var PultPathAsked  ; 1 - флажок показан, выбор сделал человек

; ---------------------------------------------------------------------------
; ШАБЛОН 1: SetContext. Шаблон вставляет его в .onInit установщика и удалятора до того,
; как выбрать папку. Тело ниже - копия utils.nsh из @tauri-apps/cli 2.12.1 плюс наш хвост.
!macroundef SetContext
!macro SetContext
  !if "${INSTALLMODE}" == "currentUser"
    SetShellVarContext current
  !else if "${INSTALLMODE}" == "perMachine"
    SetShellVarContext all
  !endif

  ${If} ${RunningX64}
    !if "${ARCH}" == "x64"
      SetRegView 64
    !else if "${ARCH}" == "arm64"
      SetRegView 64
    !else
      SetRegView 32
    !endif
  ${EndIf}

  StrCpy $PultKey "${MANUPRODUCTKEY}"
  StrCpy $PultExe "$INSTDIR\${MAINBINARYNAME}.exe"

  !ifndef __UNINSTALL__
    !ifndef PLACEHOLDER_INSTALL_DIR
      !error "В шаблоне Tauri нет PLACEHOLDER_INSTALL_DIR: сверить app/installer/hooks.nsh с новым installer.nsi"
    !endif
    ; шаблон ставит папку по умолчанию, только если $INSTDIR равен заглушке (ключ /D= не передан);
    ; ставим свою раньше и, как шаблон, берём место прошлой установки из реестра
    ${If} $INSTDIR == "${PLACEHOLDER_INSTALL_DIR}"
      StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${PRODUCTNAME}"
      Call RestorePreviousInstallLocation
    ${EndIf}
    ; удаление прошлой версии, которое запускает установщик, - часть обновления:
    ; удалятор видит эту метку и не спрашивает про данные
    System::Call 'kernel32::SetEnvironmentVariable(w "PULT_SETUP", w "1")'
  !endif
!macroend

; ---------------------------------------------------------------------------
; ШАБЛОН 2: первая страница установщика - Welcome, у неё шаблон задаёт только PRE.
; Свои SHOW и LEAVE добавляют флажок PATH. Если шаблон задаст их сам, makensis упадёт
; на повторном !define - это и нужно: молча флажок не пропадёт.
!define MUI_PAGE_CUSTOMFUNCTION_SHOW PultWelcomeShow
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE PultWelcomeLeave

Function PultWelcomeShow
  ${If} $PultPathAsked != 1
    ; по умолчанию флажок стоит; если человек раньше его снял - помним
    ReadRegStr $0 HKCU $PultKey "AddToPath"
    ${If} $0 == "0"
      StrCpy $PultAddPath 0
    ${Else}
      StrCpy $PultAddPath 1
    ${EndIf}
  ${EndIf}
  ${If} $LANGUAGE = 1049
    StrCpy $0 "Добавить команду pult в PATH, чтобы звать Пульт из любого терминала"
  ${Else}
    StrCpy $0 "Add the pult command to PATH to run Pult from any terminal"
  ${EndIf}
  ${NSD_CreateCheckbox} 120u 160u 195u 24u $0
  Pop $PultPathBox
  ; цвета страницы Welcome в MUI2 (MUI_TEXTCOLOR и MUI_BGCOLOR по умолчанию)
  SetCtlColors $PultPathBox "000000" "FFFFFF"
  ${If} $PultAddPath = 1
    ${NSD_Check} $PultPathBox
  ${EndIf}
FunctionEnd

Function PultWelcomeLeave
  ${NSD_GetState} $PultPathBox $0
  ${If} $0 = ${BST_CHECKED}
    StrCpy $PultAddPath 1
  ${Else}
    StrCpy $PultAddPath 0
  ${EndIf}
  StrCpy $PultPathAsked 1
FunctionEnd

; ---------------------------------------------------------------------------
; Помощник pult-setup.ps1 (лежит рядом, едет внутри установщика и удалятора): правит PATH и
; останавливает WebView2 закрытого Пульта. Строка NSIS здесь ограничена 1024 знаками, а PATH
; разработчика часто длиннее - обрезанный PATH сломал бы чужие программы; процессов NSIS не видит.
; Папка уходит аргументом -Dir, а не текстом команды. Код выхода - в $R0: 10 - делать нечего,
; 11 - сделано, 12 - папку держит другой запущенный Пульт, остальное - ошибка.
!macro PultHelper ACTION DIR
  InitPluginsDir
  File "/oname=$PLUGINSDIR\pult-setup.ps1" "${PULT_HOOKS_DIR}\pult-setup.ps1"
  nsExec::Exec '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\pult-setup.ps1" -Action ${ACTION} -Dir "${DIR}"'
  Pop $R0
!macroend

; PATH пользователя: HKCU\Environment\Path, только своя запись, остальное как было.
; В PATH идёт $INSTDIR\bin, где лежит один pult.cmd, а не сама папка программы: там рядом Pult.exe,
; и по порядку PATHEXT (.EXE раньше .CMD) команда pult открыла бы окно вместо терминала.
; MODE: add или remove. Портит $R0.
!macro PultPathEdit MODE
  !insertmacro PultHelper "path-${MODE}" "$INSTDIR\bin"
  ${If} $R0 == "11"
    ; открытые окна (проводник, новые терминалы) узнают о новом PATH без перезахода
    SendMessage 0xFFFF 0x001A 0 "STR:Environment" /TIMEOUT=500
    !if "${MODE}" == "add"
      DetailPrint "PATH: добавлена папка $INSTDIR\bin"
    !else
      DetailPrint "PATH: убрана папка $INSTDIR\bin"
    !endif
  ${ElseIf} $R0 != "10"
    DetailPrint "PATH не изменён: pult-setup.ps1 вернул $R0"
  ${EndIf}
!macroend

; ---------------------------------------------------------------------------
; Есть ли в строке $R3 подстрока $R5 (без учёта регистра). Ответ в $R6: 1 или 0. Портит $R2, $R7, $R8, $R9.
!macro PultContains
  StrCpy $R6 0
  StrLen $R7 $R5
  StrLen $R8 $R3
  IntOp $R8 $R8 - $R7
  StrCpy $R9 0
  ${DoWhile} $R9 <= $R8
    StrCpy $R2 $R3 $R7 $R9
    ${If} $R2 == $R5
      StrCpy $R6 1
      ${ExitDo}
    ${EndIf}
    IntOp $R9 $R9 + 1
  ${Loop}
!macroend

; Автозапуск: значения в Run, которые запускают Pult.exe из этой папки, под любым именем
; (шаблон сам удаляет только значение с именем программы).
!macro PultRemoveAutostart
  StrCpy $R4 0
  ${Do}
    ClearErrors
    EnumRegValue $R0 HKCU "${PULT_RUNKEY}" $R4
    ${If} ${Errors}
    ${OrIf} $R0 == ""
      ${ExitDo}
    ${EndIf}
    ReadRegStr $R3 HKCU "${PULT_RUNKEY}" $R0
    StrCpy $R5 $PultExe
    !insertmacro PultContains
    ${If} $R6 = 1
      DeleteRegValue HKCU "${PULT_RUNKEY}" $R0
      DeleteRegValue HKCU "${PULT_STARTUPKEY}" $R0
      DetailPrint "Автозапуск: убрано значение $R0"
    ${Else}
      IntOp $R4 $R4 + 1
    ${EndIf}
  ${Loop}
  DeleteRegValue HKCU "${PULT_STARTUPKEY}" "${PRODUCTNAME}"
!macroend

; Значок в трее: Windows 11 помнит каждый значок в HKCU\Control Panel\NotifyIconSettings\<номер>
; с путём к exe и показывает его в «Параметры - Значки на панели задач» и после удаления
; программы. Путь там бывает записан и через известную папку ({GUID}\...), сверяем все три вида.
!macro PultRemoveTrayEntries
  ; $R1 - путь через LOCALAPPDATA, $R2 - через Programs; пусто, если папка не там
  StrCpy $R1 ""
  StrCpy $R2 ""
  StrLen $R4 $LOCALAPPDATA
  StrCpy $R5 $PultExe $R4
  ${If} $R5 == $LOCALAPPDATA
    StrCpy $R5 $PultExe "" $R4
    StrCpy $R1 "${PULT_FOLDERID_LOCALAPPDATA}$R5"
    StrCpy $R6 $R5 10
    ${If} $R6 == "\Programs\"
      StrCpy $R5 $R5 "" 9
      StrCpy $R2 "${PULT_FOLDERID_USERPROGRAMS}$R5"
    ${EndIf}
  ${EndIf}
  StrCpy $R4 0
  ${Do}
    ClearErrors
    EnumRegKey $R0 HKCU "${PULT_TRAYKEY}" $R4
    ${If} ${Errors}
    ${OrIf} $R0 == ""
      ${ExitDo}
    ${EndIf}
    ReadRegStr $R3 HKCU "${PULT_TRAYKEY}\$R0" "ExecutablePath"
    StrCpy $R6 0
    ${If} $R3 != ""
      ${If} $R3 == $PultExe
      ${OrIf} $R3 == $R1
      ${OrIf} $R3 == $R2
        StrCpy $R6 1
      ${EndIf}
    ${EndIf}
    ${If} $R6 = 1
      DeleteRegKey HKCU "${PULT_TRAYKEY}\$R0"
      DetailPrint "Трей: убрана запись значка $R0"
    ${Else}
      IntOp $R4 $R4 + 1
    ${EndIf}
  ${Loop}
!macroend

; ---------------------------------------------------------------------------
!macro NSIS_HOOK_PREINSTALL
  ; терминальная версия из этой папки держит pult-cli.exe - закрываем её, как шаблон закрывает окно
  !insertmacro CheckIfAppIsRunning "$INSTDIR\${PULT_CLI}" "pult-cli"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; путь в $PultExe ставился в .onInit, а папку человек мог сменить на странице выбора
  StrCpy $PultExe "$INSTDIR\${MAINBINARYNAME}.exe"
  ${If} $PultPathAsked != 1
    ; страницы не было (тихая или пассивная установка): ключ /ADDTOPATH или прошлый выбор
    ${GetOptions} $CMDLINE "/ADDTOPATH" $R0
    ${IfNot} ${Errors}
      StrCpy $PultAddPath 1
    ${Else}
      ReadRegStr $R0 HKCU $PultKey "AddToPath"
      ${If} $R0 == "1"
        StrCpy $PultAddPath 1
      ${Else}
        StrCpy $PultAddPath 0
      ${EndIf}
    ${EndIf}
  ${EndIf}
  ${If} $PultAddPath = 1
    !insertmacro PultPathEdit add
    WriteRegStr HKCU $PultKey "AddToPath" "1"
  ${ElseIf} $PultPathAsked = 1
    ; флажок сняли руками - убираем запись прошлой установки и помним выбор
    !insertmacro PultPathEdit remove
    WriteRegStr HKCU $PultKey "AddToPath" "0"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro CheckIfAppIsRunning "$INSTDIR\${PULT_CLI}" "pult-cli"

  ; ШАБЛОН 3: $DeleteAppDataCheckboxState = 1 - код шаблона удалит обе папки данных
  ${GetOptions} $CMDLINE "/WIPEDATA" $R0
  ${IfNot} ${Errors}
    StrCpy $DeleteAppDataCheckboxState 1
  ${ElseIf} $UpdateMode <> 1
  ${AndIf} $PassiveMode <> 1
  ${AndIf} $DeleteAppDataCheckboxState <> 1
  ${AndIfNot} ${Silent}
    ReadEnvStr $R0 "PULT_SETUP"
    ${If} $R0 != "1"
      ${If} $LANGUAGE = 1049
        StrCpy $R1 "Удалить также настройки, историю и кэш Пульта?$\r$\n$\r$\nДа - удалить всё.$\r$\nНет - оставить: после новой установки Пульт вернёт настройки, недавние проекты и историю."
      ${Else}
        StrCpy $R1 "Also delete Pult settings, history and cache?$\r$\n$\r$\nYes - delete everything.$\r$\nNo - keep them: a new install brings back settings, recent projects and history."
      ${EndIf}
      MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 $R1 /SD IDNO IDNO +2
      StrCpy $DeleteAppDataCheckboxState 1
    ${EndIf}
  ${EndIf}

  ${If} $UpdateMode <> 1
    !insertmacro PultPathEdit remove
    !insertmacro PultRemoveAutostart
    !insertmacro PultRemoveTrayEntries
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    ; WebView2 закрытого окна живёт ещё минуты и пишет в EBWebView: папка, которую шаблон
    ; только что удалил, появилась бы снова. Останавливаем его и удаляем ещё раз.
    !insertmacro PultHelper "webview-stop" "$LOCALAPPDATA\${BUNDLEID}\EBWebView"
    ${If} $R0 == "11"
      DetailPrint "WebView2 закрытого Пульта остановлен"
    ${EndIf}
    StrCpy $R4 0
    ${Do}
      ${IfNot} ${FileExists} "$LOCALAPPDATA\${BUNDLEID}\*.*"
      ${AndIfNot} ${FileExists} "$APPDATA\${BUNDLEID}\*.*"
        ${ExitDo}
      ${EndIf}
      ${If} $R4 >= 10
        ${ExitDo}
      ${EndIf}
      Sleep 500
      RMDir /r "$LOCALAPPDATA\${BUNDLEID}"
      RMDir /r "$APPDATA\${BUNDLEID}"
      IntOp $R4 $R4 + 1
    ${Loop}
    ${If} ${FileExists} "$LOCALAPPDATA\${BUNDLEID}\*.*"
      DetailPrint "Не удалось удалить $LOCALAPPDATA\${BUNDLEID}: файлы заняты другим запущенным Пультом"
      ${IfNot} ${Silent}
        ${If} $LANGUAGE = 1049
          MessageBox MB_OK|MB_ICONEXCLAMATION "Часть данных Пульта занята - скорее всего, Пульт запущен из другой папки.$\r$\nЗакройте его и удалите папку вручную:$\r$\n$LOCALAPPDATA\${BUNDLEID}"
        ${Else}
          MessageBox MB_OK|MB_ICONEXCLAMATION "Some Pult data is in use - Pult is probably running from another folder.$\r$\nClose it and delete the folder manually:$\r$\n$LOCALAPPDATA\${BUNDLEID}"
        ${EndIf}
      ${EndIf}
    ${EndIf}
    ; чужие файлы в папке программы (например, своя база knowledge\ рядом с exe) не трогаем:
    ; папка уйдёт, только если пуста
    RMDir "$INSTDIR"
  ${EndIf}
!macroend
