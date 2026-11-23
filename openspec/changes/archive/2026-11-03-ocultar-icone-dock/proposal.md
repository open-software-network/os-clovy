# Proposal

## Why

Fechar a janela do Clovy só a esconde: o app segue rodando pelo ícone da barra de menus, mas o ícone continua no Dock. Quem usa o Clovy em segundo plano (ditado, captura de atividade) quer só o ícone no topo, sem ocupar o Dock.

## What Changes

- Nova preferência "Ocultar do Dock" em Configurações → Geral, só no macOS, desligada por padrão.
- Com a preferência ligada, ao fechar a janela principal (botão de fechar ou Cmd+Shift+W) o ícone sai do Dock e do Cmd-Tab; o app continua na barra de menus.
- Ao reabrir a janela (barra de menus, segunda instância, notificação, atalho de Configurações), o ícone volta ao Dock enquanto a janela estiver aberta, para o app receber foco de teclado e manter o menu do app.
- Com a preferência desligada, nada muda em relação a hoje.
- A preferência é gravada pelo Rust e sobrevive a reinícios.

## Capabilities

### New Capabilities
- `dock-presence`: presença do Clovy no Dock do macOS enquanto a janela principal está fechada e a preferência que controla isso.

### Modified Capabilities

## Impact

- Rust: `src-tauri/src/main_window.rs` (troca de política de ativação ao esconder e mostrar), `src-tauri/src/lib.rs` (`close_main_window`, registro de comandos e setup), novo arquivo de preferências do app em `app_config_dir`.
- Frontend: `src/lib/tauri.ts` (tipo e wrappers), `src/components/settings/AppSettings.tsx` (linha nova em Geral), `src/i18n/messages/settings.ts` (en e pt-BR).
- Sem dependência nova: usa `AppHandle::set_activation_policy` do Tauri 2.
- Windows e Linux: sem efeito; a linha não aparece.
