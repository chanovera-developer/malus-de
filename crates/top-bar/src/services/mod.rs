pub mod audio;
pub mod bluetooth;
pub mod brightness;
pub mod dbusmenu;
pub mod network;

#[cfg(test)]
mod tests {
    use super::audio::{AudioService, MprisService};
    use super::bluetooth::BluetoothService;
    use super::brightness::BrightnessService;
    use super::network::NetworkService;

    #[test]
    fn test_network_service_dbus() {
        glib::MainContext::new().block_on(async {
            let enabled = NetworkService::is_wifi_enabled().await;
            println!("Wi-Fi enabled via D-Bus: {}", enabled);

            let active_ssid = NetworkService::get_active_wifi_ssid().await;
            println!("Active Wi-Fi SSID via D-Bus: {:?}", active_ssid);

            let (scan_ok, networks) = NetworkService::scan_networks().await;
            println!(
                "Wi-Fi scan result: enabled={}, total redes={}",
                scan_ok,
                networks.len()
            );
            for net in &networks {
                println!(
                    "  - Red: '{}' (señal: {}%, segura: {}, en uso: {})",
                    net.ssid, net.signal, net.is_encrypted, net.in_use
                );
            }
        });
    }

    #[test]
    fn test_bluetooth_service_dbus() {
        glib::MainContext::new().block_on(async {
            let powered = BluetoothService::is_powered().await;
            println!("Bluetooth powered via D-Bus: {}", powered);

            let active_dev = BluetoothService::get_active_device_name().await;
            println!("Active Bluetooth device via D-Bus: {:?}", active_dev);

            let paired = BluetoothService::get_paired_devices().await;
            println!("Paired Bluetooth devices count: {}", paired.len());
            for dev in &paired {
                println!(
                    "  - Dispositivo: '{}' ({}) [conectado: {}, batería: {:?}]",
                    dev.name, dev.address, dev.connected, dev.battery
                );
            }
        });
    }

    #[test]
    fn test_audio_service() {
        glib::MainContext::new().block_on(async {
            let (vol, muted) = AudioService::get_volume().await;
            println!("PipeWire/WirePlumber Volume: {:.1}%, Muted: {}", vol, muted);
            assert!(vol >= 0.0 && vol <= 150.0);
        });
    }

    #[test]
    fn test_mpris_service_dbus() {
        glib::MainContext::new().block_on(async {
            let media = MprisService::get_current_media().await;
            println!("Active MPRIS Media via D-Bus: {:?}", media);
        });
    }

    #[test]
    fn test_brightness_service() {
        glib::MainContext::new().block_on(async {
            let b = BrightnessService::get_brightness().await;
            println!("Brightness: {:.1}%", b);
            assert!(b >= 5.0 && b <= 100.0);

            BrightnessService::set_brightness(85.0);
            let b2 = BrightnessService::get_brightness().await;
            assert_eq!(b2, 85.0);

            let ns = BrightnessService::get_night_shift().await;
            println!("Night Shift enabled: {}", ns);
        });
    }
}
