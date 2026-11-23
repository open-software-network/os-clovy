# Design

## Context

- A lavagem é só CSS: `.agent-workspace::before` (`src/styles/app.css` ~271-307, hero), `[data-home="true"]::before` (~347-380, claro e escuro), camadas de marca do véu do composer do Início (~5532-5552) e o cartão da etapa de personalidade do onboarding (~23676-23691). Tokens `--hero-wash`, `--hero-wash-soft`, `--hero-wash-max-w`, `--hero-wash-side-fade` em `src/styles/tokens.css`. O `[data-home]::after` é véu de rolagem, não brilho, e fica.
- `.agent-workspace` já cria contexto de empilhamento (`isolation: isolate`), então uma camada de fundo dentro dela fica local.
- Tema: `src/lib/theme.ts` (padrão "system"), espelhado antes do paint em `index.html`, `styleguide.html` e `onboarding-preview.html`. Preferências de interface ficam em localStorage, um módulo por preferência em `src/lib/` (modelo: `src/lib/date-format.ts`).
- pnpm 11 com `minimumReleaseAge: 10080` (7 dias) e `allowBuilds` negado por padrão; instalação pela regra `spec/package-install-security.md` (`sfw pnpm add`).
- Em 2026-10-28, a versão mais nova com mais de 7 dias é `asciify-engine@4.29.0` (2026-10-20). O skill do asciify foi testado na 4.32.8; `background`, `drift` e `mark` existem desde 4.14/4.19.

## Goals / Non-Goals

**Goals:**
- Uma camada de fundo única para Nova sessão e Início, controlada por uma preferência.
- Reproduzir a seleção Drift do dono sem trocar padrão nem estilo.

**Non-Goals:**
- Outros presets além de Drift (ficam para depois).
- Fundo nas janelas HUD, nas notas ou nas configurações.
- Mudar a cor de destaque (`--brand`) usada em botões e ícones.

## Decisions

1. **Integração pelo kit, não pelos arquivos baixáveis.** `background(host, { preset: drift, mark: 'dot' })` de `asciify-engine/kit`, como na integração de referência. Alternativa descartada: template baixado (`asciiBackground` de `./drift.js`), que o próprio skill marca como legado.
2. **Versão fixa `asciify-engine@4.29.0`** via `sfw pnpm add asciify-engine@4.29.0`. Antes de usar, conferir nos tipos instalados (`node_modules/asciify-engine/dist/kit*.d.ts`) que `background`, `drift`, `mark`, `clear` e `destroy` existem e quais dos controles da seleção o tipo aceita. Se o pacote tiver script de instalação, adicionar entrada versionada em `allowBuilds`. Atualizar para 4.32.x é tarefa separada, depois da janela de 7 dias.
3. **Mapeamento da seleção.** Override do dono em 2026-10-28: a opacidade original 1 foi reduzida para **0.14**, como textura quase imperceptível nos dois temas (a tentativa intermediária 0.28 ainda ficou forte). Usar uma única constante `DRIFT_OPACITY` na opção pública `opacity` do kit; Os controles de campo seguem o ajuste posterior abaixo. A seleção do site é `{renderStyle: original, algorithm: bayer4, ditherStrength: 0, scheme: dark, ink: #8f8d86, speed: 1, size: 11, opacity: 1, characters: original, scene: flow, hover: trail, mark: dot}`. Os campos que o tipo do kit aceita vão como opção; a tinta vai como `--ascii-ink: #8f8d86` no host (o kit pinta pelos tokens `--ascii-ink*`); `scheme` segue o tema do app (o kit acompanha claro e escuro). Campo da seleção que o kit 4.29.0 não expõe e não é o padrão do preset fica registrado nesta seção na implementação; se algum controle essencial (preset, marca, tinta) não existir, parar e perguntar ao dono em vez de inventar export.
4. **Componente único** (nome final do implementador, por exemplo `AgentBackground`) montado dentro de `.agent-workspace` quando `heroMode` ou `homeMode`, com `position: absolute; inset: 0 calc(var(--sp-8) * -1); z-index: -1; pointer-events: none`. Import dinâmico de `asciify-engine/kit` dentro de `useEffect`, com guarda: se desmontar antes do import resolver, não criar o renderer. Um handle por montagem; `destroy()` no cleanup. O campo cobre toda a superfície, de ponta a ponta, inclusive atrás do título, do composer e das sugestões. Não passar `clear` para uma coluna nem recortar uma faixa central. O cartão existente do composer mantém sua legibilidade; somente se a inspeção mostrar necessidade, pode haver um véu justo aos elementos de título/composer, nunca uma coluna de altura total.
5. **Pausa e movimento reduzido ficam com o kit** (o skill garante pausa fora da tela e quadro parado com movimento reduzido). O componente não duplica esse controle; a verificação visual confirma.
6. **Preferência** `src/lib/app-background.ts`: chave `os-clovy:background`, valores `drift | image | none`, padrão `drift`, normalizador, evento de mudança na janela (padrão de `date-format.ts`). Sem evento Tauri, porque as HUDs não usam o fundo.
7. **Tema escuro padrão**: sem chave salva, `getStoredTheme` devolve `"dark"` e os três scripts antes do paint resolvem escuro. Mantém o literal `localStorage.getItem("os-clovy:theme")` que `technical-identity.test.mjs` exige; o texto que o teste espera em `styleguide.html` muda junto.
8. **Remoção da lavagem**: apagar as regras de marca e os tokens `--hero-wash*` sem uso; no véu do composer, manter só a camada de fade do card. O teste `home-polish-css.test.ts` "contains the new-session tint..." deixa de valer e é removido ou trocado por uma checagem de ausência de `--brand` atrás do conteúdo.

## Risks / Trade-offs

- [WKWebView sem WebGL2 em algum Mac] → o kit devolve handle inerte; cenário de falha coberto por teste e verificação com WebGL desligado.
- [Custo de GPU com o app aberto o dia todo] → pausa fora da tela e janela oculta vem do kit; medir no `pnpm tauri:dev` e citar dispositivo se houver número de FPS.
- [Versão 4.29.0 atrás da testada pelo skill] → conferir os tipos instalados antes de usar cada opção.
- [Quem tinha o tema em "sistema" sem salvar passa a ver escuro] → é o pedido; quem salvou "sistema" continua em "sistema".

## Registro de implementação (2026-10-28)

- Skill lido diretamente de https://asciify.org/skill/SKILL.md e das três referências exigidas. O CLI `skills` não está instalado; instalar a skill criaria arquivos fora do escopo autorizado. Nenhum `npx` ou código de registry foi executado para essa leitura. A dependência, que também distribui a skill, foi instalada via `sfw pnpm add -w --save-exact asciify-engine@4.29.0`.
- Tipos públicos conferidos: `dist/kit.d.ts` (`KitOptions`, `KitHandle`, `BackgroundOptions`, `BackgroundPreset`, `BackgroundHandle`, export `drift`) e `dist/glyph-field-DRC-7uVf.d.ts` (`GlyphFieldOptions`). Sem script de instalação do asciify; nenhuma entrada nova em `allowBuilds`.
- Decisão 3, mapeamento confirmado: `preset: drift`, `mark: "dot"`, `speed: 1`, `cellSize: 11`, `opacity: DRIFT_OPACITY` (0.14 após override do dono), `cursor: "trail"`, `look: { scene: "flow", contrast: 1.4, scale: 1.6 }`. O preset tem scene `drift` por padrão; `look.scene` é a opção pública que aplica a seleção `flow` do dono. `ink: "--ascii-ink"` referencia no host o token `--agent-background-ink: #8f8d86` (o kit não lê automaticamente esse token para presets; a referência explícita é necessária). `scheme` acompanha o tema do app, sem forçar dark no renderer. `characters: original` preserva o charset do preset; `renderStyle: original` não injeta um global look. `algorithm: bayer4` não é opção do background/field nesta versão; com `ditherStrength: 0`, nenhum algoritmo adicional é aplicado (`dither` já é 0). Não há substituição de preset, marca ou tinta.
- Após feedback do dono, o componente `AgentBackground` não passa `clear`: o campo preenche também o centro da superfície. Nova sessão e Início têm montagens separadas; mudança de tema não remonta. O kit gerencia visibilidade e movimento reduzido; o app gerencia import assíncrono, preferência e destruição.

- Verificação em 390 px encontrou o mínimo global de 720 px no `body`. A exceção usa `body[data-agent-surface="true"]`, mantendo o mínimo nas outras telas; sugestões e toolbar do hero podem quebrar linha. O desenho continua cobrindo o centro também em 390 px.

O teste de estabilização proíbe `:has()` no stylesheet. O estado de superfície é explícito: `AgentWorkspace` adiciona `data-agent-surface` ao body enquanto hero/Início estão montados e remove no cleanup. A regra de largura estreita depende desse atributo.

- Ajuste de bordas solicitado pelo dono (2026-10-28): o host herdava a caixa de conteúdo de `.main-panel-body`, que tem padding horizontal `--sp-8` (24 CSS px na prova). Canvas e host ficavam 24 px para dentro de cada borda; topo e base já coincidiam com o painel. O inset horizontal negativo usa o mesmo token para estender apenas a decoração, sem mexer no conteúdo. O overflow/raio já existente de `.main-panel` faz o recorte; não há margem nem edge fade adicional no kit. As caixas e os pixels nas quatro bordas são verificados em 1180 e 390 px, com crops das laterais.

- Ajustes de rodapé e vazios (2026-10-28): a faixa inferior não era altura do host nem arredondamento da grade. O canvas já coincidia com o painel nos quatro lados; `.agent-composer:not([data-hero="true"])::before` pintava o gap inferior (bottom -12 px e largura 924 px para um cartão de 680 px na prova). No Início o véu neutro fica limitado à largura/altura do cartão, com bottom 0; o fade de outras conversas e o véu superior de rolagem continuam.
- `GlyphFieldOptions` público de 4.29.0 expõe `brightness`, `contrast`, `gamma`, `levels`, `scale` e `cellFit`; não expõe threshold/density. O preset Drift usa `levels: none`, `shade: 0`, brightness 0 para dot e scale responsivo (1 largo / 1.8 estreito). Mantemos levels, shade, brightness e a cena flow; scale passa a 1.6 nos dois tamanhos para vários vazios orgânicos no centro, adicionando `look.contrast: 1.4, scale: 1.6` (faixa normalizada suportada 0 a 4) para recortar valores baixos a zero dentro do próprio campo. A opacidade global 0.14 é só a composição final e não muda a distribuição ligada/desligada dos pontos. Não há máscara, clear, tinta mais escura nem vazio central fixo. Duas capturas separadas por alguns segundos verificam vazios móveis no centro.

## Decisões do orquestrador: imagem local (2026-10-28)

- Fundo passa a `drift | image | none`, mantendo o default Drift e normalização dos valores existentes. A opção Imagem abre o diálogo nativo quando não há cópia válida; após escolher, Aparência mostra nome, Escolher imagem e Remover. Cancelamento ou rejeição não mudam preferência nem cópia.
- O plugin de diálogo já existe. Comandos próprios escolhem, validam, copiam, leem e removem somente a imagem gerenciada em `app_paths::app_data_dir()/backgrounds/`. O frontend nunca envia nem persiste um caminho arbitrário. Nome original (sem diretório) e nome interno UUID ficam em `current.json`. Um arquivo temporário publica o manifesto atomicamente depois da nova cópia validada; a anterior é removida após a troca. Operações de arquivo são serializadas no backend; o diálogo nativo espera a escolha antes de adquirir o mutex, sem bloquear leituras enquanto está aberto.
- PNG/JPEG/WebP, extensão e conteúdo verificados, limite de 20 MiB antes e durante leitura. `image =0.25.10` (já transitivo no lock) ganha dependência direta com somente png/jpeg/webp para decodificar antes de substituir; limites de alocação padrão do decoder permanecem. Cargo via sfw; verificação de idade mínima passa. Arquivo corrompido é rejeitado. A leitura do fundo retorna um data URL local via IPC, sem ampliar o escopo asset e sem upload. Cópia ausente/corrompida retorna nome e conteúdo nulo.
- Imagem e Drift são ramos mutuamente exclusivos, destruindo o handle ao trocar. A mesma geometria do host cobre quatro bordas, recortada pelo painel. Imagem usa cover/center, aria-hidden, sem ponteiro, e véu `--background-image-veil: 0.82` em um único ponto, com a cor `--card` do tema. Erros de carregamento geram fundo liso silencioso; Aparência mostra indisponibilidade. Remover apaga apenas a cópia e salva Nenhum.
- A faixa inferior anterior vinha do pseudo-elemento do composer Home (bottom -12px e largura maior que o cartão). Ele agora fica limitado ao cartão, preservando o dissolve interno e o véu de scroll do topo. O canvas já tinha a altura inteira; nenhum overscan vertical foi necessário.
- Drift final mantém 0.14 e usa flow com contraste 1.4/escala 1.6. O contraste 2.4 de teste esvaziava quase todo o centro e foi descartado. Capturas separadas por quatro segundos mostram mudanças dos vazios em ambos os temas e larguras, com campo cobrindo as bordas sem áreas de limpeza fixas.

- Revisão 6: metadados JSON corrompidos ou UUID inválido passam a significar imagem ausente. Escolher e Remover não dependem de ler esse manifesto: recuperam somente cópias com nome UUID no diretório backgrounds e preservam arquivos de outros nomes. Cópias são publicadas a partir de NamedTempFile (permissão privada em Unix); substituição bem-sucedida também remove cópias órfãs de uma interrupção anterior.

### Verificação visual corrigida (2026-10-28)

A captura nativa antiga da imagem escura foi feita antes de a leitura assíncrona terminar: uma região vazia do painel tinha exatamente uma cor. Repetindo após o carregamento, a mesma região apresenta 162 cores e variação de até 32 níveis, tanto no Início quanto na Nova sessão. O véu 0.82 preserva a imagem e a legibilidade nos dois temas; não foi necessário alterar sua intensidade. A prova de imagem deve esperar pelo carregamento e verificar pixels do fundo, não apenas a presença do título/composer. A galeria final usa uma lista explícita de capturas atuais e pares de Drift separados no tempo, sem incorporar capturas históricas por glob.

### Limites de decodificação (revisão 11, 2026-10-28)

Além do limite comprimido de 20 MB, configurar `image::Limits` antes de `decode`: largura/altura máximas de 8192 pixels e `max_alloc` de 128 MiB. O ImageReader reserva o tamanho total do buffer de saída antes de decodificar; os limites de dimensão são estritos, e o limite de alocações internas depende do decoder. Excesso mantém a cópia anterior e pede reduzir a resolução na mensagem inline. PNG/JPEG/WebP normais continuam no mesmo caminho de validação/cópia.

Substituir uma imagem pela tela Aparência não exige um segundo evento: essa rota desmonta o workspace. Ao voltar, ImageBackground monta e lê a cópia nova mesmo com a preferência ainda `image`. A prova `image-replacement-proof.log` verifica desmontagem, preferência inalterada e novo src/naturalWidth em ambos temas e larguras.
