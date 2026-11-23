# Design

## Context

Ver proposal.md. Em `scripts/tauri-dev.mjs:30-68`, o caminho normal prepara o driver, limpa staging e chama reset de TCC incondicional. `scripts/cargo-clovy-runner.sh:54-88` compila e executa um hard link do Mach-O, sem bundle local responsável. `src-tauri/build.rs:242-302,920-944` usa `APPLE_SIGNING_IDENTITY` quando disponível, mas recai em assinatura ad hoc. O debug final observado tinha assinatura linker ad hoc com Info.plist não vinculada; isso é evidência de identidade inadequada, não prova isolada da causa de cada prompt.

Consulta somente leitura `security find-identity -v -p codesigning` encontrou uma identidade Apple Development válida já existente e duas identidades locais. Não é necessário criar certificado ou keychain para tentar uma assinatura estável nesta máquina. A identidade específica e os dados pessoais não devem ser fixados no código.

## Goals / Non-Goals

**Goals:** fluxo normal de dev com HMR, bundle local estável e reconhecível, assinatura de desenvolvimento consistente e permissões reais preservadas. Corrigir o reset observado e as fronteiras responsáveis por Acessibilidade/captura de tela, microfone e notificações.

**Non-Goals:** instalar release; conceder permissões por escrita de TCC; mudar credenciais, default/search-list/ACL do Keychain; alterar grants internos de tarefas, planos ou rollout; certificar captura real sem autorização e observação efetivas.

## Decisions

1. Remover o reset do caminho normal, em vez de exigir concessão em cada restart. Não manter uma opção nova de reset neste escopo: casos de revogação continuam pelo macOS e a receita de QA pertinente.
2. Preparar um bundle local estável no fluxo do runner, sem instalação em Applications. Preservar argumentos, ciclo de vida, HMR, target/profile, variantes não macOS e as garantias de alias/inode usadas na autenticação dos helpers. Ajustar essa autenticação somente se necessário, sem aceitar parents arbitrários. O executor escolhe o caminho concreto coerente com os leitores atuais e documenta o mapa de app/helper responsáveis.
3. Reusar a infraestrutura de assinatura existente com identidade de desenvolvimento válida: seleção explícita por configuração de dev ou descoberta não ambígua de Apple Development. Propagar a mesma seleção aos helpers e ao app; nunca persistir uma escolha pessoal no código ou exportá-la globalmente para release. Demonstrar designated requirements antes/depois de rebuild. Assinatura ad hoc não é evidência de persistência de TCC entre builds; não anunciar esse resultado sem prova.
4. Reusar helper inalterado quando assinatura e conteúdo correspondem; eliminar limpeza indiscriminada que mude a identidade usada nas concessões. Não reescrever os bundles imutáveis de evidência anteriores.

## Risks / Trade-offs

- [Autorização inicial de TCC] → continua exigida pelo macOS para o novo alvo estável; a mudança deve oferecer um bundle selecionável e consultar estados reais.
- [Uso da chave privada de assinatura pede autorização] → não digitar senha, desbloquear ou alterar Keychain; se aparecer diálogo de senha, interromper somente o processo da tarefa e registrar a dependência exata.
- [Mudança do caminho altera autenticação do helper] → testar parent legítimo e rejeição de parent/alias externos; não relaxar a regra para qualquer bundle com mesmo nome.
- [Ferramenta nativa indisponível] → não confundir smoke de processos, assinatura ou testes com prova de TCC/captura real. Main recupera ou registra a infraestrutura sem declarar o pedido completo funcionando.

## Migration Plan

A correção permanece em main no checkout existente, sem branch, instalação ou push. Não resetar concessões antigas. O launcher passa a apresentar o alvo dev estável; uma autorização inicial desse alvo pode ser necessária. Reverter apenas os arquivos da correção se necessário, preservando dados e autorizações do dono.

## Functional Proof

Antes do fix, executar uma reprodução comportamental em sandbox do launcher normal que detecte a revogação de grants como efeito, sem executar `tccutil reset` no macOS real. Capturar o RED específico antes da implementação e continuar ao GREEN sem esperar associação pelo Main.

Depois do fix, testar o runner com executável controlado e validar bundle/assinatura reais, argumentos, ciclo de vida, identidade do parent e ausência de revogação. Rodar gates uma única vez ao finalizar as edições. A prova nativa é serial e fica com o Main: mesma identidade de prova compilada, cópia do perfil dev sem socket ativo, HOME normal, digest dos dados reais preservado e caminho verificado por lsof; obter autorização inicial real pela UI (confirmação no momento da ação), reiniciar, recompilar uma mudança de código sem trocar assinatura/identidade e reiniciar novamente; reler permissões efetivas no app/helper e exercer as capacidades correspondentes. Não usar o perfil real como fixture, não abrir app por bundle id genérico, não mudar autostart/telemetria. Evidências novas sob `.tmp/verify/<ticket>/run-<id>/`; etapas sem permissão/UI ou com substituição permanecem não comprovadas.

## Implementação adotada

Registrada em [ADR-0060](../../../docs/adr/0060-development-app-runs-as-a-signed-bundle-that-owns-its-grants.md) e [docs/development.md](../../../docs/development.md#the-macos-development-app):

- O runner monta `target/<profile>/Clovy Dev.app` (bundle id `<identifier>.dev`, `Contents/MacOS/os-june`, `clovy-mcp` e `clovy-nm-shim` ao lado), reaproveita o bundle assinado quando a saída do Cargo não mudou e o substitui no mesmo caminho quando mudou. `CLOVY_DEV_APP_IDENTIFIER` isola identidade compilada, diretório de dados e bundle de uma prova.
- Uma identidade de assinatura para app e helpers: `CLOVY_DEV_SIGNING_IDENTITY` ou a única Apple Development válida; ausência ou ambiguidade interrompe com diagnóstico, e ad hoc só com `CLOVY_DEV_SIGNING_IDENTITY=-` explícito. `build.rs` deixou de apagar os bundles de dictation/system audio a cada execução e só reassina o que não satisfaz a identidade.
- Helpers de debug usam `<identificador publicado>.dev.w<worktree>` (mesma chave SHA-256 do checkout que o driver de Computer use já usava): dictation e system audio em `build.rs`, driver em `scripts/computer-use-dev.mjs`. Release mantém os identificadores publicados; o helper de dictation ignora o próprio bundle id em qualquer build.
- Responsabilidade TCC: o processo iniciado do terminal herda o terminal como responsável. `open` (LaunchServices) foi avaliado e rejeitado porque o app deixa de ser filho do runner: o restart do Tauri dev deixaria a instância antiga viva (single-instance), o ambiente dev se perderia e stdio/status de saída não voltariam ao Tauri. O `clovy-dev-launch` faz exec no mesmo pid com `responsibility_spawnattrs_setdisclaim` (SPI não documentada, só no launcher dev, resolvida em runtime; ausência falha com mensagem).
- O helper de Computer use em debug aceita apenas `os-june` num diretório de perfil do Cargo deste checkout (marcado por `.cargo-lock`), como binário do Cargo ou executável principal de um bundle diretamente nesse diretório; os aliases por hard link saíram junto com o runner que os criava.
- Revisão independente F1: em builds debug com `CLOVY_DEV_SIGNING_IDENTITY`, a identidade dev selecionada vence um `APPLE_SIGNING_IDENTITY` herdado (`-` mantém ad hoc explícito); builds release seguem com `APPLE_SIGNING_IDENTITY`.
- Revisão independente F2: `build.rs` compila o identificador selecionado (`TAURI_CONFIG`, depois arquivo de plataforma, depois `tauri.conf.json`) em `CLOVY_APP_IDENTIFIER`; app e shim do native host aceitam só o descriptor da release ou o `-dev` dessa identidade e não caem no perfil normal.
- Re-review F3: a receita de primeira autorização diz que o macOS *pode* pedir a pasta protegida, possivelmente antes da janela; confirmar o pedido real é decisão do dono, e acesso à pasta não comprova readiness.
- Re-review F4: `CLOVY_DEV_SIGNING_IDENTITY=-` explícito exige assinatura ad hoc para reusar helpers (preparo do CU, `build.rs` para CU e Swift), reassinando conteúdo inalterado; sem seleção (cargo comum, release sem identidade) qualquer assinatura válida é mantida.
- Prova do Main (ditado): o bundle dev roda sem `CARGO_MANIFEST_DIR` em runtime e o resolvedor do helper de ditado não achava nenhum executável. Builds debug passam a resolver `.tauri-helper` em tempo de compilação, como os outros helpers; release mantém o fallback runtime existente. O bundle dev passa a carregar todos os recursos que `tauri.conf.json` declara para todas as plataformas (extensão e agent skills), com a árvore no fingerprint.
