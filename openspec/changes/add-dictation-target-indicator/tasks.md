# Tasks

## 1. Contrato e apresentação

- [x] 1.1 Capturar RED do consumidor com testes imutáveis de indicador, foco, pin, fallback e cancelamento.
- [x] 1.2 Publicar metadados correlacionados no helper e preservar snapshot no Rust; verificar testes Rust e build Swift, documentar protocolo no mapa nativo.
- [x] 1.3 Implementar indicador persistente, resolução local e strings EN/pt-BR; verificar GREEN focado e regressões HUD.
- [x] 1.4 Documentar método nativo isolado para Main sem abrir o app; conferir contra ADR-0014 e AGENTS.

## 2. Integração da entrega

- [x] 2.1 Capturar smoke de rendering com bridge controlada e screenshots de listening, processing, paste e fallback; registrar limites no report.
- [x] 2.2 Preparar snapshot final e report ignorado para recibos; verificar git diff --check, pnpm check e pnpm typecheck.

Capturas finais de make verify, build canônico e verify-cli-worker são registradas no report ignorado após o commit, sem mutar este snapshot. Aceite e prova nativa pertencem ao Main.
