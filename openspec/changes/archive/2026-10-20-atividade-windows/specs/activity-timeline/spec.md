# Spec Delta

## ADDED Requirements

### Requirement: Tela bloqueada no Windows
Quadros cujo app é a tela de bloqueio ou de login do Windows (`LockApp`, `LogonUI`) SHALL nunca ser úteis, mesmo com entrada do usuário. O intervalo de bloqueio MUST ser contabilizado como "Ausente", não como "Focado" ou "Ocioso", inclusive enquanto o bloqueio está em andamento. A classificação preexistente do macOS MUST permanecer inalterada.

#### Scenario: Windows bloqueado com o Clovy aberto
- **WHEN** o usuário bloqueia o Windows por 20 minutos com a captura ligada
- **THEN** o intervalo conta como "Ausente", não como tempo focado ou ocioso, tanto antes quanto depois do desbloqueio
- Test: unit — construtor com quadros de `LockApp` e `LogonUI` com entrada recente, preservando os resultados macOS; e2e — sessão Windows bloqueada e linha do tempo lida pelo app real
