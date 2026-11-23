# Design

## Context

O desktop fala com três hosts para notas: Clovy API (`/v1/notes/transcribe`, `/v1/notes/generate`, `/v1/dictate/cleanup` com o estilo `note_transcript_cleanup`), e, só pelo servidor, Groq e OpenRouter. O ditado já tem a rota direta (ADR-0065) com um transporte injetável (`DictationTransport`) que os testes apontam para servidores falsos. As três chamadas de transcrição de nota (turno final em `domain/processing.rs`, fallback por fonte e prévia em `audio/live_preview.rs`) passam por `clovy_api::transcribe_saved_audio`, então há um ponto de troca só.

## Goals / Non-Goals

**Goals:**

- Nota completa sem servidor quando as chaves Groq e OpenRouter estão salvas.
- Mesmo contrato que o servidor manda ao provedor, para o resultado não mudar com a rota.
- Falha clara e traduzida quando não há chave nem servidor.

**Non-Goals:**

- Embutir o Clovy API no app.
- Mudar Clovy API ou os contratos v1.
- Mudar os motores Endpoint e CLI.
- Rota direta para `open-software/auto`: Auto é uma escolha que o servidor resolve; com Auto a geração segue no servidor.
- Chat, mídia e web.

## Decisions

### D1. Transporte compartilhado com o ditado

Reaproveitar `DictationTransport` (cliente do Clovy API, cliente direto, bases de Clovy, Groq e OpenRouter, tokens) como o transporte das notas. A produção usa um cliente direto próprio com o prazo de 600 s do Clovy API, porque um trecho de nota pode ter até 25 MB e o cliente do ditado tem prazo de 60 s. Funções novas `*_via(transport, chave, ...)` recebem a chave lida pelo chamador, como no ditado; os testes chamam as `*_via` com servidores falsos.

Alternativa: um transporte novo só para notas. Rejeitada: duplica a estrutura e os testes do ditado.

### D2. Transcrição direta igual à do servidor

Com a chave Groq: `POST {groq}/audio/transcriptions` com `model`, `response_format=json`, `file` com nome canônico (`audio.wav` ou `audio.m4a`, nunca o nome do arquivo local), `prompt` (dicionário) e `language` quando presentes. O ditado usa `verbose_json` e descarta segmentos de silêncio (ADR-0065, adendo); a nota mantém `json`, como o servidor, porque uma reunião longa não deve perder segmentos que o Whisper marcou como prováveis silêncios.

- Antes de enviar, áudio acima de 26 214 400 bytes (o `max_audio_bytes` do servidor) falha com `audio_too_large`, sem chamada.
- Texto vazio vira `AppError("no_speech", "no_speech")`, que o pipeline já trata (`is_no_speech_error`).
- Duas tentativas no total, com 300 ms entre elas, para 408, 429, 5xx e erro de transporte que não seja prazo, como `clovy-providers::retry`.
- 401/403 viram `groq_api_key_rejected`; 402, 404 e 429 (depois da nova tentativa) seguem `direct_provider_error`; 413 vira `audio_too_large`.
- A frase genérica de falha do provedor deixa de citar "dictation" quando a chamada é de nota.

### D3. Geração direta igual à do servidor

Com motor de Notas = Clovy, modelo diferente de Auto e chave OpenRouter: `POST {openrouter}/chat/completions` sem streaming, com as mensagens `[sistema: política de conteúdo local, sistema: note_generate.md, usuário: local_note_user_message]`, sem `temperature`, como o `OpenRouterGenerator`. O resultado passa por `local_note_result` (limpeza dos rótulos de fonte e título sugerido), com `provider = "openrouter"`. Mesma política de nova tentativa de D2. Resposta sem texto vira erro do provedor, que o pipeline troca pela frase de falha da geração.

### D4. Limpeza direta da transcrição da nota

`cleanup_text` com motor Clovy passa a usar `dictate_cleanup_via` com a chave OpenRouter e o modelo padrão de limpeza. Quando o estilo é `note_transcript_cleanup`, o corpo direto usa o prompt `note_transcript_cleanup.md` do Clovy API e a mensagem do servidor para esse estilo (`<dictionary_context>`, `<asr_transcript>`, `<output_contract>`); os outros estilos seguem com o contrato do ditado. A limpeza continua opcional no pipeline: falha ou texto vazio mantém a transcrição bruta. A limpeza da nota tem a mesma nova tentativa de D2 (408, 429, 5xx e erro de transporte que não seja prazo; nunca um 4xx permanente), como o `OpenRouterChat::complete` do servidor. A limpeza do ditado continua com uma tentativa só: é opcional, sua falha cola o transcrito bruto na hora, e repetir seguraria a colagem por mais uma ida e volta, contra a meta de latência da ADR-0065.

### D5. Falha sem chave e sem servidor

Quando não há chave e a conexão com o Clovy API falha (`reqwest::Error::is_connect`), a transcrição devolve `groq_api_key_missing` com a frase "Add your Groq API key in Settings > Models > More options > Provider API keys to transcribe recordings." e a geração devolve `openrouter_api_key_missing`, que `note_generation_failure_for_user` troca por "Your recording was transcribed, but Clovy needs your OpenRouter API key to write the note. Add it in Settings > Models > More options > Provider API keys. Your transcript is saved." As frases são as chaves `notes.failure.*` em inglês, traduzidas por texto exato no `NoteFailureBanner`, como as outras. O "Tentar de novo" não muda: o áudio fica salvo.

O caminho no app é Configurações > Modelos > Mais opções > Chaves de API de provedores (`settings.providerKeys.title` na aba Modelos); o pedido falava em "Provedores", mas a frase aponta para o rótulo que existe.

## Risks / Trade-offs

- [O servidor deixa de ver notas com chave própria] → Sem cobrança nesse caso; logs e métricas do servidor contam menos, como no ditado.
- [Prompt do servidor muda e o desktop não] → Os prompts de notas são `include_str!` do próprio `clovy-api/`, então mudam juntos; a mensagem da limpeza de nota é uma cópia, coberta por teste de formato.
- [Auto sem servidor falha] → Auto continua no servidor; sem servidor a nota mostra a frase genérica de geração. Escolher um modelo explícito resolve.
- [Trecho acima de 25 MB] → O pipeline já corta em trechos; o limite só evita enviar o que a Groq recusaria.
