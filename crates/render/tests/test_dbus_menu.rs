use malus_de_render::dbus_menu::{clean_menu_label, normalize_app_id, DbusMenuService, RegisteredMenu};

#[test]
fn test_dbus_menu_label_cleaning_and_normalization() {
    assert_eq!(clean_menu_label("_Archivo"), "Archivo");
    assert_eq!(clean_menu_label("Edi_ción"), "Edición");
    assert_eq!(clean_menu_label("Visuali_zación"), "Visualización");
    assert_eq!(clean_menu_label("No_Accelerator_Here"), "NoAcceleratorHere");

    assert_eq!(
        normalize_app_id("/usr/share/applications/org.gnome.Nautilus.desktop"),
        "org.gnome.nautilus"
    );
    assert_eq!(
        normalize_app_id("org.mozilla.firefox.desktop"),
        "org.mozilla.firefox"
    );
    assert_eq!(normalize_app_id("Brave-browser"), "brave-browser");
}

#[test]
fn test_dbus_menu_service_registration_lookup() {
    let (service, _registrar) = DbusMenuService::new();

    // 1. Registro por ID de ventana
    service.registrations_by_window.lock().unwrap().insert(
        1001,
        RegisteredMenu {
            window_id: 1001,
            service: ":1.42".to_string(),
            path: "/MenuBar/1001".to_string(),
        },
    );

    let found_by_win = service.find_menu(Some(1001), None);
    assert_eq!(
        found_by_win,
        Some((":1.42".to_string(), "/MenuBar/1001".to_string()))
    );

    // 2. Registro por App ID normalizado
    service.registrations_by_service.lock().unwrap().insert(
        "org.gnome.nautilus".to_string(),
        RegisteredMenu {
            window_id: 2002,
            service: "org.gnome.nautilus".to_string(),
            path: "/org/gnome/Nautilus/menus".to_string(),
        },
    );

    let found_by_app = service.find_menu(None, Some("org.gnome.Nautilus.desktop"));
    assert_eq!(
        found_by_app,
        Some((
            "org.gnome.nautilus".to_string(),
            "/org/gnome/Nautilus/menus".to_string()
        ))
    );

    // 3. Ventana no registrada
    assert_eq!(service.find_menu(Some(9999), None), None);
    assert_eq!(service.find_menu(None, Some("app.inexistente")), None);
}
