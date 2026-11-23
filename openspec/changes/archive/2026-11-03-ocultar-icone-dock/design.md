# Design

## Context

- `main_window.rs` já trata o fechamento: `CloseRequested` chama `prevent_close` e `hide`. Todo caminho de reabrir passa por `main_window::show` → `AppSlot::reveal` (Dock `Reopen`, barra de menus, segunda instância, menu Configurações). Um teste (`no_other_module_shows_the_main_window_on_its_own`) garante esse ponto único.
- Há um segundo caminho de esconder: `close_main_window` em `lib.rs` (Cmd+Shift+W) chama `window.hide()` direto.
- Nada hoje altera a política de ativação; o app é sempre `Regular`.
- Preferências do Rust seguem o padrão de `experimental_settings.rs`: struct com `#[serde(default)]`, estado gerenciado com `Mutex`, arquivo JSON gravado de forma atômica, comandos get/set.

## Goals / Non-Goals

**Goals:**
- Um único ponto decide a política de ativação a partir de (preferência, janela visível).

**Non-Goals:**
- Esconder o ícone com a janela aberta.
- Abrir o app escondido no login.
- Qualquer mudança no Windows ou Linux.

## Decisions

1. **Trocar a política conforme a janela, não fixar `Accessory`.** Janela escondida + preferência ligada → `Accessory`; qualquer outro caso → `Regular`. App `Accessory` não tem menu do app nem entra no Cmd-Tab e `set_focus` não garante foco de teclado; os atalhos Cmd+W, Cmd+Shift+W e Configurações vêm do menu do app. Alternativa rejeitada: `LSUIElement` no Info.plist ou `Accessory` permanente.
2. **Função pura de decisão** `desired_policy(hide_from_dock, window_visible)` testável sem AppKit, mais um aplicador `#[cfg(target_os = "macos")]` que chama `AppHandle::set_activation_policy`; no-op nas outras plataformas.
3. **Pontos de chamada:** depois do `hide()` em `register_lifecycle`; em `close_main_window` (de preferência apontando para uma função `main_window::hide(app)` compartilhada); antes do `show()` em `AppSlot::reveal`. Assim os caminhos de reabrir existentes ficam cobertos sem nova função de mostrar janela.
4. **Arquivo próprio de preferências do app** (`app_preferences.rs`, `app-preferences.json` em `app_config_dir`, comandos `app_preferences_get`/`app_preferences_set`) copiando `experimental_settings.rs`. Alternativa rejeitada: campo em `ExperimentalSettings`, que tem outro significado e é reconstruído inteiro pelo frontend.
5. **Ligar a preferência com a janela aberta não muda nada na hora**; vale no próximo fechamento. Desligar com a janela escondida não acontece pela interface (a janela está aberta ao clicar).
6. **UI:** linha nova em Configurações → Geral, no mesmo cartão de Inicialização, só no macOS, com `Switch` desabilitado durante a gravação e texto de erro em falha, como `MemorySettingsSection`. A linha MUST aparecer mesmo quando o autostart não for suportado.

## Risks / Trade-offs

- [Ao voltar de `Accessory` para `Regular` o menu do app pode não aparecer até o app ser ativado] → chamar `set_focus` depois da troca; conferir à mão no app de desenvolvimento.
- [O ícone temático (`theme_icon.rs`) pode se perder na ida e volta] → conferir; se sumir, reaplicar após voltar para `Regular`.
- [HUDs e `main_window_dismissed` dependem do estado da janela] → janela escondida já conta como dispensada; nenhuma mudança esperada, conferir HUD de reunião e de ditado com a janela fechada.
