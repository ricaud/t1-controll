use crate::model::{RgbDevice, RgbMode, RgbStatus, ZoneUpdate};
use crate::openrgb::OpenRgbClient;
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub enum RgbCommand {
    Connect { host: String, port: u16 },
    Disconnect,
    Refresh,
    SetDemo(bool),
    Frame(Vec<ZoneUpdate>),
    Shutdown,
}

pub struct RgbService {
    tx: Sender<RgbCommand>,
    status: Arc<Mutex<RgbStatus>>,
    join: Option<JoinHandle<()>>,
}

impl RgbService {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(RgbStatus::default()));
        let worker_status = status.clone();
        let join = thread::spawn(move || worker(rx, worker_status));
        Self {
            tx,
            status,
            join: Some(join),
        }
    }

    pub fn status(&self) -> RgbStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }

    pub fn connect(&self, host: &str, port: u16) {
        let _ = self.tx.send(RgbCommand::Connect {
            host: host.to_string(),
            port,
        });
    }

    pub fn disconnect(&self) {
        let _ = self.tx.send(RgbCommand::Disconnect);
    }

    pub fn refresh(&self) {
        let _ = self.tx.send(RgbCommand::Refresh);
    }

    pub fn set_demo(&self, demo: bool) {
        let _ = self.tx.send(RgbCommand::SetDemo(demo));
    }

    pub fn push_frame(&self, updates: Vec<ZoneUpdate>) {
        if !updates.is_empty() {
            let _ = self.tx.send(RgbCommand::Frame(updates));
        }
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.send(RgbCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Default for RgbService {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RgbService {
    fn drop(&mut self) {
        let _ = self.tx.send(RgbCommand::Shutdown);
    }
}

fn read_devices(client: &mut OpenRgbClient) -> anyhow::Result<Vec<RgbDevice>> {
    let count = client.controller_count()?;
    let mut devices = Vec::new();
    for index in 0..count {
        devices.push(client.controller_data(index)?);
    }
    Ok(devices)
}

fn worker(rx: Receiver<RgbCommand>, status: Arc<Mutex<RgbStatus>>) {
    let mut client: Option<OpenRgbClient> = None;
    let mut devices: Vec<RgbDevice> = Vec::new();
    let mut custom_modes: HashSet<u32> = HashSet::new();
    let mut demo = false;
    let mut frames_sent = 0u64;

    loop {
        let command = rx.recv_timeout(Duration::from_millis(100));
        let mut dirty = false;
        match command {
            Ok(RgbCommand::Connect { host, port }) => {
                dirty = true;
                if demo {
                    let mut guard = status.lock().unwrap();
                    guard.mode = RgbMode::Simulated;
                    guard.connected = false;
                    guard.connecting = false;
                    guard.error = Some("Demo mode is enabled".into());
                    continue;
                }
                client = None;
                devices.clear();
                custom_modes.clear();
                {
                    let mut guard = status.lock().unwrap();
                    guard.host = host.clone();
                    guard.port = port;
                    guard.connecting = true;
                    guard.connected = false;
                    guard.error = None;
                    guard.mode = RgbMode::OpenRgb;
                }
                match OpenRgbClient::connect(&host, port, Duration::from_millis(2500)) {
                    Ok(mut new_client) => {
                        if let Err(error) = new_client.set_client_name("T1 Control") {
                            let mut guard = status.lock().unwrap();
                            guard.error = Some(error.to_string());
                            guard.connecting = false;
                        } else {
                            match read_devices(&mut new_client) {
                                Ok(found) => {
                                    devices = found;
                                    client = Some(new_client);
                                    let mut guard = status.lock().unwrap();
                                    guard.connected = true;
                                    guard.connecting = false;
                                    guard.error = None;
                                    guard.devices = devices.clone();
                                }
                                Err(error) => {
                                    let mut guard = status.lock().unwrap();
                                    guard.connected = false;
                                    guard.connecting = false;
                                    guard.error = Some(error.to_string());
                                }
                            }
                        }
                    }
                    Err(error) => {
                        let mut guard = status.lock().unwrap();
                        guard.connected = false;
                        guard.connecting = false;
                        guard.error = Some(error.to_string());
                    }
                }
            }
            Ok(RgbCommand::Disconnect) => {
                dirty = true;
                client = None;
                devices.clear();
                custom_modes.clear();
                let mut guard = status.lock().unwrap();
                guard.connected = false;
                guard.connecting = false;
                guard.error = None;
                guard.devices.clear();
            }
            Ok(RgbCommand::Refresh) => {
                dirty = true;
                if let Some(active) = client.as_mut() {
                    match read_devices(active) {
                        Ok(found) => {
                            devices = found;
                            custom_modes.clear();
                            let mut guard = status.lock().unwrap();
                            guard.devices = devices.clone();
                            guard.error = None;
                        }
                        Err(error) => {
                            let mut guard = status.lock().unwrap();
                            guard.error = Some(error.to_string());
                        }
                    }
                }
            }
            Ok(RgbCommand::SetDemo(enabled)) => {
                dirty = true;
                demo = enabled;
                if demo {
                    client = None;
                    devices.clear();
                    custom_modes.clear();
                    let mut guard = status.lock().unwrap();
                    guard.mode = RgbMode::Simulated;
                    guard.connected = false;
                    guard.connecting = false;
                    guard.error = None;
                    guard.devices.clear();
                } else {
                    let mut guard = status.lock().unwrap();
                    guard.mode = RgbMode::OpenRgb;
                    guard.error = None;
                }
            }
            Ok(RgbCommand::Frame(updates)) => {
                if !demo && client.is_some() {
                    let active = client.as_mut().unwrap();
                    let mut failure = None;
                    for update in updates {
                        if !custom_modes.contains(&update.device_index) {
                            if let Err(error) = active.set_custom_mode(update.device_index) {
                                failure = Some(error.to_string());
                                break;
                            }
                            custom_modes.insert(update.device_index);
                        }
                        if let Err(error) = active.update_zone_leds(
                            update.device_index,
                            update.zone_index,
                            &update.colors,
                        ) {
                            failure = Some(error.to_string());
                            break;
                        }
                        frames_sent += 1;
                    }
                    if let Some(error) = failure {
                        client = None;
                        custom_modes.clear();
                        let mut guard = status.lock().unwrap();
                        guard.connected = false;
                        guard.error = Some(error);
                    } else {
                        dirty = true;
                    }
                }
            }
            Ok(RgbCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if dirty {
            let mut guard = status.lock().unwrap();
            guard.frames_sent = frames_sent;
            guard.devices = devices.clone();
        }
    }
}
