# Design — publicação Mac-only aprovada

## Context

A integração inicial chegou à main em ed74fe3e; a correção de empacotamento b56de142 é a fonte da tag v0.1.6 publicada. O CI anterior exigia Windows mesmo para entregar o updater do Mac; faltam os secrets do certificado Windows. A spec canônica ainda descrevia obrigatoriedade de ambos e Windows sem Authenticode.

## Goals / Non-Goals

Entregar o pacote universal Mac pelo canal estável real do GitHub, com assinatura updater preservada. Não fabricar instalador Windows, importar credenciais, trocar chaves, instalar outro Clovy local ou declarar aceite nativo não executado.

## Approaches

Manter os dois sistemas exigiria certificado Windows do dono e adiaria a publicação. Publicar somente Mac agora foi a escolha explícita do dono no ask. Uma seleção temporária de repositório, com all como padrão, evita hardcode da tag e não elimina suporte ao fluxo dos dois sistemas.

## Decisions

CLOVY_RELEASE_PLATFORMS aceita all/macos, capturado no job de versão como output imutável. Publicação compara os resultados dos jobs selecionados e rejeita falha/cancelamento. O CLI recebe o mesmo modo; macos exige os três arquivos Mac e só gera duas entradas Darwin. Em all, Windows e suas assinaturas continuam obrigatórios. A unidade anterior estava ausente; a unidade própria macos foi conferida integralmente e removida após cada tentativa, restaurando essa ausência sem tocar vizinhas.

## Verification and limits

Worker:20 testes de release, RED/GREEN e CLI real com bytes sintéticos; actionlint e análise de384 combinações de condição, sem alegar execução do scheduler. Pai: smoke Node24 no Mac; prova Tauri real app/DMG/updater; revisão da implementação exit0 e scanner clean. O run42306597955 falhou antes de publicar por ordem de argumentos de lipo. O run42314751511, na fonte b56de142, publicou v0.1.6 em2026-11-07T01:06:09Z: versão/Mac/publish aprovados e Windows pulado. O manifesto público e a assinatura do archive foram verificados com a chave pública configurada, sem extração ou instalação. Sem execução Windows, autenticação real ou inferência paga; Computer use tem apenas aceite estático e exige assinatura Apple com Team ID para executar.
