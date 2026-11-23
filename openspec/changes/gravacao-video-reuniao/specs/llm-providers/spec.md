# Spec Delta

## MODIFIED Requirements

### Requirement: Execução isolada de CLIs em geração pontual
Chamadas de geração pontual a CLIs SHALL rodar sem persistir sessão no CLI, sem ferramentas e sem carregar arquivos de contexto/skills, com tempo limite, e MUST marcar o prompt com o marcador de autoria do Clovy para que a ingestão de sessões de agentes as ignore. CLIs que não garantem todas as ferramentas desligadas (hoje `codex`, `agy`, `cursor-agent`, `copilot`) MUST ser recusados para notas, limpeza de ditado e atividade, sem redirecionamento silencioso; continuam disponíveis como motor do chat. Chamadas em segundo plano dos recursos de atividade MUST ser serializadas (uma por vez). Única exceção: a geração de nota de uma sessão com vídeo no `claude` MAY liberar só a execução do script da skill `video-analysis` sobre o vídeo daquela sessão e a leitura do vídeo e da pasta de evidências da chamada, sem MCP, sem outras ferramentas e sem persistir sessão, com tempo limite próprio.

#### Scenario: CLI que mantém ferramentas
- **WHEN** o usuário tenta escolher `codex` para notas
- **THEN** a escolha é recusada com a explicação de que as ferramentas desse CLI não podem ser desligadas, e nenhum processo do `codex` é iniciado
- Test: unit — seleção por uso e invocação recusadas com CLI falso instalado

#### Scenario: Marcador de autoria presente
- **WHEN** o Clovy executa qualquer chamada de geração pontual por CLI
- **THEN** o prompt enviado contém o marcador de autoria do Clovy
- Test: unit — montagem do comando/prompt por provedor

#### Scenario: Tempo limite
- **WHEN** um CLI não responde dentro do tempo limite configurado
- **THEN** o processo é encerrado e a chamada falha com erro de tempo esgotado, sem processo órfão
- Test: integration — CLI falso que dorme além do limite

#### Scenario: Exceção do vídeo restrita
- **WHEN** o Clovy gera a nota de uma sessão com vídeo no `claude`
- **THEN** a invocação libera só `Bash` para o script da skill com o caminho do vídeo da sessão e `Read` restrito ao vídeo e à pasta de evidências, mantém MCP desligado e sem persistência, e nenhuma outra chamada pontual recebe ferramentas
- Test: unit — argumentos montados para nota com vídeo e para nota sem vídeo
