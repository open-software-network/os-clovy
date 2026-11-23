# Spec Delta

## Purpose

Permitir trabalhar no app nativo em desenvolvimento com uma identidade reconhecível e estável no macOS, preservando autorizações reais entre reinícios e recompilações sem instalar o produto de release nem usar dados de produção.

## ADDED Requirements

### Requirement: Inicialização preserva permissões
O fluxo normal de dev SHALL preservar autorizações existentes do app e dos helpers e MUST NOT executar resets de privacidade ou remover entradas de permissão como preparação de uma inicialização.

#### Scenario: Reinício com autorizações existentes
- **WHEN** o desenvolvedor reinicia o mesmo dev já autorizado
- **THEN** o launcher não revoga Acessibilidade, Screen Recording, microfone nem notificações
- **AND** o app continua consultando os estados reais do macOS
- Test: integration — regressão comportamental do launcher e prova nativa de reinício descrita em design.md.

### Requirement: Identidade reconhecível e estável
O dev SHALL fornecer um bundle local selecionável nos Ajustes do macOS e manter identidade, caminho responsável e requisito de assinatura estáveis entre builds normais usando a mesma identidade válida de desenvolvimento.

#### Scenario: Recompilação normal
- **WHEN** o desenvolvedor recompila código do app e reinicia pelo launcher documentado, usando a mesma identidade de assinatura
- **THEN** o macOS reconhece o mesmo app e os mesmos helpers responsáveis pelas autorizações anteriores
- **AND** helpers com conteúdo inalterado não são substituídos ou assinados novamente sem necessidade
- Test: e2e — duas execuções do runner com build intermediário, designated requirements e estado real das permissões.

### Requirement: Autorizações continuam reais
O dev MUST NOT falsificar permissões, escrever no banco de TCC, alterar proteções do sistema nem modificar configuração ou itens de credencial do Keychain para contornar autorizações.

#### Scenario: Primeira autorização necessária
- **WHEN** uma permissão nunca foi concedida ao bundle dev responsável
- **THEN** o app informa corretamente a permissão ausente e oferece o alvo concreto correto para a concessão pelo macOS
- **AND** não informa prontidão até observar a permissão real
- Test: e2e — fluxo nativo existente com a identidade dev correta e controle de permissão ausente em sandbox.

### Requirement: Fronteiras de dev permanecem protegidas
O launcher SHALL preservar isolamento dos perfis de produção, separação entre worktrees e autenticação restrita dos helpers ao executável dev legítimo.

#### Scenario: Runner com bundle local
- **WHEN** o runner inicia o bundle local de dev
- **THEN** os helpers autenticam o parent legítimo sem aceitar executáveis externos ou aliases não correspondentes
- **AND** o app usa o perfil dev previsto e não o perfil de produção
- Test: integration — identidade do parent e self-test do helper pelo launcher exato; confirmação de perfil por lsof.
