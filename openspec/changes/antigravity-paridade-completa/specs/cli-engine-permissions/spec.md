# Spec Delta

## ADDED Requirements

### Requirement: Autoridade das ferramentas independente do motor
A autorização das ferramentas do Clovy SHALL ser decidida pelo app com a mesma política nos três motores. A troca de runtime MUST NOT alterar partição, superfície, modo de aprovação ou concessões do usuário. Cada turno MUST possuir vínculo próprio e revogável; configuração e identificadores internos do runtime MUST NOT substituir esse vínculo.

#### Scenario: Mesmo pedido e mesma política
- **WHEN** Claude Code, Codex ou Antigravity pede a mesma ferramenta do Clovy na mesma superfície e política
- **THEN** o app aplica o mesmo catálogo, validação, aprovação, execução e formato de resultado

#### Scenario: Motor atualizado para uma execução futura
- **WHEN** o usuário seleciona outro motor durante a execução de uma rotina
- **THEN** a execução atual conserva seu vínculo e política, e a próxima recebe vínculo próprio e sua política efetiva

#### Scenario: Autoridade expirada
- **WHEN** um turno termina, falha, é cancelado ou o app encerra
- **THEN** suas conexões perdem autoridade e uma ferramenta atrasada não executa usando o vínculo de outro turno ou motor

### Requirement: Conexão temporária sem alterar configuração do usuário
O app SHALL entregar ao Antigravity a conexão privada do gateway em perfil e diretório de execução pertencentes ao Clovy, sem editar configuração pessoal nem copiar credenciais. HOME/XDG MUST mudar somente no processo filho. A configuração temporária MUST impedir colisão ou reutilização de vínculo entre turnos e ser encerrada em sucesso, erro, cancelamento e recuperação de crash. O timeout MUST exceder a espera do broker; autenticação durável MUST NOT ser removida junto com artefatos de turno.

#### Scenario: Configuração preexistente e concorrência
- **WHEN** o workspace contém configuração MCP ou outra sessão compartilha o mesmo diretório
- **THEN** o app não modifica esse workspace, prepara a conexão em diretório próprio e impede que um motor use a autorização do outro turno

#### Scenario: Falha na preparação ou encerramento
- **WHEN** preparar a conexão falha ou o processo termina antes da resposta
- **THEN** o app revoga a autorização, não deixa configuração utilizável do turno e não remove dados ou entradas de terceiros

#### Scenario: Perfil durável e transporte temporário
- **WHEN** um turno termina ou seu preparo falha após o perfil ter sido autenticado
- **THEN** o app revoga o vínculo e limpa somente o transporte próprio, mantendo autenticação, histórico canônico e dados alheios

#### Scenario: Configuração própria insegura
- **WHEN** o destino do perfil ou transporte é um symlink ou tem propriedade/permissões incompatíveis com sua privacidade
- **THEN** o app falha antes de expor autoridade, sem reconfigurar diretórios pessoais ou apagar dados de origem não demonstrada

#### Scenario: Identidade do relay preservada
- **WHEN** Agy executa com HOME privado e inicia o relay MCP
- **THEN** o relay usa o canal absoluto e o vínculo do app, não outro armazenamento, e sua proteção de desenvolvimento contra acesso a dados de produção continua efetiva


### Requirement: Rotinas com qualquer um dos três motores
O usuário SHALL poder executar rotinas com Claude Code, Codex ou Antigravity, inclusive quando só um deles está instalado. A política não assistida MUST ser a mesma para ferramentas do Clovy, sem aprovação implícita, bypass ou uso de outro runtime. Restrições e capacidades próprias do CLI MUST ser verificadas antes de disponibilizar esse modo; uma flag de sandbox isolada MUST NOT ser aceita como prova.

#### Scenario: Leitura autorizada em rotina Antigravity
- **WHEN** uma rotina Antigravity solicita uma leitura do gateway permitida pela política não assistida
- **THEN** a leitura é executada e o resultado fica no histórico, sem instalar ou chamar Claude Code ou Codex

#### Scenario: Escrita sem pessoa para aprovar
- **WHEN** qualquer um dos três motores solicita uma escrita que exige aprovação num turno não assistido
- **THEN** o broker recusa imediatamente com o mesmo motivo e nenhum efeito ou cartão pendente é criado

#### Scenario: Configuração global permissiva
- **WHEN** o usuário tem permissões globais mais amplas no runtime
- **THEN** um turno do Clovy não ganha autoridade adicional sobre ferramentas, comandos, arquivos ou conexões além da política efetiva do app

### Requirement: Execução Clovy com extensões globais ligadas
O Clovy SHALL funcionar com extensões globais do runtime ligadas sem exigir sua desativação. Hooks, plugins, MCPs externos e processos auxiliares MUST NOT ampliar a autoridade da execução Clovy nem produzir efeitos fora da política do app. O isolamento MUST ser específico dessa execução e preservar autenticação e configuração usadas fora do Clovy; detectar uma extensão e recusar permanentemente o modo MUST NOT contar como paridade entregue.

#### Scenario: Hook global não concede autoridade
- **WHEN** o runtime tem hooks ou plugins ativos e o Clovy inicia um turno, rotina ou geração pontual
- **THEN** a operação autorizada funciona e automações externas não executam efeitos fora da política, sem desativar a extensão global nem trocar o motor

#### Scenario: Processo auxiliar fora da restrição
- **WHEN** um processo auxiliar ou backend previamente iniciado poderia executar fora do isolamento do turno
- **THEN** o app não aceita essa configuração como contida; a prova deve qualificar o isolamento efetivo de toda a execução antes de liberar o modo

### Requirement: Resultado real e terminalidade comuns
Os adaptadores SHALL normalizar os eventos reais sem criar sucesso, saída ou efeito não observado. Cancelar MUST encerrar a árvore de processos própria, revogar o vínculo e encerrar indicadores pendentes. Resultado real tardio MUST ser preservado segundo o contrato do histórico, sem reativar o run ou contaminar outro motor.

#### Scenario: Resultado ausente ao parar
- **WHEN** o usuário para qualquer um dos três motores durante uma chamada sem resultado observado
- **THEN** a ferramenta fica interrompida, sem spinner permanente nem afirmação de rollback

#### Scenario: Falha terminal do Antigravity
- **WHEN** o Antigravity emite erro terminal, termina sem resposta válida ou devolve somente o resultado final
- **THEN** o app respeita o resultado e o estado reais, exibe a resposta quando existente e não converte ausência de eventos intermediários em sucesso vazio
