# Spec Delta

## MODIFIED Requirements

### Requirement: Seleção de provedor por uso
O sistema SHALL permitir escolher o provedor separadamente para: chat/agente, geração de notas, limpeza de ditado e recursos de atividade (relatórios, resumo do dia, resumo de sessões de agentes). O chat MUST aceitar só motores CLI. As notas MUST aceitar CLI, endpoint cadastrado ou OpenRouter com a chave do usuário. A limpeza de ditado usa a OpenRouter. Nenhum uso MUST passar pelo Clovy API. Para recursos de atividade, o padrão MUST ser "nenhum", e sem provedor escolhido esses recursos ficam desligados.

#### Scenario: Atividade sem provedor
- **WHEN** nenhum provedor de atividade foi escolhido e chega a hora de gerar o resumo do dia
- **THEN** nenhum texto de atividade sai da máquina e a interface mostra que o recurso precisa de um provedor configurado
- Test: integration — agendador com provedor "nenhum" não realiza chamada e registra estado desligado

#### Scenario: Nota gerada por CLI
- **WHEN** o provedor de notas é `claude` e uma gravação termina de ser transcrita
- **THEN** a nota estruturada é gerada pelo CLI e aparece no editor como as notas geradas pela OpenRouter
- Test: integration — CLI falso devolvendo nota e verificação da nota persistida

#### Scenario: Perfil salvo com uso Clovy
- **WHEN** um `provider-settings.json` salvo tem `notes` ou `dictationCleanup` com `kind: "clovy"` e o app abre
- **THEN** notas passam a OpenRouter e a limpeza segue na OpenRouter, sem chamada ao Clovy API, e o arquivo é regravado com o valor novo
- Test: integration — fixture de `provider-settings.json`, carga, gravação e recarga

### Requirement: Notas com chave própria vão direto ao provedor
Com o motor de Notas = OpenRouter, o desktop SHALL mandar a transcrição de nota à Groq e a geração e a limpeza da nota à OpenRouter, sempre direto, com um modelo explícito. A requisição MUST levar só a chave do provedor chamado. Não existe rota de reserva: sem a chave, a chamada falha como descrito em "Falha clara sem chave nem servidor".

#### Scenario: Transcrição final com chave Groq
- **WHEN** a chave Groq está salva e um turno da reunião é transcrito
- **THEN** a requisição vai a `/audio/transcriptions` da Groq com modelo, `response_format=json`, idioma, prompt do dicionário e nome de arquivo canônico
- Test: integration — Groq falsa confere o corpo multipart e os cabeçalhos

#### Scenario: Prévia ao vivo com chave Groq
- **WHEN** a chave Groq está salva e a prévia ao vivo envia um trecho
- **THEN** o trecho vai direto à Groq
- Test: integration — pedido de prévia contra Groq falsa

#### Scenario: Sem chave Groq
- **WHEN** não há chave Groq salva e uma gravação começa e termina
- **THEN** a prévia ao vivo não inicia, a transcrição falha com `groq_api_key_missing` e nenhuma requisição de transcrição ou prévia é feita
- Test: unit — portão da prévia com e sem chave; integration — servidor sentinela em porta efêmera, injetada como a URL legada, com zero requisições

#### Scenario: Falha transitória da Groq
- **WHEN** a Groq responde 503 na primeira tentativa e 200 na segunda
- **THEN** a transcrição devolve o texto da segunda resposta depois de exatamente duas requisições; um 400 não é repetido
- Test: integration — Groq falsa com contador de chamadas

#### Scenario: Áudio sem fala
- **WHEN** a Groq devolve texto vazio
- **THEN** a transcrição falha com `no_speech`, que o pipeline trata como fonte sem fala
- Test: integration — Groq falsa

#### Scenario: Chave Groq rejeitada
- **WHEN** a Groq responde 401 ou 403
- **THEN** a transcrição falha com `groq_api_key_rejected` e a frase que manda atualizar a chave
- Test: integration — Groq falsa

#### Scenario: Áudio acima de 25 MB
- **WHEN** o arquivo do trecho tem mais de 26 214 400 bytes
- **THEN** a transcrição falha com `audio_too_large` sem nenhuma requisição
- Test: integration — Groq falsa sem requisições

#### Scenario: Geração com chave OpenRouter
- **WHEN** a chave OpenRouter está salva e o modelo de notas é explícito
- **THEN** a nota vem de uma chamada `/chat/completions` à OpenRouter com o prompt `note_generate.md` e os rótulos de fonte limpos
- Test: integration — OpenRouter falsa

#### Scenario: Limpeza da transcrição da nota com chave OpenRouter
- **WHEN** a chave OpenRouter está salva e o pipeline limpa a transcrição com o estilo `note_transcript_cleanup`
- **THEN** a chamada vai à OpenRouter com o prompt `note_transcript_cleanup.md` e `temperature: 0`
- Test: integration — OpenRouter falsa

#### Scenario: Geração com Auto ou sem chave
- **WHEN** o modelo de notas salvo é `open-software/auto`, ou não há chave OpenRouter
- **THEN** com Auto o app troca, uma vez, pelo modelo padrão explícito e a geração vai direto à OpenRouter; sem chave a geração falha com `openrouter_api_key_missing` sem requisição
- Test: unit (Rust) — carga de settings com Auto; integration — sem chave, zero requisições

### Requirement: Falha clara sem chave nem servidor
Quando falta a chave do provedor de que a nota precisa, a nota SHALL dizer, na língua da interface (pt-BR ou en), qual chave falta e onde salvá-la (Configurações > Modelos > Mais opções > Chaves de API de provedores), sem tentar outra rota, e MUST NOT mostrar o erro cru da biblioteca HTTP. O áudio fica salvo e "Tentar de novo" continua disponível. Sem a chave OpenRouter, a limpeza da transcrição é pulada e a nota segue.

#### Scenario: Transcrição sem chave Groq e sem servidor
- **WHEN** não há chave Groq e uma gravação termina
- **THEN** a transcrição falha com `groq_api_key_missing` sem nenhuma requisição de rede, e a nota mostra traduzida a frase "Add your Groq API key in Settings > Models > More options > Provider API keys to transcribe recordings."
- Test: integration — servidor sentinela em porta efêmera, injetada como a URL legada, recebe zero requisições; unit — `note-failure-banner.test.tsx` traduz a frase em pt-BR

#### Scenario: Geração sem chave OpenRouter e sem servidor
- **WHEN** o motor de Notas é OpenRouter e não há chave OpenRouter
- **THEN** a geração falha com `openrouter_api_key_missing` e a nota mostra a frase de transcrição salva que pede a chave OpenRouter
- Test: integration — nenhuma requisição; unit — `processing.rs` mapeia o código para a frase; `note-failure-banner.test.tsx`

## REMOVED Requirements

### Requirement: Endpoint OpenAI-compatível como modelo do agente
**Reason**: O chat é só dos motores CLI, e o agente nativo sai com a fase 4 de `cli-substitui-agente-nativo`.
**Migration**: `llm_usage.chat` com `endpoint` ou `clovy` migra uma vez para o CLI padrão. Endpoints cadastrados continuam valendo para notas e atividade.

Nota de verificação (orquestrador, 2026-10-28): os testes automatizados usam porta efêmera para evitar conflitos locais. A prova nativa em `127.0.0.1:8080` é separada, executada somente com a porta livre e limitada aos fluxos autorizados, conforme a tarefa 7.2 e seu relatório. Isso não declara sucesso dos fluxos reais de provedores adiados para validação manual pós-merge pelo dono.
