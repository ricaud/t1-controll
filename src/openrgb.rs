use crate::model::{DeviceZone, Rgb, RgbDevice};
use anyhow::{anyhow, bail, Context, Result};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const MAGIC: &[u8; 4] = b"ORGB";
const HEADER_SIZE: usize = 16;
const MAX_PACKET_SIZE: usize = 8 * 1024 * 1024;

pub const PACKET_REQUEST_CONTROLLER_COUNT: u32 = 0;
pub const PACKET_REQUEST_CONTROLLER_DATA: u32 = 1;
pub const PACKET_SET_CLIENT_NAME: u32 = 50;
pub const PACKET_UPDATE_LEDS: u32 = 1050;
pub const PACKET_UPDATE_ZONE_LEDS: u32 = 1051;
pub const PACKET_SET_CUSTOM_MODE: u32 = 1100;

pub struct OpenRgbClient {
    stream: TcpStream,
}

impl OpenRgbClient {
    pub fn connect(host: &str, port: u16, timeout: Duration) -> Result<Self> {
        let addresses = (host, port)
            .to_socket_addrs()
            .with_context(|| format!("resolving {host}:{port}"))?;
        let mut last_error = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, timeout) {
                Ok(stream) => {
                    stream.set_nodelay(true).ok();
                    stream.set_read_timeout(Some(timeout)).ok();
                    stream.set_write_timeout(Some(timeout)).ok();
                    return Ok(Self { stream });
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(anyhow!(
            "cannot connect to OpenRGB at {host}:{port}: {}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "no address resolved".into())
        ))
    }

    fn write_packet(&mut self, device_id: u32, packet_id: u32, payload: &[u8]) -> Result<()> {
        let mut header = [0u8; HEADER_SIZE];
        header[0..4].copy_from_slice(MAGIC);
        header[4..8].copy_from_slice(&device_id.to_le_bytes());
        header[8..12].copy_from_slice(&packet_id.to_le_bytes());
        header[12..16].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        self.stream.write_all(&header)?;
        self.stream.write_all(payload)?;
        self.stream.flush()?;
        Ok(())
    }

    fn read_packet(&mut self) -> Result<(u32, u32, Vec<u8>)> {
        let mut header = [0u8; HEADER_SIZE];
        self.stream.read_exact(&mut header)?;
        if &header[0..4] != MAGIC {
            bail!("bad OpenRGB packet magic");
        }
        let device_id = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let packet_id = u32::from_le_bytes(header[8..12].try_into().unwrap());
        let size = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
        if size > MAX_PACKET_SIZE {
            bail!("OpenRGB packet exceeds the 8 MiB limit");
        }
        let mut payload = vec![0u8; size];
        self.stream.read_exact(&mut payload)?;
        Ok((device_id, packet_id, payload))
    }

    fn request(&mut self, device_id: u32, packet_id: u32, payload: &[u8]) -> Result<Vec<u8>> {
        self.write_packet(device_id, packet_id, payload)?;
        for _ in 0..64 {
            let (reply_device, reply_id, reply) = self.read_packet()?;
            if reply_id == packet_id && reply_device == device_id {
                return Ok(reply);
            }
        }
        bail!("OpenRGB sent no matching reply for packet {packet_id}")
    }

    pub fn controller_count(&mut self) -> Result<u32> {
        let payload = self.request(0, PACKET_REQUEST_CONTROLLER_COUNT, &[])?;
        if payload.len() < 4 {
            bail!("OpenRGB returned an empty controller count");
        }
        Ok(u32::from_le_bytes(payload[0..4].try_into().unwrap()))
    }

    pub fn controller_data(&mut self, index: u32) -> Result<RgbDevice> {
        let payload = self.request(index, PACKET_REQUEST_CONTROLLER_DATA, &[])?;
        let mut device = parse_controller_data(&payload)?;
        device.index = index;
        Ok(device)
    }

    pub fn set_client_name(&mut self, name: &str) -> Result<()> {
        self.write_packet(0, PACKET_SET_CLIENT_NAME, name.as_bytes())
    }

    pub fn set_custom_mode(&mut self, device_index: u32) -> Result<()> {
        self.write_packet(device_index, PACKET_SET_CUSTOM_MODE, &[])
    }

    pub fn update_zone_leds(
        &mut self,
        device_index: u32,
        zone_index: u32,
        colors: &[Rgb],
    ) -> Result<()> {
        let payload = build_zone_led_payload(zone_index, colors);
        self.write_packet(device_index, PACKET_UPDATE_ZONE_LEDS, &payload)
    }

    pub fn update_all_leds(&mut self, device_index: u32, colors: &[Rgb]) -> Result<()> {
        let payload = build_all_led_payload(colors);
        self.write_packet(device_index, PACKET_UPDATE_LEDS, &payload)
    }
}

struct Reader<'a> {
    buffer: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(buffer: &'a [u8]) -> Self {
        Self { buffer, offset: 0 }
    }

    fn need(&self, bytes: usize) -> Result<()> {
        if self.offset + bytes > self.buffer.len() {
            bail!("truncated OpenRGB controller description");
        }
        Ok(())
    }

    fn u16(&mut self) -> Result<u16> {
        self.need(2)?;
        let value = u16::from_le_bytes(
            self.buffer[self.offset..self.offset + 2]
                .try_into()
                .unwrap(),
        );
        self.offset += 2;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32> {
        self.need(4)?;
        let value = u32::from_le_bytes(
            self.buffer[self.offset..self.offset + 4]
                .try_into()
                .unwrap(),
        );
        self.offset += 4;
        Ok(value)
    }

    fn skip(&mut self, bytes: usize) -> Result<()> {
        self.need(bytes)?;
        self.offset += bytes;
        Ok(())
    }

    fn string(&mut self) -> Result<String> {
        let length = self.u16()? as usize;
        self.need(length)?;
        let raw = &self.buffer[self.offset..self.offset + length];
        self.offset += length;
        let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
        Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
    }
}

pub fn parse_controller_data(payload: &[u8]) -> Result<RgbDevice> {
    let mut reader = Reader::new(payload);
    let declared = reader.u32()? as usize;
    if declared != payload.len() {
        bail!(
            "OpenRGB controller description size mismatch (declared {declared}, received {})",
            payload.len()
        );
    }

    let _device_type = reader.u32()?;
    let name = reader.string()?;
    let description = reader.string()?;
    let _version = reader.string()?;
    let _serial = reader.string()?;
    let location = reader.string()?;

    let mode_count = reader.u16()?;
    let _active_mode = reader.u32()?;
    for _ in 0..mode_count {
        let _name = reader.string()?;
        reader.u32()?; // value
        reader.u32()?; // flags
        reader.u32()?; // speed min
        reader.u32()?; // speed max
        reader.u32()?; // colors min
        reader.u32()?; // colors max
        reader.u32()?; // speed
        reader.u32()?; // direction
        reader.u32()?; // color mode
        let colors = reader.u16()? as usize;
        reader.skip(colors * 4)?;
    }

    let zone_count = reader.u16()?;
    let mut zones = Vec::with_capacity(zone_count as usize);
    for index in 0..zone_count {
        let zone_name = reader.string()?;
        let zone_type = reader.u32()?;
        let _leds_min = reader.u32()?;
        let _leds_max = reader.u32()?;
        let led_count = reader.u32()?;
        let matrix_size = reader.u16()? as usize;
        reader.skip(matrix_size)?;
        zones.push(DeviceZone {
            index: index as u32,
            name: zone_name,
            zone_type,
            led_count: led_count.min(u16::MAX as u32) as u16,
        });
    }

    let led_count = reader.u16()?;
    for _ in 0..led_count {
        let _name = reader.string()?;
        reader.u32()?;
    }

    let color_count = reader.u16()?;
    reader.skip(color_count as usize * 4)?;

    Ok(RgbDevice {
        index: 0,
        name,
        description,
        location,
        zones,
    })
}

pub fn build_zone_led_payload(zone_index: u32, colors: &[Rgb]) -> Vec<u8> {
    let size = 4 + 4 + 2 + colors.len() * 4;
    let mut payload = Vec::with_capacity(size);
    payload.extend_from_slice(&(size as u32).to_le_bytes());
    payload.extend_from_slice(&zone_index.to_le_bytes());
    payload.extend_from_slice(&(colors.len() as u16).to_le_bytes());
    for color in colors {
        payload.extend_from_slice(&[color.r, color.g, color.b, 0]);
    }
    payload
}

pub fn build_all_led_payload(colors: &[Rgb]) -> Vec<u8> {
    let size = 4 + 2 + colors.len() * 4;
    let mut payload = Vec::with_capacity(size);
    payload.extend_from_slice(&(size as u32).to_le_bytes());
    payload.extend_from_slice(&(colors.len() as u16).to_le_bytes());
    for color in colors {
        payload.extend_from_slice(&[color.r, color.g, color.b, 0]);
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    fn u16b(value: u16) -> Vec<u8> {
        value.to_le_bytes().to_vec()
    }
    fn u32b(value: u32) -> Vec<u8> {
        value.to_le_bytes().to_vec()
    }
    fn string(value: &str) -> Vec<u8> {
        let mut data = value.as_bytes().to_vec();
        data.push(0);
        let mut out = u16b(data.len() as u16);
        out.extend_from_slice(&data);
        out
    }
    fn mode(name: &str) -> Vec<u8> {
        let mut out = string(name);
        out.extend_from_slice(&u32b(0));
        out.extend_from_slice(&u32b(0));
        out.extend_from_slice(&u32b(0));
        out.extend_from_slice(&u32b(100));
        out.extend_from_slice(&u32b(1));
        out.extend_from_slice(&u32b(1));
        out.extend_from_slice(&u32b(0));
        out.extend_from_slice(&u32b(0));
        out.extend_from_slice(&u32b(0));
        out.extend_from_slice(&u16b(1));
        out.extend_from_slice(&[255, 255, 255, 0]);
        out
    }
    fn zone(name: &str, zone_type: u32, leds: u32) -> Vec<u8> {
        let mut out = string(name);
        out.extend_from_slice(&u32b(zone_type));
        out.extend_from_slice(&u32b(leds));
        out.extend_from_slice(&u32b(leds));
        out.extend_from_slice(&u32b(leds));
        out.extend_from_slice(&u16b(0));
        out
    }

    fn controller_payload() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&u32b(1));
        body.extend_from_slice(&string("Test MSI Controller"));
        body.extend_from_slice(&string("Mock OpenRGB device"));
        body.extend_from_slice(&string("1.0"));
        body.extend_from_slice(&string("SERIAL-1"));
        body.extend_from_slice(&string("PCI:0"));
        body.extend_from_slice(&u16b(1));
        body.extend_from_slice(&u32b(0));
        body.extend_from_slice(&mode("Direct"));
        body.extend_from_slice(&u16b(2));
        body.extend_from_slice(&zone("Pump Block", 1, 6));
        body.extend_from_slice(&zone("AIO Fans", 2, 8));
        body.extend_from_slice(&u16b(0));
        body.extend_from_slice(&u16b(0));
        let mut payload = u32b(body.len() as u32 + 4);
        payload.extend_from_slice(&body);
        payload
    }

    fn spawn_mock() -> (
        TcpListener,
        mpsc::Receiver<(u32, u32, Vec<u8>)>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let probe = listener.try_clone().unwrap();
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut socket, _) = probe.accept().unwrap();
            loop {
                let mut header = [0u8; 16];
                if socket.read_exact(&mut header).is_err() {
                    break;
                }
                let device = u32::from_le_bytes(header[4..8].try_into().unwrap());
                let packet = u32::from_le_bytes(header[8..12].try_into().unwrap());
                let size = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
                let mut payload = vec![0u8; size];
                if socket.read_exact(&mut payload).is_err() {
                    break;
                }
                let _ = tx.send((device, packet, payload.clone()));
                if packet == PACKET_REQUEST_CONTROLLER_COUNT {
                    let reply = u32b(1);
                    let mut header = [0u8; 16];
                    header[0..4].copy_from_slice(MAGIC);
                    header[4..8].copy_from_slice(&device.to_le_bytes());
                    header[8..12].copy_from_slice(&packet.to_le_bytes());
                    header[12..16].copy_from_slice(&(reply.len() as u32).to_le_bytes());
                    let _ = socket.write_all(&header);
                    let _ = socket.write_all(&reply);
                } else if packet == PACKET_REQUEST_CONTROLLER_DATA {
                    let reply = controller_payload();
                    let mut header = [0u8; 16];
                    header[0..4].copy_from_slice(MAGIC);
                    header[4..8].copy_from_slice(&device.to_le_bytes());
                    header[8..12].copy_from_slice(&packet.to_le_bytes());
                    header[12..16].copy_from_slice(&(reply.len() as u32).to_le_bytes());
                    let _ = socket.write_all(&header);
                    let _ = socket.write_all(&reply);
                }
            }
        });
        (listener, rx, handle)
    }

    #[test]
    fn payload_layouts_match_openrgb() {
        let payload = build_zone_led_payload(2, &[Rgb::new(255, 0, 0), Rgb::new(0, 128, 255)]);
        assert_eq!(payload.len(), 4 + 4 + 2 + 8);
        assert_eq!(
            u32::from_le_bytes(payload[0..4].try_into().unwrap()) as usize,
            payload.len()
        );
        assert_eq!(u32::from_le_bytes(payload[4..8].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(payload[8..10].try_into().unwrap()), 2);
        assert_eq!(&payload[10..], &[255, 0, 0, 0, 0, 128, 255, 0]);

        let all = build_all_led_payload(&[Rgb::new(1, 2, 3)]);
        assert_eq!(
            u32::from_le_bytes(all[0..4].try_into().unwrap()) as usize,
            all.len()
        );
        assert_eq!(u16::from_le_bytes(all[4..6].try_into().unwrap()), 1);
        assert_eq!(&all[6..], &[1, 2, 3, 0]);
    }

    #[test]
    fn parses_controller_description() {
        let device = parse_controller_data(&controller_payload()).unwrap();
        assert_eq!(device.name, "Test MSI Controller");
        assert_eq!(device.location, "PCI:0");
        assert_eq!(device.zones.len(), 2);
        assert_eq!(device.zones[0].name, "Pump Block");
        assert_eq!(device.zones[0].led_count, 6);
        assert_eq!(device.zones[1].led_count, 8);
    }

    #[test]
    fn rejects_bad_description() {
        let mut payload = controller_payload();
        payload[0] = payload[0].wrapping_add(1);
        assert!(parse_controller_data(&payload).is_err());
        assert!(parse_controller_data(&payload[..20]).is_err());
    }

    #[test]
    fn speaks_the_protocol_to_a_mock_server() {
        let (listener, rx, handle) = spawn_mock();
        let port = listener.local_addr().unwrap().port();
        let mut client = OpenRgbClient::connect("127.0.0.1", port, Duration::from_secs(2)).unwrap();
        assert_eq!(client.controller_count().unwrap(), 1);
        let device = client.controller_data(0).unwrap();
        assert_eq!(device.zones[1].name, "AIO Fans");

        client.set_client_name("T1 Control test").unwrap();
        client.set_custom_mode(0).unwrap();
        client
            .update_zone_leds(0, 1, &[Rgb::new(255, 0, 0), Rgb::new(0, 255, 0)])
            .unwrap();

        let mut saw_name = false;
        let mut saw_update = false;
        for _ in 0..8 {
            let Ok((_, packet, payload)) = rx.recv_timeout(Duration::from_secs(2)) else {
                break;
            };
            if packet == PACKET_SET_CLIENT_NAME {
                assert_eq!(payload, b"T1 Control test");
                saw_name = true;
            }
            if packet == PACKET_UPDATE_ZONE_LEDS {
                assert_eq!(payload[0..4], (payload.len() as u32).to_le_bytes());
                assert_eq!(&payload[10..], &[255, 0, 0, 0, 0, 255, 0, 0]);
                saw_update = true;
            }
            if saw_name && saw_update {
                break;
            }
        }
        assert!(saw_name);
        assert!(saw_update);
        drop(client);
        let _ = handle.join();
    }
}
