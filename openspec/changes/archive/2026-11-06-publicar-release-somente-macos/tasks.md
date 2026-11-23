# Tasks

## 1. Contrato aprovado e implementação

- [x] 1.1 Registrar a seleção explícita do dono: publicar0.1.6 somente para Mac, sem novo instalador Windows.
- [x] 1.2 Implementar seleção única all/macos e geração de manifesto sem redução silenciosa; manter all padrão e assinatura updater.
- [x] 1.3 Provar RED/GREEN focado, CLI, actionlint e gate final sem achados; alinhar guias e AGENTS.
- [x] 1.4 Preparar e validar o delta de release-distribution, preservando os cenários existentes e sem modificar app-updates; a sincronização canônica será feita pelo CLI no arquivamento, após os destinos abaixo.

## 2. Destinos autorizados

- [x] 2.1 Integrar e enviar main b56de1422ddfecf0a674c117eaede17032a48e01 e tag v0.1.6, sem force e sem build local de produção. Provas: main-integration-receipt.json, publication-dispatch.json e publication-dispatch-attempt2.json em .tmp/verify/release-20261106/.
- [x] 2.2 Run GitHub42314751511 aprovado na fonte b56de142; release pública v0.1.6, quatro assets completos e latest.json anônimo com exatamente dois alvos Darwin. Archive454.417.965bytes: SHA256 igual ao GitHub e assinatura criptográfica válida com a chave pública configurada, usando minisign-verify0.2.5 em fluxo. Provas: published-release.json, public-latest.json e updater-signature-verification/published-verification.json. App instalado não substituído.
- [x] 2.3 Ausência original de CLOVY_RELEASE_PLATFORMS restaurada após o run; unidade própria completa conferida antes de remover, sem tocar vizinhas. Prova: github-variable-restored-final.json.
- [x] 2.4 Recursos próprios limpos e provas preservadas: dois executores/checkouts da box, dois espelhos locais/branches, targets exatos autorizados e refs antigas integradas. Dados/WIP/credenciais e stores compartilhados mantidos; sete worktrees antigos e o checkout original retidos com justificativa. Medidas e limites: cleanup-summary.md, cleanup-consolidated.json e cleanup-final-retained-refs.json.

## Evidência já observada

- Integração inicial em ed74fe3e, árvore912fdec68f5055592099f87609a563af5a5909d0. Gate final dos dois revisores exit0 e scanner clean: components/packaging/review-pair/final-integration-3/summary.md. A correção adicional revisada está em b56de142.
- 20 testes de release passaram; smoke CLI no Mac confirmou só Darwin. Fonte Rust do delta final passou2669 testes,0falhas,16ignorados e Clippy-Dwarnings; não confundir isso com captura real ou Windows nativo.
- A variável não secreta estava ausente e foi criada macos exclusivamente para esse run; valor anterior e unidade posterior foram preservados.
- Este registro documental foi criado depois da implementação pequena aprovada. Não reapresenta o trabalho como uma proposta executada antes dos fatos.
- A primeira execução42306597955 compilou as duas fatias do app, mas falhou no relay MCP: Xcode16 interpretou o executável depois de `-verify_arch` como outra arquitetura. `publish` foi pulado e não havia release público. O estado anterior ausente da variável foi restaurado ao final dessa tentativa.
- A correção mínima b56de142 passa o executável antes da opção. Seis casos Mach-O reais passaram; Xcode27 aceita a chamada antiga, portanto o RED é o log real do CI, não uma falha local inventada. Gate adicional dos dois arquivos: ambos revisores concluídos, zero achados, scanner limpo; `.tmp/verify/release-20261106/lipo-order-fix/review-pair/1/summary.md`.
- A tag própria ainda não publicada foi substituída após conferir sua identidade e a ausência de release. Novo push atômico sem force; a unidade temporária da tentativa2 foi preservada integralmente para restauração após o run corrigido. O app instalado não foi substituído; a nova compilação e a publicação são executadas exclusivamente no GitHub.
- Publicação aceita: https://github.com/guilhermexp/os-clovy/releases/tag/v0.1.6, publicada em2026-11-07T01:06:09Z. Windows foi pulado como aprovado. CI verificou resources do browser nos dois alvos e codesign deep/strict do app; release Mac é ad hoc, não notarizada. Computer use passou apenas nos checks estáticos e ainda exige assinatura Apple com Team ID para executar; não alegar prova de runtime desse driver.
- Verificador efêmero, vetores, probes e archive bruto de verificação foram removidos após a prova; somente resultados, hashes e material público de assinatura permanecem. Nenhuma instalação/extracão de app ou acesso a chave privada real.
- Arquivamento desta change sincroniza somente release-distribution pelo CLI; app-updates e changes com aceites nativos abertos ficam inalteradas. A rodada documental1 apontou a referência ao run falhado no design, corrigida para identificar fonte/run/publicação reais. Gate do commit documental final: `.tmp/verify/release-20261106/documentary-review/2/summary.md`; o recibo definitivo registra o resultado, sem antecipar aprovação.
