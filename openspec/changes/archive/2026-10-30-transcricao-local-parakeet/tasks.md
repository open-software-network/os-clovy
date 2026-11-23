# Tasks

## Aceitação atual, decisão do dono em 2026-10-29

O dono aceitou a prova nativa observada na própria sessão e determinou **não executar novos testes nativos nem tocar no app em uso**. Os itens 1.1, 1.3, 4.2 e 5.1–5.3 estão fechados por implementação, evidência automatizada e essa aceitação explícita; os critérios manuais dispensados abaixo **não foram executados nem comprovados**. O gate 6.3 segue a regra de convergência do dono em 2026-10-30, registrada abaixo. Os registros anteriores que dizem “aguarda”, “não instalado” ou “aberto” são histórico preservado, substituído por esta decisão.

Verificação automatizada final após a aceitação: `logs/final-owner-accepted-verify.log`, `make -k verify` exit 2 **somente** pelos dois erros de formatação preexistentes autorizados (`.mcp.json`, `opencode.json`). Frontend: 204 suítes/1.916 testes verdes; Rust desktop: 2.187 testes verdes/21 ignorados; crates companion: 33 testes verdes; typecheck, fmt, Clippy all-targets e guardiões de referências passaram. Não houve novo teste ou controle do app nativo. O hash do WIP protegido permanece `18a1176bd467b6ad1741ce303f1e51415a15778614291e165f005eb0557682c5`.

Estado final da revisão: rodada 21 concluiu ambos os revisores, exit 6, somente três minors refutados em 6.3 conforme a regra explícita do dono de 2026-10-30; não se declara exit 0. Summary: `/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/21/summary.md`. Entrega encerrada com evidências e limitações preservadas; sem push, merge ou archive.

Atualização do dono para a rodada 8: o patch externo do agente **não foi autorizado e não foi aplicado**. O dono corrigiu o harness no commit `5fb88a257d1593914908f5214875eba80d385b8b` e informou 32 testes verdes; a versão ativa exclui binários detectados pelo Git do escopo textual esperado pelo CodeRabbit. A licença no proposal foi explicitamente autorizada e corrigida para Apache-2.0.

Fechamento da rodada 7 para a nova revisão: a captura do backend do ditado foi corrigida em `9e3d77a3`, depois do vermelho real em `logs/review-5-regressions-red.log` (o transcritor relia Groq após um await) e verde em `logs/review-5-fixes-green.log`. A mesma regressão agora também cobre helper Local que falha após a seleção mudar: resultado continua Local quando disponível, ou erro `local_transcriber_failed` com mensagem canônica no HUD, descarte da tomada e nenhuma chamada à sentinela Groq. Não foi fabricada uma nova falha em código já corrigido. A regra expressa pelo dono aceita transcrever pelo backend capturado **ou falhar claramente**, mantendo D4 de encerrar o helper ao escolher Groq.

Os outros achados da rodada 7 já foram reproduzidos e corrigidos em `63cd7900`: `.part` completo é conferido por hash e reutilizado; arquivo completo corrompido exige GET novo. O teste de três turnos reproduz a fusão por timestamps atrasados, mantém `PACK_GAP_MS == 500` por asserção explícita e exercita a alternativa de um turno por chamada Local já aceita no design, com texto no índice correto e zero rede. A agregação de 400 ms e o empacotamento Groq não foram alterados. Evidências vermelho→verde: `logs/review-7-regressions-red.log`, `logs/review-7-regressions-green.log`.

`logs/review-8-guard-regressions.log`, exit 0: nove regressões passaram, incluindo resultado Local ou erro claro no HUD após mudança de seleção, zero conexões à sentinela, .part completo/corrompido, cancelamento, limite de progresso e split com gap preservado em 500 ms. Nenhum teste nativo do app foi feito.

Evidência coletada sem alterar o perfil: `run-native-1/owner-observed/20261029T230729Z/`, sob `.tmp/verify/transcricao-local-parakeet/`. Contém cópias de `dictation-events.log` e `installed.json`, texto e metadados da transcrição em `note-transcripts.txt/json`, recorte `dictation-owner-take.json`, cálculo `dictation-owner-timing.json` e saída de `pgrep`. Comandos: `logs/native-owner-evidence-collect.log` e `logs/native-owner-transcript-export.log`, ambos exit 0. SQLite foi aberto somente para leitura; nenhum áudio, chave ou banco inteiro foi copiado.

Comprovado: instalação pelo botão, conforme sessão confirmada pelo dono; UI real mostrou progresso e Pronto; payload de 632.162.964 bytes e recibo no perfil de prova. `provider-settings.json` seleciona Local. Uma transcrição de reunião de 98 caracteres tem `provider=local`, `status=succeeded`, span 3.810–10.200 ms. Um ditado tem `final_transcript` de 14 caracteres em 22:54:45.116229 UTC; o log registra key-up em 22:54:44.697776 (**418,453 ms**) e recording_ready em 22:54:44.773993 (**342,236 ms**). O “340 ms após key-up” relatado pelo dono corresponde aproximadamente ao segundo intervalo; os timestamps originais estão preservados. Houve erro `accessibility_permission_missing` 4,957 ms depois do texto final; a colagem não foi comprovada.

Também observado pelo agente: Pronto e Local selecionado persistiram após reabrir o bundle debug atualizado (`run-native-1/ready-after-restart.png`); isolamento pelo `lsof` em `logs/native-review-7-isolation.log`, exit 0. O login item passou a apontar para o bundle de prova; foi desativado somente pela UI Geral e sua ausência confirmada em `logs/native-proof-autostart-cleanup.log`, exit 0. Nenhum TCC ou Keychain foi alterado pelo agente.

Não comprovado, aceito pelo dono sem novos testes: três tomadas com segunda/terceira mais rápidas; nota ≥3 min com prévia ao vivo; retry nativo; nettop/proxy sem Groq; pgrep após troca/remoção e aviso de remoção; colagem por Acessibilidade; cancelamento/retomada nativos e quatro estados capturados no bundle; self-test CLI com pesos reais. O pgrep coletado teve exit 1/saída vazia após o app estar encerrado, portanto **não** comprova encerramento causado pela troca/remoção. As quatro capturas com IPC simulado continuam sendo evidência apenas de renderização.

Antes da instrução explícita de não encerrar a instância usada pelo dono, o agente abriu PID 4676 do snapshot atualizado e o encerrou para parar uma tomada de solicitação de permissão cujo HUD não pôde controlar (`logs/native-permission-probe-stop.log`). Não se atribui essa tomada à aceitação. A partir da instrução do dono, não houve novo lançamento, encerramento ou interação com o app. O perfil e os dados foram preservados.

Ordem de dependência: grupo 1 (helper) e grupo 2 (modelo) são independentes entre si; grupos 3 e 4 dependem dos dois; grupo 5 depende do 3; grupo 6 depende de tudo. Todo teste Rust segue vermelho→verde (`/test-driven-development`); comandos canônicos: `cargo test --manifest-path src-tauri/Cargo.toml --locked`, `pnpm test`, `pnpm typecheck`, `pnpm check`, `make verify`.

## 1. Helper nativo `Clovy Local Transcriber`

- [x] 1.1 Criar o pacote SwiftPM em `src-tauri/native/mac-local-transcriber/` (`Package.swift` com FluidAudio em tag exata, `traits: []` sob `#if compiler(>=6.2)`, alvo macOS 14 arm64; `Package.resolved` versionado) e `Sources/main.swift` com o protocolo JSON por linha do design D2 (`load`, `transcribe`, `unload`; tokens com `start_ms`/`end_ms`; dica de idioma mapeada para `Language`, `nil` quando não existir). Verificar: `swift build -c release` sai 0 e, com um diretório de modelo real, `echo '{"op":"load",...}' | .build/release/clovy-local-transcriber` responde `{"ok":true}`.
- [x] 1.2 Criar `scripts/prepare-local-transcriber.mjs` (build, `.tauri-helper/Clovy Local Transcriber.app` com `Info.plist` e `LSMinimumSystemVersion` 14.0, assinatura igual a `prepare-cua-driver.mjs`, pin por SHA-256 da fonte e do `Package.resolved`, no-op fora de macOS), script `local-transcriber:prepare` em `package.json`, chamada em `beforeBuildCommand` e em `scripts/tauri-before-dev.mjs`, mapeamento em `tauri.macos.conf.json > bundle.resources`. Verificar: rodar o script duas vezes — a segunda não recompila; `codesign --verify --deep --strict` no `.app` sai 0.
- [x] 1.3 Criar `scripts/local-transcriber-self-test.mjs` com um WAV curto em pt-BR em `src-tauri/tests/fixtures/` (gravado pelo worker ou sintetizado com `say -v Luciana` + `afconvert`): carrega o modelo instalado em `<app_data_dir>/models/parakeet-ultra-coreml`, transcreve e confere que o texto contém as palavras esperadas; sai 0 com aviso quando o modelo não está instalado. Verificar: execução com o modelo instalado mostra o texto em português; documentar o comando em `docs/audio-pipeline.md`.

Evidência parcial do grupo 1 (2026-10-29): Swift 6.4 compilou o helper arm64 (exit 0, `.tmp/verify/transcricao-local-parakeet/logs/swift-build-second.log`). Empacotamento e assinatura verificados em `helper-prepare-fixed.log`; segunda execução sem recompilar em `helper-reuse.log`; protocolo sem modelo devolve erro local e aceita unload em `helper-protocol-fixed.log`. Fixture pt-BR sintetizada, `portuguese-fixture-convert.log`; self-test sem modelo pula explicitamente (`helper-self-test-skip.log`). 1.1 e 1.3 aguardam a prova com modelo instalado pelo app.

A API conferida na tag v0.18.0 (`e15287985625dbf544a343df679436789ffb03fb`) oferece `AsrModels.loadLocal`, que falha sem rede; `load` chama ModelHub e pode baixar. Foi usado `loadLocal` para cumprir D2/D4 e zero rede. A API de transcrição dessa tag recebe `decoderState` explícito, reiniciado por requisição, no lugar do antigo argumento `source`. Manifesto Swift mínimo 6.1: `#if compiler(>=6.2)` sozinho não permite traits sob PackageDescription 6.0. Em Swift 6.1 a dependência usa a variante sem traits.

## 2. Instalação, estado e remoção do modelo

- [x] 2.1 Fixar o commit de `FluidInference/parakeet-ultra-coreml` e gerar `src-tauri/resources/local-transcriber-model.json` (lista de arquivos internos dos `.mlmodelc` + `parakeet_vocab.json`, tamanho e SHA-256 por arquivo, total em bytes) com um script reproduzível `scripts/pin-local-transcriber-model.mjs`. Verificar: rodar o script duas vezes gera o mesmo JSON; o total bate com a soma dos tamanhos.
- [x] 2.2 Criar `src-tauri/src/transcription_model.rs` (`cfg(target_os = "macos")`): `ModelState`, derivação do estado a partir do disco com `installed.json`, download por `reqwest` stream com `Range` e `.part`, hash incremental, `CancellationToken`, progresso por `Channel<ModelDownloadEvent>` e evento `local-transcription-model-state`, remoção. Testes com servidor HTTP falso: sucesso, hash divergente mantém íntegros e falha com nome do arquivo, cancelamento volta a ausente, retomada baixa só o que falta, estado a partir de diretório completo e parcial. Verificar: testes vermelhos antes, verdes depois no `cargo test`.
- [x] 2.3 Comandos Tauri `local_transcription_model_state`, `install_local_transcription_model`, `cancel_local_transcription_model_install`, `remove_local_transcription_model` registrados em `commands.rs`/`lib.rs`; remoção com motor Local grava `groq` e devolve `switched_to_groq`. Em plataformas sem suporte os comandos devolvem `local_transcription_unsupported`. Verificar: teste Rust da remoção com motor Local; `cargo build` no alvo Windows (`cargo check --target x86_64-pc-windows-msvc` se o toolchain existir, senão `cfg` revisado) sem erro.

Evidência do grupo 2 (em andamento): 2.1 fixou HF `95eaa59a39d4394f047a4dc5cce480388a60d1b6`, 18 arquivos, 632.162.964 bytes, SHA-256 do JSON `1a0850e90f792a48768038db301e1c440cf930438ab34dc48bafd3a1dd9d5310`. Duas execuções idênticas: `.tmp/verify/transcricao-local-parakeet/logs/pin-model-first.log` e `pin-model-second.log`, exit 0. O manifesto inclui `JointDecisionv3.mlmodelc`, confirmado na tag do runtime. Pesos LFS foram somente consultados por metadados; pequenos blobs Git foram conferidos em memória, sem instalação de arquivos de modelo.

2.2: vermelho real com 4 falhas (`logs/model-tests-red.log`, exit 101) antes da implementação; os mesmos 4 testes passaram (`logs/model-tests-green.log`, exit 0). Cobertura: bytes/progresso, hashes, corrupção com preservação dos íntegros, cancelamento, retomada HTTP Range, diretório parcial/completo, marcador de instalação e remoção. Caminhos sob `.tmp/verify/transcricao-local-parakeet/`. Nenhuma crate adicionada: o token de cancelamento usa `tokio::sync::watch`, já disponível, e interrompe tanto request quanto stream.

2.3: remoção da seleção e overrides passou vermelho→verde em `logs/model-removal-red.log` (exit 101, 1 falha) e `logs/model-removal-green.log` (exit 0, 1 teste). Os quatro comandos foram registrados no handler Tauri. Implementação de download/estado é cfg macOS; comandos recusam plataforma sem suporte antes de acessar o estado. A verificação cruzada Windows e o encerramento do helper serão conferidos com a integração dos grupos 3/6.

## 3. Seam de transcrição no Rust

- [x] 3.1 Criar `providers/transcription.rs` com `TranscriptionBackend { Groq { key, model }, Local { model_dir } }`, `effective_transcription_backend(settings, model_state, platform)` e a detecção de plataforma; `providers/mod.rs` aceita `transcriptionProvider: "local"` (sanitização por plataforma, override de perfil, `set_provider_model` recusa `local` sem modelo pronto com `local_model_not_installed`, grava `parakeet-ultra` como modelo); `catalog.rs` devolve os grupos Groq e Local com `available`. Testes unitários de cada regra de D6. Verificar: testes de `providers/mod.rs` e `catalog.rs` verdes; fixture `provider-settings.json` com `local` em plataforma sem suporte lê Groq.
- [x] 3.2 Criar `providers/local_transcriber.rs`: trait `LocalTranscribe`, `LocalTranscriber` (spawn preguiçoso do helper de `native/bin/`, `Mutex` serializando requisições, `warm`, `shutdown`, kill no drop), conversão de tokens em `TimedTranscription` com corte em pausas ≥ 400 ms, texto vazio → `no_speech`, falha → `local_transcriber_failed` com `LOCAL_TRANSCRIBER_FAILED_MESSAGE`. Testes com executável falso (script que responde JSON, trava ou falha) contando cargas e conferindo segmentos. Verificar: testes verdes; teste de que trocar para Groq e remover o modelo encerram o helper.
- [x] 3.3 `domain/processing.rs`: `TranscriptionRoute.backend`, `pacer: Option`, `note_route_from` só exige chave com Groq, `note_provider_call`/`note_timed_call`/`note_pack_transcriber`/`transcribe_on_route` com `match` no backend, limite de 25 MB e `transcribe_with_transient_retries` só para Groq, `maybe_post_process_note_transcript` pelo backend. Testes com `FakeLocalTranscriber` por task-local: turno, trecho de 600 s, pacote de três turnos dividido pelos tempos, nota de 40 min sem pausa de ritmo, falha local vira erro da nota com áudio salvo, servidor sentinela Groq com zero requisições em todos. Verificar: `cargo test` verde, incluindo os testes Groq existentes intactos.
- [x] 3.4 `dictation.rs`: `dictation_transcription_provider` aceita Local com modelo pronto; `ServiceDictationTranscriber` faz `match` no backend e manda o `.m4a` como está; sem chave Groq e motor Local, o ditado transcreve; falha local mostra o HUD "modelo local falhou"; `warm_dictation_route_if_idle` aquece o helper com motor Local. Testes em `dictation_tests.rs` com transcritor local falso e sentinela Groq. Verificar: testes verdes; mensagem do HUD em en e pt-BR.
- [x] 3.5 `audio/live_preview.rs`: `live_preview_route` habilitada com motor Local, sem orçamento de segundos por hora. Teste do portão com e sem modelo pronto e do trecho transcrito localmente. Verificar: testes verdes.

Evidência do grupo 3: seleção/sanitização/catálogo tiveram 3 falhas reais (`logs/backend-settings-red.log`); resolver teve 3 falhas (`logs/backend-resolver-red.log`); os 7 passaram em `logs/backend-settings-green.log`. Cliente do helper: 3 falhas antes (`logs/helper-client-red.log`), 4 testes verdes depois (`logs/helper-client-green.log`), incluindo reuso de load, serialização, encerramento/reap, timeout, EOF/protocolo e segmentos com pausa de 400 ms. Rotas/ditado: 3 falhas (`logs/local-routes-red.log`) e 37 verdes (`logs/local-routes-green.log`). Integração ampliada: 39 verdes (`logs/local-routes-integration-fixed.log`), incluindo WAV sintético de 40 minutos com chunks de 600 s, split de 3 turnos, preview sem orçamento, ditado M4A com idioma pt e falha local, sentinelas HTTP com zero conexões. Os arquivos ficam sob `.tmp/verify/transcricao-local-parakeet/`.

A suíte Rust completa passou em `logs/all-rust-tests-fixed.log`, exit 0, incluindo todas as suítes Groq existentes e integrações. A rodada anterior encontrou o guardião `hide_console` (2.012 testes passavam, 1 falhava); o spawn foi corrigido para o construtor canônico. Preview continua usando `audio/live_preview.rs` sem novo transporte: seus portões foram corrigidos em `audio/capture.rs` e `domain/processing.rs`. Falta medir o helper real na prova nativa. D6 tem uma frase contraditória sobre fallback sem modelo; a implementação segue a instrução explícita do objetivo e ADR-0073: erro local, nunca Groq automático. Isso foi registrado no ADR-0074; o texto da proposta/spec não foi alterado.

Regressões adicionais: remoção concorrente teve vermelho→verde (`logs/removal-race-red.log` e `logs/removal-race-green.log`, 5 testes do instalador). Arquivo acima de 25 MB passou pelo transcritor falso sem rede. A prova persistida de falha local encontrou a agregação que substituía o código por `transcription_failed` (`logs/local-final-regressions-fixed.log`); corrigida sem alterar Groq, ficou verde em `logs/local-note-failure-green.log`. Confere estado Failed, frase canônica, áudio preservado e sentinela sem chamadas. O helper limita timestamps do último frame à duração real, conforme a implementação TDT da tag (`logs/helper-timing-build.log`, exit 0). Assinatura existente é preservada sem identidade explícita (`logs/helper-signature-preserve.log`, exit 0).

## 4. Interface das Configurações

- [x] 4.1 `lib/tauri.ts`: `ProviderModelSettingsDto.localTranscriptionSupported`, DTO `LocalTranscriptionModelState`, funções dos quatro comandos novos e inscrição no evento `local-transcription-model-state`. Verificar: `pnpm typecheck` verde.
- [x] 4.2 `AppSettings.tsx`: bloco de motor na linha Transcrição (cartões Groq/Local; estados ausente/baixando com bytes e Cancelar/pronto com tamanho e Remover/falhou com motivo e Tentar de novo; Local desabilitado com "Instale o modelo para usar"; aviso ao remover com Local escolhido), `ModelPickerDialog` mostra modelo Groq só com motor Groq, bloco oculto quando `localTranscriptionSupported` é falso; strings en e pt-BR em `settings*.ts`; `notes.failure.localTranscriberFailed` byte-idêntica à constante Rust. Seguir `~/.agents/skills/impeccable/SKILL.md`. Testes `local-transcription-settings.test.tsx` para cada estado e `note-failure-banner.test.tsx` para a frase nova. Verificar: `pnpm test` verde, `pnpm check` sem erros novos; captura de tela da linha nos quatro estados no app de dev.

Evidência do grupo 4: bindings com 2 testes vermelhos (`logs/frontend-bindings-red-targeted.log`); estados da UI com 4 falhas reais (`logs/frontend-settings-red.log`); frase local com 1 falha (`logs/note-local-message-red.log`). Os 23 testes passaram em `logs/frontend-local-green-second.log`; `logs/local-typecheck.log` exit 0. `4.2` implementada, mas permanece aberta pela captura exigida no bundle nativo. Capturas reais do componente AppSettings no Chrome com IPC simulado: `ui/absent.png`, `ui/downloading.png`, `ui/ready.png`, `ui/failed.png`, `ui/compact.png`; zero erros JS (`logs/browser-settings-final.log`). Elas provam renderização, não instalação ou transcrição. Impeccable detector exit 0 (`logs/design-detector.log`) e revisor visual independente `disposition: ship` (`ui/finish-review.md`). O seletor CSS :has foi substituído por atributos de estado após revelar incompatibilidade jsdom; 59 testes do bloco e conectores passaram em `logs/css-selector-regression-fixed.log`.

A suíte frontend completa passou: `logs/frontend-final-full.log`, 204 suítes/1.911 testes, exit 0. A falha de rede é traduzida em en/pt-BR, com vermelho→verde em `logs/download-network-copy-red.log` e `logs/download-network-copy-green.log` (6 testes). O fixture do launcher agora simula também o preparo do helper (`logs/frontend-launcher-fixed.log`).

## 5. Prova no app de dev (macOS, Apple Silicon)

- [x] 5.1 Instalar o modelo pelo botão no app de dev (bundle debug de identidade `dev.guilhermexp.clovy.parakeetproof`, nunca o de release): progresso visível, cancelamento no meio e retomada, estado "pronto" após reabrir o app. Verificar: capturas de tela e `installed.json` com hashes em `~/Library/Application Support/dev.guilhermexp.clovy.parakeetproof-dev/models/parakeet-ultra-coreml/`.
- [x] 5.2 Com motor Local e a chave Groq removida: três tomadas de ditado em português (a segunda e a terceira mais rápidas que a primeira, por `dictation-events.log`), uma nota gravada de ≥ 3 min com prévia ao vivo ligada, e um "Tentar de novo" numa nota. Verificar: transcritos em português corretos; `dictation-events.log` com os tempos; monitor de rede (`nettop` ou proxy local) sem conexão a `api.groq.com` durante os testes.
- [x] 5.3 Trocar para Groq e conferir que o helper encerra (`pgrep -f "Clovy Local Transcriber"` vazio); remover o modelo com Local escolhido e conferir o aviso e a troca para Groq. Verificar: capturas e saída do `pgrep`.

## 6. Documentação e integração

- [x] 6.1 `docs/audio-pipeline.md` (backend Local, protocolo do helper, memória, comando do self-test), `PRIVACY.md` (áudio não sai da máquina com Local), `README.md` (como instalar o modelo), ADR novo `docs/adr/` sobre o motor local sem fallback, `CONTEXT.md` (glossário: motor de transcrição, modelo local). Verificar: `make org-references-check clovy-api-references-check` e demais checks de docs do `make verify` verdes.
- [x] 6.2 `make verify` completo verde (frontend, Rust, clippy, fmt, biome) e `openspec validate transcricao-local-parakeet --strict`. Verificar: logs em `.tmp/verify/transcricao-local-parakeet/`.
- [x] 6.3 Gate de revisão `scripts/review-pair.sh --gate` concluído na rodada 21, exit 6, ambos os revisores completos e somente minors refutados abaixo, encerramento autorizado pela regra de convergência do dono em 2026-10-30; summary e refutações incluídos no commit final.

Evidência do grupo 6: documentação implementada; `logs/documentation-reference-checks.log` exit 0 e `logs/openspec-validate.log` exit 0. `pnpm check` e `make verify` encontram apenas formatação preexistente em `.mcp.json` e `opencode.json`, que o dono mandou preservar; não foram alterados nem staged. `logs/scoped-final-frontend-checks.log` comprova typecheck e Biome em `src/` e `scripts/`. A rodada `make -k verify` roda os demais alvos; seu frontend capturou o teste de rede ainda vermelho durante implementação, substituído pela suíte final verde acima. Não se declara `make verify` verde.

A prova nativa aguarda resolver com o dono o conflito de identidade: o brief fixa June/dev antigo, enquanto o AGENTS.md mais recente exige identidade Clovy e perfil próprio. Nenhum app foi aberto, peso instalado, Keychain ou TCC alterado. Os itens que dependem do modelo real continuam abertos.

### Revisão pareada, rodada 1

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/1/summary.md`, exit **10**. Commit final bloqueado. OpenQodex também declarou revisão incompleta: os quatro caches `.trail/cache/.cache/ask-index.json`, `extract.b3c5002f26d64244.json`, `extract.c3679fb5d24a63a0.json` e `.trail/cache/.graph/wiring.json` não couberam na cobertura; são tooling preexistente fora do escopo. O snapshot tinha 81 arquivos/402.711 linhas por incluí-los. Nenhum desses arquivos foi staged.

Tratamento de todos os achados:

- `.cursor/hooks.json:7`, dependência de outro worktree: fora desta entrega, alteração preexistente que o dono mandou preservar. Refutado como regressão deste trabalho; não foi editado nem staged. O diff de `f58ba8ec` contra a base não inclui `.cursor/`.
- `.trail/hook.cjs:143`, instalação sem sfw (dois achados duplicados): tooling preexistente, fora do escopo e nunca seguido nesta execução. Nenhuma instalação global foi feita. Refutados como regressões desta entrega; arquivo não foi staged.
- Ordem do helper em `tauri-before-dev.mjs`: confirmado. Regressão reproduzida no launcher (`logs/launcher-order-red.log`, 1 falha) e corrigida (`logs/launcher-order-green.log`, 2 testes). `tauri-dev.mjs` e `tauri-build.mjs` preparam Local antes de qualquer Cargo do helper Computer use. O hook beforeBuild também prepara Local primeiro.
- Modelo Local encaminhado à Groq durante troca: confirmado; regressão vermelha registrada em `logs/final-ci-parity.log` e `logs/route-engine-switch-red.log`. A correção recusa a rota Groq quando o modelo capturado da rodada é Local; verificação verde (`logs/route-engine-switch-green.log`, exit 0, 1 teste). O run Local deve falhar localmente quando desativado, preservando o áudio para retry com a seleção explícita atual; não pode enviar `parakeet-ultra` ou áudio Local à Groq.
- `installed.json` evita rehash com tamanho igual: refutado por decisão **D5**, já aceita e imutável no escopo. `design.md:58` exige explicitamente hashes conferidos uma vez, rehash somente sem marcador ou com tamanho divergente. A implementação corresponde a essa decisão; corrupção externa de mesmo tamanho após a instalação é um limite conhecido dessa otimização.
- Helper de desenvolvimento no build de release: confirmado. Hook macOS passa `--release`, e o wrapper prepara o helper antes do Cargo com o perfil solicitado. Não foi criado bundle de release para validar isso; a checagem é de configuração e os builds de prova são debug.

Gate de segredos: todos os 24 registros são falsos positivos do cache de extração do Trail. Tokens sintéticos foram comparados em memória com os fixtures de `src-tauri/src/activity/redact.rs`. Os dois hashes classificados como chave privada correspondem a spans de 5.234/5.222 bytes do código de regex/teste serializado em JSON, não PEM válido (OpenSSL rejeitou); o segundo cache contém exatamente os mesmos hashes. Nenhum valor foi exibido. Evidência: `logs/verify-secret-fixtures-final.log` (exit 0, 24 verificados/0 pendentes), `secret-fixture-verification.json` e allowlist exata por regra/caminho/SHA-256 em `secret-fixture-allowlist.json`, todos sob `.tmp/verify/transcricao-local-parakeet/`. Rodada seguinte deve usar essa allowlist; nunca liberar por exit 10/4.

### Revisão pareada, rodada 2 e verificações finais

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/2/summary.md`, exit **4**. A allowlist validada eliminou os falsos positivos de segredo. OpenQodex continua incompleto por caches preexistentes; CodeRabbit recusou o payload acima de 100 MiB (`payload_too_large`). Não é quota. A autorização do dono para revisar uma cópia isolada contendo somente a entrega está pendente; nenhum cache protegido foi removido ou staged para contornar o gate.

Tratamento dos cinco achados:

- `.cursor/hooks.json:7` e `.trail/hook.cjs:143`: mesmas alterações preexistentes fora do escopo, refutadas como regressões desta entrega na rodada 1. Permanecem intactas.
- `scripts/tauri-dev.mjs:55`: confirmado. O caminho do Xcode era definido somente depois do preparo Swift. Teste reproduziu a falha (`logs/launcher-xcode-red.log`, 1 falha); o launcher agora resolve e passa `DEVELOPER_DIR` aos dois helpers antes de compilar, sem mudar o Xcode global. Resultado da correção em `logs/launcher-xcode-green.log` (exit 0, 2 testes).
- `transcription_model.rs:325`: mesmo comportamento exigido por D5; refutado como na rodada 1. Sem mudança da decisão aceita.
- `LocalTranscriptionSettings.tsx:193`: confirmado. Remover já persiste Groq antes de apagar arquivos; a UI agora atualiza a seleção também quando o filesystem falha. Teste vermelho→verde em `logs/settings-error-recovery-red.log` e `logs/settings-error-recovery-green.log`. A mesma rodada cobre não duplicar o erro técnico de rede abaixo do estado traduzido (10 testes de UI/launcher e typecheck verdes).

Resultados completos após as correções Rust/UI: `logs/rust-after-review.log` exit 0 (fmt, Clippy all-targets e cargo test --locked, 2.176 testes passaram, 21 ignorados); `logs/frontend-after-review-full.log` exit 0 (204 suítes, 1.913 testes). A última correção do launcher tem regressão focada separada. `logs/final-scoped-checks.log` exit 0: Biome em src/scripts, typecheck e OpenSpec estrito. Capturas atualizadas em `ui/final/` com zero erros JS (`logs/browser-final-code.log`). O servidor de preview criado nesta execução foi encerrado (`logs/stop-owned-preview.log`).

Empacotamento debug: `logs/debug-bundle-direct.log`, `debug-bundle-inspection.log` e `debug-bundle-snapshot.log` exit 0. Bundle arm64 com identidade compilada `dev.guilhermexp.clovy.parakeetproof`, assinatura verificada, copiado à evidência, nunca aberto. Esse artefato antecede as últimas correções de recuperação da UI e não comprova o comportamento nativo final. Nenhum modelo foi baixado. Windows: `logs/windows-cross-check.log` e `windows-rustup-check.log` exit 101; o Rust Homebrew não tem std Windows, e o Rustup instalado avançou até faltar `assert.h` do SDK Windows. Nenhum toolchain foi instalado. Revisão cfg confirma que a implementação nativa só é acessada em macOS e que comandos recusam plataformas não suportadas antes do acesso ao estado; execução Windows permanece não comprovada.

## Workflow follow-up

- Arquivar `remover-clovy-api` antes desta change; só então `openspec archive transcricao-local-parakeet`.
- Após o arquivamento, `/sync` do `AGENTS.md` do projeto e `/cleanup` da branch.

### Continuação autorizada pelo dono (2026-10-29)

O dono corrigiu D6 e o cenário de override para erro `local_model_not_installed`, sem fallback. O perfil autorizado para a prova é `dev.guilhermexp.clovy.parakeetproof`, com dados sintéticos novos em `dev.guilhermexp.clovy.parakeetproof-dev`; as referências June de 5.1 ficaram obsoletas. Prova segue AGENTS.md; solicitações de permissão devem parar a execução até confirmação do dono.

Gate: autorizado commit intermediário de toda a entrega, stash temporário somente de `.claude .cursor .mcp.json AGENTS.md opencode.json`, revisão sem `--include-untracked` e restauração imediata do stash. Os erros Biome em `.mcp.json` e `opencode.json` são aceitos pelo dono como preexistentes; os demais alvos de `make verify` continuam obrigatórios.

Contrato D6 revisado: `logs/profile-override-final-contract.log` exit 0, teste `local_profile_override_without_model_fails_before_groq_credentials_or_network` passou e `openspec validate transcricao-local-parakeet --strict` passou. A base global é Groq, o override efetivo é Local, ausência do modelo devolve o código local antes de ler a chave, e a sentinela não recebe conexões.

### Revisão pareada, rodada 3

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/3/summary.md`, exit **4**: OpenQodex concluiu; CodeRabbit pediu espera de 1 minuto por quota. O snapshot teve 55 arquivos/4.124 linhas, sem untracked. Toda a entrega estava no commit intermediário `872152981837dd952faf714a2a8d92564767c15b`. Stash/pop autorizado concluído, exit 0; diff protegido antes/depois tem SHA-256 idêntico (`review-3-wip-preservation.json`). Nenhuma alteração `.cursor/hooks.json` ou `.trail/hook.cjs` pertence à entrega; refutações das rodadas 1/2 permanecem válidas.

Achados em tratamento: o script dedicado `scripts/build-macos-release.sh` também precisa preparar o helper Local antes do Computer use; a recuperação do modelo sem marcador deve verificar hashes em worker, sem bloquear a inicialização da janela. Testes de regressão antes das correções em `logs/release-helper-order-red.log` e `logs/startup-hashing-red.log`. O build de release real permanece proibido e não foi executado.

Fila de builds: um daemon sccache criado pelo teste deste run reteve o lock depois de terminar. O PID exato foi encerrado (`logs/release-owned-cache-lock.log`); comandos Rust seguintes usam `RUSTC_WRAPPER=` somente no ambiente do comando, sem alterar configuração global.

Rodada 3, correções concluídas: `logs/review-3-fixes-green.log` exit 0, 10 testes do plano de release e 6 testes do instalador. `build-macos-release.sh` prepara Local antes do Computer use; nenhum build de release foi executado. `TranscriptionModel::new` não faz I/O; setup agenda `refresh_from_disk` com `spawn_blocking`, publica o evento ao concluir e usa a mesma exclusão mútua de instalação/remoção. O teste confere ausência de marcador durante registro, recuperação/publicação posterior e que remoção pendente impede reviver Ready. D5 permanece intacta. A espera de quota de 1 minuto foi cumprida durante as correções; rodada 4 é a próxima tentativa.

### Revisão pareada, rodada 4

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/4/summary.md`, exit **4**: OpenQodex concluiu; CodeRabbit sem quota, espera indicada de 7 minutos e 33 segundos, até 2026-10-29 20:29:51 UTC (`review-4-quota-wait.json`). Stash/pop exit 0 e hashes idênticos novamente (`review-4-wip-preservation.json`). Nenhum crédito extra, agente alternativo, mudança de conta/remoto ou push.

Tratamento dos quatro achados:

- `prepare-local-transcriber.mjs:100`: confirmado, nomes de certificado não servem como requisito SHA-1 do matcher. Resolver compartilhado `resolveDevSigningIdentity` agora converte a seleção explícita antes de comparar a assinatura; nenhuma alteração no Keychain.
- `local_transcriber.rs:216`: confirmado, disable podia perder a corrida com uma chamada bem-sucedida. Teste determinístico devolve um processo ao slot depois de disable sob o mutex, reproduzindo retenção. A limpeza agora adquire o mutex de forma assíncrona e encerra/recolhe o processo; uma nova escolha explícita de Local não é desativada por limpeza antiga. Resultado tardio também revalida geração/seleção.
- `transcription_model.rs:239`: confirmado, hash de retomada síncrono atrasava Cancelar. Verificação de arquivos reutilizados e prefixos agora usa leitura assíncrona em blocos de 64 KiB, com cancelamento entre leituras. O hash síncrono restante é exclusivo da recuperação em worker da rodada 3.
- `transcription_model.rs:339`: refutado novamente por D5, `design.md:58`, que exige confiar no marcador e nos tamanhos após a instalação verificada. Re-hashear toda abertura contrariaria a decisão aceita. Limite de corrupção externa de mesmo tamanho permanece explícito nas refutações anteriores. A ausência do marcador e tamanhos divergentes continuam causando verificação.

As duas regressões tiveram falhas reais antes das correções (`logs/review-4-races-red.log`, exit 101, 2 falhas). Verificação das correções em `logs/review-4-fixes-green.log`.

Rodada 4, correções verdes: `logs/review-4-fixes-green.log` exit 0, 2 regressões Rust e 6 testes do resolver de assinatura. `make -k verify` da revisão atual em `logs/resumed-full-verify.log`; somente os dois erros de formatação preexistentes e autorizados podem ser excepcionados.

`logs/resumed-full-verify.log`: frontend 204 suítes/1.914 testes, typecheck, fmt/Clippy all-targets e quatro crates companion passaram. O alvo check encontrou somente `.mcp.json`/`opencode.json`, como autorizado. Rust teve uma falha no teste existente `a_short_groq_retry_after_is_waited_then_retried` (3 chamadas em vez de 2); a repetição isolada passou sem alteração (`logs/groq-retry-isolated.log`, exit 0). A suíte Rust completa é repetida em `logs/resumed-all-rust-tests.log` com `RUST_TEST_THREADS=2` para reduzir concorrência. Não foi alterado o retry Groq nem enfraquecida sua asserção.

6.2 concluída com as exceções expressamente autorizadas pelo dono: `logs/resumed-all-rust-tests.log` exit 0, 2.180 testes passaram/21 ignorados em 28 suítes com dois workers, incluindo o retry Groq. Somado aos demais alvos verdes de `logs/resumed-full-verify.log`, todos os checks estão cobertos; não se declara exit 0 literal de `make verify`, pois `.mcp.json` e `opencode.json` permanecem intocados e fora de formato. A falha de retry anterior é registrada como intermitente sob concorrência; nenhum código ou asserção Groq foi alterado para fazê-la passar.


### Revisão pareada, rodada 5

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/5/summary.md`, exit **4**. Ambos os CLIs concluíram; CodeRabbit marcou o WAV de teste como binário excluído, mas o comparador do harness exige todos os paths em `reviewedFiles`, causando `scope_mismatch`. Correção fora do checkout depende da autorização solicitada ao dono; gate não liberado. Stash/pop novamente preservou o hash exato (`review-5-wip-preservation.json`).

- `Package.resolved` ausente: refutado. `logs/review-5-scope-and-lockfile-evidence.log` exit 0 mostra o mesmo blob `f0ab8609d98ca9ff09eaf99093d653557dc0aa3a` no commit inicial `59223a7` e no snapshot revisado `7b9087e`. O próprio revisor lista esse arquivo em `reviewedFiles`.
- Ditado relê seleção depois de await; nota Groq admite Local com fingerprint antigo; progresso emite por chunk; startup assíncrono perde aquecimento: confirmados, correções em andamento com regressões em `logs/review-5-regressions-red.log`.


Rodada 5, regressões reproduzidas: `logs/review-5-regressions-red.log` exit 101, quatro falhas reais. Correções em `logs/review-5-fixes-green.log` exit 0: seis regressões focadas e nove testes do instalador passaram. Ditado captura provider/modelo no token ao iniciar a tomada, resolve credenciais somente ao transcrever e mantém o backend através dos awaits; Local nunca consulta credenciais Groq. Notas recusam também a troca Groq→Local no meio do run. Progresso de download emite no primeiro chunk, no máximo a cada 100 ms e no fim de cada arquivo; cancelamento preserva a retomada no watermark observado. Ready após verificação no worker chama novamente o aquecimento condicionado à seleção atual. `logs/review-5-openspec.log` exit 0 confirma o contrato D6 revisado válido.


A primeira suíte completa pós-rodada 5 (`logs/review-5-rust-full.log`, exit 101) detectou um caso adicional: preview lia modelo e backend separadamente, e a proteção simétrica rejeitou a combinação do fixture Local com o modelo global Groq. Corrigida a causa também no caminho real: preview agora captura modelo/backend juntos, enquanto runs persistidos continuam vinculados ao modelo admitido. A suíte completa após esse ajuste está em `logs/review-5-rust-full-fixed.log`.


Após corrigir a seleção atômica do preview, a suíte desktop completa passou: **2.184 testes, 21 ignorados, 28 suítes**, incluindo todos os testes Groq (`logs/review-5-rust-full-fixed.log`). Clippy/fmt do mesmo comando continuam em andamento. Nenhum comportamento de retry Groq ou asserção existente foi alterado. O próximo commit é intermediário; não libera a entrega antes do gate 0 e da prova nativa.


Clippy da mesma rodada identificou `Option::is_none_or` acima do MSRV 1.80 declarado. Substituído por `map_or(true, ...)`, com a mesma condição de progresso, sem elevar o MSRV nem suprimir o lint. Portanto `logs/review-5-rust-full-fixed.log` tem exit 101 apesar dos 2.184 testes verdes; verificação de lint após a correção em `logs/review-5-msrv-check.log`.

`logs/review-5-msrv-check.log` exit 0: fmt e Clippy all-targets com `-D warnings` passaram após usar a API compatível com MSRV.


### Prova nativa, run-native-1

`logs/native-current-debug-build.log` e `logs/native-current-bundle-snapshot.log`, exit 0: build **debug**, arm64, helper reutilizado, bundle copiado para `.tmp/verify/transcricao-local-parakeet/run-native-1/Clovy-Parakeet-Proof.app` e assinatura ad hoc verificada. Identidade compilada `dev.guilhermexp.clovy.parakeetproof`; perfil sintético novo `~/Library/Application Support/dev.guilhermexp.clovy.parakeetproof-dev`. A cópia anterior de evidência foi preservada. Metadados de nome da cópia foram ajustados para “Clovy Parakeet Proof” e reassinados, sem mudar a identidade compilada (`logs/native-distinct-display-name.log`, exit 0).

App aberto pelo caminho exato. `logs/native-isolation-confirmed.log` e `logs/native-waiting-readiness-evidence.log`, exit 0, confirmam bancos do app no perfil de prova; WebKit usa o namespace da mesma identidade. O primeiro validador (`logs/native-startup-isolation.log`, exit 1) era restrito demais e rejeitou o banco interno do WebKit, também isolado; não houve abertura de banco de produção. Login item do dono manteve o mesmo SHA-256 do baseline.

A sessão do macOS está **bloqueada**: `logs/native-console-readiness-fixed.log` exit 0, `CGSSessionScreenIsLocked=true`. Computer Use devolve `cgWindowNotFound`; nenhum prompt de permissão foi observado ou aceito. Solicitado ao dono desbloquear a sessão; não houve tentativa de desbloqueio ou alteração de TCC/Keychain. Nenhum botão Install foi acionado, nenhum peso foi baixado, nenhum ditado/nota gravado. `run-native-1/dictation-events-startup-only.log` contém somente startup, sem tempos de transcrição.

Correção do comparador do harness preparada somente como artefato revisável `harness-binary-scope.patch`, **não aplicada** fora do checkout. `logs/harness-scope-regression-red.log` exit 1 reproduz o defeito; `logs/harness-candidate-regression-green.log` exit 0, 10 testes. O candidato cruza exclusões explícitas do CodeRabbit com binários detectados pelo Git; continua rejeitando texto omitido, base incorreta, arquivo extra, exclusão não declarada ou inválida. O resultado antigo permanece exit 4 e nenhum gate foi dispensado. Aplicação depende da autorização pedida ao dono, pois está fora do escopo original.

Enquanto aguarda o desbloqueio, somente o PID 20635 desta prova foi encerrado (`logs/native-stop-owned-while-locked.log`, exit 0); bundle, perfil sintético e evidências ficam preservados para a retomada. O app do dono não foi controlado.


### Retomada autorizada: rodada 6 e desbloqueio da sessão

O dono determinou: consultar `CGSSessionScreenIsLocked` a cada 60 segundos, sem polling mais frequente, e abrir o bundle exato assim que a sessão estiver desbloqueada. Monitor em `run-native-1/console-polls.jsonl`. Pedidos reais de permissão do macOS continuam exigindo parada e confirmação do dono. A reconstrução de histórico foi interrompida por instrução expressa; o manifesto existente foi aceito como suficiente.

Escopo exato da rodada 5: snapshot `ed99edce01948728deea1fe009362fc7612a955f..7b9087e8a923549e12aa6337a96dc3a698741d3d`, **57 paths**. CodeRabbit declarou **56 paths revisados**, a mesma base e revisão `committed`; declarou `src-tauri/tests/fixtures/parakeet-pt.wav` em `excludedBinaryFiles`. Esse WAV é o único `not_reviewed`; não há path inesperado. O CLI concluiu com exit 0, mas o gate exige igualdade literal com os 57 paths e classificou `scope_mismatch`, exit 4. Evidência exata em `review-5-scope-explanation.json` e `review-pair/5/summary.md`. Não é cobertura integral aceita nem liberação de commit. O harness externo não foi alterado; nova tentativa 6 expressamente autorizada pelo dono, sem `--include-untracked`, com allowlist e stash/pop dos mesmos paths protegidos. A última janela de quota informada, da rodada 4, terminou em 2026-10-29 20:29:51 UTC; rodada 5 não informou nova quota.

Fechamento dos achados OpenQodex 3–5 (todos ligados aos summaries absolutos já registrados acima):

- Rodada 3, ordem em `build-macos-release.sh`: corrigida em `8059537e`, regressão de 10 testes do plano de release verde, sem executar build release.
- Rodada 3, hash no startup: corrigido em `8059537e`, verificação em worker e remoção coordenada; testes do instalador verdes.
- Rodada 4, nome de certificado na assinatura: corrigido em `749740f2`, resolver comum converte para identidade comparável; seis testes verdes.
- Rodada 4, helper sobrevive à troca: corrigido em `749740f2`, limpeza assíncrona sob mutex, revalidação de geração; regressão verde.
- Rodada 4, hash de retomada bloqueia cancelamento: corrigido em `749740f2`, leitura assíncrona cancelável; regressão verde.
- Rodada 4, confiança em marcador/tamanho: refutado por D5; comentário e adendo ADR em `7b9087e8` documentam a política aceita e o limite de corrupção externa de mesmo tamanho.
- Rodada 5, perda de aquecimento após verificação assíncrona: corrigida em `9e3d77a3`; callback Ready volta a aplicar a seleção atual e o teste `parakeet_review_startup_warms_only_after_ready_is_published` passou.

Verificações pós-correções: 2.184 testes Rust verdes, fmt e Clippy all-targets verdes (API equivalente compatível com MSRV em `87eb03d4`). Gate 6 não libera commit final a menos que devolva exit 0; prova nativa continua pendente.


### Revisão pareada, rodada 6

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/6/summary.md`, exit **4**. Snapshot integral da entrega em `7f47e46c088f536bbcb17484b8832b58f4fbf93c`: 57 arquivos, sem WIP ou untracked. OpenQodex concluiu com um achado menor novo; CodeRabbit recusou por quota e indicou 7 minutos e 34 segundos. Prazo conservador registrado em `review-6-quota-wait.json`: 2026-10-29 22:15:27 UTC. Nenhum crédito extra, alteração de conta/remoto ou harness. Stash/pop exit 0, hash protegido idêntico em `review-6-wip-preservation.json`.

Achado `scripts/tauri-dev.mjs:61`: confirmado. Em branches com identidade automática, preparo inicial usava a identidade resolvida, mas o hook filho recebia somente `OS_CLOVY_DEV_APP_IDENTIFIER`, não `CLOVY_DEV_APP_IDENTIFIER`, invalidando o cache do helper. Regressão cobre branch isolada sem override e override explícito, com ferramentas e helpers falsos em sandbox; não acessa TCC real. Teste vermelho em `logs/review-6-launcher-red.log`.

As duas observações não contadas no veredito sobre `role=status` são semântica válida de live region já exercitada pela UI; não alteram o contrato nem exigem correção para este achado. Nenhum item fora das linhas da entrega foi incluído no escopo.

### Revisão pareada, rodada 7

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/7/summary.md`, exit **4**: CodeRabbit concluiu com exit 0, mas o comparador novamente rejeitou a exclusão declarada do único WAV binário (56 textos revisados dos 57 paths do snapshot). Não houve liberação do gate. Stash/pop concluído com exit 0 e WIP protegido idêntico em `review-7-wip-preservation.json`.

- OpenQodex, `dictation.rs:5947`: **refutado**. D4 e o cenário “Troca para Groq” exigem encerrar o helper ao trocar o motor. A tomada captura Local; uma troca explícita a faz falhar localmente, sem enviar seu áudio à Groq. Manter o helper até a tomada terminar, como sugerido, violaria esse contrato. Regressões de captura do backend em `logs/review-5-fixes-green.log` e de encerramento/revalidação do processo em `logs/helper-client-green.log` e evidências da rodada 4 sustentam a decisão.
- OpenQodex, `transcription_model.rs:217`: confirmado; cancelamento no último bloco pode deixar um `.part` completo. Em tratamento com teste de cancelamento exato em 7/17 bytes, retomada sem novo GET do arquivo completo e contraprova para arquivo completo corrompido.
- CodeRabbit, licença em `proposal.md:31`: licença real confirmada como Apache-2.0. Correção pontual no proposal aguarda autorização por estar além do `tasks.md` explicitamente incluído no escopo de edição da change; docs e atribuição do runtime já registram a licença correta.
- CodeRabbit, segmentos que atravessam turnos: risco confirmado no caso sintético de três turnos com fim de token 160 ms atrasado. O próprio design, seção Risks / Trade-offs, autoriza usar um turno por requisição Local quando esse teste falha. A regressão preserva a agregação de 400 ms de D4 e deve exercitar o caminho individual, sem rede nem alteração do empacotamento Groq.

Correções da rodada 7: `logs/review-7-regressions-red.log`, exit 101, duas falhas reais (GET repetido de arquivo completo; seleção do caminho de pacotes Local); `logs/review-7-regressions-green.log`, exit 0, nove regressões verdes. Um `.part` completo agora só é promovido após SHA-256 cancelável; corrupção exige novo download. Local usa um turno por chamada, conforme o risco previsto no design; teste sintético confirma a ambiguidade e três transcrições individuais com índices corretos e sentinela sem rede. Groq e o adaptador de 400 ms permanecem intactos. Docs e adendo ADR registram a alternativa.

`logs/review-7-full-rust.log`, exit 0: suíte Rust completa, fmt e Clippy all-targets com `-D warnings` passaram. O monitor de console não está mais ativo, pois detectou desbloqueio e terminou com exit 0.

Monitor nativo: identificada e corrigida interpretação incorreta de chave ausente. macOS pode omitir `CGSSessionScreenIsLocked` no console desbloqueado; a ausência anteriormente era lida como bloqueio. O monitor antigo foi encerrado somente após confirmar seu cwd; substituto preservou intervalo de 60 s e registrou `locked=false`, uma sessão no console e chave ausente às 22:51:35 UTC. `logs/native-wait-for-console-unlock-fixed.log`, exit 0. As leituras antigas continuam preservadas, mas não provam bloqueio quando não registraram o valor bruto.

Bundle exato reaberto, PID 24809, bancos confirmados no perfil sintético por `logs/native-unlocked-isolation.log`, exit 0. A UI recebeu interações locais enquanto o agente observava: download de 165 MB/632,2 MB, conclusão “Pronto · 632,2 MB”, escolha Local e Notas em Claude Code. O agente não presenciou nem executou o clique inicial; cancelamento/retomada não foram comprovados. Capturas `run-native-1/ready-observed-1.png` e `ready-before-permissions.png` mostram o estado pronto. O arquivo inicialmente chamado downloading.png já mostrava Pronto e foi renomeado para não atribuir evidência falsa. O banner exige Acessibilidade; as tentativas de “Conceder acesso” foram recusadas pelo controle de UI devido a mudanças concorrentes. Nenhum prompt do sistema confirmado e nenhum grant alterado. Prova ainda usa o bundle anterior às correções da rodada 7; requer atualização debug antes da aceitação final.


Rodada 6, correção concluída: `logs/review-6-launcher-red.log` exit 1, uma falha real em branch automaticamente isolada (identidade resolvida no primeiro preparo, vazia no segundo). `scripts/tauri-dev.mjs` agora transmite `CLOVY_DEV_APP_IDENTIFIER` resolvida ao filho Tauri. `logs/review-6-launcher-green.log` exit 0: 16 testes e typecheck verdes; um aviso no código gerado do fixture foi limpo. `logs/review-6-launcher-clean.log` exit 0: 16 testes e Biome sem avisos. Nenhuma chamada a TCC real, compilação nativa ou alteração no bundle de prova nessa correção de launcher. O prazo conservador de quota 22:15:27 UTC passou; próxima rodada é 7, para cobrir o novo diff, com o mesmo stash/pop e allowlist. Sem commit final até gate 0 e aceitação nativa.


### Revisão pareada, rodada 8: dois achados corrigidos

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/8/summary.md`, exit **6**. OpenQodex concluiu sem achados; CodeRabbit concluiu com dois achados menores. WIP restaurado exatamente, conforme `review-8-wip-preservation.json`.

- `LocalTranscriptionSettings.tsx:195`: confirmado. A falha de remoção atualizava o provider, mas deixava o snapshot do modelo em Pronto. Agora consulta novamente o estado; se a consulta também falhar, invalida o Pronto anterior como ausente, mantendo Local desabilitado e exibindo o erro original. Dois casos vermelhos antes da correção em `logs/review-9-removal-state-red.log` (exit 1, 2 falhas/8 verdes); `logs/review-9-removal-state-green-fixed.log` (exit 0, 10 verdes) cobre snapshot falho e consulta indisponível. A primeira rodada verde revelou apenas o seletor exato do botão sem seu sufixo de tamanho; corrigido para corresponder ao rótulo existente, sem alterar a UI.
- `specs/local-transcription/spec.md:101`: confirmado e corrigido com autorização explícita do dono. O cenário agora exige uma requisição Local por turno e mantém expressa a regra de intervalo de 500 ms dos pacotes Groq. O glossário Turn pack acompanha o contrato. A regressão Rust já verde `parakeet_review_imprecise_pack_tokens_use_individual_local_turns` cobre três turnos, texto no índice correto, zero rede e a asserção `PACK_GAP_MS == 500` (`logs/review-8-guard-regressions.log`).

Nenhum novo teste ou interação com o app nativo foi executado. A prova aceita e seus limites permanecem os registrados no início deste arquivo.

`logs/review-9-preflight.log`: typecheck passou; Biome apontou só a formatação do novo teste. `logs/review-9-preflight-fixed.log`: Biome corrigido e limpo, OpenSpec estrito válido; exit 1 veio exclusivamente de `trail check`, que comparou 500 arquivos e sinalizou 20 sobreposições lexicais em WIP protegido, texto herdado do glossário e `.gitignore`. Refutadas: estes fixes não alteram esses arquivos/configurações, chaves, MCP, grants ou isolamento; o staging contém somente os cinco paths da entrega. A justificativa acompanha o commit e uma nova nota pessoal, sem editar aprendizados anteriores.


### Revisão pareada, rodada 9: inicialização pelo recibo

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/9/summary.md`, exit **6**. OpenQodex concluiu sem achados; CodeRabbit trouxe um achado menor novo em `transcription_model.rs:474`. Os dois achados da rodada 8 não retornaram. Stash/pop restaurou o WIP protegido exatamente em `review-9-wip-preservation.json`.

Confirmado: o construtor iniciava em Ausente mesmo com recibo verificado e arquivos do tamanho correto, e dependia do worker para admitir Local. `logs/review-10-startup-receipt-red.log`, exit 101, reproduziu Ausente em vez de Pronto antes do worker. A correção consulta somente o pequeno recibo e metadados dos arquivos no construtor, reutilizando a mesma função de D5 usada pelo worker. Não faz hash dos pesos na thread da janela. Arquivos ausentes, tamanho divergente ou recibo ausente continuam sem admissão até a verificação assíncrona: D5 exige conferência antes de Pronto e D6 exige erro local enquanto não está pronto, sem fallback. O teste cobre também invalidação por remoção e recibo obsoleto.

Repowise não possui índice deste worktree e sugeriu consultar o checkout principal; essa alternativa foi recusada por estar expressamente proibida pelo dono. O grafo Trail e os callers locais foram consultados. Nenhum processo do app ou dado de prova foi tocado.

`logs/review-10-startup-receipt-green.log`, exit 0: todos os 12 testes do instalador passaram. `logs/review-10-trail-check.log`, exit 1: a base explícita reduziu a comparação a 64 paths, mas retornou 20 sobreposições lexicais em `.cursor` protegido, glossário, README, ADR e documentação. Refutadas: texto sobre provider, chaves, cache Swift, wrappers e prova não executa essas ações; não se copiou perfil/chave, abriu app, alterou tooling, reinstalou dependência, compartilhou cache por symlink ou contornou quota/permissão. A nova suíte Rust se justifica pela alteração de prontidão no startup. A nota pessoal anterior registra o limite; esta justificativa acompanha o commit.

`logs/review-10-rust-full.log`, exit 0: fmt, Clippy all-targets com `-D warnings` e suíte Rust completa verdes, 2188 testes passaram, 21 ignorados, 28 suítes. Todos os testes Groq permanecem verdes.


### Revisão pareada, rodada 10: precisão da redação

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/10/summary.md`, exit **6**. OpenQodex sem achados; CodeRabbit com dois ajustes menores de texto, ambos confirmados e corrigidos:

- Cenário “Ditado sem chave Groq”: a garantia agora se limita a nenhuma requisição **de transcrição** fora da máquina. Limpeza opcional do texto pode usar OpenRouter, conforme o contrato de ditado já existente; não se ampliou a promessa de privacidade.
- Descrição pt-BR da prévia: “ambas” foi substituído por “as duas transcrições”, explicitando a referência sem mudar o comportamento.

`logs/review-11-copy-validation.log`, exit 0: Biome no arquivo de mensagens e OpenSpec estrito passaram. São alterações de redação sem lógica nova; não se criou teste que apenas repete o literal. As regressões de comportamento UI (10 testes), instalador (12 testes) e suíte Rust completa (2.188 testes) continuam as evidências anteriores. WIP protegido restaurado exatamente em `review-10-wip-preservation.json`; nenhum controle do app.


### Revisão pareada, rodada 11: seleção na admissão do ditado

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/11/summary.md`, exit **6**. OpenQodex sem achados; CodeRabbit confirmou uma janela entre admissão e `listening_started`, anterior à proteção da rodada 5. A seleção era capturada somente no acknowledgement, podendo usar Groq após uma tomada admitida como Local.

`logs/review-12-admission-red.log`, exit 101: com o armazenamento de admissão preparado e o consumidor antigo, o teste recebeu provider `groq` em vez de `local`. A correção registra a seleção efetiva validada uma única vez por ID na admissão e a consome ao criar o token na confirmação ou na finalização sem confirmação inicial. O caso legado sem ID usa a admissão sem tag. Credenciais não são guardadas no token; o backend de transcrição é resolvido pela seleção capturada. Admissão Groq conserva sua validação existente; Local não consulta credenciais Groq.

A admissão não substitui nem cancela a tomada anterior antes do acknowledgement. Comandos não enviados e eventos terminais limpam as seleções pendentes. A regressão cobre os três caminhos e faz a leitura de chave Groq falhar se consultada para Local. WIP restaurado exatamente em `review-11-wip-preservation.json`. Nenhuma interação com o app nativo.

`logs/review-12-admission-green.log`, exit 0: 202 testes de ditado passaram, incluindo a regressão de admissão. Cobertura adicional protege a tomada anterior até acknowledgement e a limpeza seletiva de envio recusado. `logs/review-12-openspec.log`, exit 0: change válida. `logs/review-12-trail-check.log`, exit 1: mesmas 20 sobreposições lexicais já refutadas; não houve ação sobre config, chaves, MCP, caches ou app.

`logs/review-12-rust-full.log`, exit 0: 2190 testes passaram, 21 ignorados em 28 suítes; fmt e Clippy all-targets com `-D warnings` verdes. Inclui 203 testes de ditado e todos os testes Groq existentes.


### Revisão pareada, rodada 12: quota

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/12/summary.md`, exit **4**. OpenQodex concluiu sem achados no snapshot `4f481e870b0fe780b1cca50588868c35030089c1`; CodeRabbit recusou por quota e não revisou. Informou espera de 20 minutos. Deadline conservador calculado a partir da observação do erro: **2026-10-30 01:47:54 UTC** (22:47:54 local), em `review-12-quota-wait.json`. A próxima rodada só será iniciada depois desse instante; sem `--use-credits`, mudança de conta, seat ou API key.

Stash/pop exit 0 e hash protegido idêntico em `review-12-wip-preservation.json`. A rodada incompleta não libera commit final nem é tratada como revisão integral aprovada.

`logs/closure-frontend-full.log`, exit 0: suíte frontend completa da árvore corrigida, 204 arquivos e **1.918 testes verdes**. Mensagens `Chunk unavailable` são fixtures dos testes de lazy loading; nenhuma falha. Rust final permanece **2.190 testes verdes**, 21 ignorados, fmt e Clippy verdes (`logs/review-12-rust-full.log`). Não houve repetição da prova nativa.


### Revisão pareada, rodada 13: orientação para instalar

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/13/summary.md`, exit **6**. Iniciada às 2026-10-30 01:48:12 UTC, depois do prazo conservador 01:47:54; a quota não retornou. OpenQodex sem achados; CodeRabbit apontou que o erro de modelo ausente reutilizava o texto curto do cartão de Configurações e não dizia onde instalar no HUD/outros contextos.

Confirmado: `logs/review-14-install-guidance-red.log`, exit 1, duas falhas reais en/pt-BR. O código agora mapeia para `lib.errors.localModelNotInstalled`, com orientação explícita Settings > Models / Configurações > Modelos; a frase curta da opção desabilitada em Settings permanece intacta. `logs/review-14-install-guidance-green.log`: 23 testes verdes e typecheck verde; exit 1 somente por formatação do teste, corrigida na preflight. WIP restaurado exatamente em `review-13-wip-preservation.json`; nenhuma nova prova nativa.

`logs/review-14-preflight.log`, exit 0: Biome corrigido/limpo e OpenSpec estrito válido. `logs/review-14-trail-check.log`, exit 1: mesmas sobreposições lexicais em tooling protegido e documentação; refutação já registrada permanece aplicável, também na mensagem do commit.


### Revisão pareada, rodada 14: espera curta de quota

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/14/summary.md`, exit **4**. OpenQodex concluiu sem achados; CodeRabbit não revisou por quota de um minuto. Prazo conservador **2026-10-30 02:00:17 UTC**, registrado em `review-14-quota-wait.json`, já ultrapassado antes da rodada seguinte. Sem créditos extras, troca de conta/seat/API key. `review-14-wip-preservation.json` confirma stash/pop 0 e hash protegido idêntico. A revisão incompleta não libera commit final.


### Revisão pareada, rodada 15: quatro ajustes confirmados

`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/15/summary.md`, exit **6**. OpenQodex sem achados; quatro achados menores do CodeRabbit:

- Design D4/D8: alinhados à alternativa de um turno Local por requisição já aceita pelo dono, pelo cenário e pelo adendo ADR-0074. Não se escolheu arquitetura nova nem se alterou a regra de 500 ms dos pacotes Groq.
- Remoção na UI: o resultado do comando agora é separado do refresh de configuração. Sucesso conserva o estado Ausente e o aviso; AppSettings aplica a escolha Groq confirmada antes da leitura posterior. Se remoção e refresh falham, o erro original da remoção é preservado. `logs/review-16-ui-red.log`, exit 1, reproduziu duas falhas de remoção e uma de tradução; `logs/review-16-ui-green.log`, exit 0, 42 testes e typecheck verdes. Biome aplicou só formatação e manteve avisos preexistentes dos hooks fora das linhas alteradas.
- Teste de turnos individuais: passou a segurar `GLOBAL_SETTINGS_TEST_LOCK`, pois pós-processamento lê configuração global e testes vizinhos instalam servidores falsos sob esse lock. Correção de isolamento do teste, sem inventar uma reprodução determinística da corrida intermitente.
- Nota iniciada na Groq após troca para Local: o guard continua bloqueando a mudança de fingerprint/engine, mas devolve `transcription_engine_changed`, com mensagem de retry usando o motor atual, sem acusar falha do modelo Local. A agregação preserva o novo código e a frase armazenada é traduzida pelo banner en/pt-BR. `logs/review-16-route-red.log`, exit 101, falha real por receber `local_transcriber_failed`; `logs/review-16-persisted-route-red-fixed.log`, exit 101, reproduz o mesmo erro na nota persistida. A primeira tentativa do novo teste (`review-16-persisted-route-red.log`) teve erro de namespace SQLx, corrigido antes de reproduzir a falha funcional. A regressão de integração também exige áudio preservado e zero conexões à sentinela.

`review-15-wip-preservation.json` confirma restauração exata do WIP. Nenhuma nova prova nativa ou interação com o app.

Validação da rodada 16: `logs/review-16-rust-full.log`, exit 0, **2191 testes Rust verdes**, 21 ignorados, 28 suítes, fmt e Clippy all-targets com `-D warnings` verdes. `logs/review-16-frontend-full.log`, exit 0, **204 arquivos/1.923 testes frontend verdes**. `logs/review-16-preflight.log`, exit 0: Biome sem erros novos e OpenSpec estrito válido. `logs/review-16-trail-check.log`, exit 1: mesmas sobreposições lexicais já refutadas, sem mudança de tooling, credenciais, MCP, caches, permissões ou app.


### Rodada 16: três achados corrigidos para a rodada 17

- Summary: `/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/16/summary.md`, exit 6. OpenQodex sem achados; CodeRabbit concluiu com três. `review-16-wip-preservation.json` confirma restauração byte a byte do WIP protegido.
- `scripts/prepare-local-transcriber.mjs:115`: aceito. O cache agora compara o SHA-256 dos bytes pós-assinatura, registrado fora do bundle selado. Executável alterado ou digest ausente obriga build/cópia antes de assinar. `logs/review-17-helper-red-fixed.log`: falha real (1 build em vez de 2 após alteração). A primeira tentativa `review-17-helper-red.log` falhou no ambiente URL do teste, não na asserção; corrigida antes do vermelho real. Testes isolam Swift/codesign, não compilam nem controlam o app do dono.
- `processing.rs:2655`: aceito. Local→Groq agora retorna `transcription_engine_changed`, igual a Groq→Local, sem enviar áudio ao novo motor. `logs/review-17-route-red.log`: asserção real recebeu `local_transcriber_failed` em vez do código de troca. A mensagem orienta retry e não reinstalação; a proteção contra fallback permanece intacta.
- `LocalTranscriptionSettings.tsx:103`: aceito. O parent fornece o modelo ativo; a tela memoriza a última escolha Groq e a restaura ao voltar de Local. `logs/review-17-ui-red.log`: recebeu turbo em vez de large-v3. D6 mantém somente o modelo ativo persistido; ao reabrir a tela já em Local, sem escolha Groq anterior disponível, permanece o default turbo. Não se adicionou armazenamento ou migração.

Validação das três correções: `logs/review-17-frontend-green.log`, exit 0, 205 suítes/1.926 testes frontend, typecheck e Biome dos arquivos alterados; `logs/review-17-rust-green.log`, exit 0, 130 testes de processamento, fmt e Clippy all-targets com -D warnings; `logs/review-17-openspec.log`, exit 0. A suíte Rust completa anterior tem 2.191 testes verdes (`review-16-rust-full.log`); a única mudança Rust posterior é o código/mensagem de troca, coberto pelo vermelho→verde e pela suíte de processamento. `logs/review-17-trail-check.log`, exit 1: os mesmos 20 falsos positivos lexicais já refutados (configuração protegida e documentação herdada; nenhuma mudança de MCP, chave, permissão, perfil ou prova nativa). Não se altera o WIP do dono para silenciar esse checker.


### Rodada 17: cancelamento e quota

Summary: `/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/17/summary.md`, exit 4. CodeRabbit: quota, espera informada de um minuto respeitada enquanto OpenQodex concluía; nenhum crédito pago ou mudança de conta/assento/chave. `review-17-wip-preservation.json` confirma restauração exata do WIP.

OpenQodex `transcription_model.rs:167`: aceito. O select externo descartava a future inteira, mas escrita Tokio em thread bloqueante podia continuar depois da liberação do lock; retry/removal concorreria com essa escrita. Cancelamento agora é cooperativo nos waits de request e body; operações de disco são aguardadas e o lock só é liberado depois. `logs/review-18-cancel-red.log`, exit 101: teste determinístico pausa antes do flush, cancela e demonstra retorno prematuro; teste de rede travada já passava. Não há novo teste nativo nem acesso ao perfil do dono.

Verde: `logs/review-18-rust-green.log`, exit 0, suíte Rust completa com 2.193 testes verdes/21 ignorados, Clippy all-targets -D warnings e formato. Os dois testes novos passaram (flush pendente e headers/corpo travados), assim como cancel/resume por Range e `.part` completo. `logs/review-18-openspec.log`, exit 0. `logs/review-18-trail-check.log`, exit 1 pelos mesmos 20 falsos positivos lexicais refutados acima; nenhuma mudança de tooling, credenciais, permissão, MCP ou prova nativa. Frontend segue em 1.926 testes verdes, sem alteração desde a rodada 17.


### 6.3 Regra de convergência do dono e rodada 18

Em 2026-10-30 o dono substituiu o requisito absoluto de exit 0: corrigir somente majors e minors com efeito visível ou quebra de cenário; demais minors (estilo, higiene de locks de teste, wording e duplicação defensiva) recebem uma frase de evidência com o caminho do summary. Depois de mais uma rodada com apenas minors refutáveis, o commit final inclui a lista de refutações e encerra a entrega. Isso não transforma exit 6 em exit 0, nem aceita quota ou revisão incompleta.

Summary: `/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/18/summary.md`, exit 6; ambos concluídos; WIP restaurado exatamente (`review-18-wip-preservation.json`).

- Major CodeRabbit `scripts/prepare-local-transcriber.mjs:126`, aceito: assinatura com identidade de release agora usa `--options runtime --timestamp`, inclusive ao reutilizar output, sem recompilar desnecessariamente; debug/ad hoc mantém o comportamento anterior. `logs/review-19-signing-red.log` demonstrou falta da assinatura de release ao reutilizar; `logs/review-19-signing-green.log`, exit 0, 17 testes dos scripts/build/launcher. Nenhum certificado, Keychain, build release ou notarização real foi usado; assinatura do pacote de release permanece **não comprovada nativamente**, conforme limite do dono.
- Minor OpenQodex `AppSettings.tsx:1847`, aceito por efeito visível: o fallback otimista agora troca somente seleções que eram Local; remoção de override não substitui o modelo global Groq intacto. `logs/review-19-override-red-loaded.log` reproduziu a linha passando de large-v3 a turbo após falha de refresh. As três tentativas anteriores foram ajustes de fixture/seletores, não evidência de regressão. Sem refatoração adicional.

`logs/review-19-override-green.log`, exit 0: 30 testes de configurações/catálogo, typecheck e Biome dos arquivos alterados. Com os 17 testes de assinatura/build/launcher, são 47 testes direcionados verdes após a última suíte frontend completa de 1.926; Rust permanece no resultado completo de 2.193 testes da rodada 18, sem mudança posterior. `logs/review-19-trail-check.log`: os mesmos 20 falsos positivos lexicais, refutados pelo escopo e preservação do WIP acima.


### 6.3 Rodada 19: major de toolchain

Summary: `/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/19/summary.md`, exit 6, ambos concluídos; CodeRabbit sem achados; WIP restaurado exatamente (`review-19-wip-preservation.json`). OpenQodex `scripts/prepare-local-transcriber.mjs:75`, major aceito: os dois comandos Swift agora recebem o mesmo `DEVELOPER_DIR` do padrão existente em `tauri-dev.mjs`, preferindo Xcode instalado ao CLT antigo, sem mudar a seleção global da máquina. `logs/review-20-toolchain-red.log`: ambos os subprocessos não tinham esse ambiente; nenhuma build real ou mudança global foi executada. A nova rodada é necessária por ser major, conforme a regra do dono; nenhuma refatoração fora desse achado.

`logs/review-20-toolchain-green.log`, exit 0: 18 testes dos scripts/build/launcher, Biome e OpenSpec estrito. `logs/review-20-trail-check.log`, exit 1: mesmos 20 falsos positivos lexicais já refutados, sem alteração de tooling, seleção global de Xcode, chaves, permissões ou prova nativa.


### 6.3 Rodada 20: refutações sob a regra de convergência

- OpenQodex minor `transcription_model.rs:190`, refutado: D2 mantém carga preguiçosa na primeira transcrição e a aceitação prevê primeira tomada fria, enquanto D4/D8 exigem warm ao escolher Local e na abertura com modelo pronto, não um novo warm após reparo sem mudar a seleção; antecipar essa mesma carga é otimização adicional, sem perda de transcrição, fallback ou cenário quebrado (`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/20/summary.md`).
- OpenQodex minor `LocalTranscriptionSettings.tsx:74`, refutado como apresentação: o separador decimal usa a preferência do sistema, mas bytes recebidos/total, unidade, proporção e cancelamento continuam corretos nos testes, sem cenário que imponha o separador do idioma da interface (`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/20/summary.md`).

A rodada 20 tem exit 4, não aprovação: CodeRabbit não concluiu por quota de dois minutos; prazo respeitado sem créditos, conforme `review-20-quota-wait.json`. `review-20-wip-preservation.json` confirma restauração exata do WIP. Repete-se a revisão na rodada 21 sem alterar código nem repetir testes verdes.


### 6.3 Fechamento: rodada 21 e refutações finais

Ambos os revisores concluíram; exit real 6, sem major, sem segredo detectado e sem quota pendente; fechamento autorizado pela regra de convergência do dono, não por um exit 0 inexistente (`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/21/summary.md`).

- OpenQodex minor `scripts/prepare-local-transcriber.mjs:136`, refutado como endurecimento de configuração de build fora dos cenários: o modo ad hoc coberto usa identidade ausente (`review-20-toolchain-green.log`, 18 testes), enquanto `APPLE_SIGNING_IDENTITY=-` explícito em release ainda solicita timestamp e fica como limitação de configuração documentada, sem efeito no app debug aceito nem quebra de cenário OpenSpec (`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/21/summary.md`).
- CodeRabbit minor `tasks.md:94`, refutado: o dono exigiu caminhos absolutos desta worktree e os summaries exatos nas evidências, portanto preservá-los cumpre uma instrução explícita e não muda comportamento do produto (`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/21/summary.md`).
- CodeRabbit minor `local_transcriber.rs:211`, refutado como redação/código diagnóstico: D4 manda encerrar o helper ao deselecionar Local e converter sua falha em `local_transcriber_failed`, logo a chamada já em voo falha claramente com opção de tentar de novo, sem enviar áudio a Groq, enquanto a guarda das próximas chamadas relata `transcription_engine_changed` (`/Users/guilhermevarela/.zeron/worktrees/guilhermexp-os-clovy-da19cb436b9df051/feat-transcricao-local-parakeet/.tmp/verify/transcricao-local-parakeet/review-pair/21/summary.md`).

Mantêm-se as duas refutações da rodada 20 acima (warm adicional após reparo e separador decimal), sem refatoração ou novo teste nativo. `review-21-wip-preservation.json` confirma WIP restaurado byte a byte; hash `18a1176bd467b6ad1741ce303f1e51415a15778614291e165f005eb0557682c5`. Todos os checkboxes estão concluídos por implementação/evidência ou aceitação explícita do dono, preservando a lista de critérios não comprovados no início deste arquivo e no report.

Validação final de fechamento: `logs/final-delivery-validation.log`, exit 0, OpenSpec estrito e diff check; branch correta. `logs/final-delivery-trail-check.log`, exit 1 pelos mesmos 20 falsos positivos já refutados no texto e no corpo do commit final. Nenhum teste de código foi repetido depois do fechamento apenas documental.
