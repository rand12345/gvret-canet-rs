// #![allow(dead_code)]

use byteorder::{BigEndian, ByteOrder};
use log::info;
/// Original implentation - https://github.com/raffber/async-can
/// Added dual CAN control, bus in Message
use std::{
    fmt::Display,
    io::{self},
    result::Result as StdResult,
};
use thiserror::Error;
use tokio::{io::AsyncReadExt, net::tcp::OwnedReadHalf};
/// Maximum value for CAN ID if extended 29-bit ID is selected
pub const CAN_EXT_ID_MASK: u32 = 0x1FFFFFFF;

/// Maximum value for CAN ID if standard 11-bit ID is selected
pub const CAN_STD_ID_MASK: u32 = 0x7FF;

/// Maximum data length or dlc in a CAN message
pub const CAN_MAX_DLC: usize = 8;

pub(crate) mod base {
    #[derive(Debug, Clone, Eq, PartialEq)]
    pub(crate) struct DataFrame {
        pub(crate) id: u32,
        pub(crate) ext_id: bool,
        pub(crate) data: Vec<u8>,
    }

    #[derive(Debug, Clone, Eq, PartialEq)]
    pub(crate) struct RemoteFrame {
        pub(crate) id: u32,
        pub(crate) ext_id: bool,
        pub(crate) dlc: u8,
    }
}

/// A CAN data frame, i.e. the RTR bit is set to 0
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DataFrame(base::DataFrame);

impl DataFrame {
    /// Create a new [`DataFrame`] and returns an error in case the ID is out of range or the data is too long.
    pub fn new(id: u32, ext_id: bool, data: Vec<u8>) -> StdResult<Self, CanFrameError> {
        CanFrameError::validate_id(id, ext_id)?;
        if data.len() > CAN_MAX_DLC {
            return Err(CanFrameError::DataTooLong);
        }
        Ok(Self(base::DataFrame { id, ext_id, data }))
    }

    pub fn id(&self) -> u32 {
        self.0.id
    }
    pub fn ext_id(&self) -> bool {
        self.0.ext_id
    }
    pub fn data(&self) -> &[u8] {
        &self.0.data
    }
    pub fn dlc(&self) -> u8 {
        self.0.data.len() as u8
    }
}

/// A CAN remote frame, i.e. the RTR bit is set to 1. Also, this type of frame
///  does not have a data field.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RemoteFrame(base::RemoteFrame);

impl RemoteFrame {
    /// Create a new [`RemoteFrame`] and returns an error in case the ID is out of range or the dlc is too long.
    pub fn new(id: u32, ext_id: bool, dlc: u8) -> StdResult<Self, CanFrameError> {
        CanFrameError::validate_id(id, ext_id)?;
        if dlc as usize > CAN_MAX_DLC {
            return Err(CanFrameError::DataTooLong);
        }
        Ok(Self(base::RemoteFrame { id, ext_id, dlc }))
    }

    pub fn id(&self) -> u32 {
        self.0.id
    }
    pub fn ext_id(&self) -> bool {
        self.0.ext_id
    }
    pub fn dlc(&self) -> u8 {
        self.0.dlc
    }
}

/// A message on the CAN bus, either a [`DataFrame`] or a [`RemoteFrame`].
///
/// In the future this will also contain a CAN-FD frame type.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Message {
    Data(u8, DataFrame),
    Remote(u8, RemoteFrame),
}

impl Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Message::Data(bus, data_frame) => write!(
                f,
                "Data Frame: bus={}, id={:02x}, ext_id={}, dlc={}, data={:02x?}",
                bus,
                data_frame.id(),
                data_frame.ext_id(),
                data_frame.dlc(),
                data_frame.data()
            ),
            Message::Remote(bus, remote_frame) => write!(
                f,
                "Remote Frame: bus={}, id={:02x}, ext_id={}, dlc={}",
                bus,
                remote_frame.id(),
                remote_frame.ext_id(),
                remote_frame.dlc()
            ),
        }
    }
}

impl Message {
    /// Create a new message containing a data frame. Returns an error in case the ID is out of range or the data is too long.
    pub fn new_data(
        bus: u8,
        id: u32,
        ext_id: bool,
        data: &[u8],
    ) -> StdResult<Message, CanFrameError> {
        CanFrameError::validate_id(id, ext_id)?;
        if data.len() > CAN_MAX_DLC {
            return Err(CanFrameError::DataTooLong);
        }
        Ok(Message::Data(
            bus,
            DataFrame(base::DataFrame {
                id,
                ext_id,
                data: data.to_vec(),
            }),
        ))
    }

    pub fn data(&self) -> Option<&[u8]> {
        match self {
            Message::Data(_, x) => Some(x.data()),
            Message::Remote(_, _) => None,
        }
    }

    /// Create a new message containing a remote frame. Returns an error in case the ID is out of range or the dlc is too long.
    pub fn new_remote(
        bus: u8,
        id: u32,
        ext_id: bool,
        dlc: u8,
    ) -> StdResult<Message, CanFrameError> {
        CanFrameError::validate_id(id, ext_id)?;
        if dlc as usize > CAN_MAX_DLC {
            return Err(CanFrameError::DataTooLong);
        }
        Ok(Message::Remote(bus, RemoteFrame::new(id, ext_id, dlc)?))
    }

    pub fn bus(&self) -> u8 {
        match self {
            Message::Data(b, _) => *b,
            Message::Remote(b, _) => *b,
        }
    }

    pub fn id(&self) -> u32 {
        match self {
            Message::Data(_, data_frame) => data_frame.0.id,
            Message::Remote(_, remote_frame) => remote_frame.0.id,
        }
    }

    pub fn ext_id(&self) -> bool {
        match self {
            Message::Data(_, x) => x.0.ext_id,
            Message::Remote(_, x) => x.0.ext_id,
        }
    }

    pub fn dlc(&self) -> u8 {
        match self {
            Message::Data(_, x) => x.dlc(),
            Message::Remote(_, x) => x.0.dlc,
        }
    }
}

/// Encodes errors that may occur when attempting to create/validate CAN message fields.
#[derive(Debug)]
pub enum CanFrameError {
    IdTooLong,
    DataTooLong,
}

impl From<CanFrameError> for UsrError {
    fn from(x: CanFrameError) -> Self {
        match x {
            CanFrameError::IdTooLong => UsrError::IdTooLong,
            CanFrameError::DataTooLong => UsrError::DataTooLong,
        }
    }
}

impl CanFrameError {
    fn validate_id(id: u32, ext_id: bool) -> StdResult<(), CanFrameError> {
        if ext_id {
            if id > CAN_EXT_ID_MASK {
                return Err(CanFrameError::IdTooLong);
            }
        } else if id > CAN_STD_ID_MASK {
            return Err(CanFrameError::IdTooLong);
        }
        Ok(())
    }
}

/// Error type encoding all possible errors that may occur in this crate
#[derive(Error, Debug)]
pub enum UsrError {
    #[error("Io Error: {0}")]
    Io(io::Error),
    #[error("Id is too long")]
    IdTooLong,
    #[error("Data is too long")]
    DataTooLong,
    // #[error("Other Error: {0}")]
    // Other(String),
}

impl From<io::Error> for UsrError {
    fn from(x: io::Error) -> Self {
        UsrError::Io(x)
    }
}

pub(crate) async fn decode_canet_frame(
    canet_socket: &mut OwnedReadHalf,
    bus: u8,
) -> Option<Message> {
    let mut buf = [0_u8; 13];
    match canet_socket.read_exact(&mut buf).await {
        Ok(v) => {
            info!("Recv {v} bus={bus} data={buf:02x?}");
            let ext_id = (buf[0] & 0x80) != 0;
            let id = BigEndian::read_u32(&buf[1..]);
            let dlc = buf[0] & 0xF;
            if (buf[0] & 0x40) != 0 {
                Message::new_remote(bus, id, ext_id, dlc).ok()
            } else {
                Message::new_data(bus, id, ext_id, &buf[5..5 + (dlc as usize)]).ok()
            }
        }
        Err(_e) => None,
    }
}

#[derive(Debug)]
pub(crate) enum CanetMsg {
    Can1([u8; 13]),
    Can2([u8; 13]),
}

pub(crate) fn convert_to_canet(msg: Message) -> CanetMsg {
    let mut buf = [0_u8; 13];
    buf[0] = if msg.ext_id() { 0x80_u8 } else { 0x00 };
    buf[0] |= msg.dlc() & 0xF;
    BigEndian::write_u32(&mut buf[1..], msg.id());
    let bus = match msg {
        Message::Data(bus, msg) => {
            buf[5..5 + msg.dlc() as usize].copy_from_slice(msg.data());
            bus
        }
        Message::Remote(bus, msg) => {
            buf[0] |= 0x40;
            BigEndian::write_u32(&mut buf[1..], msg.id());
            bus
        }
    }
    .max(1);

    match bus {
        0 => CanetMsg::Can1(buf),
        1 => CanetMsg::Can2(buf),
        _ => unreachable!(), // max(1)
    }
}
