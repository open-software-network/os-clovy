# Reunião assistida

## ADDED Requirements

### Requirement: Mesma nota antes, durante e depois
A recording session SHALL manter o início explícito, indicador e controles de
captura. A nota SHALL mostrar fontes Você / Outros e substituir preview pelos
resultados definitivos sem duplicação, respeitando decisões de descarte.

#### Scenario: Encerrar e reabrir
- **WHEN** a pessoa encerra e reabre a nota pelo histórico
- **THEN** encontra marcações persistidas, transcrição final e resumo existente,
  com decisões e próximos passos apenas quando sustentados pelo conteúdo
- Test: frontend + SQLite; captura nativa depende da plataforma

### Requirement: Marcação com tempo ativo
Marcações SHALL ser duráveis e qualificadas por nota e sessão. Pausas MUST NOT
avançar o tempo das marcações nem contaminar outra sessão.

#### Scenario: Pausa e isolamento
- **WHEN** a pessoa marca, pausa, retoma e inicia outra reunião
- **THEN** as marcações permanecem nos tempos ativos e na nota de origem
- Test: Rust repository/command contracts + frontend

### Requirement: Assistência explícita e limitada
O pedido de sugestão SHALL usar o motor Notes selecionado, contexto recente
da sessão ativa e nenhuma ferramenta. Pausa, stop e desativação da transcrição
ao vivo SHALL impedir novos pedidos. Falha MUST ser visível sem parar a captura.

#### Scenario: Motor indisponível
- **WHEN** o motor falha ou responde depois da pausa/stop
- **THEN** nenhum texto obsoleto é apresentado como sugestão atual e o áudio
  permanece disponível para processamento/retry existente
- Test: falha injetada e resposta atrasada em testes de integração
