# Proposal

## Why

O Clovy deve oferecer o mesmo alcance de suas ferramentas quando o usuário possui apenas Claude Code, apenas Codex ou apenas Antigravity, nas superfícies elegíveis. O app administra contexto, gateway, conexões e permissões. O motor escolhido executa as rotinas e os recursos de texto nos quais foi selecionado.

## What Changes

- Estabelecer um contrato comum para os três runtimes em rotinas, geração de notas e Atividade. Preservar contexto, catálogo autorizado do Clovy e dos conectores, política de aprovação, cancelamento e histórico de execuções. Clientes externos usam a autoridade definida em `retirar-chat-do-clovy`.
- Reutilizar o gateway host-owned existente como pool único de ferramentas, sem catálogo, credenciais ou regras de autorização paralelos por runtime. Cada adaptador entrega ao motor o vínculo do turno e normaliza os eventos reais.
- Garantir operação com qualquer um dos três instalado isoladamente, respeitando a escolha explícita quando há mais de um. A preferência automática existente só resolve a ausência de escolha; não pode substituir a decisão do usuário.
- Ao mudar o motor de uma execução futura, preservar a autoridade aplicável, impedir reuso de vínculos e decisões pendentes e manter o histórico existente.
- Completar a conexão MCP do Antigravity em diretório de execução privado pertencente ao Clovy, com perfil nativo próprio, vínculo por turno, nome único, timeout acima do broker e revogação no encerramento. HOME/XDG mudam somente no processo filho; não escrever configuração no workspace pessoal nem copiar credenciais.
- Definir e provar a política explícita do Antigravity para rotinas e geração pontual. Exigir prova de contenção além de `--sandbox` e preservar as restrições de autorização.
- Manter funcionamento com extensões pessoais do runtime ligadas: o Clovy administra o perfil e o diretório usados por suas execuções, sem carregar essas personalizações externas nem exigir sua desativação. Oferecer no app a entrada para a primeira autenticação nativa desse perfil; o login do terminal pessoal não prova autenticação nele. A recusa por extensão pessoal detectada é proteção transitória, não entrega da paridade.
- Qualificar os recursos de texto com schema e proteção contra autoingestão, distinguindo o runtime escolhido da leitura de suas transcrições. Atualizar documentação e aceites nos dois idiomas com prova nativa e matriz dos três motores.

## Capabilities

### New Capabilities

Nenhuma: a mudança completa capacidades existentes.

### Modified Capabilities

- `llm-providers`: operação isolada com qualquer um dos três motores elegíveis e gateway comum.
- `cli-engine-permissions`: mesma política host-owned para as ferramentas do Clovy, política explícita do Antigravity, rotinas e ciclo de vida da configuração temporária.

## Impact

Áreas existentes: `src-tauri/src/chat_engine/`, `llm/`, `providers/mod.rs`, `routines.rs`, ajustes de provedores e Rotinas, traduções e testes comportamentais próximos. Reutilizar `McpLaunch`, `TurnRegistry`, `TurnGrant`, broker, transporte nativo e histórico canônico. A retirada do chat interno preserva as provas de versões anteriores e encerra seus critérios de interface, alternância e reabertura.

A prova nativa com Antigravity CLI1.4.12 confirmou autenticação própria, MCP em workspace privado, retomada com novo servidor e comparação de hook global ligado/desligado no mesmo perfil autenticado. Isso sustenta separação de configuração, não contenção geral do SO nem aceite de builtins, Stop ou gateway integrado. O dono aprovou A1–A5 e a implementação permanente após essa prova; preservar seus limites e todos os aceites restantes.

Fora do escopo: substituir ASR Local/Groq, gerar áudio/imagens/vídeos diretamente com um CLI de texto, mudar a limpeza de ditado OpenRouter-only para todos os provedores, replicar ferramentas proprietárias internas de um CLI nos outros, alterar credenciais/perfis pessoais ou liberar ações sem autorização. Acesso a funções de mídia já oferecidas como ferramentas do Clovy faz parte do pool comum, com os mesmos pré-requisitos do provedor independentemente do motor. Windows exige prova no Windows, não compilação cruzada. A correção Stop/migração56 em qualificação é base separada, não será reimplementada aqui.
