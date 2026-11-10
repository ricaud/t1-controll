use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CONFIG_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };
    pub const WHITE: Rgb = Rgb {
        r: 255,
        g: 255,
        b: 255,
    };

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        Rgb {
            r: (a.r as f32 + (b.r as f32 - a.r as f32) * t).round() as u8,
            g: (a.g as f32 + (b.g as f32 - a.g as f32) * t).round() as u8,
            b: (a.b as f32 + (b.b as f32 - a.b as f32) * t).round() as u8,
        }
    }

    pub fn scale(self, factor: f32) -> Rgb {
        let factor = factor.clamp(0.0, 1.0);
        Rgb {
            r: (self.r as f32 * factor).round() as u8,
            g: (self.g as f32 * factor).round() as u8,
            b: (self.b as f32 * factor).round() as u8,
        }
    }

    pub fn luminance(self) -> f32 {
        (0.2126 * self.r as f32 + 0.7152 * self.g as f32 + 0.0722 * self.b as f32) / 255.0
    }

    pub fn to_array(self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }

    pub fn from_array(value: [u8; 3]) -> Rgb {
        Rgb::new(value[0], value[1], value[2])
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Pattern {
    Off,
    Static,
    Gradient,
    Breathing,
    Cycle,
    Rainbow,
    #[default]
    Wave,
    Meteor,
    Temperature,
}

impl Pattern {
    pub fn label(self) -> &'static str {
        match self {
            Pattern::Off => "Off",
            Pattern::Static => "Static",
            Pattern::Gradient => "Gradient",
            Pattern::Breathing => "Breathing",
            Pattern::Cycle => "Colour cycle",
            Pattern::Rainbow => "Rainbow",
            Pattern::Wave => "Wave",
            Pattern::Meteor => "Meteor",
            Pattern::Temperature => "Temperature",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Pattern::Off => "Keep the zone dark",
            Pattern::Static => "Single palette colour",
            Pattern::Gradient => "Palette across the zone",
            Pattern::Breathing => "Slow brightness pulse",
            Pattern::Cycle => "Palette rotates",
            Pattern::Rainbow => "Full-spectrum sweep",
            Pattern::Wave => "Travelling wave",
            Pattern::Meteor => "Comet with a fading tail",
            Pattern::Temperature => "Cyan to red by CPU sensor",
        }
    }

    pub const ALL: [Pattern; 9] = [
        Pattern::Static,
        Pattern::Gradient,
        Pattern::Breathing,
        Pattern::Cycle,
        Pattern::Rainbow,
        Pattern::Wave,
        Pattern::Meteor,
        Pattern::Temperature,
        Pattern::Off,
    ];
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    #[default]
    Forward,
    Reverse,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Page {
    #[default]
    Build,
    Lighting,
    Cooling,
    Devices,
    Profiles,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneKind {
    Pump,
    Fan,
    #[default]
    Strip,
    Block,
}

impl ZoneKind {
    pub fn label(self) -> &'static str {
        match self {
            ZoneKind::Pump => "Pump block",
            ZoneKind::Fan => "Fan",
            ZoneKind::Strip => "Strip",
            ZoneKind::Block => "Block",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Placement {
    PumpBlock,
    AioFan1,
    AioFan2,
    TopFan,
    BottomFan1,
    BottomFan2,
    #[default]
    BottomStrip,
    TopStrip,
    FrontStrip,
    RearStrip,
    SideDiffuser,
    GpuBlock,
}

impl Placement {
    pub const ALL: [Placement; 12] = [
        Placement::PumpBlock,
        Placement::AioFan1,
        Placement::AioFan2,
        Placement::TopFan,
        Placement::BottomFan1,
        Placement::BottomFan2,
        Placement::BottomStrip,
        Placement::TopStrip,
        Placement::FrontStrip,
        Placement::RearStrip,
        Placement::SideDiffuser,
        Placement::GpuBlock,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Placement::PumpBlock => "CPU block · 67 mm ring",
            Placement::AioFan1 => "Side radiator · lower fan",
            Placement::AioFan2 => "Side radiator · upper fan",
            Placement::TopFan => "Top panel · 120 mm",
            Placement::BottomFan1 => "Bottom panel · front",
            Placement::BottomFan2 => "Bottom panel · rear",
            Placement::BottomStrip => "Front floor · under GPU",
            Placement::TopStrip => "Top rail · behind mesh",
            Placement::FrontStrip => "Front panel · vertical",
            Placement::RearStrip => "Rear panel · above I/O",
            Placement::SideDiffuser => "Side panel · perimeter",
            Placement::GpuBlock => "GPU block · side",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FanBackendPreference {
    #[default]
    Auto,
    Sysfs,
    Lhm,
    Smc,
    None,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Effect {
    pub pattern: Pattern,
    pub palette: Vec<Rgb>,
    pub brightness: u8,
    pub speed: u8,
    pub direction: Direction,
}

impl Default for Effect {
    fn default() -> Self {
        Self {
            pattern: Pattern::Wave,
            palette: vec![
                Rgb::new(34, 211, 238),
                Rgb::new(129, 140, 248),
                Rgb::new(192, 132, 252),
            ],
            brightness: 75,
            speed: 40,
            direction: Direction::Forward,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ZoneEffect {
    pub pattern: Option<Pattern>,
    pub palette: Option<Vec<Rgb>>,
    pub brightness: Option<u8>,
    pub speed: Option<u8>,
    pub direction: Option<Direction>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Mapping {
    pub device_index: u32,
    pub device_name: String,
    pub zone_index: u32,
    pub zone_name: String,
    pub led_count: u16,
}

impl Default for Mapping {
    fn default() -> Self {
        Self {
            device_index: 0,
            device_name: String::new(),
            zone_index: 0,
            zone_name: String::new(),
            led_count: 1,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SceneZone {
    pub id: String,
    pub template_id: String,
    pub name: String,
    pub kind: ZoneKind,
    pub placement: Placement,
    pub led_count: u16,
    pub builtin: bool,
    pub enabled: bool,
    pub effect: ZoneEffect,
    pub mapping: Option<Mapping>,
}

impl Default for SceneZone {
    fn default() -> Self {
        Self {
            id: "zone".into(),
            template_id: "custom-strip".into(),
            name: "RGB zone".into(),
            kind: ZoneKind::Strip,
            placement: Placement::BottomStrip,
            led_count: 12,
            builtin: false,
            enabled: true,
            effect: ZoneEffect::default(),
            mapping: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LightingConfig {
    pub enabled: bool,
    pub global: Effect,
    pub zones: Vec<SceneZone>,
}

impl Default for LightingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            global: Effect::default(),
            zones: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct FanCurvePoint {
    pub temp_c: f32,
    pub rpm: u16,
}

impl Default for FanCurvePoint {
    fn default() -> Self {
        Self {
            temp_c: 40.0,
            rpm: 700,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct FanCurve {
    pub sensor: String,
    pub points: Vec<FanCurvePoint>,
    pub hysteresis_c: f32,
    pub min_rpm: u16,
    pub max_rpm: u16,
    pub max_step_up_rpm_per_sec: u16,
    pub max_step_down_rpm_per_sec: u16,
}

impl Default for FanCurve {
    fn default() -> Self {
        Self {
            sensor: "cpu".into(),
            points: vec![
                FanCurvePoint {
                    temp_c: 30.0,
                    rpm: 550,
                },
                FanCurvePoint {
                    temp_c: 45.0,
                    rpm: 700,
                },
                FanCurvePoint {
                    temp_c: 60.0,
                    rpm: 950,
                },
                FanCurvePoint {
                    temp_c: 70.0,
                    rpm: 1250,
                },
                FanCurvePoint {
                    temp_c: 80.0,
                    rpm: 1700,
                },
                FanCurvePoint {
                    temp_c: 90.0,
                    rpm: 2000,
                },
            ],
            hysteresis_c: 3.0,
            min_rpm: 500,
            max_rpm: 2000,
            max_step_up_rpm_per_sec: 400,
            max_step_down_rpm_per_sec: 150,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct FanChannel {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub max_rpm: u16,
    pub curve: FanCurve,
    pub device_id: Option<String>,
    pub device_label: Option<String>,
}

impl Default for FanChannel {
    fn default() -> Self {
        Self {
            id: "fan".into(),
            name: "Fan".into(),
            enabled: true,
            max_rpm: 2000,
            curve: FanCurve::default(),
            device_id: None,
            device_label: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CoolingConfig {
    pub enabled: bool,
    pub fail_safe_rpm: u16,
    pub channels: Vec<FanChannel>,
}

impl Default for CoolingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            fail_safe_rpm: 2000,
            channels: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub lighting: LightingConfig,
    pub cooling: CoolingConfig,
}

impl Default for Profile {
    fn default() -> Self {
        crate::presets::default_profile()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenRgbSettings {
    pub host: String,
    pub port: u16,
    pub auto_connect: bool,
}

impl Default for OpenRgbSettings {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 6742,
            auto_connect: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceSettings {
    pub openrgb: OpenRgbSettings,
    pub fan_backend: FanBackendPreference,
    pub lhm_dll_path: Option<String>,
    pub demo_mode: bool,
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            openrgb: OpenRgbSettings::default(),
            fan_backend: FanBackendPreference::Auto,
            lhm_dll_path: None,
            demo_mode: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct UiSettings {
    pub show_panels: bool,
    pub page: Page,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            show_panels: false,
            page: Page::Build,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub version: u32,
    pub active_profile_id: String,
    pub profiles: Vec<Profile>,
    pub devices: DeviceSettings,
    pub ui: UiSettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        crate::presets::default_config()
    }
}

impl AppConfig {
    pub fn active_profile(&self) -> &Profile {
        self.profiles
            .iter()
            .find(|profile| profile.id == self.active_profile_id)
            .or_else(|| self.profiles.first())
            .expect("validated config always has a profile")
    }

    pub fn active_profile_mut(&mut self) -> &mut Profile {
        let id = self
            .profiles
            .iter()
            .find(|profile| profile.id == self.active_profile_id)
            .map(|profile| profile.id.clone())
            .or_else(|| self.profiles.first().map(|profile| profile.id.clone()))
            .unwrap_or_else(|| "t1-balanced".into());
        self.profiles
            .iter_mut()
            .find(|profile| profile.id == id)
            .expect("validated config always has a profile")
    }

    pub fn touch(&mut self) {
        self.active_profile_mut().updated_at = now_secs();
    }

    pub fn validate(&mut self) {
        self.version = CONFIG_VERSION;
        if self.profiles.is_empty() {
            self.profiles.push(crate::presets::default_profile());
        }
        if !self
            .profiles
            .iter()
            .any(|profile| profile.id == self.active_profile_id)
        {
            self.active_profile_id = self.profiles[0].id.clone();
        }
        if self.profiles[0].id.is_empty() {
            self.profiles[0].id = "t1-balanced".into();
        }
        self.devices.openrgb.port = self.devices.openrgb.port.max(1);
        if self.devices.openrgb.host.trim().is_empty() {
            self.devices.openrgb.host = "127.0.0.1".into();
        }
        for profile in &mut self.profiles {
            if profile.id.is_empty() {
                profile.id = "profile".into();
            }
            if profile.name.trim().is_empty() {
                profile.name = "T1 profile".into();
            }
            profile.lighting.global.brightness = profile.lighting.global.brightness.min(100);
            profile.lighting.global.speed = profile.lighting.global.speed.min(100);
            if profile.lighting.global.palette.is_empty() {
                profile.lighting.global.palette = vec![Rgb::WHITE];
            }
            for zone in &mut profile.lighting.zones {
                zone.led_count = zone.led_count.clamp(1, 512);
                if let Some(palette) = &mut zone.effect.palette {
                    if palette.is_empty() {
                        *palette = profile.lighting.global.palette.clone();
                    }
                }
                if let Some(mapping) = &mut zone.mapping {
                    mapping.led_count = mapping.led_count.max(1);
                }
            }
            profile.cooling.fail_safe_rpm = profile.cooling.fail_safe_rpm.min(6000);
            for channel in &mut profile.cooling.channels {
                channel.max_rpm = channel.max_rpm.clamp(100, 6000);
                channel.curve.max_rpm = channel.max_rpm;
                channel.curve.min_rpm = channel.curve.min_rpm.min(channel.curve.max_rpm);
                channel.curve.hysteresis_c = channel.curve.hysteresis_c.clamp(0.0, 20.0);
                channel.curve.max_step_up_rpm_per_sec =
                    channel.curve.max_step_up_rpm_per_sec.clamp(10, 5000);
                channel.curve.max_step_down_rpm_per_sec =
                    channel.curve.max_step_down_rpm_per_sec.clamp(10, 5000);
                channel.curve.normalize();
            }
        }
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq)]
pub struct SensorInfo {
    pub id: String,
    pub label: String,
    pub source: String,
    pub value_c: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanDeviceInfo {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub max_rpm: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FanChannelReading {
    pub id: String,
    pub label: String,
    pub rpm: Option<u16>,
    pub duty_percent: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FanBackendKind {
    Sysfs,
    Lhm,
    Smc,
    Simulated,
    #[default]
    None,
}

impl FanBackendKind {
    pub fn label(self) -> &'static str {
        match self {
            FanBackendKind::Sysfs => "hwmon",
            FanBackendKind::Lhm => "LHM",
            FanBackendKind::Smc => "SMC",
            FanBackendKind::Simulated => "DEMO",
            FanBackendKind::None => "OFF",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FanDiscovery {
    pub backend: FanBackendKind,
    pub available: bool,
    pub controllable: bool,
    pub reason: Option<String>,
    pub channels: Vec<FanDeviceInfo>,
    pub sensors: Vec<SensorInfo>,
}

#[derive(Clone, Debug)]
pub struct FanChannelStatus {
    pub id: String,
    pub name: String,
    pub label: String,
    pub enabled: bool,
    pub mapped: bool,
    pub rpm: Option<u16>,
    pub duty_percent: Option<u8>,
    pub target_rpm: Option<u16>,
    pub sensor_label: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct FansStatus {
    pub backend: FanBackendKind,
    pub available: bool,
    pub controllable: bool,
    pub demo: bool,
    pub applied: bool,
    pub reason: Option<String>,
    pub devices: Vec<FanDeviceInfo>,
    pub sensors: Vec<SensorInfo>,
    pub channels: Vec<FanChannelStatus>,
    pub last_tick: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RgbMode {
    #[default]
    OpenRgb,
    Simulated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceZone {
    pub index: u32,
    pub name: String,
    pub zone_type: u32,
    pub led_count: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbDevice {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub location: String,
    pub zones: Vec<DeviceZone>,
}

#[derive(Clone, Debug, Default)]
pub struct RgbStatus {
    pub mode: RgbMode,
    pub connected: bool,
    pub connecting: bool,
    pub host: String,
    pub port: u16,
    pub error: Option<String>,
    pub devices: Vec<RgbDevice>,
    pub frames_sent: u64,
}

#[derive(Clone, Debug)]
pub struct ZoneUpdate {
    pub device_index: u32,
    pub zone_index: u32,
    pub colors: Vec<Rgb>,
}
