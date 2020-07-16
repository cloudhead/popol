use std::{io, io::prelude::*, process, time};

use popol;

fn main() -> io::Result<()> {
    // Create a registry to hold I/O sources.
    let mut sources = popol::Sources::with_capacity(1);
    // Create an events buffer to hold readiness events.
    let mut events = popol::Events::with_capacity(1);

    // Register the program's standard input as a source of "read" readiness events.
    sources.register((), &io::stdin(), popol::interest::READ);

    // Wait on our event sources for at most 6 seconds. If an event source is
    // ready before then, process its events. Otherwise, timeout.
    match sources.wait_timeout(&mut events, time::Duration::from_secs(6)) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::TimedOut => process::exit(1),
        Err(err) => return Err(err),
    }

    // Iterate over source events. Since we only have one source
    // registered, this will only iterate once.
    for ((), event) in events.iter() {
        // An error occured with the standard input.
        if event.errored {
            panic!("error on {:?}", io::stdin());
        }
        // The standard input has data ready to be read.
        if event.readable || event.hangup {
            let mut buf = [0; 1024];

            // Read what we can from standard input and echo it.
            match io::stdin().read(&mut buf[..]) {
                Ok(n) => io::stdout().write_all(&buf[..n])?,
                Err(err) => panic!(err),
            }
        }
    }

    Ok(())
}
