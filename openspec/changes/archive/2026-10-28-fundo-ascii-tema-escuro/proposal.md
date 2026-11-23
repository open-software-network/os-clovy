# Proposal

## Why

A Nova sessão e o Início pintam uma lavagem na cor da marca (o brilho azul/verde-água que sobe do rodapé) atrás do conteúdo, e o dono quer o fundo neutro. Ele também quer que o app abra em modo escuro e poder escolher o fundo dessas telas, começando pelo fundo animado Drift do asciify.org com os controles que ele selecionou.

## What Changes

- Remove a lavagem colorida da Nova sessão, do Início, do véu que esconde a borda dessa lavagem atrás do composer e do cartão da etapa de personalidade do onboarding. Os tokens `--hero-wash*` saem se ficarem sem uso.
- Sem preferência de tema salva, o app abre em modo escuro (antes resolvia pelo sistema). Preferência salva (claro, escuro ou sistema) continua valendo.
- Nova linha "Fundo" em Configurações → Aparência com três opções: "Drift" (padrão), "Imagem" e "Nenhum".
- "Drift" desenha o preset `drift` do `asciify-engine/kit`, com os controles escolhidos pelo dono, atrás do conteúdo da Nova sessão e do Início. Texto e controles continuam HTML acessível acima do desenho.
- "Imagem" permite escolher PNG, JPEG ou WebP local de até 20 MB, copiado para o diretório do app, com véu do tema, remoção e fallback liso se indisponível. Nenhuma imagem é enviada.
- Dependência nova: `asciify-engine`, numa versão fixa que passa na regra de 7 dias de idade mínima do pnpm do projeto.

## Capabilities

### New Capabilities

- `app-appearance`: tema inicial do app e fundo das telas de conversa (Nova sessão e Início).

### Modified Capabilities

Nenhuma.

## Impact

- CSS: `src/styles/app.css` (regras `.agent-workspace::before`, `[data-home]::before`, camadas de marca do véu do composer, cartão do onboarding), `src/styles/tokens.css`, `src/styleguide/sections/Color.tsx`.
- Tema: `src/lib/theme.ts` e os scripts antes do primeiro paint em `index.html`, `styleguide.html` e `onboarding-preview.html`.
- Fundo: módulo de preferência novo em `src/lib/`, componente novo montado em `src/components/agent/AgentWorkspace.tsx`.
- Configurações: `src/components/settings/AppSettings.tsx` (aba Aparência), `src/i18n/messages/settings.ts` (en e pt-BR).
- Dependências: `package.json`, `pnpm-lock.yaml`, talvez `pnpm-workspace.yaml` (`allowBuilds`).
- Testes: `src/test/home-polish-css.test.ts` e `src/test/technical-identity.test.mjs` mudam; testes novos para a preferência e o ciclo de vida do fundo.
- Janelas HUD (`hud.html`, `agent-hud.html`, `meeting-hud.html`) já são escuras fixas e não mudam.

- Extensão de escopo aprovada em 2026-10-28: comandos nativos em `src-tauri/src/background_image.rs`, registro em `src-tauri/src/lib.rs`, validação de decodificação em `src-tauri/Cargo.toml`/`Cargo.lock`, controle `src/components/settings/BackgroundSettings.tsx` e seus testes.
