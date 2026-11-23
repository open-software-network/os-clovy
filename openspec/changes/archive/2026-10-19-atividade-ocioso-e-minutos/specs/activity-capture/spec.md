# Spec Delta

## ADDED Requirements

### Requirement: Tempo desde a última entrada do sistema
Cada quadro SHALL registrar há quantos milissegundos o sistema operacional não recebe nenhuma entrada do usuário (teclado, mouse, rolagem, trackpad), lido no tick que gerou o quadro. O valor MUST ser só uma duração, sem tipo, posição ou conteúdo da entrada. Quando o sistema não informar o valor, o quadro MUST ser gravado sem ele, e a leitura MUST NOT exigir permissão além das que a captura já pede.

#### Scenario: Mouse movido entre dois ticks
- **WHEN** o usuário só move o mouse sobre uma janela parada e o próximo tick grava um quadro
- **THEN** o quadro registra um tempo desde a última entrada menor que o intervalo entre ticks, e nenhum evento de mouse é gravado
- Test: unit — motor de captura com plataforma falsa que informa tempo ocioso de 500 ms

#### Scenario: Sistema sem o dado
- **WHEN** a plataforma não consegue informar o tempo desde a última entrada
- **THEN** o quadro é gravado normalmente, sem o valor
- Test: unit — motor de captura com plataforma falsa que não informa o tempo ocioso

#### Scenario: Sem Monitoramento de Entrada
- **WHEN** a captura roda no macOS sem a permissão de Monitoramento de Entrada
- **THEN** os quadros continuam registrando o tempo desde a última entrada
- Test: e2e — app de desenvolvimento sem a permissão, leitura do quadro pelo export de depuração
