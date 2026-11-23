# Spec Delta

## Purpose

Oferecer controle de navegador próprio do Clovy, com logins separados persistentes e autoridade temporária isolada por tarefa, preservando segurança e encerramento.

## ADDED Requirements

### Requirement: Navegador próprio e login humano
O Clovy SHALL usar agent-browser integrado por ferramentas próprias, com perfil persistente exclusivo no uso assistido e perfil anônimo descartável em rotinas autorizadas. Não SHALL copiar cookies/perfil pessoal ou auto-conectar ao navegador habitual. Login/senha/OTP exigem takeover humano, durante o qual captura e ações do agente ficam suspensas. Stop encerra controle/processos, não apaga os logins persistidos.

#### Scenario: Login e reabertura
- **WHEN** usuário autentica-se no navegador dedicado, encerra a tarefa e o reabre
- **THEN** login persiste, mas nova tarefa precisa de autoridade própria e não herda abas/refs da anterior
- Test: native Mac/Windows — login sintético e isolamento; box — perfil/lease/lifecycle reais

### Requirement: Transporte privado e identidade estrita
Controle SHALL passar pelo broker Clovy com dono definido pelo host, origem e referências qualificadas por aba/documento/nó. Inspeção e ação MUST revalidar a identidade exata. O canal MUST ser privado e autenticado, sem streaming, CDP ou comandos públicos desprotegidos. Não SHALL publicar MCP upstream, eval, cookies, credenciais ou sessão escolhida pelo modelo.

#### Scenario: Autoridade do contexto
- **WHEN** um consumidor interno ou externo solicita controle
- **THEN** a sessão/run interna ou o contexto externo autorizado pelo host, conforme `retirar-chat-do-clovy`, delimita a ação

#### Scenario: Nó substituído depois da aprovação
- **WHEN** um formulário troca o nó por outro com mesmo nome/role
- **THEN** ação é recusada sem clicar/preencher o substituto
- Test: browser real — fixture local, duas abas e troca de documento/nó

#### Scenario: Página não autoriza seus próprios efeitos
- **WHEN** um controle fora de formulário tem rótulo neutro ou localizado, mas pode executar efeito em click, input ou key handler
- **THEN** click, fill e press exigem aprovação central vinculada ao elemento inspecionado; negar não despacha input e rotina sem pessoa não executa silenciosamente
- Test: policy portável — rótulos e operações; native broker/MCP — aprovação negada e zero dispatch; browser real — handlers e nó exato

#### Scenario: Alvo concreto na aprovação
- **WHEN** dois elementos da mesma aba e origem têm rótulos Delete draft e Delete account
- **THEN** a card central distingue os alvos por rótulo inspecionado sanitizado, capado e explicitamente não confiável, sem valor de campo, e preserva a revalidação exata após a decisão
- Test: frontend — consumer AgentWorkspace real en/pt e decisão no id correto; native broker — preview e efeito recusado

#### Scenario: Campo sensível mostrado como texto
- **WHEN** campo text usa autocomplete current-password ou new-password, inclusive com outros tokens
- **THEN** click, fill e press exigem takeover humano e não disparam handlers do campo
- Test: browser real — contador de eventos zero; policy portável — tokens combinados; native broker — sem card ou input

#### Scenario: Conteúdo editável não sai por captura ou metadados
- **WHEN** texto editável transborda a caixa ancestral ou um campo muda entre inventário e captura AX
- **THEN** imagem e snapshot não emitem conteúdo de campo nem nomes acessíveis que o reproduzem; se a fronteira consistente não pode ser estabelecida a operação falha fechada
- Test: browser real — descendente editável distante, snapshot com mutação intercalada e input novo, controle de contexto público legível

### Requirement: Rede e encerramento supervisionados
O app SHALL preservar políticas de rede e bloqueio privado/DNS pinning aplicáveis às rotinas e grants assistidos, sem substituir proteção por allowlist genérica. Stop/revogação/shutdown MUST encerrar daemon e descendentes desde startup; resultado perdido após mutação MUST permanecer incerto sem replay. Perfil ocupado por outra tarefa MUST ser recusado explicitamente.

#### Scenario: Stop durante startup e entrega incerta
- **WHEN** startup é cancelado ou a resposta se perde depois do dispatch
- **THEN** processos não sobrevivem autorizados e a mutação não é reenviada
- Test: integration — supervisor real, processo filho e fixture de conexão interrompida

#### Scenario: Rede muda após efeito entregue
- **WHEN** o host entrega uma mutação e a validação posterior encontra destino privado ou erro DNS
- **THEN** tarefa e handles são encerrados e o recibo durável permanece incerto com instrução de não repetir, em vez de recusa anterior ao efeito
- Test: browser/adapter reais — resolver controlado e contador um; native broker — ledger unknown, handles inválidos e controle pré-efeito zero/refused

### Requirement: Integração completa e modo efetivo
A capacidade autorizada SHALL executar transporte real em produção, com navegador/motor pinados e empacotados para macOS/Windows, erros acionáveis e fluxo nos dois CLIs nas superfícies autorizadas. Segundo plano é padrão; modo visível é permitido quando background não suporta a ação e antes de despacho incerto. Dependência exclusiva de extensão/pareamento e backend antigo MUST ser retirados após cutover, preservando utilidades compartilhadas.

#### Scenario: Usar sem terminal manual
- **WHEN** o usuário habilita o recurso e Claude Code ou Codex solicita navegação em um contexto autorizado
- **THEN** navegador dedicado inicia e ferramentas funcionam sem instalar/rodar CLI global ou anexar Chrome pessoal
- Test: box — smoke com motor real; native — pacote/macOS/Windows e ambos CLIs
