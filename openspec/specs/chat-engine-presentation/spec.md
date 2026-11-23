# chat-engine-presentation Specification

## Purpose
Define o que a interface mostra sobre o motor do chat e como runtimes e provedores são identificados visualmente.

## Requirements

### Requirement: Sem rodapé de privacidade no início do chat
O início de uma sessão nova SHALL NOT mostrar o rodapé "O Clovy roda localmente…" (nem as variantes de privacidade) abaixo das sugestões.

#### Scenario: Sessão nova vazia
- **WHEN** o usuário abre uma sessão nova sem mensagens
- **THEN** as sugestões aparecem e nenhum texto de privacidade ou de modelo aparece abaixo delas

Test: unit — render do `AgentWorkspace` em modo hero sem `.agent-hero-footnote`.

### Requirement: Sem aviso de motor no composer
O composer e o seletor de motor SHALL NOT mostrar o aviso de motor ("{CLI} executa neste Mac com as ferramentas do Clovy", "Nenhuma ferramenta do Clovy nesta sessão…" ou "… não pode receber o servidor MCP do Clovy").

#### Scenario: Chat em um CLI
- **WHEN** o motor do chat é Claude Code, Codex ou outro CLI, com ou sem o servidor MCP ligado
- **THEN** nenhum aviso de motor aparece no composer nem no menu do seletor

Test: unit — `chat-engine-picker` e composer sem `.agent-composer-notice` nos três estados de ferramentas.

### Requirement: Ícone de marca dos runtimes e provedores
Todo lugar da interface que identifica um runtime de agente ou provedor (seletor de motor do chat, Configurações de provedores, servidor MCP, fontes de agentes de código, faixa de agentes de código, importação de projetos do Claude Code, cartões que citam o runtime, seletores de modelo) SHALL mostrar o ícone de marca dele em SVG, junto do nome. Marcas sem SVG de licença aberta MUST cair num monograma. Os SVGs MUST vir de fonte com licença registrada.

#### Scenario: Seletor de motor
- **WHEN** o usuário abre o seletor de motor do chat
- **THEN** Claude Code, Codex e os demais CLIs aparecem cada um com o ícone da sua marca

#### Scenario: Marca sem SVG
- **WHEN** um runtime não tem SVG disponível
- **THEN** aparece o monograma de duas letras, sem quebrar o alinhamento

Test: unit — componente de logo resolve cada id de CLI (`claude`, `codex`, `copilot`, `cursor-agent`, `agy`, `pi`) e provedor para o SVG ou monograma esperado; render do seletor de motor com o ícone por item.
