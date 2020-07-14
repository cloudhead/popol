# popol

*Minimal non-blocking I/O for Rust.*

Async I/O in Rust is still an unsolved problem. With the stabilization of
*async/await*, we are seeing a number of libraries and runtimes crop up, such
as *async-std* and *smol*, while others such as *tokio* and *mio* are maturing.
The problem with *async/await* is that you can't use any of the standard
library traits, such as `Read` and `Write`.  The async ecosystem comes with an
entirely separate suite of traits (eg. `AsyncRead` and `AsyncWrite`) and I/O
libraries. Furthermore, most of these runtimes have a large dependency
footprint, partly from having to provide async alternatives to the standard
library functions, and partly due to the complexity of these runtimes.

What do we need? For most use-cases, the ability to handle between a dozen
and up to a few hundred open connections without blocking, is all we need.
This places us squarely within the territory of the venerable `poll()` function,
which is available on almost all platforms.

Popol is designed as a minimal ergonomic wrapper around `poll()`, built for
peer-to-peer networking.

By building on `poll`, we have the following advantages:

* Much smaller implementation than even the smallest *async/await* runtimes
* All of the Rust standard library just works (`io::Read`, `io::Write`, etc.)
* Virtually zero-dependency (the *libc* crate is the only dependency)
* No "runtime". Keeps the code much easier to reason about

Why not use `epoll`? A couple of reasons:

1. It is more complex than `poll` and requires us to write more code
2. It isn't portable (only works on Linux)
3. `poll` is sufficient for handling most workloads

Compared to *mio*, *popol* is:

* A lot smaller (about 10% of the size)
* A little more flexible and easy to use
* Supports standard library sockets
* Currently focused on unix-based system compatibility

Some of the advantages of *popol*'s API over *mio*'s:

* *popol* supports multiple *wakers* per wait call.
* *popol* event source identifiers are not limited to `u64`.
* *popol*'s API is composed mostly of infallible functions.

On the other hand, *mio* is more mature and probably better at handling very
large number of connections. *Mio* also currently supports more platforms.

## License

This software is licensed under the MIT license. See the `LICENSE` file for
details.

## About

(c) Alexis Sellier <https://cloudhead.io>
