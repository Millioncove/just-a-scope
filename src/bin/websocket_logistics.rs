use embedded_io_async::{ErrorType, Write};
use esp_println::println;
use zerocopy::{FromBytes, Immutable, IntoBytes};

#[derive(IntoBytes, FromBytes, Immutable, Clone, Copy, Debug)]
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
    let len = data.len() as u64;

    if len == 0 {
        return Ok(());
    } else if len <= 126 {
        let payload_length = len as u8;
        let header = [fin_rsv_opcode, payload_length];

        to.write_all(&[&header, data].concat()).await
    } else if len <= 2u64.pow(16) - 1 {
        let header = [fin_rsv_opcode, 126u8];
        let payload_length = len as u16;
        let mut payload_length: [u8; 2] = payload_length.as_bytes().try_into().unwrap();
        payload_length.reverse();

        let to_be_sent = &[&header, &payload_length, data].concat();
        to.write_all(&to_be_sent).await?;
        to.flush().await
    } else {
        if len <= 2u64.pow(63) - 1 {
            panic!("Data too huge for a WebSocket message!");
        }
        let header = [fin_rsv_opcode, 127u8];
        let payload_length = len as u64;
        let mut payload_length: [u8; 8] = payload_length.as_bytes().try_into().unwrap();
        payload_length.reverse();

        to.write_all(&[&header, &payload_length[..], data].concat())
            .await?;
        to.flush().await
    }
}
