# Proposal

## Why

Desde a v0.1.3 a transcrição de nota vai direto à Groq com a chave do usuário (ADR-0070). Depois de uma reunião longa a nota falha com "Sistema: Groq reports that its usage limit was reached. ... | Microfone: Groq reports that its usage limit was reached. ...", em inglês com a interface em pt-BR.

O tamanho do áudio não é o problema: cada turno tem no máximo 30 s (cerca de 0,96 MB, longe dos 25 MB da Groq). O problema é o limite de taxa, e os 30 s pioram: muitas requisições pequenas, cada uma cobrada como no mínimo 10 s. No plano gratuito do `whisper-large-v3-turbo` a Groq permite 20 requisições por minuto, 2.000 por dia, 7.200 s de áudio por hora e 28.800 s por dia, e cobra no mínimo 10 s por requisição. Uma reunião de 1 h gera centenas de turnos nas duas faixas, enviados com concorrência 2 e sem ritmo, então o limite por minuto estoura em segundos.

Causa raiz no código:

- `src-tauri/src/clovy_api.rs:516-554` (`send_direct`): um 429 tem uma única nova tentativa depois de 300 ms fixos (`DIRECT_PROVIDER_RETRY_BACKOFF`, `:388`), sem ler `retry-after`; depois vira `groq_usage_limit_exceeded` (`:917`, frase em `:3466`), sem o tempo de espera.
- `src-tauri/src/domain/processing.rs:2731-2744` (`is_retryable_transcription_error`): esse código não é transitório, então o turno falha na hora.
- `processing.rs:3423-3449` (`transcribe_one_turn_job`): a falha vira `Failure` do turno e o stream (`:2914`) segue para os próximos turnos, que também levam 429; nada pausa as duas faixas.
- `processing.rs:3120` (`transcribe_source_fallbacks`): com a cobertura baixa, o fallback reenvia a faixa inteira, ainda dentro do limite.
- `processing.rs:1749-1756`: qualquer turno com falha bloqueia a nota (`transcription_partially_failed`).
- `src/components/note-editor/NoteFailureBanner.tsx:27`: a frase de limite não está em `STORED_FAILURE_KEYS`, então aparece em inglês.
- `src-tauri/src/providers/mod.rs:1595` e `audio/live_preview.rs:619`: a prévia ao vivo vem ligada e manda cada trecho de 8 s com fala das duas faixas à mesma conta, cobrado como 10 s. Numa reunião de 1 h ela gasta ~4.050 s do limite de 7.200 s por hora antes de a transcrição final começar.

O que já funciona: os jobs duráveis por turno (`db/repositories.rs:4031`) guardam cada turno pronto, e "Tentar de novo" reabre só os que falharam.

## What Changes

- 429 da Groq: ler `retry-after` (e `x-ratelimit-reset-requests` quando falta) e devolver a espera no erro. A transcrição da nota espera e repete a mesma requisição, pausando as duas faixas juntas.
- Espera longa (limite por hora ou por dia): a nota para com o progresso salvo, mostra "Limite de uso da Groq atingido. O Clovy continua a transcrição às HH:MM" e retoma sozinha nessa hora se o app estiver aberto. Os turnos que faltam ficam pendentes; nenhum turno pronto é reenviado e o fallback da faixa inteira não roda.
- Ritmo: no máximo 18 requisições por minuto e 7.200 s de áudio cobrado por hora, somando as duas faixas, todas as notas e a prévia ao vivo; a concorrência continua 2.
- A prévia ao vivo (ligada por padrão) gasta a mesma conta Groq: ela nunca espera e usa no máximo metade do áudio da hora, para a transcrição final não começar já estourada.
- Menos requisições e menos segundos: com a chave Groq, os turnos de fala seguidos de cada faixa vão em pacotes de até 600 s (19,2 MB em WAV, abaixo dos 25 MB), sem o silêncio entre eles, e voltam por turno pelos tempos de segmento. WAV maior que 600 s é cortado em pedaços de 600 s com 10 s de sobreposição, como no exemplo da Groq. Uma sessão com turnos já transcritos no layout antigo de 30 s mantém esse layout, para não reenviar nada.
- A frase do limite e a de pausa aparecem em pt-BR e en.

## Capabilities

### New Capabilities

Nenhuma.

### Modified Capabilities

- `llm-providers`: limite de taxa da Groq na transcrição de nota direta.

## Impact

- `src-tauri/src/clovy_api.rs`, `src-tauri/src/clovy_api_notes_tests.rs`, `src-tauri/src/llm/endpoint.rs` (servidor falso com cabeçalhos).
- `src-tauri/src/domain/processing.rs` (ritmo, espera, pausa, pacotes, layout dos turnos), `src-tauri/src/audio/turns.rs` (teto e corte com sobreposição), `src-tauri/src/audio/live_preview.rs` (prévia no pacer), `src-tauri/src/commands.rs` (retomada automática).
- `src/i18n/messages/notes.ts`, `src/components/note-editor/NoteFailureBanner.tsx`, testes em `src/test/`.
- Docs: `docs/llm-providers.md`, adendo datado da ADR-0070, `docs/audio-pipeline.md`.
- Clovy API não muda.
