## MODIFIED Requirements

### Requirement: Catálogo de provedores CLI detectados
O sistema SHALL reconhecer os CLIs `claude`, `codex`, `pi`, `agy`, `cursor-agent` e `copilot`, resolvendo-os pelo PATH do shell de login do usuário (não apenas pelo PATH herdado pelo app gráfico) e exibindo, para cada um, se está instalado, o caminho resolvido e a versão quando o CLI a informar. O ambiente do perfil do usuário (por exemplo `HOME` real e variáveis como `PI_CODING_AGENT_DIR`) MUST ser preservado nas execuções. No Windows, que não tem shell de login, o sistema MUST resolver os CLIs pelo `Path` do ambiente do app, lido sem diferenciar maiúsculas de minúsculas, mais os diretórios onde os instaladores oficiais desses CLIs gravam; MUST aceitar só arquivos com extensão listada em `PATHEXT`; e MUST entregar ao CLI resolvido como `.cmd` ou `.bat` o prompt exatamente como foi composto.

#### Scenario: CLI instalado fora do PATH do app gráfico
- **WHEN** o `claude` existe apenas em um diretório adicionado pelo shell de login (por exemplo `~/.local/bin`) e o usuário abre a lista de provedores
- **THEN** o Clovy mostra `claude` como instalado com o caminho resolvido e permite selecioná-lo
- Test: unit — resolução de PATH via shell de login com executável falso em diretório temporário

#### Scenario: CLI ausente
- **WHEN** nenhum executável `cursor-agent` é encontrado
- **THEN** o provedor aparece como não instalado, não pode ser selecionado e o motivo é exibido
- Test: unit — detecção com PATH sem o executável

#### Scenario: CLI instalado pelo npm no Windows
- **WHEN** no Windows o `codex` foi instalado pelo npm, que grava `codex.cmd` e um script `codex` sem extensão em `%APPDATA%\npm`, e o ambiente do app traz esse diretório na variável `Path`
- **THEN** o Clovy mostra `codex` como instalado com o caminho de `codex.cmd` e a versão informada pelo CLI
- Test: unit — resolução com chave `Path`, `PATHEXT` e diretório temporário contendo `codex.cmd` e `codex` sem extensão

#### Scenario: CLI fora do Path no Windows
- **WHEN** no Windows o `claude.exe` existe só em `%USERPROFILE%\.local\bin` e esse diretório não está no `Path` do app
- **THEN** o Clovy mostra `claude` como instalado com esse caminho
- Test: unit — diretórios de reserva do Windows derivados de `USERPROFILE` e `APPDATA`

#### Scenario: Prompt íntegro num CLI `.cmd`
- **WHEN** no Windows um CLI resolvido como `.cmd` recebe um prompt com várias linhas, aspas, `%`, `^` e `&`, por argumento ou pela entrada padrão
- **THEN** o CLI recebe o texto byte a byte igual ao composto pelo Clovy, ou a execução falha com erro explícito, nunca com o texto alterado
- Test: integration (Windows) — `.cmd` falso que devolve o argumento e a entrada padrão recebidos
