use std::collections::HashMap;
use std::io;
use std::net;

use popol::{Poll, Timeout};

/// The identifier we'll use with `popol` to figure out the source
/// of an event.
#[derive(Eq, PartialEq, Clone)]
enum Source {
    /// An event from a connected peer.
    Peer(net::SocketAddr),
    /// An event on the listening socket. Most probably a new peer connection.
    Listener,
}

fn main() -> io::Result<()> {
    let listener = net::TcpListener::bind("0.0.0.0:8888")?;
    let mut poll = Poll::new();
    let mut peers = HashMap::new();

    // It's important to set the socket in non-blocking mode. This allows
    // us to know when to stop accepting connections.
    listener.set_nonblocking(true)?;

    // Register the listener socket, using the corresponding identifier.
    poll.register(Source::Listener, &listener, popol::event::READ);

    loop {
        poll.wait_timeout(Timeout::Never)?;

        let mut new_peers = HashMap::new();
        for (key, event) in &poll {
            match key {
                Source::Peer(addr) if event.is_readable() => {
                    // Peer socket has data to be read.
                    println!("{} is readable", addr);
                }
                Source::Peer(addr) if event.is_writable() => {
                    // Peer socket is ready to be written.
                    println!("{} is writable", addr);
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
                    conn.set_nonblocking(true)?;

                    // Save the connection to make sure it isn't dropped.
                    new_peers.insert(addr, conn);
                },
            }
        }

        for (addr, stream) in &new_peers {
            // Register the new peer using the `Peer` variant of `Source`.
            poll.register(Source::Peer(*addr), stream, popol::event::ALL);
        }
        peers.extend(new_peers);
    }
}
