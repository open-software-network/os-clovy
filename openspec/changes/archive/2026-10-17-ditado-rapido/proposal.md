# Proposal

## Why

O ditado do Clovy é lento comparado ao liv-light, que usa o mesmo provedor (Groq Whisper). Medido no `dictation-events.log` do perfil `-dev`: apertar fn → começar a gravar leva 510-760 ms em toda tomada; soltar → colar leva 2,9-5,3 s para 14-25 s de áudio. No liv (`history.json`, 1.117 tomadas), 4 s de áudio colam em 0,4 s e 54 s em 1,7 s, e a gravação começa no ato. As causas estão no Clovy, não no modelo: a limpeza do texto roda num CLI de agente (Claude Code) por tomada e não pode ser desligada; o som de início bloqueia o microfone por 336 ms; a transcrição passa pela Clovy API mesmo com chave própria do Groq; uma análise de fala da Apple roda antes do envio quando o volume é baixo; e há uma espera fixa de 180 ms antes de colar. O dono decidiu em 2026-10-17: limpeza nunca em CLI, desligável, e quando ligada só por modelos da OpenRouter em chamada simples; ditado tão ágil quanto o liv.

## What Changes

- Novo controle "Limpar texto do ditado" em Configurações, **desligado por padrão**. Desligado, o transcrito do Whisper é colado direto, sem nenhuma chamada de LLM.
- **BREAKING** A limpeza do ditado não aceita mais motor CLI nem endpoint local: quando ligada, usa só um modelo da OpenRouter numa chamada única de chat completions. Escolhas antigas de CLI/endpoint para a limpeza viram "desligada" numa migração única.
- Transcrição e limpeza vão direto ao Groq/OpenRouter quando o usuário tem a própria chave salva (BYOK), como no liv, com conexão HTTP reaproveitada e aquecida; sem chave própria, seguem pela Clovy API como hoje.
- A gravação começa no aperto do atalho: o som de início deixa de atrasar a captura e não entra no transcrito.
- Microfone, som de início, token e conexão HTTP ficam prontos desde a abertura do app, para o primeiro aperto não pagar inicialização.
- A análise de fala da Apple deixa de rodar antes do envio; só decide "nenhuma fala" depois que o Whisper devolve texto vazio.
- Sem espera fixa antes de colar quando o app de destino já está ativo.
- Regra do `AGENTS.md` sobre chaves de provedor e ADR nova: chave BYOK do usuário pode ir direto ao provedor dela a partir do desktop; chaves de serviço continuam só na Clovy API.

## Capabilities

### New Capabilities

- `dictation`: latência de início e de entrega do ditado, limpeza opcional e onde ela roda, rota da transcrição.

### Modified Capabilities

(nenhuma: não há spec canônica de ditado em `openspec/specs/`.)

## Non-goals

- Transcrição local (Whisper on-device), streaming durante a gravação ou envio por pedaços.
- Mudar o formato do áudio (AAC/m4a continua; é menor que o WAV do liv).
- Mudar rotas ou contratos da Clovy API (ADR-0021).
- Limpeza de notas de reunião (`LlmUsage::Notes`), que continua como está.

## Impact

- Swift: `src-tauri/native/mac-dictation-helper/main.swift` (início de captura, som, aquecimento, análise de fala, colagem).
- Rust: `src-tauri/src/dictation.rs`, `src-tauri/src/llm/registry.rs` (`DictationCleanup`), `src-tauri/src/clovy_api.rs` (transcrição/limpeza diretas com BYOK, cliente HTTP aquecido), migração de configurações.
- Frontend: seção de ditado em Configurações (controle de limpeza e escolha do modelo OpenRouter), i18n.
- Docs: ADR nova, `AGENTS.md` (Boundaries), `docs/llm-providers.md`, doc de ditado.
