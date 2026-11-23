# Tasks

## 1. Migração

- [x] 1.1 RED: testes do `identity_migration` (decisão com pasta nova ausente, vazia e preenchida; cópia completa com origem intacta; falha injetada deixa pasta nova vazia; credenciais copiadas incluindo a chave SQLCipher; credencial negada falha tudo); verificar que falham. Evidência: `ad5e2e34`, 12/12 falham (`.tmp/verify/identidade-clovy/1.1-red.txt`), testes sha256 `af57db47…`.
- [x] 1.2 GREEN: D3 e D2, chamados no início do `setup` antes de qualquer banco; testes de 1.1 passam. Evidência: `d3c8870c`, 12/12 passam com o mesmo `tests.rs` (`1.2-green.txt`). Desvio com evidência: roda no início de `run()`, antes do `Builder`, porque o Tauri cria as janelas e o WebView antes do `setup` (`tauri-2.11.3/src/app.rs:2524-2531`).

## 2. Renomeação

- [x] 2.1 D1: identificador, nomes, executável, auxiliares, inicialização automática, editor e `authors`, com constante única no Rust; scripts de verificação e testes que fixavam os nomes antigos. Evidência: `4633f9f9` (+ `e034cd9e` e `25b77337`, correções achadas no Windows).
- [x] 2.2 D4 e D5: extensão com chave e id novos, host de mensagens nativas novo, só `clovy://`. Evidência: `7ffeabca`, id `cmfpkfaibipkocbjcpgamaednndpopcp`; `osjune` já tinha saído no `main`.
- [x] 2.3 D6: remover camadas de compatibilidade June, mantendo só as origens da migração. Evidência: `3fc4ce0f`. Persona, skill e linha de configuração do Obsidian da era June são lidas uma vez e nunca escritas.

## 3. Documentação

- [x] 3.1 Nova ADR substituindo a 0055 (D7); `AGENTS.md`, `CONTEXT.md` e docs com os nomes e pastas novos. Evidência: `32c9a085` (ADR-0068).

## 4. Verificação

- [x] 4.1 `make verify` verde no Mac; guarda de domínios estendida para `co\.opensoftware|os-june|June\.app`. Evidência: `a49bc197`; `make verify` exit 0 em `make-verify.txt` e de novo após as correções em `make-verify-2.txt` (vitest 1786/1786).
- [x] 4.2 Prova no Mac com um perfil antigo sintético criado pela execução (notas, banco de atividade e chave SQLCipher de teste, sob origens e destinos de prova injetados por variável de ambiente só em build de dev), sem tocar o perfil nem o Keychain reais do dono: o app dev migra, abre com as mesmas notas e atividade, origem conferida intacta por hash; capturas e números em `.tmp/verify/identidade-clovy/`. O perfil real do dono só migra quando ele abrir o app depois do merge, com o orquestrador acompanhando. Evidência: `.tmp/verify/identidade-clovy/mac/report.md`: migrou 3 arquivos e 2 credenciais; notas e atividade lidas pelo app via MCP e na UI; origem igual por hash.
- [x] 4.3 Prova no Windows `cert` com perfil de teste antigo criado pela execução: migração e abertura; limpeza do `cert` ao final. Evidência: `.tmp/verify/identidade-clovy/windows/report.md`: migrou 3 arquivos e 2 credenciais; UI via CDP mostra notas e atividade; origem igual por hash; `cert` limpo (45,17 → 45,02 GB).

## 5. Correções da revisão independente (I1–I5)

- [x] 5.1 RED: `b17a1f57` (I2–I5) e `25d105d1` (I1), com as falhas em `.tmp/verify/identidade-clovy/review-red-*.txt`.
- [x] 5.2 GREEN: `5bfff0c8`, 24/24 testes da migração (`review-green.txt`). O que mudou:
  - I1: caminhos para dentro do perfil antigo reescritos na cópia em staging (bancos, localStorage do WebKit, arquivos de texto, symlinks).
  - I2: bloqueio enquanto o app antigo roda, com bancos copiados por snapshot `VACUUM INTO`, cifrado com a chave copiada.
  - I3: falha antes de publicar se a credencial de destino tiver valor diferente.
  - I4: lock por destino; só é limpo o staging cujo dono morreu.
  - I5: dados publicados por último, com journal para retomar.
  - `41fdf98f`: o alerta do caso bloqueado agora é um `NSAlert`.
- [x] 5.3 Docs: ADR-0068 e AGENTS.md (`9c4f137e`); passo a passo do dono em `.tmp/verify/identidade-clovy/owner-first-launch.md`.
- [x] 5.4 `make verify` exit 0 (`make-verify-3.txt`).
- [x] 5.5 Prova no Mac em `.tmp/verify/identidade-clovy/mac-review/report.md`:
  - Caso A (app antigo rodando): recusa com mensagem clara e nenhum perfil criado. O alerta não foi capturado nesta sessão.
  - Caso B: caminhos reescritos (gravação e workspace), notas e atividade lidas pelo app, origem igual por hash.
  - Windows não repetido; o código só-Windows novo usa apenas `std` e não foi compilado lá.

## 6. Segunda revisão (J1–J7)

- [x] 6.1 RED `4125a3af`: 6 falhas (`review2-red.txt`).
- [x] 6.2 GREEN `2d1a7d3d`, 29/29 (`review2-green.txt`):
  - J1/J7: lock de SO (flock/LockFileEx) com prazo e erro bloqueante;
  - J2: retomada que falha bloqueia e preserva o estado;
  - J3: fsync do staging, do journal e da pasta; journal ilegível bloqueia e fica;
  - J4/J5: JSON reescrito folha a folha, só onde um caminho começa pela raiz;
  - J6: falha do `tasklist` é erro.
  - `e82bfd44`: flush por handle gravável (achado no CI Windows).
- [x] 6.3 Docs: ADR-0068 (`b29e8251`) e `owner-first-launch.md` (estado de recuperação preservado).
- [x] 6.4 `make verify` exit 0 (`make-verify-6.txt`). Falhas de carga fora desta change em `flaky.txt`.
- [x] 6.5 Prova no Mac, caso B (`mac-review2/report.md`): `args_json` igual à raiz reescrito e workspace externo com a raiz como sufixo intacto. Houve um incidente com um diálogo do Keychain, registrado no report.
- [x] 6.6 CI Windows 39666093272 (`mac-review2/windows-ci.md`): compila; os 4 testes `identity_migration` selecionados passam; as falhas restantes estão em `activity::windows`, não tocado.

## 7. Terceira revisão (K1–K5)

- [x] 7.1 RED `bb9bbac2` (6 falhas, `review3-red.txt`).
- [x] 7.2 GREEN `baa8de4f` + `1666b710` (correção de fixture com conexão fechada de forma assíncrona), 33/33 em 10 rodadas (`review3-green.txt`):
  - K1: rollback do que foi publicado quando o flush falha;
  - K2: flush sem exigir escrita na origem; arquivo só-leitura preservado;
  - K3/K4: reescrita só na lista explícita de campos de caminho; conteúdo livre intacto;
  - K5: qualquer falha bloqueia a abertura.
- [x] 7.3 Docs: ADR-0068 (`112d6a1c`) e `owner-first-launch.md`.
- [x] 7.4 `make verify` exit 0 (`make-verify-7.txt`).
- [x] 7.5 Prova no Mac (`mac-review3/report.md`): falha forçada bloqueia no `NSAlert` sem criar perfil; nota e mensagem com o caminho antigo intactas; campos de caminho reescritos; objeto Git 0444 preservado; origem igual por hash.
- [x] 7.6 CI Windows 39671302382: compila; os 4 testes `identity_migration` do filtro passam; só a falha de clipboard preexistente.

## 8. Quarta revisão (L1–L5)

- [x] 8.1 RED `74ef4004` (4 falhas, `mac-review4/review4-red.txt`; o teste do vault fora do perfil já passava e fica como guarda).
- [x] 8.2 GREEN `f3b2e658`, 36/36 (`mac-review4/review4-green.txt`):
  - L1: o rollback é gravado no journal (fsync) antes de qualquer remoção; a abertura seguinte o termina ou bloqueia; publicação sem staging também vira rollback gravado;
  - L2: migração só no macOS (módulo e chamada sob `cfg(target_os = "macos")`); removidos a enumeração de credenciais do Windows, `LockFileEx`, `tasklist`, a pasta do WebView2, o caminho com barra invertida, o flush com handle gravável e a feature `Win32_Security_Credentials`;
  - L3: execuções pausadas aposentadas na cópia (cartões `expired`, execução e sessão `interrupted` com `identity_migrated`, `interrupted_state_json` zerado);
  - L4/L5: `hermes/config.yaml` (MCP `command`/`cmd`/`args`), `hermes/cron/jobs.json` (`script`/`workdir`) e `obsidian.json` (`vaultPath`) na lista de caminhos.
- [x] 8.3 Docs: adendo na ADR-0068, spec, design, proposal e `owner-first-launch.md` (Windows não migra; sessões pausadas não voltam).
- [x] 8.4 `make verify` exit 0 (`mac-review4/make-verify-8.txt`, worktree limpa em `97c85371`; vitest 1786/1786).
- [x] 8.5 Prova no Mac (`mac-review4/report.md`): rollback gravado com resto impossível de apagar bloqueia no `NSAlert`; depois termina e migra de novo; nada retomável; caminhos de Hermes e Obsidian reescritos; origem igual por hash; sem Keychain.
- [x] 8.6 CI 39700732378 (`mac-review4/windows-ci.md`): Windows compila e passa (184/0); só as falhas conhecidas (vitest codesign, clippy Rust 1.99).
