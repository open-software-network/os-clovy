## ADDED Requirements

### Requirement: Notas com chave própria vão direto ao provedor
Com o motor de Notas = Clovy e a chave do provedor salva, o desktop SHALL mandar a transcrição de nota à Groq e a geração (modelo explícito, não Auto) e a limpeza da nota à OpenRouter, direto. A requisição direta MUST levar só a chave do provedor chamado e o mesmo contrato que o Clovy API manda. Sem a chave, a chamada MUST seguir pelo Clovy API como antes.

#### Scenario: Transcrição final com chave Groq
- **WHEN** a chave Groq está salva e um turno da reunião é transcrito
- **THEN** a requisição vai a `/audio/transcriptions` da Groq com modelo, `response_format=json`, idioma, prompt do dicionário e nome de arquivo canônico, e nenhuma requisição vai ao Clovy API
- Test: integration — `clovy_api_notes_tests.rs`, Groq e Clovy API falsos contam requisições e conferem o corpo multipart e os cabeçalhos

#### Scenario: Prévia ao vivo com chave Groq
- **WHEN** a chave Groq está salva e a prévia ao vivo envia um trecho
- **THEN** o trecho vai direto à Groq e nenhum campo de prévia do Clovy API (`preview`, `previewOptedIn`) vai junto
- Test: integration — `clovy_api_notes_tests.rs`, pedido com `preview: true` contra Groq falsa

#### Scenario: Falha transitória da Groq
- **WHEN** a Groq responde 503 na primeira tentativa e 200 na segunda
- **THEN** a transcrição devolve o texto da segunda resposta depois de exatamente duas requisições; um 400 não é repetido
- Test: integration — `clovy_api_notes_tests.rs`, Groq falsa com contador de chamadas

#### Scenario: Áudio sem fala
- **WHEN** a Groq devolve texto vazio
- **THEN** a transcrição falha com `no_speech`, que o pipeline trata como fonte sem fala
- Test: integration — `clovy_api_notes_tests.rs`

#### Scenario: Chave Groq rejeitada
- **WHEN** a Groq responde 401 ou 403
- **THEN** a transcrição falha com `groq_api_key_rejected` e a frase que manda atualizar a chave
- Test: integration — `clovy_api_notes_tests.rs`

#### Scenario: Áudio acima de 25 MB
- **WHEN** o arquivo do trecho tem mais de 26 214 400 bytes
- **THEN** a transcrição falha com `audio_too_large` sem nenhuma requisição
- Test: integration — `clovy_api_notes_tests.rs`, Groq falsa sem requisições

#### Scenario: Sem chave Groq
- **WHEN** não há chave Groq salva
- **THEN** a transcrição vai a `/v1/notes/transcribe` do Clovy API com o token local, como antes
- Test: integration — `clovy_api_notes_tests.rs`

#### Scenario: Geração com chave OpenRouter
- **WHEN** a chave OpenRouter está salva e o modelo de notas é explícito
- **THEN** a nota vem de uma chamada `/chat/completions` à OpenRouter com o prompt `note_generate.md`, a mensagem do usuário do servidor e os rótulos de fonte limpos, e nenhuma requisição vai ao Clovy API
- Test: integration — `clovy_api_notes_tests.rs`

#### Scenario: Geração com Auto ou sem chave
- **WHEN** o modelo é `open-software/auto` ou não há chave OpenRouter
- **THEN** a geração vai a `/v1/notes/generate` do Clovy API sem cabeçalho de chave
- Test: integration — `clovy_api_notes_tests.rs`

#### Scenario: Limpeza da transcrição da nota com chave OpenRouter
- **WHEN** a chave OpenRouter está salva e o pipeline limpa a transcrição com o estilo `note_transcript_cleanup`
- **THEN** a chamada vai à OpenRouter com o prompt `note_transcript_cleanup.md`, a mensagem do servidor para esse estilo e `temperature: 0`
- Test: integration — `clovy_api_notes_tests.rs`

### Requirement: Falha clara sem chave nem servidor
Quando não há a chave do provedor e o Clovy API não responde, a nota SHALL dizer, na língua da interface (pt-BR ou en), qual chave falta e onde salvá-la (Configurações > Modelos > Mais opções > Chaves de API de provedores), e MUST NOT mostrar o erro cru da biblioteca HTTP. O áudio fica salvo e "Tentar de novo" continua disponível.

#### Scenario: Transcrição sem chave Groq e sem servidor
- **WHEN** não há chave Groq e a conexão com o Clovy API é recusada
- **THEN** a transcrição falha com `groq_api_key_missing` e a frase "Add your Groq API key in Settings > Models > More options > Provider API keys to transcribe recordings.", que a nota mostra traduzida
- Test: integration — `clovy_api_notes_tests.rs` com porta fechada; unit — `note-failure-banner.test.tsx` traduz a frase em pt-BR

#### Scenario: Geração sem chave OpenRouter e sem servidor
- **WHEN** não há chave OpenRouter e a conexão com o Clovy API é recusada
- **THEN** a geração falha com `openrouter_api_key_missing` e a nota mostra a frase de transcrição salva que pede a chave OpenRouter
- Test: integration — `clovy_api_notes_tests.rs`; unit — `processing.rs` mapeia o código para a frase; `note-failure-banner.test.tsx`
