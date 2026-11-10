use crate::fan::{duty_for_rpm, resolve_sensor, CurveFollower};
use crate::fan_backend::{create_backend, FanBackend};
use crate::model::{
    now_secs, CoolingConfig, DeviceSettings, FanChannelReading, FanChannelStatus, FanDiscovery,
    FansStatus, SensorInfo,
};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub enum FanCommand {
    Configure {
        devices: DeviceSettings,
        cooling: CoolingConfig,
    },
    Scan,
    Apply,
    Disable,
    Shutdown,
}

pub struct FanService {
    tx: Sender<FanCommand>,
    status: Arc<Mutex<FansStatus>>,
    join: Option<JoinHandle<()>>,
}

impl FanService {
    pub fn new(devices: DeviceSettings, cooling: CoolingConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(FansStatus::default()));
        let worker_status = status.clone();
        let join = thread::spawn(move || {
            let mut worker = FanWorker::new(devices, cooling, worker_status);
            worker.run(rx);
        });
        Self {
            tx,
            status,
            join: Some(join),
        }
    }

    pub fn status(&self) -> FansStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }

    pub fn configure(&self, devices: DeviceSettings, cooling: CoolingConfig) {
        let _ = self.tx.send(FanCommand::Configure { devices, cooling });
    }

    pub fn scan(&self) {
        let _ = self.tx.send(FanCommand::Scan);
    }

    pub fn apply(&self) {
        let _ = self.tx.send(FanCommand::Apply);
    }

    pub fn disable(&self) {
        let _ = self.tx.send(FanCommand::Disable);
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.send(FanCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for FanService {
    fn drop(&mut self) {
        let _ = self.tx.send(FanCommand::Shutdown);
    }
}

struct FanWorker {
    status: Arc<Mutex<FansStatus>>,
    backend: Box<dyn FanBackend>,
    backend_key: String,
    discovery: Option<FanDiscovery>,
    sensors: Vec<SensorInfo>,
    readings: HashMap<String, FanChannelReading>,
    cooling: CoolingConfig,
    devices: DeviceSettings,
    followers: HashMap<String, CurveFollower>,
    targets: HashMap<String, u16>,
    errors: HashMap<String, String>,
    seen: HashSet<String>,
    applied: bool,
    last_tick: Option<Instant>,
    read_error: Option<String>,
    action_error: Option<String>,
}

fn backend_key(devices: &DeviceSettings) -> String {
    format!(
        "{:?}|{:?}|{:?}",
        devices.demo_mode, devices.fan_backend, devices.lhm_dll_path
    )
}

impl FanWorker {
    fn new(
        devices: DeviceSettings,
        cooling: CoolingConfig,
        status: Arc<Mutex<FansStatus>>,
    ) -> Self {
        let backend = create_backend(&devices);
        let mut worker = Self {
            status,
            backend_key: backend_key(&devices),
            backend,
            discovery: None,
            sensors: Vec::new(),
            readings: HashMap::new(),
            cooling,
            devices,
            followers: HashMap::new(),
            targets: HashMap::new(),
            errors: HashMap::new(),
            seen: HashSet::new(),
            applied: false,
            last_tick: None,
            read_error: None,
            action_error: None,
        };
        worker.sync_followers();
        worker.publish();
        worker
    }

    fn run(&mut self, rx: Receiver<FanCommand>) {
        let mut last_publish = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(FanCommand::Configure { devices, cooling }) => {
                    self.configure(devices, cooling);
                }
                Ok(FanCommand::Scan) => self.scan(),
                Ok(FanCommand::Apply) => self.apply(),
                Ok(FanCommand::Disable) => self.disable(),
                Ok(FanCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.shutdown();
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if self.applied
                && self
                    .last_tick
                    .map(|last| last.elapsed() >= Duration::from_secs(1))
                    .unwrap_or(true)
            {
                self.tick();
            }
            if last_publish.elapsed() >= Duration::from_millis(250) {
                self.publish();
                last_publish = Instant::now();
            }
        }
    }

    fn configure(&mut self, devices: DeviceSettings, cooling: CoolingConfig) {
        let key = backend_key(&devices);
        if key != self.backend_key {
            if self.applied {
                self.disable();
            }
            self.backend.close();
            self.backend = create_backend(&devices);
            self.backend_key = key;
            self.discovery = None;
            self.sensors.clear();
            self.readings.clear();
            self.followers.clear();
            self.targets.clear();
            self.errors.clear();
            self.seen.clear();
        }
        self.devices = devices;
        self.cooling = cooling;
        self.sync_followers();
        if !self.cooling.enabled && self.applied {
            self.disable();
        }
        self.publish();
    }

    fn sync_followers(&mut self) {
        for channel in &self.cooling.channels {
            if let Some(follower) = self.followers.get_mut(&channel.id) {
                follower.set_curve(channel.curve.clone());
            } else {
                self.followers.insert(
                    channel.id.clone(),
                    CurveFollower::new(channel.curve.clone(), None),
                );
            }
        }
        let ids: HashSet<String> = self
            .cooling
            .channels
            .iter()
            .map(|channel| channel.id.clone())
            .collect();
        self.followers.retain(|id, _| ids.contains(id));
    }

    fn scan(&mut self) {
        match self.backend.scan() {
            Ok(discovery) => {
                self.sensors = discovery.sensors.clone();
                self.discovery = Some(discovery);
            }
            Err(error) => {
                self.discovery = Some(FanDiscovery {
                    backend: self.backend.kind(),
                    reason: Some(error.to_string()),
                    ..FanDiscovery::default()
                });
                self.sensors.clear();
            }
        }
        self.publish();
    }

    fn apply(&mut self) {
        if self.applied {
            return;
        }
        if self.discovery.is_none() {
            self.scan();
        }
        if !self
            .discovery
            .as_ref()
            .map(|discovery| discovery.controllable)
            .unwrap_or(false)
            && !self.devices.demo_mode
        {
            self.action_error = self
                .discovery
                .as_ref()
                .and_then(|discovery| discovery.reason.clone())
                .or_else(|| Some("Fan control is unavailable on this system".into()));
            self.errors.clear();
            self.publish();
            return;
        }
        let mapped = self
            .cooling
            .channels
            .iter()
            .filter(|channel| channel.enabled && channel.device_id.is_some())
            .count();
        if mapped == 0 {
            self.action_error =
                Some("Map at least one fan channel to a hardware output first.".into());
            self.publish();
            return;
        }
        self.action_error = None;
        self.applied = true;
        self.last_tick = None;
        self.errors.clear();
        self.seen.clear();
        self.tick();
        if self.read_error.is_some() {
            self.applied = false;
            let _ = self.backend.restore();
        }
        self.publish();
    }

    fn disable(&mut self) {
        let was_applied = self.applied;
        self.applied = false;
        self.last_tick = None;
        if was_applied {
            let _ = self.backend.restore();
        }
        for follower in self.followers.values_mut() {
            follower.reset(None);
        }
        self.targets.clear();
        self.errors.clear();
        self.seen.clear();
        self.read_error = None;
        self.action_error = None;
        self.publish();
    }

    fn tick(&mut self) {
        if !self.applied {
            return;
        }
        let now = Instant::now();
        let dt = self
            .last_tick
            .map(|last| now.duration_since(last).as_secs_f32().clamp(0.1, 10.0))
            .unwrap_or(1.0);

        let (sensors, readings) = match self.backend.read() {
            Ok(result) => {
                self.read_error = None;
                result
            }
            Err(error) => {
                let message = error.to_string();
                self.read_error = Some(message.clone());
                let channels = self.cooling.channels.clone();
                for channel in channels.iter().filter(|channel| channel.enabled) {
                    if let Some(device) = &channel.device_id {
                        let duty =
                            duty_for_rpm(self.cooling.fail_safe_rpm as f32, channel.max_rpm, 20);
                        if self.backend.set_duty(device, duty).is_err() {
                            self.errors.insert(
                                channel.id.clone(),
                                format!("{message} — fail-safe write failed"),
                            );
                        } else {
                            self.errors.insert(
                                channel.id.clone(),
                                format!("{message} — fail-safe speed requested"),
                            );
                        }
                    }
                }
                self.last_tick = Some(now);
                self.publish();
                return;
            }
        };

        self.sensors = sensors;
        self.readings = readings
            .into_iter()
            .map(|reading| (reading.id.clone(), reading))
            .collect();

        let channels = self.cooling.channels.clone();
        for channel in &channels {
            if !channel.enabled {
                continue;
            }
            let Some(device) = channel.device_id.as_ref() else {
                self.errors.insert(
                    channel.id.clone(),
                    "Not mapped to a hardware fan output".into(),
                );
                continue;
            };
            let sensor = resolve_sensor(&channel.curve.sensor, &self.sensors);
            let target = match sensor {
                Some(sensor) if sensor.value_c.is_some() => {
                    self.seen.insert(channel.id.clone());
                    self.errors.remove(&channel.id);
                    let follower = self
                        .followers
                        .entry(channel.id.clone())
                        .or_insert_with(|| CurveFollower::new(channel.curve.clone(), None));
                    follower.update(sensor.value_c.unwrap(), dt)
                }
                _ => {
                    if self.seen.contains(&channel.id) {
                        self.errors.insert(
                            channel.id.clone(),
                            "Temperature sensor lost — fail-safe speed applied".into(),
                        );
                        self.cooling.fail_safe_rpm as f32
                    } else {
                        self.errors
                            .insert(channel.id.clone(), "No matching temperature sensor".into());
                        continue;
                    }
                }
            };
            let duty = duty_for_rpm(target, channel.max_rpm, 20);
            self.targets
                .insert(channel.id.clone(), target.round() as u16);
            if let Err(error) = self.backend.set_duty(device, duty) {
                self.errors.insert(channel.id.clone(), error.to_string());
            }
        }
        self.last_tick = Some(now);
        self.publish();
    }

    fn shutdown(&mut self) {
        self.disable();
        self.backend.close();
    }

    fn publish(&self) {
        let mut channels = Vec::new();
        for channel in &self.cooling.channels {
            let reading = channel
                .device_id
                .as_ref()
                .and_then(|id| self.readings.get(id));
            let sensor_label =
                resolve_sensor(&channel.curve.sensor, &self.sensors).map(|sensor| sensor.label);
            channels.push(FanChannelStatus {
                id: channel.id.clone(),
                name: channel.name.clone(),
                label: reading
                    .map(|reading| reading.label.clone())
                    .or_else(|| channel.device_label.clone())
                    .unwrap_or_else(|| "Unmapped".into()),
                enabled: channel.enabled,
                mapped: channel.device_id.is_some(),
                rpm: reading.and_then(|reading| reading.rpm),
                duty_percent: reading.and_then(|reading| reading.duty_percent),
                target_rpm: self.targets.get(&channel.id).copied(),
                sensor_label,
                error: self.errors.get(&channel.id).cloned(),
            });
        }
        let discovery = self.discovery.clone().unwrap_or_default();
        let status = FansStatus {
            backend: self.backend.kind(),
            available: discovery.available,
            controllable: discovery.controllable,
            demo: self.devices.demo_mode,
            applied: self.applied,
            reason: self
                .read_error
                .clone()
                .or(self.action_error.clone())
                .or(discovery.reason)
                .or_else(|| (!self.applied).then(|| "Fan control is not applied".into())),
            devices: discovery.channels,
            sensors: self.sensors.clone(),
            channels,
            last_tick: self.last_tick.map(|_| now_secs()),
        };
        if let Ok(mut guard) = self.status.lock() {
            *guard = status;
        }
    }
}
