# Tasks

## 1. Dependência e skill

- [x] 1.1 Ler https://asciify.org/skill/SKILL.md e as referências `applying-asciify.md`, `components.md` e `kit.md`; instalar o skill do agente (`skills add ayangabryl/asciify-engine --skill asciify-engine`) pelo caminho que `spec/package-install-security.md` permite, ou registrar no design por que foi lido direto. Verificar: skill presente ou nota no design.
- [x] 1.2 `sfw pnpm add asciify-engine@4.29.0` (entrada em `allowBuilds` se pedir script de instalação) e conferir nos tipos instalados `background`, `drift`, `mark`, `clear`, `destroy` e os controles da seleção. Verificar: `pnpm install --frozen-lockfile` passa e o mapeamento dos controles fica registrado na decisão 3 do design.

## 2. Tema escuro padrão

- [x] 2.1 Sem chave salva, `src/lib/theme.ts`, `index.html`, `styleguide.html` e `onboarding-preview.html` resolvem escuro; teste unitário do padrão e da preferência salva; ajustar `technical-identity.test.mjs`. Verificar: testes passam e a primeira abertura em perfil novo mostra escuro sem flash claro.

## 3. Remover a lavagem colorida

- [x] 3.1 Apagar as regras de marca em `.agent-workspace::before`, `[data-home]::before` (claro e escuro), as camadas de marca do véu do composer e o gradiente do cartão do onboarding; remover tokens `--hero-wash*` sem uso e a entrada em `Color.tsx`; trocar o teste de `home-polish-css.test.ts` por checagem de ausência. Verificar: `pnpm test` do arquivo passa; capturas da Nova sessão e do Início em claro e escuro sem faixa colorida.

## 4. Preferência e configuração

- [x] 4.1 Criar `src/lib/app-background.ts` (chave `os-clovy:background`, `drift | none`, padrão `drift`, evento de mudança) com teste unitário de padrão, valor inválido e evento. Verificar: teste passa.
- [x] 4.2 Linha "Fundo" em Configurações → Aparência com "Drift" e "Nenhum", chaves i18n em en e pt-BR, blurb da aba atualizado. Verificar: `i18n-catalog.test.ts` passa e a troca aparece na Nova sessão sem recarregar.

## 5. Componente do fundo

- [x] 5.1 Componente de fundo montado em `AgentWorkspace.tsx` para hero e Início: import dinâmico do kit, um handle por montagem, `destroy` no cleanup e na troca para "Nenhum", guarda de desmontagem antes do import, `aria-hidden`, `pointer-events: none`, `--ascii-ink: #8f8d86`, cobertura de toda a superfície sem `clear` de coluna. Testes de componente com o kit simulado: opções recebidas, destroy único, import rejeitado sem erro. Verificar: testes passam.
- [x] 5.2 Registrar o fundo e a preferência nos docs de design que descrevem a lavagem do hero (`docs/design/*`) e no styleguide, se houver seção. Verificar: nenhuma menção restante a `--hero-wash` fora do histórico.

## 6. Verificação integrada

- [x] 6.1 Rodar `pnpm check`, `pnpm typecheck` e `NODE_OPTIONS=--no-experimental-webstorage pnpm test`. Verificar: saída sem falhas novas.
- [x] 6.2 No app (`pnpm tauri:dev` com identificador de dev próprio) e no `pnpm dev`: capturas da Nova sessão e do Início em claro e escuro, janela larga e estreita (390 px); movimento reduzido mostra quadro parado; WebGL desligado deixa fundo liso; ir e voltar da Nova sessão mantém um único canvas; trocar para "Nenhum" remove o canvas. Evidência em `.tmp/verify/fundo-ascii-tema-escuro/`.
- [x] 6.3 Rodar `/Users/guilhermevarela/orchestrator.v5/scripts/review-pair.sh --gate` antes do commit final e tratar os achados. Verificar: exit 0 ou achados refutados registrados aqui com o caminho do `summary.md`.

## Workflow follow-up

- Arquivar a change depois da revisão e atualizar o `AGENTS.md` do projeto.

## Evidência por tarefa

- 1.1/1.2: referências em `.tmp/verify/fundo-ascii-tema-escuro/reference/`; `install.log` e `frozen-install.log` (exit 0); tipos e mapeamento registrados na decisão 3 acima.
- 4.1: `tdd-theme-preference-red.log` reproduz a falta da preferência e o tema inicial errado; `tdd-theme-preference-green.log`: 35 testes passam.
- 5.1: `tdd-background-red.log` reproduz a falta do componente e a lavagem; `tdd-background-green.log`: 26 testes passam (incluindo import rejeitado, desmontagem durante import e troca live).
- 5.2: `docs/design/foundations.md` descreve preferência, tinta, limites e fallback; `Color.tsx` mostra o novo token. Não há referências a `--hero-wash` em estilos, componentes ou docs de design; testes de ausência e esta change preservam o histórico.

- 3.1/4.2: matriz de 16 capturas em `screenshots/`, `browser-results.json` sem erros, `tdd-settings-green.log` (22 testes). O arquivo `i18n-catalog.test.ts` não existe nesta base; a verificação equivalente usa `i18n-core.test.ts` + `i18n-settings-language.test.tsx`. Troca real em Aparência persistiu `none`; retorno sem reload e troca live removeram o canvas (`browser-lifecycle-progress.json`).
- Revisão 1: `.tmp/verify/fundo-ascii-tema-escuro/review-pair/1/summary.md`, exit 10 (OpenQodex incompleto e CodeRabbit bloqueado). Corrigidos perda da escolha se storage rejeita escrita e dependência de ordem no teste de import tardio; `tdd-review-green.log` e `tdd-pending-green.log` passam. Achados de `.cursor/hooks.json` e `.trail/hook.cjs` são alterações preexistentes fora do escopo e não entram no commit desta change. Alertas de segredo foram rastreados a fixtures sintéticas de testes de redação já presentes em `81f85c1e`, copiadas pelo cache Trail: evidência só com hashes em `secret-origin-check.json`/`secret-remaining-check.json`; allowlist pontual por regra/caminho/hash para a próxima rodada. Nenhum segredo real foi liberado.

- Ajuste aprovado pelo dono em 2026-10-28: remover o `clear` amplo para desenhar também no centro. Spec, decisão 4, docs e teste do componente atualizados; `tdd-full-surface-red.log` reproduz a coluna ainda sendo passada. A matriz visual e os gates serão refeitos sobre essa versão. O dono autorizou excluir somente `.trail/cache/` e `.tmp/verify/` do snapshot da revisão via configuração Git temporária, sem remover arquivos.

- Segundo ajuste do dono em 2026-10-28: intensidade reduzida de 1 para `DRIFT_OPACITY = 0.28`, única constante nos dois temas. `tdd-subtle-drift-red.log` reproduz a opacidade anterior. A rodada 2 (`review-pair/2/summary.md`) terminou com exit 4: OpenQodex incompleto e CodeRabbit `payload_too_large`; não autoriza commit. A rodada seguinte usará as duas exclusões temporárias autorizadas.

- 2.1/6.2: prova nativa no perfil `dev.guilhermexp.clovy.asciiproof20261028-dev`, caminhos conferidos por lsof; fresh escuro, Claro/Nenhum persistidos e reabertura confirmada por captura e SQLite (`native-preferences-{before,after}-restart.json`). Matriz final em `screenshots/{home,new-session}-{dark,light}-{drift,none}-{1180,390}.png`, galeria `screenshots/index.html`; `browser-subtle-drift.log`: 12 cenários passam, opacidade 0.28 e pixels no centro em todos os pares tema/largura. Movimento reduzido, WebGL ausente, duas navegações/um canvas, Nenhum live e clique/teclado passam no browser. Ocultação é sinal emulado; offscreen usa IntersectionObserver real. Matriz 390 e contagem de canvas são provas do browser; no nativo foram observados tema, persistência e renderização real.
- Revisão 2: os três achados de `.cursor/hooks.json`, `.claude/settings.json` e `.trail/hook.cjs` pertencem à migração de ferramentas já presente no checkout antes da tarefa. Refutados como regressões desta change, não como avaliação geral desses arquivos: não são editados nem farão parte do commit. Rodada 3 foi interrompida ao detectar que a configuração temporária precisava preservar o ignore global implícito; rodada 4 preserva todas as exclusões originais e remove do snapshot apenas os dois diretórios autorizados, comprovado por diferença de listas `git ls-files --others --exclude-standard`.
- Arquivamento da change e atualização de AGENTS.md permanecem follow-up, fora do escopo autorizado desta entrega.

- Revisão 4: `.tmp/verify/fundo-ascii-tema-escuro/review-pair/4/summary.md`, OpenQodex concluído; achados em `.cursor/hooks.json` e `.trail/hook.cjs` são os mesmos preexistentes refutados como regressões desta change acima. Gate exit 4 por quota do CodeRabbit. Rodada 5 aberta após a espera informada, com snapshot final e exclusões autorizadas; nenhum commit com gate incompleto.

- Terceiro ajuste do dono (2026-10-28): `DRIFT_OPACITY = 0.14` e desenho até as quatro bordas. A faixa lateral vinha dos 24 CSS px de padding de `.main-panel-body`, comprovados em `edge-before.json`; o inset negativo usa o mesmo token sem deslocar conteúdo. `tdd-edge-red.log` reproduz a divergência entre canvas e painel; `tdd-faint-drift-red.log` reproduz a opacidade antiga. A versão anterior fechou `test-final-node24.log` com 205 arquivos e 1903 testes passando; os gates serão repetidos para esta versão.


## 7. Imagem local (extensão aprovada em 2026-10-28)

- [x] 7.1 Ampliar normalizador para image mantendo drift/none e persistência. Verificar testes de preferência e troca/destruição do Drift.
- [x] 7.2 Comandos nativos de diálogo, validação, cópia/substituição/leitura/remoção no diretório isolado do app; registro em lib.rs. Verificar PNG/JPEG/WebP, rejeição tipo/tamanho/ilegível, preservação original/anterior, cópia ausente/corrompida; teste de registro.
- [x] 7.3 Aparência com três opções, nome, escolher/remover e erro inline en/pt-BR. Verificar testes dos estados, cancelamento e rejeição.
- [x] 7.4 Imagem cover centralizada, véu único do tema, cobertura de quatro bordas e fallback liso. Verificar componentes e capturas Nova sessão/Início largo/390 claro/escuro, caso ausente.
- [x] 7.5 Repetir gates frontend/Rust pertinentes e review-pair em diretório novo, sem provas/cache no snapshot, antes do commit.

- Rodadas 4/5 do dono: o inset horizontal corrige o padding do painel; a faixa inferior era o pseudo-elemento Home do composer (bottom -12px, mais largo que o cartão). `tdd-bottom-veil-{red,green}.log` registra a correção para bottom 0/largura do cartão. `edge-flow-final.log`/`edge-coverage.json` e crops `edge-*-{left,right,top,bottom,bottom-left,bottom-right}.png` confirmam host/canvas cobrindo o painel.
- Drift final: opacidade 0.14, flow contraste 1.4/escala 1.6. `browser-flow-tuned.log`: 12 cenários passam; `browser-results.json` mostra 57 a 145 blocos centrais alternando entre vazio/desenhado em quatro segundos nos oito pares superfície/tema/largura. Capturas `*-time2.png` mostram o segundo instante, sem máscara/clear.
- 7.1/7.3: `tdd-image-preference-red.log` e `tdd-image-row-red.log` reproduzem a ausência; `image-focused-final.log` passa 18 testes em 5 arquivos, incluindo preservação da imagem anterior quando a substituição é rejeitada. Registro dos três comandos passa.
- 7.4: `image-proof.log` passa 8 combinações com cover, host igual ao painel, véu 0.82, zero canvas e zero erros. Capturas `screenshots/{home,new-session}-{dark,light}-image-{1180,390}.png`; estados de configuração e ausência em `appearance-*-image*` e `home-*-image-missing-*`. São provas Chromium com bridge Tauri, não do diálogo nativo.

- Revisão 6: `.tmp/verify/fundo-ascii-tema-escuro/review-pair/6/summary.md`, exit 4 (CodeRabbit quota/assento). OpenQodex concluiu: dois achados de hooks preexistentes refutados como regressões desta change, conforme evidência das rodadas anteriores; o achado novo de recuperação com metadados corrompidos é válido e corrigido. `tdd-image-recovery-red.log`: 2 testes passam e o novo caso falha antes da correção; green será registrado após execução. Nova rodada 7 obrigatória antes de qualquer commit.

- 7.2: `tdd-image-native-green.log`: 2 testes passam; após o achado da revisão, `tdd-image-recovery-green.log`: 3 testes passam, incluindo manifesto corrompido e preservação de arquivos não relacionados. Idade Cargo passa em `cargo-release-age.log`.
- Bordas finais: captura espera o primeiro desenho e uma fase em que os quatro lados tenham pontos; `edge-coverage.json` mede entre 0 e 6 CSS px até o primeiro pixel de tinta nas oito combinações. O espaço é menor que uma célula, sem faixa fixa.

- Revisão 7: `.tmp/verify/fundo-ascii-tema-escuro/review-pair/7/summary.md`, exit 4 por scope_mismatch. OpenQodex não encontrou mais problema na implementação; CodeRabbit concluiu sem achado no código desta change e repetiu o caminho absoluto do hook preexistente. Os três nomes ausentes são fontes de renomeações da migração preexistente (.claude/helpers/graft-hooks.cjs → .cursor/hooks/trail-hooks.cjs; .claude/helpers/graft-statusline.cjs → .trail/hook.cjs; .cursor/rules/graft.mdc → .cursor/rules/trail.mdc). Evidência: `review-renames.txt`, obtida com índice temporário sem tocar no índice real. Rodada 8 desativa apenas diff.renames no ambiente Git temporário para contabilizar as duas pontas; nenhum arquivo adicional é excluído. Os achados de hooks são refutados como regressões desta change por pertencerem ao estado inicial, não ao conjunto de arquivos de implementação em changed-files.txt.

- Gates finais frontend: `test-final-default-image.log`, comando exato `NODE_OPTIONS=--no-experimental-webstorage pnpm test` com Node 24 oficial: 206 arquivos, 1910 testes passam, exit 0. `pnpm typecheck` passa. `pnpm check` mantém apenas os dois erros de formatação preexistentes em `.mcp.json`/`opencode.json`, fora do escopo; Biome nos arquivos desta change passa (`check-scoped-image.log`). Rust completo: 2192 testes passam, 22 ignorados (`test-rust-image.log`); clippy all-targets e fmt passam.
- Revisão 8: `.tmp/verify/fundo-ascii-tema-escuro/review-pair/8/summary.md`, exit 4 pelo mesmo scope_mismatch de três renomes, mesmo com diff.renames=false. Corrigidos os achados válidos: mutex adquirido somente depois de fechar o diálogo e parágrafo de imagem movido para a seção de fundos, em inglês. Alerta CSS refutado por `hero-toolbar-proof.json`: seletor encontra exatamente um toolbar no hero, data-hero=true, flex-wrap=wrap, largura 316 em viewport 390. Hooks preexistentes seguem fora do commit. Nenhum código desta change foi excluído da revisão.
- Prova nativa da imagem: `native-image-paths.log` confirma somente o perfil isolado; `native-image-copy.json` confirma hash da cópia/original e permissão 0600; `native-image-remove.json` confirma remoção apenas da cópia. Diálogo real rejeitou PNG corrompido e preservou a escolha anterior (`screenshots/native-image-rejected.png`). Capturas `native-{home,new-session}-{dark,light}-image.png` usam o renderer nativo real.

- Após os ajustes da revisão 8: `rust-image-final.log` (3 testes), `rust-fmt-final-image.log` e `clippy-final-image.log` passam. Rodada 9 aberta no script original, em `.tmp/verify/fundo-ascii-tema-escuro/review-pair/9/`; proposta de correção do classificador de renomes fica somente nos artefatos, aguardando autorização por estar fora deste checkout.

- Revisão 9: `.tmp/verify/fundo-ascii-tema-escuro/review-pair/9/summary.md`, exit 4 por scope_mismatch dos mesmos três renomes preexistentes. Ambos revisores concluíram. Os achados de `.cursor/hooks.json`, `.trail/hook.cjs` e `AGENTS.md` pertencem à migração preexistente de ferramentas e são refutados como regressões desta change, sem editar ou incluir esses arquivos no commit. O alerta condicional do toolbar é refutado novamente por `hero-toolbar-proof.json` (seletor corresponde, wrap ativo em 390 px). `review9-scope-proof.json` confirma todos os 33 arquivos desta change no snapshot, zero arquivos de prova/cache. Nenhum commit foi feito: exit 4 permanece impeditivo; patch do classificador de renomes preparado nos artefatos, não aplicado fora do escopo.
- Imagem após reinício: SQLite do perfil isolado mantém image/light (`native-image-preferences-restart.json`); recompilação e processo do app concluíram, com paths isolados confirmados. CUA retornou cgWindowNotFound na reabertura, portanto renderização após reiniciar não é afirmada; capturas nativas válidas são anteriores a esse reinício.

- Decisão do dono após revisão 9: não modificar o orchestrator. Migração Graft→Trail separada por stash com paths explícitos; `migration-stash-paths.txt` confirma zero arquivos da change, e AGENTS.md contém apenas o bloco Graft/Trail. O statusline previamente ignorado foi guardado em stash complementar, pois perder o .trail/.gitignore o tornou untracked. Ambos serão restaurados após o commit. Cache/provas seguem excluídos somente no ambiente da revisão.
- Correção das provas: `dark-image-pixels-red.log` detecta a captura nativa prematura com uma única cor; `dark-image-pixels-green.log`/`native-dark-image-pixels.json` passam nas duas telas (162 cores, variação máxima 32), sem mudar o véu. Imagem após reinício agora observada em `screenshots/native-image-after-restart.png`; o erro anterior era a seleção CUA pelo path, resolvida ao selecionar o identificador compilado confirmado do mesmo bundle. Paths isolados em `native-final-paths.log`.
- Matriz final recapturada: `browser-final-current.log` PASS 12, `image-final-current.log` PASS 8, pares `*-drift-*-time2.png` a quatro segundos. Nova sessão Drift nativa foi recapturada em dois instantes; a galeria foi reescrita com lista explícita somente de evidência atual. A matriz completa de superfícies/temas/larguras foi recapturada no Chromium. Capturas nativas adicionais foram pausadas após a janela ser reiniciada externamente, sem encerrar o novo processo. `pnpm check` com a migração guardada agora passa (603 warnings herdados, zero erros), em `check-change-only.log`.

- Revisão 10: `.tmp/verify/fundo-ascii-tema-escuro/review-pair/10/summary.md`: exatamente 33 arquivos da change, zero achados OpenQodex, CodeRabbit bloqueado por quota (11 minutos), exit 4. Nenhum arquivo da migração está no snapshot. Aguardar cooldown e abrir rodada nova com script original; não usar créditos.

- `review10-scope-proof.json` confirma igualdade exata entre os 33 paths do snapshot e `changed-files.txt`, sem paths extras ou faltantes. `screenshots/current-manifest.json` lista hashes/datas das 93 capturas atuais da galeria, excluindo as provas históricas.

- Revisão 11: `.tmp/verify/fundo-ascii-tema-escuro/review-pair/11/summary.md`, ambos revisores concluíram, exit 6 por três achados. Corrigido o limite de decodificação: `image::Limits` com 8192 px por dimensão e 128 MiB de alocação antes de decode; erro en/pt-BR orienta reduzir resolução. `image-limits-red.log`: 3 passam/1 falha; `image-limits-green.log`: 4 passam, rejeitando imagem comprimida pequena com largura ou altura excessiva e mantendo a anterior.
- Refutações da revisão 11: o seletor `.agent-workspace[data-hero="true"]` corresponde ao section de AgentWorkspace.tsx e o toolbar faz wrap em 390 (`hero-toolbar-proof.json`). A substituição da imagem não fica velha: a rota Aparência desmonta o workspace, e o retorno lê a nova cópia; `image-replacement-proof.log` confirma zero renderer em Settings, preferência ainda image e novo src/naturalWidth ao voltar, em dark/light e 1180/390. Não é necessário um segundo sinal de imagem para uma superfície que não permanece montada. Nova rodada 12 antes do commit.

- Revisão 12: `.tmp/verify/fundo-ascii-tema-escuro/review-pair/12/summary.md`, OpenQodex completo sem achados; CodeRabbit solicitou 19 segundos de cooldown, exit 4. Intervalo decorrido; rodada 13 com o mesmo código final e refutações registradas. `check-final-limits.log`, `typecheck-final-limits.log`, `fmt-final-limits.log` e `clippy-final-limits.log` passam.

- Gates após limite de decodificação: `test-final-limits.log`, comando exato `NODE_OPTIONS=--no-experimental-webstorage pnpm test` com Node 24, 206 arquivos e 1910 testes passam em 487.66s. Check/typecheck/fmt/clippy também passam.
- Refutação CodeRabbit rodada 13: base64, tempfile e uuid são dependências de runtime em `src-tauri/Cargo.toml:54` (linhas 56, 94 e 100), antes de `[dev-dependencies]`; uuid já habilita `v4`. `runtime-dependencies-proof.json` lê a tabela TOML real. O build nativo não-testes foi concluído em `native-final-visual.log`, e `clippy-final-limits.log` verifica todos os targets, incluindo código não-testes. O alerta condicional de dependências ausentes não se aplica.

- Revisão final 13: `.tmp/verify/fundo-ascii-tema-escuro/review-pair/13/summary.md`. OpenQodex completo sem achados; CodeRabbit completo com somente o alerta condicional de dependências runtime refutado acima. Exit 6, não 4/10; commit autorizado pela decisão do dono de aceitar achados refutados e registrados. `review13-scope-proof.json` confirma exatamente os 33 arquivos da change. Nenhum patch no orchestrator, nenhum crédito utilizado e nenhuma migração incluída.
