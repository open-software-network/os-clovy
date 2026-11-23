# Proposal

## Why

Toda transcrição de áudio do Clovy (ditado, notas e reuniões, prévia ao vivo) sai da máquina para a Groq com a chave do usuário. O dono quer uma segunda opção, escolhida pelo usuário: um modelo de fala instalado no próprio Mac que transcreve tudo sem rede, sem chave e sem limite de uso, com o português como idioma principal. O modelo Parakeet (família TDT 0.6B v3, 25 idiomas incluindo português) roda no Neural Engine via FluidAudio em cerca de 0,5 s por minuto de áudio, o que torna isso viável hoje.

## What Changes

- Novo motor de transcrição **Local (Parakeet)** ao lado do **Groq**, escolhido em Configurações > Modelos, na linha de Transcrição. A escolha vale para todas as transcrições: ditado, transcrição final de notas e reuniões (turnos, trechos e pacotes de turnos) e prévia ao vivo.
- Botão **Instalar modelo** nas Configurações: baixa o pacote Core ML do Parakeet Ultra (`FluidInference/parakeet-ultra-coreml`, ~630 MB, revisão fixa, SHA-256 por arquivo) para o diretório de dados do app, com progresso em bytes, cancelamento e estado persistente (ausente, baixando, pronto, falhou). Botão **Remover modelo** apaga os arquivos. O motor Local só pode ser escolhido com o modelo pronto.
- Novo helper nativo macOS `Clovy Local Transcriber` (Swift + FluidAudio), compilado e embarcado como os helpers já existentes, que mantém o modelo carregado e transcreve arquivos de áudio sob demanda, devolvendo texto, idioma e tempos por token.
- Com o motor Local escolhido, nenhuma requisição de transcrição sai da máquina, a chave Groq não é exigida, o ritmo de requisições e a espera por 429 não se aplicam, e falha do helper é erro claro da nota ou do ditado, sem cair na Groq.
- O motor Local aparece só no macOS com Apple Silicon (macOS 14+); no Windows e em Mac Intel a linha de Transcrição segue só com Groq.
- Documentação: `docs/audio-pipeline.md`, `PRIVACY.md`, `README.md` e novo ADR sobre o motor local.

## Capabilities

### New Capabilities
- `local-transcription`: instalação, remoção e estado do modelo de fala local; helper nativo que o carrega e transcreve; disponibilidade por plataforma.

### Modified Capabilities
- `dictation`: a transcrição do ditado vai ao motor de transcrição escolhido (Groq com chave própria ou Local), e a ausência de chave Groq só bloqueia o ditado quando o motor é Groq. Modifica "Transcrição direta com chave própria".
- `llm-providers`: a escolha de motor de transcrição passa a incluir Local; transcrição final e prévia ao vivo das notas seguem o motor escolhido; falha por chave ausente vale só para Groq. Modifica "Seleção de provedor por uso", "Notas com chave própria vão direto ao provedor" e "Falha clara sem chave nem servidor". O requisito "Limite de taxa da Groq na transcrição de nota" já é restrito à Groq e não muda.

## Impact

- **Rust (`src-tauri/`)**: `providers/mod.rs` (seleção `transcriptionProvider` aceita `local`, sanitização, catálogo), `providers/catalog.rs`, novo `providers/local_transcriber.rs` (cliente do helper), novo `transcription_model.rs` (download, verificação, estado, comandos Tauri), `domain/processing.rs` (`TranscriptionRoute`, `note_provider_call`, `note_timed_call`, `note_pack_transcriber`, `paced`), `dictation.rs` (`dictation_transcription_provider`, `ServiceDictationTranscriber`), `audio/live_preview.rs` (`live_preview_route`), `build.rs` e `tauri.macos.conf.json` (helper novo), `commands.rs` (registro de comandos).
- **Swift**: novo pacote `src-tauri/native/mac-local-transcriber/` com dependência FluidAudio (SPM, versão fixa), compilado com `swift build`; assinatura igual aos helpers atuais; mapeado em `bundle.resources`.
- **Frontend (`src/`)**: `components/settings/AppSettings.tsx` (linha de Transcrição com seletor Groq/Local, botão instalar/remover com progresso), `lib/tauri.ts` (DTOs e comandos novos), `i18n/messages/settings*.ts` (en e pt-BR), mensagens de falha de nota e ditado para o motor local.
- **Testes**: Rust com transcritor local falso nos caminhos de ditado, notas, pacotes e prévia; download com servidor HTTP falso; frontend para a linha de Transcrição e estados do modelo.
- **Dependências**: FluidAudio (Swift, Apache-2.0) fixada por tag; nenhuma crate Rust nova além das já presentes (`reqwest` stream, `sha2`, `tokio`).
- **Build/CI**: `swift build` do pacote do helper passa a fazer parte do build macOS; o CI macOS precisa resolver o pacote SPM (rede ou cache).
- **Sem mudança de banco**: o cache de trechos (`note_transcription_chunks`) já inclui modelo na chave; o modelo local entra como `parakeet-ultra`.
