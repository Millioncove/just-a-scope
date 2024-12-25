use embedded_io_async::{ErrorType, Write};
use zerocopy::{FromBytes, Immutable, IntoBytes};

#[derive(IntoBytes, FromBytes, Immutable)]
#[repr(C)]
pub struct OscilliscopePoint {
    pub voltage: f64,
    pub second: f64,
}

pub async fn send_message<W>(to: &mut W, data: &[u8]) -> Result<(), <W as ErrorType>::Error>
where
    W: Write,
{
    let fin_rsv_opcode = 0b10000010u8; // FIN and binary data.
    let payload_length = data.len() as u8;
    if payload_length > 126 {
        panic!("Max payload length for a simple WebSocket message is 126 bytes.")
    }
    let header = [fin_rsv_opcode, payload_length];

    to.write_all(&[&header, data].concat()).await
}
