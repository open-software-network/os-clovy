# Spec Delta

## ADDED Requirements

### Requirement: Locais padrão no Windows
No Windows, as fontes SHALL ser lidas a partir do perfil do usuário (`%USERPROFILE%`) para Claude Code, Codex CLI, Copilot CLI, Cursor CLI e Antigravity, e de `%APPDATA%` para Copilot no VS Code e Cursor, com as mesmas regras de somente leitura, segmentação e resumo do macOS.

#### Scenario: Claude Code no Windows
- **WHEN** existe uma transcrição de hoje em `%USERPROFILE%\.claude\projects` e a fonte está ligada
- **THEN** o bloco da sessão aparece na faixa de agentes do dia e o arquivo de origem não muda
- Test: unit — raízes das fontes resolvidas no Windows; e2e — app de desenvolvimento na máquina Windows

#### Scenario: Cursor no Windows
- **WHEN** o Cursor guarda conversas em `%APPDATA%\Cursor\User\globalStorage`
- **THEN** a fonte Cursor lê desse local, e não de `Library/Application Support`
- Test: unit — raízes das fontes resolvidas no Windows
