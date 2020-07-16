use std::io;
use std::net;

use popol::{Events, Sources};

#[derive(Eq, PartialEq, Clone)]
enum Source {
    Peer(net::SocketAddr),
    Listener,
}

fn main() -> io::Result<()> {
    let listener = net::TcpListener::bind("0.0.0.0:8888")?;
    let mut sources = Sources::new();
    let mut events = Events::new();

    listener.set_nonblocking(true)?;
    sources.register(Source::Listener, &listener, popol::interest::READ);

    loop {
        sources.wait(&mut events)?;

        for (key, event) in events.iter() {
            match key {
                Source::Peer(_addr) if event.readable => {
                    // Peer socket has data to be read.
                }
                Source::Peer(_addr) if event.writable => {
                    // Peer socket is ready to be written.
                }
                Source::Peer(_addr) => {
                    // Peer socket had an error or hangup.
                }
                Source::Listener => loop {
                    let (conn, addr) = match listener.accept() {
                        Ok((conn, addr)) => (conn, addr),

                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(e) => return Err(e),
                    };

                    println!("{}", addr);

                    conn.set_nonblocking(true)?;
                    sources.register(Source::Peer(addr), &conn, popol::interest::ALL);
                },
            }
        }
    }
}
