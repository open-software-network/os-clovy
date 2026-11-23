# Tasks

## 1. Plano

- [x] 1.1 Change OpenSpec (proposal, design, delta de `llm-providers`, tasks) e ADR-0070 estendendo a ADR-0065 §5 para notas; commit do plano.

## 2. Transcrição de nota direta na Groq (D1, D2, D5)

- [x] 2.1 RED: testes em `clovy_api_notes_tests.rs` para direto com chave (turno final e prévia), nova tentativa em 503 e nenhuma em 400, `no_speech`, 401/403 rejeitada, limite de 25 MB, fallback ao Clovy API sem chave e `groq_api_key_missing` com porta fechada; commit RED.
- [x] 2.2 GREEN: `transcribe_saved_audio_via` e `transcribe_saved_audio` lendo a chave Groq; testes de 2.1 passam.

## 3. Geração e limpeza diretas na OpenRouter (D3, D4, D5)

- [x] 3.1 RED: testes para geração direta com chave, Auto e sem chave pelo Clovy API, `openrouter_api_key_missing` com porta fechada e limpeza `note_transcript_cleanup` com o contrato do servidor; teste em `processing.rs` para a frase da geração sem chave; commit RED.
- [x] 3.2 GREEN: `generate_note_via`, `cleanup_text` por `dictate_cleanup_via` com o contrato da nota; testes de 3.1 passam.

## 4. Mensagem na nota (D5)

- [x] 4.1 RED: `note-failure-banner.test.tsx` traduz as frases novas em pt-BR; commit RED.
- [x] 4.2 GREEN: chaves `notes.failure.*` novas em en e pt-BR e `STORED_FAILURE_KEYS`.

> Os RED de 2.1, 3.1 e 4.1 saíram juntos no commit `a7f3b2a9` e os GREEN de 2.2, 3.2 e 4.2 no `a05051b5`. O caminho nas frases foi corrigido depois para Configurações > Modelos > Mais opções > Chaves de API de provedores, onde a linha de fato fica.

## 5. Documentação e verificação

- [x] 5.1 `docs/llm-providers.md`, `docs/configuration.md`, `AGENTS.md` e README/PRIVACY se citarem o fluxo.
- [x] 5.2 `make verify` numa worktree limpa, CI `desktop.yml` na branch, evidência em `.tmp/verify/notas-direto-provedores/report.md`.

> 5.2: `make verify` EXIT=0; CI run 40567062149 só com os vermelhos conhecidos (vitest `mcp-server-dev-runner`, clippy `dictation.rs:211`/`notion.rs`, OCR `activity::windows`); por eles o instalador Windows não foi construído e os testes Rust do macOS não rodaram no CI. Sem chamada real aos provedores.

## 6. Revisão N1: nova tentativa na limpeza de nota (D4)

- [x] 6.1 RED: 503 seguido de 200 na limpeza `note_transcript_cleanup` direta dá o texto limpo em duas requisições; 400 não é repetido; a limpeza do ditado continua com uma tentativa; commit RED (`616ee8fd`).
- [x] 6.2 GREEN: o caminho direto da limpeza de nota usa `send_direct`; o ditado fica com uma tentativa, por latência (D4).
- [x] 6.3 `make verify` numa worktree limpa no `c12e285f`: EXIT=0.
