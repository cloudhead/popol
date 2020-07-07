use std::io;
use std::net;
use std::time;

use popol::Sources;

#[derive(Eq, PartialEq, Clone)]
enum Source {
    Peer(net::SocketAddr),
    Listener,
}

fn main() -> io::Result<()> {
    let listener = net::TcpListener::bind("0.0.0.0:8888")?;
    let mut sources = Sources::new();

    listener.set_nonblocking(true)?;
    sources.register(Source::Listener, &listener, popol::events::READ);

    let mut connected = Vec::new();

    loop {
        let events = popol::wait(&mut sources, time::Duration::from_secs(6))?;

        for (key, _event) in events.iter() {
            match key {
                Source::Peer(_addr) => {
                    // Handle peer activity
                }
                Source::Listener => loop {
                    let (conn, addr) = match listener.accept() {
                        Ok((conn, addr)) => (conn, addr),

                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(e) => return Err(e),
                    };
                    connected.push((addr, conn));
                },
            }
        }

        for (addr, conn) in connected.drain(..) {
            println!("{}", addr);

            sources.register(
                Source::Peer(addr),
                &conn,
                popol::events::READ | popol::events::WRITE,
            );
        }
    }
}
