# Spec Delta

## MODIFIED Requirements

### Requirement: Transcrição direta com chave própria
A transcrição do ditado SHALL ir ao motor de transcrição escolhido em Configurações: com Groq, sempre direto do desktop à Groq com a chave do usuário; com Local, ao modelo instalado na máquina, sem rede. A limpeza ligada SHALL ir sempre direto à OpenRouter. Não existe rota de reserva entre motores. Com o motor Groq e sem a chave Groq, o ditado MUST mostrar que a chave falta e onde salvá-la, sem requisição de rede; com o motor Local, a chave Groq MUST NOT ser exigida. Sem a chave OpenRouter, a limpeza é pulada e o transcrito bruto é colado com aviso discreto. A chave MUST ir só ao provedor dela.

#### Scenario: Com chave Groq própria
- **WHEN** o motor é Groq, a chave do Groq está salva e o usuário dita
- **THEN** a requisição vai a `api.groq.com` e nenhuma outra requisição de transcrição é feita

#### Scenario: Sem chave própria
- **WHEN** o motor é Groq, não há chave do Groq salva e o usuário aperta o atalho do ditado
- **THEN** o HUD mostra que falta a chave Groq e onde salvá-la, e nenhuma requisição sai da máquina

#### Scenario: Motor Local
- **WHEN** o motor é Local e o usuário dita
- **THEN** a tomada é transcrita pelo modelo local, o texto é colado e nenhuma requisição de transcrição sai da máquina, haja ou não chave Groq salva

#### Scenario: Limpeza ligada sem chave OpenRouter
- **WHEN** a limpeza está ligada, a transcrição teve sucesso e não há chave OpenRouter
- **THEN** o transcrito bruto é colado e o usuário vê o aviso de chave OpenRouter ausente

Test: integration — servidores HTTP falsos no lugar de Groq e OpenRouter e um servidor sentinela em porta efêmera, injetada como a URL legada, que MUST receber zero requisições; cabeçalhos levam só a chave do provedor chamado; com motor Local, transcritor local falso e zero requisições à Groq falsa.

Nota de verificação (orquestrador, 2026-10-28): os testes automatizados usam porta efêmera para evitar conflitos locais. A prova nativa em `127.0.0.1:8080` é separada, executada somente com a porta livre e limitada aos fluxos autorizados, conforme a tarefa 7.2 e seu relatório. Isso não declara sucesso dos fluxos reais de provedores adiados para validação manual pós-merge pelo dono.
