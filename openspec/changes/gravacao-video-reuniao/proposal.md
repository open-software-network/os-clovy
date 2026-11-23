# Proposal

## Why

Hoje o Clovy grava só áudio (microfone e sistema), e a nota sai só da transcrição. Numa call, boa parte do conteúdo está na tela: slides, telas compartilhadas, documentos, quem aparece. O dono quer que, ao gravar uma reunião, o app ofereça gravar também o vídeo da janela da reunião e que a nota use esse vídeo, com o Claude Code rodando a skill `video-analysis`, para um resumo mais completo e preciso.

## What Changes

- Quando há uma janela de reunião detectada (Google Meet no navegador, Zoom, Teams), o início da gravação oferece "Gravar vídeo da reunião" ao lado de só áudio, no painel de opções da gravação e no aviso de reunião detectada. Só áudio continua o padrão.
- Com a opção ligada, o app grava só essa janela, sem som, junto com a sessão de gravação (pausa, retoma e termina com ela). Falha no vídeo nunca impede o áudio.
- O vídeo fica na pasta da sessão de gravação, ao lado do áudio, e é apagado junto com a nota.
- Com vídeo e motor de Notas = Claude Code, a geração da nota roda a skill `video-analysis` (script de evidências e leitura dos quadros) junto com a transcrição.
- Com outro motor de Notas, sem a skill ou sem as ferramentas dela na máquina, ou se a análise falhar, a nota sai só da transcrição e mostra um aviso dizendo que o vídeo não foi analisado e por quê.
- Exceção nova, estreita e documentada à regra de CLI sem ferramentas: só na geração de nota com vídeo, o `claude` pode rodar o script da skill sobre o vídeo daquela sessão e ler a pasta de evidências.
- Só macOS.

## Capabilities

### New Capabilities

- `meeting-video`: oferta, captura, armazenamento do vídeo da janela da reunião e uso dele na geração da nota.

### Modified Capabilities

- `llm-providers`: "Execução isolada de CLIs em geração pontual" ganha a exceção da geração de nota com vídeo no `claude`.

## Impact

- Captura nova (helper nativo com ScreenCaptureKit e AVFoundation), seguindo o contrato do helper de áudio do sistema (ADR-0004); `src-tauri/build.rs`, `tauri.macos.conf.json`, `Info.plist` (texto de uso da gravação de tela).
- Rust: `audio/capture.rs` (ciclo de vida junto da sessão), `commands.rs` (início com vídeo, detecção da janela), `domain/types.rs`, migração nova e `db/repositories.rs` (artefato de vídeo), `domain/processing.rs` e `providers/notes.rs` (geração com vídeo e aviso), `llm/cli.rs` e `llm/mod.rs` (concessão de ferramentas só para esse caso).
- Frontend: `NoteEditor.tsx` (painel de opções), `use-recording-start-actions.ts`, `src/lib/tauri.ts`, aviso de reunião detectada (`meeting_hud.rs` e a HUD), faixa de aviso na nota, i18n en e pt-BR.
- Docs: ADR nova (0071), `docs/llm-providers.md` (contrato de isolamento), `docs/audio-pipeline.md`, `CONTEXT.md` (termo novo), `PRIVACY.md`.
- Dependência da máquina do usuário: skill em `~/.agents/skills/video-analysis`, `python3` e `ffmpeg`/`ffprobe` no PATH do shell de login. Sem elas, só a transcrição.
- Disco: vídeo ocupa bem mais que o áudio.
