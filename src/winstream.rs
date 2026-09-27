use std::io::{Read, Seek, SeekFrom};

use windows::Win32::System::Com::{IStream, STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET};

pub struct WinStream<'a> {
    stream: &'a IStream,
}

impl<'a> From<&'a IStream> for WinStream<'a> {
    fn from(stream: &'a IStream) -> Self {
        Self { stream }
    }
}

impl Read for WinStream<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        let mut bytes_read = 0u32;
        unsafe {
            self.stream.Read(
                buf.as_mut_ptr() as _,
                buf.len() as u32,
                Some((&mut bytes_read) as *mut _),
            )
        }
        .ok()
        .map_err(|err| std::io::Error::other(format!("IStream::Read failed: {}", err.code().0)))?;
        Ok(bytes_read as usize)
    }
}

impl Seek for WinStream<'_> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, std::io::Error> {
        let (origin, offset) = match pos {
            SeekFrom::Start(p) => (STREAM_SEEK_SET, p as i64),
            SeekFrom::Current(p) => (STREAM_SEEK_CUR, p),
            SeekFrom::End(p) => (STREAM_SEEK_END, p),
        };
        let mut new_pos = 0u64;
        unsafe {
            self.stream
                .Seek(offset, origin, Some((&mut new_pos) as *mut _))
        }
        .map_err(|err| std::io::Error::other(format!("IStream::Seek failed: {}", err.code().0)))?;
        Ok(new_pos)
    }
}
