# Design

## Context

Investigação no código de `origin/main` (`93e3f419`):

- `meeting_detection::setup` (`meeting_detection.rs:834`) só chama `spawn_monitor` no macOS; `spawn_monitor` (`:1005`) e `observe_meeting_end` (`:1072`) estão sob `cfg(target_os = "macos")`.
- Fora do macOS, `active_input_processes` (`:1178`) devolve lista vazia.
- `ALLOWED_MIC_APPS` (`:806`) casa por prefixo de bundle id do Mac; `app_label_from_bundle_id` (`:962`) daria `exe` para `chrome.exe`.
- O macOS usa só o sinal "o processo X está com o microfone" (CoreAudio `kAudioProcessPropertyIsRunningInput`), sem título nem URL. O Meet aparece como "o Chrome está com o microfone".
- Já não dependem de sistema: `MeetingDetectionState::update`, `MeetingEndTracker`, `meeting_end_bundle_families`, `emit_detection_event`, o HUD (`src/hud.ts`, evento `meeting-detection-event`) e a elegibilidade do fim automático (`commands.rs:1422`).
- HUD no Windows: `wake_hud_window` mostra a janela com alfa 0, mas `set_window_alpha` é vazio fora do macOS (`dictation.rs:2535`). A janela `hud` é transparente, sem foco e sem barra de tarefas (`tauri.conf.json`), e o webview só pinta a pílula depois de medir e chamar `dictation_hud_show`. No Windows, então, o "acordar" mostra uma janela transparente sem conteúdo por alguns quadros; não há o que piscar. Manter o `show()` é necessário: num WebView2 escondido o `requestAnimationFrame` para, e o HUD mede a pílula em quadros de animação antes de se mostrar. Esta change não mexe no HUD.

## Goals / Non-Goals

**Goals:** aviso de reunião e fim automático no Windows com o mesmo comportamento do Mac, Google Meet incluído pelo navegador; sonda testável sem Windows.

**Non-Goals:**
- Olhar título de janela ou URL para saber se é reunião (o Mac também não olha).
- Alfa nativo no HUD do Windows (`WS_EX_LAYERED` num host WebView2 transparente arrisca quebrar a pintura; fica para outra change se o "acordar" se mostrar visível).
- Mudar o comportamento no macOS.

## Decisions

**D1. Sonda principal: sessões de captura WASAPI.** Para cada endpoint de captura ativo (`IMMDeviceEnumerator::EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)`), `IAudioSessionManager2::GetSessionEnumerator`, e de cada sessão com `GetState() == AudioSessionStateActive`, o `IAudioSessionControl2::GetProcessId`. Todos os endpoints, não só o padrão, porque apps de chamada deixam escolher o microfone. É o equivalente direto do sinal do Mac (o processo está capturando agora), tem pid, e some assim que o stream para. COM entra em MTA por chamada (mesmo padrão do `ComApartment` de `audio/system_windows.rs`).

**D2. Pid para app.** Uma foto Toolhelp (`CreateToolhelp32Snapshot`) dá pid, pai e exe de todos os processos sem abrir nenhum. O exe vira nome minúsculo (como `normalize_executable_name`). O pid sobe pela cadeia de pais enquanto o processo for `msedgewebview2.exe` (sobe até o app que hospeda o WebView2, ex. `ms-teams.exe`, WhatsApp, ou o próprio Clovy) ou enquanto o pai tiver o mesmo exe (o serviço de áudio do Chrome, `chrome.exe --type=utility`, sobe até o `chrome.exe` principal). A subida tem limite de profundidade e para em ciclo (pid reaproveitado). A exclusão de `owned_pids` (Clovy e helper de ditado) vale depois da subida, então um WebView2 do próprio Clovy também não dispara.

**D3. Reforço pelo registro, só para apps permitidos ainda não achados.** `HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone` (subchaves de pacote, como `MSTeams_8wekyb3d8bbwe`) e `...\microphone\NonPackaged` (caminhos com `#` no lugar de `\`): em uso quando `LastUsedTimeStop == 0` e `LastUsedTimeStart != 0`. Escolha: WASAPI é a fonte principal; o registro só acrescenta um app **permitido** cuja família não apareceu na enumeração, e só se houver processo vivo com aquele exe (o pid vem da foto Toolhelp, o processo raiz daquele exe). Motivos: o registro não tem pid, guarda entradas antigas de apps fechados à força e é uma interface não documentada; a WASAPI é a API pública e tem pid. Como reforço ele cobre o caso de um navegador cuja captura a enumeração não atribuiu a um processo (sessão multiprocesso ou serviço de áudio em outro processo). Entradas não permitidas do registro são ignoradas, para não tornarem ambígua a prova de fim de reunião (`meeting_end_bundle_families` devolve `None` se houver processo externo não permitido).

**D4. Parser puro.** A leitura no Windows só coleta dados crus (`Vec<u32>` de pids ativos, `Vec<ProcessEntry>`, `Vec<ConsentEntry>`); `windows_input_processes(...)` transforma isso em `Vec<MicrophoneInputProcess>`. O parser compila em todo sistema e é testado no Mac; a leitura FFI fica sob `cfg(target_os = "windows")`.

**D5. Lista de apps no Windows.** Uma tabela própria (`WINDOWS_MIC_APPS`) com, por app, nomes de exe, nomes de pacote, rótulo e família estável (`windows:zoom`, `windows:teams`, ...). Zoom, Teams (`teams.exe`, `ms-teams.exe`, `msteams.exe`, pacote `MSTeams`), Slack, Discord, WhatsApp, Telegram, Signal, Webex e os navegadores (Chrome, Edge, Firefox, Brave, Opera, Vivaldi, Arc, Comet, mais os de `BROWSER_EXECUTABLES`). Um teste garante que todo exe de `activity::filter::BROWSER_EXECUTABLES` está na lista. A identidade no Windows é o exe em minúsculas ou, para pacotes vindos do registro, o nome do pacote; o casamento é exato (exe) ou pelo nome do pacote antes do `_` (PFN/AUMID). A busca consulta a tabela do Mac e a do Windows; as identidades não colidem (bundle id com pontos e prefixo reverso vs `*.exe` ou nome de pacote), então o Mac não muda. Rótulo de identidade desconhecida com `.exe` perde a extensão (`foo.exe` vira `foo`).

**D6. Monitor e fim automático no Windows.** `spawn_monitor` e `observe_meeting_end` passam a `cfg(any(target_os = "macos", target_os = "windows"))`, e `setup` liga o monitor nos dois. O laço, a máquina de estados, a contagem e o pedido de finalização são os mesmos do Mac.

## Risks / Trade-offs

- Discord, Slack ou WhatsApp com microfone aberto sem estar em chamada disparam o aviso, como no Mac.
- O "acordar" do HUD no Windows mostra a janela transparente sem alfa 0 (ver Context). Sem prova visual neste ciclo, porque a tela do Windows de prova está bloqueada.
- Nomes de pacote da Store (WhatsApp, Telegram, Slack) vêm de documentação pública e podem mudar; um nome errado só deixa de casar pelo registro, a WASAPI continua achando pelo exe.

## Verification

- Testes unitários do parser (WASAPI, subida de pais, registro, exclusão do Clovy), da lista e do rótulo; teste da máquina de estados com processos do Windows (aviso e fim).
- `make verify` no Mac (macOS sem mudança de comportamento).
- CI Windows (`desktop.yml`).
- Windows de prova (`cert`, tela bloqueada): binário de teste que abre captura do microfone, a sonda o vê, o pid do próprio processo não dispara e o fim é observado quando a captura para. O que não puder ser provado lá fica registrado no relatório.
