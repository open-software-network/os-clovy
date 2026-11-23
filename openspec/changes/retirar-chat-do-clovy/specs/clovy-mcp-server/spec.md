# Spec Delta

## MODIFIED Requirements

### Requirement: Configuração para clientes
As configurações SHALL mostrar o comando e o trecho de configuração prontos para Claude Code e Cursor, com botão de copiar. O transporte MUST continuar sendo o relay local autenticado por instalação.

#### Scenario: Copiar configuração
- **WHEN** o usuário abre Configurações MCP com o servidor ligado
- **THEN** pode copiar o trecho de configuração para Claude Code ou Cursor

#### Scenario: Cliente já configurado
- **WHEN** um agente externo já configurado conecta após a atualização
- **THEN** pode inicializar, listar e ler com as mesmas configurações e filtros

## ADDED Requirements

### Requirement: Autoridade de clientes MCP externos
Clientes sem vínculo de turno interno SHALL manter o acesso de leitura existente. Somente conexões externas locais autenticadas pelo relay SHALL poder descobrir e invocar as ferramentas adicionais autorizadas para essa superfície. O host MUST vincular a chamada ao cliente e à autoridade de dados e, quando a operação puder produzir efeito ou custo, exigir aprovação da operação exata. Uma conexão sem identidade externa validada continua limitada à leitura.

#### Scenario: Cliente externo pede gravação
- **WHEN** um cliente externo autenticado pede para salvar uma memória
- **THEN** a ferramenta registra a solicitação com a prévia, escreve apenas após aprovação e devolve o resultado ao mesmo cliente

#### Scenario: Alvo ou autorização muda
- **WHEN** um pedido é recusado, expira, é cancelado, perde sua conexão ou o perfil muda antes do efeito
- **THEN** a chamada termina com erro sem aplicar a mudança

### Requirement: Ferramentas para agentes externos
O relay SHALL reutilizar os handlers e a validação do catálogo do Clovy para todas as ferramentas aplicáveis a um agente externo. O host MUST fornecer contexto isolado para ferramentas que dele precisem, anunciar apenas ferramentas realmente habilitadas e nunca vincular chamadas externas a conversas internas por ID fornecido pelo cliente. Extração de dados, alterações, conectores, mídia e automação devem continuar sujeitos aos escopos, permissões e pré-condições do produto.

#### Scenario: Descoberta e chamada sem chat
- **WHEN** o agente externo chama as ferramentas anunciadas através do relay
- **THEN** recebe resultados no cliente externo, com aprovação no Clovy quando aplicável, sem abrir ou criar conversa no app

## REMOVED Requirements

### Requirement: Clientes sem vínculo continuam só leitura
