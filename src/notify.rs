use log::{info, warn};
use notify_rust::{Notification, Urgency};

use crate::{battery_icon, reminder::Alert, system};

const APP_NAME: &str = "Snake Charger";
const APP_ID: &str = "SnakeCharger.App";

pub struct Notify {
    app_id: Option<&'static str>,
}

impl Notify {
    pub fn new() -> Self {
        let app_id = match Self::register() {
            Ok(()) => Some(APP_ID),
            Err(e) => {
                warn!("Toast app id not registered, falling back to default: {e}");
                None
            }
        };
        Self { app_id }
    }

    /// Writes the toast icon to %LOCALAPPDATA% and registers our app id.
    fn register() -> Result<(), String> {
        let dir = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .ok_or("LOCALAPPDATA not set")?
            .join("SnakeCharger");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

        let icon_path = dir.join("toast-icon.png");
        let style = battery_icon::Style {
            outline: [0x80, 0x80, 0x80],
            fill: [0x44, 0xD6, 0x2C],
            bolt: false,
        };
        let size = 64;
        image::save_buffer(
            &icon_path,
            &battery_icon::render(size, 75, style),
            size,
            size,
            image::ColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;

        system::register_app_id(APP_ID, APP_NAME, &icon_path)?;
        info!("Registered toast app id {APP_ID}");
        Ok(())
    }

    fn show(&self, title: &str, device_name: &str, body: &str, urgency: Urgency) {
        let mut n = Notification::new();
        n.summary(title)
            .subtitle(device_name)
            .body(body)
            .urgency(urgency);
        match self.app_id {
            Some(id) => {
                n.app_id(id);
            }
            // Without our app id the toast header says "Windows PowerShell",
            // so name the app in the body instead.
            None => {
                n.summary(&format!("{APP_NAME}: {title}"));
            }
        }
        if let Err(e) = n.show() {
            warn!("Failed to show notification: {e}");
        }
    }

    pub fn battery_low(&self, device_name: &str, level: i32, alert: Alert) {
        let title = if alert.critical {
            format!("Battery critical — {level}%")
        } else {
            format!("Battery low — {level}%")
        };
        let body = match (alert.critical, alert.count) {
            (true, _) => "Plug in the cable now, the mouse will turn off soon.".to_owned(),
            (false, 1) => "Time to plug in the cable.".to_owned(),
            (false, n) => format!("Reminder #{}. Plug in the cable.", n - 1),
        };
        let urgency = if alert.critical {
            // Shown as a Windows "reminder" toast that stays until dismissed.
            Urgency::Critical
        } else {
            Urgency::Normal
        };
        self.show(&title, device_name, &body, urgency);
    }

    pub fn battery_full(&self, device_name: &str) {
        self.show(
            "Fully charged",
            device_name,
            "You can unplug the cable.",
            Urgency::Normal,
        );
    }

    pub fn device_connected(&self, device_name: &str) {
        self.show("Connected", device_name, "", Urgency::Low);
    }

    pub fn device_disconnected(&self, device_name: &str) {
        self.show("Disconnected", device_name, "", Urgency::Low);
    }
}
