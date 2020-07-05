use std::io;
use std::io::prelude::*;
use std::net;
use std::time;

use popol::Descriptors;

fn main() -> io::Result<()> {
    let mut stream = net::TcpStream::connect("localhost:8888").unwrap();
    let mut descriptors = Descriptors::new();

    stream.set_nonblocking(true)?;
    descriptors.register(stream.peer_addr()?, &stream, popol::events::READ);

    loop {
        let events = popol::wait(&mut descriptors, time::Duration::from_secs(6))?;

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
