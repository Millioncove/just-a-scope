use core::{
    cell::{RefCell, UnsafeCell},
    error::Error,
};

use alloc::boxed::Box;
use critical_section::Mutex;
use esp_hal::{analog::adc::Adc, delay::Delay, time::now};
use zerocopy::{Immutable, IntoBytes};

use crate::websocket_logistics::OscilliscopePoint;

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
}

impl<'a, const L: usize, T> CyclicWriter<'a, L, T> {
    pub fn insert(&mut self, value: T) -> Result<usize, Box<dyn Error>> {
        let count_before = self.buffer.entry_count();
        if count_before >= L - 1 {
            return Err(Box::from("Trying to overwrite unread entries."));
        }

        let entries_pointer = self.buffer.entries.get();
        let write_index = self.buffer.write_index.get();
        unsafe {
            (*entries_pointer)[*write_index] = value;
            *write_index = (*write_index + 1) % L;
        }

        Ok(count_before + 1)
    }
}

// Original reading functionality of CyclicBuffer. Too much copying of single elements.
/*impl<const L: usize, T> Iterator for CyclicReader<'_, L, T>
where
    T: Copy,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buffer.entry_count() == 0 {
            return None;
        } else {
            let entries = self.buffer.entries.get();
            let read_index = self.buffer.read_index.get();
            let item: T;
            unsafe {
                item = (*entries)[*read_index].clone();
                *read_index = (*read_index + 1) % L;
            }
            return Some(item);
        }
    }
}*/

impl<'a, const L: usize, T: IntoBytes + Immutable> CyclicReader<'a, L, T> {
    pub fn get_batch_holder(&self) -> CyclicBatch<'a, L, T> {
        let read_index: usize;
        let write_index: usize;
        let entries_array: *mut [T; L] = self.buffer.entries.get() as *mut [T; L];
        unsafe {
            read_index = *self.buffer.read_index.get();
            write_index = *self.buffer.write_index.get();

            if read_index <= write_index {
                return CyclicBatch {
                    buffer: &self.buffer,
                    batches: [
                        &((*entries_array)[read_index..write_index]).as_bytes(),
                        &((*entries_array)[0..0]).as_bytes(),
                    ],
                    reads_until: write_index,
                };
            } else {
                return CyclicBatch {
                    buffer: &self.buffer,
                    batches: [
                        &((*entries_array)[read_index..]).as_bytes(),
                        &((*entries_array)[..write_index]).as_bytes(),
                    ],
                    reads_until: write_index,
                };
            }
        }
    }
}

impl<'a, const L: usize, T> Drop for CyclicBatch<'a, L, T> {
    fn drop(&mut self) {
        unsafe {
            *self.buffer.read_index.get() = self.reads_until;
        }
    }
}

pub fn measuring_task<const L: usize, ADCI>(
    _adc: Adc<'_, ADCI>,
    point_buffer_writer: &mut CyclicWriter<'_, L, OscilliscopePoint>,
) -> ! {
    let delay = Delay::new();
    loop {
        // Dummy sawtooth waveform.
        let point = OscilliscopePoint {
            voltage: (now().ticks() as f64 / 1_000_000f64) % 2f64,
            second: (now().duration_since_epoch().to_micros() as f64) * 0.000001f64,
        };

        match point_buffer_writer.insert(point) {
            Ok(_) => (),
            Err(_) => (),
        }
        delay.delay_micros(10);
    }
}
