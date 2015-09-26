//! Pippin implementation details.

//! Many code forms shamelessly lifted from Alex Crichton's flate2 library.

mod sum;

pub use self::read::read_head;

// Information stored in a file header
pub struct FileHeader {
    name: [u8; 16]
}

mod read {
    use std::io;
    use std::io::{Read, Result};
    use std::mem;
    use ::detail::FileHeader;
    use ::detail::sum;
    
    pub fn read_head(r: &mut Read) -> Result<FileHeader> {
        // A reader which also calculates a checksum:
        let mut sum_reader = sum::SumReader::new(r);
        
        let mut buf = [0; 16];
        try!(fill(&mut sum_reader, &mut buf));
        if buf != *b"PIPPINSS20150924" {
            return Err(invalid_input("not a known Pippin file format"));
        }
        
        let mut repo_name = [0; 16];
        try!(fill(&mut sum_reader, &mut repo_name));
        
        loop {
            try!(fill(&mut sum_reader, &mut buf));
            if buf[0] == b'H'{
                if buf[0..4] == *b"HSUM" {
                    match &buf[4..] {
                        b" SHA-2 256  " => { /* we don't support anything else */ },
                        _ => return Err(invalid_input("unknown checksum format"))
                    };
                    break;      // "HSUM" must be last item of header before final checksum
                }
                // else: ignore
            } else if buf[0] == b'Q' {
                let x: usize = match buf[1] {
                    b'0' ... b'9' => buf[1] - b'0',
                    b'A' ... b'Z' => buf[1] + 10 - b'A',
                    _ => return Err(invalid_input("header section Qx... has invalid length specification 'x'"))
                } as usize;
                let mut qbuf: Vec<u8> = Vec::new();
                qbuf.reserve_exact(16 * x);
                qbuf.extend(&buf);
                try!(fill(&mut sum_reader, &mut qbuf[16..]));
                //TODO: match against rules
            } else {
                return Err(invalid_input("unexpected header contents"));
            }
        }
        
        // TODO: read checksum and compare to above
        
        return Ok(FileHeader{
            name: repo_name
        });
        
        fn fill<R: Read>(r: &mut R, mut buf: &mut [u8]) -> Result<()> {
            while buf.len() > 0 {
                match try!(r.read(buf)) {
                    0 => return Err(invalid_input("corrupt (file terminates unexpectedly)")),
                    n => buf = &mut mem::replace(&mut buf, &mut [])[n..],
                }
            }
            Ok(())
        }
        
        fn invalid_input(msg: &str) -> io::Error {
            io::Error::new(io::ErrorKind::InvalidInput, msg)
        }
    }
}