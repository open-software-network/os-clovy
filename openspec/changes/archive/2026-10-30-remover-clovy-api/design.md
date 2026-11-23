# Design

## Context

- Todo tráfego ao servidor sai de `src-tauri/src/clovy_api.rs`, por meio de `clovy_api_url()` (~3846), `authed_send` (~3545), `post_json` e `get_json`, com o token de `local_identity.rs`. Os caminhos diretos já existem para Groq (transcrição e ditado, ADR-0065) e OpenRouter (notas e limpeza, ADR-0070), mas caem no servidor quando falta a chave.
- Sem caminho direto hoje:
  - catálogo de modelos (`list_models` ~1528);
  - chat do agente nativo (`proxy_agent_chat_completions` ~1993);
  - imagem e vídeo (`generate_image` ~1705, `edit_image` ~1726, `video_generate` ~1753, `video_status` ~1775);
  - web do agente nativo (`forward_web_request` ~2323);
  - compartilhamento (`share_*` ~1555-1585);
  - classificador de imagem (~2768);
  - títulos (~2712, comando não registrado).
- Acoplamento de build: `include_str!("../../clovy-api/crates/services/src/prompts/...")` em `clovy_api.rs` ~93-101 e nos testes.
- A change `cli-substitui-agente-nativo` (fases 1 a 3 feitas, falta a fase 4) já remove o sidecar, o motor nativo, o motor endpoint no chat e o efeito de `set_provider_model` no chat. Esta change parte do resultado dela.
- Mapa completo de usos (22 itens, com `arquivo:linha`): resultado do scout de 2026-10-28, reproduzido nas tarefas abaixo.

## Goals / Non-Goals

**Goals:**
- Zero requisições ao Clovy API em qualquer caminho, provado por teste com servidor falso em `127.0.0.1:8080` e por varredura no `make verify`.
- Mesmo comportamento de hoje para quem tem as chaves salvas.

**Non-Goals:**
- Substituir o compartilhamento privado por outro servidor.
- Busca web própria (os CLIs têm a deles).
- Novos provedores além de Groq, OpenRouter, fal e endpoints cadastrados.
- Mudar o servidor MCP `clovy`, exceto as ferramentas de mídia.

## Decisions

1. **Ordem:** primeiro a fase 4 de `cli-substitui-agente-nativo` (tarefas 3.2 e 4.1 a 4.7, com a 4.6 trocada por esta change), depois esta change, no mesmo branch. A fase 4 apaga o maior consumidor (agente nativo), e o que sobra em `clovy_api.rs` fica pequeno.
2. **Desmontar `clovy_api.rs` em módulos de provedor** (por exemplo `providers/groq.rs`, `providers/openrouter.rs`, `providers/fal.rs` e a rota de notas por CLI ou endpoint), mantendo os tipos ainda usados (`GenerationRequest`, `TranscriptionRequest`, DTOs de mídia) e apagando o resto. Alternativa descartada: manter o arquivo e só apagar os ramos remotos, porque o nome continuaria sugerindo um servidor.
3. **Sem rota de reserva.** Toda função `*_via` perde o ramo do servidor. Sem chave, devolve o código de chave ausente já existente (`groq_api_key_missing`, `openrouter_api_key_missing`) antes de qualquer requisição. O ditado ganha a mesma frase no HUD. A prévia ao vivo liga só com a chave Groq (`capture.rs` ~381 troca `configured()` por checagem de chave).
4. **Valor `clovy` do `ProviderRef`:** para notas vira `OpenRouter` (novo nome, mesmo comportamento direto). A limpeza de ditado fica fixa na OpenRouter, e o registro não guarda mais escolha para ela. Para o chat, migra uma vez ao CLI padrão (regra da fase 4). A migração de `provider-settings.json` é de uma vez, com regravação.
5. **Auto sai.** Modelo salvo `open-software/auto` vira `DEFAULT_GENERATION_MODEL` (`z-ai/glm-5.2`). Saem o controle de custo e qualidade e as entradas Auto dos seletores TS.
6. **Catálogo de texto:** `list_provider_models(Generation)` chama só `GET https://openrouter.ai/api/v1/models` (Bearer quando há chave), com tempo limite de 10 s e cache em memória de 10 min, invalidado quando a chave muda. O mapeamento vai para `ProviderModelDto`:

   | Campo do OpenRouter | Campo do `ProviderModelDto` |
   |---|---|
   | `id`, `name`, `description` | os mesmos |
   | `context_length` | `contextTokens` |
   | `pricing` | preço |
   | `supported_parameters` | capacidades |
   | `architecture.input_modalities` | visão |

   Só entram modelos com `text` em `architecture.output_modalities`. O filtro `modelAvailableForMode` deixa de exigir ferramentas no modo texto. Privacidade e `serviceAvailable` ficam vazios. Os catálogos de transcrição, imagem e vídeo vêm das listas curadas já existentes (`KNOWN_*_MODELS`, `IMAGE_MODELS`, `VIDEO_MODELS`).
7. **`set_provider_model(Generation)`** grava só `generation_model`/`remote_generation_model`, sem tocar `llm_usage`.
8. **fal direto:** cliente da fila da fal (envio, consulta de estado, resultado, download) com a chave fal do Keychain, tempo limite por tipo (imagem 2 min, vídeo 15 min), mesmo formato de artefato. As regras que o servidor fazia e que o desktop precisa, como a lista de modelos permitidos e o `requestId` sem reenvio após o envio, são portadas do código de `clovy-api/` antes de apagá-lo. A triagem de conteúdo explícito por palavras (`may_request_explicit_content`) roda antes do envio.
9. **Prompts:** `note_generate.md` e `note_transcript_cleanup.md` são copiados para `src-tauri/src/prompts/` antes de apagar `clovy-api/`.
10. **Varredura no `make verify`:** `scripts/check-org-references.mjs` (ou script irmão) passa a falhar com `CLOVY_API_URL`, `clovy_api_url` e as rotas `/v1/...` listadas no spec, fora de `docs/adr/` e `openspec/changes/archive/`.
11. **ADR nova** (próximo número livre após as ADRs das outras changes em curso) supera a ADR-0021 e registra o fim das rotas de reserva das ADRs 0065 e 0070, da mídia via servidor (0008, 0013, 0015) e do fallback nativo (0062). As regras de fronteira do Clovy API em `AGENTS.md` e `CLAUDE.md` são trocadas pela regra "o app não chama servidor do Clovy".

## Risks / Trade-offs

- [Quem não tem chave Groq perde transcrição e ditado] → frase clara e caminho para salvar a chave; é o pedido do dono.
- [Compartilhamento privado deixa de existir] → saem os pontos de entrada e o diálogo; links antigos não funcionavam no release.
- [fal direto reimplementa regras do servidor] → portar com testes de fal falsa antes de apagar `clovy-api/`.
- [Change grande e com muita deleção] → fase 4 e esta change em commits separados por grupo de tarefas, gates entre grupos, revisão do par no fim.
- [Conflito com a change de vídeo, que mexe na rota de notas por CLI] → quem integrar depois faz rebase; a rota CLI é mantida nesta change.
