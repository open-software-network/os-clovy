## ADDED Requirements

### Requirement: Idioma do texto gerado de atividade
Todo texto gerado a partir da atividade (relatórios por hora, frentes, resumo do dia e resumos de blocos de agentes de código) SHALL sair no idioma da interface. O sistema MUST NOT gerar esse texto antes de saber o idioma da interface; o último idioma escolhido MUST ficar guardado para valer desde o início da próxima abertura. Relatórios e resumo do dia corrente gerados em outro idioma SHALL ser gerados de novo no idioma atual.

#### Scenario: Abertura com atividade pendente
- **WHEN** a interface está em português e o app abre com horas concluídas sem relatório
- **THEN** os relatórios dessas horas saem em português, mesmo que a janela ainda não tenha carregado
- Test: unit — idioma guardado lido no início; laço de geração espera o idioma conhecido

#### Scenario: Relatórios do dia em outro idioma
- **WHEN** o dia corrente tem relatórios gravados em inglês e a interface está em português
- **THEN** esses relatórios e o resumo do dia são gerados de novo em português
- Test: unit — seleção de relatórios do dia com idioma diferente do atual
