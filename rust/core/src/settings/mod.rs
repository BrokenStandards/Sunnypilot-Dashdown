//! Per-device settings projection. `DeviceSettings` is the subset of `Device`
//! the settings UI edits (auto-sync, file selection, retention, auto-delete),
//! kept separate from the connection fields (name/IPs/port/password/active_mode)
//! which are edited via `AppCore::update_device`. Exposed across UniFFI (M8) via
//! `AppCore::get_settings`/`set_settings`.

use crate::model::{Device, FileSelection};

/// The editable, non-connection settings of a device.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DeviceSettings {
    pub auto_sync: bool,
    pub file_selection: FileSelection,
    pub retention_max_minutes: Option<i64>,
    pub auto_delete_from_comma: bool,
    pub auto_delete_min_age_min: i64,
}

impl Device {
    /// Project this device's settings subset.
    pub fn settings(&self) -> DeviceSettings {
        DeviceSettings {
            auto_sync: self.auto_sync,
            file_selection: self.file_selection,
            retention_max_minutes: self.retention_max_minutes,
            auto_delete_from_comma: self.auto_delete_from_comma,
            auto_delete_min_age_min: self.auto_delete_min_age_min,
        }
    }

    /// Apply a settings update in place; connection fields are left untouched.
    pub fn apply_settings(&mut self, s: DeviceSettings) {
        self.auto_sync = s.auto_sync;
        self.file_selection = s.file_selection;
        self.retention_max_minutes = s.retention_max_minutes;
        self.auto_delete_from_comma = s.auto_delete_from_comma;
        self.auto_delete_min_age_min = s.auto_delete_min_age_min;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ConnMode;

    fn device() -> Device {
        Device {
            id: 1,
            name: "dev".into(),
            dongle_label: None,
            hotspot_ip: "192.168.43.1".into(),
            wifi_ip: None,
            port: 3923,
            active_mode: ConnMode::Hotspot,
            password: Some("secret".into()),
            auto_sync: false,
            file_selection: FileSelection::previews_only(),
            retention_max_minutes: None,
            auto_delete_from_comma: false,
            auto_delete_min_age_min: 60,
        }
    }

    #[test]
    fn settings_round_trip_leaves_connection_fields() {
        let mut d = device();
        let new = DeviceSettings {
            auto_sync: true,
            file_selection: FileSelection::everything(),
            retention_max_minutes: Some(120),
            auto_delete_from_comma: true,
            auto_delete_min_age_min: 30,
        };
        d.apply_settings(new.clone());
        // Settings updated...
        assert_eq!(d.settings(), new);
        // ...connection identity untouched.
        assert_eq!(d.name, "dev");
        assert_eq!(d.port, 3923);
        assert_eq!(d.password.as_deref(), Some("secret"));
        assert_eq!(d.active_mode, ConnMode::Hotspot);
    }
}
