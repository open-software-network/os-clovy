# Design

## Context

Ver `proposal.md` — Why. Estado atual que molda o desenho:

- Toda transcrição passa por `src-tauri/src/providers/groq.rs` (`dictate_transcribe_via`, `transcribe_saved_audio_via`, `transcribe_saved_audio_timed_via`). A orquestração está em `domain/processing.rs` (`TranscriptionRoute { groq_key, model, pacer }`, `note_provider_call`, `note_timed_call`, `note_pack_transcriber`, `transcribe_on_route`, `paced`), em `dictation.rs` (trait `DictationTranscriber`, `dictation_transcription_provider` recusa tudo que não é Groq) e em `audio/live_preview.rs` (`live_preview_route`, desligada sem chave Groq).
- O provedor é forçado a `groq` em `providers/mod.rs` (`sanitize_settings`, sanitizador de overrides, `set_provider_model_impl` com `KNOWN_TRANSCRIPTION_MODELS`) e em `providers/catalog.rs` (`ModelMode::Transcription`). `PROVIDER_LOCAL = "local"` já existe e não é usado para transcrição.
- Formatos: notas e prévia já chegam em WAV PCM16 16 kHz mono (`audio/turns.rs::normalize_wav_for_transcription`); o ditado no macOS grava AAC `.m4a` 44,1 kHz mono (helper Swift) e manda como está; no Windows normaliza para WAV 16 kHz.
- O pacote de turnos (`transcribe_turn_pack` + `split_pack_segments`) precisa de `TimedTranscription { segments: [{start_ms, end_ms, text}] }`.
- Helpers nativos macOS vivem em `src-tauri/native/<nome>/main.swift`, compilados por `swiftc` em `build.rs` para `.tauri-helper/<Nome>.app`, assinados (Developer ID ou ad hoc) e mapeados em `tauri.macos.conf.json > bundle.resources`. O driver de computer use usa outro padrão: `scripts/prepare-cua-driver.mjs` com arquivo de pin (hash da fonte, versão mínima de macOS) e `pnpm run computer-use:prepare` no `beforeBuildCommand`.
- Download de modelo com revisão fixa e SHA-256 por arquivo existe em `day_intelligence/embedder.rs` (`LocalEmbedder`, estados `Absent|Downloading|Ready|Failed`, sem progresso em bytes). Progresso por `tauri::ipc::Channel` existe em `updates.rs` (`DownloadEvent::Progress`).
- Invariante do projeto (`AGENTS.md`, ADR-0073): não existe fallback automático entre provedores; falta de chave falha claro.
- Change `remover-clovy-api` ainda ativa (tarefa 7.2 pendente) modifica os mesmos requisitos de `dictation` e `llm-providers`. Os deltas desta change partem do texto dessa change, e esta MUST ser arquivada depois dela.

## Goals / Non-Goals

**Goals:**
- Um único seam de transcrição no Rust, com dois backends (Groq, Local) escolhidos pela configuração, cobrindo ditado, turnos, trechos, pacotes e prévia.
- Modelo instalado pelo app, verificado, com progresso e estado observável; nunca baixado implicitamente por biblioteca.
- Modelo carregado em processo separado, vivo durante a sessão, liberado ao trocar de motor.
- Windows e Mac Intel sem regressão: o código do helper e do download fica atrás de `cfg(target_os = "macos")` e de verificação de arquitetura em runtime.

**Non-Goals:**
- Transcrição em streaming durante a gravação (Parakeet EOU / sliding window). A prévia ao vivo continua por trechos.
- Diarização, VAD ou limpeza de texto locais.
- Modelo local no Windows (ONNX) ou em Mac Intel.
- Trocar o modelo Groq ou a limpeza de ditado (OpenRouter).
- Escolha entre vários modelos locais: só um pacote, fixo.

## Decisions

### D1 — Modelo: `FluidInference/parakeet-ultra-coreml` (Parakeet Ultra), revisão fixa
Mesmos 25 idiomas, mesma velocidade e mesmo contrato de decodificação do Parakeet TDT 0.6B v3, mais preciso em todos os 24 idiomas FLEURS medidos pelo FluidAudio (português incluído) e em inglês; é o padrão recomendado pela biblioteca para integrações novas. Custo: ~630 MB em disco contra ~480 MB do v3. Requer macOS 14+, igual ao v3. O id de modelo na configuração e no cache de trechos é `parakeet-ultra`.
Alternativas: v3 (`parakeet-tdt-0.6b-v3-coreml`, menor, menos preciso); Fluid Speech Mini (só inglês, descartado pelo dono); redux (220 MB, pior em todos os idiomas, macOS 15+).
O worker fixa o commit do repositório HF e grava, em `src-tauri/resources/local-transcriber-model.json`, a lista de arquivos (`Preprocessor.mlmodelc`, `Encoder.mlmodelc`, `Decoder.mlmodelc`, `JointDecision.mlmodelc`, `parakeet_vocab.json`, conforme `Documentation/ASR/ManualModelLoading.md` do FluidAudio; conferir no commit fixado os nomes exatos que `AsrModels.load(from:version:.ultra)` exige), com tamanho e SHA-256 por arquivo. Os `.mlmodelc` são diretórios no HF: baixar cada arquivo interno pela API de árvore do HF, com hash por arquivo.

### D2 — Runtime: helper Swift próprio com FluidAudio, em processo separado
`src-tauri/native/mac-local-transcriber/` vira um pacote SwiftPM (`Package.swift` + `Sources/main.swift`) com dependência `FluidInference/FluidAudio` fixada por tag exata (a mais recente estável no momento da implementação; 0.18.0 na data desta proposta), `traits: []` quando o toolchain aceitar (Swift 6.2+), para não compilar TTS e diarização. Alvo macOS 14, só `arm64` (FluidAudio não tem ASR em Intel).
Protocolo: JSON por linha em stdin/stdout, uma requisição por vez.
- `{"op":"load","model_dir":"...","version":"ultra"}` → `{"ok":true,"load_ms":n}` ou `{"ok":false,"error":"..."}`
- `{"op":"transcribe","audio_path":"...","language":"pt"|null}` → `{"ok":true,"text":"...","tokens":[{"t":"...","start_ms":n,"end_ms":n}],"duration_ms":n}`
- `{"op":"unload"}` → `{"ok":true}`; fechar o stdin encerra o helper.
O helper usa `AsrManager.transcribe(URL, source: .system, language:)`, que converte qualquer formato (WAV, M4A) para 16 kHz mono Float32 com `AudioConverter`; por isso o ditado do macOS manda o `.m4a` como está, sem normalização nova. A dica de idioma vem de `configured_transcription_language()` (ditado) e do `language` da requisição de nota, mapeada para o enum `Language` do FluidAudio; quando o idioma não existe no enum, manda `nil` (o modelo detecta sozinho).
Alternativas: crate `fluidaudio-rs` (baixa modelo sozinha para cache próprio, sem progresso nem caminho configurável, SPM dentro do `cargo build`, Core ML no processo principal); Core ML direto do Rust (reimplementar mel + decoder TDT). Ambas descartadas; a escolha B foi do dono.

### D3 — Build e empacotamento do helper: script com pin, como o driver de computer use
`scripts/prepare-local-transcriber.mjs` roda `swift build -c release --arch arm64` no pacote, copia o executável para `.tauri-helper/Clovy Local Transcriber.app/Contents/MacOS/`, escreve `Info.plist` (bundle id no padrão dos helpers atuais, `LSMinimumSystemVersion` 14.0) e assina como `prepare-cua-driver.mjs` faz. Um arquivo de pin (`src-tauri/native/mac-local-transcriber/pin.json`) com SHA-256 da fonte e do `Package.resolved` evita rebuild quando nada mudou. Entra em `package.json` como `local-transcriber:prepare`, chamado em `beforeBuildCommand` e em `scripts/tauri-before-dev.mjs`; `tauri.macos.conf.json` mapeia `../.tauri-helper/Clovy Local Transcriber.app` para `native/bin/`. `build.rs` não muda (SPM não cabe em `swiftc`). Em Windows o script sai sem fazer nada. CI macOS: cache de `~/Library/Caches/org.swift.swiftpm` e `.build/`.
Alternativa: `swift build` dentro de `build.rs` — descartada, cada `cargo build` pagaria a resolução SPM e o `build.rs` já é longo.

### D4 — Seam no Rust: `TranscriptionBackend` em `providers/transcription.rs`
Enum `TranscriptionBackend { Groq { key, model }, Local { model_dir } }` resolvido uma vez por rodada a partir da configuração efetiva do perfil e do estado do modelo. `TranscriptionRoute` troca `groq_key: Option<String>` por `backend: TranscriptionBackend`; `pacer` passa a `Option` e é `None` para Local. `note_provider_call`, `note_timed_call`, `transcribe_on_route` e `ServiceDictationTranscriber` fazem `match` no backend: Groq segue exatamente o código atual; Local chama `providers/local_transcriber.rs`.
`local_transcriber.rs`: cliente do helper (`LocalTranscriber`, singleton em `tauri::State`, `Mutex` que serializa requisições; spawn preguiçoso na primeira chamada ou em `warm`; `kill` em `shutdown`, ao trocar o motor para Groq e ao remover o modelo). Converte tokens em `TimedTranscription`: agrupa tokens consecutivos e abre segmento novo quando o intervalo entre `end_ms` e o próximo `start_ms` passa de 400 ms (os pacotes Groq continuam separando turnos por 500 ms). Texto vazio → `no_speech`, como na Groq. Qualquer falha do helper → `AppError("local_transcriber_failed")` com a mensagem traduzida (`LOCAL_TRANSCRIBER_FAILED_MESSAGE` byte-idêntica no Rust e no i18n, como as mensagens Groq).
Portões Groq que passam a depender do backend: `note_route_from` (só exige chave com Groq), `note_pack_transcriber` (somente Groq; Local usa uma requisição por turno, conforme a alternativa aceita em Risks / Trade-offs e no adendo do ADR-0074), `live_preview_route` (habilitada com Local, sem orçamento), `dictation_transcription_provider` (aceita `local` quando o modelo está pronto), `maybe_post_process_note_transcript` (checa backend, não `groq`), limite de 25 MB (só Groq), `transcribe_with_transient_retries` (não repete falha local).
Chunking de 600 s com sobreposição de 10 s continua para Local: mantém o cache de trechos e a retomada iguais; o custo é desprezível a ~110× tempo real.

### D5 — Download e estado: `transcription_model.rs` seguindo `LocalEmbedder` + `Channel`
Diretório `<app_data_dir>/models/parakeet-ultra-coreml/`. `ModelState { Absent, Downloading { received, total }, Ready { bytes }, Failed { reason } }` derivado do disco ao abrir (manifesto + hashes conferidos uma vez, resultado guardado em `installed.json` com os hashes, para não re-hashear 630 MB a cada abertura; re-hashear só se `installed.json` faltar ou o tamanho de algum arquivo divergir). Download por `reqwest` stream com `Range` para retomar arquivo parcial (`.part`), hash incremental `sha2`, progresso emitido num `Channel<ModelDownloadEvent>` e também em evento Tauri `local-transcription-model-state` para a tela de Configurações refletir sem polling. Cancelamento por `CancellationToken`. Comandos Tauri: `local_transcription_model_state`, `install_local_transcription_model(channel)`, `cancel_local_transcription_model_install`, `remove_local_transcription_model`. Remover com motor Local escolhido grava `transcriptionProvider: "groq"` e devolve `switched_to_groq: true` para a UI avisar.

### D6 — Configuração e catálogo
`transcriptionProvider` aceita `"groq" | "local"`; `transcriptionModel` com Local é sempre `parakeet-ultra` (gravado junto). `sanitize_settings` e o sanitizador de overrides: em plataforma sem suporte, `local` → `groq`; em macOS arm64, `local` é mantido na leitura mesmo sem modelo pronto (o estado do modelo é dinâmico). Com `local` escolhido e o modelo não pronto (arquivos corrompidos ou removidos fora do app), o motor efetivo continua Local e a transcrição falha com `local_model_not_installed`, sem cair para a Groq (ADR-0073: nenhum áudio sai da máquina sem escolha explícita do usuário); a única troca para Groq é a feita pelo próprio comando de remoção, que grava `groq` e avisa. `set_provider_model` recusa gravar `local` sem modelo pronto (`local_model_not_installed`). O catálogo de `ModelMode::Transcription` devolve dois grupos: Groq (`whisper-large-v3-turbo`, `whisper-large-v3`) e Local (`parakeet-ultra`, com `available: bool`).
Detecção de plataforma: `cfg!(target_os = "macos") && std::env::consts::ARCH == "aarch64"`, exposto ao frontend em `ProviderModelSettingsDto.localTranscriptionSupported`.

### D7 — Interface
Linha "Transcrição" de `AppSettings.tsx` ganha, abaixo do `ModelRow`, um bloco de motor com dois cartões de rádio ("Groq — nuvem, chave própria" / "Local — Parakeet, no dispositivo, 25 idiomas") e, no cartão Local, o estado do modelo: `Instalar modelo (630 MB)`; barra com `123 MB / 630 MB` + `Cancelar`; `Pronto · 630 MB` + `Remover`; `Falhou: <motivo>` + `Tentar de novo`. O `ModelPickerDialog` mostra o modelo Groq só com motor Groq. Strings em `src/i18n/messages/settings*.ts` (en e pt-BR). Mensagens novas de falha em `notes.failure.localTranscriberFailed` e no HUD do ditado. Segue `~/.agents/skills/impeccable/SKILL.md` para o desenho.

### D8 — Testes
- Rust: `FakeLocalTranscriber` implementando o mesmo trait que `LocalTranscriber` (trait `LocalTranscribe` com `transcribe(path, language) -> Result<LocalTranscription>`), injetado por task-local nos testes de `processing.rs`, `dictation_tests.rs` e `live_preview`; servidor sentinela Groq com zero requisições em todos os caminhos com motor Local; testes do cliente do helper com um executável falso (script) que responde JSON; testes de download com servidor HTTP falso (`httptest`/`axum` já usados nos testes de provedores) cobrindo sucesso, hash divergente, cancelamento, retomada por `Range` e estado a partir do disco.
- Rust, atribuição por turno: três turnos com timestamps imprecisos reproduzem a ambiguidade de pacotes; Local deve usar três requisições individuais e guardar cada texto no turno correto, com zero rede. O intervalo dos pacotes Groq permanece 500 ms.
- Swift: teste de fumaça do helper no `scripts/local-transcriber-self-test.mjs` (como `computer-use-self-test.mjs`): carrega o modelo real se instalado e transcreve um WAV curto do repositório em pt-BR; pulado sem modelo.
- Frontend: `local-transcription-settings.test.tsx` para os estados do bloco e o seletor desabilitado.

## Risks / Trade-offs

- [SPM no build de release/CI precisa de rede para resolver o FluidAudio] → `Package.resolved` versionado, cache do SwiftPM no CI; o pin evita rebuild local.
- [FluidAudio muda nomes de arquivos ou o contrato de `AsrModels.load` entre versões] → tag exata do FluidAudio e commit exato do modelo no manifesto; atualizar os dois juntos.
- [`traits: []` exige Swift 6.2+; máquina do dono tem 6.4, CI pode ter outro] → `Package.swift` com `#if compiler(>=6.2)` para os traits; sem eles compila tudo (mais lento, mesmo binário funcional).
- [Primeira carga do modelo leva 20–30 s (compilação do Neural Engine)] → carregar ao escolher o motor Local e ao abrir o app com Local escolhido (`warm`), com estado "carregando" visível no HUD do ditado se a tomada chegar antes.
- [Memória ~600–800 MB do helper enquanto o motor Local está escolhido] → liberar ao trocar para Groq; documentar em `docs/audio-pipeline.md`.
- [Segmentação por pausa de 400 ms pode não casar com os 500 ms do pacote de turnos] → teste com pacote sintético de três turnos; se a divisão falhar, o caminho Local usa um turno por requisição (já existe para quem não tem chave Groq).
- [Parakeet não devolve `no_speech_prob`; silêncio pode virar texto ruidoso] → a pré-checagem de silêncio existente (`source_is_effectively_silent`) roda antes; texto vazio vira `no_speech`.
- [Dica de idioma do FluidAudio só filtra alfabeto (latino/cirílico), não força o idioma] → aceitável: o modelo detecta o idioma; documentar que a configuração de idioma do ditado é dica.
- [Download de 630 MB com conexão ruim] → retomada por `Range` e por arquivo; tentativa só do que falta.
- [Duas changes ativas tocam os mesmos requisitos] → arquivar `remover-clovy-api` antes desta; os deltas aqui já partem do texto dela.

## Migration Plan

1. Entrega em branch própria; sem mudança de banco nem de formato de configuração além do valor novo `local`.
2. Instalações existentes continuam em Groq; nada muda até o usuário instalar o modelo e escolher Local.
3. Rollback: voltar o binário. Versões anteriores forçam `groq` em `sanitize_settings`, então uma configuração salva com `local` cai em Groq sem erro. O diretório do modelo fica órfão e pode ser apagado à mão.
4. Após o merge: `docs/audio-pipeline.md`, `PRIVACY.md` (áudio não sai da máquina com Local), `README.md`, ADR novo `docs/adr/ADR-00xx-motor-de-transcricao-local.md`.

## Open Questions

- Exibir o idioma detectado pelo modelo na nota (o helper pode devolvê-lo) — não muda specs; decidir na implementação se o campo `language` de `TranscriptionProviderResult` recebe o detectado ou o configurado.
