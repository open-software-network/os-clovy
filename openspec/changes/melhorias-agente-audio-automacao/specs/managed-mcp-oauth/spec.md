# Spec Delta

## Purpose

Gerenciar conexões MCP OAuth pelo navegador sem copiar API keys e distinguir credenciais/problemas administrados pelo Clovy de integrações herdadas do CLI.

## ADDED Requirements

### Requirement: Vercel pelo navegador com identidade própria
O app SHALL oferecer conexão Vercel oficial via OAuth, descoberta, PKCE/state e callback local de código trocado por tokens no app. MUST usar identidade própria aceita pelo provedor, armazenamento seguro existente e consentimento do usuário. Não SHALL usar client_id de terceiro, tokens em URL/log/argv públicos ou marcar sucesso antes de catálogo válido. Exigência externa de revisão do cliente MUST ficar explícita sem sucesso fictício.

#### Scenario: Conectar e cancelar
- **WHEN** usuário inicia conexão e conclui ou cancela consentimento
- **THEN** somente conclusão/catálogo válidos persistem conexão; cancelamento, state inválido e timeout não deixam estado conectado
- Test: integration — servidor OAuth fixture; live Mac/Windows após aceite Vercel e consentimento

### Requirement: Refresh e desconexão coerentes
Quando concedido, refresh SHALL renovar com rotação segura; revogação/invalid_grant exige reconnect. Disconnect MUST invalidar transporte/ações concorrentes, limpar credenciais próprias e informar falha de revogação remota sem esconder custódia local remanescente. Não SHALL alterar login herdado do CLI. Mutação com entrega incerta MUST NOT ser repetida automaticamente após refresh.

#### Scenario: Desconectar durante refresh
- **WHEN** refresh/reconexão concorre com disconnect
- **THEN** segredo/conexão não ressuscita, operações antigas falham e estado efetivo na UI concorda
- Test: integration — rotação, CAS/geração, revogação e reinício

### Requirement: Origem e causas acionáveis
Catálogo/UI/erros SHALL distinguir conexão gerenciada Clovy de CLI externo, com servidor/ferramenta/fase/código e ação possível, sem segredo. `UNAUTHORIZED` externo não implica chave nativa errada; `Unknown tool` externo não justifica inventar namespace no app. Contexto/diagnóstico MUST preservar aprovação, sandbox, auth, capability, driver e permissão do SO como camadas distintas.

#### Scenario: Mesmo provedor em duas origens
- **WHEN** conexão herdada falha e gerenciada está saudável
- **THEN** app identifica a origem correta sem apagar a saudável nem duplicar autoridades silenciosamente
- Test: integration/component — falhas tipadas e catálogo; live — leitura autorizada Vercel nos dois CLIs
