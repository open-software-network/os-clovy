# Spec Delta

## MODIFIED Requirements

### Requirement: Transcrição direta com chave própria
A transcrição do ditado SHALL ir sempre direto do desktop à Groq com a chave do usuário, e a limpeza ligada SHALL ir sempre direto à OpenRouter. Não existe rota de reserva. Sem a chave Groq, o ditado MUST mostrar que a chave falta e onde salvá-la, sem requisição de rede. Sem a chave OpenRouter, a limpeza é pulada e o transcrito bruto é colado com aviso discreto. A chave MUST ir só ao provedor dela.

#### Scenario: Com chave Groq própria
- **WHEN** a chave do Groq está salva e o usuário dita
- **THEN** a requisição vai a `api.groq.com` e nenhuma outra requisição de transcrição é feita

#### Scenario: Sem chave própria
- **WHEN** não há chave do Groq salva e o usuário aperta o atalho do ditado
- **THEN** o HUD mostra que falta a chave Groq e onde salvá-la, e nenhuma requisição sai da máquina

#### Scenario: Limpeza ligada sem chave OpenRouter
- **WHEN** a limpeza está ligada, há chave Groq e não há chave OpenRouter
- **THEN** o transcrito bruto é colado e o usuário vê o aviso de chave OpenRouter ausente

Test: integration — servidores HTTP falsos no lugar de Groq e OpenRouter e um servidor sentinela em porta efêmera, injetada como a URL legada, que MUST receber zero requisições; cabeçalhos levam só a chave do provedor chamado.

Nota de verificação (orquestrador, 2026-10-28): os testes automatizados usam porta efêmera para evitar conflitos locais. A prova nativa em `127.0.0.1:8080` é separada, executada somente com a porta livre e limitada aos fluxos autorizados, conforme a tarefa 7.2 e seu relatório. Isso não declara sucesso dos fluxos reais de provedores adiados para validação manual pós-merge pelo dono.
