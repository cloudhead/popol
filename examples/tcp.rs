use std::io;
use std::io::prelude::*;
use std::net;
use std::time;

use popol::{Descriptors, Poll};

fn main() -> io::Result<()> {
    let mut stream = net::TcpStream::connect("localhost:8888").unwrap();
    let mut descriptors = Descriptors::new();

    descriptors.register(stream.peer_addr()?, &stream, popol::events::READ);

    loop {
        match popol::wait(&mut descriptors, time::Duration::from_secs(6))? {
            Poll::Ready(events) => {
                for (addr, event) in events {
                    if event.readable {
                        let mut buf = [0; 32];

                        let n = stream.read(&mut buf[..])?;
                        if n == 0 {
                            return stream.shutdown(net::Shutdown::Both);
                        }

                        let msg = std::str::from_utf8(&buf[..n]).unwrap();
                        print!("{}: {}", addr, msg);
                    }
                }
            }
            Poll::Timeout => {}
        }
    }
}
