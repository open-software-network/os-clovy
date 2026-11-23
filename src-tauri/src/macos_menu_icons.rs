#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplication, NSImage, NSMenu, NSMenuItem};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSInteger, NSSize, NSString};

#[cfg(target_os = "macos")]
pub fn install_settings_symbol_on_app_menu(settings_title: &str) {
    use objc2::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(main_menu) = app.mainMenu() else {
        return;
    };
    let title = NSString::from_str(settings_title);
    let Some(settings_item) = find_menu_item(&main_menu, &title) else {
        eprintln!("Warning: Could not find {settings_title} menu item");
        return;
    };

    let symbol_name = NSString::from_str("gearshape");
    let description = NSString::from_str("settings");
    let Some(symbol) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &symbol_name,
        Some(&description),
    ) else {
        eprintln!("Warning: Could not load gearshape SF Symbol");
        return;
    };
    symbol.setSize(NSSize::new(18.0, 18.0));
    settings_item.setImage(Some(&symbol));
}

#[cfg(target_os = "macos")]
fn find_menu_item(menu: &NSMenu, title: &NSString) -> Option<objc2::rc::Retained<NSMenuItem>> {
    if let Some(item) = menu.itemWithTitle(title) {
        return Some(item);
    }

    for index in 0..menu.numberOfItems() {
        let index = index as NSInteger;
        let Some(item) = menu.itemAtIndex(index) else {
            continue;
        };
        let Some(submenu) = item.submenu() else {
            continue;
        };
        if let Some(found) = find_menu_item(&submenu, title) {
            return Some(found);
        }
    }

    None
}

#[cfg(not(target_os = "macos"))]
pub fn install_settings_symbol_on_app_menu(_settings_title: &str) {}
