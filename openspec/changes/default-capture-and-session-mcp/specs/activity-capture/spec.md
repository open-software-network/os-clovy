# Spec Delta

## Purpose

Define a inicialização automática da captura local de atividade e a persistência do opt-out sem dispensar os controles de privacidade existentes.

## ADDED Requirements

### Requirement: Captura iniciada por padrão
O Clovy SHALL iniciar a captura de atividade ao abrir, sem ativação manual, para configuração nova ou campos ausentes. Permissões obrigatórias ausentes MUST impedir a gravação e mostrar o bloqueio, sem converter a preferência ligada em desligada. Pausas, horários e filtros existentes MUST continuar aplicáveis.

#### Scenario: Primeira abertura
- **WHEN** o Clovy abre sem preferência de captura e com permissões necessárias
- **THEN** a captura fica ligada e processa atividade sem clique de ativação
- Test: integration — engine com configuração carregada de diretório temporário e permissões controladas; smoke nativo para estado visível

#### Scenario: Permissão ausente
- **WHEN** o Clovy abre sem preferência de captura e faltam permissões obrigatórias
- **THEN** a preferência permanece ligada, a interface explica o bloqueio e nenhum quadro é gravado
- Test: integration — engine com permissões ausentes e verificação de ausência de gravação; smoke nativo se o ambiente não tiver permissão

### Requirement: Migração única e opt-out durável da captura
Na primeira abertura após esta atualização, o Clovy SHALL ativar a captura dos perfis legados uma única vez, inclusive `false` antigo, conforme autorização do dono. O upgrade MUST ser gravado na abertura, antes de qualquer nova escolha; desligamento posterior salvo MUST sobreviver a todos os reinícios e às gravações de configurações do MCP. Exceção: se a gravação do upgrade falhar, o Clovy MUST registrar a falha e seguir com a captura ligada só em memória, sem declarar o upgrade persistido (a próxima abertura o repete). Uma escolha cuja gravação falhar MUST retornar erro e não ser aplicada nem anunciada; a primeira gravação bem-sucedida de qualquer switch MUST persistir o upgrade junto com a escolha.

#### Scenario: Perfil legado desligado
- **WHEN** um perfil anterior com captura desligada abre e depois reinicia
- **THEN** a captura fica ligada nas duas aberturas sem configuração manual
- Test: integration — carga, migração e recarga reais de configuração temporária

#### Scenario: Desligamento após migração
- **WHEN** o usuário desliga a captura após a primeira abertura atualizada e reinicia duas vezes
- **THEN** a captura continua desligada, sem nova migração nem gravação de quadros
- Test: integration — persistência e engine; smoke nativo por UI e reinício

#### Scenario: Falha ao gravar o upgrade
- **WHEN** o upgrade de um perfil legado não pode ser gravado, o usuário tenta desligar a captura sem conseguir gravar e depois desliga com gravação bem-sucedida
- **THEN** o arquivo segue legado e a captura ligada após a falha, a tentativa falha retorna erro sem alterar o estado, e o desligamento salvo sobrevive a dois reinícios
- Test: integration — diretório de configuração sem escrita, verificação do arquivo durável e recarga real
