use crate::model::{
    DeviceSettings, FanBackendKind, FanBackendPreference, FanChannelReading, FanDeviceInfo,
    FanDiscovery, SensorInfo,
};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub trait FanBackend: Send {
    fn kind(&self) -> FanBackendKind;
    fn scan(&mut self) -> Result<FanDiscovery>;
    fn read(&mut self) -> Result<(Vec<SensorInfo>, Vec<FanChannelReading>)>;
    fn set_duty(&mut self, channel: &str, percent: u8) -> Result<()>;
    fn restore(&mut self) -> Result<()>;
    fn close(&mut self);
}

pub fn create_backend(settings: &DeviceSettings) -> Box<dyn FanBackend> {
    if settings.demo_mode {
        return Box::new(DemoFanBackend::new());
    }
    let kind = match settings.fan_backend {
        FanBackendPreference::Auto => {
            if cfg!(target_os = "linux") {
                FanBackendKind::Sysfs
            } else if cfg!(target_os = "windows") {
                FanBackendKind::Lhm
            } else if cfg!(target_os = "macos") {
                FanBackendKind::Smc
            } else {
                FanBackendKind::None
            }
        }
        FanBackendPreference::Sysfs => FanBackendKind::Sysfs,
        FanBackendPreference::Lhm => FanBackendKind::Lhm,
        FanBackendPreference::Smc => FanBackendKind::Smc,
        FanBackendPreference::None => FanBackendKind::None,
    };
    match kind {
        FanBackendKind::Sysfs if cfg!(target_os = "linux") => Box::new(SysfsFanBackend::default()),
        FanBackendKind::Lhm if cfg!(target_os = "windows") => {
            Box::new(LhmFanBackend::new(settings.lhm_dll_path.clone()))
        }
        FanBackendKind::Smc => Box::new(UnsupportedFanBackend::new(
            FanBackendKind::Smc,
            "macOS does not expose desktop fan headers. Run fan control on the Windows or Linux host that owns the AIO fans.",
        )),
        FanBackendKind::None => Box::new(UnsupportedFanBackend::new(
            FanBackendKind::None,
            "Fan control is disabled. Enable a backend in Devices to set curves.",
        )),
        FanBackendKind::Sysfs => Box::new(UnsupportedFanBackend::new(
            FanBackendKind::Sysfs,
            "The Linux hwmon backend is only available on Linux.",
        )),
        FanBackendKind::Lhm => Box::new(UnsupportedFanBackend::new(
            FanBackendKind::Lhm,
            "LibreHardwareMonitor is Windows-only.",
        )),
        FanBackendKind::Simulated => Box::new(DemoFanBackend::new()),
    }
}

fn read_text(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
}

fn read_number(path: &Path) -> Option<f64> {
    read_text(path).and_then(|text| text.parse::<f64>().ok())
}

// ---------------------------------------------------------------------------
// Linux hwmon
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SysfsChannel {
    label: String,
    rpm_path: Option<PathBuf>,
    enable_path: Option<PathBuf>,
}

struct SysfsSensor {
    id: String,
    label: String,
    source: String,
    path: PathBuf,
}

#[derive(Default)]
struct SysfsFanBackend {
    channels: HashMap<String, SysfsChannel>,
    sensors: Vec<SysfsSensor>,
    saved: HashMap<String, (Option<String>, Option<String>)>,
}

impl SysfsFanBackend {
    fn read_sensors(&self) -> Vec<SensorInfo> {
        self.sensors
            .iter()
            .map(|sensor| SensorInfo {
                id: sensor.id.clone(),
                label: sensor.label.clone(),
                source: sensor.source.clone(),
                value_c: read_number(&sensor.path).map(|value| (value / 1000.0) as f32),
            })
            .collect()
    }
}

impl FanBackend for SysfsFanBackend {
    fn kind(&self) -> FanBackendKind {
        FanBackendKind::Sysfs
    }

    fn scan(&mut self) -> Result<FanDiscovery> {
        self.channels.clear();
        self.sensors.clear();
        let root = Path::new("/sys/class/hwmon");
        if !root.exists() {
            return Ok(FanDiscovery {
                backend: FanBackendKind::Sysfs,
                reason: Some("Linux hwmon is not present.".into()),
                ..FanDiscovery::default()
            });
        }

        for entry in fs::read_dir(root)? {
            let Ok(entry) = entry else { continue };
            let base = entry.path();
            let hw_name = read_text(&base.join("name"))
                .unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
            let Ok(files) = fs::read_dir(&base) else {
                continue;
            };
            for file in files.flatten() {
                let file_name = file.file_name().to_string_lossy().into_owned();
                if let Some(index) = file_name
                    .strip_prefix("temp")
                    .and_then(|name| name.strip_suffix("_input"))
                {
                    let label = read_text(&base.join(format!("temp{index}_label")))
                        .unwrap_or_else(|| format!("{hw_name} temp{index}"));
                    self.sensors.push(SysfsSensor {
                        id: base.join(&file_name).to_string_lossy().into_owned(),
                        label,
                        source: hw_name.clone(),
                        path: base.join(&file_name),
                    });
                }
            }
            for file in fs::read_dir(&base)?.flatten() {
                let file_name = file.file_name().to_string_lossy().into_owned();
                let Some(index) = file_name
                    .strip_prefix("pwm")
                    .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()))
                else {
                    continue;
                };
                let pwm_path = base.join(&file_name);
                if fs::OpenOptions::new().write(true).open(&pwm_path).is_err() {
                    continue;
                }
                let fan_path = base.join(format!("fan{index}_input"));
                let enable_path = base.join(format!("pwm{index}_enable"));
                let fan_label = read_text(&base.join(format!("fan{index}_label")));
                let label = match fan_label {
                    Some(label) => format!("{hw_name} · pwm{index} · {label}"),
                    None => format!("{hw_name} · pwm{index}"),
                };
                self.channels.insert(
                    pwm_path.to_string_lossy().into_owned(),
                    SysfsChannel {
                        label,
                        rpm_path: fan_path.exists().then_some(fan_path),
                        enable_path: enable_path.exists().then_some(enable_path),
                    },
                );
            }
        }

        let channels: Vec<FanDeviceInfo> = self
            .channels
            .iter()
            .map(|(id, channel)| FanDeviceInfo {
                id: id.clone(),
                label: channel.label.clone(),
                detail: id.clone(),
                max_rpm: 2000,
            })
            .collect();
        let controllable = !channels.is_empty();
        Ok(FanDiscovery {
            backend: FanBackendKind::Sysfs,
            available: true,
            controllable,
            reason: (!controllable).then(|| {
                "No writable PWM channels found. Load the Super I/O driver and make pwm files writable (udev rule or root).".into()
            }),
            channels,
            sensors: self.read_sensors(),
        })
    }

    fn read(&mut self) -> Result<(Vec<SensorInfo>, Vec<FanChannelReading>)> {
        let mut channels = Vec::new();
        for (id, channel) in &self.channels {
            let pwm = read_number(Path::new(id));
            let rpm = channel.rpm_path.as_deref().and_then(read_number);
            channels.push(FanChannelReading {
                id: id.clone(),
                label: channel.label.clone(),
                rpm: rpm.map(|value| value.round() as u16),
                duty_percent: pwm.map(|value| (value / 255.0 * 100.0).round() as u8),
            });
        }
        Ok((self.read_sensors(), channels))
    }

    fn set_duty(&mut self, channel: &str, percent: u8) -> Result<()> {
        let Some(info) = self.channels.get(channel) else {
            bail!("unknown hwmon channel: {channel}");
        };
        if !self.saved.contains_key(channel) {
            self.saved.insert(
                channel.to_string(),
                (
                    read_text(Path::new(channel)),
                    info.enable_path.as_deref().and_then(read_text),
                ),
            );
        }
        if let Some(enable) = &info.enable_path {
            if read_text(enable).as_deref() != Some("1") {
                fs::write(enable, "1").with_context(|| format!("enabling {}", enable.display()))?;
            }
        }
        let pwm = (percent.min(100) as f32 / 100.0 * 255.0).round() as u8;
        fs::write(channel, pwm.to_string()).with_context(|| format!("writing {channel}"))?;
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        for (channel, (pwm, enable)) in &self.saved {
            if let Some(info) = self.channels.get(channel) {
                if let (Some(path), Some(value)) = (&info.enable_path, enable) {
                    let _ = fs::write(path, value);
                }
            }
            if let Some(value) = pwm {
                let _ = fs::write(channel, value);
            }
        }
        self.saved.clear();
        Ok(())
    }

    fn close(&mut self) {}
}

// ---------------------------------------------------------------------------
// Windows LibreHardwareMonitor bridge
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

struct LhmFanBackend {
    dll_path: Option<String>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    rx: Option<Receiver<serde_json::Value>>,
    stderr: Arc<Mutex<String>>,
    next_id: Arc<AtomicU64>,
    started: bool,
}

impl LhmFanBackend {
    fn new(dll_path: Option<String>) -> Self {
        Self {
            dll_path,
            child: None,
            stdin: None,
            rx: None,
            stderr: Arc::new(Mutex::new(String::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            started: false,
        }
    }

    fn start(&mut self) -> Result<()> {
        if self.started {
            return Ok(());
        }
        let Some(dll) = self.dll_path.as_ref() else {
            bail!("LibreHardwareMonitorLib.dll path is not set. Set it in Devices.");
        };
        if !Path::new(dll).exists() {
            bail!("LibreHardwareMonitorLib.dll was not found at {dll}");
        }

        let script = std::env::temp_dir()
            .join("t1-control")
            .join("FanBridge.ps1");
        if let Some(parent) = script.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &script,
            include_str!("../resources/fanbridge/FanBridge.ps1"),
        )?;

        let mut command = Command::new("powershell.exe");
        command
            .arg("-NoLogo")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(&script)
            .arg("-DllPath")
            .arg(dll)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(target_os = "windows")]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW

        let mut child = command
            .spawn()
            .context("starting LibreHardwareMonitor bridge")?;
        self.stdin = child.stdin.take();
        let stdout = child.stdout.take().context("capturing bridge stdout")?;
        let stderr = child.stderr.take().context("capturing bridge stderr")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(|line| line.ok()) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                    if tx.send(value).is_err() {
                        break;
                    }
                }
            }
        });
        let stderr_shared = self.stderr.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(|line| line.ok()) {
                if let Ok(mut text) = stderr_shared.lock() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&line);
                }
            }
        });
        self.rx = Some(rx);
        self.child = Some(child);
        self.started = true;
        Ok(())
    }

    fn request(
        &mut self,
        mut command: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        self.start()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        command["id"] = serde_json::json!(id);
        let stdin = self.stdin.as_mut().context("bridge stdin closed")?;
        writeln!(stdin, "{command}")?;
        stdin.flush()?;
        let rx = self.rx.as_ref().context("bridge is not running")?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let stderr = self
                    .stderr
                    .lock()
                    .map(|text| text.clone())
                    .unwrap_or_default();
                bail!(
                    "LibreHardwareMonitor bridge did not respond{}",
                    if stderr.is_empty() {
                        String::new()
                    } else {
                        format!(": {stderr}")
                    }
                );
            }
            match rx.recv_timeout(remaining) {
                Ok(value) => {
                    if value.get("id").and_then(|id| id.as_u64()) == Some(id) {
                        if value.get("ok").and_then(|ok| ok.as_bool()) == Some(true) {
                            return Ok(value);
                        }
                        bail!(
                            "{}",
                            value
                                .get("error")
                                .and_then(|error| error.as_str())
                                .unwrap_or("LibreHardwareMonitor bridge error")
                        );
                    }
                }
                Err(_) => {
                    let stderr = self
                        .stderr
                        .lock()
                        .map(|text| text.clone())
                        .unwrap_or_default();
                    bail!(
                        "LibreHardwareMonitor bridge did not respond{}",
                        if stderr.is_empty() {
                            String::new()
                        } else {
                            format!(": {stderr}")
                        }
                    );
                }
            }
        }
    }
}

fn json_array_or_one(value: Option<&serde_json::Value>) -> Vec<serde_json::Value> {
    match value {
        Some(serde_json::Value::Array(items)) => items.clone(),
        Some(value) if !value.is_null() => vec![value.clone()],
        _ => Vec::new(),
    }
}

impl FanBackend for LhmFanBackend {
    fn kind(&self) -> FanBackendKind {
        FanBackendKind::Lhm
    }

    fn scan(&mut self) -> Result<FanDiscovery> {
        match self.request(serde_json::json!({"cmd":"list"}), Duration::from_secs(10)) {
            Ok(value) => {
                let channels: Vec<FanDeviceInfo> = json_array_or_one(value.get("channels"))
                    .into_iter()
                    .filter_map(|channel| {
                        Some(FanDeviceInfo {
                            id: channel.get("id")?.as_str()?.to_string(),
                            label: channel.get("label")?.as_str()?.to_string(),
                            detail: format!("LHM · {}", channel.get("id")?.as_str()?),
                            max_rpm: channel
                                .get("maxRpm")
                                .and_then(|value| value.as_u64())
                                .unwrap_or(2000) as u16,
                        })
                    })
                    .collect();
                let sensors: Vec<SensorInfo> = json_array_or_one(value.get("sensors"))
                    .into_iter()
                    .filter_map(|sensor| {
                        Some(SensorInfo {
                            id: sensor.get("id")?.as_str()?.to_string(),
                            label: sensor.get("label")?.as_str()?.to_string(),
                            source: "LibreHardwareMonitor".into(),
                            value_c: sensor
                                .get("valueC")
                                .and_then(|value| value.as_f64())
                                .map(|value| value as f32),
                        })
                    })
                    .collect();
                let controllable = !channels.is_empty();
                Ok(FanDiscovery {
                    backend: FanBackendKind::Lhm,
                    available: true,
                    controllable,
                    reason: (!controllable).then(|| {
                        "LibreHardwareMonitor reported no controllable fan headers.".into()
                    }),
                    channels,
                    sensors,
                })
            }
            Err(error) => Ok(FanDiscovery {
                backend: FanBackendKind::Lhm,
                available: false,
                controllable: false,
                reason: Some(error.to_string()),
                channels: Vec::new(),
                sensors: Vec::new(),
            }),
        }
    }

    fn read(&mut self) -> Result<(Vec<SensorInfo>, Vec<FanChannelReading>)> {
        let value = self.request(serde_json::json!({"cmd":"list"}), Duration::from_secs(10))?;
        let sensors = json_array_or_one(value.get("sensors"))
            .into_iter()
            .filter_map(|sensor| {
                Some(SensorInfo {
                    id: sensor.get("id")?.as_str()?.to_string(),
                    label: sensor.get("label")?.as_str()?.to_string(),
                    source: "LibreHardwareMonitor".into(),
                    value_c: sensor
                        .get("valueC")
                        .and_then(|value| value.as_f64())
                        .map(|value| value as f32),
                })
            })
            .collect();
        let channels = json_array_or_one(value.get("channels"))
            .into_iter()
            .filter_map(|channel| {
                Some(FanChannelReading {
                    id: channel.get("id")?.as_str()?.to_string(),
                    label: channel.get("label")?.as_str()?.to_string(),
                    rpm: channel
                        .get("rpm")
                        .and_then(|value| value.as_u64())
                        .map(|value| value as u16),
                    duty_percent: channel
                        .get("duty")
                        .and_then(|value| value.as_u64())
                        .map(|value| value as u8),
                })
            })
            .collect();
        Ok((sensors, channels))
    }

    fn set_duty(&mut self, channel: &str, percent: u8) -> Result<()> {
        self.request(
            serde_json::json!({"cmd":"set","channel":channel,"percent":percent}),
            Duration::from_secs(10),
        )?;
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        if self.started {
            let _ = self.request(
                serde_json::json!({"cmd":"restore"}),
                Duration::from_secs(10),
            );
        }
        Ok(())
    }

    fn close(&mut self) {
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.started = false;
    }
}

// ---------------------------------------------------------------------------
// Demo
// ---------------------------------------------------------------------------

pub struct DemoFanBackend {
    duties: HashMap<String, u8>,
    started: Instant,
    channels: Vec<FanDeviceInfo>,
}

impl DemoFanBackend {
    pub fn new() -> Self {
        let mut duties = HashMap::new();
        duties.insert("demo-aio-1".into(), 42);
        duties.insert("demo-aio-2".into(), 42);
        duties.insert("demo-top".into(), 32);
        Self {
            duties,
            started: Instant::now(),
            channels: vec![
                FanDeviceInfo {
                    id: "demo-aio-1".into(),
                    label: "AIO fan 1 · Aspect 12".into(),
                    detail: "Simulated · side radiator lower".into(),
                    max_rpm: 2000,
                },
                FanDeviceInfo {
                    id: "demo-aio-2".into(),
                    label: "AIO fan 2 · Aspect 12".into(),
                    detail: "Simulated · side radiator upper".into(),
                    max_rpm: 2000,
                },
                FanDeviceInfo {
                    id: "demo-top".into(),
                    label: "Top exhaust · 120 mm".into(),
                    detail: "Simulated · top panel".into(),
                    max_rpm: 2000,
                },
            ],
        }
    }

    fn temperature(&self, offset: f32, period: f32) -> f32 {
        let seconds = self.started.elapsed().as_secs_f32();
        let value = 46.0 + offset + 7.0 * (seconds / period).sin();
        (value * 10.0).round() / 10.0
    }

    fn sensors(&self) -> Vec<SensorInfo> {
        vec![
            SensorInfo {
                id: "demo-cpu".into(),
                label: "CPU package (demo)".into(),
                source: "Simulation".into(),
                value_c: Some(self.temperature(0.0, 9.0)),
            },
            SensorInfo {
                id: "demo-gpu".into(),
                label: "GPU edge (demo)".into(),
                source: "Simulation".into(),
                value_c: Some(self.temperature(4.0, 13.0)),
            },
            SensorInfo {
                id: "demo-coolant".into(),
                label: "AIO coolant (demo)".into(),
                source: "Simulation".into(),
                value_c: Some(self.temperature(-12.0, 24.0)),
            },
        ]
    }
}

impl Default for DemoFanBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FanBackend for DemoFanBackend {
    fn kind(&self) -> FanBackendKind {
        FanBackendKind::Simulated
    }

    fn scan(&mut self) -> Result<FanDiscovery> {
        Ok(FanDiscovery {
            backend: FanBackendKind::Simulated,
            available: true,
            controllable: true,
            reason: Some("Simulated hardware — no fan headers are touched.".into()),
            channels: self.channels.clone(),
            sensors: self.sensors(),
        })
    }

    fn read(&mut self) -> Result<(Vec<SensorInfo>, Vec<FanChannelReading>)> {
        let channels = self
            .channels
            .iter()
            .map(|channel| {
                let duty = *self.duties.get(&channel.id).unwrap_or(&0);
                FanChannelReading {
                    id: channel.id.clone(),
                    label: channel.label.clone(),
                    rpm: Some((duty as f32 / 100.0 * channel.max_rpm as f32).round() as u16),
                    duty_percent: Some(duty),
                }
            })
            .collect();
        Ok((self.sensors(), channels))
    }

    fn set_duty(&mut self, channel: &str, percent: u8) -> Result<()> {
        self.duties.insert(channel.to_string(), percent.min(100));
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        self.duties.insert("demo-aio-1".into(), 42);
        self.duties.insert("demo-aio-2".into(), 42);
        self.duties.insert("demo-top".into(), 32);
        Ok(())
    }

    fn close(&mut self) {}
}

// ---------------------------------------------------------------------------
// Unsupported
// ---------------------------------------------------------------------------

pub struct UnsupportedFanBackend {
    kind: FanBackendKind,
    reason: String,
}

impl UnsupportedFanBackend {
    pub fn new(kind: FanBackendKind, reason: &str) -> Self {
        Self {
            kind,
            reason: reason.into(),
        }
    }
}

impl FanBackend for UnsupportedFanBackend {
    fn kind(&self) -> FanBackendKind {
        self.kind
    }

    fn scan(&mut self) -> Result<FanDiscovery> {
        Ok(FanDiscovery {
            backend: self.kind,
            available: false,
            controllable: false,
            reason: Some(self.reason.clone()),
            channels: Vec::new(),
            sensors: Vec::new(),
        })
    }

    fn read(&mut self) -> Result<(Vec<SensorInfo>, Vec<FanChannelReading>)> {
        Ok((Vec::new(), Vec::new()))
    }

    fn set_duty(&mut self, _channel: &str, _percent: u8) -> Result<()> {
        Err(anyhow!(self.reason.clone()))
    }

    fn restore(&mut self) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) {}
}
