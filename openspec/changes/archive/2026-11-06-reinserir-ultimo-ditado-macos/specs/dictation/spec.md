# Spec Delta

## ADDED Requirements

### Requirement: Reinserção explícita do último ditado no macOS
No macOS, Control+V SHALL reinserir o último texto válido de ditado disponível nesta execução e no perfil ativo, no campo atualmente focado. A reinserção MUST funcionar independentemente do clipboard, sem alterar seu conteúdo, reativar o destino antigo ou iniciar transcrição, limpeza ou ação de agente.

#### Scenario: Colagem automática não entrou no campo
- **WHEN** uma transcrição válida já terminou, o clipboard anterior foi restaurado e o usuário foca outro campo e aperta Control+V
- **THEN** o último ditado é inserido uma vez no campo atual e o clipboard permanece inalterado

#### Scenario: Conteúdo Unicode
- **WHEN** o último ditado contém acentos, emoji e quebras de linha e o usuário aperta Control+V
- **THEN** a inserção preserva esse texto, sem substituições por combinações de teclas ou truncamento

#### Scenario: Colagem normal do sistema
- **WHEN** o usuário aperta Command+V no macOS
- **THEN** o macOS continua colando o conteúdo normal do clipboard, sem disparar a reinserção

### Requirement: Memória do último ditado respeita validade e isolamento
O Clovy SHALL manter somente em memória um último ditado elegível, associado ao perfil que o produziu. Um novo resultado válido SHALL substituir o anterior. Silêncio, cancelamento, texto rejeitado e resultado tardio sem autoridade MUST NOT substituir essa memória. Reinserir MUST NOT criar outro item de histórico.

#### Scenario: Falha de colagem não perde o resultado válido
- **WHEN** a transcrição de um ditado válido terminou mas sua colagem automática falhou
- **THEN** esse texto continua disponível para Control+V

#### Scenario: Tomada cancelada depois de um ditado válido
- **WHEN** existe um ditado elegível e outra tomada é cancelada, vazia ou rejeitada
- **THEN** Control+V continua usando o ditado elegível anterior

#### Scenario: Outro perfil ou execução sem ditado
- **WHEN** o perfil ativo não é o dono do texto em memória ou o app acabou de iniciar
- **THEN** Control+V não insere texto nem recupera ditados do disco ou de outro perfil

### Requirement: Atalho de reinserção preserva os controles nativos
A reinserção SHALL exigir a Acessibilidade existente e respeitar a captura de atalhos. O app MUST NOT pedir Input Monitoring para esse recurso nem registrar Control+V no Windows/Linux. Conflitos MUST preservar atalhos de gravação existentes e reportar a indisponibilidade do atalho de reinserção, sem registrá-lo silenciosamente como funcional.

#### Scenario: Permissão ausente
- **WHEN** Control+V é acionado sem Acessibilidade
- **THEN** nenhum texto é inserido nem copiado e o app sinaliza a permissão ausente pelo caminho existente

#### Scenario: Captura ou conflito de atalho
- **WHEN** o usuário está capturando um atalho ou Control+V conflita com outro atalho de gravação
- **THEN** não ocorre reinserção indevida e os controles existentes continuam utilizáveis
