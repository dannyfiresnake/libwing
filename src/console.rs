use std::collections::HashMap;
use std::net::{TcpStream, UdpSocket};
use std::io::{Read, Write};
use std::time::Duration;
use std::sync::{Mutex, Arc};

use crate::{Result, Error, WingResponse};
use crate::node::{WingNodeDef, WingNodeData};
use crate::propmap::NAME_TO_DEF;

pub enum Meter {
    Channel(u8),
    Aux(u8),
    Bus(u8),
    Main(u8),
    Matrix(u8),
    Dca(u8),
    Fx(u8),
    Source(u8),
    Output(u8),
    Monitor,
    Rta,
    Channel2(u8),
    Aux2(u8),
    Bus2(u8),
    Main2(u8),
    Matrix2(u8)
}

lazy_static::lazy_static! {
    static ref ID_TO_NAME: HashMap<i32, Vec<String>> = {
        let mut id2name = HashMap::<i32, Vec<String>>::new();
        if id2name.is_empty() {
            for (fullname, def) in NAME_TO_DEF.iter() {
                id2name.get_mut(&def.id).map(|x| x.push(fullname.to_string())).unwrap_or_else(|| {
                    id2name.insert(def.id, vec![fullname.to_string()]);
                });
            }
        }
        id2name
    };
}

const RX_BUFFER_SIZE: usize = 2048;
const DATA_KEEP_ALIVE_SECONDS: u64 = 7;
const METERS_KEEP_ALIVE_SECONDS: u64 = 3;

pub struct DiscoveryInfo {
    pub ip:       String,
    pub name:     String,
    pub model:    String,
    pub serial:   String,
    pub firmware: String,
}

pub struct Meters {
    pub socket: UdpSocket,
    pub port: u16,
}

struct _WingConsoleMain {
    keep_alive_timer:        std::time::Instant,
    rx_buf:                  [u8; RX_BUFFER_SIZE],
    rx_buf_tail:             usize,
    rx_buf_size:             usize,
    rx_esc:                  bool,
    rx_current_channel:      i8,
    rx_has_in_pipe:          Option<u8>,
    current_node_id:         i32,
}

struct _WingConsoleMeters {
    meters:                  Option<Meters>,
    next_meter_id:           u16,
    keep_alive_meters_timer: std::time::Instant,
}

#[derive(Clone)]
pub struct WingConsole {
    rsock: Arc<Mutex<TcpStream>>,
    wsock: Arc<Mutex<TcpStream>>,
    main: Arc<Mutex<_WingConsoleMain>>,
    mtrs: Arc<Mutex<_WingConsoleMeters>>,

}

impl WingConsole {
    /// Discover mixers on active IPv4 broadcast interfaces.
    ///
    /// Probes each eligible interface, excluding loopback and point-to-point
    /// interfaces. Duplicate replies are merged by IP address and serial number.
    /// All interfaces share a five-second deadline; `stop_on_first` returns as
    /// soon as one mixer is found. One interface's failure does not prevent
    /// discovery on the others. Apple platforms explicitly bind each socket to
    /// its interface to avoid VPN default broadcast routes.
    ///
    /// Returns an empty list when no mixer responds or no interface is eligible.
    /// Returns an error if interface enumeration fails or every eligible probe
    /// fails. Discovery is limited to directly attached broadcast networks; a
    /// subsequent connection still follows normal system routing.
    pub fn scan(stop_on_first: bool) -> Result<Vec<DiscoveryInfo>> {
        crate::discovery::scan(stop_on_first)
    }

    pub fn connect(host_or_ip: Option<&str>) -> Result<Self> {
        let ip =
            if let Some(i) = host_or_ip {
                i.to_string()
            } else {
                let devices = WingConsole::scan(true)?;
                if !devices.is_empty() {
                    devices[0].ip.clone()
                } else {
                    return Err(Error::DiscoveryError);
                }
            };

        let mut stream = TcpStream::connect((ip, 2222))?;
        // stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        stream.write_all(&[0xdf, 0xd1])?;

        Ok(Self {
            wsock: Arc::new(Mutex::new(stream.try_clone()?)),
            rsock: Arc::new(Mutex::new(stream)),
            main: Arc::new(Mutex::new(_WingConsoleMain {
                keep_alive_timer: std::time::Instant::now() + std::time::Duration::from_secs(DATA_KEEP_ALIVE_SECONDS),
                rx_buf: [0; RX_BUFFER_SIZE],
                rx_buf_tail: 0,
                rx_buf_size: 0,
                rx_esc: false,
                rx_current_channel: -1,
                rx_has_in_pipe: None,
                current_node_id: 0,
            })),
            mtrs: Arc::new(Mutex::new(_WingConsoleMeters {
                keep_alive_meters_timer: std::time::Instant::now() + std::time::Duration::from_secs(METERS_KEEP_ALIVE_SECONDS),
                meters: None,
                next_meter_id: 0,
            })),
        })
    }

    pub fn read(&mut self) -> Result<WingResponse> {
        loop {
            let mainptr = self.main.clone();
            let mut main = mainptr.lock().unwrap();
            let mut raw = Vec::new(); 
            let (ch, cmd) = self.decode_next(&mut main, &mut raw)?;
            //println!("Channel: {}, Command: {:X}", ch, cmd);
            if cmd <= 0x3f {
                let v = cmd as i32;
                return Ok(WingResponse::NodeData(main.current_node_id, WingNodeData::with_i32(v)));
            } else if cmd <= 0x7f {
//                let v = cmd - 0x40 + 1;
                // println!("REQUEST: NODE INDEX: {}", v);
            } else if cmd <= 0xbf {
                let len = cmd - 0x80 + 1;
                let v = self.read_string(&mut main, ch, len as usize, &mut raw)?;
                return Ok(WingResponse::NodeData(main.current_node_id, WingNodeData::with_string(v)));
            } else if cmd <= 0xcf {
                let len = cmd - 0xc0 + 1;
                let v = self.read_string(&mut main, ch, len as usize, &mut raw)?;
                return Ok(WingResponse::NodeData(main.current_node_id, WingNodeData::with_string(v)));
            } else if cmd == 0xd0 {
                let v = String::new();
                return Ok(WingResponse::NodeData(main.current_node_id, WingNodeData::with_string(v)));
            } else if cmd == 0xd1 {
                let len = self.read_u8(&mut main, ch, &mut raw)? + 1;
                let v = self.read_string(&mut main, ch, len as usize, &mut raw)?;
                return Ok(WingResponse::NodeData(main.current_node_id, WingNodeData::with_string(v)));
            } else if cmd == 0xd2 {
                let _v = self.read_u16(&mut main, ch, &mut raw)? + 1;
                // println!("REQUEST: NODE INDEX: {}", v);
            } else if cmd == 0xd3 {
                let v = self.read_i16(&mut main, ch, &mut raw)?;
                return Ok(WingResponse::NodeData(main.current_node_id, WingNodeData::with_i16(v)));
            } else if cmd == 0xd4 {
                let v = self.read_i32(&mut main, ch, &mut raw)?;
                return Ok(WingResponse::NodeData(main.current_node_id, WingNodeData::with_i32(v)));
            } else if cmd == 0xd5 || cmd == 0xd6 {
                let v = self.read_f(&mut main, ch, &mut raw)?;
                return Ok(WingResponse::NodeData(main.current_node_id, WingNodeData::with_float(v)));
            } else if cmd == 0xd7 {
                main.current_node_id = self.read_i32(&mut main, ch, &mut raw)?;
            } else if cmd == 0xd8 {
                // println!("REQUEST: CLICK");
            } else if cmd == 0xd9 {
                let _v = self.read_i8(&mut main, ch, &mut raw)?;
                // println!("REQUEST: STEP: {}", v);
            } else if cmd == 0xda {
                // println!("REQUEST: TREE: GOTO ROOT");
            } else if cmd == 0xdb {
                // println!("REQUEST: TREE: GO UP 1");
            } else if cmd == 0xdc {
                // println!("REQUEST: DATA");
            } else if cmd == 0xdd {
                // println!("REQUEST: CURRENT NODE DEFINITION");
            } else if cmd == 0xde {
                return Ok(WingResponse::RequestEnd);
            } else if cmd == 0xdf {
                let def_len = self.read_u16(&mut main, ch, &mut raw)? as u32;
                if def_len == 0 { let _ = self.read_u32(&mut main, ch, &mut raw)?; }
                raw.clear();
                for _ in 0..def_len { self.decode_next(&mut main, &mut raw)?; } 
                return Ok(WingResponse::NodeDef(WingNodeDef::from_bytes(&raw)));
            }
        }
    }

    fn read_i8(&mut self, r: &mut _WingConsoleMain, _ch:i8, raw: &mut Vec::<u8>) -> Result<i8> {
        Ok(self.decode_next(r, raw)?.1 as i8)
    }
    fn read_u8(&mut self, r: &mut _WingConsoleMain, _ch:i8, raw: &mut Vec::<u8>) -> Result<u8> {
        Ok(self.decode_next(r, raw)?.1)
    }
    fn read_u16(&mut self, r: &mut _WingConsoleMain, _ch:i8, raw: &mut Vec::<u8>) -> Result<u16> {
        let a = self.decode_next(r, raw)?;
        let b = self.decode_next(r, raw)?;
        Ok(((a.1 as u16) << 8) | b.1 as u16)
    }
    fn read_i16(&mut self, r: &mut _WingConsoleMain, ch:i8, raw: &mut Vec::<u8>) -> Result<i16> {
        Ok(self.read_u16(r, ch, raw)? as i16)
    }
    fn read_u32(&mut self, r: &mut _WingConsoleMain, _ch:i8, raw: &mut Vec::<u8>) -> Result<u32> {
        let a = self.decode_next(r, raw)?;
        let b = self.decode_next(r, raw)?;
        let c = self.decode_next(r, raw)?;
        let d = self.decode_next(r, raw)?;
        Ok(
            ((a.1 as u32) << 24) |
            ((b.1 as u32) << 16) |
            ((c.1 as u32) << 8) |
            d.1 as u32
            )
    }
    fn read_i32(&mut self, r: &mut _WingConsoleMain, ch:i8, raw: &mut Vec::<u8>) -> Result<i32> {
        Ok(self.read_u32(r, ch, raw)? as i32)
    }

    fn read_string(&mut self, r: &mut _WingConsoleMain, _ch:i8, len:usize, raw: &mut Vec::<u8>) -> Result<String> {
        // define u8 array of size len and fill it with decode_next
        let buf = (0..len).map(|_| self.decode_next(r, raw).map(|(_, v)| v)).collect::<Result<Vec<u8>>>()?;
        // convert u8 array to string
        String::from_utf8(buf).map_err(|_| Error::InvalidData)
    }

    fn read_f(&mut self, r: &mut _WingConsoleMain, _ch:i8, raw: &mut Vec::<u8>) -> Result<f32> {
        let a = self.decode_next(r, raw)?;
        let b = self.decode_next(r, raw)?;
        let c = self.decode_next(r, raw)?;
        let d = self.decode_next(r, raw)?;
        let val = ((a.1 as u32) << 24) |
            ((b.1 as u32) << 16) |
            ((c.1 as u32) << 8) |
            d.1 as u32;
        Ok(f32::from_bits(val))
    }

    /// read() will call this as needed, but if you don't call read() then the Wing Console will
    /// hang up the connection after a 10 seconds of no activity. You should call this yourself
    /// periodically if you are not calling read().
    pub fn keep_alive(&mut self) -> Result<()> {
        self._keep_alive(&mut self.main.clone().lock().unwrap())
    }

    fn _keep_alive(&mut self, r: &mut _WingConsoleMain) -> Result<()> {
        if r.keep_alive_timer <= std::time::Instant::now() {
            // println!("keep_alive");
            self.wsock.clone().lock().unwrap().write_all(&[0xdf, 0xd1])?;
            r.keep_alive_timer = std::time::Instant::now() + std::time::Duration::from_secs(DATA_KEEP_ALIVE_SECONDS);
        }
        Ok(())
    }

    /// read_meters() will call this as needed, but if you don't call read_meters() then the Wing Console will
    /// hang up the connection after a 5 seconds of no activity. You should call this yourself
    /// periodically if you are not calling read_meters().
    pub fn keep_alive_meters(&mut self) -> Result<()> {
        self._keep_alive_meters(&mut self.mtrs.clone().lock().unwrap())
    }

    fn _keep_alive_meters(&mut self, m: &mut _WingConsoleMeters) -> Result<()> {
        if m.keep_alive_meters_timer <= std::time::Instant::now() {
            // println!("keep_alive_meters");
            let meters = m.meters.as_ref().unwrap();
            let mut keepalive = [
                0xdf, 0xd3, 0xd4,
                0x00,
                0x00,
                ((meters.port >> 8) & 0xff) as u8,
                (meters.port & 0xff) as u8,
                0xdf, 0xd1
            ];
            let mut i = m.next_meter_id as i32;
            while i > 0 {
                keepalive[3] = ((i >> 8) & 0xff) as u8;
                keepalive[4] = (i & 0xff) as u8;
                self.wsock.clone().lock().unwrap().write_all(&keepalive)?;
                i -= 1;
            }
            m.keep_alive_meters_timer = std::time::Instant::now() + std::time::Duration::from_secs(METERS_KEEP_ALIVE_SECONDS);
        }
        Ok(())
    }

    fn decode_next(&mut self, r: &mut _WingConsoleMain, raw: &mut Vec::<u8>) -> Result<(i8, u8)> {
        if r.rx_has_in_pipe.is_some() {
            // println!("has in pipe");
            let value = r.rx_has_in_pipe.unwrap();
            r.rx_has_in_pipe = None;
            raw.push(value);
            return Ok((r.rx_current_channel, value));
        }

        loop {
            self._keep_alive(r)?;
            if r.rx_buf_size == 0 {
                self.rsock.clone().lock().unwrap().set_read_timeout(Some(r.keep_alive_timer.duration_since(std::time::Instant::now())))?;
                match self.rsock.clone().lock().unwrap().read(&mut r.rx_buf) {
                    Ok(n) if n > 0 => {
                        // println!("got n {}...", n);
                        r.rx_buf_size = n;
                        r.rx_buf_tail = 0;
                    }
                    // check for blocking error
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Ok(_) => return Err(Error::ConnectionError),
                    Err(e) => return Err(e.into()),
                }
            }

            let byte = r.rx_buf[r.rx_buf_tail];
            // println!("rx_buf_tail: {}, rx_buf_size: {}, byte: {:X} buf: {}",
            //     self.rx_buf_tail,
            //     self.rx_buf_size, byte,
            //     self.rx_buf.iter().map(|x| x.to_string()).collect::<Vec<String>>().join(","));
            r.rx_buf_tail += 1;
            r.rx_buf_size -= 1;

            if ! r.rx_esc {
                if byte == 0xdf {
                    r.rx_esc = true;
                } else {
                    raw.push(byte);
                    break Ok((r.rx_current_channel, byte))
                }
            } else if byte == 0xdf {
                break Ok((r.rx_current_channel, byte))
            } else {
                r.rx_esc = false;
                if byte == 0xde {
                    raw.push(0xdf);
                    break Ok((r.rx_current_channel, 0xdf))
                } else if (0xd0..0xde).contains(&byte) {
                    r.rx_current_channel = (byte - 0xd0) as i8;
                    continue;
                } else if r.rx_current_channel >= 0 {
                    r.rx_has_in_pipe = Some(byte);
                    raw.push(0xdf);
                    break Ok((r.rx_current_channel, 0xdf))
                } else {
                    raw.push(byte);
                    break Ok((r.rx_current_channel, byte))
                }
            }
        }
    }

    fn format_id(&self, id: i32, buf: &mut Vec<u8>, prefix: u8, suffix: Option<u8>) {
        buf.push(prefix);

        let b1 = ((id >> 24) & 0xFF) as u8;
        let b2 = ((id >> 16) & 0xFF) as u8;
        let b3 = ((id >>  8) & 0xFF) as u8;
        let b4 = ((id      ) & 0xFF) as u8;

        buf.push(b1); if b1 == 0xdf { buf.push(0xde); }
        buf.push(b2); if b2 == 0xdf { buf.push(0xde); }
        buf.push(b3); if b3 == 0xdf { buf.push(0xde); }
        buf.push(b4); if b4 == 0xdf { buf.push(0xde); }

        if let Some(suffix1) = suffix {
            buf.push(suffix1);
        }
    }

    pub fn request_node_definition(&mut self, id: i32) -> Result<()> {
        let mut buf = Vec::new();
        if id == 0 {
            buf.push(0xda);
            buf.push(0xdd);
        } else {
            self.format_id(id, &mut buf, 0xd7, Some(0xdd));
        };
        self.wsock.clone().lock().unwrap().write_all(&buf)?;
        Ok(())
    }

    pub fn request_node_data(&mut self, id: i32) -> Result<()> {
        let mut buf = Vec::new();
        if id == 0 {
            buf.push(0xda);
            buf.push(0xdc);
        } else {
            self.format_id(id, &mut buf, 0xd7, Some(0xdc));
        };
        self.wsock.clone().lock().unwrap().write_all(&buf)?;
        Ok(())
    }


    /// Subscribes to meters from the Wing mixer and returns a meter ID that can be used to
    /// associate the values that come back when you call read_meter()
    pub fn request_meter(&mut self, meters: &[Meter]) -> Result<u16>
    {
        let mtrsptr = self.mtrs.clone();
        let mut mtrs = mtrsptr.lock().unwrap();
        mtrs.next_meter_id += 1;

        if mtrs.meters.is_none() {
            let socket = UdpSocket::bind("0.0.0.0:0")?;
            let port = socket.local_addr()?.port();
            socket.set_read_timeout(Some(Duration::from_millis(1000))).unwrap();
            mtrs.meters = Some(Meters { socket, port });
        } else {
            self._keep_alive_meters(&mut mtrs)?;
        }
        let md = mtrs.meters.as_ref().unwrap();

        let mut buf = vec![
            0xdf, 0xd3,
            0xd3,
            ((md.port >> 8) & 0xff) as u8,
            (md.port & 0xff) as u8,
            0xd4,
            ((mtrs.next_meter_id >> 8) & 0xff) as u8,
            (mtrs.next_meter_id & 0xff) as u8,
            ((md.port >> 8) & 0xff) as u8,
            (md.port & 0xff) as u8,
            0xdc,
        ];

        for meter in meters {
            match meter {
                Meter::Channel(n) => {
                    buf.push(0xa0);
                    buf.push(*n);
                }
                Meter::Aux(n) => {
                    buf.push(0xa1);
                    buf.push(*n);
                }
                Meter::Bus(n) => {
                    buf.push(0xa2);
                    buf.push(*n);
                }
                Meter::Main(n) => {
                    buf.push(0xa3);
                    buf.push(*n);
                }
                Meter::Matrix(n) => {
                    buf.push(0xa4);
                    buf.push(*n);
                }
                Meter::Dca(n) => {
                    buf.push(0xa5);
                    buf.push(*n);
                }
                Meter::Fx(n) => {
                    buf.push(0xa6);
                    buf.push(*n);
                }
                Meter::Source(n) => {
                    buf.push(0xa7);
                    buf.push(*n);
                }
                Meter::Output(n) => {
                    buf.push(0xa8);
                    buf.push(*n);
                }
                Meter::Monitor => {
                    buf.push(0xa9);
                }
                Meter::Rta => {
                    buf.push(0xaa);
                }
                Meter::Channel2(n) => {
                    buf.push(0xab);
                    buf.push(*n);
                }
                Meter::Aux2(n) => {
                    buf.push(0xac);
                    buf.push(*n);
                }
                Meter::Bus2(n) => {
                    buf.push(0xad);
                    buf.push(*n);
                }
                Meter::Main2(n) => {
                    buf.push(0xae);
                    buf.push(*n);
                }
                Meter::Matrix2(n) => {
                    buf.push(0xaf);
                    buf.push(*n);
                }
            }
        }

        buf.push(0xde); // end of def
        buf.push(0xdf);
        buf.push(0xd1);

        self.wsock.clone().lock().unwrap().write_all(&buf)?;

        Ok(mtrs.next_meter_id)
    }

    /// reads any meter values that have been requested with request_meter() and returns the meter
    /// ID along with the meters values
    pub fn read_meters(&mut self) -> Result<(u16, Vec<i16>)> {
        loop {
            let mptr = self.mtrs.clone();
            let mut m = mptr.lock().unwrap();

            self._keep_alive_meters(&mut m)?;
            let md = m.meters.as_ref().unwrap();
            let mut buf = [0u8; 8192];
            md.socket.set_read_timeout(Some(m.keep_alive_meters_timer.duration_since(std::time::Instant::now())))?;
            match md.socket.recv_from(&mut buf) {
                Ok((received, _addr)) => {
                    return Ok((u16::from_be_bytes([buf[0], buf[1]]), buf[4..received]
                            .chunks_exact(2) // Take 2 bytes at a time
                            .map(|chunk| i16::from_be_bytes([chunk[0], chunk[1]]))
                            .collect()));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(_) => {
                    return Err(Error::ConnectionError);
                }
            }
        }
    }

    pub fn set_string(&mut self, id: i32, value: &str) -> Result<()> {
        let mut buf = Vec::new();
        self.format_id(id, &mut buf, 0xd7, None);

        if value.is_empty() {
            buf.push(0xd0);
        } else if value.len() <= 64 {
            buf.push(0x7f + value.len() as u8);
        } else if value.len() <= 256 {
            buf.push(0xd1);
            buf.push((value.len()-1) as u8);
        }

        for c in value.bytes() {
            buf.push(c);
            // do we need this escaping? i guess 0xdf never really shows up in strings unless its
            // unicode stuff that the wing probably doesn't support
            // if c == 0xdf { buf.push(0xde); }
        }
        self.wsock.clone().lock().unwrap().write_all(&buf)?;
        Ok(())
    }

    pub fn set_float(&mut self, id: i32, value: f32) -> Result<()> {
        let mut buf = Vec::new();
        self.format_id(id, &mut buf, 0xd7, Some(0xd5));

        let bytes = value.to_be_bytes();
        buf.push(bytes[0]);
        buf.push(bytes[1]);
        buf.push(bytes[2]);
        buf.push(bytes[3]);

        self.wsock.clone().lock().unwrap().write_all(&buf)?;
        Ok(())
    }

    pub fn set_int(&mut self, id: i32, value: i32) -> Result<()> {
        let mut buf = Vec::new();
        self.format_id(id, &mut buf, 0xd7, None);

        let bytes = value.to_be_bytes();

        if (0..=0x3f).contains(&value) {
            buf.push(value as u8);
        } else if (-32768..=32767).contains(&value) {
            buf.push(0xd3);
            buf.push(bytes[2]);
            buf.push(bytes[3]);
        } else {
            buf.push(0xd4);
            buf.push(bytes[0]);
            buf.push(bytes[1]);
            buf.push(bytes[2]);
            buf.push(bytes[3]);
        }

        self.wsock.clone().lock().unwrap().write_all(&buf)?;
        Ok(())
    }

    pub fn name_to_id(fullname: &str) -> Option<i32> {
        if let Ok(num) = fullname.parse::<i32>() {
            Some(num)
        } else {
            NAME_TO_DEF.get(fullname).map(|x| x.id)
        }
    }
    pub fn name_to_def(fullname: &str) -> Option<&WingNodeDef> {
        NAME_TO_DEF.get(fullname)
    }

    pub fn id_to_defs(id: i32) -> Option<Vec<(String, WingNodeDef)>> {
        ID_TO_NAME.get(&id)
            .cloned()
            .map(|names|
                names
                .iter()
                .map(|n| (n, NAME_TO_DEF.get(n)))
                .filter(|x| x.1.is_some())
                .map(|x| (x.0, x.1.unwrap()))
                .map(|(n, v)| (n.clone(), v.clone())
                ).collect())
    }
}

impl Drop for WingConsole {
    fn drop(&mut self) {
        let _ = self.wsock.clone().lock().unwrap().shutdown(std::net::Shutdown::Both);
        let _ = self.rsock.clone().lock().unwrap().shutdown(std::net::Shutdown::Both);
    }
}
