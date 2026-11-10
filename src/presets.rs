use crate::model::{
    now_secs, AppConfig, CoolingConfig, Direction, Effect, FanChannel, FanCurve, FanCurvePoint,
    LightingConfig, Placement, Profile, Rgb, SceneZone, ZoneEffect, ZoneKind,
};

pub struct PalettePreset {
    pub id: &'static str,
    pub name: &'static str,
    pub colors: &'static [Rgb],
}

pub const PALETTES: &[PalettePreset] = &[
    PalettePreset {
        id: "t1-teal",
        name: "T1 Teal",
        colors: &[
            Rgb::new(34, 211, 238),
            Rgb::new(129, 140, 248),
            Rgb::new(192, 132, 252),
        ],
    },
    PalettePreset {
        id: "ember",
        name: "Ember",
        colors: &[
            Rgb::new(249, 115, 22),
            Rgb::new(239, 68, 68),
            Rgb::new(124, 45, 18),
        ],
    },
    PalettePreset {
        id: "glacier",
        name: "Glacier",
        colors: &[
            Rgb::new(224, 242, 254),
            Rgb::new(56, 189, 248),
            Rgb::new(29, 78, 216),
        ],
    },
    PalettePreset {
        id: "acid",
        name: "Acid",
        colors: &[
            Rgb::new(163, 230, 53),
            Rgb::new(34, 197, 94),
            Rgb::new(15, 118, 110),
        ],
    },
    PalettePreset {
        id: "mono",
        name: "Warm white",
        colors: &[Rgb::new(255, 247, 237)],
    },
    PalettePreset {
        id: "vapor",
        name: "Vapor",
        colors: &[
            Rgb::new(244, 114, 182),
            Rgb::new(167, 139, 250),
            Rgb::new(34, 211, 238),
        ],
    },
];

pub struct CoolingPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub points: &'static [(f32, u16)],
    pub hysteresis_c: f32,
    pub min_rpm: u16,
    pub max_rpm: u16,
    pub up_rpm_per_sec: u16,
    pub down_rpm_per_sec: u16,
}

impl CoolingPreset {
    pub fn curve(&self) -> FanCurve {
        FanCurve {
            sensor: "cpu".into(),
            points: self
                .points
                .iter()
                .map(|(temp_c, rpm)| FanCurvePoint {
                    temp_c: *temp_c,
                    rpm: *rpm,
                })
                .collect(),
            hysteresis_c: self.hysteresis_c,
            min_rpm: self.min_rpm,
            max_rpm: self.max_rpm,
            max_step_up_rpm_per_sec: self.up_rpm_per_sec,
            max_step_down_rpm_per_sec: self.down_rpm_per_sec,
        }
    }
}

/// Starting points, not lab measurements. The T1's mesh panels restrict
/// airflow, so these ramp earlier than an open-bench curve.
pub const COOLING_PRESETS: &[CoolingPreset] = &[
    CoolingPreset {
        id: "t1-s24-silent",
        name: "T1 · S24 Silent",
        description: "Quiet idle with a late ramp. Listen for turbulence above ~1100 RPM.",
        points: &[
            (35.0, 500),
            (50.0, 600),
            (65.0, 800),
            (75.0, 1100),
            (85.0, 1600),
            (95.0, 2000),
        ],
        hysteresis_c: 4.0,
        min_rpm: 500,
        max_rpm: 2000,
        up_rpm_per_sec: 300,
        down_rpm_per_sec: 120,
    },
    CoolingPreset {
        id: "t1-s24-balanced",
        name: "T1 · S24 Balanced",
        description: "Recommended default for a 240 mm AIO in the T1 sandwich layout.",
        points: &[
            (30.0, 550),
            (45.0, 700),
            (60.0, 950),
            (70.0, 1250),
            (80.0, 1700),
            (90.0, 2000),
        ],
        hysteresis_c: 3.0,
        min_rpm: 500,
        max_rpm: 2000,
        up_rpm_per_sec: 400,
        down_rpm_per_sec: 150,
    },
    CoolingPreset {
        id: "t1-s24-performance",
        name: "T1 · S24 Performance",
        description: "Earlier ramp for sustained CPU loads in a cramped chassis.",
        points: &[
            (30.0, 700),
            (45.0, 950),
            (55.0, 1300),
            (65.0, 1700),
            (75.0, 2000),
            (90.0, 2000),
        ],
        hysteresis_c: 3.0,
        min_rpm: 500,
        max_rpm: 2000,
        up_rpm_per_sec: 500,
        down_rpm_per_sec: 200,
    },
    CoolingPreset {
        id: "t1-s24-coolant",
        name: "T1 · S24 Coolant",
        description: "Only if your AIO exposes a coolant sensor. Smoother than CPU-based control.",
        points: &[(30.0, 500), (35.0, 700), (40.0, 1100), (45.0, 2000)],
        hysteresis_c: 2.0,
        min_rpm: 500,
        max_rpm: 2000,
        up_rpm_per_sec: 250,
        down_rpm_per_sec: 80,
    },
];

pub struct ZoneTemplate {
    pub template_id: &'static str,
    pub name: &'static str,
    pub kind: ZoneKind,
    pub placement: Placement,
    pub led_count: u16,
    pub description: &'static str,
    pub builtin: bool,
}

pub const ZONE_TEMPLATES: &[ZoneTemplate] = &[
    ZoneTemplate {
        template_id: "lumen-s24-pump",
        name: "Lumen S24 pump block",
        kind: ZoneKind::Pump,
        placement: Placement::PumpBlock,
        led_count: 6,
        description: "Six addressable LEDs behind the Lumen S24 pump cap.",
        builtin: true,
    },
    ZoneTemplate {
        template_id: "aspect-12-aio-1",
        name: "AIO fan 1 · Aspect 12",
        kind: ZoneKind::Fan,
        placement: Placement::AioFan1,
        led_count: 8,
        description: "Lower Fractal Aspect 12 RGB fan on the radiator.",
        builtin: true,
    },
    ZoneTemplate {
        template_id: "aspect-12-aio-2",
        name: "AIO fan 2 · Aspect 12",
        kind: ZoneKind::Fan,
        placement: Placement::AioFan2,
        led_count: 8,
        description: "Upper Fractal Aspect 12 RGB fan on the radiator.",
        builtin: true,
    },
    ZoneTemplate {
        template_id: "top-fan-120",
        name: "Top exhaust fan",
        kind: ZoneKind::Fan,
        placement: Placement::TopFan,
        led_count: 8,
        description: "Common 120 mm T1 add-on above the motherboard.",
        builtin: false,
    },
    ZoneTemplate {
        template_id: "bottom-fan-1",
        name: "Bottom intake 1",
        kind: ZoneKind::Fan,
        placement: Placement::BottomFan1,
        led_count: 8,
        description: "Bottom-mounted 120 mm intake.",
        builtin: false,
    },
    ZoneTemplate {
        template_id: "bottom-fan-2",
        name: "Bottom intake 2",
        kind: ZoneKind::Fan,
        placement: Placement::BottomFan2,
        led_count: 8,
        description: "Second bottom-mounted 120 mm intake.",
        builtin: false,
    },
    ZoneTemplate {
        template_id: "strip-bottom",
        name: "Bottom front strip",
        kind: ZoneKind::Strip,
        placement: Placement::BottomStrip,
        led_count: 21,
        description: "Diffused ARGB strip along the front floor.",
        builtin: false,
    },
    ZoneTemplate {
        template_id: "strip-top",
        name: "Top rail strip",
        kind: ZoneKind::Strip,
        placement: Placement::TopStrip,
        led_count: 21,
        description: "Washes the top mesh from inside.",
        builtin: false,
    },
    ZoneTemplate {
        template_id: "strip-front",
        name: "Front vertical strip",
        kind: ZoneKind::Strip,
        placement: Placement::FrontStrip,
        led_count: 18,
        description: "Vertical accent behind the front panel.",
        builtin: false,
    },
    ZoneTemplate {
        template_id: "strip-rear",
        name: "Rear exhaust strip",
        kind: ZoneKind::Strip,
        placement: Placement::RearStrip,
        led_count: 12,
        description: "Small rear accent for ambient bounce.",
        builtin: false,
    },
    ZoneTemplate {
        template_id: "diffuser-side",
        name: "Side panel diffuser",
        kind: ZoneKind::Strip,
        placement: Placement::SideDiffuser,
        led_count: 24,
        description: "Perimeter diffuser along the GPU-side mesh.",
        builtin: false,
    },
    ZoneTemplate {
        template_id: "gpu-block",
        name: "GPU block accent",
        kind: ZoneKind::Block,
        placement: Placement::GpuBlock,
        led_count: 6,
        description: "Optional RGB accent for a water-cooled GPU.",
        builtin: false,
    },
];

pub fn placement_id(placement: Placement) -> String {
    match placement {
        Placement::PumpBlock => "pump-block",
        Placement::AioFan1 => "aio-fan-1",
        Placement::AioFan2 => "aio-fan-2",
        Placement::TopFan => "top-fan",
        Placement::BottomFan1 => "bottom-fan-1",
        Placement::BottomFan2 => "bottom-fan-2",
        Placement::BottomStrip => "bottom-strip",
        Placement::TopStrip => "top-strip",
        Placement::FrontStrip => "front-strip",
        Placement::RearStrip => "rear-strip",
        Placement::SideDiffuser => "side-diffuser",
        Placement::GpuBlock => "gpu-block",
    }
    .to_string()
}

pub fn zone_from_template(template: &ZoneTemplate, id: String) -> SceneZone {
    SceneZone {
        id,
        template_id: template.template_id.into(),
        name: template.name.into(),
        kind: template.kind,
        placement: template.placement,
        led_count: template.led_count,
        builtin: template.builtin,
        enabled: true,
        effect: ZoneEffect::default(),
        mapping: None,
    }
}

pub fn builtin_zones() -> Vec<SceneZone> {
    ZONE_TEMPLATES
        .iter()
        .filter(|template| template.builtin)
        .map(|template| zone_from_template(template, placement_id(template.placement)))
        .collect()
}

fn balanced_curve() -> FanCurve {
    COOLING_PRESETS
        .iter()
        .find(|preset| preset.id == "t1-s24-balanced")
        .map(CoolingPreset::curve)
        .unwrap_or_default()
}

fn channel(id: &str, name: &str) -> FanChannel {
    FanChannel {
        id: id.into(),
        name: name.into(),
        enabled: true,
        max_rpm: 2000,
        curve: balanced_curve(),
        device_id: None,
        device_label: None,
    }
}

pub fn default_profile() -> Profile {
    Profile {
        id: "t1-balanced".into(),
        name: "T1 · Lumen S24".into(),
        description: "Stock three-zone Lumen S24 build with the recommended balanced curve.".into(),
        created_at: 0,
        updated_at: 0,
        lighting: LightingConfig {
            enabled: true,
            global: Effect {
                pattern: crate::model::Pattern::Wave,
                palette: PALETTES[0].colors.to_vec(),
                brightness: 75,
                speed: 40,
                direction: Direction::Forward,
            },
            zones: builtin_zones(),
        },
        cooling: CoolingConfig {
            enabled: false,
            fail_safe_rpm: 2000,
            channels: vec![
                channel("aio-fan-1", "AIO fan 1 · Aspect 12"),
                channel("aio-fan-2", "AIO fan 2 · Aspect 12"),
            ],
        },
    }
}

pub fn default_config() -> AppConfig {
    let profile = default_profile();
    AppConfig {
        version: crate::model::CONFIG_VERSION,
        active_profile_id: profile.id.clone(),
        profiles: vec![profile],
        devices: crate::model::DeviceSettings::default(),
        ui: crate::model::UiSettings::default(),
    }
}

pub fn touch_profile(profile: &mut Profile) {
    profile.updated_at = now_secs();
}

pub fn demo_config() -> AppConfig {
    let mut config = default_config();
    config.devices.demo_mode = true;
    config.ui.show_panels = false;
    let profile = config.active_profile_mut();
    profile.cooling.enabled = true;
    if let Some(channel) = profile.cooling.channels.get_mut(0) {
        channel.device_id = Some("demo-aio-1".into());
        channel.device_label = Some("AIO fan 1 · Aspect 12".into());
    }
    if let Some(channel) = profile.cooling.channels.get_mut(1) {
        channel.device_id = Some("demo-aio-2".into());
        channel.device_label = Some("AIO fan 2 · Aspect 12".into());
    }
    config
}
