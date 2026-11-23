# Tasks

## 1. Plano

- [x] 1.1 Change OpenSpec (proposal com a causa raiz, design, delta de `llm-providers`, tasks); commit do plano (`d0ab8c0f`).

## 2. 429 com espera (D1)

- [x] 2.1 RED: servidor falso com cabeçalhos; 429 com `retry-after` dá `retryAfterMs` sem nova tentativa imediata; `retry-after` curto espera e repete; `x-ratelimit-reset-requests` sem `retry-after`.
- [x] 2.2 GREEN: `send_direct` lê os cabeçalhos.

## 3. Ritmo, espera e pausa (D2, D3)

- [x] 3.1 RED: `RequestPacer` (18 por 60 s, pausa comum); turno com 429 curto espera e sai com texto; 429 sem cabeçalho espera 60 s; 429 longo para o stream com `transcription_rate_limited`, deixa os outros pendentes e não roda o fallback; a rodada seguinte, no banco, só envia o que falta.
- [x] 3.2 GREEN: pacer, transcritor com ritmo, marcador de pausa no stream e no fallback, frase de pausa, retomada automática em `commands.rs`.

## 4. Tamanho dos pedaços (D4)

- [x] 4.1 RED: teto do turno abaixo de 25 MB; sessão com turnos prontos de 30 s mantém o layout antigo.
- [x] 4.2 GREEN: teto e escolha do layout.

## 5. Mensagens (D5)

- [x] 5.1 RED: `note-failure-banner.test.tsx` traduz a frase de limite e a de pausa em pt-BR.
- [x] 5.2 GREEN: chaves em `notes.ts` e reconhecimento no banner.

> Os RED de 2.1, 3.1, 4.1 e 5.1 saíram juntos no `48cef792`; o ponta a ponta reproduzia a falha relatada (`transcription_partially_failed`).

## 6. Revisão do orquestrador: 10 min, ASH e prévia (D2, D4, D6)

- [x] 6.1 Teto de 600 s, corte com 10 s de sobreposição e remoção das palavras repetidas.
- [x] 6.2 Orçamento de 7.200 s de áudio por hora no pacer; espera longa vira pausa.
- [x] 6.3 Pacotes de turnos por faixa com `verbose_json` e tempos de segmento, só com a chave Groq.
- [x] 6.4 Prévia ao vivo pelo mesmo pacer, sem espera e com no máximo 3.600 s por hora.

> A revisão chegou com a implementação em andamento, então os testes de 6.1 a 6.4 vieram depois do código, no mesmo commit GREEN (`6bf8541a`). Cada um foi conferido por mutação: com os pacotes desligados e com o orçamento de áudio desligado, os testes de pacote e de orçamento falham (4 falhas); com o código de volta, passam.

## 8. Revisão 1 (G1–G4)

Revisão do `7bb9ae04` bloqueada (BLOCK) com quatro achados.

- [x] 8.0 RED dos quatro (`7b2681fe`): o teste do G1 reproduz a nota que nunca termina, e o do G4 mostra dois vencedores na reivindicação.
- [x] 8.1 G2 GREEN (`d7560bf7`): pacer no relógio de parede injetável, protegido contra volta, olhando o relógio a cada 30 s.
- [x] 8.2 G3 GREEN (`c248fadf`): um pacer por conta Groq (impressão digital SHA-256) e um próprio da rota do Clovy API, escolhidos por requisição.
- [x] 8.3 G1 GREEN (`c4307809`): `note_transcription_chunks` (migração 51) guarda cada pedaço pronto de um job dividido.
- [x] 8.4 G4 GREEN (`3ce54680`): reivindicação atômica da pausa e revalidação ao pegar o lock.
- [x] 8.5 `make verify` numa worktree limpa, CI, report.

> 8.5: `make verify` no `a5f6a766`: a primeira rodada caiu só em `llm::tests::llm_cli_timeout_kills_the_cli_and_its_children` (timeout de 500 ms com a máquina carregada, fora desta mudança; isolado passa 3/3); a segunda deu EXIT=0 (vitest 1867/1867, Rust 2022 + integração). CI run 40615043799 só com os vermelhos conhecidos (vitest `mcp-server-dev-runner`, clippy `dictation.rs:211`/`notion.rs`); o job do Windows passou inteiro.

> Os RED dos quatro saíram num commit só porque os testes ficam no mesmo arquivo; os GREEN saíram um por achado, cada um fazendo passar só os seus testes. O teste do G1 usa o relógio do G2, então o G2 veio primeiro.

## 9. Revisão 2 (H1–H3)

A revisão 2 (`e1c11cef`) confirmou G1–G4 e deixou três P2.

- [x] 9.0 RED dos três (`9f480486`): o modelo não mudava a identidade do job nem a chave do pedaço, o retry manual não via o run que terminou antes dele, e cada requisição resolvia a rota duas vezes.
- [x] 9.1 H3 GREEN (`84267472`): uma rota por requisição (chave, modelo do run, pacer dessa chave) para o cache, o pacer e o transporte; a prévia também.
- [x] 9.2 H1 GREEN (`7eac3914`): modelo na impressão digital da configuração (o padrão não muda a identidade antiga) e modelo e idioma na chave do pedaço.
- [x] 9.3 H2 GREEN (`fc57dbb9`): a fila conta as rodadas concluídas por nota, e todo retry revalida sob o lock.
- [x] 9.4 `make verify` numa worktree limpa, CI, report.

> 9.4: `make verify` no `a1ed1105`: EXIT=0 na primeira rodada (vitest 1867/1867, Rust 2026 + integração). CI run 40625442214 só com os vermelhos conhecidos (vitest `mcp-server-dev-runner`, clippy `dictation.rs:211`/`notion.rs`, clipboard `activity::windows`).

> H3 veio antes de H1 porque a chave do pedaço usa o modelo da rota.

## 10. Revisão 3 (I1–I2)

A revisão 3 (`e19b446e`) aprovou H1 e a rota no caminho normal e deixou dois P2.

- [x] 10.0 RED dos dois (`9d08e9d8`): a chave salva ilegível caía na rota do Clovy API, e o retry manual marcava `transcribing` por cima de uma nota `ready`.
- [x] 10.1 I1 GREEN (`7dd6d5fb`): a resolução de rota propaga `provider_api_key_unavailable`; só `Ok(None)` vai ao Clovy API; a prévia não envia nada; frase traduzida no banner.
- [x] 10.2 I2 GREEN (`3b8f160f`): compare-and-set de `failed` para `transcribing`; um retry que marcou a nota sempre roda.
- [x] 10.3 `make verify` numa worktree limpa, CI, report.

> 10.3: `make verify` no `e70caa8f`: EXIT=0 na primeira rodada (vitest 1868/1868, Rust 2031 + integração). CI run 40628982214 só com os vermelhos conhecidos (vitest `mcp-server-dev-runner`, clippy `dictation.rs:211`/`notion.rs`); o job do Windows passou inteiro.

## 7. Documentação e verificação

- [x] 7.1 `docs/llm-providers.md`, adendo datado da ADR-0070, `docs/audio-pipeline.md`, termo `Turn pack` no `CONTEXT.md`.
- [x] 7.2 `make verify` numa worktree limpa, CI `desktop.yml`, evidência em `.tmp/verify/transcricao-limite-groq/report.md`.

> 7.2: `make verify` no `e0fdeb0c`: a primeira rodada caiu por timeout de 5 s em `cargo-clovy-runner.test.mjs` com a máquina carregada (passa isolado, 2/2); a segunda deu EXIT=0 (vitest 1867/1867, Rust 2014 + integração). CI run 40606541165 só com os vermelhos conhecidos (vitest `mcp-server-dev-runner`, clippy `dictation.rs:211`/`notion.rs`, clipboard `activity::windows`); por eles os testes Rust do macOS não rodaram no CI. Sem chamada real à Groq e sem prova nativa no app.
