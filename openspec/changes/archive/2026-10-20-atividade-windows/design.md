# Design

## Context

Motivação em `proposal.md`. Estado atual:

- `CaptureEngine<P: ActivityPlatform>` (`src-tauri/src/activity/engine.rs`) e `schedule.rs`, `filter.rs`, `input.rs`, `store.rs` e `timeline/` não têm código de sistema. O único adaptador é `MacActivityPlatform` (`activity/macos/`), e `activity::setup` só chama `start()` no macOS (`activity/mod.rs:351-361`). Sem runtime, a ingestão de agentes (`coding_agents/mod.rs:236-254`) e o pipeline horário (`day_intelligence/mod.rs:123-125`) também não sobem.
- O trait tem 11 métodos (`activity/platform.rs:100-131`, incluindo `system_idle_ms`).
- SQLCipher (`libsqlite3-sys` com `bundled-sqlcipher-vendored-openssl`) só está no alvo macOS (`Cargo.toml:163-169`), para evitar Perl e NASM no build Windows. `store.rs:585-602` recusa abrir sem cifra. A chave usa `keyring` com `windows-native`, já declarado (`Cargo.toml:179-180`), mas nunca instanciado no Windows.
- Candle e tokenizers só existem no macOS, com `accelerate` (`Cargo.toml:170-177`); fora dele `embed` devolve erro e o destilador cai para o modo lexical (`day_intelligence/embedder.rs:347-351`).
- Identidade de app e listas: browsers por prefixo de bundle id com fallback por nome (`filter.rs:17-41`), editores por bundle e nome (`timeline/context.rs:41-63`), tela bloqueada por nome `loginwindow`/`screensaverengine` (`timeline/builder.rs:24-25`), DRM por nome do app e domínio (`filter.rs:151-209`), ícones só por bundle id no macOS (`app_icon.rs`).
- Fontes de agentes usam `HOME` e `~/Library/Application Support` (`coding_agents/sources/mod.rs:50-72`).
- Frontend: a aba Atividade só aparece em `isMacLikePlatform()` (`settings-config.ts:61-66`, `AppSettings.tsx:343-348`); `ActivityTimelineView.tsx` não trata `availability: "unsupported"`.
- O crate `windows` 0.61 já é dependência no alvo Windows (áudio e shell).
- Máquina de prova: `cert` (Tailscale `100.80.156.6`, usuário `cert-`), Windows 11 build 26200 x64, pt-BR, VS Build Tools 2022, Node 24, rustup sem toolchain ativo, sem Perl, NASM e pnpm, 65 GB livres, usuário com sessão de console ativa.

A captura do Meridian no Windows (referência de comportamento; licença PolyForm Strict, nenhum código copiado) usa UI Automation, Windows Graphics Capture, `Windows.Media.Ocr`, ganchos de entrada e `GetLastInputInfo`, dentro do processo do app.

## Goals / Non-Goals

**Goals:**
- Paridade de captura, linha do tempo, frentes, resumo e agentes com o macOS no Windows 10 1903+ e 11 x64.
- Nenhuma mudança de comportamento no macOS.

**Non-Goals:**
- Linux (só a mensagem de "não suportado" na vista).
- Windows ARM64.
- Detectar vídeo protegido por API de DRM (vale a lista por nome e domínio que já existe).
- Instalação silenciosa de serviço, início automático ou captura fora da sessão do usuário.
- Mudar o instalador além do necessário para o SQLCipher.

## Decisions

### D1. Adaptador próprio com o crate `windows`, no processo do app

Módulo `activity/windows/` implementando `ActivityPlatform`, ligado por `#[cfg(target_os = "windows")]` ao lado do macOS em `activity/mod.rs`. Mesma thread de captura, mesmo tick de 2 s.

Alternativas descartadas: o fork privado do screenpipe usado pelo Meridian (inacessível e de licença não verificada); o screenpipe MIT 0.4.6 (dependência grande, parada no tempo, e traz o próprio modelo de captura); um processo separado (o Clovy é um processo só no macOS e não precisa de outro).

### D2. Janela em foco e identidade do app

`GetForegroundWindow` → `GetWindowThreadProcessId` → `QueryFullProcessImageNameW`. `bundle_id` recebe o nome do executável em minúsculas (`msedge.exe`); para apps empacotados (UWP/MSIX), o AppUserModelID. `app_name` vem do `FileDescription` do executável, com o nome do arquivo sem extensão como fallback. Título por `GetWindowTextW`. `window_id` = HWND; `display_id` = índice do monitor.

As listas de `filter.rs`, `timeline/context.rs`, `timeline/categorize.rs` e `timeline/builder.rs` ganham os nomes de executável do Windows (navegadores, editores, `lockapp.exe`, `logonui.exe`). A janela do próprio Clovy continua excluída por pid.

### D3. Texto e URL pela UI Automation

`IUIAutomation` (COM em apartamento MTA na thread de captura), `ElementFromHandle` da janela em foco, caminhada limitada (mesmo orçamento de tempo do AX no macOS: 0,25 s por chamada, teto de nós) coletando `Name` e `TextPattern`/`ValuePattern` de documento, texto e edição. Janela privada por marcador no título, já existente em `filter.rs`, mais o nome de perfil InPrivate/Incognito exposto pela UIA quando houver. Chromium ativa a árvore UIA quando um cliente a consulta; não há equivalente a `AXManualAccessibility`.

**Ajuste autorizado em 2026-10-19:** a URL identifica o documento confirmado, não o texto editável da barra. A prova no Edge abriu `http://127.0.0.1:19362/`, mas o controle omitiu o esquema e a implementação inicial inventou `https://127.0.0.1:19362`. Além disso, um endereço digitado sem navegação não identifica a página exibida. Usar metadados de URI confirmada do documento expostos pela UIA, sem ler perfis nem integrar DevTools ao produto. Não inferir esquema, não usar o valor em edição da barra e não procurar URLs no texto renderizado. Documento ausente, identidade ambígua ou metadado não disponível deixam a URL desconhecida; com exclusões de domínio configuradas, o motor falha fechado. A caminhada de descrição pode ler somente a identidade do documento, nunca seu conteúdo ou seus descendentes.

### D4. OCR com Windows Graphics Capture e `Windows.Media.Ocr`

`IGraphicsCaptureItemInterop::CreateForWindow(hwnd)` → um único quadro em memória → `SoftwareBitmap` → `OcrEngine::TryCreateFromUserProfileLanguages`. Sem gravar nada em disco. No Windows 11, `IsBorderRequired = false`; no Windows 10 a moldura amarela pode piscar durante a captura (aceito). Alternativa descartada: `PrintWindow`/`BitBlt`, que devolve preto em janelas com aceleração de GPU.

### D5. Entrada pelo Raw Input, não por ganchos globais

Janela só de mensagens numa thread própria, `RegisterRawInputDevices` com `RIDEV_INPUTSINK` para teclado e mouse; o callback registra só o instante de mouse down e key down (sem código de tecla). Alternativa descartada: `WH_KEYBOARD_LL`/`WH_MOUSE_LL`, que ficam no caminho de toda entrada do sistema e disparam heurísticas de antivírus.

`system_idle_ms`: `GetLastInputInfo` com `GetTickCount` e subtração circular (`wrapping_sub`) em `u32`.

### D6. Área de transferência, telas e disco

Área de transferência: `GetClipboardSequenceNumber` para detectar mudança, `CF_UNICODETEXT` para ler; conteúdo marcado com os formatos `ExcludeClipboardContentFromMonitorProcessing` ou `CanIncludeInClipboardHistory = 0` (usados por gerenciadores de senha) é descartado, como os tipos ocultos no macOS. Telas: `EnumDisplayMonitors` e a janela visível mais alta de cada monitor secundário por `EnumWindows` em ordem Z, monitores relidos a cada tick secundário (não há aviso de mudança). Disco: `GetDiskFreeSpaceExW`.

### D7. Permissões

O adaptador devolve `Granted` para as três permissões e `request_permission` não faz nada; `schedule::missing_required` continua o mesmo. A interface decide pela plataforma: no Windows a seção de atividade não renderiza as linhas de permissão. Alternativa descartada: novo estado "não aplicável" no enum, que mudaria a máquina de estado e o DTO para um caso só visual.

### D8. SQLCipher e chave no Windows

`libsqlite3-sys` com `bundled-sqlcipher-vendored-openssl` passa a valer também no alvo Windows. O build exige Perl (Strawberry Perl) e NASM: `scripts/build-windows.ps1` checa os dois e falha com mensagem clara; o job `windows-rust` instala ambos por Chocolatey. A chave usa `KeychainKeyStore` (Credential Manager), com o serviço de dev já separado. Alternativa descartada: banco em claro no Windows, que quebra o requisito de banco cifrado.

### D9. Embeddings no Windows

Candle e tokenizers entram também no alvo Windows, sem `accelerate` (CPU puro); o `cfg` do embedder BGE passa a incluir o Windows. O download do modelo e o caminho em `<app data>/models/` não mudam. Se o build ou o desempenho no `cert` não forem aceitáveis (tarefa de verificação), cai-se para o modo lexical já existente e isso volta ao dono.

### D10. Fontes de agentes

`SourceRoots::current()` usa `dirs::home_dir()` quando `HOME` não existir, e `app_support` passa a ser `dirs::config_dir()` fora do macOS (`%APPDATA%`). `open_regular` sem `O_NOFOLLOW` no Windows passa a recusar pontos de reparo (`FILE_FLAG_OPEN_REPARSE_POINT` e checagem do atributo) para manter a proteção contra links.

### D11. Interface

Aba Atividade visível quando a captura é suportada (`status.supported`), não por `isMacLikePlatform`. `ActivitySettingsSection` esconde permissões fora do macOS. `ActivityTimelineView` ganha estado próprio para `availability: "unsupported"`. Ícones de "Principais apps" no Windows saem do executável (`SHGetFileInfoW` → PNG), cacheados como no macOS. Trabalho de interface segue `/impeccable`.

### D12. Prova na máquina `cert`

Build nativo no `cert` (rustup `stable-x86_64-pc-windows-msvc`, Strawberry Perl, NASM, pnpm via corepack), clone do repositório num diretório do usuário `cert-`. O app de desenvolvimento precisa rodar na sessão de console (a sessão SSH não tem área de trabalho): lançamento por tarefa agendada interativa (`schtasks /Create ... /IT /RU cert-` e `/Run`). Evidência por export de depuração lido via SSH e capturas de tela feitas no `cert`. Nada é instalado no `cert` fora do perfil do usuário sem combinar com o dono.

## Risks / Trade-offs

- [UI Automation lenta ou travando em apps pesados] → orçamento de tempo e de nós por chamada, igual ao AX; quadro cai para OCR se o texto vier fraco.
- [Moldura amarela da captura no Windows 10] → aceito; Windows 11 sem moldura.
- [OpenSSL compilado aumenta tempo de build e exige Perl/NASM] → checagem no script, instalação no CI; tempo medido na tarefa de CI.
- [Candle em CPU lento no Windows] → medir no `cert`; fallback lexical já existe.
- [Antivírus marcando leitura de entrada] → Raw Input em vez de ganchos globais; executável assinado nos builds de produção.
- [Nomes de executável mudam entre versões dos apps] → listas por nome com fallback por palavra, como no macOS.
- [`cert` é uma máquina de uso real] → só ferramentas no perfil do usuário e diretório próprio para o clone; limpeza no fim.

## Migration Plan

- Sem dados antigos no Windows: o banco de atividade nasce cifrado na primeira ativação.
- macOS sem mudança de dados.
- Rollback: reverter a change; no Windows o banco de atividade fica órfão e cifrado, sem efeito no banco principal.
- Ordem: as changes `add-activity-intelligence` e `atividade-ocioso-e-minutos` já foram integradas e arquivadas na main; esta change integra o suporte ao Windows sobre as especificações canônicas.
