use std::io;
use std::io::prelude::*;
use std::net;
use std::time;

use popol::{Events, Sources};

fn main() -> io::Result<()> {
    let mut stream = net::TcpStream::connect("localhost:8888").unwrap();
    let mut events = Events::new();
    let mut sources = Sources::new();

    stream.set_nonblocking(true)?;
    sources.register(stream.peer_addr()?, &stream, popol::interest::READ);

    loop {
        sources.wait(&mut events, time::Duration::from_secs(60))?;

        for (addr, event) in events.iter() {
            if event.readable {
                let mut buf = [0; 32];

                loop {
                    match stream.read(&mut buf[..]) {
                        Ok(n) if n > 0 => {
                            let msg = std::str::from_utf8(&buf[..n]).unwrap();
                            println!("{}: {}", addr, msg.trim());
                        }
                        Ok(_) => {
                            // Connection closed.
                            return stream.shutdown(net::Shutdown::Both);
                        }
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                            // Nothing left to read.
                            break;
                        }
                        Err(err) => {
                            panic!(err);
                        }
                    }
                }
            }
        }
    }
}
