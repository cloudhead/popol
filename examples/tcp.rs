use std::io;
use std::io::prelude::*;
use std::net;

use popol::{Sources, Timeout};

fn main() -> io::Result<()> {
    let mut stream = net::TcpStream::connect("localhost:8888").unwrap();
    let mut events = Vec::new();
    let mut sources = Sources::new();

    stream.set_nonblocking(true)?;
    sources.register(stream.peer_addr()?, &stream, popol::interest::READ);

    loop {
        sources.poll(&mut events, Timeout::Never)?;

        for event in events.drain(..) {
            if event.is_readable() {
                let mut buf = [0; 32];

                match stream.read(&mut buf[..]) {
                    Ok(n) if n > 0 => {
                        let msg = std::str::from_utf8(&buf[..n]).unwrap();
                        println!("{}: {}", event.key, msg.trim());
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
                        panic!("{}", err);
                    }
                }
            }
        }
    }
}
