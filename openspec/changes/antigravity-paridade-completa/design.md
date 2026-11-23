# Design

## Context

Ver `proposal.md` e os dois deltas. O usuário confirmou que o app administra tudo e os três runtimes são motores intercambiáveis do mesmo pool de ferramentas, não integrações com alcance diferente.

O gateway, catálogo, partições, `TurnRegistry`, `TurnGrant`, broker, persistência e os adaptadores já existem. A candidata d3f027b conecta Agy por uma guarda de configuração compartilhada ainda não aceita: a ancoragem de ancestrais F11 permanece aberta. Seleção, replay e diagnóstico receberam correções verificadas separadamente; isso não qualifica o novo transporte nem autoriza preservar a guarda obsoleta como segundo caminho.

Prova local: `agy models` respondeu; um turno real com Gemini 3.8 Flash Medium respondeu `AGY_QA_OK`, sem eventos de ferramenta e sem alterar settings. Isso não prova integração MCP nem contenção. Referências oficiais consultadas em 2026-11-03: [MCP](https://antigravity.google/docs/mcp?tab=cli), [settings](https://antigravity.google/docs/settings?tab=cli), [headless](https://antigravity.google/docs/cli/headless/) e [sandbox](https://antigravity.google/docs/sandbox/). Workspace MCP existe; `--settings` é transitório; `--sandbox` pode não impor contenção onde indisponível. Hooks possuem arquivo separado, não são desativados por uma chave ignorada de settings.

A prova posterior com CLI1.4.12 autenticou um perfil próprio, executou MCP e retomou a mesma conversa com servidor novo, recuperando o marcador anterior sem fornecê-lo no prompt. Um hook global sintético executou ligado e deixou de executar após sua remoção no mesmo perfil autenticado. O fixture externo iniciou seu MCP, mas não executou modelo/hook por falta de login próprio. Os recibos em `.tmp/verify/antigravity-paridade-completa/native-capabilities/` sustentam separação de configuração, não contenção geral do SO ou aceite integrado. O dono aprovou A1–A5 e a adoção permanente explicitamente depois dessa prova.

## Goals / Non-Goals

**Goals:** mesma autoridade e mesmo catálogo do Clovy para Claude, Codex e Antigravity na mesma superfície; funcionamento com apenas um motor elegível instalado; rotinas e geração pontual seguras; escolha de motor sem ampliar autoridade.

**Non-Goals:** novo agente nativo, loop de ferramentas proprietário, catálogo por runtime, proxy de modelo, cópia de credenciais, alteração de configuração global, paridade dos comandos internos proprietários de cada CLI ou motores novos de áudio/mídia. Ferramentas de mídia/conectores existentes permanecem no pool com os mesmos pré-requisitos host-owned.

## Decisions

### D1. Um gateway existente, adaptadores somente de transporte e eventos

Reutilizar o `McpLaunch` do turno e os mesmos handlers/catalogadores. O adaptador recebe um vínculo novo e entrega a conexão ao runtime. Diferenças de catálogo dependem de superfície, partição, política e pré-requisitos do app. Comparar catálogo normalizado, leitura autorizada, recusa de efeito protegido e cancelamento em Rotinas nos três motores. Validar efeitos externos nas Configurações MCP conforme `retirar-chat-do-clovy`. Não criar outro registro de ferramentas nem reencaminhar a chamada por Claude/Codex.

Alternativas descartadas: configuração global com `agy mcp add` interfere em sessões e credenciais do dono; implementar um loop próprio duplica a autoridade que já pertence ao Clovy. A única abordagem proposta é completar o adaptador existente.

### D2. Perfil Agy gerenciado e conexão de turno em diretório próprio

Manter um perfil nativo durável em `<app_data>/runtime-profiles/agy/home`, com dados de autenticação pertencentes ao SDK e nunca tratados como cache. HOME/XDG e os equivalentes Windows mudam somente no ambiente final do filho Agy; não modificar o ambiente do app nem o `LoginEnv` compartilhado. No Windows, o Go resolve home por USERPROFILE, configuração por APPDATA e cache por LOCALAPPDATA: só mudar HOME/XDG não isola esse runtime. Definir também esses caminhos e HOMEDRIVE/HOMEPATH coerentes para o perfil próprio, resolvendo antes o executável no PATH real. O relay conserva separadamente os valores host de identidade necessários à sua guarda, sem expor chaves ou tokens ao motor. Rotinas, geração pontual, teste de provedor e entrada de login usam o mesmo contexto.

O diretório de execução do CLI pertence ao Clovy, com escopo de sessão para execuções vinculadas e exclusivo por chamada pontual. Não iniciar Agy no workspace arbitrário recebido pela API, adicionar esse diretório pessoal como raiz de descoberta ou escrever nele `.agents`. O workspace autorizado do usuário continua no contexto host-owned das ferramentas; não vira a raiz de personalizações do motor. Criar diretórios/configurações privados antes de gravar autoridade, usando primitivas existentes e verificação de ACL quando aplicável; recusar symlinks ou estado cuja propriedade não possa ser demonstrada sem alterar dados alheios.

O SDK também procura personalizações em ancestrais até a raiz Git: o teste nativo carregou um hook do pai mesmo com HOME privado. Cada workspace de execução, inclusive login e pontual, deve ser uma raiz própria de descoberta. Inicializar nele um repositório vazio válido, com metadados mínimos privados e nenhum commit, sem depender de executar Git ou acrescentar biblioteca. A prova nativa qualificou `.git/HEAD`, `config`, `objects` e `refs` criados por filesystem; `core.hooksPath` aponta para diretório vazio próprio e `info/exclude` impede versionar o transporte. Remover roteamentos `GIT_*` herdados que apontariam para repositório/configuração externos. Isso não cria histórico paralelo: só delimita a descoberta do SDK; a história da conversa continua no Clovy.

Gravar no workspace próprio `.agents/mcp_config.json` com somente o servidor do gateway atual, nome único, argumentos absolutos existentes `--dir`/`--turn` e `timeoutSeconds:900`. O nome não concede autoridade: o app valida o token e revoga o grant antes de limpar o transporte. Conservar a identidade do relay e sua proteção debug contra dados de produção; HOME privado do runtime não pode enfraquecer essa guarda. O relay continua falando com o servidor e o banco do app, nunca com outro banco calculado pelo HOME do motor.

Cobrir preparo/spawn falho, sucesso, Stop e crash; turnos concorrentes não compartilham configuração ou vínculo. Remover somente artefatos temporários próprios, preservando autenticação e recibos. Eliminar a guarda de workspace compartilhado e seus callers, testes exclusivos e documentação obsoletos: não manter fallback, alias ou duas convenções. Qualificar nativamente que uma nova execução descobre o próprio servidor e não usa autoridade antiga.

### D3. Política do app, não a configuração permissiva do runtime

Adicionar a linha Agy à tabela existente. Usar `--settings` transitório para fixar explicitamente modo e regras; permitir apenas o MCP do gateway vinculado nas exceções MCP do turno. A autorização de escrita continua exclusivamente no broker. Não usar `--dangerously-skip-permissions`, `always-proceed`, `auto` ou configuração global para fazer testes passarem.

Rotinas usam a política não assistida existente e suas capacidades de leitura; a escrita que exige aprovação é recusada sem cartão pendente. Antes de admitir Agy em rotinas, uma prova nativa deve verificar precedência das regras sobre permissões globais, negação de escrita/comando/atuação não autorizados e ausência de passagem por extensões/hooks fora da política. Geração pontual recebe a mesma verificação de isolamento, schema e efeitos, sem presumir que a flag de sandbox desliga ferramentas. Uma limitação comprovada do CLI é bloqueio da entrega de paridade, não permissão para declarar um modo reduzido como completo; deve voltar ao usuário com evidência e alternativa concreta.

O dono exige funcionamento com extensões pessoais ligadas. O mecanismo aprovado é o perfil e o workspace gerenciados pelo app, sem importar hooks/plugins/MCPs pessoais. Não copiar credenciais, desativar personalizações externas ou substituir silenciosamente o motor. O perfil é separação de configuração; suas flags não são prova de sandbox geral.

Os três filtros temporários de SO não qualificaram transporte funcional: o amplo bloqueou startup e os estreitos não descobriram MCP. Não adotar `sandbox-exec`, interface depreciada, nem ampliar a guarda compartilhada para contornar isso. A prova de perfil não dispensa observar builtins, browser, delegação, processos auxiliares, Stop e negações reais antes de admitir modo não assistido.

O app oferece uma ação explícita para iniciar a autenticação nativa no perfil gerenciado, por terminal próprio com argv/env controlados, sem prompt de tarefa nem transferência de token pelo Clovy. Instalação e versão não significam autenticação; o teste real deve usar esse perfil e explicar falha de login sem fallback. Não reaproveitar como readiness autenticada uma medição feita no HOME pessoal. Preservar o consentimento nativo escolhido pela pessoa, sem autoaceitar coleta ou sobrescrever suas preferências.

### D4. Seleção e contexto são independentes de elegibilidade

Resolver elegibilidade separadamente para cada uso. Só admitir Agy em rotinas depois de qualificar a política não assistida. Respeitar o runtime escolhido e não trocar de motor em erro. Incluir Agy na descoberta e seleção dos usos elegíveis no backend e no frontend; manter as rotas independentes de transcrição e ditado.

Preservar as proteções de identidade dos registros existentes e os leitores de contexto já utilizados. Em qualquer execução dos consumidores restantes, mudanças no motor selecionado só se aplicam a novas execuções e não transferem vínculos, grants ou decisões pendentes.

A proteção de replay registrada na tarefa concluída 4.2 recusa IDs de outra origem no SDK privado. Preservar mensagens, resultados e referências antigos com seus limites de acesso, sem copiar o armazenamento do SDK ou reenviar ações.

### D5. Normalização e prova reais

Reaproveitar a correção Stop/migração56 em qualificação como base. O parser Agy deve preservar argumentos/outputs/erros reais, estado terminal e resposta final mesmo sem deltas intermediários, sem spinner permanente nem sucesso vazio. Usar fixtures sanitizadas do protocolo real e testar a mesma projeção/hidratação comum; não fabricar resultado para acomodar diferenças do CLI.

Provas de geração pontual cobrem schema efetivo, timeout, ausência de efeitos e não autoingestão no mesmo perfil privado. A leitura de sessões pessoais permanece no HOME real e com toggle independente: não migrar sua raiz, desativá-la ou usá-la como fixture. O toggle não autentica nem configura o provedor. Prova nativa no Mac usa bundle/perfil próprio e entradas sintéticas; Windows exige host real disponível.

## Risks / Trade-offs

- Perfil do motor e workspace do usuário têm funções distintas → contexto de lançamento único, MCP com vínculo exclusivo, relay do app preservado e nenhum arquivo de autoridade no projeto pessoal.
- CLI novo aceita uma chave desconhecida e a ignora → observar configuração/negação efetivas em canários nativos, não aceitar argv como prova.
- Rotina herda permissões, hooks ou MCPs globais → qualificar isolamento e impedir admissão quando não houver garantia; não alterar os arquivos do dono.
- Mudança de provedor entre execuções → histórico preservado e vínculos independentes, com regressão de efeito único.
- Cota/auth/Windows indisponível → registrar o runtime e a prova que faltam, sem atribuir ao resultado sintético aprovação nativa integral.
- Primeiro login no perfil Clovy pode ser necessário apesar do login pessoal → autenticação nativa iniciada pelo app, com estado/erro explicado e sem copiar credenciais; manter o perfil fora da limpeza de caches.

## Migration Plan

Sem nova migração de banco presumida. Preservar escolhas e história existentes; migrar todos os lançamentos Agy para o contexto privado, sem retomar IDs de outro perfil. Remover a guarda compartilhada, a recusa baseada em extensões pessoais e textos contraditórios de HOME, mantendo o isolamento e a política verificados. Ingestão pessoal e rotas ASR/ditado não mudam. Atualizar o contexto durável do projeto depois de implementar; nunca apagar perfis autenticados como cache.

Integrar após RED/GREEN, revisão final completa e scanner; revisão automática já indisponível usa contingência manual registrada, nunca aprovação fictícia. Build assinado e smoke no Mac devem usar a fonte integrada. Rollback de código não permite abrir o perfil sintético schema56 em binário737; preservar backup integral. Publicação na main sem force é autorizada; release/deploy e instalação sobre o app real não fazem parte desta change. Arquivar somente com todos os aceites; sincronizar AGENTS e fechar recursos próprios com recibos, preservando dados e provas.
