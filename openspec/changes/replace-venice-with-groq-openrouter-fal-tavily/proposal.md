# Proposal

## Why

O dono determinou a retirada completa da dependência de Venice, preservando todas as capacidades por provedores substitutos. No dev real, Fn já abre o componente e produz áudio, mas a transcrição é recusada com `clovy_request_failed / model_not_priced`: a API local carregou zero modelos após filtrar provedores sem chaves configuradas.

## What Changes

- **BREAKING**: remover implementações, configuração, seleção, IPC/DTOs, credenciais/header, aliases, catálogos, branding e fallbacks ativos de Venice; migrar todos os consumidores, sem shims. Histórico e ADRs aceitos continuam históricos, não dependências ativas.
- Usar Groq/Whisper para o áudio do ditado e das notas; padrão multilíngue `whisper-large-v3-turbo`, sem fallback para Venice. Preservar gravação, idioma, contexto/dicionário, chunks, preview, transcrição e inserção existentes.
- Usar OpenRouter para texto remoto: chat via API, geração de notas, limpeza do ditado e consumidores de inferência textual; preservar motores CLI/endpoints já existentes e a escolha Claude do chat do dono.
- Usar fal para geração/edição de imagens e geração/animação de vídeos, com modelos/endpoints reais, filas e resultados finais, sem botões/API quebrados ou implementações simuladas.
- Usar Tavily para busca web e extração de páginas, preservando os resultados consumidos pelas ferramentas existentes.
- Migrar seleções legadas e overrides para modelos válidos dos novos catálogos, sem reescrever notas, conversas, transcrições ou histórico de providers. Chaves Venice não se tornam chaves dos novos serviços.
- Declarar configuração necessária, preços/capacidades/privacidade reais e erros úteis quando faltar chave/modelo; não afirmar TEE/zero retention dos antigos serviços sem suporte dos novos.

## Capabilities

### New Capabilities

- `audio-transcription`: transcrição de áudio de ditado/notas por Groq com os contratos locais existentes.
- `remote-text-inference`: inferência de texto OpenRouter, catálogo/capacidades/preço e continuidade das rotas CLI/endpoints.
- `generated-media`: geração/edição de imagem e geração/animação de vídeo por fal, até o asset utilizável.
- `web-augmentation`: busca e extração Tavily com resultados, erros e limites das ferramentas existentes.
- `inference-provider-settings`: seleção, configuração e migração limpa dos provedores sem dependência ativa de Venice.

### Modified Capabilities

Nenhuma: `openspec list --specs` não encontrou specs principais. As changes anteriores de captura/MCP e permissões dev continuam abertas e seus critérios não são reduzidos por este corte.

## Impact

- `clovy-api`: domínio/configuração, adapters, bootstrap, catálogo/pricing/metering e handlers/services de áudio, texto, imagem, vídeo e web; exemplos de ambiente e contratos HTTP/testes.
- Desktop Rust: `providers`, `clovy_api`, ditado, processamento de notas, registro LLM, comandos/DTOs e seleção por perfil; helper de áudio perde o rótulo legado `venice_recording`, sem alteração indevida de permissões.
- Frontend: settings, pickers/catálogos, funding/privacy/branding, chat/agente/notas e callsites companion; atualizar todos os consumidores dos símbolos exportados.
- Documentação vigente e recipes de comprovação; ADRs aceitos recebem addenda, não reescrita histórica.
- Serviços externos escolhidos pelo dono: [Groq](https://console.groq.com/docs/speech-to-text), [OpenRouter](https://openrouter.ai/docs/api_reference/overview), [fal](https://fal.ai/docs/documentation/model-apis/inference/queue) e [Tavily](https://docs.tavily.com/documentation/api-reference/introduction). Provas externas reais exigem chaves configuradas localmente e ação/autorização de custo do dono; contrato em sandbox não equivale a execução paga nem a aceite completo.
- O panic observado da captura por comparação sem ordem total é uma correção independente necessária ao fluxo real; não será mascarado nem atribuído a uma falha de TCC.
