use fmt;
use result;
use string::String;
use vec::{IntoIter, Vec};
use syscall::{sys_read, sys_write};

pub struct Error;

/// Types you can read
pub trait Read {

    /// Read a file to a buffer
    fn read(&mut self, buf: &mut [u8]) -> Option<usize>;

    /// Read the file to the end
    fn read_to_end(&mut self, vec: &mut Vec<u8>) -> Option<usize> {
        let mut read = 0;
        loop {
            let mut bytes = [0; 1024];
            match self.read(&mut bytes) {
                Some(0) => return Some(read),
                None => return None,
                Some(count) => {
                    for i in 0..count {
                        vec.push(bytes[i]);
                    }
                    read += count;
                }
            }
        }
    }
    
    /// Read the file to a string
    fn read_to_string(&mut self, string: &mut String) -> Option<usize> {
        let mut read = 0;
        loop {
            let mut bytes = [0; 1024];
            match self.read(&mut bytes) {
                Some(0) => return Some(read),
                None => return None,
                Some(count) => {
                    for i in 0..count {
                        string.push(bytes[i] as char); //TODO Allow UTF8
                    }
                    read += count;
                }
            }
        }
    }
    
    /// Return an iterator of the bytes
    fn bytes(&mut self) -> IntoIter<u8> {
        // TODO: This is only a temporary implementation. Make this read one byte at a time.
        let mut buf = Vec::new();
        self.read_to_end(&mut buf);

        buf.into_iter()
    }
}

/// Types you can write
pub trait Write {
    /// Write to the file
    fn write(&mut self, buf: &[u8]) -> Option<usize>;
}

/// Standard Input
pub struct Stdin;

/// Create a standard input
pub fn stdin() -> Stdin {
    Stdin
}

impl Stdin {
    pub fn read_line(&mut self, string: &mut String) -> Result<usize, Error> {
        let mut bytes = [0; 1024];
        match self.read(&mut bytes) {
            None => return Err(Error),
            Some(count) => {
                for i in 0..count {
                    string.push(bytes[i] as char); //TODO Allow UTF8
                }
                return Ok(count);
            }
        }
    }
}

/// Read implementation for standard input
impl Read for Stdin {
    fn read(&mut self, buf: &mut [u8]) -> Option<usize> {
        unsafe {
            let count = sys_read(0, buf.as_mut_ptr(), buf.len());
            if count == 0xFFFFFFFF {
                None
            } else {
                Some(count)
            }
        }
    }
}

/// Standard Output
pub struct Stdout;

/// Create a standard output
pub fn stdout() -> Stdout {
    Stdout
}

/// Write implementation for standard output
impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> Option<usize> {
        unsafe {
            let count = sys_write(1, buf.as_ptr(), buf.len());
            if count == 0xFFFFFFFF {
                None
            } else {
                Some(count)
            }
        }
    }
}

impl fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write(s.as_bytes());

        result::Result::Ok(())
    }
}

/// Standard Error
pub struct Stderr;

/// Create a standard error
pub fn stderr() -> Stderr {
    Stderr
}

/// Write implementation for standard error
impl Write for Stderr {
    fn write(&mut self, buf: &[u8]) -> Option<usize> {
        unsafe {
            let count = sys_write(2, buf.as_ptr(), buf.len());
            if count == 0xFFFFFFFF {
                None
            } else {
                Some(count)
            }
        }
    }
}

pub fn _print(args: fmt::Arguments) {
    fmt::write(&mut stdout(), args);
}
