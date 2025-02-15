use core::{
    cell::{RefCell, UnsafeCell},
    error::Error,
};
use embedded_io_async::{ErrorType, Write};

use alloc::boxed::Box;
use critical_section::Mutex;
use libm::fabs;
use zerocopy::{FromBytes, Immutable, IntoBytes};

#[derive(IntoBytes, FromBytes, Immutable, Clone, Copy, Debug)]
#[repr(C)]
pub struct OscilliscopePoint {
    pub voltage: f64,
    pub second: f64,
}

pub struct CyclicWriter<'a, const L: usize, T> {
    buffer: &'a CyclicBuffer<L, T>,
}

pub struct CyclicReader<'a, const L: usize, T> {
    buffer: &'a CyclicBuffer<L, T>,
}

pub struct CyclicBatch<'a, const L: usize, T> {
    buffer: &'a CyclicBuffer<L, T>,
    pub batches: [&'a [u8]; 2],
    reads_until: usize,
}

pub struct CyclicBuffer<const L: usize, T> {
    read_index: UnsafeCell<usize>,
    write_index: UnsafeCell<usize>,
    entries: UnsafeCell<[T; L]>,
    writer_available: Mutex<RefCell<bool>>,
    reader_available: Mutex<RefCell<bool>>,
    pub missed: UnsafeCell<usize>,
}

unsafe impl<const L: usize, T> Send for CyclicBuffer<L, T> {}
unsafe impl<'a, const L: usize, T> Send for CyclicWriter<'a, L, T> {}

impl<const L: usize, T> CyclicBuffer<L, T> {
    pub fn new(filler: T) -> CyclicBuffer<L, T>
    where
        T: Copy,
    {
        CyclicBuffer {
            read_index: UnsafeCell::new(0),
            write_index: UnsafeCell::new(0),
            entries: UnsafeCell::new([filler; L]),
            reader_available: Mutex::new(RefCell::new(true)),
            writer_available: Mutex::new(RefCell::new(true)),
            missed: UnsafeCell::new(0),
        }
    }

    pub fn take_writer(&self) -> Option<CyclicWriter<'_, L, T>> {
        let mut writer: Option<CyclicWriter<'_, L, T>> = None;
        critical_section::with(|cs| {
            let mut writer_available = self.writer_available.borrow_ref_mut(cs);
            if *writer_available {
                *writer_available = false;
                writer = Some(CyclicWriter { buffer: self });
            }
        });
        writer
    }

    pub fn take_reader(&self) -> Option<CyclicReader<'_, L, T>> {
        let mut reader: Option<CyclicReader<'_, L, T>> = None;
        critical_section::with(|cs| {
            let mut reader_available = self.reader_available.borrow_ref_mut(cs);
            if *reader_available {
                *reader_available = false;
                reader = Some(CyclicReader { buffer: self });
            }
        });
        reader
    }

    pub fn entry_count(&self) -> usize {
        let read_index;
        let write_index;
        unsafe {
            read_index = *self.read_index.get();
            write_index = *self.write_index.get();
        }

        let count: usize;
        if read_index <= write_index {
            count = write_index - read_index;
        } else {
            count = L - read_index + write_index;
        }
        assert!(
            count < L,
            "Seemingly more entries in the buffer than it has capacity for."
        );
        count
    }

    pub fn increment_missed(&self) {
        unsafe {
            *self.missed.get() += 1;
        }
    }
}

impl<'a, const L: usize, T> CyclicWriter<'a, L, T> {
    pub fn append(&mut self, value: T) -> Result<(), Box<dyn Error>> {
        let count_before = self.buffer.entry_count();
        if count_before >= L - 1 {
            self.buffer.increment_missed();
            return Err(Box::from("Trying to overwrite unread entries."));
        }

        let entries_pointer = self.buffer.entries.get();
        let write_index = self.buffer.write_index.get();
        unsafe {
            (*entries_pointer)[*write_index] = value;
            *write_index = (*write_index + 1) % L;
        }

        Ok(())
    }
}

pub fn is_middle_point_removable_complicated(
    left: &OscilliscopePoint,
    middle: &OscilliscopePoint,
    right: &OscilliscopePoint,
    tolerance_factor: f64,
    min_voltage_difference: f64,
) -> bool {
    if fabs(right.voltage - left.voltage) < min_voltage_difference {
        return true;
    } else {
        let delta_time_to_right = right.second - left.second;
        if delta_time_to_right == 0f64 {
            return true;
        }

        let voltage_difference = {
            let voltage_on_slope = {
                let slope_to_right = {
                    let delta_voltage_to_right = right.voltage - left.voltage;
                    delta_voltage_to_right / delta_time_to_right
                };
                slope_to_right * (middle.second - left.second) + left.voltage
            };
            voltage_on_slope - middle.voltage
        };
        let tolerance = {
            let middle_proximity =
                1f64 - fabs(1f64 - 2f64 * (middle.second - left.second) / delta_time_to_right);
            (tolerance_factor / 2f64) * middle_proximity * fabs(right.voltage - left.voltage)
        };

        fabs(voltage_difference) < tolerance
    }
}

impl<'a, const L: usize, T: IntoBytes + Immutable> CyclicReader<'a, L, T> {
    pub fn get_batch_holder(&self) -> CyclicBatch<'a, L, T> {
        let read_index: usize;
        let entries_array: *mut [T; L] = self.buffer.entries.get() as *mut [T; L];
        unsafe {
            read_index = *self.buffer.read_index.get();
            let batch_write_index = *self.buffer.write_index.get();

            if read_index <= batch_write_index {
                return CyclicBatch {
                    buffer: &self.buffer,
                    batches: [
                        &((*entries_array)[read_index..batch_write_index]).as_bytes(),
                        &((*entries_array)[0..0]).as_bytes(),
                    ],
                    reads_until: batch_write_index,
                };
            } else {
                return CyclicBatch {
                    buffer: &self.buffer,
                    batches: [
                        &((*entries_array)[read_index..]).as_bytes(),
                        &((*entries_array)[..batch_write_index]).as_bytes(),
                    ],
                    reads_until: batch_write_index,
                };
            }
        }
    }
}

impl<'a, const L: usize, T> Drop for CyclicReader<'a, L, T> {
    fn drop(&mut self) {
        critical_section::with(|cs| {
            let mut reader_available = self.buffer.reader_available.borrow_ref_mut(cs);
            *reader_available = true;
        });
    }
}

impl<'a, const L: usize, T> Drop for CyclicBatch<'a, L, T> {
    fn drop(&mut self) {
        unsafe {
            *self.buffer.read_index.get() = self.reads_until;
        }
    }
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
        to.write_all(&to_be_sent).await
    } else {
        if len <= 2u64.pow(63) - 1 {
            panic!("Data too huge for a WebSocket message!");
        }
        let header = [fin_rsv_opcode, 127u8];
        let payload_length = len as u64;
        let mut payload_length: [u8; 8] = payload_length.as_bytes().try_into().unwrap();
        payload_length.reverse();

        to.write_all(&[&header, &payload_length[..], data].concat())
            .await
    }
}
