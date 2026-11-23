# Spec Delta

## ADDED Requirements

### Requirement: Quadro útil
Um quadro SHALL ser útil quando o app não é de tela bloqueada ou protetor de tela e ao menos um destes vale: a janela (app, título ou URL) mudou, o texto mudou, houve evento de entrada gravado perto do quadro, ou o tempo desde a última entrada do sistema é menor ou igual ao intervalo entre ticks mais 1 segundo. Os demais quadros SHALL ser ociosos. Sem o tempo do sistema no quadro, vale só o restante da regra.

#### Scenario: Leitura com rolagem
- **WHEN** o usuário passa 10 minutos lendo a mesma página, sem clique nem tecla, rolando a cada poucos segundos
- **THEN** os quadros são úteis, o intervalo conta como sessão focada e não aparece lacuna ociosa
- Test: unit — construtor com quadros iguais e tempo desde a última entrada de 1 s

#### Scenario: Tela parada sem entrada
- **WHEN** a janela não muda por 6 minutos e o tempo desde a última entrada cresce em todos os quadros
- **THEN** os quadros são ociosos e o intervalo vira lacuna ociosa
- Test: unit — construtor com quadros iguais e tempo desde a última entrada crescente

#### Scenario: Quadros antigos sem o dado
- **WHEN** a linha do tempo é reconstruída com quadros gravados antes desta mudança
- **THEN** a classificação desses quadros é a mesma de antes
- Test: unit — construtor com quadros sem tempo desde a última entrada
