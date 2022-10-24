use std::io;
use std::io::prelude::*;
use std::net;

use popol::Poll;

fn main() -> io::Result<()> {
    let mut stream = net::TcpStream::connect("localhost:8888").unwrap();
    let mut poll = Poll::new();

    stream.set_nonblocking(true)?;
    poll.register(stream.peer_addr()?, &stream, popol::event::READ);

    loop {
        poll.wait()?;

        for (addr, event) in &poll {
            if event.is_readable() {
                let mut buf = [0; 32];

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
                        panic!("{}", err);
                    }
                }
            }
        }
    }
}
