use log::{error, info, trace};
use tokio::{io::AsyncReadExt, net::tcp::OwnedReadHalf, time::Instant};

use crate::usr_canet::{DataFrame, Message};

#[repr(u8)]
#[derive(Debug)]
pub enum GVRETProtocol {
    BuildCanFrame = 0,
    TimeSync = 1,
    DigInputs = 2,
    AnaInputs = 3,
    SetDigOut = 4,
    SetupCanBus = 5,
    GetCanBusParams = 6,
    GetDevInfo = 7,
    SetSwMode = 8,
    KeepAlive = 9,
    SetSysType = 10,
    EchoCanFrame = 11,
    GetNumBuses = 12,
    GetExtBuses = 13,
    SetExtBuses = 14,
    BuildFdFrame = 20,
    SetupFd = 21,
    GetFd = 22,
}

pub fn get_canbus_params(port2: bool) -> Vec<u8> {
    let can_baud = 500_000u32.to_le_bytes(); // baud set in USR Canet only
    let mut v = Vec::with_capacity(12);
    v.push(0xf1);
    v.push(0x6);
    v.push(0x1);
    v.extend_from_slice(&can_baud);
    if port2 {
        v.push(0x1);
        v.extend_from_slice(&can_baud);
    }
    v
}

pub fn get_num_busses(busses: u8) -> Vec<u8> {
    vec![0xf1, 0xc, busses]
}

fn get_dev_info() -> Vec<u8> {
    vec![0xf1, 0x07, 0x6a, 0x02, 0x20, 00, 00, 00]
}
pub fn get_timesync(now: Instant) -> Vec<u8> {
    let m = (now.elapsed().as_micros() as u32).to_le_bytes();
    vec![0xf1, 0x01, m[0], m[1], m[2], m[3]]
}
fn get_keepalive() -> Vec<u8> {
    vec![0xf1, 0x9, 0xde, 0xad]
}

pub(crate) fn build_can_frame(frame_header: [u8; 6], frame_data: [u8; 8]) -> Message {
    let mut id = u32::from_le_bytes(frame_header[0..4].try_into().unwrap());
    let ext_id = if id > crate::usr_canet::CAN_STD_ID_MASK {
        id ^= 1 << 31;
        assert!(crate::usr_canet::CAN_EXT_ID_MASK > id);
        true
    } else {
        assert!(crate::usr_canet::CAN_STD_ID_MASK > id);
        false
    };
    let dlc = (frame_header[5] & 0xf).min(8);
    let bus = frame_header[4] & 3;

    Message::Data(
        bus,
        DataFrame::new(id, ext_id, frame_data[..dlc.into()].to_vec()).unwrap(),
    )
}

impl GVRETProtocol {
    pub(crate) fn process(&self) -> Vec<u8> {
        match self {
            GVRETProtocol::BuildCanFrame => todo!(),
            GVRETProtocol::TimeSync => todo!(),
            GVRETProtocol::DigInputs => todo!(),
            GVRETProtocol::AnaInputs => todo!(),
            GVRETProtocol::SetDigOut => todo!(),
            GVRETProtocol::SetupCanBus => todo!(),
            GVRETProtocol::GetCanBusParams => todo!(),
            GVRETProtocol::GetDevInfo => get_dev_info(),
            GVRETProtocol::SetSwMode => todo!(),
            GVRETProtocol::KeepAlive => get_keepalive(),
            GVRETProtocol::SetSysType => todo!(),
            GVRETProtocol::EchoCanFrame => todo!(),
            GVRETProtocol::GetNumBuses => todo!(),
            GVRETProtocol::GetExtBuses => vec![
                0xf1, 0x0d, 00, 00, 00, 00, 00, 00, 00, 00, 00, 00, 00, 00, 00, 00, 00,
            ],
            GVRETProtocol::SetExtBuses => todo!(),
            GVRETProtocol::BuildFdFrame => todo!(),
            GVRETProtocol::SetupFd => todo!(),
            GVRETProtocol::GetFd => todo!(),
        }
    }
}

impl From<u8> for GVRETProtocol {
    fn from(value: u8) -> Self {
        match value {
            0 => GVRETProtocol::BuildCanFrame,
            1 => GVRETProtocol::TimeSync,
            2 => GVRETProtocol::DigInputs,
            3 => GVRETProtocol::AnaInputs,
            4 => GVRETProtocol::SetDigOut,
            5 => GVRETProtocol::SetupCanBus,
            6 => GVRETProtocol::GetCanBusParams,
            7 => GVRETProtocol::GetDevInfo,
            8 => GVRETProtocol::SetSwMode,
            9 => GVRETProtocol::KeepAlive,
            10 => GVRETProtocol::SetSysType,
            11 => GVRETProtocol::EchoCanFrame,
            12 => GVRETProtocol::GetNumBuses,
            13 => GVRETProtocol::GetExtBuses,
            14 => GVRETProtocol::SetExtBuses,
            20 => GVRETProtocol::BuildFdFrame,
            21 => GVRETProtocol::SetupFd,
            22 => GVRETProtocol::GetFd,
            _ => GVRETProtocol::BuildCanFrame,
        }
    }
}

#[repr(u8)]
#[derive(PartialEq)]
pub(crate) enum Mode {
    Init = 0,
    Binary = 0xe7,
    Command = 0xf1,
}

impl From<u8> for Mode {
    fn from(value: u8) -> Self {
        match value {
            0xe7 => Mode::Binary,
            0xf1 => Mode::Command,
            _ => Mode::Init, // Default to Init for unknown values
        }
    }
}

pub(crate) enum Gvret {
    Frame(crate::usr_canet::Message),
    Init(Vec<u8>),
}
pub(crate) async fn decode_gvret_frames(
    gvret_socket: &mut OwnedReadHalf,
    mode: &mut Mode,
    num_busses: u8,
    now: Instant,
) -> Gvret {
    let mut b = [0; 1];

    loop {
        let result = gvret_socket.read_exact(&mut b).await;
        match result {
            Ok(n) => {
                trace!("GVRET byte read {b:02x?}");
                if n == 0 {
                    continue;
                }
                let c: Mode = b[0].into();
                if c == Mode::Binary && *mode == Mode::Init {
                    *mode = Mode::Binary;
                    info!("GVRET handshake complete");
                    continue;
                }
                if c != Mode::Command {
                    continue;
                }

                'read: {
                    let cmd: GVRETProtocol = match gvret_socket.read_exact(&mut b).await {
                        Ok(_) => b[0].into(),
                        Err(e) => {
                            error!("GVRET TCP read error {e}");
                            break 'read;
                        }
                    };
                    let mut frame_header = [0; 6];
                    let mut frame_data = [0; 8];
                    let resp = match cmd {
                        GVRETProtocol::BuildCanFrame => {
                            if let Err(e) = gvret_socket.read_exact(&mut frame_header).await {
                                error!("BuildCanFrame header error {cmd:?} {e}");
                                break 'read;
                            };

                            let dlc = (frame_header[5] & 0xf).min(8);
                            if let Err(e) = gvret_socket
                                .read_exact(&mut frame_data[..dlc as usize])
                                .await
                            {
                                error!("BuildCanFrame data error {cmd:?} {e}");
                                break 'read;
                            }

                            let message = build_can_frame(frame_header, frame_data);
                            return Gvret::Frame(message);
                        }
                        GVRETProtocol::GetCanBusParams => get_canbus_params(num_busses > 1),
                        GVRETProtocol::TimeSync => get_timesync(now),
                        GVRETProtocol::GetNumBuses => get_num_busses(num_busses),
                        cmd => cmd.process(),
                    };
                    return Gvret::Init(resp);
                }
            }
            Err(e) => {
                error!("GVRET TCP read error {e}");
                // break;
            }
        }
    }
}
pub(crate) fn convert_to_gvret(message: Message, now: Instant) -> Option<Vec<u8>> {
    let data: &[u8] = match message.data() {
        Some(msg) => msg,
        _ => return None,
    };
    if message.dlc() > 8 {
        return None;
    }

    let mut out_buf = vec![];
    let mut id = message.id();
    if message.ext_id() {
        id |= 1 << 31;
    }
    out_buf.extend([0xf1, 0x0]);
    let millis = now.elapsed().as_micros() as u32;
    out_buf.extend(millis.to_le_bytes()); //timestamp
    out_buf.extend(&id.to_le_bytes());
    let byte = (message.bus() << 4) | (message.dlc() & 0xf);
    out_buf.push(byte);
    out_buf.extend(data);
    out_buf.push(0);
    Some(out_buf)
}
