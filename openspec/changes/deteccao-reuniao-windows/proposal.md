# Proposal

## Why

No macOS o Clovy percebe quando um app de chamada (Zoom, Teams, Meet no navegador e afins) abre o microfone, e o HUD pergunta se o usuário quer gravar; quando a chamada termina, uma gravação iniciada por esse aviso encerra sozinha depois de uma contagem visível. No Windows nada disso acontece: o monitor (`meeting_detection::setup`) só é ligado no macOS, a sonda de microfone fora do Mac é um stub vazio e a lista de apps permitidos só conhece bundle ids do Mac.

## What Changes

- O monitor de detecção de reunião e o fim automático passam a rodar também no Windows, com a mesma máquina de estados (`MeetingDetectionState`, `MeetingEndTracker`) e os mesmos eventos para o HUD.
- Nova sonda no Windows: sessões de captura WASAPI ativas em cada endpoint de captura ativo, com o pid levado ao exe do app (filhos do Chromium e do WebView2 sobem até o app que os hospeda). O registro de consentimento do microfone (`CapabilityAccessManager\ConsentStore\microphone`) entra como reforço para apps permitidos que a enumeração não achou.
- Lista de apps permitidos no Windows por nome de exe ou nome de pacote (AUMID/PFN), com rótulos legíveis no HUD ("Chrome", não "exe").
- O próprio Clovy e o helper de ditado nunca disparam.
- Nada muda no macOS.

## Capabilities

### New Capabilities

- `meeting-detection`: detecção de chamada pela posse do microfone, aviso no HUD e fim automático da gravação iniciada pelo aviso, no macOS e no Windows.

### Modified Capabilities

(nenhuma)

## Impact

- `src-tauri/src/meeting_detection.rs` (setup, monitor, lista e rótulos) e o novo `src-tauri/src/meeting_detection/windows_probe.rs` (parser puro e leitura WASAPI, Toolhelp e registro).
- `src-tauri/src/activity/filter.rs`: `BROWSER_EXECUTABLES` passa a ser `pub(crate)` para a lista de reunião conferir em teste que cobre todos os navegadores.
- `docs/audio-pipeline.md`.
- Sem mudança no Clovy API nem no frontend.
