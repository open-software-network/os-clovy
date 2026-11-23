# Spec Delta

## ADDED Requirements

### Requirement: Motores intercambiáveis sem dependência cruzada
O Clovy SHALL funcionar com apenas Claude Code, apenas Codex ou apenas Antigravity instalado, em todas as superfícies que aceitam esses motores. A escolha explícita MUST prevalecer. Sem escolha explícita, a resolução automática MUST usar um motor elegível instalado, mantendo a preferência existente e incluindo Antigravity. Falta de autenticação MUST ser explicada, sem trocar de motor silenciosamente.

#### Scenario: Um único motor instalado
- **WHEN** somente um dos três motores está instalado
- **THEN** rotinas, geração de notas e Atividade usam esse motor nos usos elegíveis, sem iniciar nem exigir os outros dois

#### Scenario: Escolha explícita preservada
- **WHEN** o usuário já escolheu um dos três motores e muda a lista de instalados
- **THEN** a escolha é mantida e uma eventual indisponibilidade é explicada sem troca silenciosa

### Requirement: Perfil Antigravity e autenticação gerenciados pelo app
O Clovy SHALL usar um único contexto de perfil Antigravity próprio para rotinas, notas, atividade, teste de provedor e entrada de autenticação nativa. O ambiente pessoal do app e a ingestão de sessões MUST permanecer independentes. Instalação ou versão do binário MUST NOT ser apresentadas como prova de autenticação desse perfil; o app MUST NOT copiar credenciais, usar outro motor ou aceitar consentimento nativo em nome da pessoa.

#### Scenario: Primeira autenticação no Clovy
- **WHEN** o usuário inicia a autenticação do Antigravity nos ajustes do Clovy
- **THEN** o app abre o fluxo nativo no perfil próprio, sem prompt de tarefa e sem alterar o HOME do processo principal ou o perfil pessoal

#### Scenario: Teste e execução compartilham a origem
- **WHEN** o usuário testa o provedor e depois executa rotina, geração de nota ou resumo com Antigravity
- **THEN** essas operações usam o mesmo perfil gerenciado; falta de login é explicada sem tomar o login pessoal ou outro runtime como substituto

#### Scenario: Preservação de dados após adoção do perfil próprio
- **WHEN** existe um identificador Antigravity criado no HOME anterior ou de origem desconhecida
- **THEN** o app preserva os dados e seus limites de acesso, sem assumir o identificador como autoridade do perfil atual


### Requirement: Pool único das ferramentas do Clovy
Claude Code, Codex e Antigravity SHALL receber do mesmo gateway o catálogo autorizado das ferramentas do Clovy e de seus conectores. O alcance MUST depender da superfície, do vínculo e da política do app, não do motor. Credenciais, efeitos e aprovações MUST permanecer sob a autoridade do Clovy, sem catálogos ou autorizações paralelos por runtime. Clientes externos seguem o contrato de `retirar-chat-do-clovy`.

#### Scenario: Acesso a nota de uma rotina
- **WHEN** uma rotina em qualquer um dos três motores referencia uma nota autorizada e solicita leitura
- **THEN** a leitura usa o gateway do Clovy e retorna o resultado com o mesmo contrato nos três motores

#### Scenario: Ação protegida ou execução cancelada
- **WHEN** uma rotina pede uma ação protegida sem autorização ou sua execução é cancelada
- **THEN** a ação sem autorização é recusada e a execução cancelada perde o vínculo, com estado e resultado verdadeiros no histórico

#### Scenario: Conector e ferramenta com pré-requisito
- **WHEN** uma ferramenta do gateway depende de conector, credencial, permissão nativa ou provedor de mídia
- **THEN** o pré-requisito e o erro são iguais nos três motores; trocar o motor não concede autorização nem remove a ferramenta por restrição específica do adaptador

### Requirement: Antigravity nos recursos de texto
Antigravity SHALL poder executar a geração de notas e os recursos de atividade que já aceitam provedores CLI, após teste efetivo do schema necessário. Conteúdo enviado MUST ser tratado como dado não confiável e identificado como autoria do Clovy para evitar autoingestão. Esse suporte MUST NOT exigir a instalação de outro agente nem alterar o motor de transcrição ou a limpeza de ditado.

#### Scenario: Nota e atividade sem outros agentes
- **WHEN** Antigravity é o único agente instalado, é escolhido para Notas e passa no teste de schema para Atividade
- **THEN** uma transcrição sintética gera a nota e os dados sintéticos de atividade geram o resumo esperado sem chamada a Claude Code, Codex ou provedor alternativo

#### Scenario: Extensões globais continuam ligadas
- **WHEN** o usuário escolhe Antigravity para gerar nota ou resumo e mantém suas extensões globais ligadas
- **THEN** o Clovy executa a geração autorizada com isolamento próprio, preservando a configuração externa e sem exigir outro motor ou desativação manual das extensões

#### Scenario: Falha ou schema inválido
- **WHEN** Antigravity falha, excede o prazo ou não devolve o schema exigido
- **THEN** o uso falha com diagnóstico explícito, sem resultado falso, processo órfão ou troca de provedor

#### Scenario: Leitura de sessões é independente
- **WHEN** o usuário liga a leitura de sessões Antigravity
- **THEN** o provedor selecionado para Atividade e os demais usos permanece o mesmo, os arquivos originais são preservados e as chamadas marcadas do próprio Clovy são excluídas da ingestão
