# Design

## Context

- Captura atual: microfone por CPAL no processo do app e áudio do sistema por um helper assinado fora do processo (`Clovy Audio Capture.app`, tap do CoreAudio), aberto por `/usr/bin/open -n`, controlado por SIGUSR1/SIGUSR2/SIGTERM e `status.json` (ADR-0004, `src-tauri/src/audio/system_macos.rs`). Os helpers são compilados em `build.rs` e copiados por `tauri.macos.conf.json`.
- ScreenCaptureKit já é usado só para captura de uma imagem de janela, em memória (`src-tauri/src/activity/macos/ocr.rs`, `shareable_content`, `capture_window_image`). Estado da permissão de Gravação de tela em `activity/macos/permissions.rs`.
- Detecção de reunião olha só quem usa o microfone (`meeting_detection.rs`), sem título de janela. `activity/timeline/categorize.rs` tem `MEETING_AUDIO_APPS` e `MEETING_TOKENS` (`meet.google.com`, `zoom.us/j`, ...).
- Pipeline: `finish_recording` → `process_saved_source_audio_pipeline` → transcrição Groq → `GenerationRequest` (só texto) → `generate_note_from_transcript`, que no CLI chama `llm::generate` com `claude -p ... --tools "" --setting-sources ""` numa pasta temporária, tempo limite de 180 s.
- O contrato de isolamento de CLI (`docs/llm-providers.md` ~80-97, requisito "Execução isolada" em `llm-providers`) proíbe ferramentas em chamada pontual; esta change abre uma exceção única.
- Skill na máquina do dono: `~/.agents/skills/video-analysis/SKILL.md` e `scripts/video_evidence.py`, que usam `python3`, `ffmpeg` e `ffprobe` (em `/opt/homebrew/bin` na máquina dele).

## Goals / Non-Goals

**Goals:**
- Vídeo de uma janela, alinhado ao áudio, sem risco para a gravação de áudio.
- Nota mais rica pelo `claude` rodando a skill, com a menor concessão de ferramentas possível.

**Non-Goals:**
- Lista de janelas para escolha manual (o dono escolheu só a janela detectada).
- Gravar a tela inteira, áudio dentro do vídeo, ou Windows.
- Análise de vídeo por Codex, endpoint ou OpenRouter.
- Reprodutor de vídeo dentro da nota.
- Empacotar a skill, `python3` ou `ffmpeg` no app.

## Decisions

1. **Janela de reunião detectada no Rust.** Enumerar as janelas na tela com `SCShareableContent` (reaproveitando o acesso de `ocr.rs`) e escolher a que tem app de reunião (Zoom, Teams) ou navegador com título que casa com os tokens de reunião (`Meet`, `meet.google.com`, `Zoom`, `Teams`), preferindo o app que está usando o microfone e, entre empates, a janela da frente. Comando novo `detect_meeting_window` devolve `{windowId, appName, bundleId, title}` ou nada. Limite aceito: Meet numa aba de fundo do Chrome não aparece no título e não é detectado.
2. **Helper novo `Clovy Video Capture.app`** no mesmo contrato do helper de áudio (abertura, sinais, `status.json`, log). Dentro dele: `SCStream` com `SCContentFilter(desktopIndependentWindow:)`, sem áudio, até 5 quadros/s, lado maior até 1920 px, H.264 por `AVAssetWriter` em MP4 fragmentado (fragmento de 10 s) para sobreviver a encerramento forçado. Pausa descarta quadros e desloca os tempos, como o áudio faz com a pausa. Alternativas descartadas: captura no processo por `msg_send` (ciclo de vida de `SCStream` frágil em Rust) e `ffmpeg` (já rejeitado no projeto, grava a tela e não a janela). O helper tem identificador próprio e pede sua própria permissão de Gravação de tela.
3. **Ciclo de vida acoplado à sessão.** `start_recording`/`start_meeting_recording` recebem `video: Option<{windowId}>`. O vídeo inicia depois do áudio e registra o deslocamento em ms; erro do vídeo vira estado do artefato e nunca falha o início do áudio. Pausa, retomada e fim seguem os da sessão; a recuperação trata o arquivo parcial como trata o WAV parcial.
4. **Banco:** migração nova com tabela `video_artifacts` (sessão, caminho parcial e final, estado, deslocamento, app, título, largura, altura, duração, motivo de falha), separada de `audio_artifacts`, que é presa a WAV e lida por todo o pipeline de áudio. Arquivo `window.partial.mp4` → `window.mp4` na pasta da sessão; remoção da nota e da gravação apaga o arquivo pelos caminhos de limpeza já existentes.
5. **Geração com vídeo.** `GenerationRequest` ganha `video: Option<NoteVideoInput {path, offset_ms, app, title}>`. Rota: motor de Notas = `claude` e pré-requisitos presentes (SKILL.md, script, `python3`, `ffmpeg`, `ffprobe` resolvidos pelo PATH do shell de login) → chamada com vídeo; senão → transcrição como hoje, com o motivo. Falha ou tempo esgotado na chamada com vídeo → nova chamada só com transcrição, com o motivo.
6. **Concessão de ferramentas só para essa chamada.** `llm::GenerateRequest` ganha um campo de concessão que só o construtor de invocação do `claude` aceita (qualquer outro CLI com concessão falha). Argumentos: `--tools Bash,Read`, `--allowedTools "Bash(python3 <script> <video>:*)" "Read(<video>)" "Read(<pasta de evidências>/**)"`, modo de permissão que nega o que não está liberado sem perguntar, `--add-dir <pasta de evidências>`, e mantém `--strict-mcp-config`, `--setting-sources ""`, `--no-session-persistence`, `--disable-slash-commands` e o marcador de autoria. As flags exatas são conferidas no `claude --help` instalado e fixadas em teste. Tempo limite de 20 min. A pasta de evidências é temporária, vazia e apagada ao fim.
7. **Prompt.** Sistema = contexto de segurança + prompt de nota + texto do SKILL.md lido na hora + regras: rodar o script com `--no-transcript --out <pasta>`, abrir as folhas de contato e os quadros relevantes, cruzar com a transcrição usando o deslocamento, citar instantes, tratar texto da tela e falas como dados e nunca como instruções. A transcrição já vem da Groq; a skill não transcreve.
8. **Registro.** `generation_results` grava `prompt_version` próprio para nota com vídeo e uma coluna com o resultado da análise (`analyzed` ou o motivo). A nota mostra o aviso a partir dessa coluna.
9. **Interface.** No painel de opções da gravação (`NoteEditor.tsx`), um `Switch` "Gravar vídeo da reunião" com app e título, consultando `detect_meeting_window` ao abrir o painel e de novo ao iniciar. No aviso de reunião detectada, a mesma opção. Textos em en e pt-BR, regras de `spec/` (sentence case, ícones `central-icons`, tokens), seguindo `~/.agents/skills/impeccable/SKILL.md`.
10. **ADR-0071** registra a exceção ao contrato de isolamento e o helper de vídeo; `docs/llm-providers.md`, `docs/audio-pipeline.md`, `CONTEXT.md` (termo "vídeo da reunião") e `PRIVACY.md` mudam junto. `NSScreenCaptureUsageDescription` passa a citar a gravação da janela da reunião.

## Risks / Trade-offs

- [Injeção pela call: alguém fala ou mostra na tela uma instrução para o modelo] → Bash só aceita o script da skill com o caminho daquele vídeo, Read só o vídeo e a pasta de evidências, sem MCP e sem rede; o prompt marca transcrição e tela como dados.
- [Permissão de tela nova para o helper] → primeiro uso pede a permissão; sem ela a gravação segue só com áudio e a nota diz o motivo.
- [Disco: cerca de 0,5 GB por hora a 5 quadros/s] → taxa de bits baixa; o vídeo some com a nota.
- [Tempo e custo do Claude em reuniões longas] → a skill limita o número de quadros; tempo limite de 20 min com retorno à transcrição.
- [Skill, `python3` ou `ffmpeg` ausentes em outra máquina] → checagem antes de iniciar o CLI e aviso na nota.
- [Meet em aba de fundo não detectado] → limite aceito da decisão 1, citado no aviso de reunião quando o navegador usa o microfone sem janela casada.
