# Spec Delta

## Purpose

Define de onde vem a lista de modelos de texto que o Clovy oferece nos seletores de modelo, quando ela é atualizada e o que o seletor informa sobre onde o modelo é usado.

## ADDED Requirements

### Requirement: Catálogo de texto do OpenRouter
A lista de modelos de texto SHALL vir de `GET https://openrouter.ai/api/v1/models`, com a chave OpenRouter do usuário quando salva. Todo modelo cuja saída inclui texto MUST aparecer selecionável com nome, contexto e preço, sem exigir suporte a ferramentas. Nenhuma outra fonte remota MUST ser consultada.

#### Scenario: Catálogo carregado
- **WHEN** o OpenRouter devolve três modelos de texto, um deles sem `tools` em `supported_parameters`
- **THEN** o seletor "Modelo de texto" mostra os três
- Test: unit (Rust) — servidor HTTP falso no lugar do OpenRouter; mapeamento de `context_length`, `pricing`, `supported_parameters` e `architecture`; component — os três selecionáveis

#### Scenario: Falha do catálogo
- **WHEN** o OpenRouter responde 500 ou não responde em 10 s
- **THEN** o seletor mostra o modelo salvo, um aviso de que a lista não carregou e a ação de tentar de novo
- Test: unit (Rust) — servidor falso com 500; component — aviso e nova tentativa

### Requirement: Recarga ao mudar a chave
Salvar ou remover a chave OpenRouter SHALL recarregar a lista de modelos de texto nos seletores abertos, sem reiniciar o app.

#### Scenario: Salvar a chave
- **WHEN** o usuário salva uma chave OpenRouter válida com as Configurações abertas
- **THEN** o seletor "Modelo de texto" carrega a lista de novo sem fechar as Configurações
- Test: component — `listProviderModels("generation")` chamado de novo após salvar e após remover

### Requirement: Escolha do modelo sem efeito no chat
Escolher um modelo de texto SHALL gravar só esse modelo e MUST NOT mudar o motor do chat nem o motor de nenhum outro uso.

#### Scenario: Chat no Claude Code
- **WHEN** o chat está no CLI `claude` e o usuário escolhe `anthropic/claude-sonnet-4.5` como modelo de texto
- **THEN** o chat continua no `claude` após salvar e após reiniciar
- Test: unit (Rust) — `set_provider_model` com `llm_usage.chat = cli:claude`

### Requirement: Uso do modelo informado no seletor
A descrição do seletor "Modelo de texto" SHALL dizer que o modelo gera as notas quando o motor de Notas é OpenRouter. Quando o motor de Notas é outro, a descrição MUST dizer qual motor gera as notas.

#### Scenario: Notas no Claude Code
- **WHEN** o motor de Notas é o CLI `claude`
- **THEN** a descrição informa que as notas são geradas pelo Claude Code e não por esse modelo
- Test: component — descrição para motor de Notas OpenRouter e CLI, em en e pt-BR
