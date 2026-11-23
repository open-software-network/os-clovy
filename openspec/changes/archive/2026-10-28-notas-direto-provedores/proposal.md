# Proposal

## Why

Na v0.1.2 publicada (Mac e Windows), toda gravação de reunião termina com a nota em falha: "Sistema: error sending request for url (http://127.0.0.1:8080/v1/notes/transcribe) | Microfone: error sending request for url (...)". O release não traz o Clovy API, e o endereço padrão do cliente é `127.0.0.1:8080`. O ditado funciona porque, pela ADR-0065, com a chave Groq salva ele chama a Groq direto, e a limpeza do ditado vai direto à OpenRouter. A transcrição de nota, a geração da nota e a limpeza da transcrição da nota têm um único caminho, que passa pelo servidor.

O dono decidiu: chamada direta, igual ao ditado. Não embutir o servidor no app.

## What Changes

- Transcrição de nota (final por turno, fallback por fonte e prévia ao vivo): com a chave Groq salva, o desktop chama `api.groq.com` direto, com os mesmos parâmetros que o Clovy API usa (modelo, padrão `whisper-large-v3-turbo`, idioma, prompt do dicionário, `response_format=json`, nome de arquivo canônico), uma nova tentativa para falhas transitórias, texto vazio como `no_speech`, 401/403 como chave rejeitada e limite de 25 MB do áudio antes do envio.
- Geração de nota com o motor de Notas = Clovy e um modelo explícito: com a chave OpenRouter salva, o desktop chama `openrouter.ai` direto com o mesmo prompt de sistema, a mesma mensagem do usuário e o mesmo pós-processamento do servidor.
- Limpeza da transcrição da nota (`note_transcript_cleanup`): com a chave OpenRouter salva, vai direto à OpenRouter com o prompt e a mensagem que o Clovy API usa para esse estilo, em `temperature: 0`.
- Sem a chave, cada chamada segue pelo Clovy API como antes. Se o servidor também não responde, a nota mostra, em pt-BR e en, que falta a chave Groq ou OpenRouter em Configurações > Modelos > Mais opções > Chaves de API de provedores, e o "Tentar de novo" continua.
- Motores Endpoint e CLI não mudam.

## Capabilities

### New Capabilities

Nenhuma.

### Modified Capabilities

- `llm-providers`: notas com chave própria vão direto ao provedor; sem chave e sem servidor, a falha diz qual chave falta.

## Impact

- `src-tauri/src/clovy_api.rs` (transcrição de nota, geração, limpeza, mapeamento de erro), testes em `src-tauri/src/clovy_api_notes_tests.rs`.
- `src-tauri/src/domain/processing.rs` (frase de falha da geração sem chave).
- `src/i18n/messages/notes.ts` e `src/components/note-editor/NoteFailureBanner.tsx` (frases novas traduzidas).
- `docs/adr/0070-notes-call-the-users-own-provider-directly.md` (estende a ADR-0065 §5), `docs/llm-providers.md`, `docs/configuration.md`, `AGENTS.md`.
- Clovy API não muda (ADR-0021): os contratos `/v1/notes/*` e `/v1/dictate/cleanup` seguem iguais para quem roda o servidor.
